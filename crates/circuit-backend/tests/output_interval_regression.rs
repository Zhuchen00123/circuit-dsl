//! Backend-level regression for the Task-A contract:
//! **`output_interval` is an output-sampling request, never a solver
//! parameter.**
//!
//! # Why this file exists
//!
//! Until this round the TRAN adapter mapped `TranSpec.output_interval` onto the
//! engine's `.tran` print step (`CqTran.step`), and that step is also the lower
//! bound the engine clamps a PULSE `rise`/`fall` to
//! (`thevenin-0.5.0/src/waveform.rs`: `tr.unwrap_or(tran.tstep).max(tran.tstep)`
//! in `evaluate`). Asking for a coarser *output* therefore silently rewrote the
//! *stimulus*: with `rise: 10.ns` and `output_interval: 100.ns` the delivered
//! edge became 100 ns wide, and the raw trace changed with it.
//!
//! The adapter now derives the print step from the declared waveform and the
//! window only (`thevenin.rs::print_step_for` -> `tran_step_for`:
//! `h_print = min(span/1000, min declared rise/fall/period)`), the engine keeps
//! its own grid, and `output_interval` is applied afterwards by
//! `circuit_results::resample`. This file pins that behaviour on the product
//! path, using the same RC the CLI reproduction uses:
//!
//! ```text
//! v1 (PULSE 0->1 V, rise = fall = 10 ns, width = 10 us, period = 20 us)
//!   -> r1 (1 kohm) -> out, c1 (100 nF) -> gnd        tau = R*C = 100 us
//! tran start: 0, stop: 2 us, max_step: 1 ns
//! output_interval: absent / 1 ns / 100 ns
//! ```
//!
//! What the three runs must agree on — and what the fixed contract makes them
//! agree on — is everything that is physics: the time axis, every sample, the
//! solver settings the run reports, and the input edge. The *only* difference
//! the three results may show is the `tran.output_interval` metadata entry
//! (and, in the session layer, the resampled output view).
//!
//! # Historical baseline this file discriminates against
//!
//! Before the fix, with `output_interval: 100.ns` the same experiment produced
//! 2015 raw points whose sample nearest 50 ns carried `v(vin) = 0.5002375000000003`
//! instead of 1 V (a 100 ns ramp observed at 50 ns), and the fine/coarse raw
//! traces differed everywhere (see `docs/review-evidence/round2/repro-baseline.md`
//! and `target/round2-evidence/repro/out-coarse/coarse.tran1.csv`). The
//! assertion `v(vin) == 1.0` at the raw sample nearest 50 ns is therefore the
//! one that flips when the interval is allowed back into the solver; the test
//! `a_declared_ten_nanosecond_edge_is_delivered_on_the_raw_grid` is that
//! assertion, and it is stated for all three intervals.
//!
//! # Product-path tolerance channel: none (do not misread this file)
//!
//! `build_circuit` builds its `CqCircuit` with `options: Vec::new()`, so the
//! product path exposes no RELTOL/ABSTOL/TRTOL. Nothing here sets or claims a
//! solver tolerance. The only solver control reachable from the product path is
//! `TranSpec.max_step -> CqTran.tmax` (`thevenin.rs`, the `AnalysisKind::Tran`
//! arm of `map_analysis`), which
//! `max_step_still_bounds_the_solver_grid` exercises.
//!
//! The IR/plan helpers are copied from `crates/circuit-backend/tests/adapter.rs`
//! (as `transient_reference_regression.rs` does) so this file stays independent
//! of a file another agent owns.

use std::collections::HashMap;

use circuit_backend::backend::SimulationBackend;
use circuit_backend::thevenin::TheveninBackend;
use circuit_core::diagnostic::Code;
use circuit_core::ir::{
    Circuit, Device, DeviceKind, Node, NodeKind, SourceSpec, Waveform, terminal,
};
use circuit_core::plan::{AnalysisKind, AnalysisPlan, AnalysisTask, NamedProbe, Probe, TranSpec};
use circuit_core::span::SourceSpan;
use circuit_core::units::Quantity;
use circuit_core::{AnalysisId, CircuitId, DeviceId, Limits, NodeId};
use circuit_results::dataset::{Data, Dataset};

