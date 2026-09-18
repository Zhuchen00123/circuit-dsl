//! Bottom-layer TRAN contract pins for Thevenin 0.5.0, kept in the standalone
//! `_probe` project (NOT in the product workspace).
//!
//! # What this bin pins
//!
//! `thevenin-0.5.0/src/waveform.rs:37-38` evaluates a PULSE as
//!
//! ```text
//! tr_used = tr.unwrap_or(tran.tstep).max(tran.tstep)
//! tf_used = tf.unwrap_or(tran.tstep).max(tran.tstep)
//! ```
//!
//! where `tran.tstep` is `cirq_ir::TranAnalysis::step` (`h_print`) and
//! **not** `TranAnalysis::tmax`. So a declared rise/fall shorter than the
//! `.tran` step is silently widened to that step, and `tmax` cannot change it.
//! The same clamped value is used for the breakpoint table
//! (`waveform.rs:271-278`), so the widened edge is what the solver integrates.
//!
//! This bin measures the **actual edge width** from the returned time axis and
//! the saved `v(in)` vector, i.e. from what the engine really produced, and
//! compares it against the formula above:
//!
//! | case | `.tran step` | declared tr | predicted `tr_used` | why |
//! |---|---|---|---|---|
//! | A  | 100 ns | 10 ns  | **100 ns** (widened) | `max(10 ns, 100 ns)` |
//! | A2 | 100 ns | 10 ns  | **100 ns** (widened), `tmax = 1 ns` | `tmax` is not in the clamp |
//! | B  | 1 ns   | 10 ns  | **10 ns** (declared) | `max(10 ns, 1 ns) = 10 ns` |
//! | C  | 10 ns  | 1 us   | **1 us** (declared) | declared edge above the step |
//! | D  | 500 ns | 1 ps   | **500 ns** (widened) | the historical 1.95e-3 V configuration |
//!
//! Case D carries the assertion that used to live in
//! `crates/circuit-backend/tests/transient_reference_regression.rs::declared_rise_below_output_interval_is_clamped_to_the_output_step`.
//! That product test file is owned by another agent and is NOT modified here
//! (nor imported): the engine-level fact is pinned in this independent project
//! instead. Note the product adapter no longer passes `output_interval` to the
//! engine (`circuit_backend::thevenin::print_step_for` picks
//! `h_print = min(span/1000, min(rise, fall, period))`), which makes the clamp
//! in case D unreachable from the product path for a declared edge — this bin
//! documents the engine rule itself, not the current adapter mapping.
//!
//! # How the edge width is measured (not assumed)
//!
//! The engine records every accepted internal step
//! (`transient.rs:2271-2285`), so the returned `time` axis is the integration
//! grid. A PULSE edge is exactly linear in `t` in the engine's evaluator
//! (`waveform.rs:143-151`), so a least-squares line through the samples
//! **strictly inside** the edge recovers the width without assuming anything
//! about the grid: `width = |v_hi - v_lo| / |slope|`, and the two level
//! crossings of that line give the reconstructed edge window.
//!
//! # Exit code
//!
//! * `0` — every case ran and every contract check passed.
//! * `1` — at least one contract check failed, or an experiment panicked
//!   (`catch_unwind` maps a panic to 1 rather than the default 101).
//! `NOT-MET` rows (if any) are printed with an explicit label and are listed
//! again before the verdict; the policy is stated in the report
//! (`docs/review-evidence/round2/breakpoint-evidence.md`).

use cirq_ir::{
    Analysis, Circuit, Connection, Element, ElementKind, Id, Net, SourceSpec, TranAnalysis,
    Waveform,
};
use thevenin::circuit::simulate_tran;
use thevenin_types::{SimResult, VectorData};

// ---------------------------------------------------------------------------
// Fixed physical setup: RC low-pass, tau = R*C = 100 us
// ---------------------------------------------------------------------------

const R_OHM: f64 = 1_000.0;
const C_FARAD: f64 = 100e-9;
/// `tau = R*C = 100 us`.
const TAU: f64 = R_OHM * C_FARAD;
const V0: f64 = 1.0;

