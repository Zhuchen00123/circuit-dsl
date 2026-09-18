//! Session-layer regression for the Task-A contract:
//! **the solved grid and the displayed grid are two different things.**
//!
//! # What is being pinned
//!
//! `circuit_session::execute` returns both views:
//!
//! * `RunOutcome.datasets` — the raw solver output (2015 points for the
//!   reproduction below, one point per accepted internal step);
//! * `RunOutcome.output_datasets` — what the user reads and exports, i.e. the
//!   raw trace resampled onto `output_interval:` (`circuit_results::resample`,
//!   implemented *after* the run).
//!
//! Measurements are evaluated on the **raw** view, so an `avg`/`rms`/`max`/`min`
//! cannot move when the user asks for a coarser *picture*. Before this round
//! `output_interval:` was pushed into the engine's `.tran` step, where it also
//! set the floor of the PULSE rise/fall: the stimulus, the raw grid and every
//! measurement changed with the output request. That is what the assertions
//! below rule out.
//!
//! # Front end
//!
//! Every test goes through the real front end — `lex` -> `parse` -> `compile`
//! / the `elaborate_experiment` call inside `execute` — so nothing here hand-
//! builds a plan that the language could not have produced. The source is the
//! CLI reproduction (`target/round2-evidence/repro/pulse-fine.cdsl`,
//! `pulse-coarse.cdsl`) with measurements added:
//!
//! ```text
//! v1 (PULSE 0->1 V, rise = fall = 10 ns, width = 10 us, period = 20 us)
//!   -> r1 (1 kohm) -> out, c1 (100 nF) -> gnd        tau = R*C = 100 us
//! tran stop: 2 us, max_step: 1 ns, output_interval: 1 ns | 100 ns | absent
//! ```
//!
//! # Grid arithmetic used below
//!
//! `0 .. 2 us` at 100 ns is `0, 100, …, 1900, 2000 ns` = **21** points: the
//! first raw point, the interior grid points strictly inside the trace, and the
//! raw last point. `Limits::for_tests()` (`max_result_values = 10_000`) makes a
//! 1 ps output grid (2e6 points) over budget without building anything large;
//! the raw 2015-point dataset (2 signals = 4030 values) stays well inside it, so
//! the limit fires on the resampling, not on the solve.

use circuit_backend::thevenin::TheveninBackend;
use circuit_core::diagnostic::{Code, Diagnostics};
use circuit_core::plan::AnalysisKind;
use circuit_core::{Limits, SourceMap};
use circuit_dsl::{lex, parse};
use circuit_results::dataset::{Axis, Data, Dataset};
use circuit_results::measure::{Measurement, measure_signal};
use circuit_session::RunOutcome;
use circuit_session::execute::{RunRequest, execute};

// ---------------------------------------------------------------------------
// The source, exactly as the CLI reproduction writes it
// ---------------------------------------------------------------------------

const SRC: &str = r#"
circuit :rc do
  node :vin, :out
  voltage_source :v1, p: :vin, n: :gnd, dc: 0.V, waveform: pulse(low: 0.V, high: 1.V, delay: 0.s, rise: 10.ns, fall: 10.ns, width: 10.us, period: 20.us)
  resistor :r1, p: :vin, n: :out, value: 1.kohm
  capacitor :c1, p: :out, n: :gnd, value: 100.nF
end

experiment :fine, circuit: :rc do
  tran stop: 2.us, max_step: 1.ns, output_interval: 1.ns
  save v(:vin), v(:out)
  measure :vout_max, max: v(:out)
  measure :vout_min, min: v(:out)
  measure :vout_avg, avg: v(:out)
  measure :vout_rms, rms: v(:out)
  measure :vin_avg, avg: v(:vin)
end

experiment :coarse, circuit: :rc do
  tran stop: 2.us, max_step: 1.ns, output_interval: 100.ns
  save v(:vin), v(:out)
  measure :vout_max, max: v(:out)
  measure :vout_min, min: v(:out)
  measure :vout_avg, avg: v(:out)
  measure :vout_rms, rms: v(:out)
  measure :vin_avg, avg: v(:vin)
