//! Round-4 QA: the result-expression value policy, tested against the
//! evaluator directly instead of through the CLI.
//!
//! Written by the QA worker, independent of the production tests: every
//! expected number below is derived in a comment from the formula it asserts,
//! and every failure case asserts a diagnostic *class* plus the operation that
//! produced it, never a bare `is_err()`.
//!
//! Contract: `docs/review-evidence/round4/design-contract.md` §1.3 (every
//! operation validates its own result; exact domain rules, no epsilon, no
//! saturation, no skipping; diagnostics carry analysis/kind/sample/index).

use circuit_core::Limits;
use circuit_core::diagnostic::Code;
use circuit_core::units::{CURRENT, VOLTAGE};
use circuit_results::dataset::{Axis, BackendInfo, Complex, Dataset, Signal};
use circuit_results::expr::{EvalSite, Expr, eval, eval_at, eval_constant, is_constant};

/// A hand-built operating-point dataset (no axis: exactly one sample).
fn scalar(signals: Vec<Signal>) -> Dataset {
    Dataset::new(
        "qa",
        "op1",
        "op",
        Axis::None,
        signals,
        BackendInfo::new("qa", "0"),
        &Limits::default(),
    )
    .expect("a hand-built scalar dataset is well formed")
}

/// A three-point time axis, so a non-finite sample has a coordinate.
fn transient(signals: Vec<Signal>) -> Dataset {
    Dataset::new(
        "qa",
        "tran1",
        "tran",
        Axis::Time(vec![0.0, 1e-3, 2e-3]),
        signals,
        BackendInfo::new("qa", "0"),
        &Limits::default(),
    )
    .expect("a hand-built transient dataset is well formed")
}

/// The single sample of a scalar evaluation.
fn one(expr: &Expr, dataset: &Dataset) -> f64 {
    let value = eval(expr, dataset).expect("the expression evaluates");
    let values = value.as_real().expect("the result is real");
    assert_eq!(values.len(), 1, "a scalar expression has one sample");
    values[0]
}

// ---------------------------------------------------------------------------
// R4-01: an illegal intermediate value is never hidden
// ---------------------------------------------------------------------------

/// `sqrt(-1)` has no real value: the evaluator must report `E_VALUE` naming
/// `sqrt`, not return NaN for a later reduction to swallow.
#[test]
fn sqrt_of_a_negative_number_is_a_value_error() {
    let ds = scalar(vec![Signal::real("v(a)", VOLTAGE, vec![1.0])]);
    let err = eval(&Expr::number(-1.0).sqrt(), &ds).expect_err("sqrt(-1) must not produce a value");

    assert_eq!(err.code, Code::Value, "{}", err.render_plain());
    assert!(err.is_error(), "{}", err.render_plain());
    let text = err.render_plain();
    assert!(text.contains("sqrt"), "the operation must be named: {text}");
    assert!(text.contains("-1"), "the operand must be shown: {text}");
    // Contract §1.3 rule 4: analysis and kind describe where it happened, and
    // a scalar expression says so instead of inventing an axis coordinate.
    assert!(text.contains("= analysis: op1"), "{text}");
    assert!(text.contains("= kind: op"), "{text}");
    assert!(
        text.contains("scalar"),
        "a dataset without an axis must be described as scalar: {text}"
    );
}

/// `min(sqrt(-1), 2)` reported `2` before the fix (f64::min ignores NaN).
/// The illegal operand must fail the whole expression.
#[test]
fn min_cannot_mask_an_illegal_intermediate() {
    let ds = scalar(vec![Signal::real("v(a)", VOLTAGE, vec![1.0])]);
    let expr = Expr::min(Expr::number(-1.0).sqrt(), Expr::number(2.0));
    let err = eval(&expr, &ds).expect_err("min(sqrt(-1), 2) must not return 2");

    assert_eq!(err.code, Code::Value, "{}", err.render_plain());
    assert!(
        err.render_plain().contains("sqrt"),
        "the failing sub-expression must be named: {}",
        err.render_plain()
    );
}

