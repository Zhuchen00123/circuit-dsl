//! Product-path regression for the **RC finite-ramp** transient response.
//!
//! Scope: this file drives the *product* adapter path only —
//! project IR (`circuit_core::ir`) -> `TheveninBackend` -> `Dataset`
//! (`circuit_results::dataset`). It does not touch `cirq_ir` and it does not
//! build SPICE/Cirq text.
//!
//! # Why this file exists (the evidence it replaces)
//!
//! The Phase-0 probe (`_probe/src/main.rs:245-339`) compared an RC transient
//! against the **ideal step** response `1 - exp(-t/tau)` while declaring a
//! 1 ps PULSE rise, and reported a worst `|diff| = 1.946e-3 V`, attributed to
//! "finite rise time and output-sample alignment". Both halves of that
//! attribution were wrong:
//!
//! * the comparison evaluated the analytic solution at the *engine's own*
//!   sample time, so sample alignment contributed exactly 0;
//! * the engine never saw a 1 ps edge. `thevenin-0.5.0/src/waveform.rs`
//!   evaluates a PULSE as `tr.unwrap_or(tran.tstep).max(tran.tstep)`, i.e. the
//!   rise is clamped **up to the `.tran` print step** — and in the revision
//!   this file was first written against, that print step *was* the adapter's
//!   `output_interval` (the `AnalysisKind::Tran` arm of `map_analysis` in
//!   `crates/circuit-backend/src/thevenin.rs`). With a 500 ns output step the
//!   real input was therefore a **500 ns ramp**, whose response differs from
//!   the ideal step by `C(T) * exp(-u/tau)` with `C(500 ns) = -2.5042e-3 V` —
//!   exactly the observed 1.95e-3 V at 0.25 tau.
//!
//! That attribution is what prompted the fix, and the contract has changed
//! (Task A, `docs/next-iteration-plan.md` §4): the print step is now derived
//! from the declared waveform and the window only (`thevenin.rs::print_step_for`
//! / `tran_step_for`), and `output_interval` is re-applied after the run by
//! `circuit_results::resample`. The adapter therefore **never widens a declared
//! edge**: the effective edge is the *declared* one.
//! `declared_rise_below_output_interval_is_not_widened` below asserts that on
//! the product path, and keeps the old `T_eff = max(declared rise,
//! output_interval)` reference as its discriminating control (against that
//! reference the new run misses by ~1 V on every point after the ramp).
//!
//! The reference solution below is unchanged; only the width it is evaluated at
//! is now the declared rise rather than the output step. Tests 2 and 4 keep
//! `effective_edge(rise, output_interval)` because there the declared rise is
//! the larger of the two, so the old clamp and the new rule agree on the value —
//! they are the two cases where the old contract was not wrong.
//!
//! # Product-path tolerance channel: none (do not misread this file)
//!
//! `build_circuit` (`crates/circuit-backend/src/thevenin.rs:437`) builds its
//! `CqCircuit` with
//! `options: Vec::new()`, so the product path has **no** RELTOL/ABSTOL/TRTOL
//! channel. Nothing in this file sets a solver tolerance, and no test here may
//! claim one was set. Solver-tolerance sensitivity can only be explored in
//! `_probe`, which constructs `cirq_ir::Circuit` directly. The only solver
//! numeric control reachable from the product path is
//! `TranSpec.max_step -> CqTran.tmax` (`thevenin.rs:781`), which the
//! `max_step_...` test below exercises.
//!
//! # Reference solution (one-pole RC, finite ramp, no load)
//!
//! Circuit: `v1` (PULSE) -> `r1` -> node `out`, `c1` from `out` to ground;
//! `tau = R * C`, output `v(out)` (probing a node voltage). Input: 0 before
//! `t = 0`, linear from 0 to `V0` over `T`, then held at `V0` (the falling
//! edge is outside the window). With `u = t`, `x = u/tau`, `rho = T/tau`:
//!
//!     u <= 0    : y = 0
//!     0 < u <= T: y = (V0/rho) * S(x),   S(x) = x + expm1(-x)
//!     u > T     : y = V0 + a * exp(-(u - T)/tau),  a = (V0/rho) * expm1(-rho)
//!
//! `a` is exactly `y(T) - V0`, and writing it with `expm1` keeps it accurate
//! for tiny rho (the algebraically equivalent `V0*(1 - (1 - exp(-rho))/rho)`
//! cancels catastrophically: at `rho = 1e-8` its absolute error is ~1e-8 V,
//! larger than `y(T) = 5e-9 V` itself). `S` uses the series
//! `S(x) = x^2/2 - x^3/6 + x^4/24 - x^5/120 + x^6/720` for `x < 1e-2`
//! (truncation relative error <= 3.97e-14) and `x + expm1(-x)` otherwise; the
//! naive `x - 1 + exp(-x)` returns exactly 0.0 for `x <= 3e-8` and loses all
//! but ~4 significant digits at `x = 1e-6`.
//!
//! # Acceptance criteria (fixed for this round, deliberately not relaxed here)
//!
//!     |actual - expected| <= atol + rtol * |expected|,  atol = 1e-5 V, rtol = 1e-3
//!
//! For `tau = 100 us, V0 = 1 V, T = 1 us` the allowance is
//! 1.005e-5 V at `u/tau = 0.001`, 1.49834e-5 V at 0.01, 1.00623e-4 at 0.1,
//! 6.40275e-4 at 1, 8.73986e-4 at 2 and 1.00323e-3 V at 5
//! (`docs/review-evidence/rc-reference-math.md` section 6). The largest
//! allowance this criterion can ever produce is `atol + rtol*V0 = 1.01e-3 V`.
//!
//! The helpers marked "copied from adapter.rs" mirror
//! `crates/circuit-backend/tests/adapter.rs` so this file can stay independent
//! of it; that file is owned by another agent and is not modified here.

