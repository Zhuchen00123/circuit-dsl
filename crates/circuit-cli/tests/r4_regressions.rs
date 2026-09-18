//! Round-4 QA regression: the older features the acceptance matrix requires to
//! stay unchanged, each checked against a reference derived here.
//!
//! Every expected number is written out in a comment from the formula it comes
//! from (Ohm/KCL, the RC transfer function, the transient integral); none is
//! copied from an existing test. The tolerances are the solver/interpolation
//! bounds argued in each test, not a blanket epsilon.

use std::f64::consts::PI;
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

/// Run one experiment and export CSV, asserting the run succeeded.
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

/// A CSV result as a header plus its data rows, with every field kept as text.
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

fn cell<'a>(header: &[String], row: &'a [String], name: &str) -> &'a str {
    &row[column(header, name)]
}

fn number(header: &[String], row: &[String], name: &str) -> f64 {
    let text = cell(header, row, name);
    text.parse()
        .unwrap_or_else(|e| panic!("column {name} is not a number ({text}): {e}"))
}

/// The whole trim-around-ed line of a named measure, e.g.
/// "vavg = 0.95 V (tran1)".
fn measure_line(stdout: &str, name: &str) -> String {
    let prefix = format!("measure {name} ");
    stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no measure {name} in:\n{stdout}"))
        .to_string()
}

/// The numeric part of a named measure, parsed from its rendered line.
fn measure_value(stdout: &str, name: &str) -> f64 {
    let line = measure_line(stdout, name);
    let rest = line
        .split_once(" = ")
        .unwrap_or_else(|| panic!("unexpected measure line: {line}"))
        .1;
    let token = rest
        .split_whitespace()
        .next()
        .unwrap_or_else(|| panic!("no value in measure line: {line}"));
    token
        .parse()
        .unwrap_or_else(|e| panic!("measure value {token} does not parse: {e}"))
}

const DIVIDER_5V: &str = "circuit :divider5 do
  node :in, :out
  voltage_source :src, p: :in, n: :gnd, dc: 5.V
  resistor :r1, p: :in, n: :out, value: 1.kohm
  resistor :r2, p: :out, n: :gnd, value: 1.5.kohm
end
";

// ---------------------------------------------------------------------------
// RC corner frequency and gain
// ---------------------------------------------------------------------------

/// H(jw) = 1 / (1 + j w R C) with R = 1 kohm and C = 100 nF, so
///   fc = 1/(2 pi R C) = 1/(2 pi 1e-4) = 1591.5494309189535 Hz,
///   Re H = 1/(1+x^2), Im H = -x/(1+x^2) with x = w R C,
///   |H| = 1/sqrt(1+x^2), gain_db = 20 log10 |H| = -10 log10(1+x^2).
/// At the corner x = 1: H = 0.5 - 0.5j, |H| = 1/sqrt(2), dB = -3.010299956639812.
#[test]
fn the_rc_corner_and_gain_match_the_transfer_function() {
    const FC: f64 = 1591.5494309189535;
    let dir = scratch("r4_reg_rc");
    let file = dir.join("rc.cdsl");
    std::fs::write(
        &file,
        "circuit :rc do
  node :vin, :vout
  voltage_source :src, p: :vin, n: :gnd, dc: 1.V, ac: 1.V
  resistor :r1, p: :vin, n: :vout, value: 1.kohm
  capacitor :c1, p: :vout, n: :gnd, value: 100.nF
end
experiment :sweep, circuit: :rc do
  ac from: 1591.5494309189535.Hz, to: 15.915494309189535.kHz, points_per_decade: 10
  save v(:vout), v(:vin)
  derive :gdb, expr: gain_db(v(:vout), v(:vin)), analysis: :ac1
end
",
    )
    .expect("write the RC fixture");

    let out = dir.join("out");
    run_csv(&file, "sweep", &out);
    let (header, rows) = read_csv(&out.join("sweep.ac1.csv"));
    // One decade starting at the corner, 10 points per decade: 11 rows, and
    // the first one is the corner itself (asserted below).
    assert!(
        rows.len() >= 10,
        "a decade at 10 points per decade: {}",
        rows.len()
    );
    let first_f = number(&header, &rows[0], "frequency");
    let last_f = number(&header, rows.last().expect("a last row"), "frequency");
    assert!(
        (first_f - FC).abs() < 1e-9 * FC,
        "the sweep must start at the corner: {first_f}"
    );
    assert!(
        (last_f - 10.0 * FC).abs() < 1e-6 * FC,
        "the sweep must end one decade above the corner: {last_f}"
    );

    let mut corner = None;
    let mut vin_checked = 0;
    for row in &rows {
        let f = number(&header, row, "frequency");
        let re = number(&header, row, "v(vout)_re");
        let im = number(&header, row, "v(vout)_im");
        let db = number(&header, row, "gdb");

        // The AC source is 1 V, so v(vin) is 1 + 0j at every point.
        assert!((number(&header, row, "v(vin)_re") - 1.0).abs() < 1e-12);
        assert!(number(&header, row, "v(vin)_im").abs() < 1e-12);
        vin_checked += 1;

        let x = 2.0 * PI * f * 1e3 * 100e-9;
        let expected_re = 1.0 / (1.0 + x * x);
        let expected_im = -x / (1.0 + x * x);
        assert!(
            (re - expected_re).abs() < 1e-9,
            "at {f} Hz: Re H = {re}, expected {expected_re}"
        );
        assert!(
            (im - expected_im).abs() < 1e-9,
            "at {f} Hz: Im H = {im}, expected {expected_im}"
        );
        let expected_db = -10.0 * (1.0 + x * x).log10();
        assert!(
            (db - expected_db).abs() < 1e-9,
            "at {f} Hz: gain_db = {db}, expected {expected_db}"
        );

        if (f - FC).abs() < 1e-9 * FC {
            corner = Some((f, re, im, db));
        }
    }
    assert_eq!(vin_checked, rows.len());

    let (f, re, im, db) = corner.expect("the sweep must contain the exact corner frequency");
    assert!((f - FC).abs() < 1e-9 * FC, "corner point at {f} Hz");
    assert!((re - 0.5).abs() < 1e-9, "Re H(fc) = {re}, expected 0.5");
    assert!((im + 0.5).abs() < 1e-9, "Im H(fc) = {im}, expected -0.5");
    assert!(
        (db + 3.010299956639812).abs() < 1e-9,
        "gain_db(fc) = {db}, expected -3.010299956639812"
    );
}

