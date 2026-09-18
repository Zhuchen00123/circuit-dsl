//! Task B kernel study: **source breakpoints** in Thevenin 0.5.0, driven
//! directly (no product adapter), in the standalone `_probe` project.
//!
//! # Question
//!
//! A recurring PULSE with a non-zero delay (the product-path stimulus in the
//! companion regression `crates/circuit-backend/tests/source_breakpoint_regression.rs`)
//! produces four breakpoints per period. The engine
//!
//! * clamps the step to the distance to the next breakpoint
//!   (`transient.rs:1434-1439`),
//! * forces Backward-Euler for a step that *starts* at a breakpoint
//!   (`is_at_breakpoint` uses the step start, `:1433`; method selection
//!   `:1464-1468`),
//! * and shrinks that first step to `step_h.min(h * 0.1).max(h_min)` (`:1443`,
//!   where `h` is the *suggested* step of that iteration, not `tmax/10`).
//!
//! This bin measures, from the returned time axis only:
//!
//! 1. the first accepted step after each breakpoint (`h1`), against
//!    `min(2*h_prev, h_max) * 0.1` (the no-LTE-bound regime) and against
//!    `0.1 * h_max`;
//! 2. whether that step is a Backward-Euler step, by comparing the returned
//!    `v(out)` against the closed-form BE and Trapezoidal one-step updates for
//!    the *same* RC and the *same* input samples (a linear circuit converges in
//!    one NR iteration, so both closed forms are exact characterisations of the
//!    two integrators, not approximations);
//! 3. the §17 (atol 1e-5 V, rtol 1e-3) residual of the whole trace against the
//!    exact piecewise-linear analytic solution, per `max_step`;
//! 4. whether `reltol`/`abstol`/`trtol` can move the restart step (they cannot
//!    when `h_max` pins it);
//! 5. a minimal reproduction of the first post-breakpoint step.
//!
//! # Exit code (predictable, documented)
//!
//! * `0` — every experiment ran; every **contract check** (`[PASS]`/`[FAIL]`,
//!   the mechanism claims above) passed; any §17 row that exceeded the
//!   criterion is printed and listed as `[NOT-MET]`.
//! * `1` — at least one contract check failed, or an experiment panicked
//!   (`catch_unwind` maps a panic to 1 instead of the default 101).
//!
//! `[NOT-MET]` rows do **not** flip the exit code: this bin studies the kernel,
//! and a §17 miss is a finding to be kept and reported, not an execution
//! failure. §17 *acceptance* lives in the product-path regression.
//! A `[CONTRACT]` failure is a different thing: it means the mechanism claimed
//! here no longer matches the engine, and then the conclusion of this study is
//! invalid.
//!
//! The kernel source is never modified by this bin.

use cirq_ir::{
    Analysis, Circuit, Connection, Element, ElementKind, Id, Net, SourceSpec, TranAnalysis, Value,
    Waveform,
};
use thevenin::circuit::simulate_tran;
use thevenin_types::{SimResult, VectorData};

// ---------------------------------------------------------------------------
// Stimulus (identical to the product-path regression, so the two files describe
// one problem) and circuit
// ---------------------------------------------------------------------------

const R_OHM: f64 = 1_000.0;
const C_FARAD: f64 = 100e-9;
/// `tau = R*C = 100 us`.
const TAU: f64 = R_OHM * C_FARAD;
const V0: f64 = 1.0;
/// PULSE delay (`td`).
const TD: f64 = 100e-6;
/// Declared PULSE rise time (`tr`).
const TR: f64 = 1e-6;
/// Declared PULSE fall time (`tf`).
const TF: f64 = 1e-6;
/// PULSE high time (`pw`).
const PW: f64 = 5e-6;
/// PULSE period (`per`): 10 pulses inside the window.
const PER: f64 = 20e-6;
/// `tran stop`.
const STOP: f64 = 300e-6;
/// §17 transient criterion (fixed by the brief; never relaxed).
const ATOL: f64 = 1e-5;
const RTOL: f64 = 1e-3;
/// The product adapter's print step for this circuit
/// (`circuit_backend::thevenin::print_step_for`):
/// `min(span/1000, min(rise, fall, period)) = min(300 ns, 1 us) = 300 ns`.
const PRINT_STEP: f64 = 300e-9;

// ---------------------------------------------------------------------------
// cirq_ir builders (same shapes as _probe/src/main.rs; that file is untouched)
// ---------------------------------------------------------------------------

fn net(id: u32, name: &str) -> Net {
    Net {
        id: Id(id),
        name: name.to_string(),
        is_global: false,
    }
}

fn conn(terminal: &str, net: u32) -> Connection {
    Connection {
        terminal: terminal.to_string(),
        net: Id(net),
    }
}

fn resistor(id: u32, name: &str, p: u32, n: u32, value: f64) -> Element {
    Element {
        id: Id(id),
        name: name.to_string(),
        kind: ElementKind::Resistor,
        connections: vec![conn("pos", p), conn("neg", n)],
        params: vec![("value".to_string(), Value::Real(value))],
        model: None,
        source_spec: None,
    }
}

fn capacitor(id: u32, name: &str, p: u32, n: u32, value: f64) -> Element {
    Element {
        id: Id(id),
        name: name.to_string(),
        kind: ElementKind::Capacitor,
        connections: vec![conn("pos", p), conn("neg", n)],
        params: vec![("value".to_string(), Value::Real(value))],
        model: None,
        source_spec: None,
    }
}

fn vsource(id: u32, name: &str, p: u32, n: u32, spec: SourceSpec) -> Element {
    Element {
        id: Id(id),
        name: name.to_string(),
        kind: ElementKind::VoltageSource,
        connections: vec![conn("pos", p), conn("neg", n)],
        params: Vec::new(),
        model: None,
        source_spec: Some(spec),
    }
}

// ---------------------------------------------------------------------------
// Runs
// ---------------------------------------------------------------------------

/// One `.tran` configuration. `step` is `TranAnalysis::step` (`h_print`),
/// `tmax` is `TranAnalysis::tmax`; `options` is `circuit.options`.
#[derive(Clone)]
struct Case {
    step: f64,
    tmax: Option<f64>,
    options: Vec<(String, Value)>,
}

impl Case {
    fn new(step: f64, tmax: Option<f64>) -> Self {
        Case {
            step,
            tmax,
            options: Vec::new(),
        }
    }
    fn with(mut self, options: Vec<(String, Value)>) -> Self {
        self.options = options;
        self
    }
    /// The engine's `h_max` (`transient.rs:799`).
    fn h_max(&self) -> f64 {
        self.tmax.unwrap_or_else(|| self.step.min(STOP / 50.0))
    }
}

struct Trace {
    time: Vec<f64>,
    vin: Vec<f64>,
    vout: Vec<f64>,
}

