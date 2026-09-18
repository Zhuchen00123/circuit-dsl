//! DSL-level regression for `sin(..., phase:)`: degrees in the source,
//! radians in the IR.
//!
//! Why this file exists: the elaborator converts the user's `phase:` from
//! degrees to radians (the `"sin"` branch of
//! `crates/circuit-dsl/src/elaborate.rs`, `deg.value.to_radians()`), but before
//! this file neither `crates/circuit-dsl/tests/elaborate.rs` nor the CLI tests
//! contained the string `phase` at all, so deleting the conversion kept every
//! test green.
//!
//! Facts re-read in the sources for this file (symbol names instead of line
//! numbers where the code moves):
//!
//! * `sin` accepts exactly `offset, amplitude, frequency, delay, damping,
//!   phase` (the `allowed` list of the `"sin"` branch in `elaborate.rs`);
//!   `phase` is read with `num_arg(..., units::DIMENSIONLESS, "sin.phase")` and
//!   stored as `deg.value.to_radians()`.
//! * `circuit_core::ir::Waveform::Sin { phase_rad, .. }` is the damped-sinusoid
//!   variant; the field name states the unit, radians.
//! * The adapter converts back for the engine:
//!   `crates/circuit-backend/src/thevenin.rs`, `map_source` fills
//!   `CqSourceSpec`'s AC phase and the SIN waveform's `phi` with
//!   `phase_rad.to_degrees()`.
//! * Two-terminal sources accept only `p, n, dc, ac, waveform`
//!   (`reject_unknown_args` in `elaborate.rs`), so there is no DSL syntax for
//!   an AC source phase today; `sin(phase:)` is the only phase-carrying path
//!   in the language.
//!
//! The helpers mirror `crates/circuit-dsl/tests/elaborate.rs`
//! (`SourceMap::add` then `lex`, `parse`, `compile`); that file is
//! deliberately not modified.

use circuit_core::diagnostic::Code;
use circuit_core::ir::Waveform;
use circuit_core::{Limits, SourceMap};
use circuit_dsl::{compile, lex, parse};

/// Run the real front end; `Err` carries the rendered diagnostics.
fn run(src: &str) -> Result<circuit_dsl::Compiled, String> {
    let mut sm = SourceMap::new();
    let id = sm.add("phase_syntax.cdsl", src);

    let tokens = match lex(id, src) {
        Ok(t) => t,
        Err(d) => return Err(d.render(&sm)),
    };
    let program = match parse(&tokens) {
        Ok(p) => p,
        Err(d) => return Err(d.render(&sm)),
    };
    match compile(&program, &Limits::default()) {
        Ok(c) => Ok(c),
        Err(d) => Err(d.render(&sm)),
    }
}

fn ok(src: &str) -> circuit_dsl::Compiled {
    match run(src) {
        Ok(c) => c,
        Err(text) => panic!(
            "expected success, got diagnostics:
{text}"
        ),
    }
}

fn err(src: &str) -> String {
    match run(src) {
        Ok(_) => panic!("expected diagnostics, but compilation succeeded"),
        Err(text) => text,
    }
}

fn assert_code(text: &str, code: Code) {
    let needle = format!("[{}]", code.as_str());
    assert!(
        text.contains(&needle),
        "expected {needle} in:
{text}"
    );
}

/// A one-source circuit whose SIN waveform carries `degrees` as `phase:`.
fn sin_source(degrees: &str) -> String {
    format!(
        r#"
circuit :phase do
  node :a
  voltage_source :v1, p: :a, n: :gnd,
    dc: 0.V,
    waveform: sin(offset: 0.V, amplitude: 1.V, frequency: 1.kHz, phase: {degrees})
  resistor :r1, p: :a, n: :gnd, value: 1.kohm
end

experiment :op_point, circuit: :phase do
  op
  save v(:a)
end
"#
    )
}

/// The same circuit with no `phase:` argument at all: the default branch.
fn sin_source_without_phase() -> String {
    r#"
circuit :phase do
  node :a
  voltage_source :v1, p: :a, n: :gnd,
    dc: 0.V,
    waveform: sin(offset: 0.V, amplitude: 1.V, frequency: 1.kHz)
  resistor :r1, p: :a, n: :gnd, value: 1.kohm
end
"#
    .to_string()
}

