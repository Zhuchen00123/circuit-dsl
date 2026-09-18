//! End-to-end tests for the Thevenin adapter.
//!
//! These build the project's IR **directly in Rust** and run it through the
//! real engine, then compare against closed-form answers. They are the
//! evidence behind the claims in `docs/backend-evaluation.md`, and they double
//! as regression tests for the five engine behaviours documented at the top of
//! `crates/circuit-backend/src/thevenin.rs`.

use std::collections::HashMap;

use circuit_backend::backend::SimulationBackend;
use circuit_backend::thevenin::TheveninBackend;
use circuit_core::diagnostic::Code;
use circuit_core::ir::{
    AcSpec as IrAcSpec, Circuit, Device, DeviceKind, Model, ModelKind, Node, NodeKind, SourceSpec,
    Waveform, terminal,
};
use circuit_core::plan::{
    AcSweep, AnalysisKind, AnalysisPlan, AnalysisTask, NamedProbe, Probe, Sweep, SweepKind,
    SweepTarget, TranSpec,
};
use circuit_core::span::SourceSpan;
use circuit_core::units::{self, Quantity};
use circuit_core::{AnalysisId, CircuitId, DeviceId, Limits, ModelId, NodeId};
use circuit_results::dataset::{Axis, Data, Dataset};

// ---------------------------------------------------------------------------
// IR builders
// ---------------------------------------------------------------------------

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

fn source(id: u32, name: &str, p: u32, n: u32, spec: SourceSpec) -> Device {
    let mut d = two_terminal(id, name, DeviceKind::VoltageSource, p, n, None);
    d.source = Some(spec);
    d
}

fn dc(v: f64) -> SourceSpec {
    SourceSpec {
        dc: Some(Quantity::volts(v)),
        ..Default::default()
    }
}

fn ac_source(dc_value: f64, mag: f64) -> SourceSpec {
    SourceSpec {
        dc: Some(Quantity::volts(dc_value)),
        ac: Some(IrAcSpec {
            magnitude: Quantity::volts(mag),
            phase_rad: 0.0,
        }),
        waveform: None,
    }
}

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

fn probe(name: &str, p: Probe) -> NamedProbe {
    NamedProbe {
        name: name.to_string(),
        probe: p,
        span: SourceSpan::synthetic(),
    }
}

fn v(node: u32) -> NamedProbe {
    probe(&format!("v(n{node})"), Probe::NodeVoltage(NodeId(node)))
}

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

fn be() -> TheveninBackend {
    TheveninBackend::with_limits(Limits::default())
}

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

