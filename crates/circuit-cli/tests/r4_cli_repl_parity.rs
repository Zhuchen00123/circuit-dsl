//! Round-4 QA: the same expression through `cdsl run` and through the REPL
//! path (`circuit_session::Session`, which is what `cdsl repl` drives), so a
//! user cannot get two different answers depending on where they typed it.
//!
//! Three levels are checked:
//! - the numeric result: the exported CSV is byte-identical, and the printed
//!   measure line is identical character for character;
//! - the error: same class (`E_...`) and same message text;
//! - the real `cdsl repl` binary, for the same numbers.
//!
//! Independent reference (Ohm/KCL, derived here and not copied from any
//! existing test): with vp at 1 V and vn at -0.5 V,
//!   (v(mid) - 1)/1k + (v(mid) + 0.5)/3k = 0  =>  4*v(mid) = 2.5  =>  v(mid) = 0.625 V
//!   v(pos)/v(neg) = 1 / -0.5 = -2.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use circuit_session::session::{Options, Reply, Session};

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

/// The fixture: one legal experiment and one whose measure is illegal at run
/// time (`sqrt` of a negative *signal* ratio, so `check` cannot decide it
/// statically and the runtime path is really exercised).
fn fixture(dir: &Path) -> PathBuf {
    let file = dir.join("parity.cdsl");
    std::fs::write(
        &file,
        "circuit :dual do
  node :pos, :neg, :mid
  voltage_source :vp, p: :pos, n: :gnd, dc: 1.V
  voltage_source :vn, p: :gnd, n: :neg, dc: 0.5.V
  resistor :r1, p: :pos, n: :mid, value: 1.kohm
  resistor :r2, p: :mid, n: :neg, value: 3.kohm
end
experiment :num, circuit: :dual do
  op
  save v(:pos)
  derive :mid, expr: v(:mid)
  measure :ratio, max: v(:pos) / v(:neg)
end
experiment :bad, circuit: :dual do
  op
  measure :boom, max: sqrt(v(:neg) / v(:pos))
end
",
    )
    .expect("write the fixture");
    file
}

fn measure_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| line.starts_with("measure "))
        .unwrap_or_else(|| panic!("no measure line in:\n{text}"))
        .to_string()
}

fn first_error_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| line.starts_with("error["))
        .unwrap_or_else(|| panic!("no diagnostic in:\n{text}"))
        .to_string()
}

/// The `E_...` class of a rendered diagnostic line.
fn error_code(line: &str) -> String {
    let start = line.find('[').expect("a code bracket") + 1;
    let end = line.find(']').expect("a code bracket");
    line[start..end].to_string()
}

/// The `= key: value` context/notes lines of a rendered diagnostic, in order.
fn context_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| line.starts_with("= "))
        .map(str::to_string)
        .collect()
}

// ---------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------

