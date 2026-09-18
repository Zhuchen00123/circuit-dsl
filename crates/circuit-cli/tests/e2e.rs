//! End-to-end CLI tests: run the real binary against the real examples.
//!
//! These are the "at least one CLI end-to-end execution" the brief asks for,
//! and they check numeric output rather than merely that the process exited
//! zero — a run that writes nothing, or writes the wrong numbers, must fail
//! here.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The example directory, relative to this crate.
fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
}

fn cdsl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cdsl"))
}

struct Output {
    status: i32,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str]) -> Output {
    let out = cdsl().args(args).output().expect("the binary runs");
    Output {
        status: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// A scratch directory unique to one test.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cdsl_e2e_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn example(name: &str) -> String {
    examples().join(name).display().to_string()
}

// ---------------------------------------------------------------------------
// check
// ---------------------------------------------------------------------------

#[test]
fn check_accepts_every_example() {
    for name in [
        "voltage_divider.cdsl",
        "rc_filter.cdsl",
        "rlc.cdsl",
        "two_stage.cdsl",
        "diode_rectifier.cdsl",
        "parameter_sweep.cdsl",
        "ladder.cdsl",
    ] {
        let out = run(&["check", &example(name)]);
        assert_eq!(out.status, 0, "check failed for {name}:\n{}", out.stderr);
        assert!(
            out.stdout.contains("circuit"),
            "check produced no summary for {name}:\n{}",
            out.stdout
        );
    }
}

#[test]
fn check_json_describes_the_circuit() {
    let out = run(&["check", &example("voltage_divider.cdsl"), "--json"]);
    assert_eq!(out.status, 0, "{}", out.stderr);
    let parsed: serde_json::Value =
        serde_json::from_str(&out.stdout).expect("check --json must emit valid JSON");

    let devices = parsed["circuits"][0]["devices"]
        .as_array()
        .expect("devices array");
    assert_eq!(devices.len(), 3);
    let names: Vec<&str> = devices
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"r1"), "{names:?}");

    let experiment = &parsed["experiments"][0];
    assert_eq!(experiment["name"], "divider");
    assert_eq!(experiment["analyses"][0]["kind"], "op");
}

/// A wrong dimension must be reported with a location, and `check` must not
/// simulate anything.
#[test]
fn check_reports_a_dimension_error_with_a_location() {
    let dir = scratch("dims");
    let file = dir.join("bad.cdsl");
    std::fs::write(
        &file,
        "circuit :bad do\n  node :a, :b\n  resistor :r1, p: :a, n: :b, value: 10.ms\nend\n",
    )
    .expect("write");

    let out = run(&["check", &file.display().to_string()]);
    assert_eq!(out.status, 1, "a user error must exit 1");
    // The diagnostic must name the code, both dimensions, the location, and
    // underline the offending token.
    assert!(out.stderr.contains("E_DIMENSION"), "{}", out.stderr);
    assert!(out.stderr.contains("= expected: ohm"), "{}", out.stderr);
    assert!(out.stderr.contains("= received: s"), "{}", out.stderr);
    assert!(out.stderr.contains("bad.cdsl:3"), "{}", out.stderr);
    assert!(out.stderr.contains("^^^^^"), "{}", out.stderr);
    // The source line is reproduced, so the caret has something to sit under.
    assert!(out.stderr.contains("resistor :r1"), "{}", out.stderr);
    // Nothing was simulated.
    assert!(!out.stdout.contains("wrote"), "{}", out.stdout);
}