// ---------------------------------------------------------------------------
// Fixed setup (identical to target/round2-evidence/repro/pulse-*.cdsl)
// ---------------------------------------------------------------------------

const R_OHMS: f64 = 1_000.0;
const C_FARADS: f64 = 100e-9;
// `tau = R*C = 100 us`; the window below is 2 us, so the RC is still climbing.
const HIGH_V: f64 = 1.0;
/// Declared PULSE rise (and fall).
const RISE_S: f64 = 10e-9;
const FALL_S: f64 = 10e-9;
const WIDTH_S: f64 = 10e-6;
const PERIOD_S: f64 = 20e-6;
/// `tran stop: 2.us`.
const STOP_S: f64 = 2e-6;
/// `tran max_step: 1.ns`.
const MAX_STEP_S: f64 = 1e-9;
/// The window is shorter than `tau`, so the RC is still in its ramp.
const PROBE_TIME_S: f64 = 50e-9;
/// The value the *old* contract delivered at `PROBE_TIME_S` for a 100 ns output
/// interval (a 100 ns ramp observed at 50 ns). Recorded only for the report and
/// for the discriminating comment; nothing asserts on it.
#[allow(dead_code)]
const OLD_COARSE_VALUE_AT_50NS: f64 = 0.5002375000000003;

// ---------------------------------------------------------------------------
// IR / plan builders (copied from crates/circuit-backend/tests/adapter.rs)
// ---------------------------------------------------------------------------

/// Copied from `adapter.rs::node`.
fn node(id: u32, name: &str) -> Node {
    Node {
        id: NodeId(id),
        name: name.to_string(),
        local_name: name.to_string(),
        kind: if id == 0 {
            NodeKind::Ground
        } else {
            NodeKind::Normal
        },
        span: SourceSpan::synthetic(),
    }
}

/// Copied from `adapter.rs::two_terminal`.
fn two_terminal(
    id: u32,
    name: &str,
    kind: DeviceKind,
    p: u32,
    n: u32,
    value: Option<Quantity>,
) -> Device {
    Device {
        id: DeviceId(id),
        kind,
        local_name: name.to_string(),
        name: name.to_string(),
        terminals: vec![
            (terminal::POS.to_string(), NodeId(p)),
            (terminal::NEG.to_string(), NodeId(n)),
        ],
        params: value
            .map(|v| HashMap::from([("value".to_string(), v)]))
            .unwrap_or_default(),
        model: None,
        source: None,
        def_span: SourceSpan::synthetic(),
        instance_path: Vec::new(),
    }
}

/// Copied from `adapter.rs::source`.
fn source(id: u32, name: &str, p: u32, n: u32, spec: SourceSpec) -> Device {
    let mut d = two_terminal(id, name, DeviceKind::VoltageSource, p, n, None);
    d.source = Some(spec);
    d
}

/// Copied from `adapter.rs::circuit`.
fn circuit(name: &str, nodes: Vec<Node>, devices: Vec<Device>) -> Circuit {
    Circuit::new(
        CircuitId(0),
        name.to_string(),
        nodes,
        devices,
        Vec::new(),
        SourceSpan::synthetic(),
    )
    .expect("valid circuit")
}

/// Copied from `adapter.rs::probe`.
fn probe(name: &str, p: Probe) -> NamedProbe {
    NamedProbe {
        name: name.to_string(),
        probe: p,
        span: SourceSpan::synthetic(),
    }
}

/// Copied from `adapter.rs::plan_for`.
fn plan_for(name: &str, kind: AnalysisKind, probes: Vec<NamedProbe>) -> AnalysisPlan {
    AnalysisPlan {
        name: name.to_string(),
        circuit_name: name.to_string(),
        tasks: vec![AnalysisTask {
            id: AnalysisId(0),
            kind,
            probes,
            implicit_probes: Vec::new(),
            span: SourceSpan::synthetic(),
        }],
        param_overrides: Vec::new(),
        derives: Vec::new(),
        measures: Vec::new(),
        span: SourceSpan::synthetic(),
    }
}