fn complex(d: &Dataset, signal: &str) -> Vec<circuit_results::Complex> {
    match &d
        .signal(signal)
        .unwrap_or_else(|| panic!("signal {signal}"))
        .data
    {
        Data::Complex(v) => v.clone(),
        Data::Real(v) => v
            .iter()
            .map(|x| circuit_results::Complex::new(*x, 0.0))
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// 1. Operating point, and the current-direction convention
// ---------------------------------------------------------------------------

/// The divider from the brief: 1 V across 1k + 2k gives 2/3 V at the tap.
///
/// Also the sign check the brief demands: current is positive in the
/// `p -> n` direction. Through `r1` (in -> mid) it must be **positive**
/// +1/3 mA; through the source (in -> gnd), whose current leaves the positive
/// terminal into the circuit, it must be **negative** -1/3 mA.
#[test]
fn divider_op_and_current_direction() {
    let c = circuit(
        "divider",
        vec![node(0, "gnd"), node(1, "in"), node(2, "mid")],
        vec![
            source(0, "v1", 1, 0, dc(1.0)),
            two_terminal(
                1,
                "r1",
                DeviceKind::Resistor,
                1,
                2,
                Some(Quantity::ohms(1000.0)),
            ),
            two_terminal(
                2,
                "r2",
                DeviceKind::Resistor,
                2,
                0,
                Some(Quantity::ohms(2000.0)),
            ),
        ],
    );

    let plan = plan_for(
        "divider",
        AnalysisKind::Op,
        vec![
            v(1),
            v(2),
            probe("i(r1)", Probe::DeviceCurrent(DeviceId(1))),
            probe("i(v1)", Probe::DeviceCurrent(DeviceId(0))),
        ],
    );

    let out = be().run(&c, &plan).expect("op runs");
    let d = &out.datasets[0];
    assert_eq!(d.kind, "op");
    assert!(matches!(d.axis, Axis::None), "an OP has no axis");

    let vmid = real(d, "v(n2)")[0];
    let vin = real(d, "v(n1)")[0];
    assert!((vmid - 2.0 / 3.0).abs() < 1e-12, "v(mid) = {vmid}");
    assert!((vin - 1.0).abs() < 1e-12, "v(in) = {vin}");

    let i_r1 = real(d, "i(r1)")[0];
    let i_v1 = real(d, "i(v1)")[0];
    assert!(
        i_r1 > 0.0,
        "current through r1 flows in -> mid, so p->n is positive; got {i_r1}"
    );
    assert!((i_r1 - 1.0 / 3000.0).abs() < 1e-12, "i(r1) = {i_r1}");
    assert!(
        i_v1 < 0.0,
        "the source delivers current out of its positive terminal, so p->n is negative; got {i_v1}"
    );
    assert!((i_v1 + 1.0 / 3000.0).abs() < 1e-12, "i(v1) = {i_v1}");

    // Charge conservation: the current leaving the source equals the current
    // entering the resistive branch.
    assert!(
        (i_r1 + i_v1).abs() < 1e-12,
        "KCL violated: i(r1)={i_r1}, i(v1)={i_v1}"
    );
}

// ---------------------------------------------------------------------------
// 2. Transient — and the plot-selection regression
// ---------------------------------------------------------------------------

/// An RC step, compared against `v(t) = 1 - exp(-t/tau)`.
///
/// This also pins down Phase-0 finding 1: `simulate_tran` returns `[op1,
/// tran1]`, so an adapter that took `plots[0]` would report the operating
/// point. If that regression returned, the axis here would not be a time axis
/// and the length check would fail.
#[test]
fn rc_transient_matches_analytic() {
    const R: f64 = 1_000.0;
    const C: f64 = 100e-9;
    let tau = R * C;

    let mut src = source(
        0,
        "v1",
        1,
        0,
        SourceSpec {
            dc: Some(Quantity::volts(0.0)),
            ac: None,
            waveform: Some(Waveform::Pulse {
                low: Quantity::volts(0.0),
                high: Quantity::volts(1.0),
                delay: Quantity::seconds(0.0),
                rise: Quantity::seconds(1e-12),
                fall: Quantity::seconds(1e-12),
                width: Quantity::seconds(10.0),
                period: Quantity::seconds(20.0),
            }),
        },
    );
    // A pulse with no DC would leave the OP undefined; give it one.
    src.source.as_mut().unwrap().dc = Some(Quantity::volts(0.0));

    let c = circuit(
        "rc",
        vec![node(0, "gnd"), node(1, "in"), node(2, "out")],
        vec![
            src,
            two_terminal(1, "r1", DeviceKind::Resistor, 1, 2, Some(Quantity::ohms(R))),
            two_terminal(
                2,
                "c1",
                DeviceKind::Capacitor,
                2,
                0,
                Some(Quantity::farads(C)),
            ),
        ],
    );

    let plan = plan_for(
        "rc",
        AnalysisKind::Tran(TranSpec {
            start_s: 0.0,
            stop_s: 5.0 * tau,
            max_step: Some(tau / 200.0),
            output_interval: Some(tau / 200.0),
            uic: false,
            span: SourceSpan::synthetic(),
        }),
        vec![
            probe("v(out)", Probe::NodeVoltage(NodeId(2))),
            probe("i(v1)", Probe::DeviceCurrent(DeviceId(0))),
        ],
    );

    let out = be().run(&c, &plan).expect("tran runs");
    let d = &out.datasets[0];
    let t = match &d.axis {
        Axis::Time(t) => t.clone(),
        other => panic!("expected a time axis, got {other:?}"),
    };
    let vout = real(d, "v(out)");
    assert_eq!(t.len(), vout.len());
    assert!(
        t.len() > 100,
        "expected a resolved transient, got {} points",
        t.len()
    );
    assert!(t[0] == 0.0);
    assert!((t[t.len() - 1] - 5.0 * tau).abs() < 1e-9);

    // Sample near several multiples of tau and compare with the closed form.
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
        let got = vout[idx];
        assert!(
            (got - expected).abs() < 1e-2,
            "at t={:.3}us ({frac}tau): got {got}, expected {expected}",
            t[idx] * 1e6
        );
    }

    // The output axis is solver-chosen: it is non-uniform, which is why
    // measurements must integrate rather than average samples.
    let mut non_uniform = false;
    for w in t.windows(3) {
        if ((w[1] - w[0]) - (w[2] - w[1])).abs() > 1e-12 {
            non_uniform = true;
            break;
        }
    }
    assert!(non_uniform, "expected a non-uniform solver time axis");

    // The source current must be reported for the transient too.
    assert_eq!(real(d, "i(v1)").len(), t.len());
}

// ---------------------------------------------------------------------------
// 3. AC
// ---------------------------------------------------------------------------

/// RC low-pass: `H(jw) = 1 / (1 + jwRC)`, compared in magnitude and phase.
#[test]
fn rc_ac_matches_analytic() {
    const R: f64 = 1_000.0;
    const C: f64 = 100e-9;
    let fc = 1.0 / (2.0 * std::f64::consts::PI * R * C);

    let c = circuit(
        "rc_ac",
        vec![node(0, "gnd"), node(1, "in"), node(2, "out")],
        vec![
            source(0, "v1", 1, 0, ac_source(0.0, 1.0)),
            two_terminal(1, "r1", DeviceKind::Resistor, 1, 2, Some(Quantity::ohms(R))),
            two_terminal(
                2,
                "c1",
                DeviceKind::Capacitor,
                2,
                0,
                Some(Quantity::farads(C)),
            ),
        ],
    );

    let plan = plan_for(
        "rc_ac",
        AnalysisKind::Ac(AcSweep {
            start_hz: 10.0,
            stop_hz: 10e6,
            points: 10,
            kind: SweepKind::Decade,
            span: SourceSpan::synthetic(),
        }),
        vec![probe("v(out)", Probe::NodeVoltage(NodeId(2)))],
    );

    let out = be().run(&c, &plan).expect("ac runs");
    let d = &out.datasets[0];
    let f = match &d.axis {
        Axis::Frequency(f) => f.clone(),
        other => panic!("expected a frequency axis, got {other:?}"),
    };
    let h = complex(d, "v(out)");
    assert_eq!(f.len(), h.len());
    assert_eq!(f.len(), 61, "10 points/decade over 6 decades -> 61 points");

    for target in [fc / 10.0, fc, fc * 10.0] {
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
        let (a, b) = (1.0_f64, w * R * C);
        let den = a * a + b * b;
        let (re, im) = (a / den, -b / den);
        let err = ((h[idx].re - re).powi(2) + (h[idx].im - im).powi(2)).sqrt();
        assert!(
            err < 1e-9,
            "at f={} Hz: got {}+{}j, expected {re}+{im}j",
            f[idx],
            h[idx].re,
            h[idx].im
        );
    }

    // A decade above the corner the response must be about -20 dB.
    let idx = f
        .iter()
        .enumerate()
        .min_by(|a, b| {
            (a.1 - fc * 10.0)
                .abs()
                .partial_cmp(&(b.1 - fc * 10.0).abs())
                .unwrap()
        })
        .map(|(i, _)| i)
        .unwrap();
    let db = 20.0 * h[idx].magnitude().log10();
    assert!((db + 20.0).abs() < 0.5, "expected about -20 dB, got {db}");
}

/// Series RLC, output across the capacitor:
/// `H(jw) = 1 / (1 - w^2 LC + jwRC)`.
#[test]
fn rlc_ac_matches_analytic() {
    const R: f64 = 100.0;
    const L: f64 = 10e-3;
    const C: f64 = 100e-9;
    let f0 = 1.0 / (2.0 * std::f64::consts::PI * (L * C).sqrt());

    let c = circuit(
        "rlc",
        vec![
            node(0, "gnd"),
            node(1, "in"),
            node(2, "mid"),
            node(3, "out"),
        ],
        vec![
            source(0, "v1", 1, 0, ac_source(0.0, 1.0)),
            two_terminal(1, "r1", DeviceKind::Resistor, 1, 2, Some(Quantity::ohms(R))),
            two_terminal(
                2,
                "l1",
                DeviceKind::Inductor,
                2,
                3,
                Some(Quantity::henries(L)),
            ),
            two_terminal(
                3,
                "c1",
                DeviceKind::Capacitor,
                3,
                0,
                Some(Quantity::farads(C)),
            ),
        ],
    );

    let plan = plan_for(
        "rlc",
        AnalysisKind::Ac(AcSweep {
            start_hz: 100.0,
            stop_hz: 1e6,
            points: 20,
            kind: SweepKind::Decade,
            span: SourceSpan::synthetic(),
        }),
        vec![probe("v(out)", Probe::NodeVoltage(NodeId(3)))],
    );

    let out = be().run(&c, &plan).expect("ac runs");
    let d = &out.datasets[0];
    let f = match &d.axis {
        Axis::Frequency(f) => f.clone(),
        other => panic!("expected a frequency axis, got {other:?}"),
    };
    let h = complex(d, "v(out)");

    for target in [f0 / 5.0, f0, f0 * 5.0] {
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
        let (a, b) = (1.0 - w * w * L * C, w * R * C);
        let den = a * a + b * b;
        let (re, im) = (a / den, -b / den);
        let err = ((h[idx].re - re).powi(2) + (h[idx].im - im).powi(2)).sqrt();
        assert!(err < 1e-9, "at f={} Hz: error {err}", f[idx]);
    }

    // At resonance the capacitor voltage is Q times the input, i.e. > 1.
    let idx = f
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - f0).abs().partial_cmp(&(b.1 - f0).abs()).unwrap())
        .map(|(i, _)| i)
        .unwrap();
    assert!(
        h[idx].magnitude() > 1.0,
        "expected resonant peaking, got {}",
        h[idx].magnitude()
    );
}

