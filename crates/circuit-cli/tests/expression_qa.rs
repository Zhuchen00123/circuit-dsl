//! Independent round-3 acceptance QA of result expressions, derived signals,
//! expression measurements, analysis binding and export.
//!
//! Every test drives the **real binary** and asserts numbers, exit codes or
//! file contents. Reference values are recomputed here from the component
//! values rather than copied from a neighbouring sample, and nothing is taken
//! on trust from `crates/circuit-cli/tests/e2e.rs`: that file is the
//! implementer's own acceptance record, this one is the independent check.
//!
//! Scratch material (sources and result files) lives under
//! `CARGO_TARGET_TMPDIR` - the cargo target dir - never in the repository.
//!
//! Contract: docs/review-evidence/round3/design-contract.md section 7.

use std::path::{Path, PathBuf};
use std::process::{Command, Output as ProcessOutput, Stdio};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn cdsl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cdsl"))
}

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str]) -> Run {
    let out: ProcessOutput = cdsl().args(args).output().expect("the binary runs");
    Run {
        status: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Run `cdsl repl` with `script` on stdin (the way `tests/repl.rs` does).
fn repl(args: &[&str], script: &str) -> Run {
    use std::io::Write;
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
    let out = child.wait_with_output().expect("wait");
    Run {
        status: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// A scratch directory unique to one test, under the cargo target dir.
fn scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("qa_{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn write_source(dir: &Path, name: &str, body: &str) -> PathBuf {
    let file = dir.join(name);
    std::fs::write(&file, body).expect("write source");
    file
}

fn path_of(p: &Path) -> String {
    p.display().to_string()
}

/// Files in `dir` (empty when it does not exist).
fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

// ---------------------------------------------------------------------------
// Numeric helpers
// ---------------------------------------------------------------------------

fn assert_close(got: f64, want: f64, tol: f64, what: &str) {
    assert!(
        (got - want).abs() <= tol,
        "{what}: got {got}, expected {want} (+-{tol})"
    );
}

/// Relative agreement, with an absolute floor for values near zero.
fn assert_rel(got: f64, want: f64, rel: f64, what: &str) {
    let scale = want.abs().max(1e-300);
    let err = (got - want).abs() / scale;
    assert!(
        err <= rel || (got - want).abs() <= 1e-15,
        "{what}: got {got}, expected {want} (rel err {err} > {rel})"
    );
}

/// `measure <name> = <value> <unit> (<analysis>)`, as the CLI prints it.
fn measure_line<'a>(stdout: &'a str, name: &str) -> &'a str {
    let prefix = format!("measure {name} = ");
    stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(prefix.as_str()))
        .unwrap_or_else(|| panic!("no measure {name} line in:\n{stdout}"))
}

fn measure_value(stdout: &str, name: &str) -> f64 {
    let prefix = format!("measure {name} = ");
    let line = measure_line(stdout, name);
    let rest = line.strip_prefix(prefix.as_str()).expect("prefix");
    rest.split_whitespace()
        .next()
        .expect("a number")
        .parse()
        .expect("parses")
}

/// The unit token of a measure line, e.g. `V*A` or `dimensionless`.
fn measure_unit<'a>(stdout: &'a str, name: &str) -> &'a str {
    let prefix = format!("measure {name} = ");
    let rest = measure_line(stdout, name)
        .strip_prefix(prefix.as_str())
        .expect("prefix");
    rest.split_whitespace().nth(1).expect("a unit")
}

/// The analysis identity in the trailing `(ac1)` of a measure line.
fn measure_analysis<'a>(stdout: &'a str, name: &str) -> &'a str {
    let line = measure_line(stdout, name);
    let open = line.rfind('(').expect("an analysis suffix");
    let close = line.rfind(')').expect("an analysis suffix");
    &line[open + 1..close]
}

// ---------------------------------------------------------------------------
// CSV
// ---------------------------------------------------------------------------

struct Table {
    header: Vec<String>,
    rows: Vec<Vec<f64>>,
    /// The field text, for checks that must be exact rather than numeric.
    raw_rows: Vec<Vec<String>>,
}

impl Table {
    fn read(path: &Path) -> Table {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let mut lines = text.lines();
        let header: Vec<String> = lines
            .next()
            .unwrap_or_else(|| panic!("{} is empty", path.display()))
            .split(',')
            .map(str::to_string)
            .collect();
        let mut rows = Vec::new();
        let mut raw_rows = Vec::new();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let fields: Vec<String> = line.split(',').map(str::to_string).collect();
            let row: Vec<f64> = fields
                .iter()
                .map(|f| {
                    f.parse::<f64>()
                        .unwrap_or_else(|_| panic!("not a number {f:?} in {}", path.display()))
                })
                .collect();
            assert_eq!(row.len(), header.len(), "ragged row in {}", path.display());
            rows.push(row);
            raw_rows.push(fields);
        }
        Table {
            header,
            rows,
            raw_rows,
        }
    }

    fn col(&self, name: &str) -> usize {
        self.header
            .iter()
            .position(|h| h == name)
            .unwrap_or_else(|| panic!("no column {name:?} in {:?}", self.header))
    }

    fn get(&self, row: usize, name: &str) -> f64 {
        let c = self.col(name);
        self.rows[row][c]
    }

    fn column(&self, name: &str) -> Vec<f64> {
        let c = self.col(name);
        self.rows.iter().map(|r| r[c]).collect()
    }

    fn len(&self) -> usize {
        self.rows.len()
    }

    /// The field exactly as the file wrote it.
    fn raw(&self, row: usize, name: &str) -> &str {
        let c = self.col(name);
        self.raw_rows[row][c].as_str()
    }
}

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

/// RC low-pass whose cut-off lands exactly on an AC sweep point:
/// C = 1/(2*pi*R*1000 Hz) makes Fc = 1 kHz, and 100 Hz -> 10 kHz with
/// 40 points per decade puts a sample exactly on 1 kHz (index 40).
const RC_AT_FC: &str = "\
circuit :rc do
  param :r, default: 1.kohm
  param :c, default: 159.15494309189535.nF
  node :vin, :vout
  voltage_source :input, p: :vin, n: :gnd, dc: 0.V, ac: 1.V
  resistor :r1, p: :vin, n: :vout, value: r
  capacitor :c1, p: :vout, n: :gnd, value: c
end
experiment :e, circuit: :rc do
  ac from: 100.Hz, to: 10.kHz, points_per_decade: 40
  save v(:vin), v(:vout)
  derive :gain, expr: v(:vout) / v(:vin)
  derive :gain_db, expr: gain_db(v(:vout), v(:vin))
  measure :peak_gain, max: abs(v(:vout) / v(:vin))
end
";

/// The component values of RC_AT_FC, recomputed here and not read back from
/// the simulation.
const R_OHM: f64 = 1_000.0;
const C_FARAD: f64 = 159.154_943_091_895_35e-9;

/// Fc = 1/(2*pi*R*C).
fn fc() -> f64 {
    1.0 / (2.0 * std::f64::consts::PI * R_OHM * C_FARAD)
}

/// The RC transient: 1 V step through 1 kohm into 100 nF (tau = 100 us), with
/// no `output_interval`, so the CSV is the **raw solver grid**.
const RC_TRAN_POWER: &str = "\
circuit :rc do
  param :r, default: 1.kohm
  param :c, default: 100.nF
  node :vin, :vout
  voltage_source :input, p: :vin, n: :gnd, dc: 0.V,
    waveform: pulse(low: 0.V, high: 1.V, delay: 0.s, rise: 1.ns, fall: 1.ns, width: 5.ms, period: 10.ms)
  resistor :r1, p: :vin, n: :vout, value: r
  capacitor :c1, p: :vout, n: :gnd, value: c
