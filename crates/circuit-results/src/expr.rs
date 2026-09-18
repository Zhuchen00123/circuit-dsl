//! Result expressions: a small typed AST evaluated against a [`Dataset`].
//!
//! The language's *result* expressions (spec §2.4, §5.2) are not the
//! elaboration-time expression language: `v(:out)` does not have a value until
//! a simulation has run. This module is the second evaluator — the one that
//! works on whole sample vectors.
//!
//! # What is in scope
//!
//! | form | meaning |
//! |---|---|
//! | `v(node)` | the saved node voltage signal |
//! | `v(a, b)` | `v(a) - v(b)`, or the saved `v(a,b)` signal if the backend produced one |
//! | `i(dev)` | the saved device current signal |
//! | `abs(x)` | absolute value (magnitude, for complex data) |
//! | `sqrt(x)` | square root of real data with even dimension exponents |
//! | `min(a, b)`, `max(a, b)` | element-wise, real data only |
//! | `20*log10(abs(a/b))` | gain in dB; `a` and `b` must have the same unit |
//!
//! Signals are identified by the probe name the user wrote (`v(out)`,
//! `i(r1)`), looked up in the dataset case-insensitively. A probe that names a
//! signal the dataset does not contain is an [`Code::Name`] error whose notes
//! list the signals that *are* available.
//!
//! # Real and complex
//!
//! Real and complex data stay distinct. A binary operation on two real vectors
//! produces a real vector; if either side is complex both sides are promoted
//! (a real sample becomes `re + 0i`) and the result is complex. In particular
//! `v(a) - v(b)` works for two complex signals, which is what an AC sweep
//! needs.

use std::fmt;
use std::ops::{Add, Div, Mul, Neg, Sub};

use circuit_core::units::{DIMENSIONLESS, Dimension};
use circuit_core::{Code, Diagnostic};

use crate::dataset::{Complex, Data, Dataset, Signal};

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// The value of an evaluated expression: a dimension and its samples.
#[derive(Clone, PartialEq, Debug)]
pub struct Value {
    pub unit: Dimension,
    pub data: Data,
}

impl Value {
    /// A single dimensionless sample; broadcasts against longer vectors.
    pub fn scalar(x: f64) -> Self {
        Self {
            unit: DIMENSIONLESS,
            data: Data::Real(vec![x]),
        }
    }

    pub fn real(unit: Dimension, values: Vec<f64>) -> Self {
        Self {
            unit,
            data: Data::Real(values),
        }
    }

    pub fn complex(unit: Dimension, values: Vec<Complex>) -> Self {
        Self {
            unit,
            data: Data::Complex(values),
        }
    }

    /// The samples of a saved signal, with its unit.
    pub fn from_signal(signal: &Signal) -> Self {
        Self {
            unit: signal.unit,
            data: signal.data.clone(),
        }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn is_complex(&self) -> bool {
        self.data.is_complex()
    }

    /// Real samples, or magnitudes for complex data (see
    /// [`Data::magnitudes`]).
    pub fn magnitudes(&self) -> Vec<f64> {
        self.data.magnitudes()
    }

    pub fn as_real(&self) -> Option<&[f64]> {
        self.data.as_real()
    }
}

// ---------------------------------------------------------------------------
// The expression AST
// ---------------------------------------------------------------------------

/// A result expression.
///
/// The AST is public so the CLI can build it directly from the parsed
/// `measure` expression without this crate depending on the parser.
#[derive(Clone, PartialEq, Debug)]
pub enum Expr {
    /// A numeric literal: one dimensionless sample that broadcasts.
    Number(f64),
    /// A signal looked up by probe name, e.g. `v(out)` or `i(r1)`.
    Signal(String),
    /// `v(pos) - v(neg)`.
    ///
    /// If the dataset contains a saved differential signal named `v(pos,neg)`
    /// it is used as-is (the backend already subtracted); otherwise the two
    /// node signals are looked up and subtracted here.
    Differential {
        pos: String,
        neg: String,
    },
    Neg(Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
    /// Absolute value: `|x|` for real data, `|z|` for complex data.
    Abs(Box<Expr>),
    /// Square root; requires real data and even dimension exponents.
    Sqrt(Box<Expr>),
    Min(Box<Expr>, Box<Expr>),
    Max(Box<Expr>, Box<Expr>),
    /// `20 * log10(abs(numerator / denominator))`.
    GainDb {
        numerator: Box<Expr>,
        denominator: Box<Expr>,
    },
}

impl Expr {
    pub fn number(x: f64) -> Self {
        Self::Number(x)
    }