/// Copied from `adapter.rs::be`.
fn be() -> TheveninBackend {
    TheveninBackend::with_limits(Limits::default())
}

/// Copied from `adapter.rs::real`.
fn real(d: &Dataset, signal: &str) -> Vec<f64> {
    match &d
        .signal(signal)
        .unwrap_or_else(|| panic!("signal {signal}"))
        .data
    {
        Data::Real(v) => v.clone(),
        Data::Complex(_) => panic!("{signal} is complex"),
    }
}

// ---------------------------------------------------------------------------
// Circuits and plans for this file
// ---------------------------------------------------------------------------

/// The reproduction's RC: `v1` (PULSE) -> `r1` -> `out`, `c1` -> gnd.
///
/// `dc` is 0 V so the operating point the transient starts from is the zero
/// state the reproduction assumes (`v(out) = 0` at `t = 0`).
fn rc_circuit(name: &str) -> Circuit {
    let src = source(
        0,
        "v1",
        1,
        0,
        SourceSpec {
            dc: Some(Quantity::volts(0.0)),
            ac: None,
            waveform: Some(Waveform::Pulse {
                low: Quantity::volts(0.0),
                high: Quantity::volts(HIGH_V),
                delay: Quantity::seconds(0.0),
                rise: Quantity::seconds(RISE_S),
                fall: Quantity::seconds(FALL_S),
                width: Quantity::seconds(WIDTH_S),
                period: Quantity::seconds(PERIOD_S),
            }),
        },
    );
    circuit(
        name,
        vec![node(0, "gnd"), node(1, "vin"), node(2, "out")],
        vec![
            src,
            two_terminal(
                1,
                "r1",
                DeviceKind::Resistor,
                1,
                2,
                Some(Quantity::ohms(R_OHMS)),
            ),
            two_terminal(
                2,
                "c1",
                DeviceKind::Capacitor,
                2,
                0,
                Some(Quantity::farads(C_FARADS)),
            ),
        ],
    )
}

/// The same topology with a caller-supplied source, for the attribution cases
/// where the presence or absence of a declared waveform is the variable.
fn rc_circuit_with(name: &str, src: Device) -> Circuit {
    circuit(
        name,
        vec![node(0, "gnd"), node(1, "vin"), node(2, "out")],
        vec![
            src,
            two_terminal(
                1,
                "r1",
                DeviceKind::Resistor,
                1,
                2,
                Some(Quantity::ohms(R_OHMS)),
            ),
            two_terminal(
                2,
                "c1",
                DeviceKind::Capacitor,
                2,
                0,
                Some(Quantity::farads(C_FARADS)),
            ),
        ],
    )
}

/// A TRAN task over `[0, stop]` probing both the drive and the response.
fn tran_plan(
    name: &str,
    stop_s: f64,
    max_step: Option<f64>,
    output_interval: Option<f64>,
) -> AnalysisPlan {
    plan_for(
        name,
        AnalysisKind::Tran(TranSpec {
            start_s: 0.0,
            stop_s,
            max_step,
            output_interval,
            uic: false,
            span: SourceSpan::synthetic(),
        }),
        vec![
            probe("v(vin)", Probe::NodeVoltage(NodeId(1))),
            probe("v(out)", Probe::NodeVoltage(NodeId(2))),
        ],
    )
}

/// Run one case and return its single dataset (the raw solver output).
fn run_dataset(c: &Circuit, plan: &AnalysisPlan) -> Dataset {
    let out = be().run(c, plan).expect("tran runs");
    assert_eq!(
        out.datasets.len(),
        1,
        "this file expects exactly one dataset per run"
    );
    out.datasets.into_iter().next().expect("one dataset")
}

/// The reproduction's own combination, parameterised by `output_interval`.
fn run_rc(output_interval: Option<f64>) -> Dataset {
    let c = rc_circuit("rc_output_interval");
    run_dataset(
        &c,
        &tran_plan(
            "rc_output_interval",
            STOP_S,
            Some(MAX_STEP_S),
            output_interval,
        ),
    )
}

// ---------------------------------------------------------------------------
// Comparison helpers
// ---------------------------------------------------------------------------