use std::collections::HashMap;

use circuit_backend::backend::SimulationBackend;
use circuit_backend::thevenin::TheveninBackend;
use circuit_core::ir::{
    Circuit, Device, DeviceKind, Node, NodeKind, SourceSpec, Waveform, terminal,
};
use circuit_core::plan::{AnalysisKind, AnalysisPlan, AnalysisTask, NamedProbe, Probe, TranSpec};
use circuit_core::span::SourceSpan;
use circuit_core::units::Quantity;
use circuit_core::{AnalysisId, CircuitId, DeviceId, Limits, NodeId};
use circuit_results::dataset::{Axis, Data, Dataset};

// ---------------------------------------------------------------------------
// Fixed physical setup
// ---------------------------------------------------------------------------

const R_OHMS: f64 = 1_000.0;
const C_FARADS: f64 = 100e-9;
/// `tau = R*C = 100 us`.
const TAU: f64 = R_OHMS * C_FARADS;
const V0: f64 = 1.0;
/// TRAN voltage `atol` in volts (round criterion).
const ATOL_V: f64 = 1e-5;
/// `rtol`, dimensionless (round criterion).
const RTOL: f64 = 1e-3;

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

/// `v1` (PULSE, `rise` wide) -> `r1` -> `out`, `c1` -> gnd, probing `v(out)`.
///
/// `dc` is 0 V so the operating point the transient starts from is the same
/// zero state the reference solution assumes (`y(0) = 0`).
fn rc_circuit(name: &str, rise_s: f64) -> Circuit {
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
                high: Quantity::volts(V0),
                delay: Quantity::seconds(0.0),
                rise: Quantity::seconds(rise_s),
                fall: Quantity::seconds(rise_s),
                // The falling edge and the next period stay far outside the
                // simulated window, so the reference never sees them.
                width: Quantity::seconds(10.0),
                period: Quantity::seconds(20.0),
            }),
        },
    );
    circuit(
        name,
        vec![node(0, "gnd"), node(1, "in"), node(2, "out")],
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

/// A TRAN task over `[0, stop_s]` with explicit `max_step` and `output_interval`.
fn tran_plan(name: &str, stop_s: f64, max_step: f64, output_interval: f64) -> AnalysisPlan {
    plan_for(
        name,
        AnalysisKind::Tran(TranSpec {
            start_s: 0.0,
            stop_s,
            max_step: Some(max_step),
            output_interval: Some(output_interval),
            uic: false,
            span: SourceSpan::synthetic(),
        }),
        vec![probe("v(out)", Probe::NodeVoltage(NodeId(2)))],
    )
}

/// Run one case and return `(time axis, v(out))`.
fn run_tran(c: &Circuit, plan: &AnalysisPlan) -> (Vec<f64>, Vec<f64>) {
    let out = be().run(c, plan).expect("tran runs");
    let d = &out.datasets[0];
    let t = match &d.axis {
        Axis::Time(t) => t.clone(),
        other => panic!("expected a time axis, got {other:?}"),
    };
    let vout = real(d, "v(out)");
    assert_eq!(
        t.len(),
        vout.len(),
        "axis and signal must have equal length"
    );
    (t, vout)
}

// ---------------------------------------------------------------------------
// Reference solution
// ---------------------------------------------------------------------------

/// `S(x) = x + expm1(-x) = sum_{k>=2} (-1)^k x^k / k!`, evaluated stably.
fn s_stable(x: f64) -> f64 {
    if x < 1e-2 {
        // Series k = 2..6. The first omitted term is x^7/7!; relative to the
        // leading x^2/2 term that is x^5/2520 <= 3.97e-14 at x = 1e-2.
        let mut sum = 0.0;
        let mut power = 1.0; // x^k / k!
        for k in 1..=6u32 {
            power *= x / f64::from(k);
            if k >= 2 {
                if k % 2 == 0 {
                    sum += power;
                } else {
                    sum -= power;
                }
            }
        }
        sum
    } else {
        x + (-x).exp_m1()
    }
}

/// Exact response of the one-pole RC to a ramp of width `t_eff` reaching `v0`.
fn ramp_reference_at(u: f64, v0: f64, tau: f64, t_eff: f64) -> f64 {
    assert!(t_eff > 0.0, "ramp width must be positive");
    if u <= 0.0 {
        return 0.0;
    }
    let rho = t_eff / tau;
    if u <= t_eff {
        (v0 / rho) * s_stable(u / tau)
    } else {
        // a = y(t_eff) - v0 = (v0/rho) * expm1(-rho); stable for tiny rho.
        let a = (v0 / rho) * (-rho).exp_m1();
        v0 + a * (-(u - t_eff) / tau).exp()
    }
}

/// The same reference in this file's fixed units, for a given effective edge.
fn ramp_reference(u: f64, t_eff: f64) -> f64 {
    ramp_reference_at(u, V0, TAU, t_eff)
}

/// The engine clamps the PULSE rise/fall up to the `.tran` print step
/// (`thevenin-0.5.0/src/waveform.rs`: `tr.unwrap_or(tran.tstep).max(tran.tstep)`),
/// and an earlier revision of the adapter mapped `TranSpec.output_interval`
/// onto that step — hence the name.
///
/// Kept because tests 2 and 4 below use it for the case in which the two rules
/// agree: a declared rise **at least as wide as** the output interval, where
/// `max(declared rise, output_interval)` is simply the declared rise. The test
/// that pins the corrected rule (the output interval never widens an edge) uses
/// the declared rise directly and is
/// `declared_rise_below_output_interval_is_not_widened`.
fn effective_edge(declared_rise_s: f64, output_interval_s: f64) -> f64 {
    declared_rise_s.max(output_interval_s)
}

fn allowance(expected: f64) -> f64 {
    ATOL_V + RTOL * expected.abs()
}

// ---------------------------------------------------------------------------
// Criteria measurement and axis contract
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Fit {
    points: usize,
    max_error: f64,
    max_error_at: f64,
    max_error_expected: f64,
    allowance_at_max: f64,
    violations: usize,
    worst_ratio: f64,
    worst_ratio_at: f64,
}

impl Fit {
    fn summary(&self) -> String {
        format!(
            "points={} max|err|={:.6e} V at t={:.6e} s (expected={:.9} V, allowance={:.6e} V) \
             violations={} worst ratio={:.3}x at t={:.6e} s (max possible allowance={:.6e} V)",
            self.points,
            self.max_error,
            self.max_error_at,
            self.max_error_expected,
            self.allowance_at_max,
            self.violations,
            self.worst_ratio,
            self.worst_ratio_at,
            ATOL_V + RTOL * V0,
        )
    }
}

/// Compare every returned point against `ramp_reference(_, t_eff)`, evaluating
/// the reference at the time the engine actually returned.
fn fit_against(t: &[f64], got: &[f64], t_eff: f64) -> Fit {
    assert_eq!(t.len(), got.len(), "axis/signal length mismatch");
    let mut f = Fit {
        points: t.len(),
        max_error: 0.0,
        max_error_at: f64::NAN,
        max_error_expected: f64::NAN,
        allowance_at_max: f64::NAN,
        violations: 0,
        worst_ratio: 0.0,
        worst_ratio_at: f64::NAN,
    };
    for (i, (&ti, &gi)) in t.iter().zip(got.iter()).enumerate() {
        assert!(ti.is_finite() && gi.is_finite(), "non-finite sample at {i}");
        let expected = ramp_reference(ti, t_eff);
        let err = (gi - expected).abs();
        let allow = allowance(expected);
        if err > allow {
            f.violations += 1;
        }
        if err > f.max_error {
            f.max_error = err;
            f.max_error_at = ti;
            f.max_error_expected = expected;
            f.allowance_at_max = allow;
        }
        let ratio = err / allow;
        if ratio > f.worst_ratio {
            f.worst_ratio = ratio;
            f.worst_ratio_at = ti;
        }
    }
    f
}

/// Assert the fixed criterion on **every** returned point. The thresholds are
/// the round's; they are not adjusted here. On failure the message carries the
/// measured numbers so the failure is reproducible without rerunning.
fn assert_within_criteria(label: &str, t: &[f64], got: &[f64], t_eff: f64) -> Fit {
    let fit = fit_against(t, got, t_eff);
    println!("[{label}] {}", fit.summary());
    assert_eq!(
        fit.violations,
        0,
        "{label}: {} of {} returned points violate |actual - expected| <= atol + rtol*|expected| \
         (atol={ATOL_V:e} V, rtol={RTOL:e}) against the finite-ramp reference with T_eff={t_eff:e} s. \
         Measured: {}",
        fit.violations,
        fit.points,
        fit.summary()
    );
    fit
}

/// Time-axis contract: strictly increasing (a04's source-level finding:
/// `h_min > 0`, rejected steps are not recorded), more than 100 points, starts
/// at 0, and covers at least `5*tau` after the input edge ends.
fn assert_axis_contract(label: &str, t: &[f64], t_eff: f64) {
    assert!(t.len() > 100, "{label}: only {} points", t.len());
    assert_eq!(t[0], 0.0, "{label}: first time point must be 0");
    for (i, w) in t.windows(2).enumerate() {
        assert!(
            w[1] > w[0],
            "{label}: time axis must be strictly increasing; index {i}: t[{i}]={:e}, t[{}]={:e}",
            w[0],
            i + 1,
            w[1]
        );
    }
    let need = t_eff + 5.0 * TAU;
    let last = t[t.len() - 1];
    assert!(
        last >= need - 1e-9,
        "{label}: last point {last:e} s does not cover 5*tau after the {t_eff:e} s edge (need >= {need:e} s)"
    );
}

// ---------------------------------------------------------------------------
// 1. The reference itself, checked against things that are not the reference
// ---------------------------------------------------------------------------

/// Independent checks of `ramp_reference_at`, so a bug in the reference cannot
/// masquerade as a passing (or failing) engine test:
///
/// * `y(0) = 0` and the two branches agree at `u = t_eff`;
/// * the analytic derivative satisfies the ODE `tau*y' + y = v_in(u)`;
/// * a fixed-step RK4 integration of the ODE (which never sees the closed form)
///   agrees with it;
/// * for tiny `t_eff` the deviation from the ideal step follows the
///   `-(V0*T/(2*tau))*exp(-u/tau)` law;
/// * the allowance table in this file's header is reproduced.
#[test]
fn reference_solution_matches_independent_checks() {
    // y(0) = 0 exactly.
    assert_eq!(ramp_reference_at(0.0, V0, TAU, 1e-6), 0.0);

    // Branch continuity at u = t_eff (branch 2 evaluated at x = t_eff/tau).
    for t_eff in [1e-6, 5e-7, 1e-4] {
        let rho = t_eff / TAU;
        let branch2 = (V0 / rho) * s_stable(rho);
        let branch3 = ramp_reference_at(t_eff, V0, TAU, t_eff);
        let diff = (branch2 - branch3).abs();
        assert!(
            diff < 1e-14,
            "branch discontinuity at t_eff={t_eff:e}: {branch2} vs {branch3} (diff {diff:e})"
        );
    }

    // ODE residual with the analytic derivative: tau*y' + y - v_in(u) = 0.
    for t_eff in [1e-6, 5e-7, 1e-4] {
        let mut worst = 0.0_f64;
        for i in 0..=1000 {
            let u = i as f64 * 5.0 * TAU / 1000.0;
            let rho = t_eff / TAU;
            let (y, dy) = if u <= t_eff {
                let x = u / TAU;
                (
                    (V0 / rho) * s_stable(x),
                    (V0 / (rho * TAU)) * (1.0 - (-x).exp()),
                )
            } else {
                let a = (V0 / rho) * (-rho).exp_m1();
                let g = (-(u - t_eff) / TAU).exp();
                (V0 + a * g, -a * g / TAU)
            };
            let v_in = if u <= 0.0 {
                0.0
            } else if u <= t_eff {
                V0 * u / t_eff
            } else {
                V0
            };
            worst = worst.max((TAU * dy + y - v_in).abs());
        }
        assert!(
            worst < 1e-12,
            "ODE residual too large for t_eff={t_eff:e}: {worst:e} V"
        );
    }

    // Independent RK4 integration (h = tau/20000), no closed form involved.
    for t_eff in [1e-6, 5e-7] {
        let h = TAU / 20_000.0;
        let mut y = 0.0_f64;
        let mut t = 0.0_f64;
        let mut worst = 0.0_f64;
        let mut checked = 0usize;
        while t < 5.0 * TAU - 1e-18 {
            let step = h.min(5.0 * TAU - t);
            let v_in = |s: f64| {
                if s <= 0.0 {
                    0.0
                } else if s <= t_eff {
                    V0 * s / t_eff
                } else {
                    V0
                }
            };
            let f = |s: f64, y: f64| (v_in(s) - y) / TAU;
            let k1 = f(t, y);
            let k2 = f(t + step / 2.0, y + step / 2.0 * k1);
            let k3 = f(t + step / 2.0, y + step / 2.0 * k2);
            let k4 = f(t + step, y + step * k3);
            y += step / 6.0 * (k1 + 2.0 * k2 + 2.0 * k3 + k4);
            t += step;
            // Compare on a sparse subset: RK4's own truncation error is far
            // below the tolerance, but only where the grid resolves the ramp.
            if checked * 500 + 17 < (t / h) as usize {
                checked += 1;
                worst = worst.max((y - ramp_reference_at(t, V0, TAU, t_eff)).abs());
            }
        }
        assert!(
            worst < 1e-9,
            "RK4 disagrees with the closed form for t_eff={t_eff:e}: {worst:e} V"
        );
        assert!(checked > 50, "RK4 check sampled only {checked} points");
    }

    // Tiny-edge limit: the deviation from the ideal step is
    // -(V0*T/(2*tau))*exp(-u/tau) to leading order.
    let tiny = 1e-12;
    for x in [0.25_f64, 1.0, 5.0] {
        let u = x * TAU;
        let deviation = ramp_reference_at(u, V0, TAU, tiny) - V0 * (1.0 - (-u / TAU).exp());
        let predicted = -(V0 * tiny / (2.0 * TAU)) * (-u / TAU).exp();
        let ratio = deviation / predicted;
        assert!(
            (ratio - 1.0).abs() < 1e-5,
            "tiny-edge scaling law off at u={u:e}: deviation={deviation:e}, predicted={predicted:e}, ratio={ratio}"
        );
    }

    // The documented allowance table (docs/review-evidence/rc-reference-math.md section 6).
    let table: [(f64, f64, f64); 6] = [
        (0.001, 4.99833375e-5, 1.00500e-5),
        (0.01, 4.98337492e-3, 1.49834e-5),
        (0.1, 9.06232765e-2, 1.00623e-4),
        (1.0, 6.30275015e-1, 6.40275e-4),
        (2.0, 8.63985779e-1, 8.73986e-4),
        (5.0, 9.93228251e-1, 1.00323e-3),
    ];
    for (frac, expected, allow) in table {
        let u = frac * TAU;
        let y = ramp_reference(u, 1e-6);
        let got_allow = allowance(y);
        assert!(
            (y - expected).abs() <= 1e-6 * expected.abs(),
            "reference y at u/tau={frac}: got {y:e}, documented {expected:e}"
        );
        assert!(
            (got_allow - allow).abs() <= 1e-4 * allow,
            "allowance at u/tau={frac}: got {got_allow:e}, documented {allow:e}"
        );
    }
}

// ---------------------------------------------------------------------------
// 2. Per-point regression, with the edge NOT clamped
// ---------------------------------------------------------------------------

/// The main regression: the declared rise is deliberately >= the output
/// interval, so the engine's `max(tr, tstep)` clamp does not bind and the input
/// really is a 1 us ramp. `T_eff = max(rise, output_interval) = 1 us` is the
/// setting the documented allowance table belongs to.
///
/// Every returned point is asserted (typically ~5000 of them), and the time
/// axis contract is checked on the same run.
#[test]
fn rc_finite_ramp_matches_reference_on_every_point() {
    let rise = 1e-6;
    let output_interval = TAU / 1000.0; // 100 ns
    let max_step = TAU / 1000.0;
    let t_eff = effective_edge(rise, output_interval);
    assert_eq!(t_eff, rise, "this test must not rely on the engine clamp");

    let stop = t_eff + 5.0 * TAU;
    let c = rc_circuit("rc_ramp", rise);
    let plan = tran_plan("rc_ramp", stop, max_step, output_interval);
    let (t, vout) = run_tran(&c, &plan);

    assert_axis_contract("rc_ramp", &t, t_eff);
    let fit = assert_within_criteria("rc_ramp", &t, &vout, t_eff);

    // Guard the shape of the run: without these the criterion could be met by
    // a degenerate axis (e.g. one point, or a window that never reaches V0).
    assert!(
        t.len() > 4000,
        "expected a dense run, got {} points",
        t.len()
    );
    assert!(
        t.iter().any(|&ti| ti > 0.0 && ti < rise),
        "expected at least one returned point inside the 1 us edge"
    );
    // The run must reach the plateau region. At 5*tau past the edge the ramp
    // response is *not* at V0 yet: the exact gap is |a|*exp(-5) with
    // a = y(T) - V0 = -0.9950166 V, i.e. 6.70e-3 V. So this guard is 1e-2 V,
    // not 1e-3 V — it checks the window, not the criterion.
    let final_v = *vout.last().unwrap();
    let plateau_gap = (V0 - final_v).abs();
    assert!(
        plateau_gap < 1e-2,
        "final v(out)={final_v} is not in the plateau region; the expected gap at 5*tau past \
         the edge is |a|*exp(-5) = 6.70e-03 V"
    );
    println!("[rc_ramp] stop={stop:e} s, T_eff={t_eff:e} s");
    println!("[rc_ramp] {}", fit.summary());
}

// ---------------------------------------------------------------------------
// 3. The contract, as reproducible evidence (rewritten for Task A)
// ---------------------------------------------------------------------------

/// **The adapter must not widen a declared edge to fit the output step.**
///
/// (This replaces `declared_rise_below_output_interval_is_clamped_to_the_output_step`,
/// which pinned the opposite, pre-Task-A behaviour `T_eff = max(declared rise,
/// output_interval)`. See the module header.)
///
/// Setup: the same RC and a PULSE whose declared rise is 1 ps, with an
/// `output_interval` of 500 ns (500 000x the declared edge). `h_print` is
/// `min(span/1000, min declared rise/fall/period) = 1 ps`
/// (`thevenin.rs::print_step_for`), so the engine's `.max(tstep)` clamp on the
/// PULSE rise cannot bind and the delivered stimulus is the declared 1 ps ramp.
/// `max_step` is deliberately left at 500 ns: the RC takes the LTE branch, where
/// the step is bounded by `h_max = max_step` rather than by `h_print`, so a
/// coarse `max_step` cannot narrow the resolution of the edge either.
///
/// Three readings, in order:
///
/// 1. **the drive** — with a probe on the source node, `v(in)` is 1 V at the
///    first returned sample on or after 1 ps. That sample sits inside the
///    engine's own starting step (`h_max/400` = 1.25 ns), so a 500 ns ramp
///    would read ~2.5e-3 V there, and a 500 ns-wide clamp is unmissable. This
///    reading needs no reference solution at all;
/// 2. **the discriminating half** — against the *old* effective edge
///    `T_eff = max(1 ps, 500 ns) = 500 ns` the same run misses by up to ~1 V
///    (the plateau of a 500 ns ramp is not the plateau of a 1 ps ramp) and
///    violates the criterion on essentially every point after the ramp. This is
///    the direction the old test asserted, kept so the rewritten test cannot
///    pass on the old contract;
/// 3. **the criterion** — against the declared 1 ps edge, every returned point
///    is inside the round criterion (atol 1e-5 V, rtol 1e-3), with no threshold
///    relaxed and no sample dropped.
#[test]
fn declared_rise_below_output_interval_is_not_widened() {
    let declared_rise: f64 = 1e-12; // 1 ps, as the old probe declared
    let output_interval: f64 = TAU / 200.0; // 500 ns = the old probe's step
    let max_step: f64 = output_interval;
    // The pre-Task-A rule, kept only as the control: this is what the engine
    // was made to execute before the fix.
    let old_effective_edge = declared_rise.max(output_interval);
    assert_eq!(
        old_effective_edge, output_interval,
        "the old rule must be the output interval here, or the control is vacuous"
    );

    let stop = output_interval + 5.0 * TAU;
    let c = rc_circuit("rc_not_widened", declared_rise);
    let plan = plan_for(
        "rc_not_widened",
        AnalysisKind::Tran(TranSpec {
            start_s: 0.0,
            stop_s: stop,
            max_step: Some(max_step),
            output_interval: Some(output_interval),
            uic: false,
            span: SourceSpan::synthetic(),
        }),
        vec![
            probe("v(in)", Probe::NodeVoltage(NodeId(1))),
            probe("v(out)", Probe::NodeVoltage(NodeId(2))),
        ],
    );
    let run = be().run(&c, &plan).expect("tran runs");
    assert_eq!(run.datasets.len(), 1, "one tran task -> one dataset");
    let d = &run.datasets[0];
    let t = match &d.axis {
        Axis::Time(t) => t.clone(),
        other => panic!("expected a time axis, got {other:?}"),
    };
    let vin = real(d, "v(in)");
    let vout = real(d, "v(out)");
    assert_eq!(
        t.len(),
        vout.len(),
        "axis and signal must have equal length"
    );
    assert_eq!(t.len(), vin.len(), "axis and drive must have equal length");

    assert_axis_contract("rc_not_widened", &t, declared_rise);

    // --- 1. The drive: a 1 ps edge, not a 500 ns one ---------------------
    //
    // The first step of the engine is `h_max/400` and it may not cross the
    // declaration's corner, so the first sample on/after 1 ps lands at most one
    // such step past it; with a 500 ns ramp the drive there would be a fraction
    // of a volt rather than the declared 1 V.
    let first_after = t
        .iter()
        .position(|&ti| ti >= declared_rise)
        .expect("a sample on or after the declared edge");
    let sample_t = t[first_after];
    let sample_v = vin[first_after];
    let first_step_bound = max_step / 400.0;
    println!(
        "[rc_not_widened/drive] first sample on/after the declared edge: index {first_after}, \
         t={sample_t:e} s (declared rise {declared_rise:e} s, starting step bound \
         {first_step_bound:e} s), v(in)={sample_v:?} V"
    );
    assert!(
        sample_t - declared_rise <= first_step_bound,
        "the drive was not resolved near the declared edge: the first sample on/after \
         {declared_rise:e} s is {sample_t:e} s, more than one starting step \
         ({first_step_bound:e} s) later"
    );
    assert_eq!(
        sample_v,
        V0,
        "v(in) at t={sample_t:e} s is {sample_v:?} V, not the declared {V0} V: the 1 ps edge \
         was widened (a {output_interval:e} s ramp would read {:.6e} V there)",
        V0 * sample_t / output_interval
    );
    assert_eq!(vin[0], 0.0, "v(in) at t=0 must be 0 V");

    // --- 2. The old reference must be badly wrong ------------------------
    //
    // This is the test's own failure direction: the pre-Task-A engine executed
    // a 500 ns ramp, which the declared-1ps criterion rejected here on 257 of
    // 1024 points (the failure this test replaced). Now the same old reference
    // is the one that misses.
    let fit_old = fit_against(&t, &vout, old_effective_edge);
    println!(
        "[rc_not_widened/old-{old_effective_edge:e}-reference] {}",
        fit_old.summary()
    );
    let max_possible_allowance = ATOL_V + RTOL * V0;
    assert!(
        fit_old.violations > 100,
        "the old `T_eff = max(rise, output_interval)` reference should now violate the \
         criterion on far more than 100 points; measured: {}",
        fit_old.summary()
    );
    assert!(
        fit_old.max_error > max_possible_allowance,
        "expected the old-reference error to exceed the largest allowance the criterion can \
         produce ({max_possible_allowance:e} V); measured: {}",
        fit_old.summary()
    );

    // --- 3. The declared edge meets the criterion on every point ---------
    let fit_declared =
        assert_within_criteria("rc_not_widened/declared-1ps", &t, &vout, declared_rise);
    println!(
        "[rc_not_widened] declared-edge worst ratio {:.3}x over {} points; old-reference \
         worst ratio {:.3}x (the pre-fix run failed the declared-edge fit with 257 of 1024 \
         points and max|err| = 2.483260e-3 V)",
        fit_declared.worst_ratio, fit_declared.points, fit_old.worst_ratio
    );
}

// ---------------------------------------------------------------------------
// 4. max_step must actually reach the engine
// ---------------------------------------------------------------------------

/// The only solver control the product path exposes is
/// `TranSpec.max_step -> CqTran.tmax -> h_max`
/// (`thevenin.rs:781`; `thevenin-0.5.0/src/transient.rs:799,1694`).
///
/// Same circuit, same plan, only `max_step` changes: the returned point counts
/// must be strictly increasing (a smaller cap means more accepted steps are
/// recorded), and every setting must still meet the criterion. If the counts
/// were equal the test would have to report that the setting never reached the
/// engine; that is an assertion here, not a note.
#[test]
fn max_step_reaches_the_engine_and_every_setting_meets_the_criteria() {
    let rise = 1e-6;
    // Keep the output interval at the declared rise so the cap under test is
    // max_step, not output_interval (h = min(..., h_max, h_print)).
    let output_interval = TAU / 100.0;
    let t_eff = effective_edge(rise, output_interval);
    let stop = t_eff + 5.0 * TAU;
    let c = rc_circuit("rc_maxstep", rise);

    let settings = [TAU / 100.0, TAU / 1000.0, TAU / 10_000.0];
    let mut counts = Vec::new();
    for (i, max_step) in settings.into_iter().enumerate() {
        let plan = tran_plan("rc_maxstep", stop, max_step, output_interval);
        let (t, vout) = run_tran(&c, &plan);
        let label = format!("rc_maxstep[{i}] max_step={max_step:e}");
        assert_axis_contract(&label, &t, t_eff);
        let fit = assert_within_criteria(&label, &t, &vout, t_eff);
        println!(
            "[max_step] max_step={max_step:e} s -> {} points, max|err|={:.3e} V at t={:.6e} s",
            t.len(),
            fit.max_error,
            fit.max_error_at
        );
        counts.push(t.len());
    }

    for w in counts.windows(2) {
        assert!(
            w[1] > w[0],
            "max_step did not change the returned grid: point counts {counts:?} are not strictly \
             increasing, so this run cannot show that max_step reached the engine"
        );
    }
    // A weaker but explicit sanity margin: the finest setting must be much
    // denser than the coarsest one, matching h_max ~ stop/max_step.
    assert!(
        counts[2] > 5 * counts[0],
        "expected the finest max_step to produce far more points; counts {counts:?}"
    );
    println!("[max_step] point counts (coarsest -> finest): {counts:?}");
}