    /// A signal by its full probe name, e.g. `v(out)`, `v(a,b)`, `i(r1)`.
    pub fn signal(name: impl Into<String>) -> Self {
        Self::Signal(name.into())
    }

    /// `v(node)`, the node voltage relative to ground.
    pub fn voltage(node: &str) -> Self {
        Self::Signal(format!("v({node})"))
    }

    /// `i(device)`, positive in the device's `p -> n` direction.
    pub fn current(device: &str) -> Self {
        Self::Signal(format!("i({device})"))
    }

    /// `v(pos) - v(neg)`.
    pub fn differential(pos: &str, neg: &str) -> Self {
        Self::Differential {
            pos: pos.to_string(),
            neg: neg.to_string(),
        }
    }

    /// `20*log10(abs(numerator / denominator))`. Both sides must share a unit.
    pub fn gain_db(numerator: Expr, denominator: Expr) -> Self {
        Self::GainDb {
            numerator: Box::new(numerator),
            denominator: Box::new(denominator),
        }
    }

    /// Element-wise `min`; both sides must share a unit and be real.
    pub fn min(a: Expr, b: Expr) -> Self {
        Self::Min(Box::new(a), Box::new(b))
    }

    /// Element-wise `max`; both sides must share a unit and be real.
    pub fn max(a: Expr, b: Expr) -> Self {
        Self::Max(Box::new(a), Box::new(b))
    }

    /// Absolute value (magnitude for complex data), keeping the unit.
    pub fn abs(self) -> Self {
        Self::Abs(Box::new(self))
    }

    /// Square root; the dimension exponents must all be even.
    pub fn sqrt(self) -> Self {
        Self::Sqrt(Box::new(self))
    }
}

impl Add for Expr {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::Add(Box::new(self), Box::new(rhs))
    }
}

impl Sub for Expr {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::Sub(Box::new(self), Box::new(rhs))
    }
}

impl Mul for Expr {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self::Mul(Box::new(self), Box::new(rhs))
    }
}

impl Div for Expr {
    type Output = Self;
    fn div(self, rhs: Self) -> Self {
        Self::Div(Box::new(self), Box::new(rhs))
    }
}

impl Neg for Expr {
    type Output = Self;
    fn neg(self) -> Self {
        Self::Neg(Box::new(self))
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(x) => write!(f, "{x}"),
            Self::Signal(name) => f.write_str(name),
            Self::Differential { pos, neg } => write!(f, "v({pos}, {neg})"),
            Self::Neg(inner) => write!(f, "-{inner}"),
            // Binary nodes are parenthesized so a printed expression can be
            // read unambiguously, even though the AST is what is evaluated.
            Self::Add(a, b) => write!(f, "({a} + {b})"),
            Self::Sub(a, b) => write!(f, "({a} - {b})"),
            Self::Mul(a, b) => write!(f, "({a} * {b})"),
            Self::Div(a, b) => write!(f, "({a} / {b})"),
            Self::Abs(x) => write!(f, "abs({x})"),
            Self::Sqrt(x) => write!(f, "sqrt({x})"),
            Self::Min(a, b) => write!(f, "min({a}, {b})"),
            Self::Max(a, b) => write!(f, "max({a}, {b})"),
            Self::GainDb {
                numerator,
                denominator,
            } => write!(f, "20*log10(abs({numerator} / {denominator}))"),
        }
    }
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