/// A differential probe is `v(a) - v(b)`, including in the complex domain.
#[test]
fn differential_probe_subtracts_complex_signals() {
    let c = circuit(
        "diff",
        vec![node(0, "gnd"), node(1, "in"), node(2, "a"), node(3, "b")],
        vec![
            source(0, "v1", 1, 0, ac_source(0.0, 1.0)),
            two_terminal(
                1,
                "r1",
                DeviceKind::Resistor,
                1,
                2,
                Some(Quantity::ohms(1000.0)),
            ),
            two_terminal(
                2,
                "r2",
                DeviceKind::Resistor,
                2,
                3,
                Some(Quantity::ohms(1000.0)),
            ),
            two_terminal(
                3,
                "c1",
                DeviceKind::Capacitor,
                3,
                0,
                Some(Quantity::farads(100e-9)),
            ),
        ],
    );

    let plan = plan_for(
        "diff",
        AnalysisKind::Ac(AcSweep {
            start_hz: 1e3,
            stop_hz: 1e5,
            points: 5,
            kind: SweepKind::Decade,
            span: SourceSpan::synthetic(),
        }),
        vec![
            probe("v(a)", Probe::NodeVoltage(NodeId(2))),
            probe("v(b)", Probe::NodeVoltage(NodeId(3))),
            probe(
                "v(a,b)",
                Probe::DifferentialVoltage {
                    pos: NodeId(2),
                    neg: NodeId(3),
                },
            ),
        ],
    );

    let out = be().run(&c, &plan).expect("ac runs");
    let d = &out.datasets[0];
    let va = complex(d, "v(a)");
    let vb = complex(d, "v(b)");
    let vab = complex(d, "v(a,b)");
    assert_eq!(va.len(), vab.len());

    let mut saw_imaginary = false;
    for i in 0..va.len() {
        let expected = circuit_results::Complex::new(va[i].re - vb[i].re, va[i].im - vb[i].im);
        assert!(
            (vab[i].re - expected.re).abs() < 1e-12 && (vab[i].im - expected.im).abs() < 1e-12,
            "at index {i}: got {:?}, expected {expected:?}",
            vab[i]
        );
        if vab[i].im.abs() > 1e-12 {
            saw_imaginary = true;
        }
    }
    assert!(
        saw_imaginary,
        "the test is only meaningful if the result is genuinely complex"
    );
}

