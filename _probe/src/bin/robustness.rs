//! Phase-0 robustness checks for Thevenin 0.5.0.
//!
//! Answers the adapter-design questions from brief §4.1 that case 1-6 did not:
//!   a) Are convergence / solve failures distinguishable from success?
//!   b) Is there global state — can two simulations run concurrently on
//!      separate threads without interference?
//!   c) What happens on an unsupported / malformed circuit?
//!   d) Is `save` honoured (probe subsetting)?
//!   e) Does an isolated sub-block (floating subcircuit) still solve?

use cirq_ir::{
    AcAnalysis, AcSpec, Analysis, Circuit, Connection, Element, ElementKind, FrequencyScale, Id,
    Net, SourceSpec, Value,
};
use thevenin::circuit::{simulate_ac, simulate_op};

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

fn main() {
    println!("=== Thevenin 0.5.0 robustness / isolation checks ===\n");

    check_a_convergence_failure();
    check_b_thread_isolation();
    check_c_malformed();
    check_d_save_subset();
    check_e_isolated_island();
}

/// (a) Can we produce a *distinguishable* failure rather than a silent
///     empty-but-successful result?
fn check_a_convergence_failure() {
    println!("[a] Failure reporting: singular / ill-posed circuit");

    // Two ideal voltage sources in parallel with different DC values is a
    // classic singular system (conflicting ideal constraints).
    let mut c = base("conflict", vec![net(0, "gnd"), net(1, "a")]);
    c.elements.push(vsource(0, "v1", 1, 0, 1.0, None));
    c.elements.push(vsource(1, "v2", 1, 0, 2.0, None));
    c.analyses.push(Analysis::Op);

    match simulate_op(&c) {
        Ok(r) => {
            println!(
                "  NOTE: conflicting sources returned Ok with plots: {:?}",
                r.plots
                    .iter()
                    .map(|p| format!("{} ({} vecs)", p.name, p.vecs.len()))
                    .collect::<Vec<_>>()
            );
            if let Some(p) = r.plot() {
                for v in &p.vecs {
                    println!("    {} = {:?}", v.name, &v.data.as_real().first());
                }
            }
            println!("  => failure is NOT surfaced as Err for this case\n");
        }
        Err(e) => println!("  OK: returned Err({e})\n"),
    }

    // A floating node with no DC path to ground.
    let mut c2 = base("float", vec![net(0, "gnd"), net(1, "a"), net(2, "b")]);
    c2.elements.push(vsource(0, "v1", 1, 0, 1.0, None));
    c2.elements.push(resistor(1, "r1", 1, 2, 1_000.0));
    c2.analyses.push(Analysis::Op);
    match simulate_op(&c2) {
        Ok(r) => {
            let p = r.plot().expect("plot");
            let vb = p
                .vector("v(b)")
                .map(|v| v.data.as_real()[0])
                .unwrap_or(f64::NAN);
            println!("  floating node v(b) = {vb} (Ok returned, no ground path)");
            println!("  => dangling-node detection must be done by OUR frontend\n");
        }
        Err(e) => println!("  floating node returned Err({e})\n"),
    }
}

/// (b) Global state: run many simulations concurrently and confirm the
///     results stay independent.
fn check_b_thread_isolation() {
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
            "  [{}] vin={vin:.1} V -> v(mid)={vmid:.6} V (expected {expected:.6})",
            if good { "PASS" } else { "FAIL" }
        );
    }
    println!(
        "  => {}\n",
        if ok {
            "no cross-thread interference observed for these cases"
        } else {
            "CROSS-THREAD INTERFERENCE DETECTED"
        }
    );
}

/// (c) Malformed input: element referencing an out-of-range net id.
fn check_c_malformed() {
    println!("[c] Malformed circuit: terminal referencing a nonexistent net id");
    let mut c = base("bad", vec![net(0, "gnd"), net(1, "a")]);
    c.elements.push(vsource(0, "v1", 1, 0, 1.0, None));
    // net id 99 does not exist
    c.elements.push(resistor(1, "r1", 1, 99, 1_000.0));
    c.analyses.push(Analysis::Op);
    match simulate_op(&c) {
        Ok(r) => println!(
            "  NOTE: returned Ok despite dangling net id; {} plot(s)\n",
            r.plots.len()
        ),
        Err(e) => println!("  OK: Err({e})\n"),
    }
}

