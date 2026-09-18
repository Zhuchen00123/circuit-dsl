//! Round-3 result-expression flow through the session API.
//!
//! These tests drive the same executor the file-mode CLI and the REPL drive
//! (`circuit_session::execute`), so the values they see are the values a
//! user sees. What is checked here that the CLI tests cannot check is the
//! **in-memory side**: the raw dataset a derive was appended to, the output
//! view the exporter receives, and the text the exporters write for one
//! specific `f64`. The two are compared field for field, text for text.
//!
//! Everything on disk goes under `CARGO_TARGET_TMPDIR`.

use std::path::{Path, PathBuf};

use circuit_backend::thevenin::TheveninBackend;
use circuit_core::Limits;
use circuit_core::format_number;
use circuit_session::execute::execute as run_experiment;
use circuit_session::{Format, Reply, RunOutcome, RunRequest, Session, write_datasets};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Feed a whole source into a fresh session, one line at a time, as the REPL
/// does. Any rejected line fails the test.
fn session_with(source: &str) -> Session {
    let mut session = Session::default();
    for line in source.lines() {
        if let Err(diagnostics) = session.feed(line) {
            panic!(
                "line {line:?} was refused:\n{}",
                session.render(&diagnostics)
            );
        }
    }
    session
}

/// Execute one experiment through the public seam, exactly as the CLI does.
fn run_experiment_in(session: &Session, experiment: &str) -> RunOutcome {
    let program = session.program();
    let limits = Limits::default();
    let request = RunRequest {
        program: &program,
        experiment,
        overrides: &[],
        limits: &limits,
    };
    let mut backend = TheveninBackend::new();
    match run_experiment(&request, &mut backend, session.sources()) {
        Ok(outcome) => outcome,
        Err(diagnostics) => panic!("{}", diagnostics.render(session.sources())),
    }
}

/// The same, keeping the rendered error instead of panicking.
fn try_run(session: &Session, experiment: &str) -> Result<RunOutcome, String> {
    let program = session.program();
    let limits = Limits::default();
    let request = RunRequest {
        program: &program,
        experiment,
        overrides: &[],
        limits: &limits,
    };
    let mut backend = TheveninBackend::new();
    run_experiment(&request, &mut backend, session.sources())
        .map_err(|diagnostics| diagnostics.render(session.sources()))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("flow_{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn signal<'a>(
    outcome: &'a RunOutcome,
    dataset: usize,
    name: &str,
) -> &'a circuit_results::dataset::Signal {
    let d = &outcome.datasets[dataset];
    d.signals
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| {
            panic!(
                "no signal {name:?} in dataset {}; signals: {:?}",
                d.analysis,
                d.signals
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
            )
        })
}

fn output_signal<'a>(
    outcome: &'a RunOutcome,
    dataset: usize,
    name: &str,
) -> &'a circuit_results::dataset::Signal {
    let d = &outcome.output_datasets[dataset];
    d.signals
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| {
            panic!(
                "no exported signal {name:?} in dataset {}; signals: {:?}",
                d.analysis,
                d.signals
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
            )
        })
}

/// The raw fields of a written CSV, header first.
fn csv_rows(text: &str) -> Vec<Vec<String>> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split(',').map(str::to_string).collect())
        .collect()
}

fn column(rows: &[Vec<String>], name: &str) -> usize {
    rows[0]
        .iter()
        .position(|h| h == name)
        .unwrap_or_else(|| panic!("no column {name:?} in {:?}", rows[0]))
}

fn real_data(signal: &circuit_results::dataset::Signal) -> &[f64] {
    signal.data.as_real().expect("a real signal")
}

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

/// RC low-pass at its own cut-off: R = 1 kohm, C = 1/(2*pi*R*1 kHz), so the
/// 1 kHz sample is exactly the corner and H = 0.5 - 0.5j there.
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