/// An AC source phase written in radians must reach the engine in degrees.
///
/// The engine stores `AcSpec::phase` in degrees (`thevenin/src/mna_ir.rs` does
/// `ac.phase * PI / 180.0`), while the project's IR stores radians, so the
/// adapter converts. Until the language gains syntax for setting a phase this
/// is the only check that the conversion happens at all, so it drives the IR
/// directly.
#[test]
fn ac_phase_is_converted_from_radians_to_degrees() {
    // A source at +90 degrees drives a resistive divider, so v(out) must land
    // on the positive imaginary axis rather than the real one.
    let mut src = source(0, "v1", 1, 0, dc(0.0));
    src.source = Some(SourceSpec {
        dc: Some(Quantity::volts(0.0)),
        ac: Some(IrAcSpec {
            magnitude: Quantity::volts(1.0),
            phase_rad: std::f64::consts::FRAC_PI_2,
        }),
        waveform: None,
    });

    let c = circuit(
        "phase",
        vec![node(0, "gnd"), node(1, "in"), node(2, "out")],
        vec![
            src,
            two_terminal(
                1,
                "r1",
                DeviceKind::Resistor,
                1,
                2,
                Some(Quantity::ohms(1000.0)),
            ),
            two_terminal(
                2,
                "r2",
                DeviceKind::Resistor,
                2,
                0,
                Some(Quantity::ohms(1000.0)),
            ),
        ],
    );

    let plan = plan_for(
        "phase",
        AnalysisKind::Ac(AcSweep {
            start_hz: 1e3,
            stop_hz: 1e3,
            points: 1,
            kind: SweepKind::LinearPoints,
            span: SourceSpan::synthetic(),
        }),
        vec![probe("v(out)", Probe::NodeVoltage(NodeId(2)))],
    );

    let out = be().run(&c, &plan).expect("ac runs");
    let h = complex(&out.datasets[0], "v(out)");
    let v = h[0];

    // A 90-degree excitation has no real part and a positive imaginary part,
    // scaled by the divider ratio of one half.
    assert!(
        v.re.abs() < 1e-9,
        "expected a purely imaginary result, got re={} im={}",
        v.re,
        v.im
    );
    assert!((v.im - 0.5).abs() < 1e-9, "expected +0.5j, got im={}", v.im);
}

// ---------------------------------------------------------------------------
// 4. Diode
// ---------------------------------------------------------------------------

/// A diode conducting through a 1k resistor, against an independent
/// bisection solution of the Shockley equation.
#[test]
fn diode_op_matches_independent_solution() {
    let mut diode = two_terminal(2, "d1", DeviceKind::Diode, 2, 0, None);
    diode.terminals = vec![
        (terminal::ANODE.to_string(), NodeId(2)),
        (terminal::CATHODE.to_string(), NodeId(0)),
    ];
    diode.model = Some(ModelId(0));

    let mut c = circuit(
        "rectifier",
        vec![node(0, "gnd"), node(1, "in"), node(2, "out")],
        vec![
            source(0, "v1", 1, 0, dc(5.0)),
            two_terminal(
                1,
                "r1",
                DeviceKind::Resistor,
                1,
                2,
                Some(Quantity::ohms(1000.0)),
            ),
            diode,
        ],
    );
    c.models.push(Model {
        id: ModelId(0),
        name: "dmod".to_string(),
        kind: ModelKind::Diode,
        params: HashMap::from([
            ("is".to_string(), Quantity::amps(1e-14)),
            ("n".to_string(), Quantity::scalar(1.0)),
        ]),
        span: SourceSpan::synthetic(),
    });

    let plan = plan_for(
        "rectifier",
        AnalysisKind::Op,
        vec![probe("v(out)", Probe::NodeVoltage(NodeId(2)))],
    );

    let out = be().run(&c, &plan).expect("op runs");
    let vd = real(&out.datasets[0], "v(out)")[0];

    // Independent reference: 5 = I*1000 + Vd with I = Is*(exp(Vd/(n*Vt)) - 1),
    // Vt = kT/q at the SPICE default 27 C.
    let vt = 0.025_864_190_383_365_096_f64;
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
    let reference = 0.5 * (lo + hi);
    assert!(
        (vd - reference).abs() < 5e-3,
        "diode drop {vd} vs independent reference {reference}"
    );
    assert!(
        (0.3..0.8).contains(&vd),
        "a forward-biased silicon diode should sit in 0.3..0.8 V, got {vd}"
    );
}

