//! Round-4 phase-B QA: check-time rejection of topology-affecting parameter
//! sweeps, and the sweeps that must keep working.
//!
//! Contract: docs/review-evidence/round4/design-contract.md §4.5-§4.6. Every
//! value asserted below is derived by hand in a comment; the refusal rows assert
//! the diagnostic code, the explanation path and the located spans.
//!
//! Before phase B the first three fixtures were accepted by cdsl check (exit 0)
//! and only the runtime topology comparison could catch them, after the solve.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .to_path_buf()
}

/// A per-test scratch directory under target/round4/qa-b.
fn scratch(tag: &str) -> PathBuf {
    let dir = workspace_root()
        .join("target")
        .join("round4")
        .join("qa-b")
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

fn fixture(dir: &Path, name: &str, text: &str) -> PathBuf {
    let file = dir.join(name);
    std::fs::write(&file, text).expect("write the fixture");
    file
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

/// A refusal: exit 1, E_TOPO_PARAM, an explanation with at least two located
/// spans, and no panic.
fn assert_topo_refusal(out: &Cli, label: &str) -> String {
    assert!(
        !out.stderr.contains("panicked"),
        "{label}: a diagnostic, not a panic:\n{}",
        out.stderr
    );
    assert_eq!(
        out.code, 1,
        "{label}: the sweep must be refused:\nstdout:\n{}\nstderr:\n{}",
        out.stdout, out.stderr
    );
    assert!(
        out.stderr.contains("[E_TOPO_PARAM]"),
        "{label}: the code must be E_TOPO_PARAM:\n{}",
        out.stderr
    );
    let arrows = out.stderr.matches("-->").count();
    assert!(
        arrows >= 2,
        "{label}: the explanation must locate the path (>= 2 spans), found {arrows}:\n{}",
        out.stderr
    );
    out.stderr.clone()
}

/// Whether the rendered diagnostic locates this source line.
fn located(stderr: &str, file: &Path, line: usize) -> bool {
    stderr.contains(&format!("{}:{}", file.display(), line))
}

fn read_csv(path: &Path) -> (Vec<String>, Vec<Vec<String>>) {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let mut lines = text.lines();
    let header: Vec<String> = lines
        .next()
        .unwrap_or_else(|| panic!("{} has no header", path.display()))
        .split(',')
        .map(str::to_string)
        .collect();
    let rows: Vec<Vec<String>> = lines
        .map(|line| line.split(',').map(str::to_string).collect())
        .collect();
    (header, rows)
}

fn column(header: &[String], name: &str) -> usize {
    header
        .iter()
        .position(|h| h == name)
        .unwrap_or_else(|| panic!("no column {name} in {header:?}"))
}

fn number(header: &[String], row: &[String], name: &str) -> f64 {
    let text = &row[column(header, name)];
    text.parse()
        .unwrap_or_else(|e| panic!("column {name} is not a number ({text}): {e}"))
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// n is read directly by the loop range, so the device count depends on it.
const DIRECT: &str = "circuit :var_flat do
  param :n, default: 2
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  for k in 1..n do
    resistor (\"r\" + k), p: :a, n: :b, value: 1.kohm
  end
  resistor :rload, p: :b, n: :gnd, value: 1.kohm
end
experiment :sweep, circuit: :var_flat do
  dc param: :n, from: 1, to: 3, step: 1
  save v(:b)
end
";

/// n reaches the loop through the intermediate parameter width.
const INDIRECT: &str = "circuit :var_indirect do
  param :n, default: 1
  param :width, default: n + 1
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  for k in 1..width do
    resistor (\"r\" + k), p: :a, n: :b, value: 1.kohm
  end
  resistor :rload, p: :b, n: :gnd, value: 1.kohm
end
experiment :sweep, circuit: :var_indirect do
  dc param: :n, from: 0, to: 2, step: 1
  save v(:b)
end
";

/// flag decides which branch of the if builds the load.
const VIA_IF: &str = "circuit :var_if do
  param :flag, default: 1
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  resistor :r1, p: :a, n: :b, value: 1.kohm
  if flag > 0 do
    resistor :r2, p: :b, n: :gnd, value: 1.kohm
  else
    resistor :r2, p: :b, n: :gnd, value: 2.kohm
  end
end
experiment :sweep, circuit: :var_if do
  dc param: :flag, from: 0, to: 2, step: 1
  save v(:b)
end
";

/// rf only ever lands in a numeric position, so sweeping it is a value sweep.
/// v(out) = 3 V * rf / (1 kohm + rf): 1.5 V, 2 V, 2.25 V.
const VALUE_SWEEP: &str = "circuit :swept do
  param :rf, default: 1.kohm
  node :in, :out
  voltage_source :src, p: :in, n: :gnd, dc: 3.V
  resistor :r1, p: :in, n: :out, value: 1.kohm
  resistor :r2, p: :out, n: :gnd, value: rf
end
experiment :sweep, circuit: :swept do
  dc param: :rf, from: 1.kohm, to: 3.kohm, step: 1.kohm
  save v(:out)
end
";

/// Two parameters named n in different scopes. The swept one is the top-level
/// n, used only as a value; the instance's own n drives its loop but is not
/// wired from the parent, so it must not be implicated.
/// With stage = two parallel 1 kohm resistors from out to ground, R = 500 ohm,
/// so v(out) = 1 V * 500 / (n * 1000 + 500): 1/3, 0.2, 1/7.
const SCOPE_ISOLATION: &str = "subcircuit :leaf, ports: [:a, :b] do
  param :n, default: 2
  for k in 1..n do
    resistor (\"rl\" + k), p: :a, n: :b, value: 1.kohm
  end
end
circuit :top_safe do
  param :n, default: 1
  node :in, :out
  voltage_source :src, p: :in, n: :gnd, dc: 1.V
  resistor :r1, p: :in, n: :out, value: n * 1.kohm
  instance :stage, of: :leaf, ports: { a: :out, b: :gnd }
end
experiment :sweep_safe, circuit: :top_safe do
  dc param: :n, from: 1, to: 3, step: 1
  save v(:out)
end
";

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[test]
fn check_refuses_a_sweep_read_by_a_loop_range() {
    let dir = scratch("r4b_topo_direct");
    let file = fixture(&dir, "direct.cdsl", DIRECT);
    let out = cdsl(&["check", &file.display().to_string()]);

    let stderr = assert_topo_refusal(&out, "direct loop range");
    // The path locates the declaration (line 2) and the topology use point
    // (the for range on line 5).
    assert!(
        located(&stderr, &file, 2),
        "the scanned parameter's declaration must be located:\n{stderr}"
    );
    assert!(
        located(&stderr, &file, 5),
        "the for range that reads it must be located:\n{stderr}"
    );
    // The message has to say that a sweep which reaches no topology use is
    // still allowed.
    let lower = stderr.to_lowercase();
    let says_value_sweep_is_allowed = [
        "value sweep",
        "plain value",
        "sweep of a value",
        "value-only sweep",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
        || (lower.contains("value") && lower.contains("allow"));
    assert!(
        says_value_sweep_is_allowed,
        "the refusal must say that a sweep which only changes a value is still allowed:\n{stderr}"
    );
    assert!(
        !out.stdout.contains("wrote"),
        "check writes nothing:\n{}",
        out.stdout
    );
}

#[test]
fn check_explains_the_indirect_path_through_an_intermediate_parameter() {
    let dir = scratch("r4b_topo_indirect");
    let file = fixture(&dir, "indirect.cdsl", INDIRECT);
    let out = cdsl(&["check", &file.display().to_string()]);

    let stderr = assert_topo_refusal(&out, "indirect loop range");
    // n (line 2) -> width (line 3) -> the for range that reads width (line 6).
    assert!(
        located(&stderr, &file, 2),
        "the scanned parameter's declaration must be located:\n{stderr}"
    );
    assert!(
        located(&stderr, &file, 3),
        "the intermediate parameter must be located:\n{stderr}"
    );
    assert!(
        located(&stderr, &file, 6),
        "the use point must be located:\n{stderr}"
    );
    assert!(
        stderr.contains("width"),
        "the intermediate parameter must be on the path:\n{stderr}"
    );
}

#[test]
fn check_refuses_a_sweep_read_by_an_if_condition() {
    let dir = scratch("r4b_topo_if");
    let file = fixture(&dir, "via_if.cdsl", VIA_IF);
    let out = cdsl(&["check", &file.display().to_string()]);

    let stderr = assert_topo_refusal(&out, "if condition");
    assert!(
        located(&stderr, &file, 2),
        "the scanned parameter's declaration must be located:\n{stderr}"
    );
    assert!(
        located(&stderr, &file, 6),
        "the if condition that reads it must be located:\n{stderr}"
    );
}

#[test]
fn run_refuses_the_same_sweep_before_solving_and_writes_nothing() {
    let dir = scratch("r4b_topo_run");
    let file = fixture(&dir, "direct.cdsl", DIRECT);
    let out_dir = dir.join("out");
    let out = cdsl(&[
        "run",
        &file.display().to_string(),
        "--experiment",
        "sweep",
        "--out",
        &out_dir.display().to_string(),
    ]);

    assert_topo_refusal(&out, "run refuses the topology sweep");
    assert!(
        !out.stdout.contains("wrote"),
        "nothing may be written:\n{}",
        out.stdout
    );
    assert!(
        files_in(&out_dir).is_empty(),
        "a refused sweep leaves no file: {:?}",
        files_in(&out_dir)
    );
}

// ---------------------------------------------------------------------------
// What must keep working
// ---------------------------------------------------------------------------

#[test]
fn a_plain_value_sweep_is_still_accepted_and_exact() {
    let dir = scratch("r4b_value_sweep");
    let file = fixture(&dir, "value.cdsl", VALUE_SWEEP);

    let checked = cdsl(&["check", &file.display().to_string()]);
    assert_eq!(
        checked.code, 0,
        "a value sweep is not a topology sweep:\n{}",
        checked.stderr
    );
    assert!(
        !checked.stderr.contains("E_TOPO_PARAM"),
        "{}",
        checked.stderr
    );

    let out_dir = dir.join("out");
    let run = cdsl(&[
        "run",
        &file.display().to_string(),
        "--experiment",
        "sweep",
        "--out",
        &out_dir.display().to_string(),
        "--format",
        "csv",
    ]);
    assert_eq!(run.code, 0, "{}", run.stderr);

    // A DC parameter sweep puts the swept coordinate in the "parameter" column.
    let (header, rows) = read_csv(&out_dir.join("sweep.dc_param_rf.csv"));
    assert_eq!(rows.len(), 3, "1, 2 and 3 kohm: {}", rows.len());
    for row in &rows {
        let rf = number(&header, row, "parameter");
        let vout = number(&header, row, "v(out)");
        // v(out) = 3 * rf / (1000 + rf).
        let expected = 3.0 * rf / (1000.0 + rf);
        assert!(
            (vout - expected).abs() < 1e-9,
            "at rf = {rf}: got {vout}, expected {expected}"
        );
    }
    let values: Vec<f64> = rows
        .iter()
        .map(|row| number(&header, row, "v(out)"))
        .collect();
    assert!((values[0] - 1.5).abs() < 1e-9, "{values:?}");
    assert!((values[1] - 2.0).abs() < 1e-9, "{values:?}");
    assert!((values[2] - 2.25).abs() < 1e-9, "{values:?}");
}

#[test]
fn a_same_named_instance_parameter_is_not_implicated() {
    let dir = scratch("r4b_scope_isolation");
    let file = fixture(&dir, "scope.cdsl", SCOPE_ISOLATION);

    let checked = cdsl(&["check", &file.display().to_string()]);
    assert_eq!(
        checked.code, 0,
        "the instance's own n is a different node in the graph:\n{}",
        checked.stderr
    );

    let out_dir = dir.join("out");
    let run = cdsl(&[
        "run",
        &file.display().to_string(),
        "--experiment",
        "sweep_safe",
        "--out",
        &out_dir.display().to_string(),
        "--format",
        "csv",
    ]);
    assert_eq!(run.code, 0, "{}", run.stderr);

    let (header, rows) = read_csv(&out_dir.join("sweep_safe.dc_param_n.csv"));
    assert_eq!(rows.len(), 3, "{}", rows.len());
    let values: Vec<f64> = rows
        .iter()
        .map(|row| number(&header, row, "v(out)"))
        .collect();
    // R(stage) = 1k || 1k = 500 ohm is fixed: the instance keeps its own n = 2.
    for (n, v) in values.iter().enumerate() {
        let n = (n + 1) as f64;
        let expected = 500.0 / (n * 1000.0 + 500.0);
        assert!(
            (v - expected).abs() < 1e-9,
            "at n = {n}: got {v}, expected {expected}"
        );
    }
}