/// The same, one level deeper: neither a surrounding `max` nor a surrounding
/// `min` may hide the failure.
#[test]
fn nested_min_and_max_cannot_hide_the_failure() {
    let ds = scalar(vec![Signal::real("v(a)", VOLTAGE, vec![1.0])]);
    let inner = Expr::min(Expr::number(-1.0).sqrt(), Expr::number(2.0));
    let nested = Expr::max(Expr::min(inner, Expr::number(1.0)), Expr::number(3.0));
    let err = eval(&nested, &ds).expect_err("the nested extreme must not return a number");
    assert_eq!(err.code, Code::Value, "{}", err.render_plain());
}

/// `1e308 * 1e308` is `+inf` in IEEE arithmetic. The product is not a legal
/// sample, and `max(inf, 2)` must not report `2`.
#[test]
fn max_cannot_mask_an_overflowing_product() {
    let ds = scalar(vec![Signal::real("v(a)", VOLTAGE, vec![1.0])]);
    let overflow = Expr::number(1e308) * Expr::number(1e308);

    let bare = eval(&overflow, &ds).expect_err("1e308 * 1e308 must not evaluate to inf");
    assert_eq!(bare.code, Code::Value, "{}", bare.render_plain());

    let nested = Expr::max(overflow.clone(), Expr::number(2.0));
    let err = eval(&nested, &ds).expect_err("max(inf, 2) must not return 2");
    assert_eq!(err.code, Code::Value, "{}", err.render_plain());
    assert!(
        err.render_plain().contains('*'),
        "the overflowing operation must be named: {}",
        err.render_plain()
    );
}

/// A finite-but-overflowing division is the same class of error: the result is
/// not representable, so it is a diagnostic rather than `inf`.
///
/// 1.0 / 1e-320 = 1e320 > f64::MAX (1.7976931348623157e308), so IEEE gives
/// +inf.
#[test]
fn a_division_that_overflows_is_a_value_error() {
    let ds = scalar(vec![Signal::real("v(a)", VOLTAGE, vec![1.0])]);
    let expr = Expr::number(1.0) / Expr::number(1e-320);
    let err = eval(&expr, &ds).expect_err("1.0 / 1e-320 must not evaluate to inf");
    assert_eq!(err.code, Code::Value, "{}", err.render_plain());
}

/// Reading a signal whose sample is already NaN or +/-inf is refused by the
/// read itself; a later `abs` must not turn `inf` into a plausible magnitude.
#[test]
fn a_non_finite_signal_sample_is_refused_when_read() {
    let ds = transient(vec![Signal::real(
        "v(bad)",
        VOLTAGE,
        vec![1.0, f64::NAN, f64::INFINITY],
    )]);

    let err = eval(&Expr::signal("v(bad)"), &ds).expect_err("a NaN sample must not be read");
    assert_eq!(err.code, Code::Value, "{}", err.render_plain());
    let text = err.render_plain();
    assert!(text.contains("v(bad)"), "the signal must be named: {text}");
    // The offending sample has a coordinate on this axis: t = 1e-3, index 1.
    assert!(
        text.contains("= sample:"),
        "an axis coordinate must be reported: {text}"
    );
    assert!(
        text.contains("0.001"),
        "the sample coordinate (t = 1e-3 s) must be shown: {text}"
    );
    assert!(text.contains("= index: 1"), "{text}");

    let abs = eval(&Expr::signal("v(bad)").abs(), &ds).expect_err("abs(inf) is still inf");
    assert_eq!(abs.code, Code::Value, "{}", abs.render_plain());
}

/// A complex signal with a non-finite component follows the same policy: both
/// components are checked, not just the magnitude.
#[test]
fn a_non_finite_complex_component_is_refused() {
    let ds = Dataset::new(
        "qa",
        "ac1",
        "ac",
        Axis::Frequency(vec![1e3, 2e3]),
        vec![Signal::complex(
            "v(out)",
            VOLTAGE,
            vec![Complex::new(1.0, 0.0), Complex::new(0.0, f64::INFINITY)],
        )],
        BackendInfo::new("qa", "0"),
        &Limits::default(),
    )
    .expect("a hand-built AC dataset is well formed");

    let err =
        eval(&Expr::signal("v(out)"), &ds).expect_err("an infinite component must be refused");
    assert_eq!(err.code, Code::Value, "{}", err.render_plain());
    assert!(
        err.render_plain().contains("v(out)"),
        "{}",
        err.render_plain()
    );

    // |(0, inf)| is inf, so abs must not launder it either.
    let abs = eval(&Expr::signal("v(out)").abs(), &ds).expect_err("abs(inf) is still inf");
    assert_eq!(abs.code, Code::Value, "{}", abs.render_plain());
}

