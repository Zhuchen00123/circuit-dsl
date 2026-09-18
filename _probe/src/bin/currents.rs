//! Which devices does Thevenin report a branch current for?
//!
//! Run with: cargo run --bin currents

use cirq_ir::*;
use thevenin::circuit::{simulate_ac, simulate_op, simulate_tran};

fn net(id: u32, name: &str) -> Net {
    Net {
        id: Id(id),
        name: name.to_string(),
        is_global: false,
    }
}

fn conn(t: &str, n: u32) -> Connection {
    Connection {
        terminal: t.to_string(),
        net: Id(n),
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

fn r(id: u32, name: &str, p: u32, n: u32, v: f64) -> Element {
    Element {
        id: Id(id),
        name: name.to_string(),
        kind: ElementKind::Resistor,
        connections: vec![conn("pos", p), conn("neg", n)],
        params: vec![("value".to_string(), Value::Real(v))],
        model: None,
        source_spec: None,
    }
}
fn c_(id: u32, name: &str, p: u32, n: u32, v: f64) -> Element {
    Element {
        id: Id(id),
        name: name.to_string(),
        kind: ElementKind::Capacitor,
        connections: vec![conn("pos", p), conn("neg", n)],
        params: vec![("value".to_string(), Value::Real(v))],
        model: None,
        source_spec: None,
    }
}
fn l_(id: u32, name: &str, p: u32, n: u32, v: f64) -> Element {
    Element {
        id: Id(id),
        name: name.to_string(),
        kind: ElementKind::Inductor,
        connections: vec![conn("pos", p), conn("neg", n)],
        params: vec![("value".to_string(), Value::Real(v))],
        model: None,
        source_spec: None,
    }
}
fn vsrc(id: u32, name: &str, p: u32, n: u32, dc: f64, ac: Option<f64>) -> Element {
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

fn show(tag: &str, r: &thevenin_types::SimResult) {
    for p in &r.plots {
        let names: Vec<_> = p.vecs.iter().map(|v| v.name.clone()).collect();
        println!("  {tag} plot '{}': {names:?}", p.name);
    }
}

fn main() {
    println!("=== which branch currents does the engine report? ===\n");

    // R + L + C + V all in one series loop, so every device carries current.
    let mut cir = base(
        "all",
        vec![net(0, "gnd"), net(1, "a"), net(2, "b"), net(3, "c")],
    );
    cir.elements.push(vsrc(0, "v1", 1, 0, 1.0, Some(1.0)));
    cir.elements.push(r(1, "r1", 1, 2, 100.0));
    cir.elements.push(l_(2, "l1", 2, 3, 1e-3));
    cir.elements.push(c_(3, "c1", 3, 0, 1e-6));
    cir.analyses.push(Analysis::Op);
    println!("[OP]");
    show("op", &simulate_op(&cir).expect("op"));

    cir.analyses.clear();
    cir.analyses.push(Analysis::Ac(AcAnalysis {
        start: 100.0,
        stop: 1e4,
        points: 5,
        scale: FrequencyScale::Decade,
    }));
    println!("[AC]");
    show("ac", &simulate_ac(&cir).expect("ac"));

    cir.analyses.clear();
    cir.analyses.push(Analysis::Tran(TranAnalysis {
        step: 1e-6,
        stop: 1e-3,
        start: 0.0,
        uic: false,
        tmax: Some(1e-6),
    }));
    println!("[TRAN]");
    show("tran", &simulate_tran(&cir).expect("tran"));

    println!("\n=== does the engine honour `save`? ===");
    let mut cir2 = base("save", vec![net(0, "gnd"), net(1, "a"), net(2, "b")]);
    cir2.elements.push(vsrc(0, "v1", 1, 0, 1.0, None));
    cir2.elements.push(r(1, "r1", 1, 2, 100.0));
    cir2.elements.push(r(2, "r2", 2, 0, 100.0));
    cir2.save = vec!["v(b)".to_string()];
    cir2.analyses.push(Analysis::Op);
    show("op(save=v(b))", &simulate_op(&cir2).expect("op"));
}