#[test]
fn the_same_expression_gives_the_same_number_and_the_same_file() {
    let dir = scratch("r4_parity_num");
    let file = fixture(&dir);
    let cli_out = dir.join("cli");
    let repl_out = dir.join("repl");

    let cli = cdsl(&[
        "run",
        &file.display().to_string(),
        "--experiment",
        "num",
        "--out",
        &cli_out.display().to_string(),
        "--format",
        "csv",
    ]);
    assert_eq!(cli.code, 0, "{}", cli.stderr);

    let mut session = Session::new(Options::default());
    session.load(&file, false).expect("the fixture loads");
    let reply = session
        .run("num", &[], Some(&repl_out))
        .expect("the same experiment runs in the session");
    let text = match reply {
        Reply::Message(text) => text,
        other => panic!("expected a run summary, got {other:?}"),
    };

    // 1. The printed measure is identical, character for character.
    let cli_measure = measure_line(&cli.stdout);
    let repl_measure = measure_line(&text);
    assert_eq!(
        cli_measure, repl_measure,
        "CLI and REPL must print the same measure line"
    );
    // ... and it is the independently derived -2 (dimensionless).
    assert!(
        cli_measure.contains("-2"),
        "v(pos)/v(neg) = 1/-0.5 = -2, got {cli_measure}"
    );

    // 2. The exported CSV is byte-identical.
    let cli_csv = std::fs::read_to_string(cli_out.join("num.op1.csv")).expect("CLI csv");
    let repl_csv = std::fs::read_to_string(repl_out.join("num.op1.csv")).expect("REPL csv");
    assert_eq!(
        cli_csv, repl_csv,
        "the same experiment must export the same bytes"
    );

    // 3. Both containers carry the same derived value: v(mid) = 0.625 V.
    let mut lines = cli_csv.lines();
    let header: Vec<&str> = lines.next().expect("header").split(',').collect();
    let row: Vec<&str> = lines.next().expect("one row").split(',').collect();
    let mid = header
        .iter()
        .position(|h| *h == "mid")
        .unwrap_or_else(|| panic!("the derive must be a column: {header:?}"));
    let value: f64 = row[mid].parse().expect("a number");
    assert!(
        (value - 0.625).abs() < 1e-12,
        "v(mid) must be 0.625 V, got {value}"
    );
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[test]
fn the_same_illegal_expression_gives_the_same_error_class_and_text() {
    let dir = scratch("r4_parity_bad");
    let file = fixture(&dir);

    let cli = cdsl(&[
        "run",
        &file.display().to_string(),
        "--experiment",
        "bad",
        "--out",
        &dir.join("cli").display().to_string(),
        "--format",
        "csv",
    ]);
    assert_eq!(
        cli.code, 1,
        "an illegal measure is a user error:\n{}",
        cli.stderr
    );
    assert!(!cli.stderr.contains("panicked"), "{}", cli.stderr);

    let mut session = Session::new(Options::default());
    session.load(&file, false).expect("the fixture loads");
    let diagnostics = session
        .run("bad", &[], None)
        .expect_err("the same expression must fail in the session");
    let repl_text = diagnostics.render(session.sources());

    let cli_line = first_error_line(&cli.stderr);
    let repl_line = first_error_line(&repl_text);
    assert_eq!(
        error_code(&cli_line),
        "E_VALUE",
        "sqrt of a negative sample is a value error:\n{cli_line}"
    );
    assert_eq!(
        cli_line, repl_line,
        "CLI and REPL must report the same error text\nCLI:\n{}\nREPL:\n{}",
        cli.stderr, repl_text
    );
    assert!(
        cli_line.contains("sqrt"),
        "the failing operation must be named: {cli_line}"
    );

    // The structured keys are part of the contract (§1.3 rule 4) and must be
    // the same on both paths.
    let cli_keys = context_lines(&cli.stderr);
    let repl_keys = context_lines(&repl_text);
    assert!(!cli_keys.is_empty(), "a diagnostic must carry context keys");
    assert_eq!(
        cli_keys, repl_keys,
        "CLI and REPL must attach the same context keys"
    );
    assert!(
        cli_keys
            .iter()
            .any(|line| line.starts_with("= analysis: op1")),
        "{cli_keys:?}"
    );

    // And neither path produced a result file.
    for sub in ["cli", "repl"] {
        let out = dir.join(sub);
        let files: Vec<_> = std::fs::read_dir(&out)
            .map(|entries| entries.filter_map(Result::ok).collect())
            .unwrap_or_default();
        assert!(files.is_empty(), "a failed run must write nothing in {sub}");
    }
}

// ---------------------------------------------------------------------------
// The real REPL binary
// ---------------------------------------------------------------------------

#[test]
fn the_real_repl_binary_reports_the_same_numbers() {
    let dir = scratch("r4_parity_repl_bin");
    let file = fixture(&dir);
    let repl_out = dir.join("repl");
    let script = format!(
        ":load {}\n:run num --out {}\n:quit\n",
        file.display(),
        repl_out.display()
    );

    let mut child = Command::new(env!("CARGO_BIN_EXE_cdsl"))
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cdsl repl");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write the script");
    let out = child.wait_with_output().expect("wait for the repl");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(0), "stderr:\n{stderr}");

    let repl_measure = measure_line(&stdout);
    assert!(
        repl_measure.contains("-2"),
        "the REPL must print the same -2: {repl_measure}"
    );

    // Byte-identical to what the session API wrote above.
    let mut session = Session::new(Options::default());
    session.load(&file, false).expect("the fixture loads");
    let api_out = dir.join("api");
    let _ = session
        .run("num", &[], Some(&api_out))
        .expect("the session runs the experiment");
    let repl_csv = std::fs::read_to_string(repl_out.join("num.op1.csv")).expect("repl csv");
    let api_csv = std::fs::read_to_string(api_out.join("num.op1.csv")).expect("api csv");
    assert_eq!(repl_csv, api_csv);
}