// ---------------------------------------------------------------------------
// 5. DC sweep
// ---------------------------------------------------------------------------

#[test]
fn dc_source_sweep_is_linear_through_a_divider() {
    let c = circuit(
        "sweep",
        vec![node(0, "gnd"), node(1, "in"), node(2, "mid")],
        vec![
            source(0, "v1", 1, 0, dc(0.0)),
            two_terminal(
                1,
                "r1",
                DeviceKind::Resistor,
                1,
                2,
                Some(Quantity::ohms(1000.0)),
            ),
            two_terminal(
                2,
                "r2",
                DeviceKind::Resistor,
                2,
                0,
                Some(Quantity::ohms(1000.0)),
            ),
        ],
    );

    let plan = plan_for(
        "sweep",
        AnalysisKind::Dc(circuit_core::plan::DcSpec {
            sweep: Sweep {
                target: SweepTarget::SourceValue {
                    device: DeviceId(0),
                    name: "v1".to_string(),
                },
                dimension: units::VOLTAGE,
                start: 0.0,
                stop: 5.0,
                step: Some(1.0),
                points: None,
                kind: SweepKind::Linear,
                include_endpoint: true,
                span: SourceSpan::synthetic(),
            },
        }),
        vec![probe("v(mid)", Probe::NodeVoltage(NodeId(2)))],
    );

    let out = be().run(&c, &plan).expect("dc runs");
    let d = &out.datasets[0];
    let sweep = match &d.axis {
        Axis::Parameter(v) => v.clone(),
        other => panic!("expected a parameter axis, got {other:?}"),
    };
    let vmid = real(d, "v(mid)");
    assert_eq!(sweep.len(), 6);
    assert_eq!(vmid.len(), 6);

    for i in 0..6 {
        let expected = sweep[i] * 0.5;
        assert!(
            (vmid[i] - expected).abs() < 1e-12,
            "at sweep index {i}: v(mid)={}, expected {expected}",
            vmid[i]
        );
    }
}

// ---------------------------------------------------------------------------
// Regressions found by auditing the finished tool against its own brief
// ---------------------------------------------------------------------------

/// A resistor with a grounded terminal must still have a usable current.
///
/// The derivation needs `v(gnd)`, which the engine never emits because ground
/// is the reference. Every example in the repo happened to probe a resistor
/// with *both* terminals off ground, so this path was never exercised until
/// an audit asked for `i(r)` on a resistor to ground and got a backend error.
#[test]
fn resistor_current_is_derivable_with_a_grounded_terminal() {
    // 5 V across 1k to ground: i(r1) must be +5 mA in the p -> n direction.
    let c = circuit(
        "grounded",
        vec![node(0, "gnd"), node(1, "in")],
        vec![
            source(0, "v1", 1, 0, dc(5.0)),
            two_terminal(
                1,
                "r1",
                DeviceKind::Resistor,
                1,
                0,
                Some(Quantity::ohms(1000.0)),
            ),
        ],
    );
    let plan = plan_for(
        "grounded",
        AnalysisKind::Op,
        vec![
            probe("i(r1)", Probe::DeviceCurrent(DeviceId(1))),
            probe("i(v1)", Probe::DeviceCurrent(DeviceId(0))),
        ],
    );
    let out = be()
        .run(&c, &plan)
        .expect("a resistor to ground has a current");
    let d = &out.datasets[0];
    assert!(
        (real(d, "i(r1)")[0] - 5e-3).abs() < 1e-12,
        "i(r1) = {:?}",
        real(d, "i(r1)")
    );
    // KCL again: the source delivers what the resistor absorbs.
    assert!((real(d, "i(r1)")[0] + real(d, "i(v1)")[0]).abs() < 1e-12);
}

/// The same derivation in the complex domain, with a grounded terminal.
#[test]
fn grounded_resistor_current_is_derivable_in_ac() {
    const R: f64 = 1_000.0;
    let c = circuit(
        "grounded_ac",
        vec![node(0, "gnd"), node(1, "in")],
        vec![
            source(0, "v1", 1, 0, ac_source(0.0, 1.0)),
            two_terminal(1, "r1", DeviceKind::Resistor, 1, 0, Some(Quantity::ohms(R))),
        ],
    );
    let plan = plan_for(
        "grounded_ac",
        AnalysisKind::Ac(AcSweep {
            start_hz: 1e3,
            stop_hz: 1e5,
            points: 3,
            kind: SweepKind::Decade,
            span: SourceSpan::synthetic(),
        }),
        vec![probe("i(r1)", Probe::DeviceCurrent(DeviceId(1)))],
    );
    let out = be().run(&c, &plan).expect("ac runs");
    let i = complex(&out.datasets[0], "i(r1)");
    // v(in) = 1 + 0j across 1k to ground, so i(r1) = 1 mA with no phase.
    for sample in &i {
        assert!((sample.re - 1e-3).abs() < 1e-12, "{sample:?}");
        assert!(sample.im.abs() < 1e-12, "{sample:?}");
    }
}