const R_OHM: f64 = 1_000.0;
const C_FARAD: f64 = 159.154_943_091_895_35e-9;

/// One derive reads a probe that is not in the save list.
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
  measure :vend, max: v(:vout)
end
";

/// A parameter sweep: one stitched dataset, addressed as dc1.
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
end
";

/// A runtime failure: the measure divides by zero on the first sample.
const DIVIDE_BY_ZERO: &str = "\
circuit :div do
  node :in, :out
  voltage_source :v1, p: :in, n: :gnd, dc: 1.V
  resistor :r1, p: :in, n: :out, value: 1.kohm
  resistor :r2, p: :out, n: :gnd, value: 1.kohm
end
experiment :e, circuit: :div do
  op
  save v(:out)
  measure :bad, max: v(:out) / 0
end
";

// ===========================================================================

/// The in-memory derived signals and the written files carry the same numbers,
/// digit for digit, and a derived complex signal keeps both parts.
#[test]
fn exported_files_match_the_in_memory_derived_signals() {
    let dir = scratch("ac_export");
    let session = session_with(RC_AT_FC);
    let outcome = run_experiment_in(&session, "e");

    // Raw and output views: one AC dataset, named after the analysis identity.
    assert_eq!(outcome.datasets.len(), 1);
    assert_eq!(outcome.datasets[0].analysis, "ac1");
    assert_eq!(outcome.datasets[0].kind, "ac");
    assert_eq!(outcome.output_datasets.len(), 1);
    assert_eq!(outcome.output_datasets[0].analysis, "ac1");

    // The derived signals are in the output view, with the right handedness:
    // gain came from a complex division and stays complex, gain_db is real.
    let gain = output_signal(&outcome, 0, "gain");
    assert!(gain.is_complex(), "a complex derive must stay complex");
    let gain_db = output_signal(&outcome, 0, "gain_db");
    assert!(!gain_db.is_complex(), "gain_db must be real");
    assert!(
        gain_db.unit.is_dimensionless(),
        "a dB ratio has no unit, got {}",
        gain_db.unit
    );

    // The corner sample, straight out of the in-memory data, against the
    // component values.
    let f = outcome.output_datasets[0].axis.samples()[40];
    let fc = 1.0 / (2.0 * std::f64::consts::PI * R_OHM * C_FARAD);
    assert!(
        ((f - fc) / fc).abs() < 1e-12,
        "sample 40 is {f} Hz, not Fc = {fc} Hz"
    );
    let rc = 2.0 * std::f64::consts::PI * f * R_OHM * C_FARAD;
    let (re, im) = match &gain.data {
        circuit_results::dataset::Data::Complex(values) => (values[40].re, values[40].im),
        circuit_results::dataset::Data::Real(values) => (values[40], 0.0),
    };
    assert!((re - 0.5).abs() < 1e-9, "gain.re = {re}");
    assert!((im + 0.5).abs() < 1e-9, "gain.im = {im}");
    assert!(
        (re - 1.0 / (1.0 + rc * rc)).abs() < 1e-9,
        "gain.re against 1/(1+(wRC)^2)"
    );
    assert!(
        (im + rc / (1.0 + rc * rc)).abs() < 1e-9,
        "gain.im against -wRC/(1+(wRC)^2)"
    );

    // Write both formats and compare the text against the in-memory values.
    let mut approve = |_: &Path| -> Result<(), circuit_core::diagnostic::Diagnostics> { Ok(()) };
    let written =
        write_datasets(&dir, Format::Both, &outcome.output_datasets, &mut approve).expect("write");
    assert_eq!(written.paths.len(), 2, "{written:?}");
    let csv_text = std::fs::read_to_string(dir.join("e.ac1.csv")).expect("csv");
    let json_text = std::fs::read_to_string(dir.join("e.ac1.json")).expect("json");
    let rows = csv_rows(&csv_text);
    assert_eq!(rows.len(), 82, "81 samples + header");
    assert_eq!(
        rows[0],
        [
            "frequency",
            "v(vin)_re",
            "v(vin)_im",
            "v(vout)_re",
            "v(vout)_im",
            "gain_re",
            "gain_im",
            "gain_db"
        ]
    );

    let frequency = column(&rows, "frequency");
    let gain_re = column(&rows, "gain_re");
    let gain_im = column(&rows, "gain_im");
    let gain_db_col = column(&rows, "gain_db");

    for row in 0..81 {
        // The axis: in-memory f64 -> file text -> back, exactly.
        let axis_value = outcome.output_datasets[0].axis.samples()[row];
        assert_eq!(rows[row + 1][frequency], format_number(axis_value));
        assert_eq!(
            rows[row + 1][frequency].parse::<f64>().expect("parses"),
            axis_value
        );

        // The derived complex signal, component by component.
        let (re, im) = match &gain.data {
            circuit_results::dataset::Data::Complex(v) => (v[row].re, v[row].im),
            circuit_results::dataset::Data::Real(v) => (v[row], 0.0),
        };
        assert_eq!(
            rows[row + 1][gain_re],
            format_number(re),
            "gain.re row {row}"
        );
        assert_eq!(
            rows[row + 1][gain_im],
            format_number(im),
            "gain.im row {row}"
        );
        assert_eq!(
            rows[row + 1][gain_re].parse::<f64>().expect("parses"),
            re,
            "a written real part must read back to the same f64"
        );
        assert_eq!(
            rows[row + 1][gain_im].parse::<f64>().expect("parses"),
            im,
            "a written imaginary part must read back to the same f64"
        );

        // The derived real signal, and its value against the complex one.
        let db_value = match &gain_db.data {
            circuit_results::dataset::Data::Real(v) => v[row],
            circuit_results::dataset::Data::Complex(_) => panic!("gain_db must be real"),
        };
        assert_eq!(rows[row + 1][gain_db_col], format_number(db_value));
        assert!(
            (db_value - 20.0 * (re * re + im * im).sqrt().log10()).abs() < 1e-9,
            "gain_db row {row}"
        );
    }

    // The JSON must literally contain the CSV's field text for the derived
    // signals: both exporters use the same shortest round-trip formatter.
    for row in [0usize, 20, 40, 80] {
        for col in [gain_re, gain_im, gain_db_col] {
            let field = rows[row + 1][col].clone();
            assert!(
                json_text.contains(&field),
                "the JSON must carry the CSV field {field:?} ({} at row {row})",
                rows[0][col]
            );
        }
    }
    // The identity and the units travel with the data.
    assert!(json_text.contains("\"analysis\": \"ac1\""), "{json_text}");
    assert!(json_text.contains("\"type\": \"complex\""), "{json_text}");
    assert!(json_text.contains("\"name\": \"gain_db\""), "{json_text}");

    // The measure is reported with its analysis, from the same outcome.
    assert_eq!(outcome.measures.len(), 1);
    assert_eq!(outcome.measures[0].name, "peak_gain");
    assert_eq!(outcome.measures[0].analysis, "ac1");
    assert_eq!(
        outcome.measures[0].render_with_analysis(),
        "peak_gain = 0.9950371902099894 dimensionless (ac1)"
    );
}

