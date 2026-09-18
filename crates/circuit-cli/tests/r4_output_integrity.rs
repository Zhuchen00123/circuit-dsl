//! Round-4 QA: a failed run must not produce a successful output file, and it
//! must not delete or corrupt whatever an earlier run wrote.
//!
//! The plan requires both shapes:
//! - a *runtime* failure: a valid derive is evaluated first, then an illegal
//!   measure (sqrt of a negative signal ratio) fails;
//! - a *check-time* failure: a valid derive, then a constant illegal derive
//!   (sqrt(-1)), which the front end can decide without running.
//!
//! A control test proves the same harness sees a successful run replace the
//! sentinel, so "nothing was written" is not an artefact of a broken fixture.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .to_path_buf()
}

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

/// One circuit, three experiments: a legal one, a runtime failure, and a
/// check-time failure. Each experiment's first statement is a valid derive.
const FIXTURE: &str = "circuit :dual do
  node :pos, :neg, :mid
  voltage_source :vp, p: :pos, n: :gnd, dc: 1.V
  voltage_source :vn, p: :gnd, n: :neg, dc: 0.5.V
  resistor :r1, p: :pos, n: :mid, value: 1.kohm
  resistor :r2, p: :mid, n: :neg, value: 3.kohm
end
experiment :ok, circuit: :dual do
  op
  save v(:pos)
  derive :good, expr: v(:mid)
end
experiment :runtime_fail, circuit: :dual do
  op
  save v(:pos)
  derive :good, expr: v(:mid)
  measure :boom, max: sqrt(v(:neg) / v(:pos))
end
experiment :constant_fail, circuit: :dual do
  op
  save v(:pos)
  derive :good, expr: v(:mid)
  derive :bad, expr: sqrt(-1)
end
";

fn fixture(dir: &Path) -> PathBuf {
    let file = dir.join("integrity.cdsl");
    std::fs::write(&file, FIXTURE).expect("write the fixture");
    file
}

/// An output directory that already holds the previous run's files.
fn out_dir_with_old_results(dir: &Path, experiment: &str) -> PathBuf {
    let out = dir.join("out");
    std::fs::create_dir_all(&out).expect("create the output directory");
    std::fs::write(out.join(format!("{experiment}.op1.csv")), "OLD CSV\n").expect("old csv");
    std::fs::write(out.join(format!("{experiment}.op1.json")), "OLD JSON\n").expect("old json");
    out
}

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

/// Run one experiment with the default (both) format.
fn run(file: &Path, experiment: &str, out: &Path) -> Cli {
    cdsl(&[
        "run",
        &file.display().to_string(),
        "--experiment",
        experiment,
        "--out",
        &out.display().to_string(),
    ])
}

#[test]
fn a_runtime_failure_after_a_valid_derive_writes_nothing_and_keeps_the_old_files() {
    let dir = scratch("r4_integrity_runtime");
    let file = fixture(&dir);
    let out = out_dir_with_old_results(&dir, "runtime_fail");

    let cli = run(&file, "runtime_fail", &out);

    assert_eq!(
        cli.code, 1,
        "the illegal measure must fail the run:\n{}",
        cli.stderr
    );
    assert!(!cli.stderr.contains("panicked"), "{}", cli.stderr);
    assert!(cli.stderr.contains("E_VALUE"), "{}", cli.stderr);
    assert!(
        !cli.stdout.contains("measure boom"),
        "a failed run prints no measure:\n{}",
        cli.stdout
    );
    assert!(
        !cli.stdout.contains("wrote"),
        "a failed run announces no file:\n{}",
        cli.stdout
    );

    assert_eq!(
        files_in(&out),
        vec!["runtime_fail.op1.csv", "runtime_fail.op1.json"],
        "no new file may appear, and no old file may vanish"
    );
    assert_eq!(
        std::fs::read_to_string(out.join("runtime_fail.op1.csv")).expect("old csv"),
        "OLD CSV\n",
        "an existing result must not be overwritten by a failed run"
    );
    assert_eq!(
        std::fs::read_to_string(out.join("runtime_fail.op1.json")).expect("old json"),
        "OLD JSON\n"
    );
}

#[test]
fn a_check_time_failure_after_a_valid_derive_writes_nothing_and_keeps_the_old_files() {
    let dir = scratch("r4_integrity_constant");
    let file = fixture(&dir);
    let out = out_dir_with_old_results(&dir, "constant_fail");

    let cli = run(&file, "constant_fail", &out);

    assert_eq!(cli.code, 1, "sqrt(-1) must fail the run:\n{}", cli.stderr);
    assert!(!cli.stderr.contains("panicked"), "{}", cli.stderr);
    assert!(
        cli.stderr.contains("E_VALUE"),
        "an illegal constant is E_VALUE:\n{}",
        cli.stderr
    );
    assert!(cli.stderr.contains("sqrt"), "{}", cli.stderr);
    assert!(
        !cli.stdout.contains("wrote"),
        "a failed run announces no file:\n{}",
        cli.stdout
    );

    assert_eq!(
        files_in(&out),
        vec!["constant_fail.op1.csv", "constant_fail.op1.json"],
        "no new file may appear, and no old file may vanish"
    );
    assert_eq!(
        std::fs::read_to_string(out.join("constant_fail.op1.csv")).expect("old csv"),
        "OLD CSV\n"
    );

    // The same file is also refused by check, so the failure is not something
    // only run notices.
    let checked = cdsl(&["check", &file.display().to_string()]);
    assert_eq!(
        checked.code, 1,
        "check must reject the file:\n{}",
        checked.stderr
    );
    assert!(checked.stderr.contains("sqrt"), "{}", checked.stderr);
}

#[test]
fn the_control_experiment_replaces_the_old_files() {
    let dir = scratch("r4_integrity_control");
    let file = fixture(&dir);
    let out = out_dir_with_old_results(&dir, "ok");

    let cli = run(&file, "ok", &out);

    assert_eq!(
        cli.code, 0,
        "the legal experiment must run:\n{}",
        cli.stderr
    );
    assert!(
        cli.stdout.contains("wrote"),
        "a successful run announces its files:\n{}",
        cli.stdout
    );
    let csv = std::fs::read_to_string(out.join("ok.op1.csv")).expect("csv");
    assert_ne!(csv, "OLD CSV\n", "a successful run replaces the old file");
    // v(mid) = 0.625 V is exported as a derived column.
    assert!(csv.contains("good"), "{csv}");
    assert!(csv.contains("0.625"), "{csv}");
    // The JSON result is there too (the default format is both).
    assert!(out.join("ok.op1.json").exists());
}