// ---------------------------------------------------------------------------
// Resistor power
// ---------------------------------------------------------------------------

/// 5 V across 1 kohm + 1.5 kohm: I = 5/2500 = 2 mA, so
///   v(out) = 5 * 1500/2500 = 3 V,
///   P(r1) = I^2 R = (2e-3)^2 * 1e3 = 4e-3 V*A,
///   P(r2) = v(out)^2 / 1.5k = 9 / 1500 = 6e-3 V*A,
///   total 10 mW = 5 V * 2 mA.
#[test]
fn resistor_power_matches_i_squared_r() {
    let dir = scratch("r4_reg_power");
    let file = dir.join("power.cdsl");
    std::fs::write(
        &file,
        format!(
            "{DIVIDER_5V}experiment :op_only, circuit: :divider5 do
  op
  save v(:in), v(:out)
  derive :pr1, expr: (v(:in) - v(:out)) * i(:r1)
  derive :pr2, expr: v(:out) * i(:r2)
  measure :itotal, max: i(:r1)
end
"
        ),
    )
    .expect("write the power fixture");

    let out = dir.join("out");
    let cli = run_csv(&file, "op_only", &out);
    let (header, rows) = read_csv(&out.join("op_only.op1.csv"));
    assert_eq!(rows.len(), 1, "an operating point has one row");
    let row = &rows[0];

    assert!((number(&header, row, "v(in)") - 5.0).abs() < 1e-12);
    assert!((number(&header, row, "v(out)") - 3.0).abs() < 1e-12);
    assert!(
        (number(&header, row, "pr1") - 4e-3).abs() < 1e-15,
        "P(r1) = I^2 R = 4e-3 V*A, got {}",
        number(&header, row, "pr1")
    );
    assert!(
        (number(&header, row, "pr2") - 6e-3).abs() < 1e-15,
        "P(r2) = v(out)^2 / 1.5k = 6e-3 V*A, got {}",
        number(&header, row, "pr2")
    );
    // The total power is the source's 5 V * 2 mA = 10 mW.
    let total = number(&header, row, "pr1") + number(&header, row, "pr2");
    assert!((total - 1e-2).abs() < 1e-15, "{total}");
    // i(r1) = 2 mA, rendered as 0.002 in SI base units.
    let itotal = measure_value(&cli.stdout, "itotal");
    assert!((itotal - 2e-3).abs() < 1e-12, "{itotal}");
}

// ---------------------------------------------------------------------------
// Multi-analysis binding
// ---------------------------------------------------------------------------

