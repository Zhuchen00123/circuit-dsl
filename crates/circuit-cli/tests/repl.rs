//! End-to-end REPL tests: pipe a script into the real binary.
//!
//! When stdin is not a terminal the REPL reads plain lines, so a session can be
//! driven exactly as a user would drive it — definitions, runs, overrides,
//! errors and commands — and the transcript can be asserted on. What this does
//! *not* cover is line editing, history and completion, which need a terminal;
//! those are verified by hand (see docs/repl.md).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn cdsl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cdsl"))
}

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn example(name: &str) -> PathBuf {
    project_root().join("examples").join(name)
}

/// Run `cdsl repl` with `script` on stdin.
fn repl(args: &[&str], script: &str) -> Output {
    let mut child = cdsl()
        .arg("repl")
        .args(args)
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
        .expect("write script");
    child.wait_with_output().expect("wait")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The `v(out) = ...` value from the transcript, in SI units.
fn v_out_values(transcript: &str) -> Vec<f64> {
    transcript
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("v(out) = ")?;
            let number: f64 = rest.split_whitespace().next()?.parse().ok()?;
            Some(if rest.contains("k") {
                number * 1e3
            } else {
                number
            })
        })
        .collect()
}

/// The session the brief asks for: define, run, change a parameter, run again.
#[test]
fn a_session_defines_runs_changes_a_parameter_and_runs_again() {
    let script = "\
circuit :divider do
  param :r1, default: 1.kohm
  param :r2, default: 1.kohm
  node :in, :out
  voltage_source :v1, p: :in, n: :gnd, dc: 5.V
  resistor :ra, p: :in, n: :out, value: r1
  resistor :rb, p: :out, n: :gnd, value: r2
end
experiment :divider, circuit: :divider do
  op
  save v(:out), i(:ra)
end
:run divider
:run divider r1=3.kohm
:quit
";
    let out = repl(&[], script);
    assert_eq!(out.status.code(), Some(0), "stderr:\n{}", stderr(&out));
    let text = stdout(&out);

    // Both runs happened and the numbers differ, which is the whole point of
    // an override: 5 V over equal halves, then over a 3:1 split.
    let values = v_out_values(&text);
    assert_eq!(values.len(), 2, "two runs expected in:\n{text}");
    assert!((values[0] - 2.5).abs() < 1e-9, "{values:?}");
    assert!((values[1] - 1.25).abs() < 1e-9, "{values:?}");

    // The transcript shows what happened, including the fixed override value.
    assert!(text.contains("defined circuit `divider`"), "{text}");
    assert!(text.contains("defined experiment `divider`"), "{text}");
    assert!(text.contains("override r1 = 3 kohm"), "{text}");
}

#[test]
fn a_session_evaluates_expressions_and_keeps_variables() {
    let out = repl(
        &[],
        "r = 1.kohm\nc = 100.nF\ntau = r * c\ntau\ntau / 2\n:quit\n",
    );
    assert_eq!(out.status.code(), Some(0));
    let text = stdout(&out);
    assert!(text.contains("r = 1 kohm"), "{text}");
    assert!(text.contains("=> 100 us"), "{text}");
    assert!(text.contains("=> 50 us"), "{text}");
}

#[test]
fn multi_line_input_shows_a_continuation_prompt() {
    let out = repl(
        &[],
        "circuit :one do\n  node :a\n  voltage_source :v1, p: :a, n: :gnd, dc: 1.V\nend\n:list\n:quit\n",
    );
    assert_eq!(out.status.code(), Some(0));
    let text = stdout(&out);
    // The first line prompts for input, the following lines prompt for a
    // continuation, and the definition is reported once `end` closes it.
    assert!(text.starts_with("cdsl> "), "{text}");
    assert!(text.contains("....> "), "{text}");
    assert!(text.contains("defined circuit `one`"), "{text}");
    assert!(text.contains("circuit :one"), "{text}");
}