/// Assert two traces are **bit for bit** identical (axis and every sample).
///
/// `to_bits` rather than `==`: this is the "changing the output request did not
/// perturb the solve" claim, and `-0.0 == 0.0` under `==` would hide a sign
/// change.
fn assert_trace_bits_equal(label: &str, a: &Dataset, b: &Dataset) {
    let (ta, tb) = (a.axis.samples(), b.axis.samples());
    assert_eq!(
        ta.len(),
        tb.len(),
        "{label}: axis lengths differ ({} vs {})",
        ta.len(),
        tb.len()
    );
    for (i, (x, y)) in ta.iter().zip(tb).enumerate() {
        assert_eq!(
            x.to_bits(),
            y.to_bits(),
            "{label}: time[{i}] differs: {x:?} vs {y:?}"
        );
    }
    assert_eq!(
        a.signals.len(),
        b.signals.len(),
        "{label}: signal counts differ"
    );
    for (sa, sb) in a.signals.iter().zip(&b.signals) {
        assert_eq!(
            sa.name, sb.name,
            "{label}: signal names differ ({} vs {})",
            sa.name, sb.name
        );
        match (&sa.data, &sb.data) {
            (Data::Real(x), Data::Real(y)) => {
                assert_eq!(x.len(), y.len(), "{label}: `{}` lengths differ", sa.name);
                for (i, (vx, vy)) in x.iter().zip(y).enumerate() {
                    assert_eq!(
                        vx.to_bits(),
                        vy.to_bits(),
                        "{label}: `{}`[{i}] differs: {vx:?} vs {vy:?}",
                        sa.name
                    );
                }
            }
            _ => panic!("{label}: `{}` is not a real signal", sa.name),
        }
    }
}

/// The settings of `d`, with every entry whose key is `ignore` removed.
fn settings_except(d: &Dataset, ignore: &str) -> Vec<(String, String)> {
    d.backend
        .settings
        .iter()
        .filter(|(k, _)| k != ignore)
        .cloned()
        .collect()
}

/// Read a numeric setting, or panic naming the missing key.
fn setting_f64(d: &Dataset, key: &str) -> f64 {
    let raw = d
        .backend
        .setting(key)
        .unwrap_or_else(|| panic!("missing setting `{key}` in {:?}", d.backend.settings));
    raw.parse::<f64>()
        .unwrap_or_else(|e| panic!("setting `{key}` = {raw:?} is not a number: {e}"))
}

/// Index of the sample nearest `target` (ties resolve to the earlier sample).
fn nearest_index(times: &[f64], target: f64) -> usize {
    assert!(!times.is_empty(), "empty axis");
    let mut best = 0usize;
    for (i, &t) in times.iter().enumerate() {
        if (t - target).abs() < (times[best] - target).abs() {
            best = i;
        }
    }
    best
}

// ---------------------------------------------------------------------------
// 1. The physics is independent of the output request
// ---------------------------------------------------------------------------

/// The three runs differ only in `tran.output_interval`, and that difference
/// must stop at the metadata: the solved time axis and every sample are bit for
/// bit identical.
///
/// This is the direct evidence that no output request reaches the solver. If
/// `output_interval` were mapped back onto `CqTran.step` (the bug this round
/// removed), the engine's PULSE clamp would widen the input edge for the coarse
/// run: `v(vin)` at 50 ns would become 0.50023... instead of 1 V and the two
/// traces would diverge.
#[test]
fn output_interval_does_not_change_the_solved_trace() {
    let none = run_rc(None);
    let fine = run_rc(Some(1e-9));
    let coarse = run_rc(Some(100e-9));

    println!(
        "[trace] points: none={} fine={} coarse={}",
        none.axis.len(),
        fine.axis.len(),
        coarse.axis.len()
    );
    println!("[trace] settings none  = {:?}", none.backend.settings);
    println!("[trace] settings fine  = {:?}", fine.backend.settings);
    println!("[trace] settings coarse= {:?}", coarse.backend.settings);

    assert!(
        none.axis.len() > 1000,
        "the raw solver grid should be dense (max_step = 1 ns over 2 us), got {} points",
        none.axis.len()
    );

    assert_trace_bits_equal("fine vs none", &fine, &none);
    assert_trace_bits_equal("coarse vs none", &coarse, &none);

    // The only metadata that may differ is the recorded output request itself.
    assert_eq!(
        settings_except(&fine, "tran.output_interval"),
        settings_except(&none, "tran.output_interval"),
        "the coarse/fine runs must not change any solver setting"
    );
    assert_eq!(
        settings_except(&coarse, "tran.output_interval"),
        settings_except(&none, "tran.output_interval"),
        "the coarse run must not change any solver setting"
    );
    assert_eq!(
        none.backend.setting("tran.output_interval"),
        None,
        "an absent output_interval must not be recorded"
    );
    assert_eq!(
        fine.backend.setting("tran.output_interval"),
        Some("0.000000001")
    );
    assert_eq!(
        coarse.backend.setting("tran.output_interval"),
        Some("0.0000001")
    );
}