end
experiment :power, circuit: :rc do
  tran stop: 1.ms, max_step: 100.ns
  save v(:vin), v(:vout), i(:r1)
  measure :avg_power, avg: v(:vin, :vout) * i(:r1)
end
";

/// Two experiments that differ **only** in `output_interval`.
const FINE_COARSE: &str = "\
circuit :rc do
  param :r, default: 1.kohm
  param :c, default: 100.nF
  node :vin, :vout
  voltage_source :input, p: :vin, n: :gnd, dc: 0.V,
    waveform: pulse(low: 0.V, high: 1.V, delay: 0.s, rise: 1.ns, fall: 1.ns, width: 5.ms, period: 10.ms)
  resistor :r1, p: :vin, n: :vout, value: r
  capacitor :c1, p: :vout, n: :gnd, value: c
end
experiment :fine, circuit: :rc do
  tran stop: 1.ms, max_step: 100.ns, output_interval: 1.us
  save v(:vin), v(:vout)
  derive :vdiff, expr: v(:vin) - v(:vout)
  measure :vavg, avg: v(:vout)
end
experiment :coarse, circuit: :rc do
  tran stop: 1.ms, max_step: 100.ns, output_interval: 20.us
  save v(:vin), v(:vout)
  derive :vdiff, expr: v(:vin) - v(:vout)
  measure :vavg, avg: v(:vout)
end
experiment :raw, circuit: :rc do
  tran stop: 1.ms, max_step: 100.ns
  save v(:vin), v(:vout)
  derive :vdiff, expr: v(:vin) - v(:vout)
  measure :vavg, avg: v(:vout)
end
";

/// The same derive twice: once with only v(:vin) saved, once with both.
const SAVE_ONLY_VIN: &str = "\
circuit :rc do
  param :r, default: 1.kohm
  param :c, default: 100.nF
  node :vin, :vout
  voltage_source :input, p: :vin, n: :gnd, dc: 0.V,
    waveform: pulse(low: 0.V, high: 1.V, delay: 0.s, rise: 1.ns, fall: 1.ns, width: 5.ms, period: 10.ms)
  resistor :r1, p: :vin, n: :vout, value: r
  capacitor :c1, p: :vout, n: :gnd, value: c
end
experiment :e, circuit: :rc do
  tran stop: 100.us, max_step: 100.ns
  save v(:vin)
  derive :vdiff, expr: v(:vin) - v(:vout)
end
experiment :both, circuit: :rc do
  tran stop: 100.us, max_step: 100.ns
  save v(:vin), v(:vout)
  derive :vdiff, expr: v(:vin) - v(:vout)
end
";

/// op + ac with every result explicitly bound.
const TWO_BOUND: &str = "\
circuit :rc do
  param :r, default: 1.kohm
  param :c, default: 100.nF
  node :vin, :vout
  voltage_source :input, p: :vin, n: :gnd, dc: 1.V, ac: 1.V
  resistor :r1, p: :vin, n: :vout, value: r
  capacitor :c1, p: :vout, n: :gnd, value: c
end
experiment :two, circuit: :rc do
  op
  ac from: 100.Hz, to: 10.kHz, points_per_decade: 5
  save v(:vout)
  derive :g, expr: v(:vout) / v(:vin), analysis: :ac1
  derive :gdc, expr: v(:vout) / v(:vin), analysis: :op1
  measure :gm, max: abs(v(:vout) / v(:vin)), analysis: :ac1
  measure :vop, max: v(:vout), analysis: :op1
end
";

/// op + ac with the derive's binding omitted: must be ambiguous.
const AMBIGUOUS: &str = "\
circuit :rc do
  param :r, default: 1.kohm
  param :c, default: 100.nF
  node :vin, :vout
  voltage_source :input, p: :vin, n: :gnd, dc: 0.V, ac: 1.V
  resistor :r1, p: :vin, n: :vout, value: r
  capacitor :c1, p: :vout, n: :gnd, value: c
end
experiment :e, circuit: :rc do
  op
  ac from: 100.Hz, to: 10.kHz, points_per_decade: 5
  save v(:vout)
  derive :g, expr: v(:vout) / v(:vin)
end
";

/// Four analyses, one bare-probe measure: the legacy selection order is
/// TRAN -> AC -> DC -> OP.
const RANK_FOUR: &str = "\
circuit :rc do
  param :r, default: 1.kohm
  param :c, default: 100.nF
  node :vin, :vout
  voltage_source :input, p: :vin, n: :gnd, dc: 0.V, ac: 1.V,
    waveform: pulse(low: 0.V, high: 1.V, delay: 0.s, rise: 1.ns, fall: 1.ns, width: 5.ms, period: 10.ms)
  resistor :r1, p: :vin, n: :vout, value: r
  capacitor :c1, p: :vout, n: :gnd, value: c
end
experiment :e, circuit: :rc do
  op
  dc source: :input, from: 0.V, to: 1.V, step: 0.5.V
  ac from: 100.Hz, to: 10.kHz, points_per_decade: 5
  tran stop: 100.us, max_step: 100.ns
  save v(:vout)
  measure :vm, max: v(:vout)
end
";

/// op + dc: the legacy order must prefer the DC sweep over the operating point.
const RANK_DC_OP: &str = "\
circuit :div do
  node :in, :out
  voltage_source :v1, p: :in, n: :gnd, dc: 1.V
  resistor :r1, p: :in, n: :out, value: 1.kohm
  resistor :r2, p: :out, n: :gnd, value: 1.kohm
end
experiment :e, circuit: :div do
  op
  dc source: :v1, from: 1.V, to: 3.V, step: 1.V
  save v(:out)
  measure :vm, max: v(:out)
end
";

/// A single-parameter DC sweep: the derive is bound to the swept analysis.
const PARAM_SWEEP: &str = "\
circuit :swept do
  param :r, default: 1.kohm
  node :in, :out
  voltage_source :src, p: :in, n: :gnd, dc: 3.V
  resistor :r1, p: :in, n: :out, value: r
  resistor :r2, p: :out, n: :gnd, value: 1.5.kohm
end
experiment :sweep, circuit: :swept do
  dc param: :r, from: 0.5.kohm, to: 2.kohm, step: 0.5.kohm
  save v(:out)
  derive :scaled, expr: v(:out) * 2, analysis: :dc1
  measure :vmax, max: v(:out)
  measure :vmin, min: v(:out), analysis: :dc1
end
";

/// The same sweep with a binding to the operating point: a capability error
/// before the run, never a silent drop.
const SWEEP_WRONG_BINDING: &str = "\
circuit :swept do
  param :r, default: 1.kohm
  node :in, :out
  voltage_source :src, p: :in, n: :gnd, dc: 3.V
  resistor :r1, p: :in, n: :out, value: r
  resistor :r2, p: :out, n: :gnd, value: 1.5.kohm
end
experiment :sweep, circuit: :swept do
  op
  dc param: :r, from: 0.5.kohm, to: 2.kohm, step: 0.5.kohm
  save v(:out)
  derive :x, expr: v(:out) * 2, analysis: :op1
end
";

/// A plain two-node divider used by the negative cases.
fn divider_experiment(body: &str) -> String {
    format!(
        "circuit :div do
  node :in, :out
  voltage_source :v1, p: :in, n: :gnd, dc: 1.V
  resistor :r1, p: :in, n: :out, value: 1.kohm
  resistor :r2, p: :out, n: :gnd, value: 1.kohm
end
experiment :e, circuit: :div do
  op
  save v(:out)
{body}
end
"
    )
}