// ---------------------------------------------------------------------------
// R4-02: dimension overflow is a diagnostic, never a panic or a wrap
// ---------------------------------------------------------------------------

/// 128 voltage factors: V^1 .. V^127 are representable (the exponent is an
/// `i8` in -128..=127), V^128 is not. The evaluator must report
/// `E_DIMENSION`; before the fix a debug build panicked with "attempt to add
/// with overflow" in `units.rs`.
#[test]
fn a_long_voltage_product_is_a_dimension_error_not_a_panic() {
    let ds = scalar(vec![Signal::real("v(in)", VOLTAGE, vec![1.0])]);

    let mut expr = Expr::signal("v(in)");
    for _ in 1..128 {
        expr = expr * Expr::signal("v(in)");
    }
    let err = eval(&expr, &ds).expect_err("V^128 is not representable");
    assert_eq!(err.code, Code::Dimension, "{}", err.render_plain());

    // 127 factors are still legal: the guard is an overflow check, not a ban on
    // long products, and the value of 1 V raised to any power is 1.
    let mut ok = Expr::signal("v(in)");
    for _ in 1..127 {
        ok = ok * Expr::signal("v(in)");
    }
    assert_eq!(one(&ok, &ds), 1.0);
}

/// The boundary is exact: 127 is the largest representable exponent and -128
/// the smallest, so the checked arithmetic must refuse the step that leaves the
/// range and accept the one that lands on the edge.
#[test]
fn the_dimension_boundary_is_exact_at_the_representable_edge() {
    use circuit_core::units::Dimension;

    // 126 + 1 = 127 fits; 127 + 1 = 128 does not.
    assert_eq!(
        Dimension::new(126, 0, 0).checked_mul(Dimension::new(1, 0, 0)),
        Some(Dimension::new(127, 0, 0))
    );
    assert_eq!(
        Dimension::new(127, 0, 0).checked_mul(Dimension::new(1, 0, 0)),
        None,
        "127 + 1 leaves the i8 range"
    );
    // On the negative side -128 is i8::MIN: -127 + -1 lands on it, -128 + -1
    // leaves the range.
    assert_eq!(
        Dimension::new(-127, 0, 0).checked_mul(Dimension::new(-1, 0, 0)),
        Some(Dimension::new(-128, 0, 0))
    );
    assert_eq!(
        Dimension::new(-128, 0, 0).checked_mul(Dimension::new(-1, 0, 0)),
        None
    );
}

// ---------------------------------------------------------------------------
// Legal arithmetic still evaluates: the policy must not reject good numbers
// ---------------------------------------------------------------------------

#[test]
fn legal_arithmetic_still_evaluates() {
    let ds = scalar(vec![Signal::real("v(a)", VOLTAGE, vec![2.0])]);

    // sqrt(4) = 2.
    assert_eq!(one(&Expr::number(4.0).sqrt(), &ds), 2.0);
    // min(3, 2) = 2 and max(3, 2) = 3 are elementwise extremes.
    assert_eq!(
        one(&Expr::min(Expr::number(3.0), Expr::number(2.0)), &ds),
        2.0
    );
    assert_eq!(
        one(&Expr::max(Expr::number(3.0), Expr::number(2.0)), &ds),
        3.0
    );
    // 1e300 * 1e-300 is 1 up to one rounding step (the true product is
    // 1.00000000000000002...e0), and the point is that it is finite, so it is
    // legal. 2 V * 1e300 = 2e300 V likewise stays finite.
    let tiny = one(&(Expr::number(1e300) * Expr::number(1e-300)), &ds);
    assert!((tiny - 1.0).abs() < 1e-12, "{tiny}");
    assert_eq!(
        one(&(Expr::signal("v(a)") * Expr::number(1e300)), &ds),
        2e300
    );
    // A tiny but non-zero denominator is not a zero denominator: 1/4 = 0.25.
    assert_eq!(one(&(Expr::number(1.0) / Expr::number(4.0)), &ds), 0.25);
    // gain_db(2, 1) = 20*log10(2) = 6.020599913279624.
    let db = one(&Expr::gain_db(Expr::number(2.0), Expr::number(1.0)), &ds);
    assert!((db - 6.020_599_913_279_624).abs() < 1e-12, "{db}");
}