// ---------------------------------------------------------------------------
// 2. The declared edge is delivered (the assertion that flips on regression)
// ---------------------------------------------------------------------------

/// The declared 10 ns rise must be visible in the raw trace: the sample nearest
/// 50 ns carries `v(vin) = 1 V`, for **every** output interval.
///
/// Regression direction: mapping `output_interval` onto the print step makes the
/// engine widen the edge to that step, so with `output_interval: 100 ns` the
/// same sample carried `v(vin) = 0.5002375000000003` (a 100 ns ramp at 50 ns)
/// and this assertion fails. That number is the historical baseline in
/// `docs/review-evidence/round2/repro-baseline.md`.
///
/// The check is on the **raw** dataset, which is what the solver produced: the
/// output view is allowed to be coarse, the physics is not.
#[test]
fn a_declared_ten_nanosecond_edge_is_delivered_on_the_raw_grid() {
    for (label, interval) in [("none", None), ("1ns", Some(1e-9)), ("100ns", Some(100e-9))] {
        let d = run_rc(interval);
        let times = d.axis.samples();
        let vin = real(&d, "v(vin)");
        let i = nearest_index(times, PROBE_TIME_S);
        let (t_at, v_at) = (times[i], vin[i]);
        println!("[edge/{label}] nearest to 50 ns: sample {i} at t={t_at:?} s, v(vin)={v_at:?} V");

        assert!(
            (t_at - PROBE_TIME_S).abs() <= MAX_STEP_S,
            "[{label}] no raw sample within one max_step (1 ns) of 50 ns: nearest is {t_at:?} s"
        );
        // 10 ns of declared rise, 50 ns of simulated time: the drive is fully
        // high. A widened edge shows up here as a fraction of 1 V.
        assert_eq!(
            v_at, HIGH_V,
            "[{label}] v(vin) at t={t_at:?} s is {v_at:?} V, expected {HIGH_V} V; \
             a value near 0.5 V means the 10 ns edge was widened to the output interval"
        );
        assert!(
            (v_at - OLD_COARSE_VALUE_AT_50NS).abs() > 0.4,
            "[{label}] v(vin) at 50 ns matches the pre-fix coarse value {OLD_COARSE_VALUE_AT_50NS} V"
        );

        // And the drive really is low at the start of the window: the plateau
        // above is not an artefact of a constant axis.
        let first = real(&d, "v(vin)")[0];
        assert_eq!(first, 0.0, "[{label}] v(vin) at t=0 must be 0 V");
    }
}

// ---------------------------------------------------------------------------
// 3. The solve is reported honestly
// ---------------------------------------------------------------------------