#[test]
fn a_syntax_error_is_reported_and_the_session_continues() {
    let out = repl(&[], "node 5\nr = 2.ohm\nr\n:quit\n");
    // One input failed, so a scripted session reports it in its exit code.
    assert_eq!(out.status.code(), Some(1));
    let errors = stderr(&out);
    assert!(errors.contains("E_SYNTAX"), "{errors}");
    assert!(errors.contains("body statement"), "{errors}");
    // The session carried on: the variable was defined and shown.
    let text = stdout(&out);
    assert!(text.contains("=> 2 ohm"), "{text}");
}

#[test]
fn a_failed_definition_leaves_the_previous_one_running() {
    let script = "\
circuit :d do
  node :a
  voltage_source :v1, p: :a, n: :gnd, dc: 2.V
end
experiment :e, circuit: :d do
  op
  save v(:a)
end
circuit :d do
  node :a
  resistor :r, p: :a, n: :missing, value: 1.kohm
end
:run e
:quit
";
    let out = repl(&[], script);
    let errors = stderr(&out);
    assert!(errors.contains("E_NAME"), "{errors}");
    let text = stdout(&out);
    // The rejected redefinition changed nothing, so the original still runs.
    assert!(text.contains("v(a) = 2 V"), "{text}");
}

#[test]
fn a_session_can_load_an_example_file_and_run_it() {
    let file = example("voltage_divider.cdsl");
    let out = repl(
        &[&file.display().to_string()],
        ":list\n:run divider\n:quit\n",
    );
    assert_eq!(out.status.code(), Some(0), "stderr:\n{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("loaded"), "{text}");
    assert!(text.contains("circuit :divider"), "{text}");
    // v(out) = 3 V and i(r1) = 2 mA, the numbers the example documents.
    assert!(text.contains("v(out) = 3 V"), "{text}");
    assert!(text.contains("i(r1) = 2 mA"), "{text}");
}

#[test]
fn loading_a_file_twice_reports_the_clash_unless_told_to_replace() {
    let file = example("voltage_divider.cdsl");
    let path = file.display().to_string();
    let out = repl(
        &[],
        &format!(":load {path}\n:load {path}\n:load {path} --replace\n:quit\n"),
    );
    let errors = stderr(&out);
    assert!(errors.contains("E_DUPLICATE"), "{errors}");
    assert!(errors.contains("--replace"), "{errors}");
    let text = stdout(&out);
    // The first load defined, the `--replace` load replaced, and the middle
    // one changed nothing.
    assert_eq!(
        text.matches("defined circuit `divider`").count(),
        1,
        "{text}"
    );
    assert_eq!(
        text.matches("replaced circuit `divider`").count(),
        1,
        "{text}"
    );
}

#[test]
fn a_run_can_write_its_results() {
    let dir = std::env::temp_dir().join("cdsl_repl_out");
    let _ = std::fs::remove_dir_all(&dir);
    let out = repl(
        &[&example("rc_filter.cdsl").display().to_string()],
        &format!(":run response --out {}\n:quit\n", dir.display()),
    );
    assert_eq!(out.status.code(), Some(0), "stderr:\n{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("has"),
        "a transient should say it has points:\n{text}"
    );
    assert!(dir.join("response.tran1.csv").exists(), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quitting_and_end_of_input_both_exit_cleanly() {
    let by_command = repl(&[], ":quit\n");
    assert_eq!(by_command.status.code(), Some(0));

    // No `:quit`: end of input is the same as Ctrl+D.
    let by_eof = repl(&[], "1 + 1\n");
    assert_eq!(by_eof.status.code(), Some(0));
    assert!(stdout(&by_eof).contains("=> 2"));
}

#[test]
fn an_unknown_command_is_reported_and_a_symbol_is_not_one() {
    let out = repl(&[], ":ruhn\n:vin\n:quit\n");
    let text = stdout(&out);
    // `:vin` is a symbol value, not a command: only the fixed list is.
    assert!(text.contains("=> :vin"), "{text}");
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn a_variable_never_reaches_a_circuit() {
    let script = "\
r = 5.kohm
circuit :leak do
  node :a
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  resistor :r1, p: :a, n: :gnd, value: r
end
:quit
";
    let out = repl(&[], script);
    let errors = stderr(&out);
    assert!(errors.contains("`r` is not declared"), "{errors}");
    assert_eq!(out.status.code(), Some(1));
}
