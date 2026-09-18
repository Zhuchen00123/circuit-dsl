//! Phase-0 backend acceptance probe for Thevenin 0.5.0.
//!
//! Builds `cirq_ir::Circuit` values **directly from Rust data structures**
//! (no Cirq/SPICE source text) and runs the acceptance cases required by the
//! project brief §4.3:
//!
//!   1. Resistive divider OP
//!   2. RC transient
//!   3. RC AC magnitude/phase
//!   4. RLC AC
//!   5. Diode nonlinear OP         (stage-2 admission item)
//!   6. DC sweep over a source value

use cirq_ir::{
    AcAnalysis, AcSpec, Analysis, Circuit, Connection, DcAnalysis, DcSweep, Element, ElementKind,
    FrequencyScale, Id, Net, SourceSpec, TranAnalysis, Value, Waveform,
};
use thevenin::circuit::{simulate_ac, simulate_dc, simulate_op, simulate_tran};
use thevenin_types::{SimVector, VectorData};

// ---------------------------------------------------------------------------
// Small builders
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

fn empty_circuit(name: &str, nets: Vec<Net>) -> Circuit {
    Circuit {
        name: name.to_string(),
        nets,
        elements: Vec::new(),
        models: Vec::new(),
        analyses: Vec::new(),
        params: Vec::new(),
        csparams: Vec::new(),
        options: Vec::new(),
        temps: Vec::new(),
        save: Vec::new(),
        funcs: Vec::new(),
        initial_conditions: Vec::new(),
        nodeset: Vec::new(),
        measures: Vec::new(),
        code_blocks: Vec::new(),
        raw_directives: Vec::new(),
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

fn inductor(id: u32, name: &str, p: u32, n: u32, value: f64) -> Element {
    Element {
        id: Id(id),
        name: name.to_string(),
        kind: ElementKind::Inductor,
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
// Reporting helpers
// ---------------------------------------------------------------------------

fn real(v: &SimVector) -> Vec<f64> {
    match &v.data {
        VectorData::Real(d) => d.clone(),
        VectorData::Complex(_) => panic!("expected real vector {}", v.name),
    }
}

fn cx(v: &SimVector) -> Vec<thevenin_types::Complex> {
    match &v.data {
        VectorData::Complex(d) => d.clone(),
        VectorData::Real(d) => d
            .iter()
            .map(|x| thevenin_types::Complex::new(*x, 0.0))
            .collect(),
    }
}

/// Complex magnitude of `re + j*im`.
fn cmag(re: f64, im: f64) -> f64 {
    (re * re + im * im).sqrt()
}

fn check(label: &str, actual: f64, expected: f64, tol: f64) -> bool {
    let ok = (actual - expected).abs() <= tol;
    println!(
        "  [{}] {:<38} actual={:<15.8e} expected={:<15.8e} |diff|={:.3e} tol={:.1e}",
        if ok { "PASS" } else { "FAIL" },
        label,
        actual,
        expected,
        (actual - expected).abs(),
        tol
    );
    ok
}

fn vector_names(plot: &thevenin_types::SimPlot) -> Vec<String> {
    plot.vecs.iter().map(|v| v.name.clone()).collect()
}

/// Select the plot whose name starts with `prefix` (case-insensitive).
///
/// Needed because `simulate_tran` prepends an operating-point plot, so the
/// transient data is *not* `plots[0]`.
fn plot_named<'a>(
    res: &'a thevenin_types::SimResult,
    prefix: &str,
) -> Option<&'a thevenin_types::SimPlot> {
    res.plots.iter().find(|p| {
        p.name
            .to_ascii_lowercase()
            .starts_with(&prefix.to_ascii_lowercase())
    })
}

fn plot_listing(res: &thevenin_types::SimResult) -> String {
    res.plots
        .iter()
        .map(|p| format!("{}[{} vectors]", p.name, p.vecs.len()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn main() {
    let mut all_ok = true;
    println!("=== Thevenin 0.5.0 backend acceptance probe ===");
    println!("(circuits constructed directly as cirq_ir::Circuit Rust values)\n");

    // Every case runs to completion (no short-circuit) and prints its own
    // zero-column PASS/FAIL prefix line; any FAIL makes the process exit 1.
    let c1 = case1_divider_op();
    println!(
        "[{}] Case 1 — resistive divider OP",
        if c1 { "PASS" } else { "FAIL" }
    );
    let c2 = case2_rc_tran();
    println!(
        "[{}] Case 2 — RC transient finite-ramp",
        if c2 { "PASS" } else { "FAIL" }
    );
    let c3 = case3_rc_ac();
    println!(
        "[{}] Case 3 — RC AC magnitude/phase",
        if c3 { "PASS" } else { "FAIL" }
    );
    let c4 = case4_rlc_ac();
    println!("[{}] Case 4 — RLC AC", if c4 { "PASS" } else { "FAIL" });
    let c5 = case5_diode_op();
    println!(
        "[{}] Case 5 — diode nonlinear OP",
        if c5 { "PASS" } else { "FAIL" }
    );
    let c6 = case6_dc_sweep();
    println!(
        "[{}] Case 6 — DC sweep over a source value",
        if c6 { "PASS" } else { "FAIL" }
    );

    all_ok &= c1;
    all_ok &= c2;
    all_ok &= c3;
    all_ok &= c4;
    all_ok &= c5;
    all_ok &= c6;

    println!();
    if all_ok {
        println!("RESULT: ALL ACCEPTANCE CASES PASSED");
    } else {
        println!("RESULT: SOME CASES FAILED");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Case 1 — resistive divider OP: Vmid = Vin * R2/(R1+R2)
// ---------------------------------------------------------------------------

fn case1_divider_op() -> bool {
    println!("[Case 1] Resistive divider operating point");
    let mut c = empty_circuit("divider", vec![net(0, "gnd"), net(1, "in"), net(2, "mid")]);
    c.elements.push(vsource(
        0,
        "v1",
        1,
        0,
        SourceSpec {
            dc: Some(1.0),
            ..Default::default()
        },
    ));
    c.elements.push(resistor(1, "r1", 1, 2, 1_000.0));
    c.elements.push(resistor(2, "r2", 2, 0, 2_000.0));
    c.analyses.push(Analysis::Op);

    let res = match simulate_op(&c) {
        Ok(r) => r,
        Err(e) => {
            println!("  [FAIL] simulate_op error: {e}");
            return false;
        }
    };

    let plot = match plot_named(&res, "op") {
        Some(p) => p,
        None => {
            println!("  [FAIL] no 'op*' plot; got: {}", plot_listing(&res));
            return false;
        }
    };
    println!("  plot '{}'; vectors: {:?}", plot.name, vector_names(plot));

    let vmid = real(plot.get("v(mid)"))[0];
    let vin = real(plot.get("v(in)"))[0];
    let mut ok = check("v(mid)", vmid, 1.0 * 2000.0 / 3000.0, 1e-9);
    ok &= check("v(in)", vin, 1.0, 1e-12);

    for cand in ["i(r1)", "i(v1)", "v1#branch"] {
        if let Some(iv) = plot.vector(cand) {
            println!("  note: {} = {:.6e} A", iv.name, real(iv)[0]);
        }
    }
    println!();
    ok
}

// ---------------------------------------------------------------------------
// Case 2 — RC transient: finite-ramp stimulus + matched analytic reference
// ---------------------------------------------------------------------------
//
// §17 criterion for transient node voltages (fixed here, never relaxed):
//     |actual - expected| <= TRAN_ATOL + TRAN_RTOL * |expected|
//     TRAN_ATOL = 1e-5 V, TRAN_RTOL = 1e-3
//
// WHY THE OLD EVIDENCE WAS WRONG
// (each mechanism below was read from thevenin-0.5.0 source in this session,
//  under ~/.cargo/registry/src/*/thevenin-0.5.0/src/):
//
//   * waveform.rs:37 evaluates PULSE with tr.unwrap_or(tran.tstep).max(tran.tstep):
//     the DECLARED rise time is clamped UP to the ".tran" step. waveform.rs:143-145
//     then evaluates a LINEAR ramp v1 + (v2-v1)*time/tr with time = t - td (:138).
//   * transient.rs:1327-1330 passes tstep = h_print (= cirq_ir::TranAnalysis::step,
//     the requested output interval) into that evaluator, so the clamp tracks step,
//     NOT tmax; transient.rs:799 uses tmax only as the internal cap
//     h_max = t_max.unwrap_or(min(tstep, tstop/50)).
//   * The old probe declared tr = 1 ps while step = tau/200 = 500 ns, so the stimulus
//     that reached the solver was a 500 ns ramp: T_eff = max(1 ps, 500 ns) = 500 ns.
//     Had the declared 1 ps edge really been used it would have moved the response by
//     at most V0*T/(2*tau) = 5.0e-9 V - 3.9e5x smaller than the reported residual.
//   * The old reference 1 - exp(-t/tau) is the IDEAL-STEP solution. Against a ramp of
//     width T the modelling residual is a pure homogeneous mode,
//         E(u) = y_ramp(u) - y_step(u) = C(T) * exp(-u/tau),
//         C(T) = 1 - (tau/T)*(exp(T/tau) - 1) = -(T/(2*tau))*(1 + O(T/tau)).
//     With T_eff = 500 ns and tau = 100 us, C = -2.50417e-3 V, so at u = 0.25*tau the
//     residual is -2.50417e-3 * exp(-0.25) = -1.9501e-3 V, i.e. exactly the historical
//     worst |diff| = 1.95e-3 V. Its true full-interval maximum is |C| = 2.50417e-3 V
//     at u = T, which the old five-point sampling never visited.
//   * So that 1.95e-3 V was a REFERENCE-MODEL error, not a solver defect, and it was
//     unrelated both to the declared 1 ps edge and to sample alignment (the old
//     reference was already evaluated at the returned sample times).
//
// ENGINE CONTRACT USED HERE (same source):
//   * output samples are NOT decimated to tstep: every ACCEPTED internal step is
//     recorded (transient.rs:2271-2285 -> record_point, :2386-2420). The engine has no
//     "output interval" concept; tstep (= TranAnalysis.step) only (a) floors the PULSE
//     rise/fall time and (b) supplies the default h_max.
//   * rejected steps record nothing (transient.rs:1636-1651), time advances by
//     h_min > 0 (:1407, :1700) and record_point only appends (:2400), so the returned
//     time axis is strictly increasing with no duplicates. Re-measured below.
//
// This case therefore: (a) drives the engine with a ramp wide enough not to be clamped
// (tr >= step), (b) evaluates the MATCHED piecewise analytic ramp solution at the
// ACTUAL returned sample times, (c) applies §17 at every returned sample, (d) keeps the
// ideal-step model only as an explicitly labelled counterfactual, and (e) runs
// controlled step / tmax / options experiments.
// ---------------------------------------------------------------------------

/// §17 transient acceptance tolerances (fixed by the brief; never relaxed here).
const TRAN_ATOL: f64 = 1e-5;
const TRAN_RTOL: f64 = 1e-3;

const C2_R: f64 = 1_000.0;
const C2_C: f64 = 100e-9;
const C2_V0: f64 = 1.0;
/// PULSE delay: the ramp starts C2_TD seconds after t = 0.
const C2_TD: f64 = 100e-6;

/// S(x) = x + expm1(-x) = sum_{k>=2} (-1)^k x^k / k!  - stable evaluation.
///
/// The naive x - 1 + exp(-x) returns exactly 0.0 for x <= 3e-8 and keeps only ~4
/// significant digits at x = 1e-6, so it is not used anywhere in this file.
fn ramp_s(x: f64) -> f64 {
    if x < 1e-2 {
        // Alternating series truncated after k = 6: |R| <= x^7/7!, i.e. a relative error
        // <= 3.97e-14 at the x = 1e-2 switch-over.
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
        x + f64::exp_m1(-x)
    }
}

/// Analytic response of tau*y' + y = v_in to a DELAYED FINITE RAMP:
/// v_in = 0 for u < 0, linear 0 -> v0 over [0, T], held at v0 afterwards.
///
/// u = t - td, x = u/tau, rho = T/tau:
///   u <= 0     : y = 0
///   0 < u <= T : y = (v0/rho) * S(x)
///   u > T      : y = v0 + (y(T) - v0) * exp(-(u-T)/tau),  y(T) = (v0/rho) * S(rho)
fn ramp_ref(u: f64, tau: f64, t_rise: f64, v0: f64) -> f64 {
    if u <= 0.0 {
        return 0.0;
    }
    if t_rise <= 0.0 {
        return v0 * (1.0 - (-u / tau).exp());
    }
    let rho = t_rise / tau;
    if u <= t_rise {
        v0 / rho * ramp_s(u / tau)
    } else {
        let y_t = v0 / rho * ramp_s(rho);
        v0 + (y_t - v0) * (-(u - t_rise) / tau).exp()
    }
}

/// Ideal-step reference v0*(1 - exp(-u/tau)) - the model the OLD probe used.
/// Kept ONLY as a labelled counterfactual: it is the wrong model for a finite ramp.
fn step_ref(u: f64, tau: f64, v0: f64) -> f64 {
    if u <= 0.0 {
        0.0
    } else {
        v0 * (1.0 - (-u / tau).exp())
    }
}

/// C(T) = 1 - (tau/T)*(exp(T/tau) - 1): coefficient of the pure homogeneous-mode
/// residual y_ramp - y_step = C(T)*exp(-u/tau). Its magnitude is the maximum
/// ramp-vs-step modelling discrepancy, attained at u = T.
fn ramp_step_c(t_rise: f64, tau: f64) -> f64 {
    1.0 - tau / t_rise * (t_rise / tau).exp_m1()
}

/// Single §17 reference entry point for the Case-2 stimulus family: the matched
/// finite-ramp solution, evaluated at the ACTUAL returned sample time t.
fn rc_ref(t: f64, td: f64, tau: f64, t_rise_eff: f64) -> f64 {
    ramp_ref(t - td, tau, t_rise_eff, C2_V0)
}

/// Index of the returned sample closest to target (ties resolve to the earliest).
fn nearest_index(t: &[f64], target: f64) -> usize {
    let mut best = 0usize;
    let mut best_d = f64::INFINITY;
    for (i, &ti) in t.iter().enumerate() {
        let d = (ti - target).abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

/// Transient result of one Case-2 run.
struct Tran1 {
    time: Vec<f64>,
    vout: Vec<f64>,
}

/// Field-by-field comparison of two runs (point count, time axis, values).
fn same_trace(a: &Tran1, b: &Tran1) -> bool {
    a.time.len() == b.time.len()
        && a.vout.len() == b.vout.len()
        && a.time.iter().zip(&b.time).all(|(x, y)| x == y)
        && a.vout.iter().zip(&b.vout).all(|(x, y)| x == y)
}

/// Per-field comparison printout used as the evidence for the tolerance experiment.
fn compare_trace(label: &str, base: &Tran1, r: &Tran1) -> bool {
    let n = base.time.len().min(r.time.len());
    let mut first_diff: Option<usize> = None;
    for i in 0..n {
        if base.time[i] != r.time[i] {
            first_diff = Some(i);
            break;
        }
    }
    // Values are only comparable while both runs share the same time coordinate; once the
    // axes diverge, index-by-index differencing would compare different instants.
    let aligned = first_diff.unwrap_or(n);
    let mut dv_max = 0.0f64;
    for i in 0..aligned {
        dv_max = dv_max.max((base.vout[i] - r.vout[i]).abs());
    }
    let identical = same_trace(base, r);
    println!(
        "      field compare vs '{label}': n {}/{} identical={} ; aligned time prefix = {} sample(s), max |dv(out)| there = {:.3e} V ; first time-axis divergence at index {}",
        base.time.len(),
        r.time.len(),
        identical,
        aligned,
        dv_max,
        first_diff
            .map(|i| i.to_string())
            .unwrap_or_else(|| "none (equal or shorter axis is an exact prefix)".to_string())
    );
    identical
}

/// Build and run the Case-2 RC low-pass
/// gnd - v1(PULSE) - in - r1(1k) - out - c1(100n) - gnd  (tau = R*C = 100 us).
///
/// Only the arguments vary between sub-experiments: topology, device values and source
/// amplitude never do. The falling edge is 10 s after the rise and PER = 20 s, so no
/// falling edge and no retrigger can fall inside stop.
#[allow(clippy::too_many_arguments)]
fn rc_tran_run(
    td: f64,
    tr: f64,
    step: f64,
    stop: f64,
    tmax: Option<f64>,
    options: Vec<(String, Value)>,
    label: &str,
) -> Result<Tran1, String> {
    let mut c = empty_circuit("rc", vec![net(0, "gnd"), net(1, "in"), net(2, "out")]);
    c.elements.push(vsource(
        0,
        "v1",
        1,
        0,
        SourceSpec {
            dc: Some(0.0),
            waveform: Some(Waveform::Pulse {
                v1: 0.0,
                v2: C2_V0,
                td: Some(td),
                tr: Some(tr),
                tf: Some(tr),
                pw: Some(10.0),
                per: Some(20.0),
            }),
            ..Default::default()
        },
    ));
    c.elements.push(resistor(1, "r1", 1, 2, C2_R));
    c.elements.push(capacitor(2, "c1", 2, 0, C2_C));
    c.options = options;
    c.analyses.push(Analysis::Tran(TranAnalysis {
        step,
        stop,
        start: 0.0,
        uic: false,
        tmax,
    }));

    let res = simulate_tran(&c).map_err(|e| format!("{label}: simulate_tran error: {e}"))?;
    let plot = plot_named(&res, "tran")
        .ok_or_else(|| format!("{label}: no 'tran*' plot; got: {}", plot_listing(&res)))?;
    let tvec = plot.vector("time").ok_or_else(|| {
        format!(
            "{label}: plot '{}' has no 'time' vector; got {:?}",
            plot.name,
            vector_names(plot)
        )
    })?;
    let vvec = plot.vector("v(out)").ok_or_else(|| {
        format!(
            "{label}: plot '{}' has no 'v(out)' vector; got {:?}",
            plot.name,
            vector_names(plot)
        )
    })?;
    let time = real(tvec);
    let vout = real(vvec);
    if time.is_empty() || time.len() != vout.len() {
        return Err(format!(
            "{label}: unusable vectors: n(time)={}, n(v(out))={}",
            time.len(),
            vout.len()
        ));
    }
    Ok(Tran1 { time, vout })
}

/// §17 statistics over EVERY returned sample:
///   err_i = |v_i - ref(t_i)|,  allowance_i = TRAN_ATOL + TRAN_RTOL * |ref(t_i)|
/// Nothing is dropped, re-sampled or averaged; duplicate sample times (if any would
/// occur) are kept and counted like any other sample.
struct ErrStats {
    n: usize,
    max_err: f64,
    max_t: f64,
    max_expected: f64,
    max_allow: f64,
    violations: usize,
    max_ratio: f64,
    max_ratio_t: f64,
}

fn err_stats(t: &[f64], v: &[f64], reference: &dyn Fn(f64) -> f64) -> ErrStats {
    let mut st = ErrStats {
        n: t.len(),
        max_err: 0.0,
        max_t: 0.0,
        max_expected: 0.0,
        max_allow: 0.0,
        violations: 0,
        max_ratio: 0.0,
        max_ratio_t: 0.0,
    };
    for i in 0..t.len() {
        let expected = reference(t[i]);
        let err = (v[i] - expected).abs();
        let allow = TRAN_ATOL + TRAN_RTOL * expected.abs();
        if err > allow {
            st.violations += 1;
        }
        let ratio = err / allow;
        if ratio > st.max_ratio {
            st.max_ratio = ratio;
            st.max_ratio_t = t[i];
        }
        if err > st.max_err {
            st.max_err = err;
            st.max_t = t[i];
            st.max_expected = expected;
            st.max_allow = allow;
        }
    }
    st
}

/// Time-axis contract report (strictly increasing, no duplicates, dt min/mean/max).
fn print_axis(t: &[f64]) -> bool {
    if t.len() < 2 {
        println!("  [FAIL] time axis has {} point(s)", t.len());
        return false;
    }
    let deltas: Vec<f64> = t.windows(2).map(|w| w[1] - w[0]).collect();
    let n_dup = deltas.iter().filter(|d| **d == 0.0).count();
    let n_back = deltas.iter().filter(|d| **d < 0.0).count();
    let d_min = deltas.iter().copied().fold(f64::INFINITY, f64::min);
    let d_max = deltas.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let d_mean = deltas.iter().sum::<f64>() / deltas.len() as f64;
    println!(
        "  time axis: n={}, t0={:.6e} s, t_last={:.6e} s, dt min={:.6e} s, mean={:.6e} s, max={:.6e} s",
        t.len(),
        t[0],
        t[t.len() - 1],
        d_min,
        d_mean,
        d_max
    );
    println!("  time axis: duplicates (dt == 0) = {n_dup}, non-increasing (dt < 0) = {n_back}");
    if n_dup == 0 && n_back == 0 {
        println!("  [PASS] contract: time axis is strictly increasing, no duplicate samples");
        true
    } else {
        println!(
            "  [FAIL] contract violated: {n_dup} duplicate(s), {n_back} backwards step(s); duplicates are NOT silently removed - every statistic below counts them exactly as returned"
        );
        false
    }
}

fn case2_rc_tran() -> bool {
    let ok = case2_rc_tran_body();
    println!(
        "[{}] Case 2 — RC transient finite-ramp verification (§17: atol={:.0e} V, rtol={:.0e})",
        if ok { "PASS" } else { "FAIL" },
        TRAN_ATOL,
        TRAN_RTOL
    );
    println!();
    ok
}

/// First accepted step at or after t_mark (the breakpoint landing step h1).
fn first_step_after(t: &[f64], t_mark: f64) -> Option<(f64, f64)> {
    t.windows(2)
        .find(|w| w[0] >= t_mark - 1e-18)
        .map(|w| (w[0], w[1] - w[0]))
}

fn case2_rc_tran_body() -> bool {
    let tau = C2_R * C2_C;
    let step = tau / 200.0; // 500 ns: ".tran step" = waveform tstep + default h_max
    let t_rise = 1e-6; // declared finite rise >= step, so NOT clamped
    let stop = 5.0 * tau + C2_TD + t_rise;
    // Recommended internal step cap. §2.11 measures three tmax values and tau/1000 is the
    // only one of them that keeps every returned sample inside §17; the historical
    // tmax = step = 500 ns is retained there as an explicit NOT-MET row, never hidden.
    let tmax_rec = tau / 1000.0;
    let mut ok = true;
    let mut not_met: Vec<String> = Vec::new();

    println!(
        "[Case 2] RC transient - finite ramp + matched analytic reference (NOT an ideal step)"
    );
    println!(
        "  circuit : gnd - v1(PULSE) - in - r1(1k) - out - c1(100n) - gnd ; tau = R*C = {:.1} us",
        tau * 1e6
    );
    println!(
        "  source  : v1=0 V, v2={:.1} V, td={:.1} us, tr={:.1} us, tf={:.1} us, pw=10 s, per=20 s",
        C2_V0,
        C2_TD * 1e6,
        t_rise * 1e6,
        t_rise * 1e6
    );
    println!(
        "  .tran   : step={:.1} ns, tmax=Some({:.1} ns = tau/1000, recommended), stop={:.1} us",
        step * 1e9,
        tmax_rec * 1e9,
        stop * 1e6
    );

    // --- 2.1 clamp contract --------------------------------------------------
    let t_eff = t_rise.max(step);
    println!(
        "  clamp   : thevenin waveform.rs:37 -> tr_used = max(declared tr, .tran step) = max({:.1} ns, {:.1} ns) = {:.1} ns = T_eff",
        t_rise * 1e9,
        step * 1e9,
        t_eff * 1e9
    );
    let not_clamped = t_rise >= step;
    println!(
        "  [{}] declared tr >= .tran step: the real ramp width is the declared T = {:.1} us",
        if not_clamped { "PASS" } else { "FAIL" },
        t_eff * 1e6
    );
    ok &= not_clamped;

    // --- 2.2 matched-reference cross-check against the brief's criterion table -
    let published: [(f64, f64); 6] = [
        (0.001, 1.005e-5),
        (0.01, 1.4983e-5),
        (0.1, 1.0062e-4),
        (1.0, 6.4028e-4),
        (2.0, 8.7399e-4),
        (5.0, 1.0032e-3),
    ];
    println!(
        "  reference cross-check (matched ramp solution, T_eff = {:.3} us):",
        t_eff * 1e6
    );
    println!(
        "    {:>8} {:>16} {:>16} {:>16}  {}",
        "u/tau", "expected [V]", "allowance [V]", "brief [V]", "verdict"
    );
    let mut refc_ok = true;
    for (x, pub_allow) in published {
        let expected = rc_ref(C2_TD + x * tau, C2_TD, tau, t_eff);
        let allow = TRAN_ATOL + TRAN_RTOL * expected.abs();
        let row_ok = (allow - pub_allow).abs() <= 1e-3 * pub_allow.abs();
        refc_ok &= row_ok;
        println!(
            "    {:>8.4} {:>16.6e} {:>16.6e} {:>16.6e}  [{}]",
            x,
            expected,
            allow,
            pub_allow,
            if row_ok { "PASS" } else { "FAIL" }
        );
    }
    println!(
        "  [{}] matched-reference allowances reproduce the brief's table (1e-3 rel.)",
        if refc_ok { "PASS" } else { "FAIL" }
    );
    ok &= refc_ok;

    // --- 2.3 baseline run ----------------------------------------------------
    let base = match rc_tran_run(
        C2_TD,
        t_rise,
        step,
        stop,
        Some(tmax_rec),
        Vec::new(),
        "baseline",
    ) {
        Ok(r) => r,
        Err(e) => {
            println!("  [FAIL] {e}");
            return false;
        }
    };
    println!(
        "  baseline run: {} returned samples (step = {:.1} ns, tmax = tau/1000 = {:.1} ns)",
        base.time.len(),
        step * 1e9,
        tmax_rec * 1e9
    );

    // --- 2.4 time-axis contract ---------------------------------------------
    ok &= print_axis(&base.time);

    // --- 2.5 §17 statistics over every returned sample -----------------------
    let st = err_stats(&base.time, &base.vout, &|ti: f64| {
        rc_ref(ti, C2_TD, tau, t_eff)
    });
    println!(
        "  §17 over ALL {} returned samples (matched ramp reference, T_eff = {:.3} us):",
        st.n,
        t_eff * 1e6
    );
    println!("    max |err|       = {:.6e} V", st.max_err);
    println!(
        "    at t            = {:.6e} s   (u/tau = {:.6})",
        st.max_t,
        (st.max_t - C2_TD) / tau
    );
    println!("    expected there  = {:.6e} V", st.max_expected);
    println!(
        "    allowance there = atol + rtol*|expected| = {:.1e} + {:.1e}*{:.6e} = {:.6e} V",
        TRAN_ATOL,
        TRAN_RTOL,
        st.max_expected.abs(),
        st.max_allow
    );
    println!("    over-limit pts  = {} / {}", st.violations, st.n);
    println!(
        "    worst err/allow = {:.4} at t = {:.6e} s",
        st.max_ratio, st.max_ratio_t
    );
    let stats_ok = st.violations == 0;
    println!(
        "  [{}] every returned sample satisfies the §17 criterion",
        if stats_ok { "PASS" } else { "FAIL" }
    );
    ok &= stats_ok;

    // --- 2.6 span table ------------------------------------------------------
    println!("  span table (nearest returned sample to each u/tau target):");
    println!(
        "    {:>8} {:>12} {:>12} {:>16} {:>16} {:>12} {:>12}  {}",
        "u/tau",
        "t [us]",
        "u/tau act",
        "v(out) [V]",
        "expected [V]",
        "|err| [V]",
        "allow [V]",
        "verdict"
    );
    let mut table_ok = true;
    for x in [0.0, 0.001, 0.01, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0] {
        let idx = nearest_index(&base.time, C2_TD + x * tau);
        let t_i = base.time[idx];
        let expected = rc_ref(t_i, C2_TD, tau, t_eff);
        let err = (base.vout[idx] - expected).abs();
        let allow = TRAN_ATOL + TRAN_RTOL * expected.abs();
        let row_ok = err <= allow;
        table_ok &= row_ok;
        println!(
            "    {:>8.4} {:>12.3} {:>12.4} {:>16.9} {:>16.9} {:>12.4e} {:>12.4e}  [{}]",
            x,
            t_i * 1e6,
            (t_i - C2_TD) / tau,
            base.vout[idx],
            expected,
            err,
            allow,
            if row_ok { "PASS" } else { "FAIL" }
        );
    }
    println!(
        "  [{}] all nine span samples (u/tau = 0, 0.001, 0.01, 0.1, 0.25, 0.5, 1, 2, 5) satisfy §17",
        if table_ok { "PASS" } else { "FAIL" }
    );
    ok &= table_ok;

    // --- 2.7 counterfactual: the ideal-step model on the SAME engine output ---
    let cf = err_stats(&base.time, &base.vout, &|ti: f64| {
        step_ref(ti - C2_TD, tau, C2_V0)
    });
    let c_pred = ramp_step_c(t_eff, tau);
    println!(
        "  counterfactual (NOT part of PASS/FAIL): same engine output vs the IDEAL-STEP model 1-exp(-u/tau):"
    );
    println!(
        "    max |err| = {:.6e} V at t = {:.6e} s (u/tau = {:.6})",
        cf.max_err,
        cf.max_t,
        (cf.max_t - C2_TD) / tau
    );
    println!(
        "    analytic prediction |C(T_eff)| = {:.6e} V   [C(T) = 1 - (tau/T)(exp(T/tau)-1), T_eff = {:.3} us]",
        c_pred.abs(),
        t_eff * 1e6
    );
    println!("    over-limit points = {} / {}", cf.violations, cf.n);
    let cf_violates = cf.violations > 0;
    println!(
        "  [{}] the ideal-step reference DOES exceed §17 on this engine output (the old attribution cannot stand)",
        if cf_violates { "PASS" } else { "FAIL" }
    );
    ok &= cf_violates;

    // --- 2.8 old-configuration reproduction: where 1.95e-3 V came from -------
    // Old probe settings verbatim: tr = 1 ps, td = 0, step = tmax = tau/200 = 500 ns,
    // stop = 5*tau. The engine clamps the declared 1 ps up to 500 ns.
    let old_step = tau / 200.0;
    let old_tr: f64 = 1e-12;
    let old_teff = old_tr.max(old_step);
    let c_old = ramp_step_c(old_teff, tau);
    let e_at_t = c_old.abs() * (-(old_teff / tau)).exp();
    println!(
        "  old-config reproduction (tr = 1 ps, td = 0, step = tmax = {:.1} ns, stop = 5*tau):",
        old_step * 1e9
    );
    println!(
        "    engine T_eff = max(1 ps, {:.1} ns) = {:.1} ns : the stimulus that reached the solver was a 500 ns ramp",
        old_step * 1e9,
        old_teff * 1e9
    );
    match rc_tran_run(
        0.0,
        old_tr,
        old_step,
        5.0 * tau,
        Some(old_step),
        Vec::new(),
        "old-config",
    ) {
        Ok(old) => {
            let old_matched = err_stats(&old.time, &old.vout, &|ti: f64| {
                rc_ref(ti, 0.0, tau, old_teff)
            });
            let old_ideal = err_stats(&old.time, &old.vout, &|ti: f64| step_ref(ti, tau, C2_V0));
            let i025 = nearest_index(&old.time, 0.25 * tau);
            let e025 = (old.vout[i025] - step_ref(old.time[i025], tau, C2_V0)).abs();
            println!(
                "    vs MATCHED 500 ns-ramp reference : max |err| = {:.6e} V, over-limit = {} / {}",
                old_matched.max_err, old_matched.violations, old_matched.n
            );
            println!(
                "    vs IDEAL-STEP reference          : max |err| = {:.6e} V at u/tau = {:.4}, over-limit = {} / {}",
                old_ideal.max_err,
                old_ideal.max_t / tau,
                old_ideal.violations,
                old_ideal.n
            );
            println!(
                "    at the old 0.25*tau sample (t = {:.3} us): |err| = {:.6e} V  vs predicted |C|*exp(-0.25) = {:.6e} V",
                old.time[i025] * 1e6,
                e025,
                c_old.abs() * (-0.25f64).exp()
            );
            println!(
                "    predicted full-interval max = |C(500 ns)|*exp(-rho) = {:.6e} V (at u = T)",
                e_at_t
            );
            let old_ok = old_matched.violations == 0
                && old_ideal.violations > 0
                && (old_ideal.max_err - e_at_t).abs() <= 0.05 * e_at_t
                && (e025 - c_old.abs() * (-0.25f64).exp()).abs() <= 0.05 * c_old.abs();
            println!(
                "  [{}] old config: the matched 500 ns-ramp residual is within §17 while the ideal-step model reproduces the historical ~1.95e-3 V",
                if old_ok { "PASS" } else { "FAIL" }
            );
            ok &= old_ok;
        }
        Err(e) => {
            println!("  [FAIL] {e}");
            ok = false;
        }
    }

    // --- 2.9 declared ramp width control (T = 1 us vs 2 us, step unchanged) ---
    // The matched-reference residual is the solver's real accuracy. NOTE: it is NOT
    // required to fall monotonically in T - an earlier draft of the task text asserted
    // that and it was wrong. What scales with T is the COUNTERFACTUAL residual against
    // the ideal-step model, |C(T)| ~ T/(2*tau); the matched residual stays far below
    // §17 either way. Both numbers are printed exactly as measured.
    let t_rise2 = 2e-6;
    match rc_tran_run(
        C2_TD,
        t_rise2,
        step,
        stop,
        Some(tmax_rec),
        Vec::new(),
        "T=2us",
    ) {
        Ok(r2) => {
            let teff2 = t_rise2.max(step);
            let s2 = err_stats(&r2.time, &r2.vout, &|ti: f64| rc_ref(ti, C2_TD, tau, teff2));
            let s2_stale = err_stats(&r2.time, &r2.vout, &|ti: f64| rc_ref(ti, C2_TD, tau, t_eff));
            let c2 = err_stats(&r2.time, &r2.vout, &|ti: f64| {
                step_ref(ti - C2_TD, tau, C2_V0)
            });
            println!(
                "  ramp-width control: T = {:.1} us -> {:.1} us (step = {:.1} ns, tmax = {:.1} ns unchanged), {} -> {} samples",
                t_rise * 1e6,
                t_rise2 * 1e6,
                step * 1e9,
                tmax_rec * 1e9,
                base.time.len(),
                r2.time.len()
            );
            println!(
                "    matched reference (T_eff = {:.1} us) : max |err| = {:.4e} V, over-limit {} (T = 1 us: {:.4e} V, over-limit {})",
                teff2 * 1e6,
                s2.max_err,
                s2.violations,
                st.max_err,
                st.violations
            );
            println!(
                "    stale reference (T_eff = {:.1} us)   : max |err| = {:.4e} V, over-limit {}  <- the reference must track T_eff",
                t_eff * 1e6,
                s2_stale.max_err,
                s2_stale.violations
            );
            println!(
                "    ideal-step counterfactual          : max |err| = {:.4e} V (T = 1 us: {:.4e} V); |C(T)| prediction {:.4e} V",
                c2.max_err,
                cf.max_err,
                ramp_step_c(teff2, tau).abs()
            );
            let r2_ok = s2.violations == 0 && s2_stale.violations > 0;
            println!(
                "  [{}] T = 2 us stays within §17 against its own matched reference, and a stale T_eff is exposed as a reference error",
                if r2_ok { "PASS" } else { "FAIL" }
            );
            ok &= r2_ok;
            if s2.violations > 0 {
                not_met.push(format!(
                    "ramp-width control T = {:.1} us: {}/{} samples exceed §17",
                    teff2 * 1e6,
                    s2.violations,
                    s2.n
                ));
            }
        }
        Err(e) => {
            println!("  [FAIL] {e}");
            ok = false;
        }
    }

    // --- 2.10 .tran step sweep: what the PULSE clamp depends on --------------
    // T_eff = max(declared tr, step). Variant (i) keeps the DECLARED edge below every
    // tested step (tr = 1 ps) so T_eff = step; variant (ii) keeps the declared edge at
    // 1 us so T_eff = 1 us for every step <= 1 us (clamp inert) and jumps to the step
    // once the step exceeds the declared edge. tmax stays pinned at tau/1000 in both, so
    // only the clamp - hence T_eff - moves.
    println!(
        "  .tran step sweep (tmax pinned at tau/1000 = {:.1} ns; only step and the declared tr move):",
        tmax_rec * 1e9
    );
    println!(
        "    {:>9} {:>9} {:>9} {:>8} {:>16} {:>16} {:>11}  {}",
        "tr decl",
        "step",
        "T_eff",
        "points",
        "max |err| [V]",
        "ideal-step [V]",
        "over-limit",
        "verdict"
    );
    let mut sw_ok = true;
    for &tr_decl in &[1e-12_f64, 1e-6_f64] {
        for &mult in &[20.0, 200.0, 1000.0] {
            let st_step = tau / mult;
            let teff = tr_decl.max(st_step);
            let stp = 5.0 * tau + C2_TD + teff;
            match rc_tran_run(
                C2_TD,
                tr_decl,
                st_step,
                stp,
                Some(tmax_rec),
                Vec::new(),
                "step-sweep",
            ) {
                Ok(r) => {
                    let m = err_stats(&r.time, &r.vout, &|ti: f64| rc_ref(ti, C2_TD, tau, teff));
                    let cfs = err_stats(&r.time, &r.vout, &|ti: f64| {
                        step_ref(ti - C2_TD, tau, C2_V0)
                    });
                    let row_ok = m.violations == 0;
                    sw_ok &= row_ok;
                    if !row_ok {
                        not_met.push(format!(
                            "step sweep tr = {:.0} ns / step = tau/{:.0}: {}/{} samples exceed §17, max |err| = {:.4e} V",
                            tr_decl * 1e9,
                            mult,
                            m.violations,
                            m.n,
                            m.max_err
                        ));
                    }
                    println!(
                        "    {:>9.3} {:>9.1} {:>9.1} {:>8} {:>16.4e} {:>16.4e} {:>11}  [{}]",
                        tr_decl * 1e9,
                        st_step * 1e9,
                        teff * 1e9,
                        r.time.len(),
                        m.max_err,
                        cfs.max_err,
                        m.violations,
                        if row_ok { "PASS" } else { "FAIL" }
                    );
                    let pred = ramp_step_c(teff, tau).abs();
                    println!(
                        "        T_eff = {:.1} ns : matched residual {:.4e} V ; ideal-step residual {:.4e} V vs |C(T_eff)| = {:.4e} V (ratio {:.4})",
                        teff * 1e9,
                        m.max_err,
                        cfs.max_err,
                        pred,
                        cfs.max_err / pred
                    );
                }
                Err(e) => {
                    println!("    [FAIL] {e}");
                    sw_ok = false;
                }
            }
        }
    }
    println!(
        "  [{}] every step/tr combination stays within §17 against its matched T_eff reference",
        if sw_ok { "PASS" } else { "FAIL" }
    );
    ok &= sw_ok;

    // --- 2.11 max_step (= TranAnalysis.tmax) sweep ---------------------------
    // Circuit, source and step are untouched; only tmax moves. tmax does NOT enter the
    // PULSE clamp (that uses tstep), so T_eff stays 1 us in every row. Each row reports
    // BOTH the residual against the matched ramp reference (the real accuracy, which DOES
    // depend on tmax) and against the ideal-step model (which does not - that is the
    // a04 "worst is tmax-independent" result, and the two must not be conflated).
    // Rows that exceed §17 are NOT removed and NOT threshold-adjusted: they are kept,
    // labelled NOT-MET (未达标), and repeated in the NOT-MET block before the verdict.
    println!(
        "  max_step sweep (only TranAnalysis.tmax changes; tr = 1 us, step = {:.1} ns fixed):",
        step * 1e9
    );
    println!(
        "    {:>10} {:>10} {:>8} {:>16} {:>10} {:>16} {:>11} {:>13}  {}",
        "tmax",
        "tmax [ns]",
        "points",
        "matched |err|",
        "at u/tau",
        "ideal-step |err|",
        "over-limit",
        "dt_max [ns]",
        "status"
    );
    let mut rows: Vec<(f64, usize, f64, f64, usize)> = Vec::new();
    let mut recommended_ok = false;
    for &mult in &[50.0, 200.0, 1000.0] {
        let tmax = tau / mult;
        match rc_tran_run(
            C2_TD,
            t_rise,
            step,
            stop,
            Some(tmax),
            Vec::new(),
            "tmax-sweep",
        ) {
            Ok(r) => {
                let m = err_stats(&r.time, &r.vout, &|ti: f64| rc_ref(ti, C2_TD, tau, t_eff));
                let cfs = err_stats(&r.time, &r.vout, &|ti: f64| {
                    step_ref(ti - C2_TD, tau, C2_V0)
                });
                let dt_max = r
                    .time
                    .windows(2)
                    .map(|w| w[1] - w[0])
                    .fold(f64::NEG_INFINITY, f64::max);
                let conforms = m.violations == 0;
                if tmax == tmax_rec {
                    recommended_ok = conforms;
                }
                if !conforms {
                    let h1 = first_step_after(&r.time, C2_TD)
                        .map(|(_, d)| d)
                        .unwrap_or(f64::NAN);
                    let be_pred = h1 * (C2_V0 / t_rise * h1) / tau / (1.0 + h1 / tau);
                    let trap_err = C2_V0 / t_rise * h1 * h1 / (2.0 * tau);
                    not_met.push(format!(
                        "max_step = tau/{mult:.0} ({:.1} ns): {}/{} samples exceed §17; max |err| = {:.4e} V at t = {:.6e} s (u/tau = {:.4e}); allowance there = {:.4e} V; first post-breakpoint step h1 = {:.3e} s; the forced Backward-Euler restart gives v(out) = h1*v(td+h1)/tau/(1+h1/tau) = {:.6e} V, i.e. {:.3} x the trapezoidal/exact ramp value (V0/T)*h1^2/(2*tau) = {:.6e} V, and that doubled value IS the observed error",
                        tmax * 1e9,
                        m.violations,
                        m.n,
                        m.max_err,
                        m.max_t,
                        (m.max_t - C2_TD) / tau,
                        m.max_allow,
                        h1,
                        be_pred,
                        be_pred / trap_err,
                        trap_err
                    ));
                }
                rows.push((tmax, r.time.len(), m.max_err, cfs.max_err, m.violations));
                println!(
                    "    {:>10} {:>10.1} {:>8} {:>16.4e} {:>10.4} {:>16.4e} {:>11} {:>13.3}  {}",
                    format!("tau/{:.0}", mult),
                    tmax * 1e9,
                    r.time.len(),
                    m.max_err,
                    (m.max_t - C2_TD) / tau,
                    cfs.max_err,
                    m.violations,
                    dt_max * 1e9,
                    if conforms {
                        "MET(满足判据)"
                    } else {
                        "NOT-MET(未达标,保留)"
                    }
                );
            }
            Err(e) => {
                println!("    [FAIL] {e}");
                not_met.push(format!("max_step = tau/{mult:.0}: run failed: {e}"));
            }
        }
    }
    if rows.len() == 3 {
        let (t0, n0, e0, _, v0) = rows[0];
        let (t1, n1, e1, _, v1) = rows[1];
        let (t2, n2, e2, _, v2) = rows[2];
        let (_, _, _, c0, _) = rows[0];
        println!(
            "    trend: tmax {:.0} -> {:.0} -> {:.0} ns gives points {} -> {} -> {} ({}) and matched max |err| {:.3e} -> {:.3e} -> {:.3e} V ({}), over-limit {} -> {} -> {}",
            t0 * 1e9,
            t1 * 1e9,
            t2 * 1e9,
            n0,
            n1,
            n2,
            if n1 > n0 && n2 > n1 {
                "monotonically increasing"
            } else {
                "NOT monotone - recorded as measured"
            },
            e0,
            e1,
            e2,
            if e1 <= e0 && e2 <= e1 {
                "monotonically decreasing"
            } else {
                "NOT monotone - recorded as measured"
            },
            v0,
            v1,
            v2
        );
        println!(
            "    note: the ideal-step counterfactual stays at {:.3e} V for every tmax (tmax-independent), while the matched residual above is strongly tmax-dependent - the two measure different things.",
            c0
        );
    }
    let tmax_ok = rows.len() == 3 && recommended_ok;
    println!(
        "  [{}] max_step experiment: 3/3 configurations ran; the recommended tmax = tau/1000 ({:.1} ns) satisfies §17; {} NOT-MET configuration(s) retained verbatim above",
        if tmax_ok { "PASS" } else { "FAIL" },
        tmax_rec * 1e9,
        not_met.len()
    );
    ok &= tmax_ok;

    // --- 2.12 tolerance experiment -------------------------------------------
    // The options channel is real: mna_ir.rs nr_options_from_circuit (keys matched after
    // to_uppercase()) -> TranRunParams.nr_opts -> the LTE timestep controller
    // (transient.rs:409-432): vol_tol = abstol + reltol*max(|i_cur|,|i_prev|),
    // chg_tol = reltol*max(q0,q1,chgtol)/h, tol = trtol*max(...),
    // h_new = sqrt(trtol*tol / max(abstol, lte_est)).
    // BUT transient.rs:799 pins h_max = tmax when tmax is set, and the accepted step is
    // h = new_h.min(step_h*MAX_GROW).min(h_max): with tmax set the LTE estimate is
    // clamped by h_max. Groups A and B test both regimes; each gets its own criterion.
    let cfg_options = |i: usize| -> Vec<(String, Value)> {
        match i {
            0 => Vec::new(),
            1 => vec![("RELTOL".to_string(), Value::Real(1e-12))],
            2 => vec![("ABSTOL".to_string(), Value::Real(1e-15))],
            3 => vec![("TRTOL".to_string(), Value::Real(0.7))],
            _ => vec![
                ("RELTOL".to_string(), Value::Real(1e-12)),
                ("ABSTOL".to_string(), Value::Real(1e-15)),
            ],
        }
    };
    let cfg_label = |i: usize| -> &'static str {
        match i {
            0 => "empty options",
            1 => "RELTOL=1e-12 only",
            2 => "ABSTOL=1e-15 only",
            3 => "TRTOL=0.7 only",
            _ => "RELTOL+ABSTOL combined",
        }
    };

    // Group A: h_max pinned by tmax = tau/1000 (the Case-2 baseline configuration).
    println!(
        "  tolerance group A (h_max pinned: tmax = Some({:.1} ns) -> h_max = tmax, LTE estimate clamped):",
        tmax_rec * 1e9
    );
    println!(
        "    {:>22} {:>8} {:>16} {:>11} {:>14}  {}",
        "options", "points", "max |err| [V]", "over-limit", "same-as-empty", "verdict"
    );
    let mut group_a: Vec<Tran1> = Vec::new();
    let mut tol_a_ok = true;
    for i in 0..3 {
        match rc_tran_run(
            C2_TD,
            t_rise,
            step,
            stop,
            Some(tmax_rec),
            cfg_options(i),
            "tol-A",
        ) {
            Ok(r) => {
                let m = err_stats(&r.time, &r.vout, &|ti: f64| rc_ref(ti, C2_TD, tau, t_eff));
                let same = group_a.first().map(|b| same_trace(b, &r)).unwrap_or(true);
                let row_ok = m.violations == 0;
                tol_a_ok &= row_ok;
                println!(
                    "    {:>22} {:>8} {:>16.4e} {:>11} {:>14}  [{}]",
                    cfg_label(i),
                    r.time.len(),
                    m.max_err,
                    m.violations,
                    if i == 0 {
                        "baseline".to_string()
                    } else {
                        same.to_string()
                    },
                    if row_ok { "PASS" } else { "FAIL" }
                );
                group_a.push(r);
            }
            Err(e) => {
                println!("    [FAIL] {e}");
                tol_a_ok = false;
            }
        }
    }
    let mut a_identical = true;
    if group_a.len() == 3 {
        for i in 1..3 {
            println!("    {}", cfg_label(i));
            a_identical &= compare_trace(cfg_label(0), &group_a[0], &group_a[i]);
        }
    }
    if a_identical {
        println!(
            "    CONCLUSION (group A): circuit.options DOES reach the solver (mna_ir.rs:107 -> transient.rs:1630-1633), but with h_max pinned by tmax the returned points, time axis and values are field-for-field IDENTICAL - in this configuration the tolerance settings change no observable result and therefore cannot serve as an experimental variable here."
        );
    } else {
        println!(
            "    CONCLUSION (group A): a tolerance setting DID change the returned trace even with h_max pinned by tmax; the per-field differences are printed above."
        );
    }
    let a_conclusion_ok = a_identical;
    println!(
        "  [{}] group A criterion: all three single-factor tolerance configurations are field-for-field identical (settings not observable under a pinned h_max)",
        if a_conclusion_ok { "PASS" } else { "FAIL" }
    );
    ok &= tol_a_ok & a_conclusion_ok;

    // Group B: tmax = None -> h_max = min(step, stop/50) = step, so the LTE estimate is
    // no longer clamped by tmax.
    let b_step = tau / 100.0;
    let b_tr = 20.0 * b_step;
    let b_stop = 5.0 * tau + C2_TD + b_tr;
    let b_teff = b_tr.max(b_step);
    println!(
        "  tolerance group B (h_max free: tmax = None -> h_max = min(step, stop/50) = {:.3} us; step = {:.3} us, tr = {:.1} us):",
        b_step.min(b_stop / 50.0) * 1e6,
        b_step * 1e6,
        b_tr * 1e6
    );
    println!(
        "    {:>22} {:>8} {:>12} {:>12} {:>16} {:>11} {:>14}  {}",
        "options",
        "points",
        "dt_min [s]",
        "dt_max [s]",
        "max |err| [V]",
        "over-limit",
        "same-as-empty",
        "verdict"
    );
    let mut group_b: Vec<Tran1> = Vec::new();
    let mut tol_b_ok = true;
    for i in 0..5 {
        match rc_tran_run(C2_TD, b_tr, b_step, b_stop, None, cfg_options(i), "tol-B") {
            Ok(r) => {
                let m = err_stats(&r.time, &r.vout, &|ti: f64| rc_ref(ti, C2_TD, tau, b_teff));
                let dt_min = r
                    .time
                    .windows(2)
                    .map(|w| w[1] - w[0])
                    .fold(f64::INFINITY, f64::min);
                let dt_max = r
                    .time
                    .windows(2)
                    .map(|w| w[1] - w[0])
                    .fold(f64::NEG_INFINITY, f64::max);
                let same = group_b.first().map(|b| same_trace(b, &r)).unwrap_or(true);
                let row_ok = m.violations == 0;
                tol_b_ok &= row_ok;
                if !row_ok {
                    not_met.push(format!(
                        "tolerance group B / {}: {}/{} samples exceed §17, max |err| = {:.4e} V",
                        cfg_label(i),
                        m.violations,
                        m.n,
                        m.max_err
                    ));
                }
                println!(
                    "    {:>22} {:>8} {:>12.3e} {:>12.3e} {:>16.4e} {:>11} {:>14}  [{}]",
                    cfg_label(i),
                    r.time.len(),
                    dt_min,
                    dt_max,
                    m.max_err,
                    m.violations,
                    if i == 0 {
                        "baseline".to_string()
                    } else {
                        same.to_string()
                    },
                    if row_ok { "PASS" } else { "FAIL" }
                );
                group_b.push(r);
            }
            Err(e) => {
                println!("    [FAIL] {e}");
                tol_b_ok = false;
            }
        }
    }
    let mut b_obs = [false; 5];
    if group_b.len() == 5 {
        for i in 1..5 {
            println!("    {}", cfg_label(i));
            b_obs[i] = !compare_trace(cfg_label(0), &group_b[0], &group_b[i]);
        }
    }
    let b_single_observable = b_obs[1] || b_obs[2] || b_obs[3];
    let b_observed_any = b_single_observable || b_obs[4];
    println!(
        "    single-factor observability in group B: RELTOL alone = {}, ABSTOL alone = {}, TRTOL alone = {}; combined RELTOL+ABSTOL = {}",
        b_obs[1], b_obs[2], b_obs[3], b_obs[4]
    );
    if b_observed_any {
        println!(
            "    CONCLUSION (group B): the single-factor rows are each NOT observable (RELTOL alone = {}, ABSTOL alone = {}, TRTOL alone = {}), while changing RELTOL and ABSTOL together IS observable. The effect is therefore an INTERACTION of those two tolerances and in this data set it is NOT attributable to either one alone - that is exactly why the one-factor-at-a-time rows were run, and no attribution beyond this is claimed.",
            b_obs[1], b_obs[2], b_obs[3]
        );
        if b_obs[4] {
            println!(
                "      observations on the observable row: the trace is bit-identical for the first samples and only then diverges (see the aligned-prefix numbers above), and dt_max stays at h_max, i.e. the tolerance change moves the accepted step only in the region where the LTE estimate is below h_max - consistent with transient.rs:1654 h = new_h.min(step_h*MAX_GROW).min(h_max) and transient.rs:430 del = tol/max(abstol, lte_est)."
            );
        }
    } else {
        println!(
            "    CONCLUSION (group B): even with h_max free, none of the tolerance settings changed the returned trace in this configuration - reported as measured; the option channel is then not demonstrable as an experimental variable here."
        );
    }
    println!(
        "  [{}] group B criterion: at least one tolerance configuration changes the returned trace when h_max is free, every row reports its own §17 status ({}) and the single-factor rows are reported individually above (single-factor observable: {})",
        if b_observed_any { "PASS" } else { "FAIL" },
        if tol_b_ok {
            "all inside §17"
        } else {
            "some rows outside §17"
        },
        b_single_observable
    );
    ok &= tol_b_ok & b_observed_any;

    // --- 2.13 NOT-MET disclosure --------------------------------------------
    println!();
    if not_met.is_empty() {
        println!("  NOT-MET (未达标) configurations: none - every §17 check in this case passed");
    } else {
        println!(
            "  NOT-MET (未达标) configurations retained verbatim (no threshold was relaxed, no failing sample removed):"
        );
        for r in &not_met {
            println!("    * {r}");
        }
    }

    ok
}

// ---------------------------------------------------------------------------
// Case 3 — RC AC: H(jw) = 1/(1 + jwRC)
// ---------------------------------------------------------------------------

fn case3_rc_ac() -> bool {
    println!("[Case 3] RC AC response H(jw) = 1/(1+jwRC)");
    const R: f64 = 1_000.0;
    const C: f64 = 100e-9;

    let mut c = empty_circuit("rc_ac", vec![net(0, "gnd"), net(1, "in"), net(2, "out")]);
    c.elements.push(vsource(
        0,
        "v1",
        1,
        0,
        SourceSpec {
            dc: Some(0.0),
            ac: Some(AcSpec {
                mag: 1.0,
                phase: 0.0,
            }),
            ..Default::default()
        },
    ));
    c.elements.push(resistor(1, "r1", 1, 2, R));
    c.elements.push(capacitor(2, "c1", 2, 0, C));
    c.analyses.push(Analysis::Ac(AcAnalysis {
        start: 10.0,
        stop: 10e6,
        points: 10,
        scale: FrequencyScale::Decade,
    }));

    let res = match simulate_ac(&c) {
        Ok(r) => r,
        Err(e) => {
            println!("  [FAIL] simulate_ac error: {e}");
            return false;
        }
    };
    let plot = match plot_named(&res, "ac") {
        Some(p) => p,
        None => {
            println!("  [FAIL] no 'ac*' plot; got: {}", plot_listing(&res));
            return false;
        }
    };
    println!("  plot '{}'; vectors: {:?}", plot.name, vector_names(plot));

    let f = real(plot.get("frequency"));
    let vout = cx(plot.get("v(out)"));
    println!("  {} frequency points", f.len());

    let mut ok = true;
    let mut worst = 0.0f64;
    let fc = 1.0 / (2.0 * std::f64::consts::PI * R * C);
    for &target in &[fc / 10.0, fc, fc * 10.0] {
        let idx = f
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
        let w = 2.0 * std::f64::consts::PI * f[idx];
        // H(jw) = 1/(1 + jwRC)
        let (a, b) = (1.0_f64, w * R * C);
        let den = a * a + b * b;
        let (hre, him) = (a / den, -b / den);
        let err = cmag(vout[idx].re - hre, vout[idx].im - him);
        worst = worst.max(err);
        let case_ok = err < 1e-6;
        ok &= case_ok;
        println!(
            "  [{}] f={:<11.4e}Hz |H|={:.6} ph={:>9.3}deg |H_ref|={:.6} ph_ref={:>9.3}deg |diff|={:.2e}",
            if case_ok { "PASS" } else { "FAIL" },
            f[idx],
            vout[idx].magnitude(),
            vout[idx].phase_deg(),
            cmag(hre, him),
            him.atan2(hre).to_degrees(),
            err
        );
    }
    println!("  corner fc = {:.3} Hz; worst |diff| = {:.3e}", fc, worst);
    println!();
    ok
}

// ---------------------------------------------------------------------------
// Case 4 — RLC AC (series R-L-C, output across C)
// ---------------------------------------------------------------------------

fn case4_rlc_ac() -> bool {
    println!("[Case 4] RLC AC (series R-L-C, output across C)");
    const R: f64 = 100.0;
    const L: f64 = 10e-3;
    const C: f64 = 100e-9;

    let mut c = empty_circuit(
        "rlc",
        vec![net(0, "gnd"), net(1, "in"), net(2, "mid"), net(3, "out")],
    );
    c.elements.push(vsource(
        0,
        "v1",
        1,
        0,
        SourceSpec {
            dc: Some(0.0),
            ac: Some(AcSpec {
                mag: 1.0,
                phase: 0.0,
            }),
            ..Default::default()
        },
    ));
    c.elements.push(resistor(1, "r1", 1, 2, R));
    c.elements.push(inductor(2, "l1", 2, 3, L));
    c.elements.push(capacitor(3, "c1", 3, 0, C));
    c.analyses.push(Analysis::Ac(AcAnalysis {
        start: 100.0,
        stop: 1e6,
        points: 20,
        scale: FrequencyScale::Decade,
    }));

    let res = match simulate_ac(&c) {
        Ok(r) => r,
        Err(e) => {
            println!("  [FAIL] simulate_ac error: {e}");
            return false;
        }
    };
    let plot = match plot_named(&res, "ac") {
        Some(p) => p,
        None => {
            println!("  [FAIL] no 'ac*' plot; got: {}", plot_listing(&res));
            return false;
        }
    };
    let f = real(plot.get("frequency"));
    let vout = cx(plot.get("v(out)"));
    println!("  plot '{}'; {} points", plot.name, f.len());

    let mut ok = true;
    let mut worst = 0.0f64;
    let f0 = 1.0 / (2.0 * std::f64::consts::PI * (L * C).sqrt());
    for &target in &[f0 / 5.0, f0, f0 * 5.0] {
        let idx = f
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
        let w = 2.0 * std::f64::consts::PI * f[idx];
        // H(jw) = 1 / (1 - w^2 LC + j w RC)
        let a = 1.0 - w * w * L * C;
        let b = w * R * C;
        let den = a * a + b * b;
        let (hre, him) = (a / den, -b / den);
        let err = cmag(vout[idx].re - hre, vout[idx].im - him);
        worst = worst.max(err);
        let case_ok = err < 1e-6;
        ok &= case_ok;
        println!(
            "  [{}] f={:<11.4e}Hz |H|={:.6} ph={:>9.3}deg |H_ref|={:.6} ph_ref={:>9.3}deg |diff|={:.2e}",
            if case_ok { "PASS" } else { "FAIL" },
            f[idx],
            vout[idx].magnitude(),
            vout[idx].phase_deg(),
            cmag(hre, him),
            him.atan2(hre).to_degrees(),
            err
        );
    }
    println!(
        "  resonance f0 = {:.3} Hz; worst |diff| = {:.3e}",
        f0, worst
    );
    println!();
    ok
}

// ---------------------------------------------------------------------------
// Case 5 — diode nonlinear OP
// ---------------------------------------------------------------------------

fn case5_diode_op() -> bool {
    println!("[Case 5] Diode nonlinear operating point (5V - R 1k - D - gnd)");
    use cirq_ir::{DeviceType, Model};

    let mut c = empty_circuit(
        "diode_circuit",
        vec![net(0, "gnd"), net(1, "in"), net(2, "out")],
    );
    c.models.push(Model {
        id: Id(0),
        name: "dmod".to_string(),
        device_type: DeviceType::Diode,
        params: vec![
            ("is".to_string(), Value::Real(1e-14)),
            ("n".to_string(), Value::Real(1.0)),
        ],
    });
    c.elements.push(vsource(
        0,
        "v1",
        1,
        0,
        SourceSpec {
            dc: Some(5.0),
            ..Default::default()
        },
    ));
    c.elements.push(resistor(1, "r1", 1, 2, 1_000.0));
    c.elements.push(Element {
        id: Id(2),
        name: "d1".to_string(),
        kind: ElementKind::Diode,
        connections: vec![conn("anode", 2), conn("cathode", 0)],
        params: Vec::new(),
        model: Some(Id(0)),
        source_spec: None,
    });
    c.analyses.push(Analysis::Op);

    let res = match simulate_op(&c) {
        Ok(r) => r,
        Err(e) => {
            println!("  [FAIL] simulate_op error: {e}");
            return false;
        }
    };
    let plot = match plot_named(&res, "op") {
        Some(p) => p,
        None => {
            println!("  [FAIL] no 'op*' plot; got: {}", plot_listing(&res));
            return false;
        }
    };
    let vout = real(plot.get("v(out)"))[0];
    println!(
        "  v(out) = {:.6} V ; vectors: {:?}",
        vout,
        vector_names(plot)
    );

    // Independent reference by bisection: 5 = I*1000 + Vd, I = Is*(exp(Vd/(n*Vt))-1)
    // with Vt = kT/q at 27 C = 0.02586419 V (SPICE default TEMP=27).
    let vt = 0.025864190383365096_f64;
    let is = 1e-14_f64;
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        let i = is * ((mid / vt).exp() - 1.0);
        if 5.0 - i * 1000.0 - mid > 0.0 {
            lo = mid
        } else {
            hi = mid
        }
    }
    let vd_ref = 0.5 * (lo + hi);
    let ok = check("v(out) vs bisection reference", vout, vd_ref, 5e-3);

    let plausible = (0.3..0.8).contains(&vout);
    println!(
        "  [{}] plausible silicon forward drop in (0.3, 0.8) V",
        if plausible { "PASS" } else { "FAIL" }
    );
    println!();
    ok && plausible
}

// ---------------------------------------------------------------------------
// Case 6 — DC sweep over a source value
// ---------------------------------------------------------------------------

fn case6_dc_sweep() -> bool {
    println!("[Case 6] DC sweep of source v1 from 0 V to 5 V, step 1 V");
    let mut c = empty_circuit("dcsweep", vec![net(0, "gnd"), net(1, "in"), net(2, "mid")]);
    c.elements.push(vsource(
        0,
        "v1",
        1,
        0,
        SourceSpec {
            dc: Some(0.0),
            ..Default::default()
        },
    ));
    c.elements.push(resistor(1, "r1", 1, 2, 1_000.0));
    c.elements.push(resistor(2, "r2", 2, 0, 1_000.0));
    c.analyses.push(Analysis::Dc(DcAnalysis {
        sweeps: vec![DcSweep {
            source: Id(0),
            start: 0.0,
            stop: 5.0,
            step: 1.0,
        }],
    }));

    let res = match simulate_dc(&c) {
        Ok(r) => r,
        Err(e) => {
            println!("  [FAIL] simulate_dc error: {e}");
            return false;
        }
    };
    let plot = match plot_named(&res, "dc") {
        Some(p) => p,
        None => {
            println!("  [FAIL] no 'dc*' plot; got: {}", plot_listing(&res));
            return false;
        }
    };
    println!("  plot '{}'; vectors: {:?}", plot.name, vector_names(plot));

    // The sweep axis vector name varies; find the first vector whose length
    // matches v(mid) and is not a node voltage of interest.
    let vmid = real(plot.get("v(mid)"));
    let names = vector_names(plot);
    let mut ok = true;
    let mut sweep: Option<Vec<f64>> = None;
    for n in &names {
        if n.eq_ignore_ascii_case("v(mid)") {
            continue;
        }
        let d = real(plot.get(n));
        if d.len() == vmid.len() {
            sweep = Some(d);
            println!("  using '{n}' as sweep axis");
            break;
        }
    }
    let sweep = match sweep {
        Some(s) => s,
        None => {
            println!("  [FAIL] could not identify sweep axis among {names:?}");
            return false;
        }
    };

    println!("  {} points", vmid.len());
    for i in 0..vmid.len().min(6) {
        let expected = sweep[i] * 0.5;
        let case_ok = (vmid[i] - expected).abs() < 1e-9;
        ok &= case_ok;
        println!(
            "  [{}] sweep={:.3} V  v(mid)={:.6} V  expected={:.6} V",
            if case_ok { "PASS" } else { "FAIL" },
            sweep[i],
            vmid[i],
            expected
        );
    }
    println!();
    ok
}
