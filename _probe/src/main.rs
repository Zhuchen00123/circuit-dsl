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
        VectorData::Real(d) => d.iter().map(|x| thevenin_types::Complex::new(*x, 0.0)).collect(),
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
fn plot_named<'a>(res: &'a thevenin_types::SimResult, prefix: &str) -> Option<&'a thevenin_types::SimPlot> {
    res.plots
        .iter()
        .find(|p| p.name.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()))
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

    all_ok &= case1_divider_op();
    all_ok &= case2_rc_tran();
    all_ok &= case3_rc_ac();
    all_ok &= case4_rlc_ac();
    all_ok &= case5_diode_op();
    all_ok &= case6_dc_sweep();

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
// Case 2 — RC transient: step response
// ---------------------------------------------------------------------------

fn case2_rc_tran() -> bool {
    println!("[Case 2] RC transient (step response)");
    const R: f64 = 1_000.0;
    const C: f64 = 100e-9;
    let tau = R * C;

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
                v2: 1.0,
                td: Some(0.0),
                tr: Some(1e-12),
                tf: Some(1e-12),
                pw: Some(10.0),
                per: Some(20.0),
            }),
            ..Default::default()
        },
    ));
    c.elements.push(resistor(1, "r1", 1, 2, R));
    c.elements.push(capacitor(2, "c1", 2, 0, C));
    c.analyses.push(Analysis::Tran(TranAnalysis {
        step: tau / 200.0,
        stop: 5.0 * tau,
        start: 0.0,
        uic: false,
        tmax: Some(tau / 200.0),
    }));

    let res = match simulate_tran(&c) {
        Ok(r) => r,
        Err(e) => {
            println!("  [FAIL] simulate_tran error: {e}");
            return false;
        }
    };
    let plot = match plot_named(&res, "tran") {
        Some(p) => p,
        None => {
            println!("  [FAIL] no 'tran*' plot; got: {}", plot_listing(&res));
            return false;
        }
    };
    println!("  plots: {}; using '{}'", plot_listing(&res), plot.name);
    println!("  vectors: {:?}", vector_names(plot));

    let t = real(plot.get("time"));
    let vout = real(plot.get("v(out)"));
    println!(
        "  {} time points, t in [{:.3e}, {:.3e}] s",
        t.len(),
        t[0],
        t[t.len() - 1]
    );

    let mut ok = true;
    let mut worst = 0.0f64;
    for frac in [0.25, 0.5, 1.0, 2.0, 3.0] {
        let target = frac * tau;
        let idx = t
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
        let expected = 1.0 - (-t[idx] / tau).exp();
        let err = (vout[idx] - expected).abs();
        worst = worst.max(err);
        let case_ok = err < 1e-2;
        ok &= case_ok;
        println!(
            "  [{}] t={:>8.3}us ({:.2}tau)  v(out)={:.6}  analytic={:.6}  |diff|={:.2e}",
            if case_ok { "PASS" } else { "FAIL" },
            t[idx] * 1e6,
            t[idx] / tau,
            vout[idx],
            expected,
            err
        );
    }
    println!("  worst |diff| = {:.3e}", worst);
    println!();
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
    println!("  resonance f0 = {:.3} Hz; worst |diff| = {:.3e}", f0, worst);
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
    println!("  v(out) = {:.6} V ; vectors: {:?}", vout, vector_names(plot));

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
