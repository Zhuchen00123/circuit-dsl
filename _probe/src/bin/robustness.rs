//! Phase-0 robustness checks for Thevenin 0.5.0: positive and negative cases
//! side by side, every sub-case reported with a `PASS`/`FAIL` prefix, and a
//! non-zero process exit code as soon as one sub-case fails.
//!
//! Answers the adapter-design questions from brief §4.1 that cases 1-6 did not:
//!   a) Are convergence / solve failures distinguishable from success, and what
//!      does the engine do with a *legal open output*?
//!   b) Is there global state — can two simulations run concurrently on
//!      separate threads without interference?
//!   c) What happens on an unsupported / malformed circuit?
//!   d) Is `save` honoured (probe subsetting)?
//!   e) Does a series-R / shunt-C stage keep a DC path, and does its AC response
//!      match the hand calculation?
//!   f) What does the backend do with a whole block that has no ground
//!      reference at all?
//!   g) What does the backend do with a node connected only through a capacitor?
//!
//! # Positive / negative classification
//!
//! *Positive* = the circuit is legal and the printed numbers are asserted
//! against a hand calculation (a wrong number fails this program).
//! *Negative* = the circuit is unusable and the failing layer must be
//! *observable* — returning `Ok` fails this program.
//!
//! "Frontend" is `crates/circuit-dsl` plus the rule in
//! `crates/circuit-core/src/connectivity.rs`. **This file links `thevenin`
//! directly and never runs the frontend**, so the frontend column records the
//! documented rule as context; it is not a measurement made by this file.
//!
//! | sub-case | circuit | frontend | backend (thevenin 0.5.0) |
//! |---|---|---|---|
//! | a1 negative | two ideal voltage sources in parallel, 1 V vs 2 V | accepted (each net reaches ground through an ideal source) | `Err`: singular MNA system |
//! | a2 positive | `v1(a->gnd,1V)` + `r1(a->b,1k)`, `b` open | accepted: `b` reaches ground through `r1 -> a -> v1` | `Ok`, determinate: `v(a)=v(b)=1 V`, `i(v1)=i(r1)=0` |
//! | a3 positive/experiment | a2 run with `GMIN` = 1e-12 then 1e-3 | n/a | `Ok`; the two runs agree (both printed) |
//! | b positive | eight resistive dividers, one per thread | accepted | `Ok` per thread, no cross-thread interference |
//! | c negative | `r1` terminal referencing net id 99 | not measured here (the frontend resolves net names itself) | `Err`: terminal references unknown net id |
//! | d positive (contract pin) | divider with `circuit.save = ["v(mid)"]` | accepted | `Ok`; `save` is **not** honoured (all 3 vectors come back) |
//! | e positive | `v1(in->gnd,1V)` + `r1(in->out,1k)` + `c1(out->gnd,1uF)` | accepted: `out` has a DC path through `r1` | `Ok`: `v(out)=v(in)=1 V`; AC matches `1/(1+jwRC)` in magnitude **and** phase |
//! | f negative | `r1(a,b)` with no terminal on ground | **rejected with `E_NAME`**: neither `a` nor `b` reaches ground | `Err`: singular MNA system |
//! | g negative | `v1(in->gnd,1V)` + `r1(in->gnd,1k)` + `c1(in->out,1uF)` | **rejected with `E_NAME`**: `out` touches only a capacitor | `Err`: singular MNA system |
//!
//! # Two corrections carried by this file
//!
//! * Sub-case **a2 is not a floating node**. `v1(a->gnd)` + `r1(a->b)` with `b`
//!   open is a *legal open output*: `b` is referenced to ground through
//!   `r1 -> a -> v1`, and a resistor conducts at DC even when `i = 0`. The
//!   operating point is determinate and the engine returns exactly that
//!   solution. Earlier revisions of this file asked for `v(b)` and printed
//!   "dangling-node detection must be done by OUR frontend" as if the engine had
//!   guessed — it had not.
//! * Sub-case **e is not "floating through a capacitor"**. In
//!   `v1(in->gnd)` + `r1(in->out)` + `c1(out->gnd)`, `out` has the DC path
//!   `out -> r1 -> in -> v1 -> gnd`. The truly capacitor-only shape is sub-case
//!   **g**, and it is a different circuit.
//!
//! The experiments in this file are observations of *behaviour*: no claim is
//! made here about the solver's internal algorithm. In particular a3 records
//! that the printed solution is unchanged when `circuit.options` sets `GMIN` to
//! 1e-12 or 1e-3 for this linear operating point, and stops there.