// ===========================================================================
// 7.1 / 7.9 - RC gain at the cut-off frequency
// ===========================================================================

#[test]
fn rc_gain_on_the_sample_that_lands_on_the_cut_off_frequency() {
    let dir = scratch("rc_fc");
    let file = write_source(&dir, "rc_fc.cdsl", RC_AT_FC);
    let out = run(&[
        "run",
        &path_of(&file),
        "--out",
        &path_of(&dir),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);

    let table = Table::read(&dir.join("e.ac1.csv"));
    // The derived complex signal keeps its two parts; the real one is scalar.
    for column in [
        "frequency",
        "v(vin)_re",
        "v(vout)_re",
        "gain_re",
        "gain_im",
        "gain_db",
    ] {
        table.col(column);
    }
    assert!(
        !table.header.iter().any(|h| h == "gain_db_im"),
        "a real derived signal must not grow an imaginary column: {:?}",
        table.header
    );

    // The sample that lands on Fc: the closest one, and it must really be Fc.
    let target = fc();
    let frequencies = table.column("frequency");
    let (index, _) = frequencies
        .iter()
        .enumerate()
        .min_by(|a, b| {
            (a.1 - target)
                .abs()
                .partial_cmp(&(b.1 - target).abs())
                .expect("no NaNs")
        })
        .expect("a frequency axis");
    let f = frequencies[index];
    assert!(
        ((f - target) / target).abs() < 1e-12,
        "the sample nearest Fc = {target} Hz is at {f} Hz, which is not Fc"
    );

    // Reference recomputed from the component values, not from a neighbour.
    let omega = 2.0 * std::f64::consts::PI * f;
    let rc = omega * R_OHM * C_FARAD;
    let expected_re = 1.0 / (1.0 + rc * rc);
    let expected_im = -rc / (1.0 + rc * rc);
    let expected_mag = 1.0 / (1.0 + rc * rc).sqrt();
    let expected_db = 20.0 * expected_mag.log10();

    let got_re = table.get(index, "gain_re");
    let got_im = table.get(index, "gain_im");
    let got_db = table.get(index, "gain_db");
    let got_mag = (got_re * got_re + got_im * got_im).sqrt();

    assert_close(
        table.get(index, "v(vin)_re"),
        1.0,
        1e-15,
        "|v(vin)| must be the 1 V AC source",
    );
    assert_rel(got_re, expected_re, 1e-9, "gain.re at Fc");
    assert_rel(got_im, expected_im, 1e-9, "gain.im at Fc");
    assert_rel(got_mag, expected_mag, 1e-9, "|gain| at Fc");
    assert_rel(got_db, expected_db, 1e-9, "gain_db at Fc");

    // The contract's frozen numbers, on this sample.
    assert_close(got_re, 0.5, 1e-9, "gain.re at Fc halved");
    assert_close(got_im, -0.5, 1e-9, "gain.im at Fc halved");
    // 1/sqrt(2) as the standard library defines it, so the literal cannot
    // drift from the constant clippy sees.
    assert_close(
        got_mag,
        std::f64::consts::FRAC_1_SQRT_2,
        1e-12,
        "|gain| = 1/sqrt(2)",
    );
    assert_close(got_db, -3.010_299_956_639_812, 1e-9, "gain_db = -3.0103 dB");
    // 0.70710678 and -3.01029996 at the printed precision of the contract.
    assert_close(
        got_mag,
        std::f64::consts::FRAC_1_SQRT_2,
        1e-8,
        "|gain| to 8 digits",
    );
    assert_close(got_db, -3.010_299_96, 1e-8, "gain_db to 8 digits");

    // gain_db(a, b) is 20*log10(|a|/|b|) for every sample, not only at Fc.
    for row in 0..table.len() {
        let re: f64 = table.get(row, "gain_re");
        let im: f64 = table.get(row, "gain_im");
        let mag = (re * re + im * im).sqrt();
        assert_rel(
            table.get(row, "gain_db"),
            20.0 * mag.log10(),
            1e-9,
            &format!("gain_db at row {row}"),
        );
    }

    // The measure reports the sweep maximum, which is at the lowest frequency.
    let measured = measure_value(&out.stdout, "peak_gain");
    let independent = frequencies
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let re = table.get(i, "gain_re");
            let im = table.get(i, "gain_im");
            (re * re + im * im).sqrt()
        })
        .fold(0.0_f64, f64::max);
    assert_rel(measured, independent, 1e-12, "peak_gain over the AC sweep");
    assert_eq!(measure_unit(&out.stdout, "peak_gain"), "dimensionless");
    assert_eq!(measure_analysis(&out.stdout, "peak_gain"), "ac1");
    assert_eq!(table.len(), 81, "100 Hz -> 10 kHz at 40 points/decade");
}

// ===========================================================================
// 7.2 - resistor power in a transient run
// ===========================================================================