end

experiment :raw, circuit: :rc do
  tran stop: 2.us, max_step: 1.ns
  save v(:vin), v(:out)
end

experiment :toofine, circuit: :rc do
  tran stop: 2.us, max_step: 1.ns, output_interval: 1.ps
  save v(:vin), v(:out)
end
"#;

/// A source that must be refused by the front end (Task A item 1: an explicit
/// `output_interval:` that is zero or negative is a user error, never a silent
/// fallback).
const BAD_INTERVALS: &str = r#"
circuit :rc do
  node :vin, :out
  voltage_source :v1, p: :vin, n: :gnd, dc: 0.V, waveform: pulse(low: 0.V, high: 1.V, delay: 0.s, rise: 10.ns, fall: 10.ns, width: 10.us, period: 20.us)
  resistor :r1, p: :vin, n: :out, value: 1.kohm
  capacitor :c1, p: :out, n: :gnd, value: 100.nF
end

experiment :zero, circuit: :rc do
  tran stop: 2.us, max_step: 1.ns, output_interval: 0.s
  save v(:out)
end

experiment :negative, circuit: :rc do
  tran stop: 2.us, max_step: 1.ns, output_interval: -1.ns
  save v(:out)
end
"#;

// ---------------------------------------------------------------------------
// Front end helpers
// ---------------------------------------------------------------------------

/// Lex, parse and keep the source map for rendering diagnostics.
fn front(src: &str) -> (circuit_dsl::Program, SourceMap) {
    let mut sm = SourceMap::new();
    let id = sm.add("tran_output_interval.cdsl", src);
    let tokens = match lex(id, src) {
        Ok(t) => t,
        Err(d) => panic!("lex failed:\n{}", d.render(&sm)),
    };
    let program = match parse(&tokens) {
        Ok(p) => p,
        Err(d) => panic!("parse failed:\n{}", d.render(&sm)),
    };
    (program, sm)
}

/// The program the tests below use, with its real elaboration checked once
/// (`compile`, the same entry point `cdsl check` uses).
fn compiled() -> (circuit_dsl::Program, SourceMap, circuit_dsl::Compiled) {
    let (program, sm) = front(SRC);
    let c = match circuit_dsl::compile(&program, &Limits::default()) {
        Ok(c) => c,
        Err(d) => panic!("compile failed:\n{}", d.render(&sm)),
    };
    (program, sm, c)
}

/// Run one experiment through the session's own `execute`, with `limits`.
fn run(
    program: &circuit_dsl::Program,
    sources: &SourceMap,
    experiment: &str,
    limits: &Limits,
) -> Result<RunOutcome, Diagnostics> {
    let mut backend = TheveninBackend::with_limits(*limits);
    let request = RunRequest {
        program,
        experiment,
        overrides: &[],
        limits,
    };
    execute(&request, &mut backend, sources)
}

/// Axis of a dataset, for the time-axis assertions.
fn times(d: &Dataset) -> &[f64] {
    match &d.axis {
        Axis::Time(t) => t,
        other => panic!("expected a time axis, got {other:?}"),
    }
}

/// Bit-for-bit axis comparison (a sign flip or a re-derived value must fail).
fn assert_axis_bits(label: &str, a: &[f64], b: &[f64]) {
    assert_eq!(a.len(), b.len(), "{label}: axis lengths differ");
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        assert_eq!(
            x.to_bits(),
            y.to_bits(),
            "{label}: time[{i}] differs: {x:?} vs {y:?}"
        );
    }
}

/// A numeric backend setting (the adapter formats seconds with `Display`).
fn setting_f64(d: &Dataset, key: &str) -> f64 {
    let raw = d
        .backend
        .setting(key)
        .unwrap_or_else(|| panic!("missing setting `{key}` in {:?}", d.backend.settings));
    raw.parse::<f64>()
        .unwrap_or_else(|e| panic!("setting `{key}` = {raw:?} is not a number: {e}"))
}