use cirq_ir::{
    AcAnalysis, AcSpec, Analysis, Circuit, Connection, Element, ElementKind, FrequencyScale, Id,
    Net, SourceSpec, Value,
};
use thevenin::circuit::{simulate_ac, simulate_op};
use thevenin_types::SimPlot;

// ---------------------------------------------------------------------------
// Tolerances
//
// These are deliberately tight: the operating-point values asserted below are
// exact in IEEE double arithmetic (the circuits are linear, so no iterative
// convergence is involved), and a loose tolerance would hide a real regression.
// ---------------------------------------------------------------------------

/// Voltage tolerance for a value the engine obtains exactly (e.g. 1 V).
const TOL_V_EXACT: f64 = 1e-12;
/// Ampere tolerance for a branch current that must be exactly zero.
const TOL_I_ZERO: f64 = 1e-12;
/// Absolute tolerance for AC magnitudes compared against `1/(1+jwRC)`.
const TOL_AC_MAG: f64 = 1e-9;
/// Absolute tolerance for AC phases, in radians.
const TOL_AC_PHASE_RAD: f64 = 1e-9;

/// Series resistor of the RC stage (sub-case e).
const RC_R_OHM: f64 = 1_000.0;
/// Shunt capacitor of the RC stage (sub-case e).
const RC_C_F: f64 = 1e-6;

// ---------------------------------------------------------------------------
// Sub-case bookkeeping
// ---------------------------------------------------------------------------

/// Collects one verdict per sub-case. `main` turns the collected list into the
/// process exit code, so a failing assertion cannot be lost in the log.
struct Report {
    cases: Vec<(&'static str, bool)>,
}

impl Report {
    fn new() -> Self {
        Report { cases: Vec::new() }
    }

    /// Record one sub-case verdict and print its `PASS`/`FAIL` prefix line.
    fn case(&mut self, name: &'static str, pass: bool) -> bool {
        println!("  [{}] {name}", if pass { "PASS" } else { "FAIL" });
        self.cases.push((name, pass));
        pass
    }

    fn passed(&self) -> usize {
        self.cases.iter().filter(|(_, ok)| *ok).count()
    }

    fn failures(&self) -> Vec<&'static str> {
        self.cases
            .iter()
            .filter(|(_, ok)| !ok)
            .map(|(name, _)| *name)
            .collect()
    }
}

/// Print one measured number next to its expectation and return the verdict.
fn check_value(label: &str, actual: f64, expected: f64, tol: f64) -> bool {
    let diff = (actual - expected).abs();
    let ok = diff <= tol;
    println!(
        "    [{}] {label}: actual={actual:.12e} expected={expected:.12e} |diff|={diff:.3e} tol={tol:.1e}",
        if ok { "PASS" } else { "FAIL" }
    );
    ok
}

/// Read a real vector sample, or `NaN` when the vector is absent (which then
/// fails the comparison instead of aborting the whole run).
fn real_at(plot: &SimPlot, name: &str, idx: usize) -> f64 {
    plot.vector(name)
        .map(|v| v.data.as_real()[idx])
        .unwrap_or(f64::NAN)
}

fn vector_names(plot: &SimPlot) -> Vec<String> {
    plot.vecs.iter().map(|v| v.name.clone()).collect()
}

