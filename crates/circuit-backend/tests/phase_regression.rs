//! Phase regressions for the Thevenin adapter.
//!
//! Scope: the adapter's two phase-carrying paths driven with **non-zero**
//! phases, straight through the project IR. The language has no syntax for
//! setting an AC source phase (`crates/circuit-dsl/src/elaborate.rs` builds
//! `AcSpec { phase_rad: 0.0 }`), so an integration test like this one is the
//! only way to exercise the conversion.
//!
//! # Conventions this file pins down
//!
//! Both were read in the pinned engine sources, Thevenin 0.5.0 (the checkout
//! lives under `~/.cargo/registry/src/index.crates.io-*/thevenin-0.5.0`):
//!
//! | path | engine code | convention |
//! |---|---|---|
//! | AC excitation | `src/mna_ir.rs:496-498` | `phase_rad = ac.phase * PI / 180`; `real = mag * cos(phase_rad)`, `imag = mag * sin(phase_rad)` |
//! | SIN waveform | `src/waveform.rs:158-176` (`eval_sin`) | before/at `td`: `v0 + va*sin(phi_rad)`; after `td`: `v0 + va*sin(2*PI*freq*(t-td) + phi_rad) * exp(-theta*(t-td))`; `phi_rad = phi_deg * PI/180` |
//!
//! So the shared convention is **positive phase gives a positive imaginary
//! part / a positive leading `sin` argument**, and the engine takes
//! **degrees** on both paths. The adapter converts the project's radians to
//! those degrees (`crates/circuit-backend/src/thevenin.rs:648-654` for
//! `AcSpec`, `:682-690` for `Sin.phi`), which is what these tests drive
//! end to end. `tests/adapter.rs::ac_phase_is_converted_from_radians_to_degrees`
//! already covers +90 degrees; this file covers other angles, both signs, the
//! real and imaginary parts separately, the RC transfer function and the SIN
//! path.
//!
//! # Thresholds
//!
//! The AC checks apply the brief's §17 target
//! (`|actual - expected| within 1e-8 + 1e-4 * |expected|`) to each component.
//! `LINEAR_STRICT` is an *additional*, tighter bound documenting how far the
//! linear solves actually sit from that target; it never relaxes it.
//!
//! # Provenance
//!
//! The IR builders (`node`, `two_terminal`, `source`, `circuit`,
//! `probe`, `plan_for`, `be`) follow the established style of
//! `crates/circuit-backend/tests/adapter.rs` and are reproduced here so that
//! file stays untouched.

use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_3, FRAC_PI_4, FRAC_PI_6, PI};

use circuit_backend::backend::SimulationBackend;
use circuit_backend::thevenin::TheveninBackend;
use circuit_core::ir::{
    AcSpec as IrAcSpec, Circuit, Device, DeviceKind, Node, NodeKind, SourceSpec, Waveform, terminal,
};
use circuit_core::plan::{
    AcSweep, AnalysisKind, AnalysisPlan, AnalysisTask, NamedProbe, Probe, SweepKind, TranSpec,
};
use circuit_core::span::SourceSpan;
use circuit_core::units::Quantity;
use circuit_core::{AnalysisId, CircuitId, Limits, NodeId};
use circuit_results::dataset::{Axis, Data, Dataset};

// ---------------------------------------------------------------------------
// Thresholds
// ---------------------------------------------------------------------------

/// Brief §17.2 target for an AC voltage component.
const AC_ATOL: f64 = 1e-8;
const AC_RTOL: f64 = 1e-4;

/// Additional bound for the purely linear cases. The observed error is a few
/// 1e-16 (one linear solve), so this leaves nine orders of margin; it is far
/// below the ~5e-5 the brief target permits, and it exists so that a regression
/// which stays inside the coarse target still fails here.
const LINEAR_STRICT: f64 = 1e-9;

