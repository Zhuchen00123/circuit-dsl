//! Round-4 phase-B QA regression: rounds 2 and 3 behaviour with parameters
//! resolved through the new dependency graph.
//!
//! Contract: docs/review-evidence/round4/design-contract.md §4.4-§4.6 — the
//! graph decides the order, but the language's own evaluation and units rules
//! are unchanged, and nothing about transient grids, raw-grid measurements,
//! analysis binding or the export set may move.
//!
//! Each expected number is derived in a comment.

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

fn run_csv(file: &Path, experiment: &str, out: &Path) -> Cli {
    let cli = cdsl(&[
        "run",
        &file.display().to_string(),
        "--experiment",
        experiment,
        "--out",
        &out.display().to_string(),
        "--format",
        "csv",
    ]);
    assert_eq!(cli.code, 0, "the run must succeed:\n{}", cli.stderr);
    cli
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

fn measure_line(stdout: &str, name: &str) -> String {
    let prefix = format!("measure {name} ");
    stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no measure {name} in:\n{stdout}"))
        .to_string()
}

// ---------------------------------------------------------------------------
// Transient grid and raw-grid measurements
// ---------------------------------------------------------------------------

/// R = 1 kohm and C = 100 nF give tau = R C = 100 us; the task pulse is a 0 -> 1 V
/// step at t = 0. The response is v(t) = 1 - exp(-t/tau), so with T = 2 ms:
///   avg = 1 - (tau/T)(1 - exp(-T/tau)) = 1 - (1/20)(1 - e^-20) = 0.95.
///
/// The parameter chain here is c -> tau_c (= 2c) -> c_eff (= tau_c / 2), all
/// resolved by the graph, so the device value is 100 nF again and neither the
/// grid nor the measurement may move because of it.
const RC: &str = "circuit :rc do
  param :tau_c, default: 2 * c
  param :c_eff, default: tau_c / 2
  param :r, default: 1.kohm
  param :c, default: 100.nF
  node :vin, :vout
  voltage_source :src, p: :vin, n: :gnd, dc: 0.V,
    waveform: pulse(low: 0.V, high: 1.V, delay: 0.s, rise: 1.ns,
                    fall: 1.ns, width: 5.ms, period: 10.ms)
  resistor :r1, p: :vin, n: :vout, value: r
  capacitor :c1, p: :vout, n: :gnd, value: c_eff
end
experiment :grid, circuit: :rc do
  tran stop: 2.ms, max_step: 10.us, output_interval: 250.us
  save v(:vout)
  measure :vavg, avg: v(:vout)
end
experiment :raw, circuit: :rc do
  tran stop: 2.ms, max_step: 10.us
  save v(:vout)
  measure :vavg, avg: v(:vout)
end
";

#[test]
fn a_parameterized_transient_keeps_its_grid_and_raw_measures() {
    let dir = scratch("r4b_reg_transient");
    let file = dir.join("rc.cdsl");
    std::fs::write(&file, RC).expect("write the fixture");

    let out = dir.join("out");
    let grid = run_csv(&file, "grid", &out);
    let raw = run_csv(&file, "raw", &out);

    let tau = 100e-6;
    let interval = 250e-6;
    let (header, rows) = read_csv(&out.join("grid.tran1.csv"));
    let times: Vec<f64> = rows.iter().map(|r| number(&header, r, "time")).collect();
    let volts: Vec<f64> = rows.iter().map(|r| number(&header, r, "v(vout)")).collect();

    assert!(times.len() >= 4, "grid points: {}", times.len());
    for (k, t) in times.iter().enumerate() {
        if k + 1 == times.len() {
            assert!(
                (t - 2e-3).abs() < 1e-12,
                "the raw endpoint 2 ms must be kept: {t}"
            );
        } else {
            let expected = k as f64 * interval;
            assert!(
                (t - expected).abs() < 1e-12,
                "grid point {k}: {t} s, expected {expected} s"
            );
        }
    }
    // Interpolation error over 10 us solver steps is about
    // (max_step^2/8) * max|v| = 1.25e-7 V, so 1e-3 V still catches a moved
    // waveform or a wrong component value.
    for (t, v) in times.iter().zip(&volts) {
        let expected = 1.0 - (-t / tau).exp();
        assert!(
            (v - expected).abs() < 1e-3,
            "at t = {t} s: v = {v}, expected {expected}"
        );
    }

    // The measure is computed on the raw grid, so the coarse output request
    // cannot move it.
    assert_eq!(
        measure_line(&grid.stdout, "vavg"),
        measure_line(&raw.stdout, "vavg"),
        "output_interval must not move a measurement"
    );
    assert!(
        measure_line(&grid.stdout, "vavg").contains("0.95"),
        "avg over 20 tau is 1 - (1/20)(1 - e^-20) = 0.95: {}",
        measure_line(&grid.stdout, "vavg")
    );
}

// ---------------------------------------------------------------------------
// Analysis binding
// ---------------------------------------------------------------------------

/// The experiment overrides c to 10 nF, so the RC corner becomes
/// fc = 1/(2 pi R C) = 1/(2 pi * 1e3 * 1e-8) = 15915.494309189535 Hz, ten times
/// the default. The AC point is placed exactly there, where x = w R C = 1 and
///   |H| = 1/sqrt(1 + x^2) = 1/sqrt(2).
/// The op derive still sees the operating point: the capacitor blocks DC and
/// the dc source is 1 V, so v(vout) = 1 V.
const BINDING: &str = "circuit :rc do
  param :r, default: 1.kohm
  param :c, default: 100.nF
  node :vin, :vout
  voltage_source :src, p: :vin, n: :gnd, dc: 1.V, ac: 1.V
  resistor :r1, p: :vin, n: :vout, value: r
  capacitor :c1, p: :vout, n: :gnd, value: c
end
experiment :binding, circuit: :rc do
  param :c, value: 10.nF
  op
  ac from: 15915.494309189535.Hz, to: 159154.94309189535.Hz, points_per_decade: 1
  save v(:vin)
  derive :dc_gain, expr: v(:vout), analysis: :op1
  derive :ac_mag, expr: abs(v(:vout)), analysis: :ac1
end
";

#[test]
fn an_analysis_binding_still_picks_the_bound_analysis() {
    let dir = scratch("r4b_reg_binding");
    let file = dir.join("binding.cdsl");
    std::fs::write(&file, BINDING).expect("write the fixture");

    let out = dir.join("out");
    run_csv(&file, "binding", &out);

    let (op_header, op_rows) = read_csv(&out.join("binding.op1.csv"));
    assert_eq!(op_rows.len(), 1, "an operating point has one row");
    let dc_gain = number(&op_header, &op_rows[0], "dc_gain");
    assert!(
        (dc_gain - 1.0).abs() < 1e-12,
        "the capacitor blocks DC, so v(vout) = 1 V: {dc_gain}"
    );

    let (ac_header, ac_rows) = read_csv(&out.join("binding.ac1.csv"));
    assert!(!ac_rows.is_empty(), "the AC sweep must have a point");
    for row in &ac_rows {
        let f = number(&ac_header, row, "frequency");
        let x = 2.0 * std::f64::consts::PI * f * 1e3 * 10e-9;
        let expected = 1.0 / (1.0 + x * x).sqrt();
        let got = number(&ac_header, row, "ac_mag");
        assert!(
            (got - expected).abs() < 1e-9,
            "at {f} Hz: |H| = {got}, expected {expected}"
        );
    }
    let x = 2.0 * std::f64::consts::PI * 15915.494309189535 * 1e3 * 10e-9;
    assert!(
        (x - 1.0).abs() < 1e-9,
        "the sweep must start exactly at the corner (x = wRC = {x})"
    );
}

// ---------------------------------------------------------------------------
// Export set
// ---------------------------------------------------------------------------

/// r_lo = 3 * r_hi = 1500 ohm, so v(out) = 5 V * 1500/2000 = 3.75 V and
/// ratio = v(in)/v(out) = 5/3.75 = 4/3.
///
/// The save list is the export set: the derive is a column, and v(in) is read
/// by the expression but must not appear as a column.
const EXPORT: &str = "circuit :div do
  param :r_lo, default: 3 * r_hi
  param :r_hi, default: 500.ohm
  node :in, :out
  voltage_source :src, p: :in, n: :gnd, dc: 5.V
  resistor :r1, p: :in, n: :out, value: r_hi
  resistor :r2, p: :out, n: :gnd, value: r_lo
end
experiment :implicit, circuit: :div do
  op
  save v(:out)
  derive :ratio, expr: v(:in) / v(:out)
end
";

#[test]
fn the_export_set_still_follows_save_with_parameterized_signals() {
    let dir = scratch("r4b_reg_export");
    let file = dir.join("export.cdsl");
    std::fs::write(&file, EXPORT).expect("write the fixture");

    let out = dir.join("out");
    run_csv(&file, "implicit", &out);
    let (header, rows) = read_csv(&out.join("implicit.op1.csv"));

    assert_eq!(header.len(), 2, "saved probe plus derive: {header:?}");
    assert!(header.iter().any(|h| h == "v(out)"), "{header:?}");
    assert!(header.iter().any(|h| h == "ratio"), "{header:?}");
    assert!(
        !header.iter().any(|h| h == "v(in)"),
        "an expression input is read, not exported: {header:?}"
    );
    assert_eq!(rows.len(), 1);
    let vout = number(&header, &rows[0], "v(out)");
    assert!(
        (vout - 3.75).abs() < 1e-12,
        "v(out) = {vout}, expected 3.75"
    );
    let ratio = number(&header, &rows[0], "ratio");
    assert!(
        (ratio - 4.0 / 3.0).abs() < 1e-12,
        "v(in)/v(out) = 5/3.75 = 4/3, got {ratio}"
    );
}