/// The `phase_rad` the elaborator stored for device `device`.
fn phase_rad_of(compiled: &circuit_dsl::Compiled, device: &str) -> f64 {
    let circuit = compiled
        .circuit("phase")
        .unwrap_or_else(|| panic!("no circuit named phase in {compiled:?}"));
    let id = circuit
        .device_id(device)
        .unwrap_or_else(|| panic!("no device named {device}"));
    let d = circuit
        .device(id)
        .unwrap_or_else(|| panic!("device {device} vanished"));
    let spec = d
        .source
        .as_ref()
        .unwrap_or_else(|| panic!("{device} has no source spec"));
    match spec
        .waveform
        .as_ref()
        .unwrap_or_else(|| panic!("{device} has no waveform"))
    {
        Waveform::Sin { phase_rad, .. } => *phase_rad,
        other => panic!("expected a SIN waveform, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 1. The degrees -> radians conversion
// ---------------------------------------------------------------------------

/// `phase: 90` must reach the IR as radians.
///
/// Guards: deleting `to_radians()` (the IR would hold 90.0), replacing it with
/// `to_degrees()` (5156.6), or dropping the argument on the floor (0.0).
#[test]
fn sin_phase_is_converted_from_degrees_to_radians() {
    let compiled = ok(&sin_source("90"));
    let phase_rad = phase_rad_of(&compiled, "v1");
    println!("[phase] phase: 90 -> IR phase_rad = {phase_rad:.17e}");

    assert!(
        (phase_rad - 90.0_f64.to_radians()).abs() < 1e-12,
        "phase: 90 must be stored as 90 degrees converted to radians ({:.17e}), got {phase_rad:.17e}",
        90.0_f64.to_radians()
    );
    // 90 degrees is about 1.5708 rad, so a degrees-valued IR is 88.43 away.
    // This second assertion states the failure mode directly: it is what a
    // deleted conversion trips even if the equality above were ever loosened.
    assert!(
        (phase_rad - 90.0).abs() > 1.0,
        "phase_rad = {phase_rad:.17e} looks like degrees, not radians"
    );
}

// ---------------------------------------------------------------------------
// 2. Several angles, both signs, and the half turn
// ---------------------------------------------------------------------------

/// The conversion must hold at every angle, including the two that are easy to
/// get wrong: a negative angle and 180 degrees.
///
/// Guards: a sign error (an `abs()` or a missing unary minus), a wrong
/// principal value for the half turn (`-PI` or `2*PI` instead of `PI`), and
/// a lost conversion anywhere except 0 degrees.
#[test]
fn sin_phase_covers_zero_both_signs_and_a_half_turn() {
    let cases = [
        ("0", 0.0_f64),
        ("30", 30.0),
        ("90", 90.0),
        ("-45", -45.0),
        ("180", 180.0),
    ];

    for (text, degrees) in cases {
        let compiled = ok(&sin_source(text));
        let got = phase_rad_of(&compiled, "v1");
        let want = degrees.to_radians();
        println!("[phase] phase: {text:>4} deg -> phase_rad = {got:.17e} (want {want:.17e})");

        assert!(
            (got - want).abs() < 1e-12,
            "phase: {text} must store {want:.17e} rad, got {got:.17e}"
        );

        if degrees == 0.0 {
            // The explicit zero must survive: a stale default must not invent
            // an angle, and a conversion must not turn 0 into a non-zero value.
            assert!(
                got == 0.0,
                "phase: 0 must store exactly 0 rad, got {got:.17e}"
            );
        } else {
            // A degrees-in-IR implementation is at least 1.0 away at every one
            // of these angles (44.2 at -45 deg, 88.4 at 90 deg, 176.9 at 180).
            assert!(
                (got - degrees).abs() > 1.0,
                "phase: {text} stored {got:.17e}, which looks like degrees rather than radians"
            );
            // Sign preservation: -45 degrees is a negative angle and must stay
            // negative; this is the assertion an abs()/negation bug fails.
            assert_eq!(
                got.is_sign_negative(),
                degrees.is_sign_negative(),
                "phase: {text} lost its sign: {got:.17e}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 3. The IR unit, and the adapter that converts it back
// ---------------------------------------------------------------------------

/// The IR field is named `phase_rad` and really holds radians.
///
/// Guards: storing degrees in `phase_rad`. The adapter's `map_source`
/// (`crates/circuit-backend/src/thevenin.rs`) hands the engine
/// `phase_rad.to_degrees()` for both the AC excitation phase and the SIN
/// waveform's `phi`; the engine reads those as degrees. A degrees-valued IR
/// would therefore be converted a second time and rotate every phasor by a
/// further factor of about 57.3.
#[test]
fn ir_phase_is_radians_so_the_adapter_can_convert_back_to_degrees() {
    let compiled = ok(&sin_source("90"));
    let phase_rad = phase_rad_of(&compiled, "v1");

    // Converting back recovers exactly what the user wrote.
    assert!(
        (phase_rad.to_degrees() - 90.0).abs() < 1e-12,
        "phase_rad.to_degrees() = {} but the source wrote phase: 90",
        phase_rad.to_degrees()
    );
    // A degrees-in-IR implementation would make 90.0.to_degrees() = 5156.62.
    assert!(
        (phase_rad - 90.0_f64.to_degrees()).abs() > 1.0,
        "phase_rad = {phase_rad:.17e} cannot be a radian value"
    );
    println!(
        "[phase] IR phase_rad = {phase_rad:.17e}; to_degrees() = {} (the value map_source sends)",
        phase_rad.to_degrees()
    );
}

// ---------------------------------------------------------------------------
// 4. The argument is dimensionless
// ---------------------------------------------------------------------------

/// `phase:` is an angle, so it is dimension-neutral; a value carrying a unit
/// must be rejected rather than silently reinterpreted.
///
/// Guards: relaxing the argument to any dimension (for example by passing the
/// `Quantity` straight through), which would let `phase: 1.s` become an
/// angle of 1 radian.
#[test]
fn sin_phase_must_be_dimensionless() {
    let text = err(&sin_source("1.s"));
    assert_code(&text, Code::Dimension);
    assert!(
        text.contains("sin.phase"),
        "the diagnostic must name the argument it rejected:
{text}"
    );
    println!(
        "[phase] phase: 1.s -> {}",
        text.lines().next().unwrap_or("")
    );
}

// ---------------------------------------------------------------------------
// 5. The AC source has no phase syntax (contract boundary)
// ---------------------------------------------------------------------------

/// `ac:` takes an amplitude only; there is no `phase:` argument on a source
/// statement.
///
/// This is the current, known limitation of the language, not something this
/// round implements: `sin(phase:)` is the only phase-carrying DSL path.
///
/// Guards: someone adding a source-level `phase:` argument without a decision
/// and without the reader/parser/diagnostic work that implies. If the argument
/// is ever implemented deliberately, this test must be updated on purpose --
/// the pinned accepted-list below is what makes the change visible.
#[test]
fn an_ac_source_has_no_phase_argument_yet() {
    let text = err(r#"
circuit :ac_phase do
  node :a
  voltage_source :v1, p: :a, n: :gnd, dc: 0.V, ac: 1.V, phase: 90
  resistor :r1, p: :a, n: :gnd, value: 1.kohm
end
"#);

    assert_code(&text, Code::Argument);
    assert!(text.contains("has no argument `phase`"), "{text}");
    assert!(
        text.contains("accepted: p, n, dc, ac, waveform"),
        "the accepted argument list changed; re-check whether `phase` became legal:
{text}"
    );
    println!(
        "[phase] source phase: -> {}",
        text.lines().next().unwrap_or("")
    );
}

// ---------------------------------------------------------------------------
// 6. Extra: a SIN waveform with no phase argument at all
// ---------------------------------------------------------------------------

/// A missing `phase:` is a zero angle, not an uninitialised or borrowed value.
///
/// Guards: the `None => 0.0` branch of the `sin` arm, which no other test
/// reaches (no test in this crate elaborated a `sin` waveform before this
/// file).
#[test]
fn sin_without_a_phase_defaults_to_zero_radians() {
    let compiled = ok(&sin_source_without_phase());
    let phase_rad = phase_rad_of(&compiled, "v1");
    println!("[phase] no phase: -> phase_rad = {phase_rad:.17e}");
    assert!(
        phase_rad == 0.0,
        "a missing phase: must default to 0 rad, got {phase_rad:.17e}"
    );
}