/// Evaluate `expr` against `dataset`.
pub fn eval(expr: &Expr, dataset: &Dataset) -> Result<Value, Diagnostic> {
    match expr {
        Expr::Number(x) => Ok(Value::scalar(*x)),
        Expr::Signal(name) => Ok(Value::from_signal(lookup(dataset, name)?)),
        Expr::Differential { pos, neg } => eval_differential(dataset, pos, neg),
        Expr::Neg(inner) => {
            let value = eval(inner, dataset)?;
            Ok(Value {
                unit: value.unit,
                data: negate(&value.data),
            })
        }
        Expr::Add(a, b) => eval_sum(a, b, dataset, BinOp::Add),
        Expr::Sub(a, b) => eval_sum(a, b, dataset, BinOp::Sub),
        Expr::Mul(a, b) => eval_product(a, b, dataset, BinOp::Mul),
        Expr::Div(a, b) => eval_product(a, b, dataset, BinOp::Div),
        Expr::Abs(inner) => {
            let value = eval(inner, dataset)?;
            let data = match value.data {
                Data::Real(values) => Data::Real(values.into_iter().map(f64::abs).collect()),
                Data::Complex(values) => {
                    Data::Real(values.into_iter().map(|z| z.magnitude()).collect())
                }
            };
            Ok(Value {
                unit: value.unit,
                data,
            })
        }
        Expr::Sqrt(inner) => eval_sqrt(inner, dataset),
        Expr::Min(a, b) => eval_elementwise(a, b, dataset, "min", f64::min),
        Expr::Max(a, b) => eval_elementwise(a, b, dataset, "max", f64::max),
        Expr::GainDb {
            numerator,
            denominator,
        } => eval_gain_db(numerator, denominator, dataset),
    }
}

/// Look up a signal, or explain which signals exist.
fn lookup<'a>(dataset: &'a Dataset, name: &str) -> Result<&'a Signal, Diagnostic> {
    dataset.signal(name).ok_or_else(|| {
        let names = dataset.signal_names();
        let available = if names.is_empty() {
            "<none>".to_string()
        } else {
            names.join(", ")
        };
        Diagnostic::error(
            Code::Name,
            format!(
                "no signal named `{name}` in analysis `{}`",
                dataset.analysis
            ),
        )
        .with_context("analysis", dataset.analysis.clone())
        .with_context("kind", dataset.kind.clone())
        .with_note(format!("available signals: {available}"))
    })
}

fn eval_differential(dataset: &Dataset, pos: &str, neg: &str) -> Result<Value, Diagnostic> {
    // A saved differential probe is already the difference; use it verbatim.
    let combined = format!("v({pos},{neg})");
    if let Some(signal) = dataset.signal(&combined) {
        return Ok(Value::from_signal(signal));
    }

    let pos_name = format!("v({pos})");
    let neg_name = format!("v({neg})");
    let a = lookup(dataset, &pos_name)?;
    let b = lookup(dataset, &neg_name)?;
    if a.unit != b.unit {
        return Err(dimension_error(
            "-",
            &Expr::Signal(pos_name),
            &Value::from_signal(a),
            &Expr::Signal(neg_name),
            &Value::from_signal(b),
        ));
    }
    // Real - real is real; anything else promotes to complex.
    let data = combine(&a.data, &b.data, |x, y| x - y, |x, y| x - y)?;
    Ok(Value { unit: a.unit, data })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl BinOp {
    fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
        }
    }
}

fn eval_sum(a: &Expr, b: &Expr, dataset: &Dataset, op: BinOp) -> Result<Value, Diagnostic> {
    let va = eval(a, dataset)?;
    let vb = eval(b, dataset)?;
    if va.unit != vb.unit {
        return Err(dimension_error(op.symbol(), a, &va, b, &vb));
    }
    let data = match op {
        BinOp::Add => combine(&va.data, &vb.data, |x, y| x + y, |x, y| x + y)?,
        BinOp::Sub => combine(&va.data, &vb.data, |x, y| x - y, |x, y| x - y)?,
        _ => unreachable!("eval_sum is only called with + and -"),
    };
    Ok(Value {
        unit: va.unit,
        data,
    })
}

fn eval_product(a: &Expr, b: &Expr, dataset: &Dataset, op: BinOp) -> Result<Value, Diagnostic> {
    let va = eval(a, dataset)?;
    let vb = eval(b, dataset)?;
    let (unit, data) = match op {
        BinOp::Mul => (
            va.unit.mul(vb.unit),
            combine(&va.data, &vb.data, |x, y| x * y, |x, y| x * y)?,
        ),
        BinOp::Div => (
            va.unit.div(vb.unit),
            combine(&va.data, &vb.data, |x, y| x / y, |x, y| x / y)?,
        ),
        _ => unreachable!("eval_product is only called with * and /"),
    };
    Ok(Value { unit, data })
}