/// (d) Does `circuit.save` restrict the returned vectors?
fn check_d_save_subset() {
    println!("[d] Save subsetting via `circuit.save`");
    let mut c = divider("save_test", 1.0);
    c.save = vec!["v(mid)".to_string()];
    match simulate_op(&c) {
        Ok(r) => {
            let p = r.plot().expect("plot");
            println!(
                "  save=[\"v(mid)\"] -> {} vectors: {:?}",
                p.vecs.len(),
                p.vecs.iter().map(|v| v.name.clone()).collect::<Vec<_>>()
            );
            println!("  => save is {}\n", if p.vecs.len() == 1 { "HONOURED (useful for probe subsetting)" } else { "NOT honoured by simulate_op (adapter must subset itself)" });
        }
        Err(e) => println!("  Err({e})\n"),
    }
}

/// (e) A sub-block with no DC path to the rest of the circuit.
fn check_e_isolated_island() {
    println!("[e] Isolated island (AC-coupled block) operating point");
    // v1 - R1 - out, with C1 from out to gnd: no DC path from out to gnd.
    // This is the classic "floating through a capacitor" case the brief warns
    // about (a capacitor path is not a DC reference path).
    let mut c = base("island", vec![net(0, "gnd"), net(1, "in"), net(2, "out")]);
    c.elements.push(vsource(0, "v1", 1, 0, 1.0, Some(1.0)));
    c.elements.push(resistor(1, "r1", 1, 2, 1_000.0));
    c.elements.push(Element {
        id: Id(2),
        name: "c1".to_string(),
        kind: ElementKind::Capacitor,
        connections: vec![conn("pos", 2), conn("neg", 0)],
        params: vec![("value".to_string(), Value::Real(1e-6))],
        model: None,
        source_spec: None,
    });
    c.analyses.push(Analysis::Op);
    match simulate_op(&c) {
        Ok(r) => {
            let p = r.plot().expect("plot");
            for n in ["v(out)", "v(in)"] {
                if let Some(v) = p.vector(n) {
                    println!("  v{} = {}", n.trim_start_matches('v'), v.data.as_real()[0]);
                }
            }
        }
        Err(e) => println!("  Err({e})"),
    }

    // The same circuit in AC: no DC path needed for the AC solve.
    let mut c2 = base("island_ac", vec![net(0, "gnd"), net(1, "in"), net(2, "out")]);
    c2.elements.push(vsource(0, "v1", 1, 0, 1.0, Some(1.0)));
    c2.elements.push(resistor(1, "r1", 1, 2, 1_000.0));
    c2.elements.push(Element {
        id: Id(2),
        name: "c1".to_string(),
        kind: ElementKind::Capacitor,
        connections: vec![conn("pos", 2), conn("neg", 0)],
        params: vec![("value".to_string(), Value::Real(1e-6))],
        model: None,
        source_spec: None,
    });
    c2.analyses.push(Analysis::Ac(AcAnalysis {
        start: 1.0,
        stop: 1e5,
        points: 5,
        scale: FrequencyScale::Decade,
    }));
    match simulate_ac(&c2) {
        Ok(r) => {
            let p = r.plot().expect("plot");
            let f = p.vector("frequency").unwrap().data.as_real().to_vec();
            let v = p.vector("v(out)").unwrap().data.as_complex().to_vec();
            // Topology is R1 in series then C1 from out to gnd => LOW-pass:
            //   H(jw) = 1/(1 + jw*R1*C1)
            println!("  AC on the same island (low-pass: C1 is out->gnd):");
            for i in [0, f.len() / 2, f.len() - 1] {
                let w = 2.0 * std::f64::consts::PI * f[i];
                let rc = w * 1e3 * 1e-6;
                let expected = 1.0 / (1.0 + rc * rc).sqrt();
                println!(
                    "    f={:<11.4e}Hz |H|={:.6} expected={:.6} |diff|={:.2e}",
                    f[i],
                    v[i].magnitude(),
                    expected,
                    (v[i].magnitude() - expected).abs()
                );
            }
        }
        Err(e) => println!("  AC Err({e})"),
    }
    println!();
}