/// A derive reading a probe the user never saved: read on the raw dataset,
/// absent from the export, present in both as its own signal.
#[test]
fn an_implicit_read_lives_in_the_raw_dataset_only() {
    let dir = scratch("implicit");
    let session = session_with(SAVE_ONLY_VIN);
    let outcome = run_experiment_in(&session, "e");

    let raw_names: Vec<&str> = outcome.datasets[0]
        .signals
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    let export_names: Vec<&str> = outcome.output_datasets[0]
        .signals
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    // The engine produced v(vout) because the expression needed it...
    assert!(raw_names.contains(&"v(vout)"), "{raw_names:?}");
    assert!(raw_names.contains(&"vdiff"), "{raw_names:?}");
    // ...and the export kept the save list plus the derived signal.
    assert_eq!(export_names, ["v(vin)", "vdiff"], "{export_names:?}");

    // The derived samples are exactly v(vin) - v(vout) on the raw grid.
    let vin = real_data(signal(&outcome, 0, "v(vin)"));
    let vout = real_data(signal(&outcome, 0, "v(vout)"));
    let vdiff = real_data(signal(&outcome, 0, "vdiff"));
    assert_eq!(vin.len(), vout.len());
    assert_eq!(vin.len(), vdiff.len());
    let mut worst = 0.0_f64;
    for i in 0..vdiff.len() {
        worst = worst.max((vdiff[i] - (vin[i] - vout[i])).abs());
    }
    assert!(worst < 1e-15, "vdiff drifts from v(vin)-v(vout) by {worst}");

    // And the exported values are the in-memory ones, text for text.
    let mut approve = |_: &Path| -> Result<(), circuit_core::diagnostic::Diagnostics> { Ok(()) };
    write_datasets(&dir, Format::Csv, &outcome.output_datasets, &mut approve).expect("write");
    let rows = csv_rows(&std::fs::read_to_string(dir.join("e.tran1.csv")).expect("csv"));
    assert_eq!(rows[0], ["time", "v(vin)", "vdiff"]);
    let exported = column(&rows, "vdiff");
    for row in [0usize, 1, 500, 1000, rows.len() - 2] {
        assert_eq!(rows[row + 1][exported], format_number(vdiff[row]));
    }
}

