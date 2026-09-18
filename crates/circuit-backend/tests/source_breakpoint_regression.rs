//! Product-path regression for a **running source breakpoint**: RC + periodic
//! PULSE with a non-zero delay, finite rise/fall and at least two periods.
//!
//! Scope: the *product* adapter path only — project IR (`circuit_core::ir`) ->
//! `TheveninBackend` -> `Dataset` (`circuit_results::dataset`). The reference is
//! an independent piecewise-analytic solution; the solver is never consulted to
//! build it.
//!
//! # Stimulus
//!
//! `v1` (PULSE) drives `r1` into node `out`, `c1` from `out` to ground;
//! `tau = R*C = 100 us`. The PULSE has `delay = 100 us`, `rise = fall = 1 us`,
//! `width = 5 us` and `period = 20 us`, so the window `[0, 300 us]` contains ten
//! pulses and forty breakpoints (four per period: delay + k*period, +rise,
//! +rise+width, +rise+width+fall).
//!
//! # The four breakpoints per period, and why they are a real accuracy question
//!
//! The engine (thevenin 0.5.0, vendored source in the cargo registry)
//!
//! * clamps the step to the distance to the next breakpoint
//!   (`transient.rs:1434-1439`),
//! * forces **Backward-Euler** for a step that *starts* at a breakpoint
//!   (`is_at_breakpoint` tests the step start, `:1433`; method selection
//!   `:1464-1468`),
//! * and shrinks that first step to `step_h.min(h * 0.1).max(h_min)` (`:1443`),
//!   where `h` is the *suggested* step of that iteration — not `tmax/10`.
//!
//! A forced-BE restart at a quiescent ramp start is wrong by
//! `(V0/T) * h1^2 / (2*tau)` (`h1` = the restart step): the BE update returns
//! `~ m*h1^2/tau` where the exact response is `~ m*h1^2/(2*tau)`. Measuring this
//! on the product path is what this file is for; `_probe/src/bin/breakpoint_study.rs`
//! pins the mechanism at the engine level (measured: the returned value matches
//! the Backward-Euler one-step update to ~5e-19 V while differing from the
//! trapezoidal one by ~5e-7 V in this same configuration).
//!
//! # Passing configuration and its basis (asserted below, not asserted in prose)
//!
//! With `h1 <= 0.1 * h_max` the restart error is bounded by
//! `(V0/T)*(0.1*h_max)^2/(2*tau)`, so keeping it inside `atol` needs
//!
//! ```text
//! h_max <= 10 * sqrt(2 * atol * tau * T / V0)                        (*)
//!        = 10 * sqrt(2*1e-5*1e-4*1e-6/1) = 4.4721e-7 s = tau/223.6
//! ```
//!
//! The passing configuration here is `max_step = tau/1000 = 1e-7 s`, i.e. 4.5x
//! below that bound; `tau/500 = 2e-7 s` is also below it and is checked too.
//! **This is a bound for this stimulus** (`V0/T = 1e6 V/s`) and this pole
//! (`tau = 100 us`) — both factors are in `(*)`, so a different edge rate or
//! time constant needs its own evaluation. It is not a general "tau/1000"
//! guarantee for arbitrary circuits.
//!
//! # Coarse configurations are kept as limitation rows, never hidden
//!
//! `tau/50` (2 us) and `tau/200` (500 ns) are above the bound and *do* violate
//! §17. They are exercised by a separate test whose name starts with
//! `limitation_`, which prints every row and pins the measured behaviour; they
//! are never merged into the passing assertions, no sample is dropped, and
//! `#[ignore]` is not used anywhere in this file.
//!
//! # Tolerance channel: none on this path
//!
//! `build_circuit` (`crates/circuit-backend/src/thevenin.rs`) builds its
//! `CqCircuit` with `options: Vec::new()`, so RELTOL/ABSTOL/TRTOL cannot be set
//! through the product path. No test here claims otherwise; the tolerance
//! experiments live in `_probe/src/bin/breakpoint_study.rs`, which builds
//! `cirq_ir::Circuit` directly.
//!
//! # §17 criterion (fixed for this round; never relaxed here)
//!
//!     |actual - expected| <= atol + rtol * |expected|,  atol = 1e-5 V, rtol = 1e-3
//!
//! The largest allowance this criterion can produce is `atol + rtol*V0`.

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

/// PULSE delay (`td`): the source is quiescent before this.
const DELAY: f64 = 100e-6;
/// Declared PULSE rise (`tr`).
const RISE: f64 = 1e-6;
/// Declared PULSE fall (`tf`).
const FALL: f64 = 1e-6;
/// Declared PULSE high time (`pw`).
const WIDTH: f64 = 5e-6;
/// Declared PULSE period (`per`).
const PERIOD: f64 = 20e-6;
/// Simulated window: ten pulses, forty breakpoints.
const STOP: f64 = 300e-6;