// ---------------------------------------------------------------------------
// cirq_ir builders (same shapes as _probe/src/main.rs; that file is not
// modified by this one)
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
        params: vec![("value".to_string(), cirq_ir::Value::Real(value))],
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
        params: vec![("value".to_string(), cirq_ir::Value::Real(value))],
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
// One run
// ---------------------------------------------------------------------------

/// A PULSE configuration for one run. `step` is `TranAnalysis::step`
/// (`h_print`), `tmax` is `TranAnalysis::tmax`, `None` means "engine default"
/// (`h_max = min(tstep, tstop/50)`, `transient.rs:799`).
#[derive(Clone, Copy)]
struct PulseCase {
    label: &'static str,
    /// `.tran step` handed to the engine (`h_print`).
    step: f64,
    /// `tran.tmax`, or `None`.
    tmax: Option<f64>,
    /// Declared PULSE rise/fall (`tr`/`tf`).
    declared_tr: f64,
    declared_tf: f64,
    /// PULSE delay (`td`).
    td: f64,
    /// PULSE high time (`pw`).
    pw: f64,
    /// `tran stop`.
    stop: f64,
}

/// What the engine returned.
struct Trace {
    time: Vec<f64>,
    vin: Vec<f64>,
    vout: Vec<f64>,
}

impl Trace {
    fn point_count(&self) -> usize {
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

fn run_pulse(case: &PulseCase) -> Result<Trace, String> {
    let mut c = Circuit {
        name: format!("rc_pulse_{}", case.label),
        nets: vec![net(0, "gnd"), net(1, "in"), net(2, "out")],
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
                td: Some(case.td),
                tr: Some(case.declared_tr),
                tf: Some(case.declared_tf),
                pw: Some(case.pw),
                // A single pulse inside the window: the retrigger must not
                // interfere with the edge measurement.
                per: None,
            }),
            ..Default::default()
        },
    ));
    c.elements.push(resistor(1, "r1", 1, 2, R_OHM));
    c.elements.push(capacitor(2, "c1", 2, 0, C_FARAD));
    c.analyses.push(Analysis::Tran(TranAnalysis {
        step: case.step,
        stop: case.stop,
        start: 0.0,
        uic: false,
        tmax: case.tmax,
    }));

    let res = simulate_tran(&c).map_err(|e| format!("{}: simulate_tran error: {e}", case.label))?;
    let plot = plot_named(&res, "tran")
        .ok_or_else(|| format!("{}: no 'tran*' plot in the result", case.label))?;
    let tvec = plot
        .vector("time")
        .ok_or_else(|| format!("{}: no 'time' vector", case.label))?;
    let vinvec = plot
        .vector("v(in)")
        .ok_or_else(|| format!("{}: no 'v(in)' vector", case.label))?;
    let voutvec = plot
        .vector("v(out)")
        .ok_or_else(|| format!("{}: no 'v(out)' vector", case.label))?;
    let time = real(tvec);
    let vin = real(vinvec);
    let vout = real(voutvec);
    if time.len() != vin.len() || time.len() != vout.len() {
        return Err(format!(
            "{}: vector length mismatch: n(time)={}, n(v(in))={}, n(v(out))={}",
            case.label,
            time.len(),
            vin.len(),
            vout.len()
        ));
    }
    if time.len() < 3 {
        return Err(format!("{}: only {} points", case.label, time.len()));
    }
    Ok(Trace { time, vin, vout })
}

// ---------------------------------------------------------------------------
// Edge measurement: least-squares line through the samples strictly inside
// one PULSE edge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct EdgeFit {
    /// Samples strictly inside the edge that were fitted.
    n: usize,
    /// Slope in V/s (sign carries the direction).
    slope: f64,
    /// `|v_hi - v_lo| / |slope|`: the reconstructed edge width, in seconds.
    width_s: f64,
    /// Time the fitted line reaches `v_lo` (the edge start).
    t_lo_cross: f64,
    /// Time the fitted line reaches `v_hi` (the edge end).
    t_hi_cross: f64,
    /// RMS residual of the linear fit, in volts.
    rms_residual: f64,
    /// First fitted sample time.
    t_first_s: f64,
    /// Last fitted sample time.
    t_last_s: f64,
}