/// A node reachable only through a capacitor has no defined operating point.
///
/// The engine's own failure for this topology is an unlocated singular-matrix
/// error that names no node, so the front end catches it and reports the node
/// together with the blocking capacitor. This is the case the brief singles
/// out: "a capacitor path is not a DC reference path".
#[test]
fn check_rejects_a_node_with_no_dc_path_to_ground() {
    let dir = scratch("floating");
    let file = dir.join("floating.cdsl");
    std::fs::write(
        &file,
        "circuit :ac_coupled do
  node :in, :out
  voltage_source :v1, p: :in, n: :gnd, dc: 1.V
  capacitor :c1, p: :in, n: :out, value: 1.uF
end
",
    )
    .expect("write");

    let out = run(&["check", &file.display().to_string()]);
    assert_eq!(
        out.status, 1,
        "an undetermined node must be an error:
{}",
        out.stdout
    );
    assert!(
        out.stderr.contains("no DC path to ground"),
        "{}",
        out.stderr
    );
    // The location points at the node declaration.
    assert!(out.stderr.contains("floating.cdsl:2"), "{}", out.stderr);
    // The device that blocks the path must be named.
    assert!(out.stderr.contains("c1"), "{}", out.stderr);
}

/// The same topology with a bias resistor is a normal AC-coupled stage and
/// must be accepted, so the check is not simply "does anything connect?".
#[test]
fn check_accepts_an_ac_coupled_stage_with_a_bias_resistor() {
    let dir = scratch("bias");
    let file = dir.join("bias.cdsl");
    std::fs::write(
        &file,
        "circuit :ac_coupled do
  node :in, :out
  voltage_source :v1, p: :in, n: :gnd, dc: 0.V, ac: 1.V
  capacitor :c1, p: :in, n: :out, value: 1.uF
  resistor :rb, p: :out, n: :gnd, value: 100.kohm
end
experiment :e, circuit: :ac_coupled do
  ac from: 1.Hz, to: 1.kHz, points_per_decade: 5
  save v(:out)
end
",
    )
    .expect("write");

    let out = run(&["check", &file.display().to_string()]);
    assert_eq!(
        out.status, 0,
        "a biased coupling node is fine:
{}",
        out.stderr
    );
}

#[test]
fn check_reports_a_node_nothing_connects_to() {
    let dir = scratch("unused");
    let file = dir.join("unused.cdsl");
    std::fs::write(
        &file,
        "circuit :c do
  node :a, :spare
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  resistor :r1, p: :a, n: :gnd, value: 1.kohm
end
",
    )
    .expect("write");

    let out = run(&["check", &file.display().to_string()]);
    assert_eq!(out.status, 1);
    assert!(
        out.stderr.contains("declared but nothing connects to it"),
        "{}",
        out.stderr
    );
}

#[test]
fn a_missing_file_is_a_user_error() {
    let out = run(&["check", "does_not_exist.cdsl"]);
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("E_IO"), "{}", out.stderr);
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