/// One expression per analysis: the op derive must see the operating point
/// (v(vout) = 1 V with the 1 V DC source and an open capacitor) and the AC
/// derive must see |H(f)|, not the other way round.
///
/// At f = 1 kHz, x = 2 pi f R C = 0.6283185307179586, so
/// |H| = 1/sqrt(1+x^2) = 0.846733... (computed in the test).
#[test]
fn each_derive_uses_the_analysis_it_is_bound_to() {
    let dir = scratch("r4_reg_binding");
    let file = dir.join("binding.cdsl");
    std::fs::write(
        &file,
        "circuit :rc do
  node :vin, :vout
  voltage_source :src, p: :vin, n: :gnd, dc: 1.V, ac: 1.V
  resistor :r1, p: :vin, n: :vout, value: 1.kohm
  capacitor :c1, p: :vout, n: :gnd, value: 100.nF
end
experiment :binding, circuit: :rc do
  op
  ac from: 1.kHz, to: 10.kHz, points_per_decade: 1
  save v(:vin)
  derive :dc_gain, expr: v(:vout), analysis: :op1
  derive :ac_mag, expr: abs(v(:vout)), analysis: :ac1
end
",
    )
    .expect("write the binding fixture");

    let out = dir.join("out");
    run_csv(&file, "binding", &out);

    // The operating point: the capacitor blocks DC, so v(vout) = v(vin) = 1 V.
    let (op_header, op_rows) = read_csv(&out.join("binding.op1.csv"));
    assert_eq!(op_rows.len(), 1);
    let dc_gain = number(&op_header, &op_rows[0], "dc_gain");
    assert!((dc_gain - 1.0).abs() < 1e-12, "op v(vout) = {dc_gain}");
    assert!((number(&op_header, &op_rows[0], "v(vin)") - 1.0).abs() < 1e-12);
    assert!(
        !op_header.iter().any(|h| h == "ac_mag"),
        "an op result must not carry the AC derive: {op_header:?}"
    );

    // The AC sweep: |H(f)| = 1/sqrt(1+(2 pi f R C)^2).
    let (ac_header, ac_rows) = read_csv(&out.join("binding.ac1.csv"));
    assert!(ac_rows.len() >= 2, "a decade at one point per decade");
    assert!(
        !ac_header.iter().any(|h| h == "dc_gain"),
        "an AC result must not carry the op derive: {ac_header:?}"
    );
    let mut first = None;
    for row in &ac_rows {
        let f = number(&ac_header, row, "frequency");
        let x = 2.0 * PI * f * 1e3 * 100e-9;
        let expected = 1.0 / (1.0 + x * x).sqrt();
        let got = number(&ac_header, row, "ac_mag");
        assert!(
            (got - expected).abs() < 1e-9,
            "at {f} Hz: |H| = {got}, expected {expected}"
        );
        if first.is_none() {
            first = Some((f, got));
        }
    }
    let (f1, ac_mag) = first.expect("at least one AC point");
    assert!((f1 - 1e3).abs() < 1e-6, "first AC point at {f1} Hz");
    // The two binds really are different analyses.
    assert!(
        (dc_gain - ac_mag).abs() > 0.1,
        "the op and AC derives must not be the same number: {dc_gain} vs {ac_mag}"
    );
}

// ---------------------------------------------------------------------------
// Implicit probes
// ---------------------------------------------------------------------------

/// An expression input is read but not exported: the CSV has the saved v(out)
/// and the derived ratio, and no v(in) column, while the ratio uses v(in) and
/// therefore proves the implicit read happened.
///
/// 5 V over 1 kohm + 1.5 kohm: v(out) = 3 V, so v(in)/v(out) = 5/3.
#[test]
fn an_expression_input_is_read_but_not_exported() {
    let dir = scratch("r4_reg_implicit");
    let file = dir.join("implicit.cdsl");
    std::fs::write(
        &file,
        format!(
            "{DIVIDER_5V}experiment :implicit, circuit: :divider5 do
  op
  save v(:out)
  derive :ratio, expr: v(:in) / v(:out)
end
"
        ),
    )
    .expect("write the implicit-probe fixture");

    let out = dir.join("out");
    run_csv(&file, "implicit", &out);
    let (header, rows) = read_csv(&out.join("implicit.op1.csv"));

    assert_eq!(header.len(), 2, "saved probe plus derive: {header:?}");
    assert!(header.iter().any(|h| h == "v(out)"), "{header:?}");
    assert!(
        header.iter().any(|h| h == "ratio"),
        "the derive must be exported: {header:?}"
    );
    assert!(
        !header.iter().any(|h| h == "v(in)"),
        "an expression input is read, not exported: {header:?}"
    );
    assert_eq!(rows.len(), 1);
    let ratio = number(&header, &rows[0], "ratio");
    assert!(
        (ratio - 5.0 / 3.0).abs() < 1e-12,
        "v(in)/v(out) = 5/3, got {ratio}"
    );

    // check --json documents the same read under implicit_probes.
    let checked = cdsl(&["check", &file.display().to_string(), "--json"]);
    assert_eq!(checked.code, 0, "{}", checked.stderr);
    let parsed: serde_json::Value =
        serde_json::from_str(&checked.stdout).expect("check --json must emit valid JSON");
    let implicit = parsed["experiments"][0]["analyses"][0]["implicit_probes"]
        .as_array()
        .expect("implicit_probes array");
    assert!(
        implicit
            .iter()
            .any(|p| p.as_str().unwrap_or_default().contains("v(in)")),
        "{implicit:?}"
    );
}