/// The pre-existing exact domain rules stay exact: no epsilon rescue.
#[test]
fn the_existing_exact_domain_rules_are_unchanged() {
    let ds = scalar(vec![Signal::real("v(a)", VOLTAGE, vec![1.0])]);

    let div = eval(&(Expr::number(1.0) / Expr::number(0.0)), &ds).expect_err("x/0 is an error");
    assert_eq!(div.code, Code::Value, "{}", div.render_plain());

    let gain = eval(&Expr::gain_db(Expr::number(0.0), Expr::number(1.0)), &ds)
        .expect_err("gain_db of a zero magnitude is an error");
    assert_eq!(gain.code, Code::Value, "{}", gain.render_plain());

    // A negative base under sqrt stays an error even when it is very small:
    // -1e-12 is a real negative number, not zero.
    let tiny = eval(&Expr::number(-1e-12).sqrt(), &ds).expect_err("sqrt(-1e-12) is an error");
    assert_eq!(tiny.code, Code::Value, "{}", tiny.render_plain());
}

// ---------------------------------------------------------------------------
// Named evaluation sites and check-time constants (contract 1.3.4-1.3.5)
// ---------------------------------------------------------------------------

/// A named site is attached to every diagnostic it produces, so an error
/// inside a long expression can be traced to the definition it came from.
#[test]
fn a_named_site_is_carried_by_the_diagnostic() {
    let ds = scalar(vec![Signal::real("v(a)", VOLTAGE, vec![1.0])]);

    let derive = eval_at(
        &Expr::number(-1.0).sqrt(),
        &ds,
        &EvalSite::derive("r4_derive_site"),
    )
    .expect_err("sqrt(-1) is illegal whatever the site");
    assert_eq!(derive.code, Code::Value, "{}", derive.render_plain());
    let text = derive.render_plain();
    assert!(text.contains("derive"), "{text}");
    assert!(text.contains("r4_derive_site"), "{text}");
    assert!(text.contains("= signal: r4_derive_site"), "{text}");

    let measure = eval_at(
        &Expr::number(-1.0).sqrt(),
        &ds,
        &EvalSite::measure("r4_measure_site"),
    )
    .expect_err("sqrt(-1) is illegal whatever the site");
    let text = measure.render_plain();
    assert!(text.contains("measure"), "{text}");
    assert!(text.contains("r4_measure_site"), "{text}");
    assert!(text.contains("= signal: r4_measure_site"), "{text}");

    // An anonymous site keeps the round-3 text: no name is invented.
    let anonymous = eval(&Expr::number(-1.0).sqrt(), &ds).expect_err("still illegal");
    let text = anonymous.render_plain();
    assert!(!text.contains("r4_derive_site"), "{text}");
    assert!(!text.contains("r4_measure_site"), "{text}");
}

/// check-time evaluation: what can be decided without a run is decided there,
/// and what cannot is refused as non-constant rather than guessed.
#[test]
fn is_constant_separates_check_time_from_run_time() {
    assert!(is_constant(&Expr::number(2.0)));
    assert!(is_constant(&Expr::number(4.0).sqrt()));
    assert!(is_constant(&Expr::min(
        Expr::number(3.0),
        Expr::number(2.0)
    )));
    assert!(!is_constant(&Expr::signal("v(a)")));
    assert!(!is_constant(&Expr::voltage("a")));
    assert!(!is_constant(&Expr::differential("a", "b")));
    assert!(!is_constant(&Expr::min(
        Expr::number(0.0),
        Expr::signal("v(a)")
    )));
}