/// The brief's criterion, applied per component.
fn assert_component(what: &str, actual: f64, expected: f64) {
    let tol = AC_ATOL + AC_RTOL * expected.abs();
    let err = (actual - expected).abs();
    assert!(
        err <= tol,
        "{what}: got {actual:.17e}, expected {expected:.17e}, |err| = {err:.3e} > tol {tol:.3e}"
    );
}

/// Same comparison against the stricter linear bound.
fn assert_component_strict(what: &str, actual: f64, expected: f64) {
    let err = (actual - expected).abs();
    assert!(
        err <= LINEAR_STRICT,
        "{what}: got {actual:.17e}, expected {expected:.17e}, |err| = {err:.3e} > {LINEAR_STRICT:.3e}"
    );
}

// ---------------------------------------------------------------------------
// Complex arithmetic (small, so the test does not need the results crate)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Cx {
    re: f64,
    im: f64,
}

impl Cx {
    fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }

    /// `magnitude * (cos phi + j sin phi)`.
    fn polar(magnitude: f64, phi_rad: f64) -> Self {
        Self::new(magnitude * phi_rad.cos(), magnitude * phi_rad.sin())
    }

    fn mul(self, other: Self) -> Self {
        Self::new(
            self.re * other.re - self.im * other.im,
            self.re * other.im + self.im * other.re,
        )
    }

    fn div(self, other: Self) -> Self {
        let den = other.re * other.re + other.im * other.im;
        Self::new(
            (self.re * other.re + self.im * other.im) / den,
            (self.im * other.re - self.re * other.im) / den,
        )
    }

    fn magnitude(self) -> f64 {
        self.re.hypot(self.im)
    }

    fn phase_rad(self) -> f64 {
        self.im.atan2(self.re)
    }

    fn distance(self, other: Self) -> f64 {
        (self.re - other.re).hypot(self.im - other.im)
    }
}

// ---------------------------------------------------------------------------
// IR builders (see the provenance note at the top of the file)
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
        id: circuit_core::DeviceId(id),
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

/// A voltage source with an explicit DC value, an optional AC magnitude/phase
/// and an optional transient waveform.
fn source(id: u32, name: &str, p: u32, n: u32, spec: SourceSpec) -> Device {
    let mut d = two_terminal(id, name, DeviceKind::VoltageSource, p, n, None);
    d.source = Some(spec);
    d
}

/// `dc: 0 V` plus an AC magnitude and **non-zero** phase in radians: exactly
/// the shape the elaborator would produce if the language had phase syntax.
fn ac_spec(phase_rad: f64) -> SourceSpec {
    SourceSpec {
        dc: Some(Quantity::volts(0.0)),
        ac: Some(IrAcSpec {
            magnitude: Quantity::volts(1.0),
            phase_rad,
        }),
        waveform: None,
    }
}