/// Two analyses of the same kind must not collide.
///
/// Every plot from the engine is named `<kind>1`, because each task runs as a
/// circuit with exactly one analysis. Naming the dataset from that made two
/// `ac` tasks both `ac1`, and the CLI wrote both to `all.ac1.csv` — the second
/// silently replacing the first, which the brief forbids ("results are
/// distinguished by analysis_id and must not overwrite each other").
#[test]
fn two_analyses_of_the_same_kind_get_distinct_names() {
    let c = circuit(
        "two_ac",
        vec![node(0, "gnd"), node(1, "in"), node(2, "out")],
        vec![
            source(0, "v1", 1, 0, ac_source(0.0, 1.0)),
            two_terminal(
                1,
                "r1",
                DeviceKind::Resistor,
                1,
                2,
                Some(Quantity::ohms(1000.0)),
            ),
            two_terminal(
                2,
                "c1",
                DeviceKind::Capacitor,
                2,
                0,
                Some(Quantity::farads(100e-9)),
            ),
        ],
    );

    let mut plan = plan_for(
        "two_ac",
        AnalysisKind::Ac(AcSweep {
            start_hz: 10.0,
            stop_hz: 1e6,
            points: 10,
            kind: SweepKind::Decade,
            span: SourceSpan::synthetic(),
        }),
        vec![probe("v(out)", Probe::NodeVoltage(NodeId(2)))],
    );
    // A second AC task, this time linear.
    plan.tasks.push(AnalysisTask {
        id: AnalysisId(1),
        kind: AnalysisKind::Ac(AcSweep {
            start_hz: 10.0,
            stop_hz: 1e6,
            points: 11,
            kind: SweepKind::LinearPoints,
            span: SourceSpan::synthetic(),
        }),
        probes: plan.tasks[0].probes.clone(),
        implicit_probes: Vec::new(),
        span: SourceSpan::synthetic(),
    });

    let out = be().run(&c, &plan).expect("both ac analyses run");
    assert_eq!(out.datasets.len(), 2, "both results must survive");

    let names: Vec<&str> = out.datasets.iter().map(|d| d.analysis.as_str()).collect();
    assert_eq!(names, vec!["ac1", "ac2"], "names must be distinct");

    // And the two must be genuinely different sweeps, not the same one twice.
    let counts: Vec<usize> = out
        .datasets
        .iter()
        .map(|d| match &d.axis {
            Axis::Frequency(f) => f.len(),
            other => panic!("expected a frequency axis, got {other:?}"),
        })
        .collect();
    // 10 Hz to 1 MHz is 5 decades; at 10 points/decade that is 50 intervals.
    // The linear sweep asks for 11 samples in total, and the engine gives 11.
    assert_eq!(counts, vec![51, 11], "distinct sweeps, not one repeated");
}

/// One bad probe must produce one diagnostic, not one per analysis.
///
/// `save` is replicated onto every task, so validating each task's probes
/// reported the same mistake once per analysis — five times for the audit
/// circuit that found this.
#[test]
fn a_bad_probe_is_reported_once_not_once_per_analysis() {
    let c = circuit(
        "dup",
        vec![node(0, "gnd"), node(1, "in")],
        vec![
            source(0, "v1", 1, 0, dc(1.0)),
            two_terminal(
                1,
                "c1",
                DeviceKind::Capacitor,
                1,
                0,
                Some(Quantity::farads(1e-6)),
            ),
        ],
    );

    // A capacitor current is unavailable; attach the same probe to 3 tasks.
    let bad = probe("i(c1)", Probe::DeviceCurrent(DeviceId(1)));
    let mut plan = plan_for("dup", AnalysisKind::Op, vec![bad]);
    plan.tasks.push(AnalysisTask {
        id: AnalysisId(1),
        kind: AnalysisKind::Op,
        probes: plan.tasks[0].probes.clone(),
        implicit_probes: Vec::new(),
        span: SourceSpan::synthetic(),
    });
    plan.tasks.push(AnalysisTask {
        id: AnalysisId(2),
        kind: AnalysisKind::Op,
        probes: plan.tasks[0].probes.clone(),
        implicit_probes: Vec::new(),
        span: SourceSpan::synthetic(),
    });

    let err = be().run(&c, &plan).expect_err("must refuse");
    assert_eq!(
        err.len(),
        1,
        "the same probe must be reported once, got:
{}",
        err.render_plain()
    );
}

// ---------------------------------------------------------------------------
// 6. Probe subsetting (Phase-0 finding 2)
// ---------------------------------------------------------------------------

/// The engine ignores `circuit.save` in the single-analysis entry points, so
/// the adapter must subset. Without that, a run asked for one probe would
/// return every internal vector.
#[test]
fn only_requested_probes_are_returned() {
    let c = circuit(
        "subset",
        vec![node(0, "gnd"), node(1, "in"), node(2, "mid")],
        vec![
            source(0, "v1", 1, 0, dc(1.0)),
            two_terminal(
                1,
                "r1",
                DeviceKind::Resistor,
                1,
                2,
                Some(Quantity::ohms(1000.0)),
            ),
            two_terminal(
                2,
                "r2",
                DeviceKind::Resistor,
                2,
                0,
                Some(Quantity::ohms(1000.0)),
            ),
        ],
    );

    let plan = plan_for(
        "subset",
        AnalysisKind::Op,
        vec![probe("v(mid)", Probe::NodeVoltage(NodeId(2)))],
    );

    let out = be().run(&c, &plan).expect("op runs");
    let d = &out.datasets[0];
    let names: Vec<_> = d.signals.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["v(mid)"],
        "adapter must subset to the probe list"
    );
}