/// First maximal contiguous run of samples satisfying `inside`, starting the
/// search at index `from`.
fn first_run<F: Fn(f64) -> bool>(v: &[f64], from: usize, inside: F) -> Option<(usize, usize)> {
    let mut i = from;
    while i < v.len() && !inside(v[i]) {
        i += 1;
    }
    if i >= v.len() {
        return None;
    }
    let start = i;
    while i + 1 < v.len() && inside(v[i + 1]) {
        i += 1;
    }
    Some((start, i))
}

/// Fit one edge of `v` (the PULSE input) between levels `v_lo` and `v_hi`.
///
/// The fitted points are the **first contiguous run** of samples strictly
/// between the two levels (margin `1e-9 * |v_hi - v_lo|`), searched from index
/// `from`: plateau samples sit exactly on a level in `eval_pulse`
/// (`waveform.rs:143-155` returns `v1`/`v2` verbatim) and are excluded, and the
/// run stops at the plateau/other edge so the rising and falling edges are
/// never mixed in one fit. `rising` selects which edge is being measured.
fn fit_edge(
    t: &[f64],
    v: &[f64],
    v_lo: f64,
    v_hi: f64,
    rising: bool,
    from: usize,
) -> Option<EdgeFit> {
    let span = (v_hi - v_lo).abs();
    let margin = 1e-9 * span;
    let lo = v_lo.min(v_hi);
    let hi = v_lo.max(v_hi);
    let (start, end) = first_run(v, from, |x| x > lo + margin && x < hi - margin)?;
    let idx: Vec<usize> = (start..=end).collect();
    if idx.len() < 2 {
        return None;
    }
    // Least squares v = a*t + b.
    let n = idx.len() as f64;
    let sum_t: f64 = idx.iter().map(|&i| t[i]).sum();
    let sum_v: f64 = idx.iter().map(|&i| v[i]).sum();
    let mean_t = sum_t / n;
    let mean_v = sum_v / n;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    for &i in &idx {
        let dt = t[i] - mean_t;
        sxx += dt * dt;
        sxy += dt * (v[i] - mean_v);
    }
    if !(sxx > 0.0) {
        return None;
    }
    let slope = sxy / sxx;
    if !(slope.is_finite()) || slope == 0.0 {
        return None;
    }
    let intercept = mean_v - slope * mean_t;
    let mut sumsq = 0.0;
    for &i in &idx {
        let r = v[i] - (slope * t[i] + intercept);
        sumsq += r * r;
    }
    let width_s = span / slope.abs();
    if (slope > 0.0) != rising {
        // The run is not the edge we asked for (e.g. the search started on the
        // wrong side of the trace); refuse rather than report a bogus width.
        return None;
    }
    Some(EdgeFit {
        n: idx.len(),
        slope,
        width_s,
        t_lo_cross: (v_lo - intercept) / slope,
        t_hi_cross: (v_hi - intercept) / slope,
        rms_residual: (sumsq / n).sqrt(),
        t_first_s: t[idx[0]],
        t_last_s: t[*idx.last().unwrap()],
    })
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

struct Report {
    cases: Vec<(&'static str, bool)>,
}

impl Report {
    fn new() -> Self {
        Report { cases: Vec::new() }
    }

    fn case(&mut self, name: &'static str, ok: bool) -> bool {
        println!("  [{}] {name}", if ok { "PASS" } else { "FAIL" });
        self.cases.push((name, ok));
        ok
    }

    fn failures(&self) -> Vec<&'static str> {
        self.cases
            .iter()
            .filter(|(_, ok)| !ok)
            .map(|(name, _)| *name)
            .collect()
    }
}

/// Compare a measured value against the predicted one with a relative
/// tolerance, printing both.
fn check_rel(
    report: &mut Report,
    label: &'static str,
    actual: f64,
    expected: f64,
    rel_tol: f64,
) -> bool {
    let denom = expected.abs().max(f64::MIN_POSITIVE);
    let rel = (actual - expected).abs() / denom;
    let ok = rel <= rel_tol;
    println!(
        "    [{}] {label}: measured={actual:.12e} predicted={expected:.12e} rel.diff={rel:.3e} tol={rel_tol:.1e}",
        if ok { "PASS" } else { "FAIL" }
    );
    report.case(label, ok)
}

/// Index of the first sample that reached the high plateau (`v_hi`), i.e. the
/// point from which the falling-edge search starts.
fn plateau_start(v: &[f64], v_hi: f64) -> usize {
    v.iter()
        .position(|x| (x - v_hi).abs() <= 1e-9 * v_hi.abs().max(1.0))
        .unwrap_or(0)
}

/// Measure both edges of one case and check them against the engine's
/// `t_used = max(declared, .tran step)` rule.
///
/// Returns `(rise_width, fall_width)` when both fits produced a number.
fn edge_case(
    report: &mut Report,
    names: (&'static str, &'static str),
    case: &PulseCase,
    trace: &Trace,
) -> (Option<f64>, Option<f64>) {
    let (rise_name, fall_name) = names;
    println!(
        "    grid: points={}, dt_min={:.3e} s, dt_max={:.3e} s",
        trace.point_count(),
        trace.dt_min(),
        trace.dt_max()
    );
    let rise = fit_edge(&trace.time, &trace.vin, 0.0, V0, true, 0);
    let fall_from = plateau_start(&trace.vin, V0);
    let fall = fit_edge(&trace.time, &trace.vin, V0, 0.0, false, fall_from);

    let report_edge = |name: &'static str,
                       fit: Option<EdgeFit>,
                       declared: f64,
                       pred_start: f64,
                       report: &mut Report| {
        match fit {
            Some(f) => {
                println!(
                    "    {name}: samples inside the edge = {} (t {:.6e} .. {:.6e} s)",
                    f.n, f.t_first_s, f.t_last_s
                );
                println!(
                    "    {name}: fitted slope = {:.9e} V/s, rms linear residual = {:.3e} V",
                    f.slope, f.rms_residual
                );
                println!(
                    "    {name}: reconstructed width = {:.9e} s, start = {:.9e} s \
                     (predicted {:.9e} s), end = {:.9e} s",
                    f.width_s, f.t_lo_cross, pred_start, f.t_hi_cross
                );
                check_rel(report, name, f.width_s, declared.max(case.step), 1e-6);
            }
            None => {
                println!(
                    "    [FAIL] {name}: fewer than two samples strictly inside the edge - the \
                     returned grid cannot resolve it (points={}, dt_min={:.3e} s)",
                    trace.point_count(),
                    trace.dt_min()
                );
                report.case(name, false);
            }
        }
    };

    report_edge(rise_name, rise, case.declared_tr, case.td, report);
    let fall_start = case.td + case.declared_tr.max(case.step) + case.pw;
    let fall_end = fall_start + case.declared_tf.max(case.step);
    if fall_end > case.stop {
        println!(
            "    {fall_name}: SKIPPED, not a failure - the falling edge ends at {fall_end:.3e} s, \
             outside the simulated window (stop = {:.3e} s); this case measures the rising edge only",
            case.stop
        );
        return (rise.map(|f| f.width_s), None);
    }
    report_edge(fall_name, fall, case.declared_tf, fall_start, report);
    (rise.map(|f| f.width_s), fall.map(|f| f.width_s))
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

fn run_all() -> i32 {
    println!("=== Thevenin 0.5.0 TRAN bottom-layer contract pins (`tran_contract`) ===");
    println!(
        "circuit: gnd - v1(PULSE) - in - r1(1k) - out - c1(100n) - gnd ; tau = {:.1} us",
        TAU * 1e6
    );
    println!("measured quantity: the width of the PULSE edge as it reached the solver,");
    println!("reconstructed from the returned time axis and v(in) (least-squares fit of the");
    println!("samples strictly inside the edge). Predicted by thevenin waveform.rs:37-38:");
    println!("    tr_used = tr.unwrap_or(tstep).max(tstep)   [tstep = TranAnalysis::step]");
    println!();

    let mut report = Report::new();

    // --- Case A: step = 100 ns, declared tr = 10 ns -> widened to 100 ns -----
    let case_a = PulseCase {
        label: "A",
        step: 100e-9,
        tmax: None,
        declared_tr: 10e-9,
        declared_tf: 10e-9,
        td: 1e-6,
        pw: 500e-9,
        stop: 2e-6,
    };
    println!(
        "[Case A] step = {:.0} ns, declared tr = tf = {:.0} ns, tmax = None",
        case_a.step * 1e9,
        case_a.declared_tr * 1e9
    );
    println!(
        "    engine default step bound (transient.rs:799): h_max = min(tstep, tstop/50) = {:.3e} s",
        case_a.step.min(case_a.stop / 50.0)
    );
    let mut a_points = 0usize;
    let mut a_dt_max = f64::NAN;
    let mut a_width = None;
    match run_pulse(&case_a) {
        Ok(tr) => {
            a_points = tr.point_count();
            a_dt_max = tr.dt_max();
            let (rise, _fall) = edge_case(
                &mut report,
                ("A/rise_widened_to_step", "A/fall_widened_to_step"),
                &case_a,
                &tr,
            );
            a_width = rise;
            // Plateau level check: the widened edge still reaches exactly V0.
            let plateau = tr
                .vin
                .iter()
                .copied()
                .filter(|v| *v > 0.5)
                .fold(f64::NEG_INFINITY, f64::max);
            let ok = (plateau - V0).abs() <= 1e-12;
            println!(
                "    A/plateau: max v(in) in the window = {plateau:.15} V (V0 = {V0}), |diff| = {:.3e} V",
                (plateau - V0).abs()
            );
            report.case("A/plateau_reaches_V0", ok);
        }
        Err(e) => {
            println!("    [FAIL] case A run failed: {e}");
            report.case("A/rise_widened_to_step", false);
            report.case("A/fall_widened_to_step", false);
        }
    }
    println!();

    // --- Case A2: same, but tmax = 1 ns -> clamp must not move --------------
    let case_a2 = PulseCase {
        tmax: Some(1e-9),
        label: "A2",
        ..case_a
    };
    println!(
        "[Case A2] step = {:.0} ns, declared tr = tf = {:.0} ns, tmax = Some({:.0} ns)",
        case_a2.step * 1e9,
        case_a2.declared_tr * 1e9,
        case_a2.tmax.unwrap() * 1e9
    );
    println!("    tmax is NOT an argument of the clamp: the widened edge must be identical.");
    let mut a2_width = None;
    match run_pulse(&case_a2) {
        Ok(tr) => {
            let (rise, _fall) = edge_case(
                &mut report,
                ("A2/rise_widened_to_step", "A2/fall_widened_to_step"),
                &case_a2,
                &tr,
            );
            a2_width = rise;
            println!(
                "    A vs A2 grid: points {} vs {}, dt_max {:.3e} s vs {:.3e} s \
                 (tmax changed the grid, not the edge)",
                a_points,
                tr.point_count(),
                a_dt_max,
                tr.dt_max()
            );
        }
        Err(e) => {
            println!("    [FAIL] case A2 run failed: {e}");
            report.case("A2/rise_widened_to_step", false);
            report.case("A2/fall_widened_to_step", false);
        }
    }
    match (a_width, a2_width) {
        (Some(wa), Some(wb)) => {
            let rel = (wa - wb).abs() / wa.abs();
            println!(
                "    A vs A2: reconstructed widths {wa:.12e} s vs {wb:.12e} s, rel.diff = {rel:.3e}"
            );
            report.case("A2/tmax_does_not_move_the_clamp", rel <= 1e-9);
        }
        _ => {
            println!("    [FAIL] A vs A2 comparison unavailable (a run or a fit failed)");
            report.case("A2/tmax_does_not_move_the_clamp", false);
        }
    }
    println!();

    // --- Case B: step = 1 ns <= declared tr = 10 ns -> declared edge survives
    let case_b = PulseCase {
        label: "B",
        step: 1e-9,
        tmax: None,
        declared_tr: 10e-9,
        declared_tf: 10e-9,
        td: 1e-6,
        pw: 500e-9,
        stop: 2e-6,
    };
    println!(
        "[Case B] step = {:.0} ns, declared tr = tf = {:.0} ns, tmax = None \
         (step <= tr: the clamp is inert)",
        case_b.step * 1e9,
        case_b.declared_tr * 1e9
    );
    match run_pulse(&case_b) {
        Ok(tr) => {
            edge_case(
                &mut report,
                ("B/rise_declared_kept", "B/fall_declared_kept"),
                &case_b,
                &tr,
            );
        }
        Err(e) => {
            println!("    [FAIL] case B run failed: {e}");
            report.case("B/rise_declared_kept", false);
            report.case("B/fall_declared_kept", false);
        }
    }
    println!();

    // --- Case C: declared edge above the step (control) ---------------------
    let case_c = PulseCase {
        label: "C",
        step: 10e-9,
        tmax: None,
        declared_tr: 1e-6,
        declared_tf: 1e-6,
        td: 1e-6,
        pw: 2e-6,
        stop: 5.5e-6,
    };
    println!(
        "[Case C] step = {:.0} ns, declared tr = tf = {:.0} us, tmax = None (control: step << tr)",
        case_c.step * 1e9,
        case_c.declared_tr * 1e6
    );
    match run_pulse(&case_c) {
        Ok(tr) => {
            edge_case(
                &mut report,
                ("C/rise_declared_kept", "C/fall_declared_kept"),
                &case_c,
                &tr,
            );
        }
        Err(e) => {
            println!("    [FAIL] case C run failed: {e}");
            report.case("C/rise_declared_kept", false);
            report.case("C/fall_declared_kept", false);
        }
    }
    println!();

    // --- Case D: the historical configuration (declared 1 ps, step 500 ns) --
    let case_d = PulseCase {
        label: "D",
        step: TAU / 200.0, // 500 ns: the old probe's .tran step
        tmax: Some(TAU / 1000.0),
        declared_tr: 1e-12, // 1 ps
        declared_tf: 1e-12,
        td: 0.0,
        pw: 10.0,
        stop: 5.0 * TAU,
    };
    println!(
        "[Case D] historical configuration: step = {:.0} ns (= tau/200), declared tr = tf = 1 ps, \
         tmax = tau/1000",
        case_d.step * 1e9
    );
    println!(
        "    This is the assertion that moved out of the product test file: a declared edge \
         below the .tran step is executed as the step."
    );
    match run_pulse(&case_d) {
        Ok(tr) => {
            edge_case(
                &mut report,
                ("D/rise_1ps_executed_as_step", "D/fall_1ps_executed_as_step"),
                &case_d,
                &tr,
            );
            // The old product test also observed that the ideal-step model is
            // then off by ~1.95e-3 V. Reproduce the scale from this run: the
            // ramp-vs-step modelling residual is C(T)*exp(-u/tau) with
            // C(T) = 1 - (tau/T)*(exp(T/tau)-1) == (V0/rho)*expm1(-rho) * -1...
            let t_eff = case_d.declared_tr.max(case_d.step);
            let rho = t_eff / TAU;
            let c = 1.0 - (1.0 / rho) * rho.exp_m1();
            let ideal_step = |u: f64| {
                if u <= 0.0 {
                    0.0
                } else {
                    V0 * (1.0 - (-u / TAU).exp())
                }
            };
            let mut worst = 0.0_f64;
            let mut worst_t = 0.0_f64;
            for (i, &ti) in tr.time.iter().enumerate() {
                let e = (tr.vout[i] - ideal_step(ti)).abs();
                if e > worst {
                    worst = e;
                    worst_t = ti;
                }
            }
            println!(
                "    D/counterfactual: |v(out) - ideal-step| reaches {worst:.6e} V at t={worst_t:.6e} s; \
                 analytic |C(T_eff)| = {:.6e} V (T_eff = max(1 ps, {:.0} ns) = {:.0} ns)",
                c.abs(),
                case_d.step * 1e9,
                t_eff * 1e9
            );
            let ok = (worst - c.abs()).abs() <= 0.05 * c.abs();
            report.case("D/ideal_step_residual_matches_C_T", ok);
        }
        Err(e) => {
            println!("    [FAIL] case D run failed: {e}");
            report.case("D/rise_1ps_executed_as_step", false);
            report.case("D/fall_1ps_executed_as_step", false);
        }
    }
    println!();

    // --- Verdict -----------------------------------------------------------
    let failed = report.failures();
    println!("=== verdict ===");
    println!(
        "checks: {} total, {} passed, {} failed",
        report.cases.len(),
        report.cases.len() - failed.len(),
        failed.len()
    );
    if failed.is_empty() {
        println!("RESULT: ALL CONTRACT PINS MET (exit 0)");
        0
    } else {
        println!("RESULT: CONTRACT PINS NOT MET (exit 1) - failed: {failed:?}");
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