/// The DC value is pinned to 0 V: the engine's operating point prefers an
/// explicit `dc` over the waveform (`thevenin-0.5.0/src/mna_ir.rs:388-412`,
/// MODEDCOP), so this keeps the initial state at zero independently of the
/// waveform offset. `sin_waveform_with_nonzero_phase_matches_engine_formula`
/// pins that precedence with a second assertion.
fn sin_spec(
    offset: f64,
    amplitude: f64,
    frequency_hz: f64,
    delay_s: f64,
    phase_rad: f64,
) -> SourceSpec {
    SourceSpec {
        dc: Some(Quantity::volts(0.0)),
        ac: None,
        waveform: Some(Waveform::Sin {
            offset: Quantity::volts(offset),
            amplitude: Quantity::volts(amplitude),
            frequency: Quantity::hertz(frequency_hz),
            delay: Quantity::seconds(delay_s),
            damping: Quantity::scalar(0.0),
            phase_rad,
        }),
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

fn complex(d: &Dataset, signal: &str) -> Vec<Cx> {
    match &d
        .signal(signal)
        .unwrap_or_else(|| panic!("signal {signal}"))
        .data
    {
        Data::Complex(v) => v.iter().map(|c| Cx::new(c.re, c.im)).collect(),
        Data::Real(v) => v.iter().map(|x| Cx::new(*x, 0.0)).collect(),
    }
}

/// A single-frequency AC run of `circuit`, returning `v(in)` and `v(out)`.
fn run_ac_single_point(c: &Circuit, name: &str, frequency_hz: f64) -> (Cx, Cx, f64) {
    let plan = plan_for(
        name,
        AnalysisKind::Ac(AcSweep {
            start_hz: frequency_hz,
            stop_hz: frequency_hz,
            points: 1,
            kind: SweepKind::LinearPoints,
            span: SourceSpan::synthetic(),
        }),
        vec![
            probe("v(in)", Probe::NodeVoltage(NodeId(1))),
            probe("v(out)", Probe::NodeVoltage(NodeId(2))),
        ],
    );

    let out = be().run(c, &plan).expect("ac runs");
    let d = &out.datasets[0];
    let f = match &d.axis {
        Axis::Frequency(f) => f.clone(),
        other => panic!("expected a frequency axis, got {other:?}"),
    };
    assert_eq!(f.len(), 1, "single-point sweep returned {} points", f.len());
    (complex(d, "v(in)")[0], complex(d, "v(out)")[0], f[0])
}

/// The 1 k / 1 k resistive divider used by the AC tests. Its transfer function
/// is frequency independent: `v(out) = 0.5 * v(in)`.
fn resistive_divider(phase_rad: f64) -> Circuit {
    circuit(
        "divider_phase",
        vec![node(0, "gnd"), node(1, "in"), node(2, "out")],
        vec![
            source(0, "v1", 1, 0, ac_spec(phase_rad)),
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
    )
}

/// The 1 k / 1 k divider driven by a SIN source: used for the transient phase
/// tests. No energy storage, so `v(out) = 0.5 * v_in(t)` at every instant.
fn sin_divider(spec: SourceSpec) -> Circuit {
    circuit(
        "sin_phase",
        vec![node(0, "gnd"), node(1, "in"), node(2, "out")],
        vec![
            source(0, "v1", 1, 0, spec),
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
    )
}

// ---------------------------------------------------------------------------
// 1. AC: several angles, both signs, real and imaginary parts
// ---------------------------------------------------------------------------

/// Positive phase means a positive imaginary part; negative phase a negative
/// one. Covers +30, +60 and -45 degrees, and asserts both components of
/// `v(in)` and `v(out)`, so a swapped real/imaginary pair, a lost sign or a
/// lost radian-to-degree conversion cannot pass.
#[test]
fn ac_phase_sign_convention_across_angles() {
    // (human-readable angle, radians as the IR stores them)
    let angles = [
        (30.0_f64, FRAC_PI_6),
        (60.0, FRAC_PI_3),
        (-45.0, -FRAC_PI_4),
    ];

    for (degrees, phi) in angles {
        let c = resistive_divider(phi);
        let (vin, vout, f) = run_ac_single_point(&c, "divider_phase", 1e3);
        let expected_in = Cx::polar(1.0, phi);
        let expected_out = Cx::polar(0.5, phi);

        println!(
            "[ac] phi = {degrees:+.1} deg ({phi:+.12} rad) at f = {f:.12e} Hz: v(in) = ({:.17e}, {:.17e}), v(out) = ({:.17e}, {:.17e})",
            vin.re, vin.im, vout.re, vout.im
        );

        // Brief §17 target, per component.
        assert_component("v(in).re", vin.re, expected_in.re);
        assert_component("v(in).im", vin.im, expected_in.im);
        assert_component("v(out).re", vout.re, expected_out.re);
        assert_component("v(out).im", vout.im, expected_out.im);

        // The linear solve is far closer than the target; see LINEAR_STRICT.
        assert_component_strict("v(in).re", vin.re, expected_in.re);
        assert_component_strict("v(in).im", vin.im, expected_in.im);
        assert_component_strict("v(out).re", vout.re, expected_out.re);
        assert_component_strict("v(out).im", vout.im, expected_out.im);

        // --- the sign convention, stated directly ---
        if phi > 0.0 {
            assert!(
                vin.im > 0.0 && vout.im > 0.0,
                "a positive phase must give a positive imaginary part: phi = {degrees:+.1} deg, im(v(in)) = {}, im(v(out)) = {}",
                vin.im,
                vout.im
            );
        } else {
            assert!(
                vin.im < 0.0 && vout.im < 0.0,
                "a negative phase must give a negative imaginary part: phi = {degrees:+.1} deg, im(v(in)) = {}, im(v(out)) = {}",
                vin.im,
                vout.im
            );
        }

        // Magnitude is untouched by the phase, and the divider halves it.
        assert!(
            (vin.magnitude() - 1.0).abs() <= LINEAR_STRICT,
            "|v(in)| = {} for phi = {degrees:+.1} deg",
            vin.magnitude()
        );
        let ratio = vout.div(vin);
        assert_component_strict("|v(out)/v(in)|", ratio.magnitude(), 0.5);
        assert!(
            ratio.phase_rad().abs() <= 1e-12,
            "the divider must not add phase: got {} rad",
            ratio.phase_rad()
        );
    }
}

// ---------------------------------------------------------------------------
// 2. AC: the adapter really calls to_degrees
// ---------------------------------------------------------------------------

/// The engine takes AC phase in degrees, the project stores radians, and
/// `map_source` converts (`thevenin.rs:648-654`). This test drives 30 degrees
/// of phase and compares the result with three interpretations:
///
/// * **correct** - the adapter converts, giving `0.5 at 30 deg = (0.4330127, 0.25)`;
/// * **bug "forgot to_degrees"** - the raw `0.5236` is handed to the engine,
///   which reads it as degrees and produces `0.5 at 0.5236 deg`
///   `= (0.49999, 0.0045700)`;
/// * **bug "converted the wrong way"** - degrees were converted to radians
///   again, so the phase collapses towards zero.
///
/// The first is asserted equal, the other two are asserted *far away*, so
/// deleting the conversion cannot pass this test.
#[test]
fn ac_phase_is_converted_from_radians_to_degrees() {
    let phi = FRAC_PI_6; // 30 degrees
    let c = resistive_divider(phi);
    let (vin, vout, _f) = run_ac_single_point(&c, "divider_phase", 1e3);

    let correct = Cx::polar(0.5, phi);
    // What the engine produces if the value reaches it unconverted: it applies
    // "phase * PI / 180" to a number that is already radians.
    let forgot_conversion = Cx::polar(0.5, phi * PI / 180.0);
    // What a double conversion (to_radians on a radian value) would give.
    let double_conversion = Cx::polar(0.5, phi.to_radians() * PI / 180.0);

    println!(
        "[to_degrees] correct = ({:.17e}, {:.17e}); measured v(out) = ({:.17e}, {:.17e})",
        correct.re, correct.im, vout.re, vout.im
    );

    assert_component("v(out).re (converted)", vout.re, correct.re);
    assert_component("v(out).im (converted)", vout.im, correct.im);
    assert_component_strict("v(out).re (converted)", vout.re, correct.re);
    assert_component_strict("v(out).im (converted)", vout.im, correct.im);

    let d_forgot = vout.distance(forgot_conversion);
    let d_double = vout.distance(double_conversion);
    println!(
        "[to_degrees] distance to 'forgot to_degrees' = {d_forgot:.6e}; distance to 'double conversion' = {d_double:.6e}"
    );
    assert!(
        d_forgot > 1e-3,
        "a run that forgot to_degrees would give ({:.6e}, {:.6e}), only {d_forgot:.3e} away; the conversion is not being exercised",
        forgot_conversion.re,
        forgot_conversion.im
    );
    assert!(
        d_double > 1e-3,
        "a run that converted twice would give ({:.6e}, {:.6e}), only {d_double:.3e} away",
        double_conversion.re,
        double_conversion.im
    );
    // The excitation must keep its magnitude; only the angle is in question.
    assert_component_strict("|v(in)|", vin.magnitude(), 1.0);
}

// ---------------------------------------------------------------------------
// 3. AC: a first-order RC low-pass with a non-zero source phase
// ---------------------------------------------------------------------------

/// `v(out) = H(jw) * e^(j phi)` with `H = 1 / (1 + jwRC)`, evaluated at the
/// **frequency the engine actually returned** (brief §17.2 B), for +30 and -45
/// degrees of source phase.
///
/// The brief's AC target is applied to the real and imaginary parts
/// component-wise; because an expected component can be small near a zero
/// crossing, the complex magnitude error is reported and checked against the
/// same target as well. `LINEAR_STRICT` then records how far the linear solve
/// actually is.
#[test]
fn rc_lowpass_with_nonzero_source_phase_matches_analytic() {
    const R: f64 = 1_000.0;
    const C: f64 = 100e-9;
    let fc = 1.0 / (2.0 * PI * R * C);

    for phi in [FRAC_PI_6, -FRAC_PI_4] {
        let c = circuit(
            "rc_phase",
            vec![node(0, "gnd"), node(1, "in"), node(2, "out")],
            vec![
                source(0, "v1", 1, 0, ac_spec(phi)),
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

        let (vin, vout, f) = run_ac_single_point(&c, "rc_phase", fc);
        let omega = 2.0 * PI * f;
        // H = 1 / (1 + j w R C)
        let h = Cx::new(1.0, 0.0).div(Cx::new(1.0, omega * R * C));
        let expected = h.mul(Cx::polar(1.0, phi));

        println!(
            "[rc] phi = {phi:+.6} rad at f = {f:.12e} Hz (fc = {fc:.12e} Hz): H = ({:.17e}, {:.17e}), v(out) = ({:.17e}, {:.17e}), expected = ({:.17e}, {:.17e})",
            h.re, h.im, vout.re, vout.im, expected.re, expected.im
        );

        // The source itself must carry the phase untouched.
        assert_component("v(in).re", vin.re, Cx::polar(1.0, phi).re);
        assert_component("v(in).im", vin.im, Cx::polar(1.0, phi).im);

        assert_component("v(out).re", vout.re, expected.re);
        assert_component("v(out).im", vout.im, expected.im);

        let mag_err = vout.distance(expected);
        let mag_tol = AC_ATOL + AC_RTOL * expected.magnitude();
        println!("[rc] |v(out) - expected| = {mag_err:.6e}, target = {mag_tol:.6e}");
        assert!(
            mag_err <= mag_tol,
            "complex magnitude error {mag_err:.3e} exceeds the §17 target {mag_tol:.3e}"
        );
        assert!(
            mag_err <= LINEAR_STRICT,
            "the RC AC solve should be far closer than the target; got {mag_err:.3e}"
        );

        // At the corner the transfer function magnitude is 1/sqrt(2) and its
        // phase is -45 degrees plus the source phase.
        assert!(
            (h.magnitude() - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12,
            "|H| at the returned frequency = {}",
            h.magnitude()
        );
        let expected_phase = -FRAC_PI_4 + phi;
        assert!(
            (vout.phase_rad() - expected_phase).abs() < 1e-9,
            "phase(v(out)) = {} rad, expected {expected_phase} rad",
            vout.phase_rad()
        );
    }
}

// ---------------------------------------------------------------------------
// 4. SIN: the transient waveform with a non-zero phase, zero initial state
// ---------------------------------------------------------------------------

/// The engine's SIN formula (`thevenin-0.5.0/src/waveform.rs:162-176`):
///
/// `t not after td : v0 + va * sin(phi_rad)`
/// `t after td     : v0 + va * sin(2*PI*freq*(t-td) + phi_rad) * exp(-theta*(t-td))`
///
/// with `theta = 0` here and `phi_rad = phi_deg * PI / 180`.
///
/// Note the hold value: before `td` the source is **not** `v0` but
/// `v0 + va*sin(phi)`. With `phi != 0` and `v0 = 0` the circuit would
/// therefore start at a non-zero value. To keep this test free of any startup
/// question it uses `v0 = -va*sin(phi)`, so the hold is exactly zero, and
/// `dc: 0 V`, so the operating point is zero too.
///
/// The circuit is a resistive divider: it stores no energy, so
/// `v(out) = 0.5 * v_in(t)` holds at every time point and this test checks the
/// waveform formula, **not** any initial-condition transient. That the initial
/// state really is zero is asserted twice: the OP run reports 0 V, and the
/// first transient sample is 0 V.
#[test]
fn sin_waveform_with_nonzero_phase_matches_engine_formula() {
    let phi = FRAC_PI_3; // 60 degrees
    let va = 1.0;
    let freq = 1e3;
    let td = 200e-6;
    let stop = 1e-3;
    // Hold value before td: v0 + va*sin(phi) = 0.
    let v0 = -va * phi.sin();

    let c = sin_divider(sin_spec(v0, va, freq, td, phi));

    // --- the operating point is zero ---
    let op_plan = plan_for(
        "sin_phase",
        AnalysisKind::Op,
        vec![probe("v(out)", Probe::NodeVoltage(NodeId(2)))],
    );
    let op = be().run(&c, &op_plan).expect("op runs");
    let op_vout = real(&op.datasets[0], "v(out)")[0];
    assert!(
        op_vout.abs() < 1e-12,
        "the operating point must be 0 V (dc = 0 and the waveform holds 0), got {op_vout}"
    );
    println!("[sin] OP v(out) = {op_vout:.3e} V");

    // Why `dc: 0 V` is what produces a zero state: the engine's operating
    // point prefers an explicit `dc` over the waveform
    // (`thevenin-0.5.0/src/mna_ir.rs:388-412`, MODEDCOP). The same circuit with
    // `dc = v0` therefore reports `OP = 0.5*v0`; that is pinned here so the
    // zero above cannot silently drift back into "the waveform at t = 0".
    // Measured while developing this test with the offset as the dc value:
    // OP = -0.43301270189221935 for v0 = -0.8660254037844386.
    let mut offset_dc = sin_spec(v0, va, freq, td, phi);
    offset_dc.dc = Some(Quantity::volts(v0));
    let op2 = be()
        .run(&sin_divider(offset_dc), &op_plan)
        .expect("op runs");
    let op2_vout = real(&op2.datasets[0], "v(out)")[0];
    assert!(
        (op2_vout - 0.5 * v0).abs() < 1e-12,
        "with dc = v0 = {v0:.12e} the OP must be 0.5*v0 = {:.12e}, got {op2_vout:.12e}",
        0.5 * v0
    );
    println!(
        "[sin] OP v(out) with dc = v0: {op2_vout:.12e} (0.5*v0 = {:.12e})",
        0.5 * v0
    );

    // --- the transient ---
    let plan = plan_for(
        "sin_phase",
        AnalysisKind::Tran(TranSpec {
            start_s: 0.0,
            stop_s: stop,
            max_step: Some(stop / 2000.0),
            output_interval: Some(stop / 1000.0),
            uic: false,
            span: SourceSpan::synthetic(),
        }),
        vec![probe("v(out)", Probe::NodeVoltage(NodeId(2)))],
    );
    let tran = be().run(&c, &plan).expect("tran runs");
    let d = &tran.datasets[0];
    let t = match &d.axis {
        Axis::Time(t) => t.clone(),
        other => panic!("expected a time axis, got {other:?}"),
    };
    let vout = real(d, "v(out)");
    assert_eq!(t.len(), vout.len());
    assert!(
        t.len() > 500,
        "expected a resolved transient, got {}",
        t.len()
    );

    // The first sample is the zero state the formula is compared against.
    assert_eq!(t[0], 0.0, "the transient must start at t = 0");
    assert!(
        vout[0].abs() < 1e-12,
        "the first transient sample must be 0 V, got {}",
        vout[0]
    );

    let mut before_td = 0usize;
    let mut after_td = 0usize;
    let mut worst: (f64, f64, f64) = (0.0, 0.0, 0.0); // |err|, t, expected
    for (i, &time) in t.iter().enumerate() {
        let vin = if time <= td {
            before_td += 1;
            // Waveform::Sin hold branch, evaluated exactly as the engine does.
            v0 + va * (phi.to_degrees() * PI / 180.0).sin()
        } else {
            after_td += 1;
            v0 + va * (2.0 * PI * freq * (time - td) + phi.to_degrees() * PI / 180.0).sin()
        };
        let expected = 0.5 * vin;
        let err = (vout[i] - expected).abs();
        if err > worst.0 {
            worst = (err, time, expected);
        }
    }

    println!(
        "[sin] phi = {:.1} deg, v0 = {v0:.12e}, td = {td:.3e} s, stop = {stop:.3e} s, samples = {}, pre-td = {before_td}, post-td = {after_td}",
        phi.to_degrees(),
        t.len()
    );
    println!(
        "[sin] worst |v(out) - 0.5*v_in(t)| = {:.6e} at t = {:.6e} s (expected {:.9})",
        worst.0, worst.1, worst.2
    );

    assert!(
        before_td > 0 && after_td > 0,
        "both branches must be exercised"
    );
    assert!(
        worst.0 <= 1e-9,
        "a resistive divider must reproduce the source waveform exactly; worst error {:.3e} at t = {:.6e} s",
        worst.0,
        worst.1
    );
}

// ---------------------------------------------------------------------------
// 5. SIN: phi is degrees, not radians
// ---------------------------------------------------------------------------

/// The engine's `eval_sin` converts `phi` from degrees
/// (`thevenin-0.5.0/src/waveform.rs:163`), and the adapter hands it
/// `phase_rad.to_degrees()` (`thevenin.rs:682-690`). Driving 30 degrees of
/// phase therefore gives a first sample of `0.5*sin(30 deg) = 0.25`; if the
/// adapter forgot the conversion, the engine would read `0.5236` as degrees
/// and give `0.5*sin(0.5236 deg) = 0.004569`.
///
/// `td = 0`, so both branches of the formula agree at `t = 0` and the whole
/// waveform is `v0 + va*sin(2*PI*freq*t + phi)`.
#[test]
fn sin_waveform_phase_is_degrees_not_radians() {
    let phi = FRAC_PI_6; // 30 degrees
    let freq = 1e3;
    let stop = 0.5e-3;

    let c = circuit(
        "sin_degrees",
        vec![node(0, "gnd"), node(1, "in"), node(2, "out")],
        vec![
            source(0, "v1", 1, 0, sin_spec(0.0, 1.0, freq, 0.0, phi)),
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
        "sin_degrees",
        AnalysisKind::Tran(TranSpec {
            start_s: 0.0,
            stop_s: stop,
            max_step: Some(stop / 1000.0),
            output_interval: Some(stop / 500.0),
            uic: false,
            span: SourceSpan::synthetic(),
        }),
        vec![probe("v(out)", Probe::NodeVoltage(NodeId(2)))],
    );
    let tran = be().run(&c, &plan).expect("tran runs");
    let d = &tran.datasets[0];
    let t = match &d.axis {
        Axis::Time(t) => t.clone(),
        other => panic!("expected a time axis, got {other:?}"),
    };
    let vout = real(d, "v(out)");

    let correct = 0.5 * (phi.to_degrees() * PI / 180.0).sin();
    // If the adapter passed the raw radians value, the engine would read it as
    // degrees: "phi * PI / 180" radians of actual phase.
    let raw_radians_as_degrees = 0.5 * (phi * PI / 180.0).sin();

    println!(
        "[sin-deg] t[0] = {:.3e} s, v(out)[0] = {:.17e}; correct = {correct:.17e}; raw-radians-would-give = {raw_radians_as_degrees:.17e}",
        t[0], vout[0]
    );

    assert!(
        (vout[0] - correct).abs() < 1e-12,
        "first sample {:.17e} != 0.5*sin(30 deg) = {correct:.17e}",
        vout[0]
    );
    assert!(
        (vout[0] - raw_radians_as_degrees).abs() > 1e-3,
        "first sample is only {:.3e} away from the radian-misread value {raw_radians_as_degrees:.6e}; the degrees convention is not being exercised",
        (vout[0] - raw_radians_as_degrees).abs()
    );

    let mut worst = 0.0_f64;
    for (i, &time) in t.iter().enumerate() {
        let expected = 0.5 * (2.0 * PI * freq * time + phi.to_degrees() * PI / 180.0).sin();
        worst = worst.max((vout[i] - expected).abs());
    }
    println!("[sin-deg] worst |v(out) - 0.5*sin(2*pi*f*t + phi)| = {worst:.6e}");
    assert!(
        worst <= 1e-9,
        "worst waveform error {worst:.3e} exceeds 1e-9"
    );
}

// ---------------------------------------------------------------------------
// 6. The three classic phase bugs
// ---------------------------------------------------------------------------

/// One test that fails for each of the three mistakes this file is meant to
/// catch, so a reviewer can see the separation margins:
///
/// * **sign flipped** (phase conjugated): the imaginary part changes sign, a
///   distance of 0.5 V here;
/// * **real/imaginary swapped**: the two components trade places, 0.259 V
///   apart;
/// * **forgot to_degrees**: the engine reads the raw radians as degrees,
///   0.254 V apart (see `ac_phase_is_converted_from_radians_to_degrees`).
///
/// The first assertions are the correct expectations; the three following ones
/// document that the wrong variants are nowhere near them.
#[test]
fn convention_guards_separate_sign_axis_and_degree_errors() {
    let phi = FRAC_PI_6;
    let c = resistive_divider(phi);
    let (_vin, vout, _f) = run_ac_single_point(&c, "divider_phase", 1e3);

    let correct = Cx::polar(0.5, phi);
    let sign_flipped = Cx::new(correct.re, -correct.im);
    let axis_swapped = Cx::new(correct.im, correct.re);
    let forgot_degrees = Cx::polar(0.5, phi * PI / 180.0);

    assert_component_strict("v(out).re", vout.re, correct.re);
    assert_component_strict("v(out).im", vout.im, correct.im);

    let d_sign = vout.distance(sign_flipped);
    let d_swap = vout.distance(axis_swapped);
    let d_deg = vout.distance(forgot_degrees);
    println!(
        "[guards] correct = ({:.6e}, {:.6e}); v(out) = ({:.6e}, {:.6e}); d(sign) = {d_sign:.6e}, d(swap) = {d_swap:.6e}, d(degrees) = {d_deg:.6e}",
        correct.re, correct.im, vout.re, vout.im
    );

    // Each guard is only meaningful if the wrong answer is far outside the
    // 1e-9 equality tolerance used above.
    assert!(
        d_sign > 1e-3,
        "a conjugated phase would differ by {d_sign:.3e}; the sign convention is untested"
    );
    assert!(
        d_swap > 1e-3,
        "swapping re/im would differ by {d_swap:.3e}; the component split is untested"
    );
    assert!(
        d_deg > 1e-3,
        "skipping to_degrees would differ by {d_deg:.3e}; the conversion is untested"
    );
}