fn eval_sqrt(inner: &Expr, dataset: &Dataset) -> Result<Value, Diagnostic> {
    let value = eval(inner, dataset)?;
    let Some(values) = value.data.as_real() else {
        return Err(Diagnostic::error(
            Code::Type,
            format!("sqrt(`{inner}`) requires real data, but the signal is complex"),
        )
        .with_note("take abs(...) first to work with the magnitude of a complex signal"));
    };
    let unit = half_dimension(value.unit).ok_or_else(|| {
        Diagnostic::error(
            Code::Dimension,
            format!(
                "sqrt of a quantity in {} is not representable: every dimension exponent must be even",
                value.unit
            ),
        )
        .with_dims(value.unit, Dimension::default())
    })?;
    Ok(Value::real(unit, values.iter().map(|x| x.sqrt()).collect()))
}

fn eval_elementwise(
    a: &Expr,
    b: &Expr,
    dataset: &Dataset,
    op_name: &str,
    op: fn(f64, f64) -> f64,
) -> Result<Value, Diagnostic> {
    let va = eval(a, dataset)?;
    let vb = eval(b, dataset)?;
    if va.unit != vb.unit {
        return Err(dimension_error(op_name, a, &va, b, &vb));
    }
    if va.is_complex() || vb.is_complex() {
        return Err(Diagnostic::error(
            Code::Type,
            format!("`{op_name}({a}, {b})` is defined for real samples only"),
        )
        .with_note("apply abs(...) to a complex signal to compare magnitudes"));
    }
    let data = combine(&va.data, &vb.data, op, |x, y| {
        if op(x.magnitude(), y.magnitude()) == x.magnitude() {
            x
        } else {
            y
        }
    })?;
    Ok(Value {
        unit: va.unit,
        data,
    })
}

fn eval_gain_db(
    numerator: &Expr,
    denominator: &Expr,
    dataset: &Dataset,
) -> Result<Value, Diagnostic> {
    let va = eval(numerator, dataset)?;
    let vb = eval(denominator, dataset)?;
    if va.unit != vb.unit {
        return Err(
            Diagnostic::error(
                Code::Dimension,
                format!(
                    "gain `{numerator} / {denominator}` must be dimensionless, but the units are {} and {}",
                    va.unit, vb.unit
                ),
            )
            .with_dims(va.unit, vb.unit)
            .with_note("dB is defined for a ratio of like quantities (out/in)"),
        );
    }
    let ratio = combine(&va.data, &vb.data, |x, y| x / y, |x, y| x / y)?;
    let db = ratio
        .magnitudes()
        .into_iter()
        .map(|m| 20.0 * m.log10())
        .collect();
    Ok(Value::real(DIMENSIONLESS, db))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A dimension whose exponents are all even, halved; `None` otherwise.
fn half_dimension(d: Dimension) -> Option<Dimension> {
    let half = |e: i8| if e % 2 == 0 { Some(e / 2) } else { None };
    Some(Dimension::new(half(d.volt)?, half(d.amp)?, half(d.second)?))
}

fn negate(data: &Data) -> Data {
    match data {
        Data::Real(values) => Data::Real(values.iter().map(|x| -x).collect()),
        Data::Complex(values) => Data::Complex(values.iter().map(|z| -*z).collect()),
    }
}

fn dimension_error(op: &str, a: &Expr, va: &Value, b: &Expr, vb: &Value) -> Diagnostic {
    Diagnostic::error(
        Code::Dimension,
        format!(
            "cannot apply `{op}` to `{a}` ({}) and `{b}` ({}): the units differ",
            va.unit, vb.unit
        ),
    )
    .with_dims(va.unit, vb.unit)
    .with_note("+, -, min and max require both sides to have the same unit")
}

/// Combine two sample vectors element-wise, broadcasting a length-1 vector.
///
/// Real and real stays real; if either side is complex both are promoted and
/// the result is complex. Lengths must match unless one side is a scalar.
fn combine(
    a: &Data,
    b: &Data,
    real_op: impl Fn(f64, f64) -> f64,
    complex_op: impl Fn(Complex, Complex) -> Complex,
) -> Result<Data, Diagnostic> {
    let n = broadcast_len(a.len(), b.len())?;
    match (a, b) {
        (Data::Real(x), Data::Real(y)) => {
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                out.push(real_op(sample_real(x, i, n), sample_real(y, i, n)));
            }
            Ok(Data::Real(out))
        }
        _ => {
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let x = sample_complex(a, i, n);
                let y = sample_complex(b, i, n);
                out.push(complex_op(x, y));
            }
            Ok(Data::Complex(out))
        }
    }
}