#[test]
fn resistor_power_matches_an_independent_v_squared_over_r_integral() {
    let dir = scratch("tran_power");
    let file = write_source(&dir, "power.cdsl", RC_TRAN_POWER);
    let out = run(&[
        "run",
        &path_of(&file),
        "--out",
        &path_of(&dir),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);

    // No output_interval here, so the file is the raw solver grid.
    let table = Table::read(&dir.join("power.tran1.csv"));
    assert_eq!(table.header, ["time", "v(vin)", "v(vout)", "i(r1)"]);
    assert!(table.len() > 1_000, "a resolved transient: {}", table.len());

    let time = table.column("time");
    let vin = table.column("v(vin)");
    let vout = table.column("v(vout)");
    let ir1 = table.column("i(r1)");

    let measured = measure_value(&out.stdout, "avg_power");
    assert_eq!(measure_unit(&out.stdout, "avg_power"), "V*A");
    assert_eq!(measure_analysis(&out.stdout, "avg_power"), "tran1");

    // Independent reference: trapezoidal integral of v^2/R over the RAW grid,
    // then divided by the total width. Same sign convention as the DSL: the
    // resistor absorbs, v(vin, vout) = vin - vout > 0 while charging and
    // i(r1) > 0 down the p -> n branch, so P = +v^2/R.
    let mut area_v2r = 0.0_f64;
    let mut area_vi = 0.0_f64;
    let mut width = 0.0_f64;
    let mut worst_sample_diff = 0.0_f64;
    let mut min_product = f64::INFINITY;
    for i in 0..table.len() - 1 {
        let dt = time[i + 1] - time[i];
        width += dt;
        let across = [vin[i] - vout[i], vin[i + 1] - vout[i + 1]];
        let p_v2r = [across[0] * across[0] / R_OHM, across[1] * across[1] / R_OHM];
        let p_vi = [across[0] * ir1[i], across[1] * ir1[i + 1]];
        area_v2r += 0.5 * (p_v2r[0] + p_v2r[1]) * dt;
        area_vi += 0.5 * (p_vi[0] + p_vi[1]) * dt;
        worst_sample_diff = worst_sample_diff.max((p_vi[0] - p_v2r[0]).abs());
        min_product = min_product.min(p_vi[0]).min(p_vi[1]);
    }
    assert_close(width, 1e-3, 1e-12, "total width over the raw grid");
    let reference = area_v2r / width;
    let reference_from_vi = area_vi / width;

    assert_rel(measured, reference, 1e-12, "avg_power vs v^2/R");
    assert_rel(measured, reference_from_vi, 1e-12, "avg_power vs v*i");
    // The two independent readings of the same power agree sample by sample:
    // i(r1) really is v(vin, vout)/R.
    assert!(
        worst_sample_diff < 1e-15,
        "max |v*i - v^2/R| = {worst_sample_diff}"
    );
    // Sign, explicitly: the resistor absorbs, so no sample is negative and
    // the average is positive.
    assert!(
        min_product >= -1e-18,
        "the resistor must not generate power: min sample = {min_product}"
    );
    assert!(
        measured > 0.0,
        "the average power must be positive, got {measured}"
    );
    // The order of magnitude is the step response's V^2/(2R).
    assert_close(measured, 5e-5, 5e-7, "avg_power near V^2/(2R) for a step");
}

// ===========================================================================
// 7.3 - fine vs coarse output_interval
// ===========================================================================

#[test]
fn fine_and_coarse_output_interval_agree_on_measures_and_shared_samples() {
    let dir = scratch("fine_coarse");
    let file = write_source(&dir, "fine_coarse.cdsl", FINE_COARSE);
    let source = path_of(&file);

    let fine_dir = dir.join("fine");
    let coarse_dir = dir.join("coarse");
    let fine = run(&[
        "run",
        &source,
        "--experiment",
        "fine",
        "--out",
        &path_of(&fine_dir),
        "--format",
        "csv",
    ]);
    let coarse = run(&[
        "run",
        &source,
        "--experiment",
        "coarse",
        "--out",
        &path_of(&coarse_dir),
        "--format",
        "csv",
    ]);
    assert_eq!(fine.status, 0, "{}", fine.stderr);
    assert_eq!(coarse.status, 0, "{}", coarse.stderr);

    // Different exported point counts...
    let fine_table = Table::read(&fine_dir.join("fine.tran1.csv"));
    let coarse_table = Table::read(&coarse_dir.join("coarse.tran1.csv"));
    assert_eq!(fine_table.len(), 1_001, "1 ms / 1 us + 1");
    assert_eq!(coarse_table.len(), 51, "1 ms / 20 us + 1");

    // ...identical measure values, down to the rendered text...
    assert_eq!(
        measure_line(&fine.stdout, "vavg"),
        measure_line(&coarse.stdout, "vavg")
    );
    assert_close(
        measure_value(&fine.stdout, "vavg"),
        measure_value(&coarse.stdout, "vavg"),
        0.0,
        "avg must not move with the output grid",
    );
    // The measurement is the RAW-grid integral. A third experiment with no
    // output_interval exports exactly that grid, and a trapezoid over it must
    // reproduce the value the other two report.
    let raw_dir = dir.join("raw");
    let raw = run(&[
        "run",
        &source,
        "--experiment",
        "raw",
        "--out",
        &path_of(&raw_dir),
        "--format",
        "csv",
    ]);
    assert_eq!(raw.status, 0, "{}", raw.stderr);
    let raw_table = Table::read(&raw_dir.join("raw.tran1.csv"));
    assert_eq!(raw_table.len(), 10_018, "the solver's own grid");
    assert_ne!(raw_table.len(), fine_table.len());
    let raw_time = raw_table.column("time");
    let raw_vout = raw_table.column("v(vout)");
    let mut area = 0.0;
    let mut width = 0.0;
    for i in 0..raw_table.len() - 1 {
        let dt = raw_time[i + 1] - raw_time[i];
        width += dt;
        area += 0.5 * (raw_vout[i] + raw_vout[i + 1]) * dt;
    }
    let raw_average = area / width;
    assert_rel(
        measure_value(&fine.stdout, "vavg"),
        raw_average,
        1e-12,
        "vavg vs an independent trapezoid over the raw grid",
    );
    // The exported view is a resampling, not the measurement grid: averaging
    // the 1 us export gives a different (slightly lower) number, which is what
    // "measurements are computed before resampling" means in practice.
    let view_time = fine_table.column("time");
    let view_vout = fine_table.column("v(vout)");
    let mut view_area = 0.0;
    let mut view_width = 0.0;
    for i in 0..fine_table.len() - 1 {
        let dt = view_time[i + 1] - view_time[i];
        view_width += dt;
        view_area += 0.5 * (view_vout[i] + view_vout[i + 1]) * dt;
    }
    let view_average = view_area / view_width;
    let drift = (view_average - raw_average).abs() / raw_average;
    assert!(
        (1e-9..1e-5).contains(&drift),
        "avg over the exported view must differ from the raw integral (drift {drift})"
    );

    // ...and the derived samples agree at every shared time point.
    let coarse_time = coarse_table.column("time");
    let coarse_vdiff = coarse_table.column("vdiff");
    let fine_time = fine_table.column("time");
    let fine_vdiff = fine_table.column("vdiff");
    for (k, t) in coarse_time.iter().enumerate() {
        let hit = fine_time
            .iter()
            .position(|candidate| ((candidate - t) / t.max(1e-30)).abs() < 1e-15)
            .unwrap_or_else(|| panic!("coarse time {t} is not on the fine grid"));
        let delta = (coarse_vdiff[k] - fine_vdiff[hit]).abs();
        assert!(
            delta < 1e-12,
            "vdiff at t={t}: coarse {} vs fine {} (delta {delta})",
            coarse_vdiff[k],
            fine_vdiff[hit]
        );
    }
    // The derived signal is in both exports and has settled at the end.
    assert!(fine_table.header.iter().any(|h| h == "vdiff"));
    assert!(coarse_table.header.iter().any(|h| h == "vdiff"));
    assert!(fine_vdiff[fine_table.len() - 1] < 1e-3);
}

// ===========================================================================
// 7.4 - a read that was never saved
// ===========================================================================

#[test]
fn a_read_that_is_not_saved_still_evaluates_and_is_not_exported() {
    let dir = scratch("save_only_vin");
    let file = write_source(&dir, "save_only_vin.cdsl", SAVE_ONLY_VIN);
    let source = path_of(&file);

    let only_dir = dir.join("only");
    let both_dir = dir.join("both");
    let only = run(&[
        "run",
        &source,
        "--experiment",
        "e",
        "--out",
        &path_of(&only_dir),
        "--format",
        "csv",
    ]);
    let both = run(&[
        "run",
        &source,
        "--experiment",
        "both",
        "--out",
        &path_of(&both_dir),
        "--format",
        "csv",
    ]);
    assert_eq!(only.status, 0, "{}", only.stderr);
    assert_eq!(both.status, 0, "{}", both.stderr);

    let only_table = Table::read(&only_dir.join("e.tran1.csv"));
    let both_table = Table::read(&both_dir.join("both.tran1.csv"));

    // The implicit read is not exported...
    assert_eq!(only_table.header, ["time", "v(vin)", "vdiff"]);
    assert!(
        !only_table.header.iter().any(|h| h.contains("v(vout)")),
        "v(:vout) is read but was never saved: {:?}",
        only_table.header
    );
    // ...while the derived column it feeds is.
    assert!(only_table.header.iter().any(|h| h == "vdiff"));
    assert_eq!(both_table.header, ["time", "v(vin)", "v(vout)", "vdiff"]);

    // The implicit read really was evaluated: the derived column is
    // v(vin) - v(vout), checked against the run that does export v(vout).
    assert_eq!(only_table.len(), both_table.len());
    for row in 0..only_table.len() {
        let vin = both_table.get(row, "v(vin)");
        let vout = both_table.get(row, "v(vout)");
        let vdiff = both_table.get(row, "vdiff");
        assert_rel(vdiff, vin - vout, 1e-12, &format!("vdiff row {row}"));
        assert_rel(
            only_table.get(row, "vdiff"),
            vdiff,
            1e-12,
            &format!("the same vdiff with v(:vout) unsaved, row {row}"),
        );
    }
    // And it is not a degenerate column.
    let positive = only_table
        .column("vdiff")
        .into_iter()
        .filter(|v| *v > 0.5)
        .count();
    assert!(positive > 100, "only {positive} samples are charging");
}

// ===========================================================================
// 7.5 - binding a result to an analysis
// ===========================================================================

#[test]
fn explicit_bindings_select_the_analysis_and_report_it() {
    let dir = scratch("two_bound");
    let file = write_source(&dir, "two_bound.cdsl", TWO_BOUND);
    let out = run(&[
        "run",
        &path_of(&file),
        "--out",
        &path_of(&dir),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);

    assert_eq!(measure_analysis(&out.stdout, "gm"), "ac1");
    assert_eq!(measure_analysis(&out.stdout, "vop"), "op1");
    assert_eq!(measure_unit(&out.stdout, "gm"), "dimensionless");
    assert_close(measure_value(&out.stdout, "vop"), 1.0, 1e-12, "vop");

    // Each binding wrote into its own analysis file, with its own derived
    // signal, and the two are not interchangeable.
    let op = Table::read(&dir.join("two.op1.csv"));
    assert_eq!(op.header, ["v(vout)", "gdc"]);
    assert_close(op.get(0, "gdc"), 1.0, 1e-12, "gdc on the operating point");

    let ac = Table::read(&dir.join("two.ac1.csv"));
    assert_eq!(
        ac.header,
        ["frequency", "v(vout)_re", "v(vout)_im", "g_re", "g_im"]
    );
    // On AC the same expression is complex: g = v(vout)/v(vin) and v(vin)=1.
    for row in 0..ac.len() {
        assert_rel(
            ac.get(row, "g_re"),
            ac.get(row, "v(vout)_re"),
            1e-12,
            "g.re = v(vout).re / 1",
        );
        assert_rel(
            ac.get(row, "g_im"),
            ac.get(row, "v(vout)_im"),
            1e-12,
            "g.im = v(vout).im / 1",
        );
    }
    // The AC maximum of |H| is at the lowest frequency here.
    let peak = (0..ac.len())
        .map(|row| {
            let re = ac.get(row, "g_re");
            let im = ac.get(row, "g_im");
            (re * re + im * im).sqrt()
        })
        .fold(0.0_f64, f64::max);
    assert_rel(measure_value(&out.stdout, "gm"), peak, 1e-12, "gm");
}

#[test]
fn an_omitted_binding_in_a_multi_analysis_experiment_is_ambiguous() {
    let dir = scratch("ambiguous");
    let file = write_source(&dir, "ambiguous.cdsl", AMBIGUOUS);
    let out_dir = dir.join("out");
    let out = run(&[
        "run",
        &path_of(&file),
        "--out",
        &path_of(&out_dir),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 1, "an ambiguous binding is a user error");
    assert!(out.stderr.contains("E_AMBIGUOUS"), "{}", out.stderr);
    assert!(
        out.stderr
            .contains("could be evaluated on any of 2 analyses"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("available analyses: op1, ac1"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("add `analysis: :op1`"),
        "{}",
        out.stderr
    );
    assert!(
        !out.stdout.contains("wrote"),
        "nothing may be written:\n{}",
        out.stdout
    );
    assert!(
        files_in(&out_dir).is_empty(),
        "nothing may be written: {:?}",
        files_in(&out_dir)
    );

    // check refuses it too, without simulating anything.
    let checked = run(&["check", &path_of(&file)]);
    assert_eq!(checked.status, 1);
    assert!(checked.stderr.contains("E_AMBIGUOUS"), "{}", checked.stderr);
}

#[test]
fn an_unknown_analysis_id_lists_the_available_ones() {
    let dir = scratch("unknown_analysis");
    let file = write_source(
        &dir,
        "unknown_analysis.cdsl",
        &divider_experiment("  derive :g, expr: v(:out) * 2, analysis: :ac9"),
    );
    let out = run(&[
        "run",
        &path_of(&file),
        "--out",
        &path_of(&dir.join("out")),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("E_NAME"), "{}", out.stderr);
    assert!(
        out.stderr
            .contains("`ac9` is not an analysis of this experiment"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("available analyses: op1"),
        "the real ids must be listed:\n{}",
        out.stderr
    );
    assert!(
        out.stderr
            .contains("an analysis identity is `{kind}{ordinal}`"),
        "the identity syntax must be shown:\n{}",
        out.stderr
    );

    // A bare :ac without an ordinal is refused the same way, even when the
    // experiment really has an AC analysis.
    let file = write_source(
        &dir,
        "bare_ac.cdsl",
        &TWO_BOUND.replace(
            "  derive :g, expr: v(:vout) / v(:vin), analysis: :ac1",
            "  derive :g, expr: v(:vout) / v(:vin), analysis: :ac",
        ),
    );
    let out = run(&[
        "run",
        &path_of(&file),
        "--out",
        &path_of(&dir.join("bare")),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("E_NAME"), "{}", out.stderr);
    assert!(
        out.stderr
            .contains("`ac` is not an analysis of this experiment"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("available analyses: op1, ac1"),
        "{}",
        out.stderr
    );
}

#[test]
fn a_legacy_bare_probe_measure_keeps_the_transient_first_order() {
    let dir = scratch("legacy_order");

    // op + dc: the DC sweep wins (TRAN -> AC -> DC -> OP).
    let file = write_source(&dir, "rank_dc_op.cdsl", RANK_DC_OP);
    let out = run(&[
        "run",
        &path_of(&file),
        "--out",
        &path_of(&dir.join("dc_op")),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);
    assert_eq!(measure_analysis(&out.stdout, "vm"), "dc1");
    // 1 V, 2 V, 3 V into a 2:1 divider: the maximum is 1.5 V.
    assert_close(measure_value(&out.stdout, "vm"), 1.5, 1e-12, "vm on dc1");

    // op + dc + ac + tran: the transient wins.
    let file = write_source(&dir, "rank_four.cdsl", RANK_FOUR);
    let out = run(&[
        "run",
        &path_of(&file),
        "--out",
        &path_of(&dir.join("four")),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);
    assert_eq!(measure_analysis(&out.stdout, "vm"), "tran1");
    let measured = measure_value(&out.stdout, "vm");
    // The operating point would be 0 V and the DC sweep at most 1 V; the
    // transient peak of the step response is what must be reported.
    assert!(measured > 0.6, "vm = {measured} is not a transient peak");
    let tran = Table::read(&dir.join("four").join("e.tran1.csv"));
    assert_rel(
        measured,
        tran.column("v(vout)").into_iter().fold(f64::MIN, f64::max),
        1e-12,
        "vm vs the transient maximum",
    );
    // All four analyses still ran and were exported.
    assert_eq!(
        files_in(&dir.join("four")),
        ["e.ac1.csv", "e.dc1.csv", "e.op1.csv", "e.tran1.csv"]
    );
}

// ===========================================================================
// 7.9 - CSV and JSON export of derived signals
// ===========================================================================

#[test]
fn csv_and_json_keep_the_derived_complex_signal_and_its_analysis() {
    let dir = scratch("export");
    let file = write_source(&dir, "rc_fc.cdsl", RC_AT_FC);
    let out_dir = dir.join("out");
    let out = run(&[
        "run",
        &path_of(&file),
        "--out",
        &path_of(&out_dir),
        "--format",
        "both",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);
    assert_eq!(files_in(&out_dir), ["e.ac1.csv", "e.ac1.json"]);

    let csv = Table::read(&out_dir.join("e.ac1.csv"));
    let text = std::fs::read_to_string(out_dir.join("e.ac1.json")).expect("json");
    let json: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");

    // The analysis identity is in the JSON metadata and in the CSV file name.
    assert_eq!(json["analysis"], "ac1");
    assert_eq!(json["kind"], "ac");
    assert_eq!(json["experiment"], "e");
    assert_eq!(json["axis"]["type"], "frequency");
    assert_eq!(json["axis"]["unit"], "Hz");

    let signals = json["signals"].as_array().expect("signals array");
    let by_name = |name: &str| -> &serde_json::Value {
        signals
            .iter()
            .find(|s| s["name"] == name)
            .unwrap_or_else(|| panic!("no signal {name} in {signals:?}"))
    };

    // Real vs complex is kept, and so are the units.
    assert_eq!(by_name("v(vin)")["type"], "complex");
    assert_eq!(by_name("v(vin)")["unit"], "V");
    assert_eq!(by_name("gain")["type"], "complex");
    assert_eq!(by_name("gain")["unit"], "dimensionless");
    assert_eq!(by_name("gain_db")["type"], "real");
    assert_eq!(by_name("gain_db")["unit"], "dimensionless");

    // The axis is exported once and matches the CSV row count.
    let axis = json["axis"]["values"].as_array().expect("axis values");
    assert_eq!(axis.len(), csv.len());

    // Every value in the file matches the numbers the CSV carries, including
    // the imaginary part of a derived complex signal. The tolerance is 1e-12
    // rather than bit-exact on purpose: this test reads the JSON back with
    // serde_json, whose default (no ~B~float_roundtrip~B~) parser can land one
    // ulp away from the literal. The literals themselves are compared exactly
    // by text below, which is the check that matters.
    let gain = by_name("gain")["values"].as_array().expect("gain values");
    let gain_db = by_name("gain_db")["values"].as_array().expect("db values");
    for row in 0..csv.len() {
        assert_rel(
            axis[row].as_f64().expect("a number"),
            csv.get(row, "frequency"),
            1e-12,
            &format!("axis row {row}"),
        );
        assert_rel(
            gain[row]["re"].as_f64().expect("re"),
            csv.get(row, "gain_re"),
            1e-12,
            &format!("gain.re row {row}"),
        );
        assert_rel(
            gain[row]["im"].as_f64().expect("im"),
            csv.get(row, "gain_im"),
            1e-12,
            &format!("gain.im row {row}"),
        );
        assert_rel(
            gain_db[row].as_f64().expect("a real number"),
            csv.get(row, "gain_db"),
            1e-12,
            &format!("gain_db row {row}"),
        );
        // The two files describe one computation, so the derived column is
        // consistent with the complex one it came from.
        let re = csv.get(row, "gain_re");
        let im = csv.get(row, "gain_im");
        assert_rel(
            csv.get(row, "gain_db"),
            20.0 * (re * re + im * im).sqrt().log10(),
            1e-9,
            &format!("gain_db vs |gain| row {row}"),
        );
    }

    // The two exporters print the same decimal text for the same sample: the
    // JSON must literally contain the CSV's field, digit for digit.
    for row in [1usize, 10, 40] {
        for column in ["frequency", "gain_re", "gain_im", "gain_db"] {
            let field = csv.raw(row, column).to_string();
            assert!(
                field.len() >= 4,
                "picked a field too short to be distinctive: {field}"
            );
            assert!(
                text.contains(&field),
                "the JSON must carry the CSV field {field:?} for {column} at row {row}"
            );
        }
    }

    // The provenance of a derived signal is recorded, not invented later.
    let settings = json["backend"]["settings"]
        .as_array()
        .expect("backend settings");
    let provenance: Vec<&str> = settings
        .iter()
        .filter_map(|s| s["value"].as_str())
        .collect();
    assert!(
        provenance.iter().any(|v| v.contains("v(vout) / v(vin)")),
        "the derive expression should be recorded: {provenance:?}"
    );
}

// ===========================================================================
// 7.10 - file mode and the REPL agree
// ===========================================================================

#[test]
fn file_mode_and_the_repl_produce_the_same_values_and_the_same_error() {
    let dir = scratch("file_vs_repl");
    let file = write_source(&dir, "rc_fc.cdsl", RC_AT_FC);
    let source = path_of(&file);

    let file_run = run(&[
        "run",
        &source,
        "--out",
        &path_of(&dir.join("out")),
        "--format",
        "csv",
    ]);
    assert_eq!(file_run.status, 0, "{}", file_run.stderr);

    let script = format!("{RC_AT_FC}\n:run e\n:quit\n");
    let repl_run = repl(&[], &script);
    assert_eq!(repl_run.status, 0, "{}", repl_run.stderr);

    // The same numbers, from the same rendering.
    assert_eq!(
        measure_line(&file_run.stdout, "peak_gain"),
        measure_line(&repl_run.stdout, "peak_gain"),
        "file:\n{}\nrepl:\n{}",
        file_run.stdout,
        repl_run.stdout
    );
    assert!(
        file_run
            .stdout
            .contains("ac1: 81 frequency points; signals: v(vin), v(vout), gain, gain_db"),
        "{}",
        file_run.stdout
    );
    assert!(
        repl_run
            .stdout
            .contains("ac1: 81 frequency points; signals: v(vin), v(vout), gain, gain_db"),
        "{}",
        repl_run.stdout
    );

    // The same error message for the same bad input: the code and the notes
    // must match, only the source location differs (a file vs <repl:N>).
    let bad = write_source(&dir, "ambiguous.cdsl", AMBIGUOUS);
    let file_bad = run(&["run", &path_of(&bad), "--out", &path_of(&dir.join("bad"))]);
    assert_eq!(file_bad.status, 1);
    let repl_bad = repl(&[], &format!("{AMBIGUOUS}\n:run e\n:quit\n"));
    assert_eq!(repl_bad.status, 1);

    // The first error block, without the source location: file mode points at
    // the file and line, the REPL at <repl:N>. The REPL transcript then has a
    // second error ("no experiment named e"), because the refused definition
    // never became an experiment - correct behaviour, not part of the message
    // under test. The REPL reports diagnostics on stderr, file mode does too.
    let first_error = |text: &str| -> String {
        let mut out = Vec::new();
        let mut started = false;
        for line in text.lines() {
            if line.starts_with("error[") {
                if started {
                    break;
                }
                started = true;
            }
            if started && (line.starts_with("error[") || line.trim_start().starts_with("= ")) {
                out.push(line.trim().to_string());
            }
        }
        out.join("\n")
    };
    assert_eq!(
        first_error(&file_bad.stderr),
        first_error(&repl_bad.stderr),
        "file:\n{}\nrepl:\n{}",
        file_bad.stderr,
        repl_bad.stderr
    );
    assert!(
        first_error(&file_bad.stderr).contains("E_AMBIGUOUS"),
        "{}",
        file_bad.stderr
    );
}

// ===========================================================================
// 7.11 - single-parameter DC sweep
// ===========================================================================

#[test]
fn a_parameter_sweep_binds_to_the_stitched_dataset() {
    let dir = scratch("param_sweep");
    let file = write_source(&dir, "sweep.cdsl", PARAM_SWEEP);
    let out_dir = dir.join("out");
    let out = run(&[
        "run",
        &path_of(&file),
        "--out",
        &path_of(&out_dir),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);

    // One stitched dataset, addressed as the experiment's dc1.
    assert_eq!(files_in(&out_dir), ["sweep.dc_param_r.csv"]);
    assert_eq!(measure_analysis(&out.stdout, "vmax"), "dc_param_r");
    assert_eq!(measure_analysis(&out.stdout, "vmin"), "dc_param_r");

    let table = Table::read(&out_dir.join("sweep.dc_param_r.csv"));
    assert_eq!(table.header, ["parameter", "v(out)", "scaled"]);
    assert_eq!(table.len(), 4);

    // v(out) = 3 V * 1.5k / (r + 1.5k), recomputed here; the derived column is
    // exactly twice it.
    for (r, expected) in [
        (500.0, 2.25),
        (1_000.0, 1.8),
        (1_500.0, 1.5),
        (2_000.0, 9.0 / 7.0),
    ] {
        let row = table
            .rows
            .iter()
            .position(|row| (row[table.col("parameter")] - r).abs() < 1e-9)
            .unwrap_or_else(|| panic!("no sweep point at r = {r}"));
        assert_rel(
            table.get(row, "v(out)"),
            expected,
            1e-9,
            &format!("v(out) at r = {r}"),
        );
        assert_rel(
            table.get(row, "scaled"),
            2.0 * table.get(row, "v(out)"),
            1e-12,
            &format!("scaled = 2 * v(out) at r = {r}"),
        );
    }
    assert_close(measure_value(&out.stdout, "vmax"), 2.25, 1e-12, "vmax");
    assert_close(measure_value(&out.stdout, "vmin"), 9.0 / 7.0, 1e-12, "vmin");

    // The legacy measurement has no analysis either: the single stitched
    // dataset is still the only candidate.
    assert!(
        out.stdout.contains("dc_param_r: 4 sweep points"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_binding_to_a_non_swept_analysis_is_refused_before_the_run() {
    let dir = scratch("sweep_wrong");
    let file = write_source(&dir, "sweep_wrong.cdsl", SWEEP_WRONG_BINDING);
    let out_dir = dir.join("out");
    let out = run(&[
        "run",
        &path_of(&file),
        "--out",
        &path_of(&out_dir),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("E_UNSUPPORTED"), "{}", out.stderr);
    assert!(
        out.stderr
            .contains("this experiment sweeps the parameter `r` and only the swept DC analysis"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("bind it to `dc1`"),
        "the message must say what to do:\n{}",
        out.stderr
    );
    assert!(
        !out.stdout.contains("wrote"),
        "the run must not start:\n{}",
        out.stdout
    );
    assert!(!out_dir.exists(), "no output directory may be created");
}

// ===========================================================================
// 7.7 / 7.8 - negative cases, each with its code and exit status
// ===========================================================================

/// Run a one-experiment source and return the CLI result and its output dir.
fn run_inline(dir: &Path, name: &str, body: &str) -> (Run, PathBuf) {
    let file = write_source(dir, name, body);
    let out_dir = dir.join(format!("{name}.out"));
    let out = run(&[
        "run",
        &path_of(&file),
        "--out",
        &path_of(&out_dir),
        "--format",
        "csv",
    ]);
    (out, out_dir)
}

#[test]
fn a_unit_mismatch_is_a_dimension_error() {
    let dir = scratch("neg_dimension");
    let (out, out_dir) = run_inline(
        &dir,
        "dim.cdsl",
        &divider_experiment("  derive :bad, expr: v(:out) + i(:r1)"),
    );
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("E_DIMENSION"), "{}", out.stderr);
    assert!(
        out.stderr
            .contains("cannot apply `+` to `v(out)` (V) and `i(r1)` (A)"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("the units differ"), "{}", out.stderr);
    assert!(!out.stdout.contains("wrote"));
    assert!(!out_dir.exists());

    // It is a check-stage error: check refuses it and simulates nothing.
    let file = dir.join("dim.cdsl");
    let checked = run(&["check", &path_of(&file)]);
    assert_eq!(checked.status, 1);
    assert!(checked.stderr.contains("E_DIMENSION"), "{}", checked.stderr);
    assert!(!checked.stdout.contains("backend"), "{}", checked.stdout);
}

#[test]
fn a_division_by_zero_at_run_time_is_a_value_error_and_writes_nothing() {
    let dir = scratch("neg_divzero");
    let (out, out_dir) = run_inline(
        &dir,
        "divzero.cdsl",
        &divider_experiment("  measure :bad, max: v(:out) / 0"),
    );
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("E_VALUE"), "{}", out.stderr);
    assert!(
        out.stderr.contains("division by zero in `(v(out) / 0)`"),
        "{}",
        out.stderr
    );
    // The contract requires analysis, signal and sample index.
    assert!(out.stderr.contains("at sample 0"), "{}", out.stderr);
    assert!(out.stderr.contains("analysis `op1`"), "{}", out.stderr);
    assert!(
        out.stderr.contains("no epsilon is applied"),
        "{}",
        out.stderr
    );
    assert!(
        !out.stdout.contains("measure bad"),
        "a failed measure must never be printed as a value:\n{}",
        out.stdout
    );
    assert!(!out.stdout.contains("wrote"), "{}", out.stdout);
    assert!(
        !out_dir.exists(),
        "no partial export: {} exists",
        out_dir.display()
    );
}

#[test]
fn gain_db_with_a_zero_amplitude_is_a_value_error() {
    let dir = scratch("neg_dbzero");
    let source = "\
circuit :div do
  node :in, :out
  voltage_source :v1, p: :in, n: :gnd, dc: 0.V
  resistor :r1, p: :in, n: :out, value: 1.kohm
  resistor :r2, p: :out, n: :gnd, value: 1.kohm
end
experiment :e, circuit: :div do
  op
  save v(:out)
  measure :db, max: gain_db(v(:in), v(:out))
end
";
    let (out, out_dir) = run_inline(&dir, "dbzero.cdsl", source);
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("E_VALUE"), "{}", out.stderr);
    assert!(
        out.stderr
            .contains("division by zero in `(v(in) / v(out))`"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("`v(out)` is 0"),
        "the zero operand must be named:\n{}",
        out.stderr
    );
    assert!(!out_dir.exists(), "no export on failure");
}

#[test]
fn an_unavailable_current_probe_is_a_backend_error() {
    let dir = scratch("neg_current");
    let source = "\
circuit :div do
  node :in, :out
  voltage_source :v1, p: :in, n: :gnd, dc: 1.V
  capacitor :c1, p: :in, n: :out, value: 1.uF
  resistor :r2, p: :out, n: :gnd, value: 1.kohm
end
experiment :e, circuit: :div do
  op
  save v(:out)
  measure :ic, max: i(:c1)
end
";
    let (out, out_dir) = run_inline(&dir, "current.cdsl", source);
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("E_BACKEND"), "{}", out.stderr);
    assert!(
        out.stderr.contains("no branch current reported for `c1`"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains(
            "the engine reports branch currents only for elements that own a branch unknown"
        ),
        "the reason must be explained:\n{}",
        out.stderr
    );
    assert!(!out_dir.exists(), "no export on failure");

    // An unknown device is a different, check-stage error.
    let (out, _) = run_inline(
        &dir,
        "unknown_dev.cdsl",
        &divider_experiment("  derive :x, expr: i(:nope)"),
    );
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("E_NAME"), "{}", out.stderr);
    assert!(
        out.stderr.contains("unknown device `:nope`"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("devices: v1, r1, r2"), "{}", out.stderr);
}

#[test]
fn an_unknown_node_is_a_name_error() {
    let dir = scratch("neg_node");
    let (out, _) = run_inline(
        &dir,
        "node.cdsl",
        &divider_experiment("  derive :x, expr: v(:nope) * 2"),
    );
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("E_NAME"), "{}", out.stderr);
    assert!(
        out.stderr.contains("unknown node `:nope`"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("nodes: in, out"), "{}", out.stderr);
}

#[test]
fn max_of_an_ac_signal_is_refused_with_an_abs_hint() {
    let dir = scratch("neg_complex");
    let source = "\
circuit :rc do
  param :r, default: 1.kohm
  param :c, default: 100.nF
  node :vin, :vout
  voltage_source :input, p: :vin, n: :gnd, dc: 0.V, ac: 1.V
  resistor :r1, p: :vin, n: :vout, value: r
  capacitor :c1, p: :vout, n: :gnd, value: c
end
experiment :e, circuit: :rc do
  ac from: 100.Hz, to: 10.kHz, points_per_decade: 5
  save v(:vout)
  measure :m, max: v(:vout), analysis: :ac1
end
";
    let (out, out_dir) = run_inline(&dir, "complex.cdsl", source);
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("E_TYPE"), "{}", out.stderr);
    assert!(
        out.stderr.contains("not ordered"),
        "a complex number has no ordering:\n{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("samples of `ac1` are complex"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("apply `abs(...)` first"),
        "the note must point at abs():\n{}",
        out.stderr
    );
    assert!(!out.stdout.contains("wrote"));
    assert!(!out_dir.exists());

    // The advice works: abs(...) makes the reduction legal.
    let fixed = write_source(
        &dir,
        "complex_fixed.cdsl",
        &source.replace("max: v(:vout)", "max: abs(v(:vout))"),
    );
    let out = run(&[
        "run",
        &path_of(&fixed),
        "--out",
        &path_of(&dir.join("fixed")),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);
    // |H| at 100 Hz for R = 1 kohm and C = 100 nF, from the components.
    let w = 2.0 * std::f64::consts::PI * 100.0;
    let expected = 1.0 / (1.0 + (w * 1_000.0 * 100e-9).powi(2)).sqrt();
    assert_close(
        measure_value(&out.stdout, "m"),
        expected,
        1e-9,
        "|H| at 100 Hz",
    );
    assert_eq!(measure_analysis(&out.stdout, "m"), "ac1");
}

#[test]
fn avg_on_an_analysis_without_a_time_axis_is_a_type_error() {
    let dir = scratch("neg_avg");
    let (out, out_dir) = run_inline(
        &dir,
        "avg_op.cdsl",
        &divider_experiment("  measure :m, avg: v(:out), analysis: :op1"),
    );
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("E_TYPE"), "{}", out.stderr);
    assert!(
        out.stderr
            .contains("`avg` needs a time axis, but analysis `op1` has none"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("bind it to a transient analysis"),
        "the advice must be actionable:\n{}",
        out.stderr
    );
    assert!(!out_dir.exists());

    // rms behaves the same way.
    let (out, _) = run_inline(
        &dir,
        "rms_op.cdsl",
        &divider_experiment("  measure :m, rms: v(:out), analysis: :op1"),
    );
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("E_TYPE"), "{}", out.stderr);
    assert!(
        out.stderr.contains("`rms` needs a time axis"),
        "{}",
        out.stderr
    );
}

#[test]
fn a_duplicate_derive_name_is_refused_at_check_time() {
    let dir = scratch("neg_duplicate");
    let (out, out_dir) = run_inline(
        &dir,
        "dup.cdsl",
        &divider_experiment("  derive :same, expr: v(:out) * 2\n  derive :same, expr: v(:out) * 3"),
    );
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("E_DUPLICATE"), "{}", out.stderr);
    assert!(
        out.stderr
            .contains("`same` is already defined as a derived signal"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("first defined here"),
        "the first definition must be pointed at:\n{}",
        out.stderr
    );
    assert!(!out_dir.exists());

    // The same namespace covers measurements: a measure may not reuse a
    // derived name either.
    let (out, _) = run_inline(
        &dir,
        "derive_measure.cdsl",
        &divider_experiment("  derive :same, expr: v(:out) * 2\n  measure :same, max: v(:out)"),
    );
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("E_DUPLICATE"), "{}", out.stderr);
    assert!(
        out.stderr
            .contains("`same` is already defined as a derived signal"),
        "{}",
        out.stderr
    );
}

/// The contract's "a derived name colliding with a saved signal" row.
///
/// On the surface this cannot happen: a probe is always named v(...) or i(...),
/// and the parser refuses a derive name that is not a bare symbol, so no derive
/// name can ever equal a saved probe name. The guard in
/// circuit-dsl/src/elaborate.rs is therefore unreachable from the language;
/// what *is* reachable is the collision inside the result namespace, checked
/// above. Both halves are pinned here so the report says which one the surface
/// actually enforces.
#[test]
fn a_derive_name_can_never_collide_with_a_saved_probe() {
    let dir = scratch("neg_collision");
    let (out, out_dir) = run_inline(
        &dir,
        "probe_name.cdsl",
        &divider_experiment("  derive :v(out), expr: v(:out) * 2"),
    );
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("E_SYNTAX"), "{}", out.stderr);
    assert!(
        out.stderr.contains("derive :name"),
        "the parser must say what a derive name looks like:\n{}",
        out.stderr
    );
    assert!(!out_dir.exists());

    // With a legal bare name, a saved probe with the same *text* stays itself:
    // the export keeps v(gain) and gain as two distinct columns.
    let source = "\
circuit :div do
  node :in, :gain
  voltage_source :v1, p: :in, n: :gnd, dc: 2.V
  resistor :r1, p: :in, n: :gain, value: 1.kohm
  resistor :r2, p: :gain, n: :gnd, value: 1.kohm
end
experiment :e, circuit: :div do
  op
  save v(:gain)
  derive :gain, expr: v(:gain) * 2
end
";
    let fixed = write_source(&dir, "distinct.cdsl", source);
    let out = run(&[
        "run",
        &path_of(&fixed),
        "--out",
        &path_of(&dir.join("distinct")),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);
    let table = Table::read(&dir.join("distinct").join("e.op1.csv"));
    assert_eq!(table.header, ["v(gain)", "gain"]);
    assert_close(table.get(0, "v(gain)"), 1.0, 1e-12, "v(gain)");
    assert_close(table.get(0, "gain"), 2.0, 1e-12, "gain = 2 * v(gain)");
}

// ===========================================================================
// 7.12 - the pre-existing behaviour still holds
// ===========================================================================

/// The round-2 transient invariants are owned by other crates' tests; this is
/// the round-3 read of the same rule through the CLI: a *negative*
/// output_interval is refused. Fine/coarse agreement is checked above, and the
/// raw-grid regressions live in
/// crates/circuit-session/tests/tran_output_interval.rs and
/// crates/circuit-backend/tests/output_interval_regression.rs - referenced,
/// not duplicated.
#[test]
fn a_negative_output_interval_is_still_refused() {
    let dir = scratch("negative_interval");
    let source = RC_TRAN_POWER.replace(
        "tran stop: 1.ms, max_step: 100.ns",
        "tran stop: 1.ms, max_step: 100.ns, output_interval: -1.us",
    );
    let (out, out_dir) = run_inline(&dir, "negative_interval.cdsl", &source);
    assert_eq!(out.status, 1, "{}", out.stdout);
    assert!(out.stderr.contains("E_"), "{}", out.stderr);
    assert!(
        out.stderr.to_lowercase().contains("interval"),
        "the diagnostic must name the option:\n{}",
        out.stderr
    );
    assert!(!out_dir.exists());
}