#[test]
fn eval_constant_decides_constants_and_refuses_the_rest() {
    // sqrt(4) = 2, decided without any dataset.
    let value = eval_constant(&Expr::number(4.0).sqrt()).expect("a constant is decided");
    assert_eq!(value.as_real().expect("real")[0], 2.0);
    assert_eq!(value.unit, circuit_core::units::DIMENSIONLESS);

    // The illegal constants of R4-01 are refused here too.
    let illegal = [
        ("sqrt(-1)", Expr::number(-1.0).sqrt()),
        (
            "min(sqrt(-1), 2)",
            Expr::min(Expr::number(-1.0).sqrt(), Expr::number(2.0)),
        ),
        ("1e308 * 1e308", Expr::number(1e308) * Expr::number(1e308)),
        ("1.0 / 0.0", Expr::number(1.0) / Expr::number(0.0)),
        (
            "gain_db(0, 1)",
            Expr::gain_db(Expr::number(0.0), Expr::number(1.0)),
        ),
    ];
    for (label, expr) in illegal {
        let err = eval_constant(&expr).expect_err(&format!("{label} must not evaluate to a value"));
        assert_eq!(err.code, Code::Value, "{label}: {}", err.render_plain());
    }

    // An expression that reads a signal cannot be decided at check time; it is
    // reported as such, not as a wrong value.
    let err = eval_constant(&Expr::signal("v(a)")).expect_err("a signal is not constant");
    assert_eq!(err.code, Code::Value, "{}", err.render_plain());
    assert!(
        err.render_plain().contains("constant"),
        "{}",
        err.render_plain()
    );
}

/// The runtime and check-time paths must not disagree about *what* is wrong:
/// both name the operation, and only the runtime one can add the analysis and
/// the sample it saw.
#[test]
fn the_check_time_and_runtime_diagnostics_agree_on_the_fault() {
    let ds = scalar(vec![Signal::real("v(a)", VOLTAGE, vec![1.0])]);
    let runtime = eval(&Expr::number(-1.0).sqrt(), &ds).expect_err("runtime refuses it");
    let checked = eval_constant(&Expr::number(-1.0).sqrt()).expect_err("check refuses it");

    assert_eq!(runtime.code, checked.code, "the class must agree");
    assert!(
        runtime.message.contains("sqrt") && checked.message.contains("sqrt"),
        "both must name sqrt:\nruntime: {}\ncheck: {}",
        runtime.message,
        checked.message
    );

    // The check-time diagnostic has no analysis and no sample: there is none
    // yet, and the note says it is a constant.
    let checked_text = checked.render_plain();
    assert!(!checked_text.contains("= analysis:"), "{checked_text}");
    assert!(!checked_text.contains("= sample:"), "{checked_text}");
    assert!(checked_text.contains("constant"), "{checked_text}");
    // The runtime diagnostic does say where it happened.
    let runtime_text = runtime.render_plain();
    assert!(runtime_text.contains("= analysis: op1"), "{runtime_text}");
}

/// Units survive the guards: the policy rejects illegal values without
/// changing the dimension of legal ones.
#[test]
fn dimensions_of_legal_results_are_unchanged() {
    let ds = scalar(vec![
        Signal::real("v(a)", VOLTAGE, vec![2.0]),
        Signal::real("i(a)", CURRENT, vec![0.5]),
    ]);

    let volt = eval(&Expr::signal("v(a)"), &ds).expect("a finite sample reads fine");
    assert_eq!(volt.unit, VOLTAGE);
    let ohm = eval(&(Expr::signal("v(a)") / Expr::signal("i(a)")), &ds).expect("V/A = ohm");
    // V / A = ohm, whose exponents are (1, -1, 0).
    assert_eq!(ohm.unit, circuit_core::units::RESISTANCE);
    assert_eq!(
        one(&(Expr::signal("v(a)") / Expr::signal("i(a)")), &ds),
        4.0
    );
}