fn broadcast_len(a: usize, b: usize) -> Result<usize, Diagnostic> {
    if a == b {
        Ok(a)
    } else if a == 1 {
        Ok(b)
    } else if b == 1 {
        Ok(a)
    } else {
        Err(Diagnostic::error(
            Code::Value,
            format!("cannot combine a value with {a} samples and one with {b} samples"),
        ))
    }
}

/// Index into a real vector, broadcasting a length-1 vector over `n` samples.
fn sample_real(values: &[f64], i: usize, n: usize) -> f64 {
    let index = if values.len() == 1 && n > 1 { 0 } else { i };
    values.get(index).copied().unwrap_or(0.0)
}

/// Index into any sample vector, broadcasting a length-1 vector over `n`.
fn sample_complex(data: &Data, i: usize, n: usize) -> Complex {
    let index = if data.len() == 1 && n > 1 { 0 } else { i };
    data.sample(index).unwrap_or(Complex::ZERO)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{Axis, BackendInfo, Signal};
    use circuit_core::Limits;
    use circuit_core::units::{CURRENT, VOLTAGE};

    fn dataset(kind: &str, axis: Axis, signals: Vec<Signal>) -> Dataset {
        Dataset::new(
            "exp",
            "an1",
            kind,
            axis,
            signals,
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("test dataset must be well formed")
    }

    /// Two real OP signals: `v(a) = 3 V`, `v(b) = 1 V`.
    fn real_op() -> Dataset {
        dataset(
            "op",
            Axis::None,
            vec![
                Signal::real("v(a)", VOLTAGE, vec![3.0]),
                Signal::real("v(b)", VOLTAGE, vec![1.0]),
            ],
        )
    }

    fn assert_close(got: f64, want: f64, tol: f64) {
        assert!(
            (got - want).abs() <= tol,
            "expected {want} (+-{tol}), got {got}"
        );
    }

    #[test]
    fn signal_lookup_is_case_insensitive() {
        let ds = real_op();
        let value = eval(&Expr::signal("V(A)"), &ds).expect("found");
        assert_eq!(value.unit, VOLTAGE);
        assert_eq!(value.as_real(), Some(&[3.0][..]));
    }

    #[test]
    fn voltage_helper_builds_the_probe_name() {
        assert_eq!(Expr::voltage("out"), Expr::Signal("v(out)".into()));
        assert_eq!(Expr::current("r1"), Expr::Signal("i(r1)".into()));
        assert_eq!(Expr::voltage("out").to_string(), "v(out)");
        assert_eq!(
            Expr::differential("a", "b").to_string(),
            "v(a, b)",
            "differential expressions print as the probe the user wrote"
        );
    }

    #[test]
    fn missing_signal_is_a_name_error_listing_alternatives() {
        let ds = real_op();
        let err = eval(&Expr::voltage("out"), &ds).expect_err("v(out) is not saved");
        assert_eq!(err.code, Code::Name);
        assert!(err.message.contains("v(out)"), "{}", err.message);
        assert!(err.message.contains("an1"), "{}", err.message);
        let note = err.notes.first().expect("a note listing the signals");
        assert!(note.contains("v(a)") && note.contains("v(b)"), "{note}");
    }

    #[test]
    fn missing_signal_note_says_none_when_there_are_no_signals() {
        let ds = dataset("op", Axis::None, Vec::new());
        let err = eval(&Expr::voltage("out"), &ds).expect_err("nothing is saved");
        assert_eq!(err.notes, vec!["available signals: <none>".to_string()]);
    }

    #[test]
    fn differential_subtracts_two_real_signals() {
        let ds = real_op();
        let value = eval(&Expr::differential("a", "b"), &ds).expect("both nodes are saved");
        assert_eq!(value.unit, VOLTAGE);
        assert_eq!(value.as_real(), Some(&[2.0][..]));
    }

    #[test]
    fn differential_subtracts_two_complex_signals() {
        // Complex subtraction must work: this is what an AC sweep needs.
        let ds = dataset(
            "ac",
            Axis::Frequency(vec![1e3]),
            vec![
                Signal::complex("v(a)", VOLTAGE, vec![Complex::new(1.0, 2.0)]),
                Signal::complex("v(b)", VOLTAGE, vec![Complex::new(3.0, -1.0)]),
            ],
        );
        let value = eval(&Expr::differential("a", "b"), &ds).expect("both nodes are saved");
        assert!(value.is_complex());
        let samples = value.data.as_complex().expect("complex result");
        assert_close(samples[0].re, -2.0, 1e-12);
        assert_close(samples[0].im, 3.0, 1e-12);
    }

    #[test]
    fn differential_prefers_a_saved_differential_signal() {
        // If the backend saved `v(a,b)` directly, that signal is used as-is,
        // even though v(a) alone is not present.
        let ds = dataset(
            "tran",
            Axis::Time(vec![0.0, 1.0]),
            vec![Signal::real("v(a, b)", VOLTAGE, vec![0.5, 1.5])],
        );
        let value = eval(&Expr::differential("a", "b"), &ds).expect("saved differential probe");
        assert_eq!(value.as_real(), Some(&[0.5, 1.5][..]));
    }

    #[test]
    fn differential_promotes_mixed_real_and_complex() {
        let ds = dataset(
            "ac",
            Axis::Frequency(vec![1e3]),
            vec![
                Signal::complex("v(a)", VOLTAGE, vec![Complex::new(1.0, 1.0)]),
                Signal::real("v(b)", VOLTAGE, vec![1.0]),
            ],
        );
        let value = eval(&Expr::differential("a", "b"), &ds).expect("mixed operands promote");
        let samples = value.data.as_complex().expect("complex result");
        assert_close(samples[0].re, 0.0, 1e-12);
        assert_close(samples[0].im, 1.0, 1e-12);
    }

    #[test]
    fn subtracting_different_units_is_a_dimension_error() {
        let ds = dataset(
            "op",
            Axis::None,
            vec![
                Signal::real("v(a)", VOLTAGE, vec![1.0]),
                Signal::real("i(r1)", CURRENT, vec![1e-3]),
            ],
        );
        let err = eval(&(Expr::voltage("a") - Expr::current("r1")), &ds)
            .expect_err("V - A is not a quantity");
        assert_eq!(err.code, Code::Dimension);
        assert!(err.message.contains("V"), "{}", err.message);
        assert!(err.message.contains("A"), "{}", err.message);
        assert!(
            err.context.iter().any(|(k, v)| k == "expected" && v == "V")
                && err.context.iter().any(|(k, v)| k == "received" && v == "A"),
            "{:?}",
            err.context
        );
    }

    #[test]
    fn abs_of_a_real_signal_keeps_the_unit() {
        let ds = dataset(
            "tran",
            Axis::Time(vec![0.0, 1.0, 2.0]),
            vec![Signal::real("v(out)", VOLTAGE, vec![-1.0, 2.0, -3.0])],
        );
        let value = eval(&Expr::voltage("out").abs(), &ds).expect("abs works");
        assert_eq!(value.unit, VOLTAGE);
        assert_eq!(value.as_real(), Some(&[1.0, 2.0, 3.0][..]));
    }

    #[test]
    fn abs_of_a_complex_signal_is_the_magnitude() {
        let ds = dataset(
            "ac",
            Axis::Frequency(vec![1.0]),
            vec![Signal::complex(
                "v(out)",
                VOLTAGE,
                vec![Complex::new(3.0, 4.0)],
            )],
        );
        let value = eval(&Expr::voltage("out").abs(), &ds).expect("abs works");
        assert!(!value.is_complex());
        assert_eq!(value.unit, VOLTAGE);
        let real = value.as_real().expect("real magnitudes");
        assert_close(real[0], 5.0, 1e-12);
    }

    #[test]
    fn sqrt_halves_even_dimension_exponents() {
        let ds = dataset(
            "op",
            Axis::None,
            vec![Signal::real("p(r1)", Dimension::new(2, 0, 0), vec![9.0])],
        );
        let value = eval(&Expr::signal("p(r1)").sqrt(), &ds).expect("V^2 has a square root");
        assert_eq!(value.unit, VOLTAGE);
        let real = value.as_real().expect("real");
        assert_close(real[0], 3.0, 1e-12);
    }

    #[test]
    fn sqrt_of_an_odd_dimension_is_rejected() {
        let ds = real_op();
        let err = eval(&Expr::voltage("a").sqrt(), &ds).expect_err("sqrt(V) is not a unit");
        assert_eq!(err.code, Code::Dimension);
        assert!(err.message.contains('V'), "{}", err.message);
    }

    #[test]
    fn sqrt_of_complex_data_is_rejected_as_a_type_error() {
        let ds = dataset(
            "ac",
            Axis::Frequency(vec![1.0]),
            vec![Signal::complex("v(out)", VOLTAGE, vec![Complex::ONE])],
        );
        let err =
            eval(&Expr::voltage("out").sqrt(), &ds).expect_err("complex sqrt is out of scope");
        assert_eq!(err.code, Code::Type);
    }

    #[test]
    fn min_and_max_work_element_wise_on_real_vectors() {
        let ds = dataset(
            "tran",
            Axis::Time(vec![0.0, 1.0, 2.0]),
            vec![
                Signal::real("v(a)", VOLTAGE, vec![1.0, 5.0, 3.0]),
                Signal::real("v(b)", VOLTAGE, vec![2.0, 4.0, 4.0]),
            ],
        );
        let lo = eval(&Expr::min(Expr::voltage("a"), Expr::voltage("b")), &ds).expect("min");
        assert_eq!(lo.as_real(), Some(&[1.0, 4.0, 3.0][..]));
        let hi = eval(&Expr::max(Expr::voltage("a"), Expr::voltage("b")), &ds).expect("max");
        assert_eq!(hi.as_real(), Some(&[2.0, 5.0, 4.0][..]));
    }

    #[test]
    fn min_broadcasts_a_scalar_literal() {
        // A dimensionless literal broadcasts over a dimensionless vector, for
        // example clamping a gain curve at -6 dB.
        let ds = dataset(
            "tran",
            Axis::Time(vec![0.0, 1.0, 2.0]),
            vec![
                Signal::real("v(out)", VOLTAGE, vec![1.0, 10.0, 100.0]),
                Signal::real("v(in)", VOLTAGE, vec![1.0, 1.0, 1.0]),
            ],
        );
        let gain = Expr::gain_db(Expr::voltage("out"), Expr::voltage("in"));
        let clamped = Expr::max(gain, Expr::number(-6.0));
        let value = eval(&clamped, &ds).expect("both sides are dimensionless");
        assert_eq!(value.as_real(), Some(&[0.0, 20.0, 40.0][..]));

        // A bare number is dimensionless, so it cannot be mixed with volts.
        let err = eval(&Expr::max(Expr::voltage("out"), Expr::number(2.0)), &ds)
            .expect_err("V vs dimensionless");
        assert_eq!(err.code, Code::Dimension);
        assert!(err.message.contains("dimensionless"), "{}", err.message);
    }

    #[test]
    fn min_and_max_reject_complex_data() {
        let ds = dataset(
            "ac",
            Axis::Frequency(vec![1.0]),
            vec![Signal::complex("v(a)", VOLTAGE, vec![Complex::ONE])],
        );
        let err = eval(&Expr::max(Expr::voltage("a"), Expr::voltage("a")), &ds)
            .expect_err("complex comparison is not defined");
        assert_eq!(err.code, Code::Type);
        assert!(err.message.contains("real samples only"), "{}", err.message);
        assert!(
            err.notes.iter().any(|n| n.contains("abs")),
            "the note should point at abs(): {:?}",
            err.notes
        );
    }

    #[test]
    fn gain_db_of_a_known_ratio() {
        // 10x is 20 dB, 0.1x is -20 dB, 1x is 0 dB.
        let ds = dataset(
            "ac",
            Axis::Frequency(vec![1.0, 10.0, 100.0]),
            vec![
                Signal::real("v(out)", VOLTAGE, vec![10.0, 1.0, 0.5]),
                Signal::real("v(in)", VOLTAGE, vec![1.0, 1.0, 0.5]),
            ],
        );
        let value = eval(
            &Expr::gain_db(Expr::voltage("out"), Expr::voltage("in")),
            &ds,
        )
        .expect("both signals exist");
        assert_eq!(value.unit, DIMENSIONLESS);
        let db = value.as_real().expect("real");
        assert_close(db[0], 20.0, 1e-12);
        assert_close(db[1], 0.0, 1e-12);
        assert_close(db[2], 0.0, 1e-12);
    }

    #[test]
    fn gain_db_of_a_complex_ratio_uses_the_magnitude() {
        // (1+i)/(1-i) = i, so |ratio| = 1 and the gain is 0 dB.
        let ds = dataset(
            "ac",
            Axis::Frequency(vec![1.0]),
            vec![
                Signal::complex("v(out)", VOLTAGE, vec![Complex::new(1.0, 1.0)]),
                Signal::complex("v(in)", VOLTAGE, vec![Complex::new(1.0, -1.0)]),
            ],
        );
        let value = eval(
            &Expr::gain_db(Expr::voltage("out"), Expr::voltage("in")),
            &ds,
        )
        .expect("both signals exist");
        let db = value.as_real().expect("real");
        assert_close(db[0], 0.0, 1e-12);

        // Doubling the magnitude adds 6.0206 dB.
        let ds = dataset(
            "ac",
            Axis::Frequency(vec![1.0]),
            vec![
                Signal::complex("v(out)", VOLTAGE, vec![Complex::new(2.0, 2.0)]),
                Signal::complex("v(in)", VOLTAGE, vec![Complex::new(1.0, -1.0)]),
            ],
        );
        let value = eval(
            &Expr::gain_db(Expr::voltage("out"), Expr::voltage("in")),
            &ds,
        )
        .expect("both signals exist");
        assert_close(
            value.as_real().expect("real")[0],
            6.020_599_913_279_624,
            1e-9,
        );
    }

    #[test]
    fn gain_db_requires_a_dimensionless_ratio() {
        let ds = dataset(
            "ac",
            Axis::Frequency(vec![1.0]),
            vec![
                Signal::real("v(out)", VOLTAGE, vec![1.0]),
                Signal::real("i(r1)", CURRENT, vec![1e-3]),
            ],
        );
        let err = eval(
            &Expr::gain_db(Expr::voltage("out"), Expr::current("r1")),
            &ds,
        )
        .expect_err("V/A is a resistance, not a gain");
        assert_eq!(err.code, Code::Dimension);
        assert!(err.message.contains("dimensionless"), "{}", err.message);
        assert!(err.message.contains("V"), "{}", err.message);
        assert!(err.message.contains('A'), "{}", err.message);
    }

    #[test]
    fn multiplication_derives_dimensions() {
        // V / A = ohm, then * A returns volts.
        let ds = dataset(
            "op",
            Axis::None,
            vec![
                Signal::real("v(out)", VOLTAGE, vec![2.0]),
                Signal::real("i(r1)", CURRENT, vec![1e-3]),
            ],
        );
        let r = eval(&(Expr::voltage("out") / Expr::current("r1")), &ds).expect("ratio");
        assert_eq!(r.unit, circuit_core::units::RESISTANCE);
        assert_close(r.as_real().expect("real")[0], 2000.0, 1e-9);
    }

    #[test]
    fn negation_flips_real_and_complex_samples() {
        let ds = dataset(
            "ac",
            Axis::Frequency(vec![1.0]),
            vec![Signal::complex(
                "v(a)",
                VOLTAGE,
                vec![Complex::new(1.0, -2.0)],
            )],
        );
        let value = eval(&-Expr::voltage("a"), &ds).expect("negation");
        let z = value
            .data
            .as_complex()
            .expect("complex")
            .first()
            .expect("1");
        assert_close(z.re, -1.0, 1e-12);
        assert_close(z.im, 2.0, 1e-12);
    }

    #[test]
    fn mismatched_vector_lengths_are_reported() {
        // A hand-built (invalid) dataset: the shapes differ, so the evaluator
        // must report rather than panic or silently truncate.
        let ds = Dataset {
            experiment: "exp".into(),
            analysis: "an1".into(),
            kind: "tran".into(),
            axis: Axis::Time(vec![0.0, 1.0, 2.0]),
            signals: vec![
                Signal::real("v(a)", VOLTAGE, vec![0.0, 1.0, 2.0]),
                Signal::real("v(b)", VOLTAGE, vec![0.0, 1.0]),
            ],
            diagnostics: Vec::new(),
            backend: BackendInfo::new("test", "0"),
        };
        let err = eval(&Expr::differential("a", "b"), &ds).expect_err("3 samples vs 2");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("3 samples"), "{}", err.message);
        assert!(err.message.contains("2 samples"), "{}", err.message);
    }
}