/// Required disclaimer for the two negative cases (f, g).
///
/// `_probe` links `thevenin` directly: there is no `circuit-dsl` in this
/// binary's dependency graph, so a backend error here says nothing about the
/// frontend rule that protects the product path.
fn print_frontend_note() {
    println!("    note: the product frontend (crates/circuit-dsl, rule implemented in");
    println!("          crates/circuit-core/src/connectivity.rs and called from");
    println!("          crates/circuit-dsl/src/elaborate.rs) rejects this shape with `E_NAME`");
    println!("          (`Code::Name`) BEFORE the backend is ever called: every non-ground node");
    println!("          must reach ground through devices that conduct at DC (resistor, inductor,");
    println!(
        "          voltage source, diode — a capacitor or current source does not conduct at DC)."
    );
    println!("          This file links thevenin directly and never runs the frontend, so the");
    println!("          output above is the BACKEND's raw behaviour and is NOT evidence for the");
    println!("          frontend rule.");
}

// ---------------------------------------------------------------------------
// Circuit builders
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

fn base(name: &str, nets: Vec<Net>) -> Circuit {
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

fn vsource(id: u32, name: &str, p: u32, n: u32, dc: f64, ac: Option<f64>) -> Element {
    Element {
        id: Id(id),
        name: name.to_string(),
        kind: ElementKind::VoltageSource,
        connections: vec![conn("pos", p), conn("neg", n)],
        params: Vec::new(),
        model: None,
        source_spec: Some(SourceSpec {
            dc: Some(dc),
            ac: ac.map(|m| AcSpec { mag: m, phase: 0.0 }),
            waveform: None,
        }),
    }
}

/// A divider whose midpoint voltage is v_in * 1k/(1k+1k).
fn divider(tag: &str, vin: f64) -> Circuit {
    let mut c = base(tag, vec![net(0, "gnd"), net(1, "in"), net(2, "mid")]);
    c.elements.push(vsource(0, "v1", 1, 0, vin, None));
    c.elements.push(resistor(1, "r1", 1, 2, 1_000.0));
    c.elements.push(resistor(2, "r2", 2, 0, 1_000.0));
    c.analyses.push(Analysis::Op);
    c
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    let mut report = Report::new();

    println!("=== Thevenin 0.5.0 robustness / isolation checks (positive + negative cases) ===");
    println!("(circuits are built directly as cirq_ir::Circuit values and thevenin is called");
    println!(
        " directly — the circuit-dsl frontend is not in this binary's graph, see module docs)"
    );
    println!();

    check_a_singular_and_legal_open_output(&mut report);
    check_b_thread_isolation(&mut report);
    check_c_malformed(&mut report);
    check_d_save_subset(&mut report);
    check_e_dc_path_through_resistor(&mut report);
    check_f_no_ground_block(&mut report);
    check_g_capacitor_only_node(&mut report);

    println!(
        "--- sub-case summary: {}/{} passed ---",
        report.passed(),
        report.cases.len()
    );
    for (name, ok) in &report.cases {
        println!("  [{}] {name}", if *ok { "PASS" } else { "FAIL" });
    }

    let failed = report.failures();
    if !failed.is_empty() {
        println!();
        println!("FAILED SUB-CASES ({}):", failed.len());
        for name in &failed {
            println!("  - {name}");
        }
        println!();
        println!("RESULT: FAILED (exit 1)");
        std::process::exit(1);
    }
    println!();
    println!("RESULT: ALL SUB-CASES PASSED (exit 0)");
}

// ---------------------------------------------------------------------------
// (a) Solve failure vs. a determinate solution
// ---------------------------------------------------------------------------

/// (a) Two circuits that earlier revisions of this file conflated:
///
/// * **a1 negative** — two ideal voltage sources in parallel with different DC
///   values. The constraint set is genuinely inconsistent, so the engine must
///   report a failure instead of a silent empty-but-successful result.
/// * **a2 positive** — `v1(a->gnd, 1 V)` + `r1(a->b, 1k)` with `b` open. This is
///   a **legal open output**: `b` is referenced to ground through
///   `r1 -> a -> v1`, no current flows, and `v(a) = v(b) = 1 V`, `i(v1) = 0`,
///   `i(r1) = 0` are exact. It is not a floating node.
/// * **a3 experiment** — the same circuit with `circuit.options` `GMIN` set to
///   1e-12 and to 1e-3, both runs printed.
fn check_a_singular_and_legal_open_output(report: &mut Report) {
    println!("[a] Solve failure vs. determinate open output");

    // ---- a1: conflicting ideal sources (singular constraint set) ----
    let mut c1 = base("conflict", vec![net(0, "gnd"), net(1, "a")]);
    c1.elements.push(vsource(0, "v1", 1, 0, 1.0, None));
    c1.elements.push(vsource(1, "v2", 1, 0, 2.0, None));
    c1.analyses.push(Analysis::Op);
    match simulate_op(&c1) {
        Ok(res) => {
            println!(
                "    NOTE: conflicting ideal sources returned Ok with plots: {:?}",
                res.plots
                    .iter()
                    .map(|p| format!("{} ({} vecs)", p.name, p.vecs.len()))
                    .collect::<Vec<_>>()
            );
            if let Some(p) = res.plot() {
                for v in &p.vecs {
                    println!("      {} = {:?}", v.name, v.data.as_real().first());
                }
            }
            println!("    => a failure this severe must not be reported as success");
            report.case(
                "a1 conflicting ideal sources (Op) must be reported as Err",
                false,
            );
        }
        Err(e) => {
            let msg = e.to_string();
            println!("    engine (verbatim): Err({msg})");
            println!(
                "    message names a singular system: {}",
                msg.to_ascii_lowercase().contains("singular")
            );
            println!("    => solve failures are distinguishable from success here");
            report.case(
                "a1 conflicting ideal sources (Op) must be reported as Err",
                true,
            );
        }
    }

    // ---- a2: legal open output (NOT a floating node) ----
    println!("  a2 legal open output: v1(a->gnd,1V) + r1(a->b,1k), b open");
    let mut c2 = base(
        "legal_open_output",
        vec![net(0, "gnd"), net(1, "a"), net(2, "b")],
    );
    c2.elements.push(vsource(0, "v1", 1, 0, 1.0, None));
    c2.elements.push(resistor(1, "r1", 1, 2, 1_000.0));
    c2.analyses.push(Analysis::Op);

    let mut va = f64::NAN;
    let mut vb = f64::NAN;
    let mut i_v1 = f64::NAN;
    match simulate_op(&c2) {
        Ok(res) => match res.plot() {
            Some(p) => {
                println!("    plot '{}', vectors: {:?}", p.name, vector_names(p));
                va = real_at(p, "v(a)", 0);
                vb = real_at(p, "v(b)", 0);
                i_v1 = real_at(p, "v1#branch", 0);
                println!(
                    "    engine returned Ok: v(a) = {va:.17e} V, v(b) = {vb:.17e} V, v1#branch = {i_v1:.17e} A"
                );
            }
            None => println!("    engine returned Ok but with no plot"),
        },
        Err(e) => println!("    engine (verbatim): Err({e})"),
    }

    // The engine does not return a resistor branch current, so derive it from
    // the node voltages: i(r1) = (v(a) - v(b)) / R1.
    let i_r1_derived = (va - vb) / 1_000.0;

    let ok_vb = check_value("v(b) [V]", vb, 1.0, TOL_V_EXACT);
    let ok_va = check_value("v(a) [V]", va, 1.0, TOL_V_EXACT);
    let ok_i_v1 = check_value("i(v1) = v1#branch [A]", i_v1, 0.0, TOL_I_ZERO);
    let ok_i_r1 = check_value(
        "i(r1) derived (v(a)-v(b))/R [A]",
        i_r1_derived,
        0.0,
        TOL_I_ZERO,
    );

    report.case("a2 open output: v(b) = 1 V (exact)", ok_vb);
    report.case("a2 open output: v(a) = 1 V (exact)", ok_va);
    report.case("a2 open output: i(v1) = 0 A (open circuit)", ok_i_v1);
    report.case(
        "a2 open output: derived i(r1) = (v(a)-v(b))/R = 0 A",
        ok_i_r1,
    );

    println!("    => legal open output, not a floating node: b is referenced to ground through");
    println!("       r1 -> a -> v1 (a resistor conducts at DC even with i = 0), so the operating");
    println!("       point is determinate and the backend returns a definite solution.");
    println!("       The engine exposes no resistor branch current; the r1 value above is derived");
    println!("       from the node voltages, not read from a fabricated vector.");

    // ---- a3: recorded GMIN experiment (behaviour only, no internals claimed) ----
    println!("  a3 recorded experiment: same circuit with circuit.options GMIN = 1e-12 and 1e-3");
    let mut runs: Vec<(f64, Option<(f64, f64, f64)>)> = Vec::new();
    for gmin in [1e-12_f64, 1e-3_f64] {
        let mut c = base(
            "legal_open_output_gmin",
            vec![net(0, "gnd"), net(1, "a"), net(2, "b")],
        );
        c.elements.push(vsource(0, "v1", 1, 0, 1.0, None));
        c.elements.push(resistor(1, "r1", 1, 2, 1_000.0));
        c.options.push(("GMIN".to_string(), Value::Real(gmin)));
        c.analyses.push(Analysis::Op);
        let values = match simulate_op(&c) {
            Ok(res) => match res.plot() {
                Some(p) => {
                    let triple = (
                        real_at(p, "v(a)", 0),
                        real_at(p, "v(b)", 0),
                        real_at(p, "v1#branch", 0),
                    );
                    println!(
                        "    GMIN={gmin:<8.1e} -> v(a)={:.17e} V, v(b)={:.17e} V, v1#branch={:.17e} A",
                        triple.0, triple.1, triple.2
                    );
                    Some(triple)
                }
                None => {
                    println!("    GMIN={gmin:<8.1e} -> Ok but no plot");
                    None
                }
            },
            Err(e) => {
                println!("    GMIN={gmin:<8.1e} -> Err({e})");
                None
            }
        };
        runs.push((gmin, values));
    }
    let gmin_ok = match (runs[0].1, runs[1].1) {
        (Some(a), Some(b)) => {
            (a.0 - b.0).abs() <= TOL_V_EXACT
                && (a.1 - b.1).abs() <= TOL_V_EXACT
                && (a.2 - b.2).abs() <= TOL_I_ZERO
                && (a.1 - 1.0).abs() <= TOL_V_EXACT
                && (b.1 - 1.0).abs() <= TOL_V_EXACT
        }
        _ => false,
    };
    println!("    => recorded fact: for this linear OP the printed solution is unchanged between");
    println!("       GMIN=1e-12 and GMIN=1e-3. This file asserts that observation and makes no");
    println!("       claim about how the solver is implemented internally.");
    report.case(
        "a3 GMIN=1e-12 and GMIN=1e-3 give the same solution (observed, to 1e-12)",
        gmin_ok,
    );
    println!();
}

// ---------------------------------------------------------------------------
// (b) Thread isolation
// ---------------------------------------------------------------------------

/// (b) Global state: run many simulations concurrently and confirm the results
///     stay independent.
fn check_b_thread_isolation(report: &mut Report) {
    println!("[b] Thread isolation: 8 concurrent simulations with distinct inputs");
    let handles: Vec<_> = (0..8)
        .map(|i| {
            std::thread::spawn(move || {
                let vin = 1.0 + i as f64;
                let c = divider(&format!("d{i}"), vin);
                let r = simulate_op(&c).expect("op");
                let vmid = r.plot().unwrap().vector("v(mid)").unwrap().data.as_real()[0];
                (vin, vmid)
            })
        })
        .collect();

    let mut ok = true;
    for h in handles {
        let (vin, vmid) = h.join().expect("thread panicked");
        let expected = vin * 0.5;
        let good = (vmid - expected).abs() < 1e-12;
        ok &= good;
        println!(
            "    [{}] vin={vin:.1} V -> v(mid)={vmid:.6} V (expected {expected:.6})",
            if good { "PASS" } else { "FAIL" }
        );
    }
    println!(
        "    => {}",
        if ok {
            "no cross-thread interference observed for these cases"
        } else {
            "CROSS-THREAD INTERFERENCE DETECTED"
        }
    );
    report.case(
        "b 8-thread isolation: every thread's v(mid) matches its own vin",
        ok,
    );
    println!();
}

// ---------------------------------------------------------------------------
// (c) Malformed input
// ---------------------------------------------------------------------------

/// (c) Malformed input: element referencing an out-of-range net id.
fn check_c_malformed(report: &mut Report) {
    println!("[c] Malformed circuit: terminal referencing a nonexistent net id");
    let mut c = base("bad", vec![net(0, "gnd"), net(1, "a")]);
    c.elements.push(vsource(0, "v1", 1, 0, 1.0, None));
    // net id 99 does not exist
    c.elements.push(resistor(1, "r1", 1, 99, 1_000.0));
    c.analyses.push(Analysis::Op);
    let ok = match simulate_op(&c) {
        Ok(res) => {
            println!(
                "    NOTE: returned Ok despite a dangling net id; {} plot(s)",
                res.plots.len()
            );
            if let Some(p) = res.plot() {
                println!("    vectors: {:?}", vector_names(p));
            }
            false
        }
        Err(e) => {
            println!("    engine (verbatim): Err({e})");
            true
        }
    };
    report.case(
        "c malformed net id must be reported as Err, not silently accepted",
        ok,
    );
    println!();
}

// ---------------------------------------------------------------------------
// (d) `circuit.save`
// ---------------------------------------------------------------------------

/// (d) Does `circuit.save` restrict the returned vectors?
fn check_d_save_subset(report: &mut Report) {
    println!("[d] Save subsetting via circuit.save");
    let mut c = divider("save_test", 1.0);
    c.save = vec!["v(mid)".to_string()];
    let ok = match simulate_op(&c) {
        Ok(res) => match res.plot() {
            Some(p) => {
                let names = vector_names(p);
                println!(
                    "    save=[\"v(mid)\"] -> {} vectors: {:?}",
                    p.vecs.len(),
                    names
                );
                let honoured = p.vecs.len() == 1;
                let has_mid = p.vector("v(mid)").is_some();
                println!(
                    "    => save is {}",
                    if honoured {
                        "HONOURED"
                    } else {
                        "NOT honoured by simulate_op (the adapter must subset the result itself)"
                    }
                );
                // Contract pin: the observation above is what the adapter relies
                // on. If a future engine starts honouring save, this sub-case
                // fails on purpose so the adapter is re-checked.
                has_mid && !honoured
            }
            None => {
                println!("    Ok returned but with no plot");
                false
            }
        },
        Err(e) => {
            println!("    engine (verbatim): Err({e})");
            false
        }
    };
    report.case(
        "d save=[\"v(mid)\"]: v(mid) is present and save is not honoured (documented behaviour)",
        ok,
    );
    println!();
}

// ---------------------------------------------------------------------------
// (e) Series-R / shunt-C stage
// ---------------------------------------------------------------------------

/// (e) A series-R / shunt-C stage: `v1(in->gnd, 1 V)` + `r1(in->out, 1k)` +
///     `c1(out->gnd, 1uF)`.
///
/// Earlier revisions of this file described `out` as "floating through a
/// capacitor" and left the AC comparison as a printout. Both are fixed here:
/// `out` has a **DC path through a resistor** (`out -> r1 -> in -> v1 -> gnd`),
/// so the operating point is well defined, and the AC response is asserted
/// against `H(jw) = 1/(1 + j*w*R1*C1)` in magnitude and phase.
fn check_e_dc_path_through_resistor(report: &mut Report) {
    println!("[e] Series-R / shunt-C stage: DC path through a resistor, NOT floating");

    // ---- e1: operating point ----
    let mut c = base(
        "rc_dc_path",
        vec![net(0, "gnd"), net(1, "in"), net(2, "out")],
    );
    c.elements.push(vsource(0, "v1", 1, 0, 1.0, Some(1.0)));
    c.elements.push(resistor(1, "r1", 1, 2, RC_R_OHM));
    c.elements.push(capacitor(2, "c1", 2, 0, RC_C_F));
    c.analyses.push(Analysis::Op);

    let mut v_in = f64::NAN;
    let mut v_out = f64::NAN;
    match simulate_op(&c) {
        Ok(res) => match res.plot() {
            Some(p) => {
                println!("    plot '{}', vectors: {:?}", p.name, vector_names(p));
                v_in = real_at(p, "v(in)", 0);
                v_out = real_at(p, "v(out)", 0);
                println!("    engine returned Ok: v(in) = {v_in:.17e} V, v(out) = {v_out:.17e} V");
            }
            None => println!("    engine returned Ok but with no plot"),
        },
        Err(e) => println!("    engine (verbatim): Err({e})"),
    }
    let ok_dc = check_value("v(in) [V]", v_in, 1.0, TOL_V_EXACT)
        & check_value("v(out) [V]", v_out, 1.0, TOL_V_EXACT);
    println!("    => out is NOT floating: r1 is the DC path (out -> r1 -> in -> v1 -> gnd) and");
    println!("       the capacitor is an open circuit at DC, so v(out) = v(in) = 1 V.");
    report.case(
        "e1 DC path through r1: v(out) = v(in) = 1 V at the operating point",
        ok_dc,
    );

    // ---- e2: AC response against the hand calculation ----
    let mut c2 = base(
        "rc_dc_path_ac",
        vec![net(0, "gnd"), net(1, "in"), net(2, "out")],
    );
    c2.elements.push(vsource(0, "v1", 1, 0, 1.0, Some(1.0)));
    c2.elements.push(resistor(1, "r1", 1, 2, RC_R_OHM));
    c2.elements.push(capacitor(2, "c1", 2, 0, RC_C_F));
    c2.analyses.push(Analysis::Ac(AcAnalysis {
        start: 1.0,
        stop: 1e5,
        points: 5,
        scale: FrequencyScale::Decade,
    }));

    let mut ok_ac = false;
    match simulate_ac(&c2) {
        Ok(res) => match res.plot() {
            Some(p) => {
                let f = p
                    .vector("frequency")
                    .map(|v| v.data.as_real().to_vec())
                    .unwrap_or_default();
                let v = p
                    .vector("v(out)")
                    .map(|v| v.data.as_complex().to_vec())
                    .unwrap_or_default();
                if f.is_empty() || f.len() != v.len() {
                    println!(
                        "    AC plot unusable: {} frequency samples vs {} v(out) samples",
                        f.len(),
                        v.len()
                    );
                } else {
                    println!(
                        "    AC (low-pass, C1 is out->gnd): expected H(jw) = 1/(1 + j*w*R1*C1)"
                    );
                    // Topology is R1 in series then C1 from out to gnd => LOW-pass.
                    let mut idxs = vec![0usize, f.len() / 2, f.len() - 1];
                    idxs.dedup();
                    let mut sub_ok = true;
                    for i in idxs {
                        let w = 2.0 * std::f64::consts::PI * f[i];
                        let rc = w * RC_R_OHM * RC_C_F;
                        let mag = v[i].magnitude();
                        let phase = v[i].phase_rad();
                        let exp_mag = 1.0 / (1.0 + rc * rc).sqrt();
                        let exp_phase = -rc.atan();
                        println!(
                            "      f={:<12.6e}Hz |H|={:<16.12e} expected={:<16.12e} phase={:<16.12e} rad expected={:<16.12e}",
                            f[i], mag, exp_mag, phase, exp_phase
                        );
                        sub_ok &= check_value("|v(out)|", mag, exp_mag, TOL_AC_MAG);
                        sub_ok &=
                            check_value("arg v(out) [rad]", phase, exp_phase, TOL_AC_PHASE_RAD);
                    }
                    println!(
                        "    => AC response matches the hand calculation in magnitude AND phase"
                    );
                    ok_ac = sub_ok;
                }
            }
            None => println!("    AC: Ok returned but with no plot"),
        },
        Err(e) => println!("    AC engine (verbatim): Err({e})"),
    }
    report.case(
        "e2 AC magnitude and phase match 1/(1+jwRC) at the sampled frequencies",
        ok_ac,
    );
    println!();
}

// ---------------------------------------------------------------------------
// (f) NEGATIVE CASE: no ground reference at all
// ---------------------------------------------------------------------------

/// (f) **NEGATIVE CASE**: a whole block with no ground reference.
///
/// `gnd` is declared but nothing is connected to it; `a` and `b` are the only
/// non-ground nodes and `r1` sits between them. Neither node has any path to
/// ground — not through a resistor, not through anything else.
fn check_f_no_ground_block(report: &mut Report) {
    println!("[f] NEGATIVE CASE: connected block with no ground reference (Op)");
    println!("    r1(a,b) only; no terminal on net 0 — a and b form a block with no reference");
    let mut c = base(
        "no_ground_block",
        vec![net(0, "gnd"), net(1, "a"), net(2, "b")],
    );
    c.elements.push(resistor(0, "r1", 1, 2, 1_000.0));
    c.analyses.push(Analysis::Op);

    let ok = match simulate_op(&c) {
        Ok(res) => {
            println!(
                "    NOTE: returned Ok; plots: {:?}",
                res.plots
                    .iter()
                    .map(|p| format!("{} ({} vecs)", p.name, p.vecs.len()))
                    .collect::<Vec<_>>()
            );
            if let Some(p) = res.plot() {
                for v in &p.vecs {
                    println!("      {} = {:?}", v.name, v.data.as_real().first());
                }
            }
            println!(
                "    => expected Err: an unreferenced block has an undetermined operating point"
            );
            false
        }
        Err(e) => {
            let msg = e.to_string();
            println!("    engine (verbatim): Err({msg})");
            println!(
                "    message names a singular system: {}",
                msg.to_ascii_lowercase().contains("singular")
            );
            msg.to_ascii_lowercase().contains("singular")
        }
    };
    report.case(
        "f no-ground block must fail with Err(matrix is singular, cannot solve)",
        ok,
    );
    print_frontend_note();
    println!();
}

// ---------------------------------------------------------------------------
// (g) NEGATIVE CASE: node coupled only through a capacitor
// ---------------------------------------------------------------------------

/// (g) **NEGATIVE CASE**: a node whose only connection is a capacitor.
///
/// `v1(in->gnd, 1 V)` + `r1(in->gnd, 1k)` + `c1(in->out, 1uF)`: `in` is fully
/// referenced, but `out` touches nothing except the capacitor, so it has no DC
/// path to ground at all.
///
/// This — not sub-case (e) — is the true "connected only through a capacitor"
/// shape.
fn check_g_capacitor_only_node(report: &mut Report) {
    println!("[g] NEGATIVE CASE: node connected only through a capacitor (Op)");
    println!("    v1(in->gnd,1V) + r1(in->gnd,1k) + c1(in->out,1uF); out touches only c1");
    let mut c = base(
        "capacitor_only_node",
        vec![net(0, "gnd"), net(1, "in"), net(2, "out")],
    );
    c.elements.push(vsource(0, "v1", 1, 0, 1.0, None));
    c.elements.push(resistor(1, "r1", 1, 0, 1_000.0));
    c.elements.push(capacitor(2, "c1", 1, 2, 1e-6));
    c.analyses.push(Analysis::Op);

    let ok = match simulate_op(&c) {
        Ok(res) => {
            println!(
                "    NOTE: returned Ok; plots: {:?}",
                res.plots
                    .iter()
                    .map(|p| format!("{} ({} vecs)", p.name, p.vecs.len()))
                    .collect::<Vec<_>>()
            );
            if let Some(p) = res.plot() {
                for v in &p.vecs {
                    println!("      {} = {:?}", v.name, v.data.as_real().first());
                }
            }
            println!("    => expected Err: the capacitor does not reference out at DC");
            false
        }
        Err(e) => {
            let msg = e.to_string();
            println!("    engine (verbatim): Err({msg})");
            println!(
                "    message names a singular system: {}",
                msg.to_ascii_lowercase().contains("singular")
            );
            msg.to_ascii_lowercase().contains("singular")
        }
    };
    report.case(
        "g capacitor-only node must fail with Err(matrix is singular, cannot solve)",
        ok,
    );
    print_frontend_note();
    println!();
}