// ---------------------------------------------------------------------------
// output_interval resampling
// ---------------------------------------------------------------------------

/// R = 1 kohm, C = 100 nF, so tau = R C = 100 us; the source is a 0 -> 1 V
/// pulse. The ideal response is v(t) = 1 - exp(-t/tau).
///
///   T = 2 ms = 20 tau, so
///   avg = 1 - (tau/T)(1 - exp(-T/tau)) = 1 - (1/20)(1 - 2.06e-9) = 0.95,
///   max = 1 - exp(-20) = 0.9999999979.
///
/// With output_interval: 250 us the exported grid is 0, 250 us, ..., 1.75 ms
/// plus the raw endpoint at 2 ms (the language keeps the last solver point),
/// and the measurements are computed on the raw grid, so they must not move.
#[test]
fn output_interval_resamples_the_output_without_moving_the_measurement() {
    let dir = scratch("r4_reg_output_interval");
    let file = dir.join("grid.cdsl");
    let circuit = "circuit :rc_step do
  node :vin, :vout
  voltage_source :src, p: :vin, n: :gnd, dc: 0.V,
    waveform: pulse(low: 0.V, high: 1.V, delay: 0.s, rise: 1.ns,
                    fall: 1.ns, width: 5.ms, period: 10.ms)
  resistor :r1, p: :vin, n: :vout, value: 1.kohm
  capacitor :c1, p: :vout, n: :gnd, value: 100.nF
end
";
    std::fs::write(
        &file,
        format!(
            "{circuit}experiment :grid, circuit: :rc_step do
  tran stop: 2.ms, max_step: 10.us, output_interval: 250.us
  save v(:vout)
  measure :vavg, avg: v(:vout)
  measure :vmax, max: v(:vout)
end
experiment :raw, circuit: :rc_step do
  tran stop: 2.ms, max_step: 10.us
  save v(:vout)
  measure :vavg, avg: v(:vout)
  measure :vmax, max: v(:vout)
end
"
        ),
    )
    .expect("write the resampling fixture");

    let out = dir.join("out");
    let grid = run_csv(&file, "grid", &out);
    let raw = run_csv(&file, "raw", &out);

    let tau = 100e-6;
    let interval = 250e-6;

    // The exported grid is the requested uniform one, with the raw endpoint.
    let (header, rows) = read_csv(&out.join("grid.tran1.csv"));
    let times: Vec<f64> = rows.iter().map(|r| number(&header, r, "time")).collect();
    let volts: Vec<f64> = rows.iter().map(|r| number(&header, r, "v(vout)")).collect();
    assert!(times.len() >= 4, "grid points: {}", times.len());
    assert!(
        times[0].abs() < 1e-15,
        "the grid starts at 0 s: {}",
        times[0]
    );
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
    for pair in times.windows(2).take(times.len() - 2) {
        assert!(
            ((pair[1] - pair[0]) - interval).abs() < 1e-12,
            "the interior spacing must be 250 us: {pair:?}"
        );
    }

    // The resampled values follow the analytic step: interpolation error over
    // 10 us solver steps is about (max_step^2/8) * max|v| = 1e-10/8 * 1e4 =
    // 1.25e-7 V, so 1e-3 V is a conservative bound that still catches a wrong
    // grid or a waveform that moved.
    for (t, v) in times.iter().zip(&volts) {
        let expected = 1.0 - (-t / tau).exp();
        assert!(
            (v - expected).abs() < 1e-3,
            "at t = {t} s: v = {v}, expected {expected}"
        );
    }

    // Without output_interval the solver grid is its own, generally non-uniform.
    let (raw_header, raw_rows) = read_csv(&out.join("raw.tran1.csv"));
    let raw_times: Vec<f64> = raw_rows
        .iter()
        .map(|r| number(&raw_header, r, "time"))
        .collect();
    assert!(
        raw_times
            .windows(2)
            .any(|w| ((w[1] - w[0]) - interval).abs() > 1e-12),
        "the raw grid must not be the 250 us grid"
    );

    // The measurements are identical: they are computed on the raw grid.
    for name in ["vavg", "vmax"] {
        assert_eq!(
            measure_line(&grid.stdout, name),
            measure_line(&raw.stdout, name),
            "measure {name} must not move with output_interval"
        );
    }
    let vavg = measure_value(&grid.stdout, "vavg");
    assert!((vavg - 0.95).abs() < 1e-6, "avg = {vavg}, expected 0.95");
    let vmax = measure_value(&grid.stdout, "vmax");
    assert!(
        (vmax - (1.0 - (-20.0f64).exp())).abs() < 1e-6,
        "max = {vmax}, expected 0.9999999979"
    );
}