impl Trace {
    fn points(&self) -> usize {
        self.time.len()
    }
    fn dt_min(&self) -> f64 {
        self.time
            .windows(2)
            .map(|w| w[1] - w[0])
            .fold(f64::INFINITY, f64::min)
    }
    fn dt_max(&self) -> f64 {
        self.time
            .windows(2)
            .map(|w| w[1] - w[0])
            .fold(f64::NEG_INFINITY, f64::max)
    }
    /// Field-for-field equality (point count, axis, values).
    fn identical_to(&self, other: &Trace) -> bool {
        self.time.len() == other.time.len()
            && self.vin.len() == other.vin.len()
            && self.vout.len() == other.vout.len()
            && self.time.iter().zip(&other.time).all(|(a, b)| a == b)
            && self.vin.iter().zip(&other.vin).all(|(a, b)| a == b)
            && self.vout.iter().zip(&other.vout).all(|(a, b)| a == b)
    }
    /// Index of the first sample where the two axes differ, if any.
    fn first_axis_diff(&self, other: &Trace) -> Option<usize> {
        let n = self.time.len().min(other.time.len());
        (0..n).find(|&i| self.time[i] != other.time[i])
    }
    /// Max |dv(out)| over the shared time prefix (undefined past a divergence).
    fn max_dv_on_prefix(&self, other: &Trace) -> f64 {
        let n = self
            .first_axis_diff(other)
            .unwrap_or(self.time.len().min(other.time.len()));
        (0..n)
            .map(|i| (self.vout[i] - other.vout[i]).abs())
            .fold(0.0, f64::max)
    }
}

fn real(v: &thevenin_types::SimVector) -> Vec<f64> {
    match &v.data {
        VectorData::Real(d) => d.clone(),
        VectorData::Complex(_) => panic!("expected a real vector {}", v.name),
    }
}

fn plot_named<'a>(res: &'a SimResult, prefix: &str) -> Option<&'a thevenin_types::SimPlot> {
    res.plots.iter().find(|p| {
        p.name
            .to_ascii_lowercase()
            .starts_with(&prefix.to_ascii_lowercase())
    })
}

fn run_case(case: &Case, label: &str) -> Result<Trace, String> {
    let mut c = Circuit {
        name: format!("rc_pulse_{label}"),
        nets: vec![net(0, "gnd"), net(1, "in"), net(2, "out")],
        elements: Vec::new(),
        models: Vec::new(),
        analyses: Vec::new(),
        params: Vec::new(),
        csparams: Vec::new(),
        options: case.options.clone(),
        temps: Vec::new(),
        save: Vec::new(),
        funcs: Vec::new(),
        initial_conditions: Vec::new(),
        nodeset: Vec::new(),
        measures: Vec::new(),
        code_blocks: Vec::new(),
        raw_directives: Vec::new(),
    };
    c.elements.push(vsource(
        0,
        "v1",
        1,
        0,
        SourceSpec {
            dc: Some(0.0),
            waveform: Some(Waveform::Pulse {
                v1: 0.0,
                v2: V0,
                td: Some(TD),
                tr: Some(TR),
                tf: Some(TF),
                pw: Some(PW),
                per: Some(PER),
            }),
            ..Default::default()
        },
    ));
    c.elements.push(resistor(1, "r1", 1, 2, R_OHM));
    c.elements.push(capacitor(2, "c1", 2, 0, C_FARAD));
    c.analyses.push(Analysis::Tran(TranAnalysis {
        step: case.step,
        stop: STOP,
        start: 0.0,
        uic: false,
        tmax: case.tmax,
    }));

    let res = simulate_tran(&c).map_err(|e| format!("{label}: simulate_tran error: {e}"))?;
    let plot = plot_named(&res, "tran")
        .ok_or_else(|| format!("{label}: no 'tran*' plot in the result"))?;
    let time = real(
        plot.vector("time")
            .ok_or_else(|| format!("{label}: no 'time' vector"))?,
    );
    let vin = real(
        plot.vector("v(in)")
            .ok_or_else(|| format!("{label}: no 'v(in)' vector"))?,
    );
    let vout = real(
        plot.vector("v(out)")
            .ok_or_else(|| format!("{label}: no 'v(out)' vector"))?,
    );
    if time.len() != vin.len() || time.len() != vout.len() {
        return Err(format!(
            "{label}: vector length mismatch: n(time)={}, n(v(in))={}, n(v(out))={}",
            time.len(),
            vin.len(),
            vout.len()
        ));
    }
    Ok(Trace { time, vin, vout })
}

// ---------------------------------------------------------------------------
// Analytic reference: the exact piecewise-linear solution of
// tau*y' + y = v_in, for the PULSE the engine really used
// ---------------------------------------------------------------------------

