//! Round-4 QA acceptance: the four reproductions the plan requires, driven
//! through the real CLI binary.
//!
//! Every failure test asserts the exit code, the diagnostic class, and that the
//! failed run left no result file behind. Each family also has a positive
//! control, so "everything fails" cannot make these pass.
//!
//! The same file is the release gate for R4-02:
//!   cargo build --release -p circuit-cli
//!   cargo test --release -p circuit-cli --test r4_repro_cli
//! In release the old unchecked exponent arithmetic wrapped silently instead of
//! panicking, so the release run is what proves the overflow is reported.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .to_path_buf()
}

/// A per-test scratch directory under `target/round4/qa`, removed first so a
/// leftover file can never be mistaken for this run's output.
fn scratch(tag: &str) -> PathBuf {
    let dir = workspace_root()
        .join("target")
        .join("round4")
        .join("qa")
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the QA scratch directory");
    dir
}

struct Cli {
    code: i32,
    stdout: String,
    stderr: String,
}

fn cdsl(args: &[&str]) -> Cli {
    let out = Command::new(env!("CARGO_BIN_EXE_cdsl"))
        .args(args)
        .output()
        .expect("the cdsl binary runs");
    Cli {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// The files directly inside `dir`, sorted; empty when it does not exist.
fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// A rejected input: exit 1, a diagnostic, and certainly not a panic (a panic
/// exits 101, which is what the pre-fix build did on the dimension overflow).
fn assert_user_error(out: &Cli, label: &str) {
    assert!(
        !out.stderr.contains("panicked"),
        "{label}: a panic is not a diagnostic:\n{}",
        out.stderr
    );
    assert_ne!(
        out.code, 101,
        "{label}: the process panicked instead of reporting:\n{}",
        out.stderr
    );
    assert_eq!(
        out.code, 1,
        "{label}: a user error is EXIT_USER_ERROR:\nstdout:\n{}\nstderr:\n{}",
        out.stdout, out.stderr
    );
    assert!(
        out.stderr.contains("error["),
        "{label}: a rendered diagnostic must be printed:\n{}",
        out.stderr
    );
}

/// A failed run announces nothing and leaves nothing behind.
fn assert_wrote_nothing(out: &Cli, dir: &Path, label: &str) {
    assert!(
        !out.stdout.contains("wrote"),
        "{label}: no file may be announced:\n{}",
        out.stdout
    );
    let files = files_in(dir);
    assert!(
        files.is_empty(),
        "{label}: a failed run must not export anything, found {files:?}"
    );
}

const DIVIDER: &str = "circuit :divider do
  node :vin, :out
  voltage_source :src, p: :vin, n: :gnd, dc: 1.V
  resistor :r1, p: :vin, n: :out, value: 1.kohm
  resistor :r2, p: :out, n: :gnd, value: 1.kohm
end
";

fn divider_file(dir: &Path, name: &str, experiment: &str) -> PathBuf {
    let file = dir.join(name);
    std::fs::write(&file, format!("{DIVIDER}\n{experiment}")).expect("write the fixture");
    file
}

/// `sqrt(-1)`: a negative square root is not a number, so neither the derive
/// nor the measure that wraps it in `min` may succeed.
fn sqrt_case(dir: &Path) -> PathBuf {
    divider_file(
        dir,
        "r4-01-sqrt.cdsl",
        "experiment :sqrt_repro, circuit: :divider do
  op
  derive :invalid, expr: sqrt(-1)
  measure :masked, max: min(sqrt(-1), 2)
end
",
    )
}

/// `1e308 * 1e308`: infinite in IEEE, so it must be refused rather than
/// reduced by the surrounding `min`.
fn mul_case(dir: &Path) -> PathBuf {
    divider_file(
        dir,
        "r4-01-mul.cdsl",
        "experiment :mul_repro, circuit: :divider do
  op
  derive :invalid, expr: 1e308 * 1e308
  measure :masked, max: min(1e308 * 1e308, 2)
end
",
    )
}

/// `n` copies of `v(:vin)` multiplied together: V^n, whose exponent n must
/// stay inside an i8.
fn product_case(dir: &Path, n: usize) -> PathBuf {
    let factors = vec!["v(:vin)"; n].join(" * ");
    divider_file(
        dir,
        &format!("r4-02-{n}.cdsl"),
        &format!(
            "experiment :dim_overflow, circuit: :divider do
  op
  derive :huge, expr: {factors}
  measure :m, max: abs(v(:out))
end
"
        ),
    )
}

// ---------------------------------------------------------------------------
// R4-01 through the CLI
// ---------------------------------------------------------------------------

#[test]
fn check_rejects_a_constant_sqrt_of_negative_one() {
    let dir = scratch("r4_check_sqrt");
    let file = sqrt_case(&dir);
    let out = cdsl(&["check", &file.display().to_string()]);

    assert_user_error(&out, "check sqrt(-1)");
    assert!(
        out.stderr.contains("E_VALUE"),
        "an illegal constant value is E_VALUE:\n{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("sqrt"),
        "the failing operation must be named:\n{}",
        out.stderr
    );
    // The diagnostic keys are part of the contract (contract 1.3 rule 4): the
    // check-time path names the definition (derive/measure), the offending
    // sample and its index, and says there is no axis rather than inventing one.
    for key in [
        "= signal:",
        "= index: 0",
        "= derive: invalid",
        "= measure: masked",
    ] {
        assert!(
            out.stderr.contains(key),
            "the diagnostic must carry {key}:\n{}",
            out.stderr
        );
    }
    assert!(
        !out.stdout.contains("wrote"),
        "check never writes results:\n{}",
        out.stdout
    );
}

#[test]
fn run_rejects_a_constant_sqrt_of_negative_one_without_writing() {
    let dir = scratch("r4_run_sqrt");
    let file = sqrt_case(&dir);
    let out_dir = dir.join("out");
    let out = cdsl(&[
        "run",
        &file.display().to_string(),
        "--experiment",
        "sqrt_repro",
        "--out",
        &out_dir.display().to_string(),
        "--format",
        "csv",
    ]);

    assert_user_error(&out, "run sqrt(-1)");
    assert!(out.stderr.contains("E_VALUE"), "{}", out.stderr);
    assert!(out.stderr.contains("sqrt"), "{}", out.stderr);
    // A runtime failure names the analysis it happened in, the definition that
    // asked for the value, the sample and its index, and the sub-expression.
    for key in [
        "= analysis: op1",
        "= kind: op",
        "= signal: invalid",
        "= sample:",
        "= index: 0",
        "= expression: sqrt(-1)",
    ] {
        assert!(
            out.stderr.contains(key),
            "the runtime diagnostic must carry {key}:\n{}",
            out.stderr
        );
    }
    assert!(
        !out.stdout.contains("measure masked"),
        "no successful measurement may be printed:\n{}",
        out.stdout
    );
    assert_wrote_nothing(&out, &out_dir, "run sqrt(-1)");
}

#[test]
fn check_rejects_a_constant_multiplication_overflow() {
    let dir = scratch("r4_check_mul");
    let file = mul_case(&dir);
    let out = cdsl(&["check", &file.display().to_string()]);

    assert_user_error(&out, "check 1e308 * 1e308");
    assert!(out.stderr.contains("E_VALUE"), "{}", out.stderr);
    assert!(
        out.stderr.contains('*'),
        "the failing operation must be shown:\n{}",
        out.stderr
    );
    for key in ["= signal:", "= index: 0", "= measure: masked"] {
        assert!(
            out.stderr.contains(key),
            "the diagnostic must carry {key}:\n{}",
            out.stderr
        );
    }
}

#[test]
fn run_rejects_a_constant_multiplication_overflow_without_writing() {
    let dir = scratch("r4_run_mul");
    let file = mul_case(&dir);
    let out_dir = dir.join("out");
    let out = cdsl(&[
        "run",
        &file.display().to_string(),
        "--experiment",
        "mul_repro",
        "--out",
        &out_dir.display().to_string(),
        "--format",
        "csv",
    ]);

    assert_user_error(&out, "run 1e308 * 1e308");
    assert!(out.stderr.contains("E_VALUE"), "{}", out.stderr);
    assert!(
        out.stderr.contains("= expression: (1e308 * 1e308)"),
        "the overflowing sub-expression must be named:\n{}",
        out.stderr
    );
    assert!(
        !out.stdout.contains("measure masked"),
        "min(inf, 2) must not be reported as 2:\n{}",
        out.stdout
    );
    assert_wrote_nothing(&out, &out_dir, "run 1e308 * 1e308");
}

// ---------------------------------------------------------------------------
// R4-02 through the CLI: 128 voltage factors
// ---------------------------------------------------------------------------

#[test]
fn check_rejects_a_128_factor_voltage_product() {
    let dir = scratch("r4_check_dim");
    let file = product_case(&dir, 128);
    let out = cdsl(&["check", &file.display().to_string()]);

    assert_user_error(&out, "check 128 factors");
    assert!(
        out.stderr.contains("E_DIMENSION"),
        "an unrepresentable exponent is E_DIMENSION:\n{}",
        out.stderr
    );
    assert!(
        !out.stderr.contains("E_LIMIT"),
        "128 factors is a dimension overflow, not a size limit:\n{}",
        out.stderr
    );
    // The overflow is located: a file/line/column arrow and the underlined
    // source, exactly like every other front-end diagnostic.
    assert!(out.stderr.contains("-->"), "{}", out.stderr);
    assert!(out.stderr.contains("^^^^"), "{}", out.stderr);
}

#[test]
fn run_rejects_a_128_factor_voltage_product_without_panicking() {
    let dir = scratch("r4_run_dim");
    let file = product_case(&dir, 128);
    let out_dir = dir.join("out");
    let out = cdsl(&[
        "run",
        &file.display().to_string(),
        "--experiment",
        "dim_overflow",
        "--out",
        &out_dir.display().to_string(),
        "--format",
        "csv",
    ]);

    // Debug used to panic here ("attempt to add with overflow", exit 101);
    // release used to wrap and exit 0 with a plausible measure. Both are
    // failures: the only acceptable answer is a diagnostic.
    assert_user_error(&out, "run 128 factors");
    assert!(
        !out.stderr.contains("attempt to add with overflow"),
        "the exponent arithmetic must be checked:\n{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("E_DIMENSION"),
        "the overflow must be reported as a dimension error:\n{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("-->"),
        "the rejection is located in the source:\n{}",
        out.stderr
    );
    assert!(
        !out.stdout.contains("measure m"),
        "a wrapped product must not produce a successful measurement:\n{}",
        out.stdout
    );
    assert_wrote_nothing(&out, &out_dir, "run 128 factors");
}

/// A long but representable product still runs: 64 factors give V^64, well
/// inside the exponent range, so the guard must be an overflow check and not a
/// cap on expression length.
///
/// (A 96-factor chain currently crashes the run path with a stack overflow
/// before the dimension check can speak. That is an open finding recorded in
/// docs/review-evidence/round4/qa-acceptance.md, not something this control
/// should hide: the control keeps a depth that is legal *and* executable.)
#[test]
fn a_64_factor_voltage_product_still_runs() {
    let dir = scratch("r4_edge_dim");
    let file = product_case(&dir, 64);
    let out_dir = dir.join("out");
    let out = cdsl(&[
        "run",
        &file.display().to_string(),
        "--experiment",
        "dim_overflow",
        "--out",
        &out_dir.display().to_string(),
        "--format",
        "csv",
    ]);

    assert_eq!(out.code, 0, "V^64 is representable:\n{}", out.stderr);
    assert!(
        out_dir.join("dim_overflow.op1.csv").exists(),
        "a legal product must still export:\n{}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// Positive controls: the harness can see success
// ---------------------------------------------------------------------------

/// The same divider and the same experiment shape, with legal expressions:
/// exit 0, a printed measure, and a result file. Without this, a fixture that
/// fails to parse would make every failure test above pass for the wrong reason.
#[test]
fn the_control_program_with_legal_expressions_still_succeeds() {
    let dir = scratch("r4_control");
    let file = divider_file(
        &dir,
        "control.cdsl",
        "experiment :control, circuit: :divider do
  op
  derive :half, expr: min(v(:out), v(:vin))
  measure :masked, max: min(sqrt(4), 2)
end
",
    );
    let out_dir = dir.join("out");
    let out = cdsl(&[
        "run",
        &file.display().to_string(),
        "--experiment",
        "control",
        "--out",
        &out_dir.display().to_string(),
        "--format",
        "csv",
    ]);

    assert_eq!(out.code, 0, "{}", out.stderr);
    // sqrt(4) = 2 and min(2, 2) = 2, and v(out) = 0.5 V is the divider's half.
    assert!(
        out.stdout.contains("measure masked = 2"),
        "the control measure must print:\n{}",
        out.stdout
    );
    assert!(
        out_dir.join("control.op1.csv").exists(),
        "the control run must export:\n{}",
        out.stdout
    );
    let csv = std::fs::read_to_string(out_dir.join("control.op1.csv")).expect("csv");
    assert!(csv.contains("half"), "the derive must be a column:\n{csv}");
}