/// The headline acceptance case: a divider, checked against Ohm's law
/// including the sign of the current.
#[test]
fn run_divider_produces_the_correct_voltages_and_currents() {
    let dir = scratch("divider");
    let out = run(&[
        "run",
        &example("voltage_divider.cdsl"),
        "--experiment",
        "divider",
        "--out",
        &dir.display().to_string(),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);

    let csv = std::fs::read_to_string(dir.join("divider.op1.csv")).expect("result file");
    let mut lines = csv.lines();
    let header = lines.next().expect("header");
    assert_eq!(header, "v(in),v(out),i(r1),i(v1)");
    let row: Vec<f64> = lines
        .next()
        .expect("one row")
        .split(',')
        .map(|f| f.parse().expect("a number"))
        .collect();
    assert_eq!(row.len(), 4);

    // 5 V across 1k + 1.5k.
    assert!((row[0] - 5.0).abs() < 1e-9, "v(in) = {}", row[0]);
    assert!((row[1] - 3.0).abs() < 1e-9, "v(out) = {}", row[1]);
    // The resistor absorbs, so its p -> n current is positive.
    assert!((row[2] - 2e-3).abs() < 1e-9, "i(r1) = {}", row[2]);
    // The source delivers, so its p -> n current is negative.
    assert!((row[3] + 2e-3).abs() < 1e-9, "i(v1) = {}", row[3]);
    assert!(
        (row[2] + row[3]).abs() < 1e-12,
        "KCL: i(r1) + i(v1) = {}",
        row[2] + row[3]
    );
}

/// The RC transient, against the closed-form step response, plus the
/// non-uniform time axis and the integral-based measurements.
#[test]
fn run_rc_filter_matches_the_analytic_response() {
    let dir = scratch("rc");
    let out = run(&[
        "run",
        &example("rc_filter.cdsl"),
        "--experiment",
        "response",
        "--out",
        &dir.display().to_string(),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);

    // --- transient -----------------------------------------------------
    let csv = std::fs::read_to_string(dir.join("response.tran1.csv")).expect("tran result");
    let mut lines = csv.lines();
    let header: Vec<&str> = lines.next().unwrap().split(',').collect();
    let tcol = header
        .iter()
        .position(|h| *h == "time")
        .expect("time column");
    let vcol = header
        .iter()
        .position(|h| *h == "v(vout)")
        .expect("v(vout) column");

    let mut times = Vec::new();
    let mut volts = Vec::new();
    for line in lines {
        let f: Vec<&str> = line.split(',').collect();
        times.push(f[tcol].parse::<f64>().expect("time"));
        volts.push(f[vcol].parse::<f64>().expect("voltage"));
    }
    assert!(times.len() > 100, "expected a resolved transient");

    // tau = 1k * 100n = 100 us.
    let tau = 1e-4;
    for frac in [0.5, 1.0, 2.0, 3.0] {
        let target = frac * tau;
        let idx = times
            .iter()
            .enumerate()
            .min_by(|a, b| {
                (a.1 - target)
                    .abs()
                    .partial_cmp(&(b.1 - target).abs())
                    .unwrap()
            })
            .map(|(i, _)| i)
            .unwrap();
        let expected = 1.0 - (-times[idx] / tau).exp();
        assert!(
            (volts[idx] - expected).abs() < 1e-2,
            "at t={:.3}us ({frac} tau): got {}, expected {expected}",
            times[idx] * 1e6,
            volts[idx]
        );
    }

    // The axis is solver-chosen, so `avg`/`rms` must integrate over time.
    let mut non_uniform = false;
    for w in times.windows(3) {
        if ((w[1] - w[0]) - (w[2] - w[1])).abs() > 1e-12 {
            non_uniform = true;
            break;
        }
    }
    assert!(non_uniform, "the time axis should not be uniform");

    // --- AC ------------------------------------------------------------
    let ac = std::fs::read_to_string(dir.join("response.ac1.csv")).expect("ac result");
    let mut ac_lines = ac.lines();
    let ac_header: Vec<&str> = ac_lines.next().unwrap().split(',').collect();
    let fcol = ac_header.iter().position(|h| *h == "frequency").unwrap();
    let recol = ac_header
        .iter()
        .position(|h| *h == "v(vout)_re")
        .expect("complex columns are split into _re/_im");
    let imcol = ac_header.iter().position(|h| *h == "v(vout)_im").unwrap();

    // Corner frequency of 1k / 100n.
    let fc = 1.0 / (2.0 * std::f64::consts::PI * 1e3 * 100e-9);
    let mut best = None;
    for line in ac_lines {
        let f: Vec<&str> = line.split(',').collect();
        let freq: f64 = f[fcol].parse().unwrap();
        let re: f64 = f[recol].parse().unwrap();
        let im: f64 = f[imcol].parse().unwrap();
        if best
            .map(|(d, _, _, _, _)| (freq - fc).abs() < d)
            .unwrap_or(true)
        {
            best = Some(((freq - fc).abs(), freq, re, im, ()));
        }
    }
    let (_, freq, re, im, ()) = best.expect("a point near the corner");
    let w = 2.0 * std::f64::consts::PI * freq;
    let rc = w * 1e3 * 100e-9;
    let expected_re = 1.0 / (1.0 + rc * rc);
    let expected_im = -rc / (1.0 + rc * rc);
    assert!(
        (re - expected_re).abs() < 1e-9 && (im - expected_im).abs() < 1e-9,
        "at {freq} Hz: got {re}{im}j, expected {expected_re}{expected_im}j"
    );

    // --- measurements ---------------------------------------------------
    // vfinal is the transient peak; a measure that silently used the
    // operating point would report 0.
    assert!(
        out.stdout.contains("measure vfinal = 0.9"),
        "the peak of the step response should be near 1 V:\n{}",
        out.stdout
    );
    // avg of a settling step over 5 tau is 1 - (1/5)(1 - e^-5) = 0.8013.
    assert!(
        out.stdout.contains("measure vavg = 0.80"),
        "avg should be about 0.80 V:\n{}",
        out.stdout
    );
}

/// The whole language at once: subcircuits, hierarchy, a loop with computed
/// names, a conditional, and a per-instance parameter override.
#[test]
fn run_two_stage_exercises_the_composition_features() {
    let dir = scratch("twostage");
    let out = run(&[
        "run",
        &example("two_stage.cdsl"),
        "--experiment",
        "response",
        "--out",
        &dir.display().to_string(),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);

    let csv = std::fs::read_to_string(dir.join("response.ac1.csv")).expect("result");
    let header = csv.lines().next().unwrap();
    for signal in ["v(vin)", "v(mid)", "v(out)"] {
        assert!(header.contains(signal), "missing {signal} in {header}");
    }
    // Two cascaded low-pass stages: the second corner attenuates further.
    let last = csv.lines().last().unwrap();
    let f: Vec<&str> = last.split(',').collect();
    let freq: f64 = f[0].parse().unwrap();
    assert!(freq > 1e6);
    let re: f64 = f[header.split(',').position(|h| h == "v(out)_re").unwrap()]
        .parse()
        .unwrap();
    assert!(
        re.abs() < 0.01,
        "a two-pole low-pass should be deep in stopband at {freq} Hz, got {re}"
    );
}

/// A loop-built ladder: the node names come from the loop variable, and the
/// values must match the analytic ladder (each rung halves the previous one).
#[test]
fn run_ladder_matches_the_analytic_taps() {
    let dir = scratch("ladder");
    let out = run(&[
        "run",
        &example("ladder.cdsl"),
        "--experiment",
        "tap",
        "--out",
        &dir.display().to_string(),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);

    let csv = std::fs::read_to_string(dir.join("tap.op1.csv")).expect("result");
    let mut lines = csv.lines();
    let header: Vec<&str> = lines.next().unwrap().split(',').collect();
    let row: Vec<&str> = lines.next().unwrap().split(',').collect();
    let get = |name: &str| -> f64 {
        let i = header
            .iter()
            .position(|h| *h == name)
            .unwrap_or_else(|| panic!("no column {name} in {header:?}"));
        row[i].parse().unwrap()
    };

    // 3 V into 1 kohm rungs with 2 kohm shunts: 1.5, 0.75, 0.375, 0.1875.
    assert!((get("v(top)") - 3.0).abs() < 1e-9, "{csv}");
    for (k, expected) in [(1, 1.5), (2, 0.75), (3, 0.375), (4, 0.1875)] {
        let got = get(&format!("v(mid{k})"));
        assert!(
            (got - expected).abs() < 1e-9,
            "v(mid{k}) should be {expected}, got {got}"
        );
    }
    assert!((get("i(r0)") - 0.0015).abs() < 1e-9, "{csv}");
    // KCL at mid1: what arrives through r0 leaves through r1 and rs1.
    let i_rs1 = get("i(rs1)");
    assert!((i_rs1 - 0.00075).abs() < 1e-9, "{csv}");
}

/// The nonlinear example: the forward drop must land in the physical range
/// and move only logarithmically with current.
#[test]
fn run_diode_sweep_shows_a_logarithmic_forward_drop() {
    let dir = scratch("diode");
    let out = run(&[
        "run",
        &example("diode_rectifier.cdsl"),
        "--experiment",
        "forward_drop",
        "--out",
        &dir.display().to_string(),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);

    let csv = std::fs::read_to_string(dir.join("forward_drop.dc1.csv")).expect("dc result");
    let mut lines = csv.lines();
    let header: Vec<&str> = lines.next().unwrap().split(',').collect();
    let vcol = header.iter().position(|h| *h == "v(vout)").unwrap();

    let mut drops = Vec::new();
    for line in lines {
        let f: Vec<&str> = line.split(',').collect();
        drops.push(f[vcol].parse::<f64>().unwrap());
    }
    assert_eq!(drops.len(), 11);

    // Every conducting point sits in a plausible silicon range.
    for d in drops.iter().skip(2) {
        assert!((0.3..0.8).contains(d), "implausible forward drop {d}");
    }
    // Ten times the current buys only a few tens of millivolts.
    let rise = drops[10] - drops[2];
    assert!(
        (0.01..0.15).contains(&rise),
        "the drop should move only logarithmically, moved {rise}"
    );
}

/// A parameter sweep re-elaborates per point and checks topology invariance.
#[test]
fn run_parameter_sweep_is_exact() {
    let dir = scratch("psweep");
    let out = run(&[
        "run",
        &example("parameter_sweep.cdsl"),
        "--experiment",
        "sweep",
        "--out",
        &dir.display().to_string(),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);

    let csv = std::fs::read_to_string(dir.join("sweep.dc_param_r.csv")).expect("result");
    let mut lines = csv.lines();
    lines.next();
    let mut count = 0;
    for line in lines {
        let f: Vec<&str> = line.split(',').collect();
        let r: f64 = f[0].parse().unwrap();
        let vo: f64 = f[1].parse().unwrap();
        // v(out) = 3 V * 1.5k / (r + 1.5k)
        let expected = 3.0 * 1500.0 / (r + 1500.0);
        assert!(
            (vo - expected).abs() < 1e-9,
            "at r={r}: got {vo}, expected {expected}"
        );
        count += 1;
    }
    assert_eq!(count, 8, "0.5k to 4k in 0.5k steps");
}

// ---------------------------------------------------------------------------
// capabilities and argument handling
// ---------------------------------------------------------------------------

#[test]
fn capabilities_reports_the_backend() {
    let out = run(&["capabilities"]);
    assert_eq!(out.status, 0);
    assert!(out.stdout.contains("thevenin 0.5.0"), "{}", out.stdout);
    assert!(out.stdout.contains("op, dc, ac, tran"), "{}", out.stdout);
    // The honest notes must be there, not just a feature list.
    assert!(out.stdout.contains("Windows"), "{}", out.stdout);
}

#[test]
fn version_works() {
    let out = run(&["--version"]);
    assert_eq!(out.status, 0);
    assert!(out.stdout.contains("cdsl"), "{}", out.stdout);
}

#[test]
fn running_an_unknown_experiment_lists_the_real_ones() {
    let out = run(&[
        "run",
        &example("rc_filter.cdsl"),
        "--experiment",
        "nope",
        "--out",
        &scratch("nope").display().to_string(),
    ]);
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("nope"), "{}", out.stderr);
    assert!(
        out.stderr.contains("response"),
        "the available names must be listed:\n{}",
        out.stderr
    );
}

#[test]
fn the_default_experiment_is_used_when_only_one_exists() {
    let dir = scratch("default_exp");
    let out = run(&[
        "run",
        &example("voltage_divider.cdsl"),
        "--out",
        &dir.display().to_string(),
        "--format",
        "csv",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);
    assert!(dir.join("divider.op1.csv").exists());
}

/// JSON output must be valid JSON even though the data contains no
/// non-finite values in this example; the non-finite policy itself is tested
/// in `circuit-results`.
#[test]
fn json_output_parses() {
    let dir = scratch("json");
    let out = run(&[
        "run",
        &example("rc_filter.cdsl"),
        "--experiment",
        "response",
        "--out",
        &dir.display().to_string(),
        "--format",
        "json",
    ]);
    assert_eq!(out.status, 0, "{}", out.stderr);

    let text = std::fs::read_to_string(dir.join("response.tran1.json")).expect("json result");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    assert_eq!(parsed["analysis"], "tran1");
    assert_eq!(parsed["kind"], "tran");
    assert!(parsed["signals"].as_array().unwrap().len() >= 2);
    // Units must survive into the file.
    let units: Vec<&str> = parsed["signals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["unit"].as_str().unwrap_or(""))
        .collect();
    assert!(units.contains(&"V"), "{units:?}");
}