// ---------------------------------------------------------------------------
// 7. Failure reporting
// ---------------------------------------------------------------------------

/// Conflicting ideal sources have no solution. That must surface as an error
/// with the singular-system code, not as an empty success (brief §8.1).
#[test]
fn a_singular_circuit_is_reported_not_swallowed() {
    let c = circuit(
        "conflict",
        vec![node(0, "gnd"), node(1, "a")],
        vec![
            source(0, "v1", 1, 0, dc(1.0)),
            source(1, "v2", 1, 0, dc(2.0)),
        ],
    );
    let plan = plan_for("conflict", AnalysisKind::Op, vec![v(1)]);

    let err = be().run(&c, &plan).expect_err("must not succeed");
    let d = err.iter().next().expect("a diagnostic");
    assert_eq!(d.code, Code::Singular, "{}", err.render_plain());
    let text = err.render_plain();
    // The engine's own words must survive.
    assert!(text.contains("singular"), "{text}");
}

/// A parameter sweep is not something the engine can do in one call, and the
/// adapter must say so rather than quietly sweeping the wrong thing.
#[test]
fn a_parameter_sweep_is_refused_by_the_single_point_executor() {
    let c = circuit(
        "p",
        vec![node(0, "gnd"), node(1, "a")],
        vec![
            source(0, "v1", 1, 0, dc(1.0)),
            two_terminal(
                1,
                "r1",
                DeviceKind::Resistor,
                1,
                0,
                Some(Quantity::ohms(1000.0)),
            ),
        ],
    );
    let plan = plan_for(
        "p",
        AnalysisKind::Dc(circuit_core::plan::DcSpec {
            sweep: Sweep {
                target: SweepTarget::Parameter { name: "r".into() },
                dimension: units::RESISTANCE,
                start: 1.0,
                stop: 10.0,
                step: Some(1.0),
                points: None,
                kind: SweepKind::Linear,
                include_endpoint: true,
                span: SourceSpan::synthetic(),
            },
        }),
        vec![v(1)],
    );

    let err = be().run(&c, &plan).expect_err("must refuse");
    let text = err.render_plain();
    assert_eq!(err.iter().next().unwrap().code, Code::Unsupported);
    assert!(text.contains("parameter sweep"), "{text}");
}

#[test]
fn capabilities_are_declared_honestly() {
    let caps = be().capabilities();
    assert_eq!(caps.name, "thevenin");
    assert_eq!(caps.version, "0.5.0");
    for a in ["op", "dc", "ac", "tran"] {
        assert!(caps.supports_analysis(a), "missing analysis {a}");
    }
    assert!(!caps.supports_analysis("noise"));
    for d in [
        "resistor",
        "capacitor",
        "inductor",
        "voltage_source",
        "current_source",
        "diode",
    ] {
        assert!(caps.supports_device(d), "missing device {d}");
    }
    assert!(!caps.supports_device("mosfet"));
    assert!(
        !caps.parameter_sweep,
        "parameter sweeps are driven outside the backend"
    );
    assert!(caps.source_sweep);
}

// ---------------------------------------------------------------------------
// 8. Units tagging
// ---------------------------------------------------------------------------

#[test]
fn signals_carry_their_units() {
    let c = circuit(
        "units",
        vec![node(0, "gnd"), node(1, "in"), node(2, "mid")],
        vec![
            source(0, "v1", 1, 0, dc(1.0)),
            two_terminal(
                1,
                "r1",
                DeviceKind::Resistor,
                1,
                2,
                Some(Quantity::ohms(1000.0)),
            ),
            two_terminal(
                2,
                "r2",
                DeviceKind::Resistor,
                2,
                0,
                Some(Quantity::ohms(1000.0)),
            ),
        ],
    );
    let plan = plan_for(
        "units",
        AnalysisKind::Op,
        vec![
            probe("v(mid)", Probe::NodeVoltage(NodeId(2))),
            probe("i(r1)", Probe::DeviceCurrent(DeviceId(1))),
        ],
    );
    let out = be().run(&c, &plan).expect("op runs");
    let d = &out.datasets[0];
    assert_eq!(d.signal("v(mid)").unwrap().unit, units::VOLTAGE);
    assert_eq!(d.signal("i(r1)").unwrap().unit, units::CURRENT);
}

#[test]
fn backend_metadata_is_recorded() {
    let c = circuit(
        "meta",
        vec![node(0, "gnd"), node(1, "a")],
        vec![source(0, "v1", 1, 0, dc(1.0))],
    );
    let plan = plan_for("meta", AnalysisKind::Op, vec![v(1)]);
    let out = be().run(&c, &plan).expect("op runs");
    let b = &out.datasets[0].backend;
    assert_eq!(b.name, "thevenin");
    assert_eq!(b.version, "0.5.0");
}