/// `S(x) = x + expm1(-x)`: stably evaluated, series for `x < 1e-2`
/// (relative truncation error <= 3.97e-14 at the switch-over).
fn s_stable(x: f64) -> f64 {
    if x < 1e-2 {
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
    /// Input value at `t0`.
    v0: f64,
    /// Input slope, V/s.
    m: f64,
}

/// Exact solution, plus the input, of the RC driven by a piecewise-linear
/// input. On a segment the update is
///
/// ```text
/// x  = u/tau
/// y  = v0 + m*tau*S(x) + (y_start - v0)*exp(-x)
/// ```
///
/// which is the algebraically rearranged (and cancellation-free) form of
/// `y = v0 + m*u - m*tau + (y_start - v0 + m*tau)*exp(-x)`; the naive form
/// cancels catastrophically near a ramp start where `y ~ m*tau*x^2/2`.
struct Reference {
    segs: Vec<Segment>,
    /// `y` at each segment start (exact recurrence, no per-call accumulation).
    ys: Vec<f64>,
}

impl Reference {
    /// Build the reference for the engine's PULSE semantics
    /// (`thevenin-0.5.0/src/waveform.rs:116-156`): clamp `tr`/`tf` up to
    /// `tstep`, default `pw` to `tstop`, then fold `t - td` into the period.
    fn new(tstep: f64) -> Self {
        let tr_used = TR.max(tstep);
        let tf_used = TF.max(tstep);
        let pw_used = PW.max(0.0);
        let period = PER.max(tr_used + pw_used + tf_used).max(tstep);

        let mut segs: Vec<Segment> = Vec::new();
        // Quiescent input before the delay.
        segs.push(Segment {
            t0: 0.0,
            t1: TD.min(STOP),
            v0: 0.0,
            m: 0.0,
        });
        let mut k = 0u64;
        loop {
            let base = TD + k as f64 * period;
            if base >= STOP {
                break;
            }
            let edges = [
                (base, base + tr_used, 0.0, V0 / tr_used),
                (base + tr_used, base + tr_used + pw_used, V0, 0.0),
                (
                    base + tr_used + pw_used,
                    base + tr_used + pw_used + tf_used,
                    V0,
                    -V0 / tf_used,
                ),
                (base + tr_used + pw_used + tf_used, base + period, 0.0, 0.0),
            ];
            for (a, b, v0, m) in edges {
                let a = a.max(0.0);
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
        // Merge touching segments and drop the tail after the last edge.
        segs.sort_by(|a, b| a.t0.total_cmp(&b.t0));
        let mut ys = Vec::with_capacity(segs.len());
        let mut y = 0.0_f64;
        for (i, s) in segs.iter().enumerate() {
            if i == 0 {
                assert!(s.t0 == 0.0, "the reference must start at t = 0");
                ys.push(0.0);
                continue;
            }
            let prev = segs[i - 1];
            let u = s.t0 - prev.t0;
            let x = u / TAU;
            y = prev.v0 + prev.m * TAU * s_stable(x) + (y - prev.v0) * (-x).exp();
            ys.push(y);
        }
        Reference { segs, ys }
    }

    /// Exact `v_in` from the same segments (an independent implementation from
    /// the engine-replica used in the self-check below).
    fn v_in(&self, t: f64) -> f64 {
        for s in &self.segs {
            if t >= s.t0 && t <= s.t1 {
                return s.v0 + s.m * (t - s.t0);
            }
        }
        // After the last segment the input is the low level.
        0.0
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

    /// Analytic derivative `y'` on the segment containing `t` (used for the
    /// ODE-residual self-check).
    fn dy(&self, t: f64) -> f64 {
        for (i, s) in self.segs.iter().enumerate() {
            if t >= s.t0 && t <= s.t1 {
                let y0 = self.ys[i];
                let x = (t - s.t0) / TAU;
                return s.m * (1.0 - (-x).exp()) - (y0 - s.v0) * (-x).exp() / TAU;
            }
        }
        0.0
    }

    /// Breakpoint times of the input (the same set the engine builds in
    /// `waveform::breakpoints`, `waveform.rs:257-302`).
    fn breakpoints(&self, tstep: f64) -> Vec<f64> {
        let tr_used = TR.max(tstep);
        let tf_used = TF.max(tstep);
        let pw_used = PW.max(0.0);
        let period = PER.max(tr_used + pw_used + tf_used).max(tstep);
        let edges = [0.0, tr_used, tr_used + pw_used, tr_used + pw_used + tf_used];
        let mut out = Vec::new();
        let mut k = 0u64;
        loop {
            let base = TD + k as f64 * period;
            if base > STOP {
                break;
            }
            for e in edges {
                let t = base + e;
                if t >= 0.0 && t <= STOP {
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
/// (`waveform.rs:116-156`), used only to self-check [`Reference::v_in`].
fn v_in_engine_replica(t: f64, tstep: f64) -> f64 {
    let tr_used = TR.max(tstep);
    let tf_used = TF.max(tstep);
    let period = PER.max(tr_used + PW + tf_used).max(tstep);
    if t < TD {
        return 0.0;
    }
    let mut time = t - TD;
    if period > 0.0 && time >= period {
        time -= period * (time / period).floor();
    }
    if time < tr_used {
        V0 * time / tr_used
    } else if time < tr_used + PW {
        V0
    } else if time < tr_used + PW + tf_used {
        V0 + (0.0 - V0) * (time - tr_used - PW) / tf_used
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------
// §17 statistics over every returned sample
// ---------------------------------------------------------------------------

struct ErrStats {
    n: usize,
    max_err: f64,
    max_t: f64,
    max_expected: f64,
    max_allow: f64,
    violations: usize,
    worst_ratio: f64,
    worst_ratio_t: f64,
}

impl ErrStats {
    fn summary(&self) -> String {
        format!(
            "n={} max|err|={:.6e} V at t={:.6e} s (expected={:.9} V, allowance={:.6e} V) \
             violations={} worst ratio={:.3}x at t={:.6e} s",
            self.n,
            self.max_err,
            self.max_t,
            self.max_expected,
            self.max_allow,
            self.violations,
            self.worst_ratio,
            self.worst_ratio_t
        )
    }
}

fn err_stats(t: &[f64], v: &[f64], reference: &Reference) -> ErrStats {
    let mut st = ErrStats {
        n: t.len(),
        max_err: 0.0,
        max_t: f64::NAN,
        max_expected: f64::NAN,
        max_allow: f64::NAN,
        violations: 0,
        worst_ratio: 0.0,
        worst_ratio_t: f64::NAN,
    };
    for (i, &ti) in t.iter().enumerate() {
        let expected = reference.y(ti);
        let err = (v[i] - expected).abs();
        let allow = ATOL + RTOL * expected.abs();
        if err > allow {
            st.violations += 1;
        }
        let ratio = err / allow;
        if ratio > st.worst_ratio {
            st.worst_ratio = ratio;
            st.worst_ratio_t = ti;
        }
        if err > st.max_err {
            st.max_err = err;
            st.max_t = ti;
            st.max_expected = expected;
            st.max_allow = allow;
        }
    }
    st
}

/// The list of violating samples, kept verbatim (never dropped).
fn violations(t: &[f64], v: &[f64], reference: &Reference, limit: usize) -> (usize, Vec<String>) {
    let mut n = 0usize;
    let mut shown = Vec::new();
    for (i, &ti) in t.iter().enumerate() {
        let expected = reference.y(ti);
        let err = (v[i] - expected).abs();
        let allow = ATOL + RTOL * expected.abs();
        if err > allow {
            n += 1;
            if shown.len() < limit {
                shown.push(format!(
                    "t={ti:.9e} s: |err|={err:.6e} V > allowance={allow:.6e} V \
                     (expected={expected:.9} V, actual={:.9} V, ratio={:.3}x)",
                    v[i],
                    err / allow
                ));
            }
        }
    }
    (n, shown)
}

// ---------------------------------------------------------------------------
// Integrator step models (exact for a linear RC + piecewise-linear input)
// ---------------------------------------------------------------------------

/// Backward-Euler one-step update of `tau*v' + v = v_in` on `[t0, t0+h]`.
fn be_step(v0: f64, h: f64, vin_new: f64) -> f64 {
    (v0 + h * vin_new / TAU) / (1.0 + h / TAU)
}

/// Trapezoidal one-step update of `tau*v' + v = v_in` on `[t0, t0+h]`
/// (the trapezoid rule is exact for the linear input).
fn trap_step(v0: f64, h: f64, vin_old: f64, vin_new: f64) -> f64 {
    ((2.0 * TAU / h - 1.0) * v0 + vin_old + vin_new) / (2.0 * TAU / h + 1.0)
}

// ---------------------------------------------------------------------------
// Breakpoint restart analysis
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct BpRow {
    bp: f64,
    /// `0` for the first period, then the pulse index.
    k: u64,
    kind: &'static str,
    h_before: f64,
    h1: f64,
    /// `min(2*h_before, h_max) * 0.1` — the restart step the engine's rules
    /// produce when the LTE is not binding.
    pred_h1: f64,
    /// Exact input/response at the breakpoint (the step start).
    v_exact: f64,
    v_obs: f64,
    /// Exact response at the *end* of the restart step, and the engine's error
    /// there: this is the quantity the forced-BE step is wrong by.
    v_next_exact: f64,
    v_next_obs: f64,
    err_next: f64,
    be: f64,
    trap: f64,
    be_resid: f64,
    trap_resid: f64,
}

/// Analyse the first accepted step after every breakpoint in the first
/// `periods` pulses. `bp_index` is matched to the returned axis within
/// `1e-12 s`.
fn breakpoint_rows(
    t: &[f64],
    vout: &[f64],
    reference: &Reference,
    tstep: f64,
    h_max: f64,
    periods: u64,
) -> Vec<BpRow> {
    let tr_used = TR.max(tstep);
    let tf_used = TF.max(tstep);
    let period = PER.max(tr_used + PW + tf_used).max(tstep);
    let mut rows = Vec::new();
    for k in 0..periods {
        let base = TD + k as f64 * period;
        let kinds: [(&'static str, f64); 4] = [
            ("rise_start", base),
            ("rise_end", base + tr_used),
            ("fall_start", base + tr_used + PW),
            ("fall_end", base + tr_used + PW + tf_used),
        ];
        for (kind, bp) in kinds {
            if bp >= STOP {
                continue;
            }
            let Some(i) = (0..t.len()).find(|&i| (t[i] - bp).abs() < 1e-12) else {
                continue;
            };
            if i == 0 || i + 1 >= t.len() {
                continue;
            }
            let h1 = t[i + 1] - t[i];
            let h_before = t[i] - t[i - 1];
            let pred_h1 = (2.0 * h_before).min(h_max) * 0.1;
            let v_exact = reference.y(bp);
            let v_obs = vout[i];
            let vin_old = reference.v_in(t[i]);
            let vin_new = reference.v_in(t[i + 1]);
            let be = be_step(v_obs, h1, vin_new);
            let trap = trap_step(v_obs, h1, vin_old, vin_new);
            let v_next_exact = reference.y(t[i + 1]);
            rows.push(BpRow {
                bp,
                k,
                kind,
                h_before,
                h1,
                pred_h1,
                v_exact,
                v_obs,
                v_next_exact,
                v_next_obs: vout[i + 1],
                err_next: (vout[i + 1] - v_next_exact).abs(),
                be,
                trap,
                be_resid: (vout[i + 1] - be).abs(),
                trap_resid: (vout[i + 1] - trap).abs(),
            });
        }
    }
    rows
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

struct Report {
    checks: Vec<(&'static str, bool)>,
    not_met: Vec<String>,
}

impl Report {
    fn new() -> Self {
        Report {
            checks: Vec::new(),
            not_met: Vec::new(),
        }
    }
    fn check(&mut self, name: &'static str, ok: bool) -> bool {
        println!("  [{}] {name}", if ok { "PASS" } else { "FAIL" });
        self.checks.push((name, ok));
        ok
    }
    fn failures(&self) -> Vec<&'static str> {
        self.checks
            .iter()
            .filter(|(_, ok)| !ok)
            .map(|(n, _)| *n)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Study
// ---------------------------------------------------------------------------

fn run_all() -> i32 {
    println!("=== Thevenin 0.5.0 source-breakpoint study (`breakpoint_study`) ===");
    println!(
        "circuit : gnd - v1(PULSE) - in - r1(1k) - out - c1(100n) - gnd ; tau = {:.1} us",
        TAU * 1e6
    );
    println!(
        "source  : v1=0 V, v2=1 V, td={:.0} us, tr={:.0} us, tf={:.0} us, pw={:.0} us, per={:.0} us",
        TD * 1e6,
        TR * 1e6,
        TF * 1e6,
        PW * 1e6,
        PER * 1e6
    );
    println!(
        "window  : stop={:.0} us -> {} pulses; .tran step = {:.0} ns (the product adapter's rule)",
        STOP * 1e6,
        ((STOP - TD) / PER).ceil() as u64,
        PRINT_STEP * 1e9
    );
    println!("criterion: §17 TRAN atol={ATOL:e} V, rtol={RTOL:e} (fixed; never relaxed here)");
    println!();

    let mut report = Report::new();

    // -----------------------------------------------------------------
    // 0. Reference self-check (ODE residual + independent input replica)
    // -----------------------------------------------------------------
    println!("--- 0. reference self-check ---");
    let reference = Reference::new(PRINT_STEP);
    println!(
        "  segments: {}; breakpoints in the window: {}",
        reference.segs.len(),
        reference.breakpoints(PRINT_STEP).len()
    );
    // ODE residual: tau*y' + y - v_in = 0, with the analytic derivative.
    // The grid mixes a uniform sweep with a dense sweep across the first ramp
    // (where the 2nd-order cancellation would show up if the reference used
    // the naive ramp form).
    let mut worst_residual = 0.0_f64;
    let mut worst_residual_at = 0.0_f64;
    let mut worst_input = 0.0_f64;
    let check = |t: f64, worst_residual: &mut f64, worst_residual_at: &mut f64| {
        let r = TAU * reference.dy(t) + reference.y(t) - reference.v_in(t);
        if r.abs() > *worst_residual {
            *worst_residual = r.abs();
            *worst_residual_at = t;
        }
    };
    for i in 0..=300_000u32 {
        check(
            i as f64 * STOP / 300_000.0,
            &mut worst_residual,
            &mut worst_residual_at,
        );
    }
    for i in 0..=2000u32 {
        check(
            TD + i as f64 * TR / 2000.0,
            &mut worst_residual,
            &mut worst_residual_at,
        );
        check(
            TD - i as f64 * TR / 2000.0 + TR,
            &mut worst_residual,
            &mut worst_residual_at,
        );
    }
    for i in 0..=300_000u32 {
        let t = i as f64 * STOP / 300_000.0;
        let d = (reference.v_in(t) - v_in_engine_replica(t, PRINT_STEP)).abs();
        if d > worst_input {
            worst_input = d;
        }
    }
    println!(
        "  ODE residual max |tau*y' + y - v_in| = {worst_residual:.3e} V at t={worst_residual_at:.6e} s"
    );
    println!(
        "  |reference.v_in - engine PULSE replica| max = {worst_input:.3e} V (independent evaluator)"
    );
    report.check("ref/ode_residual_below_1e-12", worst_residual < 1e-12);
    report.check("ref/input_matches_engine_replica", worst_input < 1e-12);
    // The shifted-ramp law for a pure ramp from quiescence, an independent
    // algebraic route to the same numbers: y(u) = m*tau*(x + expm1(-x)).
    let m = V0 / TR;
    let mut worst_ramp = 0.0_f64;
    for i in 0..=1000u32 {
        let u = i as f64 * TR / 1000.0;
        let x = u / TAU;
        let a = m * TAU * (x + (-x).exp_m1());
        let b = m * u - m * TAU + m * TAU * (-x).exp();
        if (a - b).abs() > worst_ramp {
            worst_ramp = (a - b).abs();
        }
    }
    println!(
        "  ramp-form cross-check |m*tau*S(x) - (m*u - m*tau + m*tau*e^-x)| max = {worst_ramp:.3e} V"
    );
    println!();

    // -----------------------------------------------------------------
    // 1. baseline configuration
    // -----------------------------------------------------------------
    let baseline = Case::new(PRINT_STEP, Some(TAU / 1000.0));
    println!(
        "--- 1. baseline: .tran step = {:.0} ns, tmax = tau/1000 = {:.0} ns -> h_max = {:.3e} s ---",
        baseline.step * 1e9,
        TAU / 1000.0 * 1e9,
        baseline.h_max()
    );
    let base = match run_case(&baseline, "baseline") {
        Ok(t) => t,
        Err(e) => {
            println!("  [FAIL] {e}");
            report.check("baseline/run", false);
            return finish(report);
        }
    };
    report.check("baseline/run", true);
    println!(
        "  returned points = {}, dt_min = {:.3e} s, dt_max = {:.3e} s",
        base.points(),
        base.dt_min(),
        base.dt_max()
    );
    let base_stats = err_stats(&base.time, &base.vout, &reference);
    println!("  §17 over ALL points: {}", base_stats.summary());
    let (nv, shown) = violations(&base.time, &base.vout, &reference, 5);
    if nv > 0 {
        println!(
            "  [NOT-MET] {nv} point(s) exceed §17 in the baseline configuration (kept verbatim):"
        );
        for s in &shown {
            println!("      {s}");
        }
        report.not_met.push(format!(
            "baseline (max_step = tau/1000 = {:.0} ns): {nv}/{} points exceed §17, max |err| = {:.4e} V at t = {:.6e} s",
            TAU / 1000.0 * 1e9,
            base_stats.n,
            base_stats.max_err,
            base_stats.max_t
        ));
    } else {
        println!("  [MET] every returned point is inside §17 in the baseline configuration");
    }

    // Breakpoint rows.
    let rows = breakpoint_rows(
        &base.time,
        &base.vout,
        &reference,
        baseline.step,
        baseline.h_max(),
        3,
    );
    println!();
    println!("  first accepted step after each breakpoint (3 pulses = 12 breakpoints):");
    println!(
        "    {:>4} {:>11} {:>13} {:>13} {:>11} {:>11} {:>12} {:>11}",
        "k", "kind", "bp [s]", "h_before [s]", "h1 [s]", "pred 0.1h", "|obs-BE|", "|obs-TRAP|"
    );
    for r in &rows {
        println!(
            "    {:>4} {:>11} {:>13.6e} {:>13.6e} {:>11.4e} {:>11.4e} {:>12.3e} {:>11.3e}",
            r.k, r.kind, r.bp, r.h_before, r.h1, r.pred_h1, r.be_resid, r.trap_resid
        );
    }
    let first = rows.iter().find(|r| r.k == 0 && r.kind == "rise_start");
    match first {
        Some(r) => {
            let expected = 0.1 * baseline.h_max();
            let rel = (r.h1 - expected).abs() / expected;
            println!(
                "  first pulse rise_start: h1 = {:.9e} s vs 0.1*h_max = {:.9e} s (rel.diff {:.3e}); \
                 h_before = {:.9e} s, min(2*h_before, h_max) = {:.9e} s",
                r.h1,
                expected,
                rel,
                r.h_before,
                (2.0 * r.h_before).min(baseline.h_max())
            );
            report.check("bp/first_restart_step_is_0.1_of_hmax", rel <= 1e-9);
            println!(
                "  first pulse rise_start: v at the breakpoint = {:.15} V (exact {:.15}); \
                 v at the end of the restart step: engine = {:.15} V, BE = {:.15} V, \
                 TRAP = {:.15} V, exact = {:.15} V",
                r.v_obs, r.v_exact, r.v_next_obs, r.be, r.trap, r.v_next_exact
            );
            println!(
                "      |engine - BE| = {:.3e} V, |engine - TRAP| = {:.3e} V, \
                 |engine - exact| = {:.3e} V (the BE restart error)",
                r.be_resid, r.trap_resid, r.err_next
            );
            let pred_err = (V0 / TR) * r.h1 * r.h1 / (2.0 * TAU);
            println!(
                "      forced-BE local error prediction (V0/T)*h1^2/(2*tau) = {pred_err:.6e} V \
                 (observed {:.6e} V, ratio {:.4})",
                r.err_next,
                r.err_next / pred_err
            );
            report.check(
                "bp/first_restart_is_backward_euler",
                r.be_resid < r.trap_resid && r.be_resid <= 1e-9,
            );
            let ratio = r.err_next / pred_err;
            report.check(
                "bp/first_restart_error_follows_be_law",
                (0.5..=2.0).contains(&ratio),
            );
        }
        None => {
            println!("  [FAIL] no breakpoint row for the first rise_start");
            report.check("bp/first_restart_step_is_0.1_of_hmax", false);
        }
    }
    // How many of the 12 rows match the `min(2*h_prev, h_max)*0.1` rule, and how
    // many are Backward-Euler steps?
    let mut n_pred = 0usize;
    let mut n_be = 0usize;
    let mut worst_be = 0.0_f64;
    for r in &rows {
        if (r.h1 - r.pred_h1).abs() / r.pred_h1 <= 1e-9 {
            n_pred += 1;
        }
        if r.be_resid < r.trap_resid {
            n_be += 1;
        }
        worst_be = worst_be.max(r.be_resid);
    }
    println!(
        "  rows matching min(2*h_prev, h_max)*0.1 within 1e-9 rel.: {n_pred}/{}; \
         rows closer to the Backward-Euler update than to Trapezoidal: {n_be}/{}; \
         worst |obs - BE| = {worst_be:.3e} V",
        rows.len(),
        rows.len()
    );
    report.check("bp/restart_step_matches_engine_rule", n_pred == rows.len());
    report.check(
        "bp/restart_step_is_backward_euler_on_every_row",
        n_be == rows.len(),
    );
    report.check("bp/backward_euler_residual_below_1e-9_V", worst_be <= 1e-9);
    println!();

    // -----------------------------------------------------------------
    // 2. minimal reproduction of the offending step
    // -----------------------------------------------------------------
    println!("--- 2. minimal reproduction: the first post-delay breakpoint ---");
    if let Some(i) = (0..base.time.len()).find(|&i| (base.time[i] - TD).abs() < 1e-12) {
        println!(
            "    {:>22} {:>15} {:>18} {:>18} {:>13} {:>13} {:>13}",
            "t [s]", "h [s]", "v(in) [V]", "v(out) [V]", "exact [V]", "|err| [V]", "allow [V]"
        );
        for j in i.saturating_sub(2)..=(i + 4).min(base.time.len() - 1) {
            let h = if j == 0 {
                f64::NAN
            } else {
                base.time[j] - base.time[j - 1]
            };
            let exact = reference.y(base.time[j]);
            let err = (base.vout[j] - exact).abs();
            println!(
                "    {:>22.15e} {:>15.4e} {:>18.12} {:>18.12} {:>13.9} {:>13.3e} {:>13.3e}",
                base.time[j],
                h,
                base.vin[j],
                base.vout[j],
                exact,
                err,
                ATOL + RTOL * exact.abs()
            );
        }
    } else {
        println!("    [FAIL] the axis has no sample at the delay breakpoint t = {TD:.6e} s");
        report.check("bp/axis_lands_on_delay_breakpoint", false);
    }
    println!();

    // -----------------------------------------------------------------
    // 3. max_step sweep
    // -----------------------------------------------------------------
    println!(
        "--- 3. max_step sweep (.tran step = {:.0} ns fixed) ---",
        PRINT_STEP * 1e9
    );
    println!(
        "    {:>12} {:>10} {:>8} {:>13} {:>13} {:>16} {:>10} {:>16} {:>11}",
        "tmax",
        "h_max [s]",
        "points",
        "h1(1st bp)",
        "pred h1",
        "max |err| [V]",
        "at u/tau",
        "pred BE err",
        "over-limit"
    );
    let sweep: [(&str, Option<f64>); 6] = [
        ("tau/50", Some(TAU / 50.0)),
        ("tau/200", Some(TAU / 200.0)),
        ("tau/500", Some(TAU / 500.0)),
        ("tau/1000", Some(TAU / 1000.0)),
        ("tau/5000", Some(TAU / 5000.0)),
        ("None (adapter default)", None),
    ];
    // name, h_max, points, h1, predicted h1 (engine rule), max err, violations
    let mut sweep_rows: Vec<(String, f64, usize, f64, f64, f64, usize)> = Vec::new();
    for (name, tmax) in sweep {
        let case = Case::new(PRINT_STEP, tmax);
        let h_max = case.h_max();
        match run_case(&case, name) {
            Ok(tr) => {
                let st = err_stats(&tr.time, &tr.vout, &reference);
                let rows = breakpoint_rows(&tr.time, &tr.vout, &reference, case.step, h_max, 1);
                let (h1, pred_h1) = rows
                    .iter()
                    .find(|r| r.k == 0 && r.kind == "rise_start")
                    .map(|r| (r.h1, r.pred_h1))
                    .unwrap_or((f64::NAN, f64::NAN));
                let pred = (V0 / TR) * h1 * h1 / (2.0 * TAU);
                println!(
                    "    {:>12} {:>10.3e} {:>8} {:>13.4e} {:>13.4e} {:>16.4e} {:>10.4} {:>16.4e} {:>11}",
                    name,
                    h_max,
                    tr.points(),
                    h1,
                    pred_h1,
                    st.max_err,
                    (st.max_t - TD) / TAU,
                    pred,
                    st.violations
                );
                let (nv, shown) = violations(&tr.time, &tr.vout, &reference, 3);
                if nv > 0 {
                    println!(
                        "        [NOT-MET] {nv}/{} points exceed §17 (kept, first 3 shown):",
                        st.n
                    );
                    for s in &shown {
                        println!("            {s}");
                    }
                    report.not_met.push(format!(
                        "{name} (h_max = {h_max:.4e} s): {nv}/{} points exceed §17, max |err| = {:.4e} V at t = {:.6e} s, h1 = {:.4e} s",
                        st.n, st.max_err, st.max_t, h1
                    ));
                } else {
                    println!("        [MET] all {} points inside §17", st.n);
                }
                sweep_rows.push((
                    name.to_string(),
                    h_max,
                    tr.points(),
                    h1,
                    pred_h1,
                    st.max_err,
                    st.violations,
                ));
            }
            Err(e) => {
                println!("    [FAIL] {name}: {e}");
                report.check("sweep/run", false);
            }
        }
    }
    if sweep_rows.len() == 6 {
        // The trend is a function of h_max, and the `tmax = None` row has
        // h_max = 300 ns, so sort by h_max (descending) before testing it.
        let mut by_h: Vec<_> = sweep_rows.iter().collect();
        by_h.sort_by(|a, b| b.1.total_cmp(&a.1));
        println!(
            "    trend (sorted by h_max descending): {:?}",
            by_h.iter()
                .map(|r| format!(
                    "{}: h_max={:.3e} pts={} h1={:.3e} err={:.3e} viol={}",
                    r.0, r.1, r.2, r.3, r.5, r.6
                ))
                .collect::<Vec<_>>()
        );
        let counts: Vec<usize> = by_h.iter().map(|r| r.2).collect();
        let errs: Vec<f64> = by_h.iter().map(|r| r.5).collect();
        let viols: Vec<usize> = by_h.iter().map(|r| r.6).collect();
        println!("    trend: points {counts:?} (h_max decreasing -> points non-decreasing)");
        println!("    trend: max |err| {errs:?}");
        println!("    trend: over-limit {viols:?}");
        let monotone_points = counts.windows(2).all(|w| w[1] >= w[0]);
        // max |err| is NOT asserted to be monotone: the reported peak sits at
        // u/tau ~ 1.8 in the coarse rows and moves as the grid changes.
        println!(
            "    trend note: max |err| is not expected to be strictly monotone - the reported peak \
             moves with the grid; the §17 violation count is the meaningful column."
        );
        report.check(
            "sweep/point_count_monotone_in_1_over_max_step",
            monotone_points,
        );
        // The §17 threshold predicted by the forced-BE restart law:
        //   (V0/T)*(0.1*h_max)^2/(2*tau) <= atol
        let bound = 10.0 * (2.0 * ATOL * TAU * TR / V0).sqrt();
        println!(
            "    derived §17 bound for the restart step: h_max <= 10*sqrt(2*atol*tau*T/V0) = {bound:.4e} s \
             (= tau/{:.1})",
            TAU / bound
        );
        let viol_above = sweep_rows.iter().filter(|r| r.1 > bound).all(|r| r.6 > 0);
        let viol_below = sweep_rows.iter().filter(|r| r.1 <= bound).all(|r| r.6 == 0);
        println!(
            "    rows with h_max above the bound exceed §17: {viol_above}; \
             rows at or below it are clean: {viol_below}"
        );
        report.check(
            "sweep/derived_bound_separates_met_from_not_met",
            viol_above && viol_below,
        );
    }
    println!();

    // -----------------------------------------------------------------
    // 4. tolerance experiments
    // -----------------------------------------------------------------
    println!("--- 4. reltol/abstol/trtol: single and interaction ---");
    let option_sets: [(&str, Vec<(String, Value)>); 5] = [
        ("empty options", Vec::new()),
        (
            "RELTOL=1e-12",
            vec![("RELTOL".to_string(), Value::Real(1e-12))],
        ),
        (
            "ABSTOL=1e-15",
            vec![("ABSTOL".to_string(), Value::Real(1e-15))],
        ),
        ("TRTOL=0.7", vec![("TRTOL".to_string(), Value::Real(0.7))]),
        (
            "RELTOL+ABSTOL",
            vec![
                ("RELTOL".to_string(), Value::Real(1e-12)),
                ("ABSTOL".to_string(), Value::Real(1e-15)),
            ],
        ),
    ];

    // Group P: h_max pinned by tmax.
    let pinned = Case::new(PRINT_STEP, Some(TAU / 1000.0));
    println!(
        "  group P (h_max pinned: tmax = tau/1000 = {:.0} ns, so h_max = {:.3e} s):",
        TAU / 1000.0 * 1e9,
        pinned.h_max()
    );
    println!(
        "    {:>18} {:>8} {:>13} {:>16} {:>11} {:>14}",
        "options", "points", "h1(1st bp)", "max |err| [V]", "over-limit", "same-as-empty"
    );
    let mut p_traces: Vec<Trace> = Vec::new();
    let mut p_h1: Vec<(String, f64, usize)> = Vec::new();
    for (label, opts) in &option_sets {
        match run_case(&pinned.clone().with(opts.clone()), label) {
            Ok(tr) => {
                let st = err_stats(&tr.time, &tr.vout, &reference);
                let rows = breakpoint_rows(
                    &tr.time,
                    &tr.vout,
                    &reference,
                    PRINT_STEP,
                    pinned.h_max(),
                    1,
                );
                let h1 = rows
                    .iter()
                    .find(|r| r.k == 0 && r.kind == "rise_start")
                    .map(|r| r.h1)
                    .unwrap_or(f64::NAN);
                let same = p_traces
                    .first()
                    .map(|b| b.identical_to(&tr))
                    .unwrap_or(true);
                println!(
                    "    {:>18} {:>8} {:>13.4e} {:>16.4e} {:>11} {:>14}",
                    label,
                    tr.points(),
                    h1,
                    st.max_err,
                    st.violations,
                    if p_traces.is_empty() {
                        "baseline".to_string()
                    } else {
                        same.to_string()
                    }
                );
                p_h1.push(((*label).to_string(), h1, st.violations));
                p_traces.push(tr);
            }
            Err(e) => {
                println!("    [FAIL] {label}: {e}");
                report.check("tol/group_p_run", false);
            }
        }
    }
    let p_identical = p_traces
        .first()
        .map(|b| p_traces.iter().all(|t| b.identical_to(t)))
        .unwrap_or(false);
    let p_single_identical = p_traces
        .first()
        .map(|b| p_traces.iter().take(4).all(|t| b.identical_to(t)))
        .unwrap_or(false);
    println!(
        "  [{}] group P single-factor question: RELTOL alone, ABSTOL alone, TRTOL alone are \
         field-for-field identical to the empty options (the tolerance channel is invisible there)",
        if p_single_identical { "PASS" } else { "FAIL" }
    );
    println!(
        "  group P interaction question: RELTOL+ABSTOL together {} the trace ({})",
        if p_identical {
            "did not change"
        } else {
            "changed"
        },
        if p_identical {
            "so no tolerance setting is observable under a pinned h_max"
        } else {
            "so the interaction IS observable even with h_max pinned by tmax; the restart step h1 \
             and the §17 verdict are unchanged, which is what matters below"
        }
    );
    if !p_identical {
        for i in 1..p_traces.len() {
            println!(
                "      {} vs empty: points {}/{}; first axis divergence at {:?}; max |dv| on shared prefix {:.3e} V",
                option_sets[i].0,
                p_traces[i].points(),
                p_traces[0].points(),
                p_traces[0].first_axis_diff(&p_traces[i]),
                p_traces[0].max_dv_on_prefix(&p_traces[i])
            );
        }
    }
    report.check(
        "tol/group_p_single_factors_not_observable",
        p_single_identical,
    );
    // The question that matters for the defect: can any tolerance setting move
    // the restart step or remove a §17 miss?
    let p_h1_same = p_h1
        .first()
        .map(|b| p_h1.iter().all(|r| (r.1 - b.1).abs() <= 1e-18))
        .unwrap_or(false);
    let p_all_met = p_h1.iter().all(|r| r.2 == 0);
    println!(
        "  group P restart step: h1 per setting = {:?} (identical: {p_h1_same}); all §17-met: {p_all_met}",
        p_h1.iter()
            .map(|r| format!("{}: {:.4e} s", r.0, r.1))
            .collect::<Vec<_>>()
    );
    report.check("tol/group_p_restart_step_unmoved_by_tolerances", p_h1_same);
    for r in &p_h1 {
        if r.2 > 0 {
            report.not_met.push(format!(
                "tolerance group P / {}: {} points exceed §17",
                r.0, r.2
            ));
        }
    }

    // Group F: h_max free (tmax = None -> h_max = min(step, stop/50) = step).
    let free = Case::new(PRINT_STEP, None);
    println!(
        "  group F (h_max free: tmax = None -> h_max = min(step, stop/50) = {:.3e} s):",
        free.h_max()
    );
    println!(
        "    {:>18} {:>8} {:>13} {:>16} {:>11} {:>14}",
        "options", "points", "h1(1st bp)", "max |err| [V]", "over-limit", "same-as-empty"
    );
    let mut f_traces: Vec<Trace> = Vec::new();
    let mut f_h1: Vec<(String, f64, usize)> = Vec::new();
    for (label, opts) in &option_sets {
        match run_case(&free.clone().with(opts.clone()), label) {
            Ok(tr) => {
                let st = err_stats(&tr.time, &tr.vout, &reference);
                let rows =
                    breakpoint_rows(&tr.time, &tr.vout, &reference, PRINT_STEP, free.h_max(), 1);
                let h1 = rows
                    .iter()
                    .find(|r| r.k == 0 && r.kind == "rise_start")
                    .map(|r| r.h1)
                    .unwrap_or(f64::NAN);
                let same = f_traces
                    .first()
                    .map(|b| b.identical_to(&tr))
                    .unwrap_or(true);
                println!(
                    "    {:>18} {:>8} {:>13.4e} {:>16.4e} {:>11} {:>14}",
                    label,
                    tr.points(),
                    h1,
                    st.max_err,
                    st.violations,
                    if f_traces.is_empty() {
                        "baseline".to_string()
                    } else {
                        same.to_string()
                    }
                );
                f_h1.push(((*label).to_string(), h1, st.violations));
                f_traces.push(tr);
            }
            Err(e) => {
                println!("    [FAIL] {label}: {e}");
                report.check("tol/group_f_run", false);
            }
        }
    }
    let f_identical = f_traces
        .first()
        .map(|b| f_traces.iter().all(|t| b.identical_to(t)))
        .unwrap_or(false);
    println!(
        "  group F verdict: {} (a tolerance change is observable here: {})",
        if f_identical {
            "IDENTICAL traces"
        } else {
            "traces differ"
        },
        !f_identical
    );
    println!(
        "  group F restart step: h1 per setting = {:?}",
        f_h1.iter()
            .map(|r| format!("{}: {:.4e} s, over-limit {}", r.0, r.1, r.2))
            .collect::<Vec<_>>()
    );
    let f_h1_same = f_h1
        .first()
        .map(|b| f_h1.iter().all(|r| (r.1 - b.1).abs() <= 1e-18))
        .unwrap_or(false);
    let f_all_met = f_h1.iter().all(|r| r.2 == 0);
    println!(
        "  group F: the restart step is unmoved by every tolerance setting: {f_h1_same}; \
         every setting stays inside §17 here: {f_all_met}"
    );
    report.check("tol/group_f_restart_step_unmoved_by_tolerances", f_h1_same);
    for r in &f_h1 {
        if r.2 > 0 {
            report.not_met.push(format!(
                "tolerance group F / {}: {} points exceed §17",
                r.0, r.2
            ));
        }
    }
    for i in 1..f_traces.len() {
        println!(
            "      {} vs empty: points {}/{}; first axis divergence at {:?}; max |dv| on shared prefix {:.3e} V",
            option_sets[i].0,
            f_traces[i].points(),
            f_traces[0].points(),
            f_traces[0].first_axis_diff(&f_traces[i]),
            f_traces[0].max_dv_on_prefix(&f_traces[i])
        );
    }

    // h_print is the only tolerance-independent lever: it sets h_max when
    // tmax is absent (and it also floors the PULSE edge).
    println!("  h_print lever (tmax = None; h_max = min(h_print, stop/50)):");
    println!(
        "    {:>14} {:>12} {:>8} {:>13} {:>16} {:>11}",
        "h_print [s]", "h_max [s]", "points", "h1(1st bp)", "max |err| [V]", "over-limit"
    );
    for step in [PRINT_STEP, 100e-9, 30e-9, 10e-9] {
        let case = Case::new(step, None);
        match run_case(&case, "hprint") {
            Ok(tr) => {
                let st = err_stats(&tr.time, &tr.vout, &reference);
                let rows = breakpoint_rows(&tr.time, &tr.vout, &reference, step, case.h_max(), 1);
                let h1 = rows
                    .iter()
                    .find(|r| r.k == 0 && r.kind == "rise_start")
                    .map(|r| r.h1)
                    .unwrap_or(f64::NAN);
                println!(
                    "    {:>14.3e} {:>12.3e} {:>8} {:>13.4e} {:>16.4e} {:>11}",
                    step,
                    case.h_max(),
                    tr.points(),
                    h1,
                    st.max_err,
                    st.violations
                );
                if st.violations > 0 {
                    report.not_met.push(format!(
                        "h_print = {step:.3e} s (h_max = {:.3e} s): {}/{} points exceed §17, max |err| = {:.4e} V",
                        case.h_max(), st.violations, st.n, st.max_err
                    ));
                }
            }
            Err(e) => {
                println!("    [FAIL] h_print = {step:.3e}: {e}");
                report.check("tol/hprint_run", false);
            }
        }
    }
    println!();

    // -----------------------------------------------------------------
    // 5. conclusions
    // -----------------------------------------------------------------
    println!("--- 5. conclusions (measured, not assumed) ---");
    println!(
        "  * the engine forces Backward-Euler for the step that STARTS at a breakpoint \
         (transient.rs:1433 -> :1464-1468) and shrinks it to min(step_h, h*0.1).max(h_min) (:1443);"
    );
    println!(
        "    measured: h1 = 0.1*h_max whenever h_max pins the suggested step, and the returned \
         v(out) matches the BE one-step update, not the trapezoidal one."
    );
    println!(
        "  * the BE restart error is (V0/T)*h1^2/(2*tau) at a quiescent ramp start; h1 scales \
         linearly with h_max, so the error scales as h_max^2 while the §17 allowance does not."
    );
    println!(
        "  * reltol/abstol/trtol cannot move h1 while tmax pins h_max; they are not an \
         experimental variable for this defect in that regime."
    );
    println!(
        "  * with tmax absent the adapter's own print-step rule already gives \
         h_max = min(span/1000, stop/50) = {:.3e} s, and the measured restart step there is \
         h1 = {:.3e} s;",
        free.h_max(),
        f_h1.first().map(|r| r.1).unwrap_or(f64::NAN)
    );
    println!(
        "    h1 is not always 0.1*h_max: the step that lands on the breakpoint is clamped to the \
         remaining distance, so min(2*h_before, h_max) is the bound that matters (both are printed \
         per row above)."
    );
    println!(
        "    a user-supplied max_step whose 0.1*h_max exceeds 10*sqrt(2*atol*tau*T/V0) = {:.4e} s \
         (= tau/{:.1}) breaks §17 here; the sweep above shows exactly that split.",
        10.0 * (2.0 * ATOL * TAU * TR / V0).sqrt(),
        TAU / (10.0 * (2.0 * ATOL * TAU * TR / V0).sqrt())
    );
    println!("  * this is intrinsic kernel behaviour: no product-path option changes it.");
    println!();

    finish(report)
}

fn finish(report: Report) -> i32 {
    let failed = report.failures();
    println!("=== verdict ===");
    println!(
        "contract checks: {} total, {} passed, {} failed",
        report.checks.len(),
        report.checks.len() - failed.len(),
        failed.len()
    );
    if report.not_met.is_empty() {
        println!("§17 NOT-MET rows: 0");
    } else {
        println!("§17 NOT-MET rows (kept verbatim; they do NOT flip the exit code):");
        for r in &report.not_met {
            println!("  [NOT-MET] {r}");
        }
    }
    if failed.is_empty() {
        println!("RESULT: ALL EXPERIMENTS RAN, ALL CONTRACT CHECKS MET (exit 0)");
        0
    } else {
        println!("RESULT: CONTRACT CHECKS FAILED (exit 1) - {failed:?}");
        1
    }
}

fn main() {
    let code = match std::panic::catch_unwind(run_all) {
        Ok(code) => code,
        Err(_) => {
            eprintln!("[FATAL] an experiment panicked; mapping the panic to exit 1");
            1
        }
    };
    std::process::exit(code);
}