/// The sample nearest `target` in `signal`, as `(time, value)`.
fn nearest_sample(d: &Dataset, signal: &str, target: f64) -> (f64, f64) {
    let t = times(d);
    let v = match &d
        .signal(signal)
        .unwrap_or_else(|| panic!("signal {signal}"))
        .data
    {
        Data::Real(v) => v,
        Data::Complex(_) => panic!("{signal} is complex"),
    };
    assert_eq!(t.len(), v.len(), "axis and signal lengths");
    let mut best = 0usize;
    for (i, &ti) in t.iter().enumerate() {
        if (ti - target).abs() < (t[best] - target).abs() {
            best = i;
        }
    }
    (t[best], v[best])
}

/// A measurement by name, or panic listing what was reported.
fn measure_of(outcome: &RunOutcome, name: &str) -> f64 {
    outcome
        .measures
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| {
            panic!(
                "no measurement `{name}`; got {:?}",
                outcome
                    .measures
                    .iter()
                    .map(|m| m.name.as_str())
                    .collect::<Vec<_>>()
            )
        })
        .value
}

// ---------------------------------------------------------------------------
// 1. The front end reads `output_interval` as an explicit option
// ---------------------------------------------------------------------------

#[test]
fn the_front_end_carries_the_output_interval_and_refuses_bad_ones() {
    let (_, _, c) = compiled();

    let interval = |name: &str| match &c.experiment(name).expect("experiment").plan.tasks[0].kind {
        AnalysisKind::Tran(spec) => spec.output_interval,
        other => panic!("expected a tran task, got {other:?}"),
    };
    println!(
        "[front] intervals: fine={:?} coarse={:?} raw={:?} toofine={:?}",
        interval("fine"),
        interval("coarse"),
        interval("raw"),
        interval("toofine")
    );
    assert_eq!(interval("fine"), Some(1e-9));
    // `100.ns` in the front end is `100.0 * 1e-9`, which is one ulp away from
    // the literal `1e-7`; compare against the arithmetic the language performs.
    assert_eq!(interval("coarse"), Some(100.0 * 1e-9));
    assert_eq!(interval("raw"), None);
    assert_eq!(interval("toofine"), Some(1e-12));

    // The plan also carries `max_step`, so the two options cannot be confused.
    match &c.experiment("coarse").expect("experiment").plan.tasks[0].kind {
        AnalysisKind::Tran(spec) => {
            assert_eq!(spec.max_step, Some(1e-9));
            assert_eq!(spec.stop_s, 2e-6);
            assert_eq!(spec.start_s, 0.0);
        }
        other => panic!("expected a tran task, got {other:?}"),
    }

    // Zero and negative intervals are refused where they are read, with a
    // `Code::Value` that points at the argument.
    let (bad, sm) = front(BAD_INTERVALS);
    for name in ["zero", "negative"] {
        match circuit_dsl::compile(&bad, &Limits::default()) {
            Ok(_) => panic!("`output_interval` in experiment `{name}` must be refused"),
            Err(d) => {
                let text = d.render(&sm);
                println!("[front/{name}] {}", text.lines().next().unwrap_or(""));
                assert!(
                    d.iter().any(|e| e.code == Code::Value),
                    "expected E_VALUE for `{name}`, got:\n{text}"
                );
                assert!(
                    text.contains("output_interval"),
                    "the diagnostic must name the option; got:\n{text}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 2. The two views: coarse output, raw solve
// ---------------------------------------------------------------------------

/// The coarse run's output view is the requested 21-point grid, while the raw
/// view keeps the solver's own points; the first and last output points are the
/// raw ones, bit for bit.
#[test]
fn the_output_view_is_the_requested_grid_and_the_raw_view_is_untouched() {
    let (program, sm, _) = compiled();
    let limits = Limits::default();
    let outcome = match run(&program, &sm, "coarse", &limits) {
        Ok(o) => o,
        Err(d) => panic!("run failed:\n{}", d.render(&sm)),
    };

    assert_eq!(
        outcome.datasets.len(),
        1,
        "one tran task -> one raw dataset"
    );
    assert_eq!(outcome.output_datasets.len(), 1);
    let raw = &outcome.datasets[0];
    let out = &outcome.output_datasets[0];
    assert_eq!(raw.analysis, "tran1");
    assert_eq!(out.kind, "tran");

    let raw_t = times(raw);
    let out_t = times(out);
    println!(
        "[coarse] raw={} points, output={} points, raw first={:?} last={:?}, \
         output first={:?} last={:?}",
        raw_t.len(),
        out_t.len(),
        raw_t.first(),
        raw_t.last(),
        out_t.first(),
        out_t.last()
    );

    // Raw view: the solver's grid, not the 21-point request.
    assert!(
        raw_t.len() > 1000,
        "the raw dataset must keep the solver's points, got {}",
        raw_t.len()
    );
    assert!(raw_t.len() > out_t.len());

    // Output view: exactly the requested uniform grid, endpoints included.
    assert_eq!(out_t.len(), 21, "2 us at 100 ns is 21 points");
    assert_eq!(out_t[0].to_bits(), raw_t[0].to_bits());
    assert_eq!(
        out_t[out_t.len() - 1].to_bits(),
        raw_t[raw_t.len() - 1].to_bits(),
        "the output grid must end on the raw last point"
    );
    assert_eq!(out_t[out_t.len() - 1], 2e-6, "stop: 2.us");
    for (i, &t) in out_t.iter().enumerate() {
        let expected = i as f64 * 100e-9;
        assert!(
            (t - expected).abs() <= 1e-18,
            "output point {i} is {t:?} s, expected {expected:?} s"
        );
    }
    for w in out_t.windows(2) {
        assert!(w[1] > w[0], "output grid must be strictly increasing");
    }

    // Signals were resampled to the same length as the axis.
    for s in &out.signals {
        assert_eq!(s.len(), out_t.len(), "signal `{}` length", s.name);
    }
    for s in &raw.signals {
        assert_eq!(s.len(), raw_t.len(), "raw signal `{}` length", s.name);
    }

    // The metadata says which view is which.
    assert_eq!(
        raw.backend.setting("tran.solve_points"),
        Some(raw_t.len().to_string().as_str()),
        "the raw dataset records the number of solved points"
    );
    assert_eq!(
        setting_f64(raw, "tran.output_interval"),
        100.0 * 1e-9,
        "the raw dataset records the requested output interval"
    );
    assert_eq!(
        out.backend.setting("tran.output_grid"),
        Some("resampled-linear")
    );
    assert_eq!(out.backend.setting("tran.output_points"), Some("21"));

    // The coarse *output view* still carries the declared 10 ns edge: at the
    // 100 ns grid point the drive is already 1 V. The pre-fix CLI coarse file
    // (`target/round2-evidence/repro/out-coarse/coarse.tran1.csv`) showed
    // `v(vin) = 0.5002375000000003` at its nearest-50 ns sample because the
    // edge had been widened to the output interval.
    let (raw_at, raw_vin) = nearest_sample(raw, "v(vin)", 100e-9);
    let (out_at, out_vin) = nearest_sample(out, "v(vin)", 100e-9);
    println!(
        "[coarse] v(vin) near 100 ns: raw sample t={raw_at:?} s -> {raw_vin:?} V, \
         output sample t={out_at:?} s -> {out_vin:?} V"
    );
    assert!(
        (out_at - 100e-9).abs() <= 1e-18,
        "expected the 100 ns output grid point, got {out_at:?} s"
    );
    assert_eq!(
        out_vin, 1.0,
        "the 100 ns output point must show the 10 ns edge completed (v(vin) = 1 V)"
    );
    assert_eq!(raw_vin, 1.0, "the raw drive at 100 ns");

    // The fine run is a 1 ns grid over the same window: 2001 points.
    let fine = match run(&program, &sm, "fine", &limits) {
        Ok(o) => o,
        Err(d) => panic!("run failed:\n{}", d.render(&sm)),
    };
    let fine_raw = times(&fine.datasets[0]);
    let fine_out = times(&fine.output_datasets[0]);
    println!(
        "[fine] raw={} points, output={} points",
        fine_raw.len(),
        fine_out.len()
    );
    assert_eq!(fine_out.len(), 2001, "2 us at 1 ns is 2001 points");
    assert_eq!(fine_out[0].to_bits(), fine_raw[0].to_bits());
    assert_eq!(
        fine_out[fine_out.len() - 1].to_bits(),
        fine_raw[fine_raw.len() - 1].to_bits()
    );
    assert!(
        fine_raw.len() > 1000,
        "the fine raw dataset must keep the solver's points, got {}",
        fine_raw.len()
    );
}

// ---------------------------------------------------------------------------
// 3. Measurements use the raw grid
// ---------------------------------------------------------------------------

/// The same experiment at two output intervals must produce bit-identical
/// measurements: `avg`, `rms`, `max` and `min` are computed on the solved
/// trace, and asking for a coarser picture cannot move them.
///
/// The control at the end is what makes this more than a restatement: reducing
/// the **output view** directly (what a viewer or an exporter would do) moves
/// `avg: v(:vin)` by tens of millivolts, because the 21-point grid does not
/// resolve the 10 ns edge. The session's number is the raw-grid one.
#[test]
fn measurements_are_computed_on_the_raw_grid() {
    let (program, sm, _) = compiled();
    let limits = Limits::default();
    let fine = match run(&program, &sm, "fine", &limits) {
        Ok(o) => o,
        Err(d) => panic!("fine run failed:\n{}", d.render(&sm)),
    };
    let coarse = match run(&program, &sm, "coarse", &limits) {
        Ok(o) => o,
        Err(d) => panic!("coarse run failed:\n{}", d.render(&sm)),
    };

    let names: Vec<&str> = fine.measures.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(
        names,
        coarse
            .measures
            .iter()
            .map(|m| m.name.as_str())
            .collect::<Vec<_>>(),
        "both runs must report the same measurements in the same order"
    );
    for required in ["vout_max", "vout_min", "vout_avg", "vout_rms"] {
        assert!(names.contains(&required), "missing measurement {required}");
    }

    for (a, b) in fine.measures.iter().zip(&coarse.measures) {
        println!(
            "[measure] {} fine={:?} coarse={:?} bits-equal={}",
            a.name,
            a.value,
            b.value,
            a.value.to_bits() == b.value.to_bits()
        );
        assert_eq!(
            a.value.to_bits(),
            b.value.to_bits(),
            "measurement `{}` moved with output_interval: {:?} vs {:?}",
            a.name,
            a.value,
            b.value
        );
        assert_eq!(a.unit, b.unit, "measurement `{}` unit", a.name);
    }

    // The measurements are the raw-grid values, not the output-view ones:
    // averaging `v(vin)` over the 21-point grid loses the 10 ns edge entirely.
    let vin_avg_raw = measure_of(&coarse, "vin_avg");
    let vin_avg_coarse_view = measure_signal(
        Measurement::Avg,
        "vin_avg_coarse_view",
        "v(vin)",
        &coarse.output_datasets[0],
    )
    .expect("avg of v(vin) on the output view");
    println!(
        "[measure] vin_avg: session={vin_avg_raw:?} (raw grid), output-view={:?}",
        vin_avg_coarse_view.value
    );
    assert!(
        (vin_avg_raw - vin_avg_coarse_view.value).abs() > 1e-3,
        "the raw-grid and output-view averages of v(vin) should differ well above 1 mV \
         (they were {:?} and {:?}); if they agree, the measurement may have been taken \
         from the output view",
        vin_avg_raw,
        vin_avg_coarse_view.value
    );
    assert!(
        (vin_avg_raw - 0.9975).abs() < 1e-6,
        "the raw-grid avg of a 0->1 V step with a 10 ns edge over 2 us is \
         1 - (0.5 * 10 ns)/2 us = 0.9975 V, got {vin_avg_raw:?}"
    );
}

// ---------------------------------------------------------------------------
// 4. No output_interval means no resampling
// ---------------------------------------------------------------------------

/// Without `output_interval:` the output view **is** the raw view, bit for bit:
/// no grid is invented, and no resampling metadata is added.
#[test]
fn an_absent_output_interval_does_not_resample() {
    let (program, sm, _) = compiled();
    let limits = Limits::default();
    let outcome = match run(&program, &sm, "raw", &limits) {
        Ok(o) => o,
        Err(d) => panic!("run failed:\n{}", d.render(&sm)),
    };
    let raw = &outcome.datasets[0];
    let out = &outcome.output_datasets[0];

    assert_axis_bits("raw vs output", times(out), times(raw));
    assert!(times(raw).len() > 1000);
    assert_eq!(raw.signals.len(), out.signals.len());
    for (a, b) in raw.signals.iter().zip(&out.signals) {
        assert_eq!(a.name, b.name);
        match (&a.data, &b.data) {
            (Data::Real(x), Data::Real(y)) => {
                assert_eq!(x.len(), y.len());
                for (i, (vx, vy)) in x.iter().zip(y).enumerate() {
                    assert_eq!(
                        vx.to_bits(),
                        vy.to_bits(),
                        "signal `{}`[{i}] differs between views",
                        a.name
                    );
                }
            }
            _ => panic!("expected real signals"),
        }
    }
    assert_eq!(raw.backend.setting("tran.output_interval"), None);
    assert_eq!(out.backend.setting("tran.output_interval"), None);
    assert_eq!(out.backend.setting("tran.output_grid"), None);
    assert_eq!(out.backend.setting("tran.output_points"), None);
}

// ---------------------------------------------------------------------------
// 5. Size limits reject, never truncate
// ---------------------------------------------------------------------------

/// A 1 ps output interval over a 2 us trace is 2e6 points, over
/// `Limits::for_tests()`'s `max_result_values = 10_000`. The run must fail with
/// `E_LIMIT` and produce nothing — no partially resampled dataset, no truncated
/// view. The raw dataset itself (2015 points x 2 signals = 4030 values) is
/// inside the same limit, so the refusal is about the output grid, and the same
/// experiment at 100 ns still succeeds under those limits.
#[test]
fn an_oversized_output_grid_is_rejected_and_truncates_nothing() {
    let (program, sm, _) = compiled();
    let tight = Limits::for_tests();

    // The coarse run proves the tight limits still allow this circuit and this
    // solve; only the 1 ps output grid is over budget.
    let ok = match run(&program, &sm, "coarse", &tight) {
        Ok(o) => o,
        Err(d) => panic!(
            "the coarse run must fit inside the test limits:\n{}",
            d.render(&sm)
        ),
    };
    assert_eq!(ok.output_datasets[0].axis.len(), 21);
    assert!(ok.datasets[0].axis.len() > 1000);
    let raw_values: u64 = ok.datasets[0].signals.iter().map(|s| s.len() as u64).sum();
    println!(
        "[limit] coarse under for_tests: raw={} values={} max_result_values={}",
        ok.datasets[0].axis.len(),
        raw_values,
        tight.max_result_values
    );
    assert!(
        raw_values <= tight.max_result_values,
        "the raw solve must fit the limit; otherwise this test would prove the wrong thing"
    );

    let err = match run(&program, &sm, "toofine", &tight) {
        Ok(o) => panic!(
            "a 1 ps output grid must be refused under max_result_values={}, but the run \
             succeeded with {} output points",
            tight.max_result_values,
            o.output_datasets[0].axis.len()
        ),
        Err(d) => d,
    };
    let text = err.render(&sm);
    println!(
        "[limit] toofine under for_tests: {}",
        text.lines().next().unwrap_or("")
    );
    assert!(
        err.iter().any(|d| d.code == Code::Limit),
        "expected E_LIMIT for the oversized output grid, got:\n{text}"
    );
    assert!(
        text.contains("no values were truncated"),
        "the refusal must state that nothing was truncated; got:\n{text}"
    );
    assert!(
        text.contains("2000001") || text.contains("2000000"),
        "the diagnostic should report the grid size it refused; got:\n{text}"
    );
}