// ---------------------------------------------------------------------------
// 9. Limits of the resistor-current derivation
// ---------------------------------------------------------------------------

/// The derived resistor current must also be right in the complex domain,
/// where it is checked against the engine's own source current.
#[test]
fn derived_resistor_current_agrees_with_source_current_in_ac() {
    const R: f64 = 1_000.0;
    const C: f64 = 100e-9;

    let c = circuit(
        "ac_kcl",
        vec![node(0, "gnd"), node(1, "in"), node(2, "out")],
        vec![
            source(0, "v1", 1, 0, ac_source(0.0, 1.0)),
            two_terminal(1, "r1", DeviceKind::Resistor, 1, 2, Some(Quantity::ohms(R))),
            two_terminal(
                2,
                "c1",
                DeviceKind::Capacitor,
                2,
                0,
                Some(Quantity::farads(C)),
            ),
        ],
    );

    let plan = plan_for(
        "ac_kcl",
        AnalysisKind::Ac(AcSweep {
            start_hz: 10.0,
            stop_hz: 1e6,
            points: 5,
            kind: SweepKind::Decade,
            span: SourceSpan::synthetic(),
        }),
        vec![
            probe("i(r1)", Probe::DeviceCurrent(DeviceId(1))),
            // The source branch current comes straight from the engine, so it
            // is an independent second opinion on the derived value.
            probe("i(v1)", Probe::DeviceCurrent(DeviceId(0))),
        ],
    );

    let out = be().run(&c, &plan).expect("ac runs");
    let d = &out.datasets[0];
    let ir1 = complex(d, "i(r1)");
    let iv1 = complex(d, "i(v1)");
    assert_eq!(ir1.len(), iv1.len());

    let mut saw_imaginary = false;
    for i in 0..ir1.len() {
        // r1 and the source are in series, so the current through r1 in its
        // own `p -> n` direction is the negative of the source's `p -> n`
        // current: the source delivers, r1 absorbs. (Same relation the DC
        // divider test checks.)
        let (er, ei) = (ir1[i].re + iv1[i].re, ir1[i].im + iv1[i].im);
        let err = (er * er + ei * ei).sqrt();
        let scale = iv1[i].magnitude().max(1e-12);
        assert!(
            err / scale < 1e-9,
            "at index {i}: derived i(r1)={:?} should be the negative of engine i(v1)={:?}",
            ir1[i],
            iv1[i]
        );
        if ir1[i].im.abs() > 1e-15 {
            saw_imaginary = true;
        }
    }
    assert!(
        saw_imaginary,
        "the check is only meaningful with a complex current"
    );
}

/// A capacitor current is not available and must be refused with a reason,
/// not silently approximated by differentiating the voltage (brief §8.5).
#[test]
fn capacitor_current_is_refused_with_a_reason() {
    let c = circuit(
        "cap",
        vec![node(0, "gnd"), node(1, "in"), node(2, "out")],
        vec![
            source(0, "v1", 1, 0, dc(1.0)),
            two_terminal(
                1,
                "r1",
                DeviceKind::Resistor,
                1,
                2,
                Some(Quantity::ohms(1000.0)),
            ),
            two_terminal(
                2,
                "c1",
                DeviceKind::Capacitor,
                2,
                0,
                Some(Quantity::farads(1e-6)),
            ),
        ],
    );

    let plan = plan_for(
        "cap",
        AnalysisKind::Op,
        vec![probe("i(c1)", Probe::DeviceCurrent(DeviceId(2)))],
    );

    let err = be().run(&c, &plan).expect_err("must refuse");
    let text = err.render_plain();
    assert_eq!(err.iter().next().unwrap().code, Code::Unsupported);
    assert!(
        text.contains("differentiated"),
        "the reason must be given: {text}"
    );
}

/// A zero-valued resistor is an ideal short, which makes the system singular.
///
/// The language rejects non-positive R/L/C during elaboration, so this cannot
/// normally reach the backend; the point of the test is that if it does, the
/// failure is reported rather than producing an infinite current.
#[test]
fn zero_resistance_does_not_produce_an_infinite_current() {
    let c = circuit(
        "zero",
        vec![node(0, "gnd"), node(1, "a"), node(2, "b")],
        vec![
            source(0, "v1", 1, 0, dc(1.0)),
            two_terminal(
                1,
                "r1",
                DeviceKind::Resistor,
                1,
                2,
                Some(Quantity::ohms(0.0)),
            ),
            two_terminal(
                2,
                "r2",
                DeviceKind::Resistor,
                2,
                0,
                Some(Quantity::ohms(1000.0)),
            ),
        ],
    );
    let plan = plan_for(
        "zero",
        AnalysisKind::Op,
        vec![probe("i(r1)", Probe::DeviceCurrent(DeviceId(1)))],
    );

    // Either an explicit refusal or the engine's singular-system error is
    // acceptable; silently returning an infinity is not.
    let err = be().run(&c, &plan).expect_err("must not succeed");
    let text = err.render_plain();
    assert!(
        text.contains("zero") || text.contains("singular"),
        "unexpected error text: {text}"
    );
    assert!(!text.is_empty());
}