/// The metadata must describe the solve that happened, not the output request:
///
/// * `tran.solver_step` is the print step handed to the engine and does not
///   depend on `output_interval` (2 ns here: `span/1000`, which is finer than
///   the declared 10 ns edge, so the engine's PULSE clamp cannot bind);
/// * `tran.solve_points` equals the raw axis length, i.e. the solver's own grid
///   — 2015 points measured here, never the 21 points of a 100 ns output grid;
/// * `tran.waveform_bound` is the smallest declared PULSE timing (10 ns);
/// * `tran.max_step`/`tran.output_interval` echo the request.
#[test]
fn solver_metadata_describes_the_solve_not_the_output_request() {
    let none = run_rc(None);
    let fine = run_rc(Some(1e-9));
    let coarse = run_rc(Some(100e-9));

    for (label, d) in [("none", &none), ("1ns", &fine), ("100ns", &coarse)] {
        let solver_step = setting_f64(d, "tran.solver_step");
        let solve_points = setting_f64(d, "tran.solve_points");
        let waveform_bound = setting_f64(d, "tran.waveform_bound");
        let max_step = setting_f64(d, "tran.max_step");
        println!(
            "[meta/{label}] solver_step={solver_step:?} solve_points={solve_points} \
             waveform_bound={waveform_bound:?} max_step={max_step:?} points={}",
            d.axis.len()
        );

        assert_eq!(
            solver_step,
            STOP_S / 1000.0,
            "[{label}] tran.solver_step must be min(span/1000, declared 10 ns) = 2 ns"
        );
        assert_eq!(
            solve_points as usize,
            d.axis.len(),
            "[{label}] tran.solve_points must equal the raw axis length"
        );
        assert!(
            solve_points > 1000.0,
            "[{label}] tran.solve_points reports the output request, not the solve: {solve_points}"
        );
        assert_eq!(
            waveform_bound, RISE_S,
            "[{label}] tran.waveform_bound must be the smallest declared PULSE timing"
        );
        assert_eq!(
            max_step, MAX_STEP_S,
            "[{label}] tran.max_step must echo the request"
        );
    }

    // Independent of the output request: same value in all three runs.
    assert_eq!(
        setting_f64(&none, "tran.solver_step"),
        setting_f64(&coarse, "tran.solver_step"),
        "tran.solver_step must not move with output_interval"
    );
    assert_eq!(
        setting_f64(&none, "tran.solve_points"),
        setting_f64(&coarse, "tran.solve_points"),
        "the number of solved points must not move with output_interval"
    );
    // The output grid metadata is absent here: the resampling happens in the
    // session layer, and the backend must not pretend it did it.
    for (label, d) in [("none", &none), ("1ns", &fine), ("100ns", &coarse)] {
        assert_eq!(
            d.backend.setting("tran.output_grid"),
            None,
            "[{label}] the backend must not claim to have resampled"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. `max_step` still reaches the engine
// ---------------------------------------------------------------------------

/// `max_step` is the one solver control the product path exposes
/// (`TranSpec.max_step -> CqTran.tmax -> h_max`). With the output request no
/// longer in the solver, a smaller cap must still produce a denser grid: three
/// settings, strictly increasing point counts, and the same 1 V drive at 50 ns
/// in every one of them.
///
/// Only the point counts are compared; no threshold is asserted against them,
/// because the engine's grid is adaptive (LTE-controlled with a first step of
/// `h_max/400`), so absolute counts are an implementation detail while
/// monotonicity is the contract.
#[test]
fn max_step_still_bounds_the_solver_grid() {
    let settings = [4e-9, 1e-9, 250e-12];
    let mut counts = Vec::new();
    for max_step in settings {
        let c = rc_circuit("rc_max_step");
        let d = run_dataset(
            &c,
            &tran_plan("rc_max_step", STOP_S, Some(max_step), Some(100e-9)),
        );
        let times = d.axis.samples();
        let vin = real(&d, "v(vin)");
        let i = nearest_index(times, PROBE_TIME_S);
        let (t_at, v_at) = (times[i], vin[i]);
        // The first step is h_max/400 and the grid is adaptive, so the sample
        // spacing is at most max_step.
        assert!(
            (t_at - PROBE_TIME_S).abs() <= max_step,
            "max_step={max_step:e}: no raw sample within one max_step of 50 ns \
             (nearest {t_at:?} s)"
        );
        assert_eq!(
            v_at, HIGH_V,
            "max_step={max_step:e}: v(vin) at {t_at:?} s is {v_at:?} V; the declared 10 ns \
             edge must hold at every step size"
        );
        assert_eq!(
            setting_f64(&d, "tran.solver_step"),
            STOP_S / 1000.0,
            "max_step={max_step:e}: the print step is derived from the waveform, not max_step"
        );
        println!(
            "[max_step] max_step={max_step:e} s -> {} raw points, v(vin)@{t_at:?} = {v_at:?} V",
            times.len()
        );
        counts.push(times.len());
    }

    println!("[max_step] raw point counts (coarsest -> finest): {counts:?}");
    assert!(
        counts[0] > 100,
        "the coarsest setting should still resolve the window, got {counts:?}"
    );
    for w in counts.windows(2) {
        assert!(
            w[1] > w[0],
            "a smaller max_step must produce a denser grid; point counts {counts:?} are not \
             strictly increasing, so max_step did not reach the engine"
        );
    }
}

// ---------------------------------------------------------------------------
// 5. Capability refusals (never a silently widened edge)
// ---------------------------------------------------------------------------

/// The other half of "the edge is never widened quietly": when the declared
/// waveform cannot be honoured, the backend refuses the run.
///
/// * a declared zero rise is refused (`E_UNSUPPORTED`): the engine would
///   substitute its own print step for it, silently changing the stimulus;
/// * a window that would need more than the step budget is refused
///   (`E_LIMIT`): a 1 ps edge over a 10 us resistive window is ~1e7 steps.
///
/// Both are checked on the product path, so a regression that reintroduced the
/// silent clamp would have to delete one of these refusals first.
#[test]
fn an_edge_that_cannot_be_honoured_is_refused_not_widened() {
    // (a) declared zero rise.
    let zero_src = source(
        0,
        "v1",
        1,
        0,
        SourceSpec {
            dc: Some(Quantity::volts(0.0)),
            ac: None,
            waveform: Some(Waveform::Pulse {
                low: Quantity::volts(0.0),
                high: Quantity::volts(HIGH_V),
                delay: Quantity::seconds(0.0),
                rise: Quantity::seconds(0.0),
                fall: Quantity::seconds(FALL_S),
                width: Quantity::seconds(WIDTH_S),
                period: Quantity::seconds(PERIOD_S),
            }),
        },
    );
    let c = circuit(
        "rc_zero_rise",
        vec![node(0, "gnd"), node(1, "vin"), node(2, "out")],
        vec![
            zero_src,
            two_terminal(
                1,
                "r1",
                DeviceKind::Resistor,
                1,
                2,
                Some(Quantity::ohms(R_OHMS)),
            ),
            two_terminal(
                2,
                "c1",
                DeviceKind::Capacitor,
                2,
                0,
                Some(Quantity::farads(C_FARADS)),
            ),
        ],
    );
    let plan = tran_plan("rc_zero_rise", STOP_S, Some(MAX_STEP_S), None);
    let err = be()
        .run(&c, &plan)
        .expect_err("a declared zero rise must be refused");
    let text = err.render_plain();
    println!("[refusal/zero-rise] {text}");
    assert!(
        err.iter().any(|d| d.code == Code::Unsupported),
        "expected E_UNSUPPORTED for a declared zero rise, got: {text}"
    );
    assert!(text.contains("rise"), "{text}");

    // (b) a window too long for the declared edge. Resistive only, so nothing
    // but the declared waveform bounds the step; 10 us / 1 ps = 1e7 steps.
    let fine_src = source(
        0,
        "v1",
        1,
        0,
        SourceSpec {
            dc: Some(Quantity::volts(0.0)),
            ac: None,
            waveform: Some(Waveform::Pulse {
                low: Quantity::volts(0.0),
                high: Quantity::volts(HIGH_V),
                delay: Quantity::seconds(0.0),
                rise: Quantity::seconds(1e-12),
                fall: Quantity::seconds(1e-12),
                width: Quantity::seconds(WIDTH_S),
                period: Quantity::seconds(PERIOD_S),
            }),
        },
    );
    let c = circuit(
        "resistive_fine_edge",
        vec![node(0, "gnd"), node(1, "vin"), node(2, "out")],
        vec![
            fine_src,
            two_terminal(
                1,
                "r1",
                DeviceKind::Resistor,
                1,
                2,
                Some(Quantity::ohms(R_OHMS)),
            ),
            two_terminal(
                2,
                "r2",
                DeviceKind::Resistor,
                2,
                0,
                Some(Quantity::ohms(R_OHMS)),
            ),
        ],
    );
    let plan = tran_plan("resistive_fine_edge", 10e-6, None, None);
    let err = be()
        .run(&c, &plan)
        .expect_err("a 1 ps edge over a 10 us window must hit the step budget");
    let text = err.render_plain();
    println!("[refusal/step-budget] {text}");
    assert!(
        err.iter().any(|d| d.code == Code::Limit),
        "expected E_LIMIT for the step budget, got: {text}"
    );
}

/// The step budget refuses **a declared waveform timing**, never the user's own
/// `max_step`.
///
/// Found in review (W6-2): an earlier revision of the guard compared
/// `span / effective_step` alone, so a purely resistive source set — a circuit
/// with no waveform at all — was refused with a message blaming "the declared
/// source timings" whenever `max_step` was small, even though that cost is
/// exactly what the user asked for. The two cases below are the attribution
/// boundary, and they are checked through `validate` (which `check` and `run`
/// both call) because the passing case is deliberately a 1e9-step run that
/// must not be executed here.
#[test]
fn the_step_budget_blames_the_waveform_not_the_users_max_step() {
    // (a) No waveform at all: a DC source, an RC and a tiny explicit max_step.
    // 1 s / 1 ns = 1e9 steps, and refusing that is not this contract's job.
    let dc_only = source(
        0,
        "v1",
        1,
        0,
        SourceSpec {
            dc: Some(Quantity::volts(HIGH_V)),
            ac: None,
            waveform: None,
        },
    );
    let c = rc_circuit_with("dc_only_tiny_max_step", dc_only);
    let plan = tran_plan("dc_only_tiny_max_step", 1.0, Some(1e-9), None);
    assert!(
        be().validate(&c, &plan).is_ok(),
        "a small max_step is the user's own budget and must not be refused as a \
         waveform error: {:?}",
        be().validate(&c, &plan).err().map(|d| d.render_plain())
    );

    // (b) The same budget, but now a declared 1 ps edge forces it. This one is
    // refused, and the message must name the declared timing (not max_step,
    // which is absent).
    let fine = source(
        0,
        "v1",
        1,
        0,
        SourceSpec {
            dc: Some(Quantity::volts(0.0)),
            ac: None,
            waveform: Some(Waveform::Pulse {
                low: Quantity::volts(0.0),
                high: Quantity::volts(HIGH_V),
                delay: Quantity::seconds(0.0),
                rise: Quantity::seconds(1e-12),
                fall: Quantity::seconds(1e-12),
                width: Quantity::seconds(WIDTH_S),
                period: Quantity::seconds(PERIOD_S),
            }),
        },
    );
    let c = rc_circuit_with("dc_and_fine_edge", fine);
    let plan = tran_plan("dc_and_fine_edge", 1.0, None, None);
    let err = be()
        .validate(&c, &plan)
        .expect_err("a declared 1 ps edge over a 1 s window must be refused");
    let text = err.render_plain();
    println!("[attribution/declared-edge] {text}");
    assert!(
        err.iter().any(|d| d.code == Code::Limit),
        "expected E_LIMIT, got: {text}"
    );
    assert!(
        text.contains("declared source rise/fall/period"),
        "the message must blame the declared waveform timing: {text}"
    );
    assert!(
        text.contains("declared waveform timing"),
        "the context must name the declared waveform timing: {text}"
    );

    // (c) Boundary control: steps = span / step = exactly 1e6 is not "over the
    // limit", so it stays accepted even without any waveform.
    let c = rc_circuit_with("dc_only_exact_budget", {
        source(
            0,
            "v1",
            1,
            0,
            SourceSpec {
                dc: Some(Quantity::volts(HIGH_V)),
                ac: None,
                waveform: None,
            },
        )
    });
    let plan = tran_plan("dc_only_exact_budget", 1.0, Some(1e-6), None);
    assert!(
        be().validate(&c, &plan).is_ok(),
        "exactly 1e6 steps is at the limit, not over it"
    );
}