/// The public measure seam takes the same datasets and must produce exactly
/// what the run produced.
#[test]
fn the_public_measure_seam_agrees_with_the_run() {
    let session = session_with(RC_AT_FC);
    let program = session.program();
    let limits = Limits::default();
    let elaborated =
        circuit_dsl::elaborate_experiment(&program, "e", &[], &limits).expect("elaborates");
    let outcome = run_experiment_in(&session, "e");

    // The public seam lives in `execute`; it is not re-exported at the crate
    // root (contract §6 names it, it is reachable, just one path deeper).
    let again = circuit_session::execute::evaluate_measures(&elaborated.plan, &outcome.datasets)
        .expect("the measures evaluate");
    assert_eq!(again.len(), outcome.measures.len());
    for (a, b) in again.iter().zip(&outcome.measures) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.analysis, b.analysis);
        assert_eq!(a.value, b.value, "{} must be the same number", a.name);
        assert_eq!(a.render_with_analysis(), b.render_with_analysis());
    }

    // The plan itself says which analysis each result is on.
    assert_eq!(elaborated.plan.analysis_names(), ["ac1"]);
    assert_eq!(elaborated.plan.derives.len(), 2);
    assert_eq!(elaborated.plan.derives[0].name, "gain");
    assert_eq!(elaborated.plan.derives[0].source, "(v(vout) / v(vin))");
    // The derive really is an expression, not a bare probe.
    assert!(!elaborated.plan.derives[0].expr.is_plain_probe());
}