/// The print step the product adapter picks for this circuit
/// (`thevenin.rs::print_step_for`):
/// `min(span/1000, min(rise, fall, period)) = min(300 ns, 1 us) = 300 ns`.
/// It is *not* an output interval: the engine records every accepted step, and
/// the adapter's `output_interval` is applied after the run
/// (`circuit-results::resample`).
const PRINT_STEP: f64 = 300e-9;

// ---------------------------------------------------------------------------
// IR / plan builders (copied from crates/circuit-backend/tests/adapter.rs so
// this file stays independent of the other test files)
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
// Circuit and plans for this file
// ---------------------------------------------------------------------------

/// `v1` (PULSE, delayed, periodic) -> `r1` -> `out`, `c1` -> gnd, probing
/// `v(out)`. `dc` is 0 V so the transient starts from the zero state the
/// reference solution assumes.
fn rc_pulse_circuit(name: &str) -> Circuit {
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
                delay: Quantity::seconds(DELAY),
                rise: Quantity::seconds(RISE),
                fall: Quantity::seconds(FALL),
                width: Quantity::seconds(WIDTH),
                period: Quantity::seconds(PERIOD),
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

/// A TRAN task over `[0, STOP]` with an explicit `max_step` and an optional
/// `output_interval` (which never reaches the solver).
fn tran_plan(name: &str, max_step: f64, output_interval: Option<f64>) -> AnalysisPlan {
    plan_for(
        name,
        AnalysisKind::Tran(TranSpec {
            start_s: 0.0,
            stop_s: STOP,
            max_step: Some(max_step),
            output_interval,
            uic: false,
            span: SourceSpan::synthetic(),
        }),
        vec![probe("v(out)", Probe::NodeVoltage(NodeId(2)))],
    )
}

/// Run one case and return the dataset plus its axis and `v(out)`.
fn run_tran(c: &Circuit, plan: &AnalysisPlan) -> (Dataset, Vec<f64>, Vec<f64>) {
    let out = be().run(c, plan).expect("tran runs");
    let d = out.datasets[0].clone();
    let t = match &d.axis {
        Axis::Time(t) => t.clone(),
        other => panic!("expected a time axis, got {other:?}"),
    };
    let vout = real(&d, "v(out)");
    assert_eq!(
        t.len(),
        vout.len(),
        "axis and signal must have equal length"
    );
    (d, t, vout)
}

// ---------------------------------------------------------------------------
// Independent reference: exact solution for a delayed periodic PULSE
// ---------------------------------------------------------------------------

/// `S(x) = x + expm1(-x) = sum_{k>=2} (-1)^k x^k / k!`, evaluated stably.
fn s_stable(x: f64) -> f64 {
    if x < 1e-2 {
        // Series k = 2..6; the first omitted term is x^7/7!, i.e. a relative
        // error <= 3.97e-14 at the x = 1e-2 switch-over.
        let mut term = x;
        let mut sum = 0.0;
        for k in 2..=6u32 {
            term *= x / f64::from(k);
            if k % 2 == 0 {
                sum += term;
            } else {
                sum -= term;
            }
        }
        sum
    } else {
        x + (-x).exp_m1()
    }
}

/// One linear piece of the input: `v_in(t) = v0 + m*(t - t0)` on `[t0, t1]`.
#[derive(Clone, Copy, Debug)]
struct Segment {
    t0: f64,
    t1: f64,
    v0: f64,
    m: f64,
}

/// Exact response of the one-pole RC to the PULSE the engine actually used.
///
/// The engine clamps the PULSE rise/fall up to the `.tran` step
/// (`thevenin-0.5.0/src/waveform.rs:37-38`), so the reference is built from
/// `tr_used = max(RISE, tstep)` and `tf_used = max(FALL, tstep)`; the same
/// clamped values drive the engine's breakpoint table (`:271-278`).
///
/// On a segment the recurrence is
///
/// ```text
/// x = u/tau
/// y = v0 + m*tau*S(x) + (y_start - v0)*exp(-x)
/// ```
///
/// which is the cancellation-free rearrangement of
/// `y = v0 + m*u - m*tau + (y_start - v0 + m*tau)*exp(-x)`. The naive form
/// loses every significant digit near a ramp start, where `y` is
/// `m*tau*x^2/2` but the terms are `m*tau`.
struct Reference {
    segs: Vec<Segment>,
    /// Exact `y` at each segment start.
    ys: Vec<f64>,
}

impl Reference {
    fn new(tstep: f64) -> Self {
        let tr_used = RISE.max(tstep);
        let tf_used = FALL.max(tstep);
        let period = PERIOD.max(tr_used + WIDTH + tf_used).max(tstep);

        let mut segs = vec![Segment {
            t0: 0.0,
            t1: DELAY.min(STOP),
            v0: 0.0,
            m: 0.0,
        }];
        let mut k = 0u64;
        loop {
            let base = DELAY + k as f64 * period;
            if base >= STOP {
                break;
            }
            let edges = [
                (base, base + tr_used, 0.0, V0 / tr_used),
                (base + tr_used, base + tr_used + WIDTH, V0, 0.0),
                (
                    base + tr_used + WIDTH,
                    base + tr_used + WIDTH + tf_used,
                    V0,
                    -V0 / tf_used,
                ),
                (base + tr_used + WIDTH + tf_used, base + period, 0.0, 0.0),
            ];
            for (a, b, v0, m) in edges {
                let b = b.min(STOP);
                if b > a {
                    segs.push(Segment {
                        t0: a,
                        t1: b,
                        v0,
                        m,
                    });
                }
            }
            k += 1;
            if k > 1_000_000 {
                break;
            }
        }
        let mut ys = Vec::with_capacity(segs.len());
        let mut y = 0.0_f64;
        for (i, s) in segs.iter().enumerate() {
            if i == 0 {
                ys.push(0.0);
                continue;
            }
            let prev = segs[i - 1];
            let x = (s.t0 - prev.t0) / TAU;
            y = prev.v0 + prev.m * TAU * s_stable(x) + (y - prev.v0) * (-x).exp();
            ys.push(y);
        }
        Reference { segs, ys }
    }

    fn last_time(&self) -> f64 {
        self.segs.last().map(|s| s.t1).unwrap_or(0.0)
    }

    /// Exact input value (piecewise linear, from the segments).
    fn v_in(&self, t: f64) -> f64 {
        if t <= 0.0 {
            return 0.0;
        }
        for s in &self.segs {
            if t >= s.t0 && t <= s.t1 {
                return s.v0 + s.m * (t - s.t0);
            }
        }
        // Beyond the last segment the input holds its final value.
        self.segs
            .last()
            .map(|s| s.v0 + s.m * (s.t1 - s.t0))
            .unwrap_or(0.0)
    }

    /// Exact response at `t`.
    fn y(&self, t: f64) -> f64 {
        let mut y = 0.0;
        for s in &self.segs {
            if t <= s.t0 {
                break;
            }
            let te = t.min(s.t1);
            let x = (te - s.t0) / TAU;
            y = s.v0 + s.m * TAU * s_stable(x) + (y - s.v0) * (-x).exp();
            if t <= s.t1 {
                break;
            }
        }
        y
    }

    /// Analytic derivative on the segment containing `t`, for the ODE residual
    /// check: `y' = m*(1 - e^-x) - (y_start - v0)*e^-x/tau`.
    fn dy(&self, t: f64) -> f64 {
        for (i, s) in self.segs.iter().enumerate() {
            if t >= s.t0 && t <= s.t1 {
                let x = (t - s.t0) / TAU;
                return s.m * (1.0 - (-x).exp()) - (self.ys[i] - s.v0) * (-x).exp() / TAU;
            }
        }
        0.0
    }

    /// Independent algebraic route to the same solution: the Green's-function
    /// integral of the ODE, integrated by parts,
    ///
    /// ```text
    /// y(t) = v_in(t) - tau * sum_j m_j * exp(-(t - a_j)/tau) * expm1((b_j - a_j)/tau)
    /// ```
    ///
    /// over the segments `[a_j, b_j]` with `a_j < t`, `b_j = min(t, t1)`.
    /// No segment recurrence is involved, so agreement with [`Self::y`] checks
    /// the recurrence itself (both share only `exp`/`expm1`). The terms are
    /// `O(1)` V here, so this route carries no cancellation.
    fn convolution_y(&self, t: f64) -> f64 {
        assert!(
            t <= self.last_time() + 1e-12,
            "the convolution reference is only defined inside the window"
        );
        let mut sum = 0.0;
        for s in &self.segs {
            if s.t0 >= t {
                break;
            }
            let a = s.t0;
            let b = t.min(s.t1);
            if b <= a {
                continue;
            }
            sum += s.m * (-((t - a) / TAU)).exp() * ((b - a) / TAU).exp_m1();
        }
        self.v_in(t) - TAU * sum
    }

    /// Breakpoint times the engine builds
    /// (`thevenin-0.5.0/src/waveform.rs:257-302`).
    fn breakpoints(&self, tstep: f64) -> Vec<f64> {
        let tr_used = RISE.max(tstep);
        let tf_used = FALL.max(tstep);
        let period = PERIOD.max(tr_used + WIDTH + tf_used).max(tstep);
        let edges = [0.0, tr_used, tr_used + WIDTH, tr_used + WIDTH + tf_used];
        let mut out = Vec::new();
        let mut k = 0u64;
        loop {
            let base = DELAY + k as f64 * period;
            if base > STOP {
                break;
            }
            for e in edges {
                let t = base + e;
                if (0.0..=STOP).contains(&t) {
                    out.push(t);
                }
            }
            k += 1;
            if k > 1_000_000 {
                break;
            }
        }
        out
    }
}

/// Independent replication of the engine's PULSE evaluator
/// (`thevenin-0.5.0/src/waveform.rs:116-156`), used to drive the RK4
/// cross-check so that check does not share the segment builder.
fn v_in_engine(t: f64, tstep: f64) -> f64 {
    let tr_used = RISE.max(tstep);
    let tf_used = FALL.max(tstep);
    let period = PERIOD.max(tr_used + WIDTH + tf_used).max(tstep);
    if t < DELAY {
        return 0.0;
    }
    let mut time = t - DELAY;
    if period > 0.0 && time >= period {
        time -= period * (time / period).floor();
    }
    if time < tr_used {
        V0 * time / tr_used
    } else if time < tr_used + WIDTH {
        V0
    } else if time < tr_used + WIDTH + tf_used {
        V0 * (1.0 - (time - tr_used - WIDTH) / tf_used)
    } else {
        0.0
    }
}

/// Fixed-step RK4, with the grid aligned to the PULSE breakpoints, compared
/// against the closed form at every grid point. Returns `(worst |diff|, steps)`.
fn rk4_worst_deviation(reference: &Reference, h: f64) -> (f64, usize) {
    let f = |s: f64, y: f64| (v_in_engine(s, PRINT_STEP) - y) / TAU;
    let mut y = 0.0_f64;
    let mut t = 0.0_f64;
    let mut worst = 0.0_f64;
    let mut steps = 0usize;
    while t < STOP - 1e-18 {
        let step = h.min(STOP - t);
        let k1 = f(t, y);
        let k2 = f(t + step / 2.0, y + step / 2.0 * k1);
        let k3 = f(t + step / 2.0, y + step / 2.0 * k2);
        let k4 = f(t + step, y + step * k3);
        y += step / 6.0 * (k1 + 2.0 * k2 + 2.0 * k3 + k4);
        t += step;
        worst = worst.max((y - reference.y(t)).abs());
        steps += 1;
    }
    (worst, steps)
}

fn allowance(expected: f64) -> f64 {
    ATOL_V + RTOL * expected.abs()
}

// ---------------------------------------------------------------------------
// Criteria measurement
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

/// Compare every returned point against the reference, evaluated at the time
/// the engine actually returned.
fn fit_against(t: &[f64], got: &[f64], reference: &Reference) -> Fit {
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
        let expected = reference.y(ti);
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

/// Assert the fixed criterion on **every** returned point. On failure the
/// message carries the measured numbers so the failure is reproducible without
/// rerunning.
fn assert_within_criteria(label: &str, t: &[f64], got: &[f64], reference: &Reference) -> Fit {
    let fit = fit_against(t, got, reference);
    println!("[{label}] {}", fit.summary());
    assert_eq!(
        fit.violations,
        0,
        "{label}: {} of {} returned points violate |actual - expected| <= atol + rtol*|expected| \
         (atol={ATOL_V:e} V, rtol={RTOL:e}). Measured: {}",
        fit.violations,
        fit.points,
        fit.summary()
    );
    fit
}

/// How many offending samples a diagnostic prints. Setting `CDSL_BP_DUMP_ALL`
/// in the environment prints **every** offending sample instead: the evidence
/// report is generated with that flag, so no failing sample is ever dropped from
/// the record.
fn offender_limit(default: usize) -> usize {
    if std::env::var_os("CDSL_BP_DUMP_ALL").is_some() {
        usize::MAX
    } else {
        default
    }
}

/// The list of violating samples, printed verbatim (never dropped).
fn violation_lines(
    t: &[f64],
    got: &[f64],
    reference: &Reference,
    limit: usize,
) -> (usize, Vec<String>) {
    let mut n = 0usize;
    let mut lines = Vec::new();
    for (i, &ti) in t.iter().enumerate() {
        let expected = reference.y(ti);
        let err = (got[i] - expected).abs();
        let allow = allowance(expected);
        if err > allow {
            n += 1;
            if lines.len() < limit {
                lines.push(format!(
                    "t={ti:.9e} s: |err|={err:.6e} V > allowance={allow:.6e} V \
                     (expected={expected:.9} V, actual={:.9} V, ratio={:.3}x)",
                    got[i],
                    err / allow
                ));
            }
        }
    }
    (n, lines)
}

/// Time-axis contract: strictly increasing, starts at 0, covers at least two
/// pulses past the first breakpoint. `min_points` is passed by the caller
/// because a coarse `max_step` legitimately returns a short axis.
fn assert_axis_contract(label: &str, t: &[f64], min_points: usize) {
    assert!(
        t.len() > min_points,
        "{label}: only {} points (expected more than {min_points})",
        t.len()
    );
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
    let need = DELAY + 2.0 * PERIOD;
    let last = t[t.len() - 1];
    assert!(
        last >= need - 1e-9,
        "{label}: last point {last:e} s does not cover two full pulses (need >= {need:e} s)"
    );
    assert!(
        (last - STOP).abs() < 1e-12,
        "{label}: the window must reach stop = {STOP:e} s; last point is {last:e} s"
    );
}

/// The first accepted step that *starts* at `bp`: `(h_before, h1)`.
fn step_after(t: &[f64], bp: f64, tol: f64) -> Option<(f64, f64)> {
    let i = (0..t.len()).find(|&i| (t[i] - bp).abs() < tol)?;
    if i == 0 || i + 1 >= t.len() {
        return None;
    }
    Some((t[i] - t[i - 1], t[i + 1] - t[i]))
}

/// The passing configurations: `tau/500` and `tau/1000`, both below the derived
/// bound `10*sqrt(2*atol*tau*T/V0) = 4.4721e-7 s` (asserted in the test).
const MET_MAX_STEPS: [f64; 2] = [TAU / 500.0, TAU / 1000.0];
/// The limitation configurations: above the bound, kept as evidence.
const LIMITATION_MAX_STEPS: [f64; 2] = [TAU / 50.0, TAU / 200.0];

// ---------------------------------------------------------------------------
// 1. The reference itself, checked against things that are not the reference
// ---------------------------------------------------------------------------

/// Two independent checks of the reference, so a bug in it cannot masquerade as
/// a passing (or failing) engine test:
///
/// * **ODE residual** with the analytic derivative: `tau*y' + y - v_in = 0`;
/// * **RK4** on a fixed grid aligned to the breakpoints, driven by an
///   independent replica of the engine's PULSE evaluator;
/// * **Green's-function route** (`convolution_y`), a different algebraic path
///   to the same solution;
/// * branch continuity at every segment boundary.
#[test]
fn reference_matches_independent_checks() {
    let r = Reference::new(PRINT_STEP);
    assert_eq!(r.segs[0].t0, 0.0, "the reference must start at t = 0");
    assert!(r.ys[0] == 0.0, "y(0) must be exactly 0");

    // Segment continuity: the two branches must meet at every breakpoint. The
    // offset is 1e-18 s (well below any segment length in this stimulus), so the
    // measured difference is |y'| * 1e-18 ~ 1e-14 V and a real jump would be far
    // larger.
    let mut worst_continuity = 0.0_f64;
    for s in &r.segs[1..] {
        let d = (r.y(s.t0 - 1e-18) - r.y(s.t0)).abs();
        worst_continuity = worst_continuity.max(d);
    }
    println!(
        "[reference] branch continuity max |y(t0 - 1e-18) - y(t0)| = {worst_continuity:.3e} V \
         (a genuine discontinuity at a breakpoint would be O(0.1) V)"
    );
    assert!(
        worst_continuity < 1e-12,
        "the reference is not continuous at its own breakpoints: {worst_continuity:e} V"
    );

    // ODE residual, uniform grid plus a dense sweep across the first ramp.
    let mut worst_residual = 0.0_f64;
    let mut worst_at = 0.0_f64;
    let mut check = |t: f64| {
        let res = TAU * r.dy(t) + r.y(t) - r.v_in(t);
        if res.abs() > worst_residual {
            worst_residual = res.abs();
            worst_at = t;
        }
    };
    for i in 0..=300_000u32 {
        check(i as f64 * STOP / 300_000.0);
    }
    for i in 0..=2000u32 {
        check(DELAY + i as f64 * RISE / 2000.0);
        check(DELAY + RISE - i as f64 * RISE / 2000.0);
    }
    println!(
        "[reference] ODE residual max |tau*y' + y - v_in| = {worst_residual:.3e} V at t={worst_at:.6e} s"
    );
    assert!(
        worst_residual < 1e-12,
        "ODE residual too large: {worst_residual:e} V (target <= 1e-12 V)"
    );

    // Independent algebraic route (Green's function, no segment recurrence).
    let mut worst_conv = 0.0_f64;
    for i in 0..=30_000u32 {
        let t = i as f64 * STOP / 30_000.0;
        worst_conv = worst_conv.max((r.convolution_y(t) - r.y(t)).abs());
    }
    println!(
        "[reference] Green's-function route vs the segment recurrence: max |diff| = {worst_conv:.3e} V"
    );
    assert!(
        worst_conv < 1e-12,
        "the two reference routes disagree by {worst_conv:e} V (target <= 1e-12 V)"
    );

    // RK4 with a 10 ns grid: every PULSE boundary in this stimulus is a
    // multiple of 10 ns, so no step crosses a slope kink.
    let (worst_rk4, steps) = rk4_worst_deviation(&r, 10e-9);
    println!(
        "[reference] RK4 (h = 10 ns, {steps} steps, independent input evaluator) max |RK4 - closed form| = {worst_rk4:.3e} V"
    );
    assert!(
        worst_rk4 < 1e-12,
        "RK4 disagrees with the closed form by {worst_rk4:e} V (target <= 1e-12 V)"
    );

    // The input replica and the segment based input must agree.
    let mut worst_in = 0.0_f64;
    for i in 0..=300_000u32 {
        let t = i as f64 * STOP / 300_000.0;
        worst_in = worst_in.max((v_in_engine(t, PRINT_STEP) - r.v_in(t)).abs());
    }
    println!("[reference] |segment v_in - engine replica| max = {worst_in:.3e} V");
    assert!(
        worst_in < 1e-12,
        "input definitions disagree: {worst_in:e} V"
    );
}

// ---------------------------------------------------------------------------
// 2. The breakpoints are samples of the returned axis
// ---------------------------------------------------------------------------

/// The step clamp (`transient.rs:1434-1439`) means the axis must *land* on
/// every breakpoint. This is asserted for the first two pulses (eight
/// breakpoints: delay, delay+rise, delay+rise+width, delay+rise+width+fall and
/// their period-2 counterparts) and the measured deviation is printed.
#[test]
fn pulse_breakpoints_are_samples_of_the_returned_axis() {
    let c = rc_pulse_circuit("rc_bp_axis");
    let plan = tran_plan("rc_bp_axis", TAU / 1000.0, None);
    let (_, t, _) = run_tran(&c, &plan);
    let r = Reference::new(PRINT_STEP);
    let bps = r.breakpoints(PRINT_STEP);
    assert!(
        bps.len() >= 40,
        "expected at least the 40 breakpoints of ten pulses, got {}",
        bps.len()
    );

    let mut worst = 0.0_f64;
    for (i, &bp) in bps.iter().enumerate() {
        let nearest = t
            .iter()
            .copied()
            .fold(f64::INFINITY, |acc, x| acc.min((x - bp).abs()));
        worst = worst.max(nearest);
        assert!(
            nearest < 1e-12,
            "breakpoint {i} at t={bp:.9e} s has no returned sample within 1e-12 s; \
             nearest deviation = {nearest:.3e} s"
        );
    }
    println!(
        "[rc_bp_axis] all {} breakpoints of the first two pulses are samples of the returned axis \
         ({} samples in total); max |t_sample - t_breakpoint| = {worst:.3e} s",
        bps.len(),
        t.len()
    );
    // The named ones the task asks about, spelled out.
    for (name, bp) in [
        ("delay", DELAY),
        ("delay+rise", DELAY + RISE),
        ("delay+rise+width", DELAY + RISE + WIDTH),
        ("delay+rise+width+fall", DELAY + RISE + WIDTH + FALL),
        ("2nd delay", DELAY + PERIOD),
        ("2nd delay+rise", DELAY + PERIOD + RISE),
    ] {
        assert!(
            t.iter().any(|&x| (x - bp).abs() < 1e-12),
            "the axis has no sample at {name} = {bp:.6e} s"
        );
    }
}

// ---------------------------------------------------------------------------
// 3. The passing configuration, every returned point
// ---------------------------------------------------------------------------

/// The main regression: `max_step = tau/1000 = 100 ns`, i.e. 4.5x below the
/// derived restart bound `10*sqrt(2*atol*tau*T/V0) = 4.4721e-7 s` — see the
/// module docs. Every returned point (about 3000 of them, over ten pulses and
/// forty breakpoints) is asserted against the independent reference, and the
/// product-path step contract is checked on the same dataset.
#[test]
fn rc_periodic_pulse_matches_reference_on_every_point() {
    let max_step = TAU / 1000.0;
    let bound = 10.0 * (2.0 * ATOL_V * TAU * RISE / V0).sqrt();
    println!(
        "[rc_pulse] derived restart bound h_max <= 10*sqrt(2*atol*tau*T/V0) = {bound:.6e} s \
         (= tau/{:.1}); chosen max_step = {max_step:.6e} s (= tau/1000), margin {:.2}x",
        TAU / bound,
        bound / max_step
    );
    assert!(
        max_step < bound,
        "the chosen max_step {max_step:e} s is not below the derived bound {bound:e} s"
    );

    let c = rc_pulse_circuit("rc_pulse");
    let plan = tran_plan("rc_pulse", max_step, Some(PERIOD / 20.0));
    let (d, t, vout) = run_tran(&c, &plan);
    let r = Reference::new(PRINT_STEP);

    // The declared edge must not be clamped: the adapter's print step is below
    // the declared rise/fall, so `tr_used = max(declared, tstep) = declared`.
    let solver_step: f64 = d
        .backend
        .setting("tran.solver_step")
        .expect("the adapter records the solver step")
        .parse()
        .expect("tran.solver_step is a number");
    let bound_setting: f64 = d
        .backend
        .setting("tran.waveform_bound")
        .expect("the adapter records the waveform bound")
        .parse()
        .expect("tran.waveform_bound is a number");
    println!(
        "[rc_pulse] adapter metadata: tran.solver_step = {solver_step:e} s, \
         tran.waveform_bound = {bound_setting:e} s"
    );
    assert_eq!(solver_step, PRINT_STEP, "adapter print step changed");
    assert_eq!(bound_setting, RISE.min(FALL).min(PERIOD));
    assert!(
        solver_step <= bound_setting,
        "the adapter's print step ({solver_step:e} s) exceeds the shortest declared timing \
         ({bound_setting:e} s), so the engine would clamp the declared edge"
    );
    assert!(
        RISE >= solver_step,
        "this test must not rely on the engine's rise clamp"
    );

    assert_axis_contract("rc_pulse", &t, 3000);
    let fit = assert_within_criteria("rc_pulse", &t, &vout, &r);

    // Shape guards: without these the criterion could be met by a degenerate axis.
    assert!(
        t.len() > 3000,
        "expected a dense run over ten pulses, got {} points",
        t.len()
    );
    assert!(
        t.iter().any(|&x| x > 0.0 && x < DELAY),
        "expected samples in the quiescent interval before the delay"
    );
    let final_v = *vout.last().unwrap();
    let expected_final = r.y(*t.last().unwrap());
    assert!(
        (final_v - expected_final).abs() < 1e-3,
        "the run must end in the reference's own value; got {final_v}, reference {expected_final}"
    );
    println!(
        "[rc_pulse] stop={STOP:e} s, {:.0} pulses, {} breakpoints, T_eff={RISE:e} s (not clamped)",
        (STOP - DELAY) / PERIOD,
        r.breakpoints(PRINT_STEP).len()
    );
    println!("[rc_pulse] {}", fit.summary());
}

// ---------------------------------------------------------------------------
// 4. max_step trend table
// ---------------------------------------------------------------------------

/// Three `max_step` settings plus the coarse ones, with the §17 verdict of each.
/// The passing rows are asserted; the coarse rows are printed and labelled
/// `LIMITATION` (the separate `limitation_*` test pins their behaviour). The
/// point count must grow as `max_step` shrinks.
#[test]
fn max_step_trend_table_marks_passing_and_limitation_rows() {
    let c = rc_pulse_circuit("rc_maxstep");
    let r = Reference::new(PRINT_STEP);
    let mut counts: Vec<(f64, usize)> = Vec::new();
    let mut limitation_rows = 0usize;

    println!(
        "[max_step trend] {:>12} {:>8} {:>16} {:>11} {:>10} {:>11}",
        "max_step", "points", "max |err| [V]", "at t [s]", "over-limit", "verdict"
    );
    for max_step in LIMITATION_MAX_STEPS.into_iter().chain(MET_MAX_STEPS) {
        let plan = tran_plan("rc_maxstep", max_step, None);
        let (_, t, vout) = run_tran(&c, &plan);
        let fit = fit_against(&t, &vout, &r);
        let met = fit.violations == 0;
        println!(
            "[max_step trend] {:>12.6e} {:>8} {:>16.6e} {:>11.3e} {:>10} {:>11}",
            max_step,
            fit.points,
            fit.max_error,
            fit.max_error_at,
            fit.violations,
            if met { "MET" } else { "LIMITATION" }
        );
        if !met {
            limitation_rows += 1;
            let (n, lines) = violation_lines(&t, &vout, &r, offender_limit(3));
            println!(
                "    [LIMITATION-DIAGNOSTIC] max_step = {max_step:e} s is above the derived \
                      bound and exceeds §17; {n} offenders (set CDSL_BP_DUMP_ALL=1 for all of them):"
            );
            for l in lines {
                println!("      {l}");
            }
        } else {
            // Passing rows are asserted on every point.
            assert_within_criteria(&format!("max_step={max_step:e}"), &t, &vout, &r);
        }
        assert_axis_contract(&format!("max_step={max_step:e}"), &t, 100);
        counts.push((max_step, fit.points));
    }

    // Point count must grow as max_step shrinks (max_step reaches the engine).
    let mut sorted = counts.clone();
    sorted.sort_by(|a, b| b.0.total_cmp(&a.0));
    let point_counts: Vec<usize> = sorted.iter().map(|(_, n)| *n).collect();
    println!("[max_step trend] point counts (coarsest -> finest): {point_counts:?}");
    for w in point_counts.windows(2) {
        assert!(
            w[1] > w[0],
            "max_step did not reach the engine: point counts {point_counts:?} are not strictly \
             increasing"
        );
    }
    assert_eq!(
        limitation_rows, 2,
        "expected exactly the two coarse configurations above the derived bound to be over the \
         criterion; measured {limitation_rows}. If this changed because the numbers improved, \
         update the row labels here and in the evidence report - do not delete the row."
    );
}

// ---------------------------------------------------------------------------
// 5. The limitation, pinned (characterization, not acceptance)
// ---------------------------------------------------------------------------

/// **LIMITATION / DIAGNOSTIC TEST — not an acceptance claim.** The coarse
/// `max_step` rows are reproduced and pinned here so the defect cannot silently
/// disappear from the evidence:
///
/// * the restart step is exactly `min(2*h_before, h_max) * 0.1` (the engine's
///   `:1443` rule, with `h_before` the step that landed on the breakpoint);
/// * the engine's error at the end of that step is
///   `(V0/T) * h1^2 / (2*tau)` within a factor of two — the forced-BE signature;
/// * the configuration is over the §17 criterion.
///
/// If a future kernel/adapter change makes the last assertion fail, that is
/// *good news*: update the measurements here (and the evidence report) instead
/// of deleting the row. Nothing is `#[ignore]`d and no sample is dropped.
#[test]
fn limitation_coarse_max_step_forced_backward_euler_restart() {
    let c = rc_pulse_circuit("rc_limit");
    let r = Reference::new(PRINT_STEP);
    for max_step in LIMITATION_MAX_STEPS {
        let plan = tran_plan("rc_limit", max_step, None);
        let (_, t, vout) = run_tran(&c, &plan);
        let fit = fit_against(&t, &vout, &r);
        println!("[LIMITATION] max_step = {max_step:e} s: {}", fit.summary());

        // The first breakpoint (the rising edge of the first pulse) is the
        // quiescent ramp start where the BE error is largest.
        let bp = DELAY;
        let (h_before, h1) = step_after(&t, bp, 1e-12)
            .unwrap_or_else(|| panic!("the axis must land on the delay breakpoint {bp:e}"));
        let pred_h1 = (2.0 * h_before).min(max_step) * 0.1;
        let predicted = (V0 / RISE) * h1 * h1 / (2.0 * TAU);
        let i = (0..t.len())
            .find(|&i| (t[i] - bp).abs() < 1e-12)
            .expect("breakpoint sample");
        let observed = (vout[i + 1] - r.y(t[i + 1])).abs();
        println!(
            "[LIMITATION] max_step = {max_step:e} s: h_before = {h_before:e} s, h1 = {h1:e} s, \
             min(2*h_before, h_max)*0.1 = {pred_h1:e} s; observed restart error = {observed:.6e} V, \
             predicted (V0/T)*h1^2/(2*tau) = {predicted:.6e} V, ratio = {:.4}",
            observed / predicted
        );
        assert!(
            (h1 - pred_h1).abs() / pred_h1 <= 1e-9,
            "the restart step no longer follows min(2*h_before, h_max)*0.1: measured {h1:e} s, \
             predicted {pred_h1:e} s"
        );
        let ratio = observed / predicted;
        assert!(
            (0.5..=2.0).contains(&ratio),
            "the restart error no longer follows (V0/T)*h1^2/(2*tau): measured {observed:e} V, \
             predicted {predicted:e} V, ratio {ratio}"
        );
        assert!(
            fit.violations > 0,
            "characterization pin: max_step = {max_step:e} s was expected to exceed §17 (it was \
             above the derived bound); measured {}. If this is now MET, the limitation is gone: \
             update this pin and the evidence report.",
            fit.summary()
        );
        let (n, lines) = violation_lines(&t, &vout, &r, offender_limit(5));
        println!(
            "[LIMITATION] {n}/{} points exceed §17 (set CDSL_BP_DUMP_ALL=1 for every offender); \
             first listed:",
            fit.points
        );
        for l in lines {
            println!("[LIMITATION]   {l}");
        }
    }
}

// ---------------------------------------------------------------------------
// 6. output_interval does not reach the solver (task A contract)
// ---------------------------------------------------------------------------

/// `output_interval` is an output-sampling request implemented after the run
/// (`circuit-results::resample`), never a solver parameter: two runs that
/// differ only in `output_interval` must return the **same raw solver grid**.
/// This is the product-path counterpart of "the engine records every accepted
/// step", and it guards the changed mapping from an earlier revision, where the
/// output interval was passed as the engine's print step and silently widened a
/// declared edge.
#[test]
fn output_interval_does_not_reach_the_solver_grid() {
    let c = rc_pulse_circuit("rc_output_interval");
    let coarse = tran_plan("rc_output_interval", TAU / 1000.0, Some(PERIOD / 20.0));
    let fine = tran_plan("rc_output_interval", TAU / 1000.0, Some(PERIOD / 1000.0));
    let (_, t_coarse, v_coarse) = run_tran(&c, &coarse);
    let (_, t_fine, v_fine) = run_tran(&c, &fine);
    assert_eq!(
        t_coarse, t_fine,
        "the raw solver grid must not depend on output_interval"
    );
    assert_eq!(
        v_coarse, v_fine,
        "the raw solver values must not depend on output_interval"
    );
    println!(
        "[output_interval] identical raw grid for output_interval = {:.3e} s and {:.3e} s: \
         {} points, same axis and values",
        PERIOD / 20.0,
        PERIOD / 1000.0,
        t_coarse.len()
    );
}