/// A parameter sweep: one stitched dataset, the derive bound to the swept
/// analysis, and the measurement taken from the stitched axis.
#[test]
fn a_parameter_sweep_yields_one_stitched_dataset() {
    let dir = scratch("sweep");
    let session = session_with(PARAM_SWEEP);
    let outcome = run_experiment_in(&session, "sweep");

    assert_eq!(outcome.datasets.len(), 1);
    assert_eq!(outcome.output_datasets.len(), 1);
    let d = &outcome.output_datasets[0];
    assert_eq!(d.analysis, "dc_param_r");
    assert_eq!(d.kind, "dc");
    let values = d.axis.samples().to_vec();
    assert_eq!(values, [500.0, 1_000.0, 1_500.0, 2_000.0]);

    // The derived signal is on the stitched dataset, and it is twice v(out).
    let vout = real_data(signal(&outcome, 0, "v(out)")).to_vec();
    let scaled = real_data(output_signal(&outcome, 0, "scaled")).to_vec();
    assert_eq!(vout.len(), 4);
    for i in 0..4 {
        let expected = 3.0 * 1_500.0 / (values[i] + 1_500.0);
        assert!(
            (vout[i] - expected).abs() < 1e-9,
            "v(out) at r={}: {} vs {expected}",
            values[i],
            vout[i]
        );
        assert!(
            (scaled[i] - 2.0 * vout[i]).abs() < 1e-12,
            "scaled at row {i}"
        );
    }

    // The measure reports the stitched dataset's identity, and the file is
    // named after it.
    assert_eq!(outcome.measures.len(), 1);
    assert_eq!(outcome.measures[0].analysis, "dc_param_r");
    assert!((outcome.measures[0].value - 2.25).abs() < 1e-12);
    let mut approve = |_: &Path| -> Result<(), circuit_core::diagnostic::Diagnostics> { Ok(()) };
    let written =
        write_datasets(&dir, Format::Csv, &outcome.output_datasets, &mut approve).expect("write");
    assert_eq!(
        written
            .paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        ["sweep.dc_param_r.csv"]
    );
}

/// A runtime expression failure is an error from the executor, never a run
/// with a shortened measure list and a plausible exit code.
#[test]
fn a_runtime_failure_aborts_the_run() {
    let session = session_with(DIVIDE_BY_ZERO);
    let error = try_run(&session, "e").expect_err("division by zero must fail the run");
    assert!(error.contains("E_VALUE"), "{error}");
    assert!(error.contains("division by zero"), "{error}");
    assert!(error.contains("at sample 0"), "{error}");
    assert!(error.contains("op1"), "{error}");

    // The same input through the REPL surface: the session reports it and the
    // experiment stays defined, so the user can fix and rerun.
    let mut session = session_with(DIVIDE_BY_ZERO);
    let reply = session.run("e", &[], None);
    let diagnostics = reply.expect_err("the REPL must not report a value");
    let text = session.render(&diagnostics);
    assert!(text.contains("E_VALUE"), "{text}");
    assert_eq!(session.experiment_names(), ["e"]);
}

/// The REPL surface renders the same measure the outcome carries, and writing
/// from a session produces the same files as the CLI.
#[test]
fn a_session_run_reports_and_writes_what_the_outcome_holds() {
    let dir = scratch("session_run");
    let mut session = session_with(RC_AT_FC);
    let outcome = run_experiment_in(&session, "e");

    let reply = session.run("e", &[], Some(&dir)).expect("the run succeeds");
    let Reply::Message(text) = reply else {
        panic!("expected a summary, got {reply:?}");
    };
    for measure in &outcome.measures {
        let rendered = format!("  measure {}", measure.render_with_analysis());
        assert!(
            text.contains(&rendered),
            "the transcript must carry {rendered:?}:\n{text}"
        );
    }
    assert!(
        text.contains("ac1: 81 frequency points; signals: v(vin), v(vout), gain, gain_db"),
        "{text}"
    );
    // Written as both formats, with the derived columns.
    assert!(dir.join("e.ac1.csv").exists(), "{text}");
    assert!(dir.join("e.ac1.json").exists(), "{text}");
    let rows = csv_rows(&std::fs::read_to_string(dir.join("e.ac1.csv")).expect("csv"));
    assert!(rows[0].iter().any(|h| h == "gain_re"));
    assert!(rows[0].iter().any(|h| h == "gain_db"));

    // Running again is deterministic: same measures, same text.
    let second = run_experiment_in(&session, "e");
    assert_eq!(
        second.measures[0].render_with_analysis(),
        outcome.measures[0].render_with_analysis()
    );
}
