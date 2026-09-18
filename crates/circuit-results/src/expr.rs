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
//!
//! # Illegal samples
//!
//! Every operation validates the samples it produces, so an illegal value can
//! never be masked by an enclosing `min`/`max` or by a reduction. Each of
//! `number`, `signal`, `neg`, `abs`, `sqrt`, `min`, `max`, `+`, `-`, `*`, `/` and
//! `gain_db` refuses a non-finite result — `x.is_finite()` for real data and
//! both components for complex data — with a [Code::Value] error naming the
//! operation and the sub-expression that produced it. A signal sample that is
//! already NaN or an infinity is refused by the operation that reads it.
//!
//! A denominator that is exactly zero is a **definite error**, never an
//! infinity: `v(out) / v(in)` where `v(in)` is 0 at one sample, and
//! `gain_db(out, in)` where the ratio has zero magnitude (`log10(0)` is not
//! a dB value), both stop the evaluation, and `sqrt` of a negative real
//! sample is an error as well. The diagnostic names the analysis, the
//! operation, the offending expression and the sample coordinate; a scalar
//! expression (no axis) says so in a note. The tests are exact (`x < 0`,
//! `x == 0`) and no epsilon is applied, so tiny but non-zero denominators
//! and `-0.0` keep working. Nothing is saturated, truncated or skipped to make
//! an illegal expression look successful, and no `catch_unwind` hides a panic.
//!
//! # Sites and constants
//!
//! [eval_at] takes an [EvalSite] — the `derive`/`measure` name the user
//! wrote — and every diagnostic it produces carries that identity, so an error
//! inside an expression can be traced back to the definition that asked for it.
//! [is_constant] and [eval_constant] are the check-time pair for expressions
//! that read no signal: the same policy, without the analysis and sample
//! context a run would add.

use std::fmt;
use std::ops::{Add, Div, Mul, Neg, Sub};

use circuit_core::limits::MAX_EXPR_DEPTH;
use circuit_core::plan::ExprIr;
use circuit_core::units::{DIMENSIONLESS, Dimension};
use circuit_core::{Code, Diagnostic};

use crate::dataset::{Complex, Data, Dataset, Signal};
use crate::format_number;

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

    /// The number of nodes on the longest path from this node to a leaf.
    ///
    /// A leaf is one level deep, so `v(out)` is 1, `-v(out)` is 2 and a
    /// left-nested `a * b * c` chain is 3. This is the quantity
    /// [MAX_EXPR_DEPTH] bounds, and the parser measures the same thing on its
    /// own AST before an expression reaches this crate.
    ///
    /// The walk uses an explicit stack: it guards recursive passes, so measuring
    /// must not be able to overflow the stack itself (round-4 FINDING-1). It
    /// never short-circuits, so the caller gets the true depth for a diagnostic.
    pub fn depth(&self) -> usize {
        let mut deepest = 1usize;
        let mut stack: Vec<(&Expr, usize)> = vec![(self, 1)];
        while let Some((node, depth)) = stack.pop() {
            if depth > deepest {
                deepest = depth;
            }
            let child = depth + 1;
            match node {
                Expr::Number(_) | Expr::Signal(_) | Expr::Differential { .. } => {}
                Expr::Neg(inner) | Expr::Abs(inner) | Expr::Sqrt(inner) => {
                    stack.push((inner, child));
                }
                Expr::Add(a, b)
                | Expr::Sub(a, b)
                | Expr::Mul(a, b)
                | Expr::Div(a, b)
                | Expr::Min(a, b)
                | Expr::Max(a, b) => {
                    stack.push((a, child));
                    stack.push((b, child));
                }
                Expr::GainDb {
                    numerator,
                    denominator,
                } => {
                    stack.push((numerator, child));
                    stack.push((denominator, child));
                }
            }
        }
        deepest
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
            // Literals use the project's number formatter so a diagnostic can
            // quote `1e308` instead of a 309-digit expansion.
            Self::Number(x) => f.write_str(&format_number(*x)),
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
// Lowering the plan IR
// ---------------------------------------------------------------------------

/// Lower a plan-layer expression ([`ExprIr`]) into this crate's runtime AST.
///
/// The dependency direction is `core <- results <- session/backend/cli`:
/// `circuit-core` describes an expression that has not been evaluated yet and
/// must never depend on this crate, so the lowering lives here rather than as a
/// method on the IR.
///
/// Every probe the expression mentions is in the analysis task's read set (the
/// explicit `save` probes plus the expression's implicit dependencies), so a
/// probe becomes a plain signal lookup by the name the backend saves. A
/// differential probe arrives as the saved `v(a, b)` signal for the same
/// reason. If a name is missing anyway, [`eval`] reports it with the usual
/// "no signal named ... available signals: ..." diagnostic instead of
/// evaluating to anything.
pub fn from_ir(ir: &ExprIr) -> Expr {
    match ir {
        ExprIr::Number(x) => Expr::Number(*x),
        ExprIr::Probe(probe) => Expr::Signal(probe.name.clone()),
        ExprIr::Neg(inner) => Expr::Neg(Box::new(from_ir(inner))),
        ExprIr::Add(a, b) => Expr::Add(Box::new(from_ir(a)), Box::new(from_ir(b))),
        ExprIr::Sub(a, b) => Expr::Sub(Box::new(from_ir(a)), Box::new(from_ir(b))),
        ExprIr::Mul(a, b) => Expr::Mul(Box::new(from_ir(a)), Box::new(from_ir(b))),
        ExprIr::Div(a, b) => Expr::Div(Box::new(from_ir(a)), Box::new(from_ir(b))),
        ExprIr::Abs(inner) => Expr::Abs(Box::new(from_ir(inner))),
        ExprIr::Sqrt(inner) => Expr::Sqrt(Box::new(from_ir(inner))),
        ExprIr::Min(a, b) => Expr::Min(Box::new(from_ir(a)), Box::new(from_ir(b))),
        ExprIr::Max(a, b) => Expr::Max(Box::new(from_ir(a)), Box::new(from_ir(b))),
        ExprIr::GainDb {
            numerator,
            denominator,
        } => Expr::GainDb {
            numerator: Box::new(from_ir(numerator)),
            denominator: Box::new(from_ir(denominator)),
        },
    }
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

/// Which result definition asked for a value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EvalKind {
    /// A `derive :name, expr: ...` definition.
    Derive,
    /// A `measure :name, <kind>: ...` definition.
    Measure,
}

impl EvalKind {
    /// The keyword as written in the DSL.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Derive => "derive",
            Self::Measure => "measure",
        }
    }
}

/// Who asked for the value, for diagnostics: the definition's kind and the
/// name the user wrote.
///
/// A named site puts that identity into every diagnostic it produces
/// (`measure `masked`: ...`, and `signal` becomes the name), which is
/// what lets an error inside an expression be traced back to the
/// `derive`/`measure` line it came from. A hand-written expression has no
/// name: see [EvalSite::anonymous].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EvalSite {
    pub kind: EvalKind,
    pub name: String,
}

impl EvalSite {
    /// The site of `derive :name`.
    pub fn derive(name: impl Into<String>) -> Self {
        Self {
            kind: EvalKind::Derive,
            name: name.into(),
        }
    }

    /// The site of `measure :name`.
    pub fn measure(name: impl Into<String>) -> Self {
        Self {
            kind: EvalKind::Measure,
            name: name.into(),
        }
    }

    /// A site with no name: what [eval] uses for an expression that belongs to
    /// no definition, and what [eval_constant] uses at check time. Diagnostics
    /// never mention an anonymous site.
    pub fn anonymous() -> Self {
        Self {
            kind: EvalKind::Derive,
            name: String::new(),
        }
    }

    /// Whether the site names the definition it came from.
    pub fn is_named(&self) -> bool {
        !self.name.is_empty()
    }

    /// `derive `g`` or `measure `vrms``; `None` for an anonymous
    /// site.
    pub fn describe(&self) -> Option<String> {
        self.is_named()
            .then(|| format!("{} `{}`", self.kind.as_str(), self.name))
    }
}

/// Evaluate `expr` against `dataset` on behalf of no named definition.
///
/// Equivalent to `eval_at(expr, dataset, &EvalSite::anonymous())`; the
/// diagnostics are exactly the round-3 ones. The depth guard lives in
/// [eval_at], so this entry point is covered by one walk, not two.
pub fn eval(expr: &Expr, dataset: &Dataset) -> Result<Value, Diagnostic> {
    eval_at(expr, dataset, &EvalSite::anonymous())
}

/// Evaluate `expr` against `dataset` on behalf of `site`.
///
/// Every diagnostic the evaluation produces carries the site's identity (see
/// [EvalSite]). An expression deeper than [MAX_EXPR_DEPTH] is refused before
/// evaluation starts: everything below this point is recursive, and the parser
/// bounds the same shape on its own AST for the same reason.
pub fn eval_at(expr: &Expr, dataset: &Dataset, site: &EvalSite) -> Result<Value, Diagnostic> {
    check_depth(expr)?;
    Evaluator {
        dataset: Some(dataset),
        site,
    }
    .eval(expr)
}

/// Refuse an expression deeper than the parser accepts (round-4 FINDING-1).
///
/// This is defence in depth, not the only line: `cdsl` never hands this crate
/// an expression deeper than [MAX_EXPR_DEPTH] — the parser refuses it first,
/// with the same message — but [Expr] is public and can be built by hand, and
/// every pass below this point recurses once per level.
///
/// The message quotes numbers only on purpose: rendering the offending
/// expression, or its sub-expressions, is exactly what must not happen here.
fn check_depth(expr: &Expr) -> Result<(), Diagnostic> {
    let depth = expr.depth();
    if depth <= MAX_EXPR_DEPTH {
        return Ok(());
    }
    Err(Diagnostic::error(
        Code::Limit,
        format!(
            "expression is {depth} levels deep, which is deeper than the {MAX_EXPR_DEPTH} level limit"
        ),
    )
    .with_note(
        "the depth counts every operand and operator; split the expression into several named parameters to go deeper",
    ))
}

/// Whether `expr` reads no signal, so its value can be decided without a run.
///
/// A `Differential` reads saved probe signals, so it is not constant either.
pub fn is_constant(expr: &Expr) -> bool {
    match expr {
        Expr::Number(_) => true,
        Expr::Signal(_) | Expr::Differential { .. } => false,
        Expr::Neg(inner) | Expr::Abs(inner) | Expr::Sqrt(inner) => is_constant(inner),
        Expr::Add(a, b)
        | Expr::Sub(a, b)
        | Expr::Mul(a, b)
        | Expr::Div(a, b)
        | Expr::Min(a, b)
        | Expr::Max(a, b) => is_constant(a) && is_constant(b),
        Expr::GainDb {
            numerator,
            denominator,
        } => is_constant(numerator) && is_constant(denominator),
    }
}

/// Evaluate a constant expression at check time.
///
/// `Err` when the expression is not constant — it reads a signal, so only a
/// run can supply its samples — and when it is constant but illegal:
/// `sqrt(-1)`, a non-finite result (`1e308 * 1e308`), `x / 0` and
/// `gain_db(0, x)` all come back as the same diagnostic the runtime path
/// produces, minus the analysis identity and the sample coordinate that only
/// exist after a run. An expression deeper than [MAX_EXPR_DEPTH] is refused
/// before anything walks it.
pub fn eval_constant(expr: &Expr) -> Result<Value, Diagnostic> {
    // Refused first: the `is_constant` walk below is recursive too.
    check_depth(expr)?;
    if !is_constant(expr) {
        return Err(Diagnostic::error(
            Code::Value,
            format!(
                "`{expr}` is not a constant expression: it reads a signal, so its value is only known after a run"
            ),
        )
        .with_context("signal", expr.to_string())
        .with_note(
            "a constant expression reads no signal; a signal-dependent expression is checked statically and evaluated at run time",
        ));
    }
    let site = EvalSite::anonymous();
    Evaluator {
        dataset: None,
        site: &site,
    }
    .eval(expr)
}

/// The operation a diagnostic names, one variant per AST node that produces
/// samples.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Number,
    Signal,
    Differential,
    Neg,
    Abs,
    Sqrt,
    Min,
    Max,
    Add,
    Sub,
    Mul,
    Div,
    GainDb,
}

impl Op {
    /// The word a diagnostic uses for the operation.
    fn label(self) -> &'static str {
        match self {
            Self::Number => "number literal",
            Self::Signal => "signal read",
            Self::Differential => "differential probe",
            Self::Neg => "negation",
            Self::Abs => "abs",
            Self::Sqrt => "sqrt",
            Self::Min => "min",
            Self::Max => "max",
            Self::Add => "addition",
            Self::Sub => "subtraction",
            Self::Mul => "multiplication",
            Self::Div => "division",
            Self::GainDb => "gain_db",
        }
    }

    /// The operator glyph a diagnostic prints next to the word, if any.
    fn symbol(self) -> Option<&'static str> {
        match self {
            Self::Add => Some("+"),
            Self::Sub | Self::Neg => Some("-"),
            Self::Mul => Some("*"),
            Self::Div => Some("/"),
            _ => None,
        }
    }

    /// `multiplication `*``, `sqrt`, ...
    fn describe(self) -> String {
        match self.symbol() {
            Some(symbol) => format!("{} `{symbol}`", self.label()),
            None => self.label().to_string(),
        }
    }
}

/// One evaluation: an optional dataset, and the site it runs for.
///
/// `dataset` is `None` only for [eval_constant], which decides an expression
/// that reads no signal.
struct Evaluator<'a> {
    dataset: Option<&'a Dataset>,
    site: &'a EvalSite,
}

impl<'a> Evaluator<'a> {
    /// Evaluate one node, validating every sample it produces.
    fn eval(&self, expr: &Expr) -> Result<Value, Diagnostic> {
        match expr {
            Expr::Number(x) => Ok(Value {
                unit: DIMENSIONLESS,
                data: self.checked(Op::Number, expr, Data::Real(vec![*x]))?,
            }),
            Expr::Signal(name) => {
                let signal = lookup(self, name)?;
                Ok(Value {
                    unit: signal.unit,
                    data: self.checked(Op::Signal, expr, signal.data.clone())?,
                })
            }
            Expr::Differential { pos, neg } => self.eval_differential(pos, neg),
            Expr::Neg(inner) => {
                let value = self.eval(inner)?;
                Ok(Value {
                    unit: value.unit,
                    data: self.checked(Op::Neg, expr, negate(&value.data))?,
                })
            }
            Expr::Add(a, b) => self.eval_sum(a, b, BinOp::Add, expr),
            Expr::Sub(a, b) => self.eval_sum(a, b, BinOp::Sub, expr),
            Expr::Mul(a, b) => self.eval_product(a, b, BinOp::Mul, expr),
            Expr::Div(a, b) => self.eval_product(a, b, BinOp::Div, expr),
            Expr::Abs(inner) => {
                let value = self.eval(inner)?;
                let data = match value.data {
                    Data::Real(values) => Data::Real(values.into_iter().map(f64::abs).collect()),
                    Data::Complex(values) => {
                        Data::Real(values.into_iter().map(|z| z.magnitude()).collect())
                    }
                };
                Ok(Value {
                    unit: value.unit,
                    data: self.checked(Op::Abs, expr, data)?,
                })
            }
            Expr::Sqrt(inner) => self.eval_sqrt(inner, expr),
            Expr::Min(a, b) => self.eval_elementwise(a, b, Op::Min, expr, f64::min),
            Expr::Max(a, b) => self.eval_elementwise(a, b, Op::Max, expr, f64::max),
            Expr::GainDb {
                numerator,
                denominator,
            } => self.eval_gain_db(numerator, denominator, expr),
        }
    }

    /// Validate the samples one operation just produced.
    ///
    /// The check is per operation, never per expression: `min(sqrt(-1), 2)`
    /// fails at the `sqrt`, and an enclosing `min`/`max` cannot hide an
    /// illegal intermediate value. Real data needs `x.is_finite()`; complex
    /// data needs both components finite, so a non-finite real part, a
    /// non-finite imaginary part and a magnitude that overflows (`abs` of a
    /// huge complex sample) all go through the same rule.
    fn checked(&self, op: Op, expr: &Expr, data: Data) -> Result<Data, Diagnostic> {
        match first_non_finite(&data) {
            Some((index, detail)) => Err(self.sample_error(
                Code::Value,
                format!(
                    "{}{} produced a non-finite sample in `{expr}`{}: {detail}",
                    self.prefix(),
                    op.describe(),
                    self.location(index)
                ),
                &expr.to_string(),
                index,
            )),
            None => Ok(data),
        }
    }

    /// ` at time = 0.02 of analysis `tran1``, or nothing when there is no
    /// dataset to name.
    fn location(&self, index: usize) -> String {
        match self.dataset {
            Some(dataset) => format!(
                " at {} of analysis `{}`",
                sample_coordinate(dataset, index),
                dataset.analysis
            ),
            None => String::new(),
        }
    }

    /// `measure `masked`: ` for a named site; nothing for an anonymous
    /// one.
    fn prefix(&self) -> String {
        match self.site.describe() {
            Some(site) => format!("{site}: "),
            None => String::new(),
        }
    }

    /// The `signal` context: the site's name when it has one, and the
    /// sub-expression that produced the value otherwise.
    fn signal_name(&self, subexpr: &str) -> String {
        if self.site.is_named() {
            self.site.name.clone()
        } else {
            subexpr.to_string()
        }
    }

    /// What to say when there is no axis coordinate to report.
    fn axis_note(&self) -> Option<&'static str> {
        match self.dataset {
            Some(dataset) if dataset.axis.is_none() => Some(
                "this analysis has no axis: the value is one scalar sample, named by its index",
            ),
            None => Some(
                "no dataset and no analysis axis: this is one scalar sample of a constant expression",
            ),
            _ => None,
        }
    }

    /// A runtime error about one sample of one analysis, carrying the site.
    ///
    /// The context pairs are what `Diagnostic::render_plain` prints as
    /// `= key: value` lines: the analysis, the kind of analysis, the site
    /// (or the offending sub-expression), the sample coordinate and the index
    /// are the minimum a user needs to find the offending point in a long
    /// sweep.
    fn sample_error(&self, code: Code, message: String, subexpr: &str, index: usize) -> Diagnostic {
        let mut diagnostic = Diagnostic::error(code, message);
        if let Some(dataset) = self.dataset {
            diagnostic = diagnostic
                .with_context("analysis", dataset.analysis.clone())
                .with_context("kind", dataset.kind.clone());
        }
        diagnostic = diagnostic.with_context("signal", self.signal_name(subexpr));
        if let Some(dataset) = self.dataset {
            diagnostic = diagnostic.with_context("sample", sample_coordinate(dataset, index));
        }
        diagnostic = diagnostic.with_context("index", index.to_string());
        match self.axis_note() {
            Some(note) => diagnostic.with_note(note),
            None => diagnostic,
        }
    }

    /// An error about the operation as a whole — a dimension no exponent can
    /// hold — carrying the site but no sample.
    fn site_error(&self, code: Code, message: String, subexpr: &str) -> Diagnostic {
        let mut diagnostic = Diagnostic::error(code, message);
        if let Some(dataset) = self.dataset {
            diagnostic = diagnostic
                .with_context("analysis", dataset.analysis.clone())
                .with_context("kind", dataset.kind.clone());
        }
        diagnostic.with_context("signal", self.signal_name(subexpr))
    }

    /// The dimension of `a op b`, or a Code::Dimension diagnostic when an
    /// exponent leaves the `i8` range.
    ///
    /// The exponents stay `i8` (round 4, R4-02) and the unchecked arithmetic
    /// is gone, so a long product of voltage factors is reported here instead
    /// of panicking in debug or wrapping in release. `None` is never
    /// unwrapped: a user-reachable product/quotient has to survive an
    /// arbitrarily long expression, so the only honest answer for an
    /// unrepresentable exponent is a diagnostic. The message names the
    /// operation and both operand dimensions, like the language's own evaluator
    /// does at check time.
    fn checked_dimension(
        &self,
        op: BinOp,
        a: &Expr,
        va: &Value,
        b: &Expr,
        vb: &Value,
    ) -> Result<Dimension, Diagnostic> {
        let checked = match op {
            BinOp::Mul => va.unit.checked_mul(vb.unit),
            BinOp::Div => va.unit.checked_div(vb.unit),
            _ => unreachable!("only * and / derive a dimension"),
        };
        checked.ok_or_else(|| {
            let subexpr = format!("({a} {} {b})", op.symbol());
            self.site_error(
                Code::Dimension,
                format!(
                    "`{}` on a value in {} and one in {} has a dimension outside the representable range",
                    self.prefix() + op.symbol(),
                    va.unit,
                    vb.unit
                ),
                &subexpr,
            )
            .with_dims(va.unit, vb.unit)
            .with_note(format!(
                "dimension exponents are 8-bit: the exponents of {} and {} would leave {}..={}, so the operation is reported instead of wrapping",
                va.unit,
                vb.unit,
                Dimension::MIN_EXPONENT,
                Dimension::MAX_EXPONENT
            ))
        })
    }
}

/// Look up a signal, or explain which signals exist.
///
/// The error names the missing probe and lists what is available. A named site
/// prefixes the message with the `derive`/`measure` it came from; the
/// `signal` context stays the name that was looked up, which is the missing
/// one.
fn lookup<'a>(evaluator: &Evaluator<'a>, name: &str) -> Result<&'a Signal, Diagnostic> {
    let Some(dataset) = evaluator.dataset else {
        return Err(Diagnostic::error(
            Code::Name,
            format!("no signal named `{name}`: a constant expression reads no signal"),
        )
        .with_context("signal", name.to_string()));
    };
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
                "{}no signal named `{name}` in analysis `{}`",
                evaluator.prefix(),
                dataset.analysis
            ),
        )
        .with_context("analysis", dataset.analysis.clone())
        .with_context("kind", dataset.kind.clone())
        .with_context("signal", name.to_string())
        .with_note(format!("available signals: {available}"))
    })
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

    /// The sample-policy operation this arithmetic maps onto.
    fn policy_op(self) -> Op {
        match self {
            Self::Add => Op::Add,
            Self::Sub => Op::Sub,
            Self::Mul => Op::Mul,
            Self::Div => Op::Div,
        }
    }
}

impl<'a> Evaluator<'a> {
    /// `v(pos) - v(neg)`: the saved differential probe when the backend made
    /// one, otherwise the difference of the two node signals.
    fn eval_differential(&self, pos: &str, neg: &str) -> Result<Value, Diagnostic> {
        // A saved differential probe is already the difference; use it verbatim.
        let combined = format!("v({pos},{neg})");
        if let Some(signal) = self.dataset.and_then(|dataset| dataset.signal(&combined)) {
            let probe = Expr::Signal(combined);
            return Ok(Value {
                unit: signal.unit,
                data: self.checked(Op::Signal, &probe, signal.data.clone())?,
            });
        }

        let pos_name = format!("v({pos})");
        let neg_name = format!("v({neg})");
        let a = lookup(self, &pos_name)?;
        let b = lookup(self, &neg_name)?;
        if a.unit != b.unit {
            return Err(self.dimension_error(
                "-",
                &Expr::Signal(pos_name),
                &Value::from_signal(a),
                &Expr::Signal(neg_name),
                &Value::from_signal(b),
            ));
        }
        // Real - real is real; anything else promotes to complex.
        let node = Expr::Differential {
            pos: pos.to_string(),
            neg: neg.to_string(),
        };
        let data = combine(&a.data, &b.data, |x, y| x - y, |x, y| x - y)?;
        Ok(Value {
            unit: a.unit,
            data: self.checked(Op::Differential, &node, data)?,
        })
    }

    /// `a + b` and `a - b`: one common unit, element-wise, validated after
    /// the combination.
    fn eval_sum(&self, a: &Expr, b: &Expr, op: BinOp, node: &Expr) -> Result<Value, Diagnostic> {
        let va = self.eval(a)?;
        let vb = self.eval(b)?;
        if va.unit != vb.unit {
            return Err(self.dimension_error(op.symbol(), a, &va, b, &vb));
        }
        let data = match op {
            BinOp::Add => combine(&va.data, &vb.data, |x, y| x + y, |x, y| x + y)?,
            BinOp::Sub => combine(&va.data, &vb.data, |x, y| x - y, |x, y| x - y)?,
            _ => unreachable!("eval_sum is only called with + and -"),
        };
        Ok(Value {
            unit: va.unit,
            data: self.checked(op.policy_op(), node, data)?,
        })
    }

    /// `a * b` and `a / b`: the dimension comes from the checked form, the
    /// samples are validated afterwards.
    fn eval_product(
        &self,
        a: &Expr,
        b: &Expr,
        op: BinOp,
        node: &Expr,
    ) -> Result<Value, Diagnostic> {
        let va = self.eval(a)?;
        let vb = self.eval(b)?;
        let (unit, data) = match op {
            BinOp::Mul => (
                self.checked_dimension(op, a, &va, b, &vb)?,
                combine(&va.data, &vb.data, |x, y| x * y, |x, y| x * y)?,
            ),
            BinOp::Div => {
                // Checked before dividing: the IEEE result of a zero
                // denominator is +/-inf (or NaN for 0/0), and a reduction would
                // then carry it into a measurement or an exported column. An
                // illegal value is a diagnostic, never a plausible number.
                let n = broadcast_len(va.data.len(), vb.data.len())?;
                if let Some(index) = first_zero_sample(&vb.data, n) {
                    return Err(self.zero_denominator_error(a, b, index));
                }
                (
                    self.checked_dimension(op, a, &va, b, &vb)?,
                    combine(&va.data, &vb.data, |x, y| x / y, |x, y| x / y)?,
                )
            }
            _ => unreachable!("eval_product is only called with * and /"),
        };
        Ok(Value {
            unit,
            data: self.checked(op.policy_op(), node, data)?,
        })
    }

    /// `sqrt(x)`: real data, even exponents, and no negative sample.
    fn eval_sqrt(&self, inner: &Expr, node: &Expr) -> Result<Value, Diagnostic> {
        let value = self.eval(inner)?;
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
        // A negative sample has no real square root, and `f64::sqrt` would
        // quietly return NaN that an enclosing min/max could then hide. The
        // test is exact (`x < 0`): `-0.0` and tiny positive samples keep
        // working, and nothing is saturated, truncated or skipped.
        if let Some(index) = values.iter().position(|x| *x < 0.0) {
            return Err(self
                .sample_error(
                    Code::Value,
                    format!(
                        "{}sqrt of a negative sample in `{node}`{}: `{inner}` is {} there, and a negative real number has no real square root",
                        self.prefix(),
                        self.location(index),
                        format_number(values[index])
                    ),
                    &inner.to_string(),
                    index,
                )
                .with_note(
                    "sqrt is defined for samples >= 0: the illegal sample is reported, never evaluated to NaN",
                ));
        }
        let data = Data::Real(values.iter().map(|x| x.sqrt()).collect());
        Ok(Value {
            unit,
            data: self.checked(Op::Sqrt, node, data)?,
        })
    }

    /// `min(a, b)` / `max(a, b)`, element-wise on real data.
    fn eval_elementwise(
        &self,
        a: &Expr,
        b: &Expr,
        op: Op,
        node: &Expr,
        apply: fn(f64, f64) -> f64,
    ) -> Result<Value, Diagnostic> {
        let va = self.eval(a)?;
        let vb = self.eval(b)?;
        if va.unit != vb.unit {
            return Err(self.dimension_error(op.label(), a, &va, b, &vb));
        }
        if va.is_complex() || vb.is_complex() {
            return Err(Diagnostic::error(
                Code::Type,
                format!(
                    "`{}({a}, {b})` is defined for real samples only",
                    op.label()
                ),
            )
            .with_note("apply abs(...) to a complex signal to compare magnitudes"));
        }
        let data = combine(&va.data, &vb.data, apply, |x, y| {
            if apply(x.magnitude(), y.magnitude()) == x.magnitude() {
                x
            } else {
                y
            }
        })?;
        Ok(Value {
            unit: va.unit,
            data: self.checked(op, node, data)?,
        })
    }

    /// `20*log10(abs(numerator / denominator))`.
    fn eval_gain_db(
        &self,
        numerator: &Expr,
        denominator: &Expr,
        node: &Expr,
    ) -> Result<Value, Diagnostic> {
        let va = self.eval(numerator)?;
        let vb = self.eval(denominator)?;
        if va.unit != vb.unit {
            return Err(
                Diagnostic::error(
                    Code::Dimension,
                    format!(
                        "{}gain `{numerator} / {denominator}` must be dimensionless, but the units are {} and {}",
                        self.prefix(),
                        va.unit,
                        vb.unit
                    ),
                )
                .with_dims(va.unit, vb.unit)
                .with_note("dB is defined for a ratio of like quantities (out/in)"),
            );
        }
        // A zero denominator of the ratio is the same division-by-zero error as
        // `/`, and a ratio of zero magnitude has no dB value: log10(0) is -inf,
        // which must never reach a measurement or an exported column.
        let n = broadcast_len(va.data.len(), vb.data.len())?;
        if let Some(index) = first_zero_sample(&vb.data, n) {
            return Err(self.zero_denominator_error(numerator, denominator, index));
        }
        let ratio = combine(&va.data, &vb.data, |x, y| x / y, |x, y| x / y)?;
        if let Some(index) = first_zero_sample(&ratio, n) {
            return Err(self.zero_gain_error(numerator, denominator, index));
        }
        let db = Data::Real(
            ratio
                .magnitudes()
                .into_iter()
                .map(|m| 20.0 * m.log10())
                .collect(),
        );
        Ok(Value {
            unit: DIMENSIONLESS,
            data: self.checked(Op::GainDb, node, db)?,
        })
    }

    /// `+`, `-`, `min` and `max` require one common unit.
    fn dimension_error(&self, op: &str, a: &Expr, va: &Value, b: &Expr, vb: &Value) -> Diagnostic {
        self.site_error(
            Code::Dimension,
            format!(
                "{}cannot apply `{op}` to `{a}` ({}) and `{b}` ({}): the units differ",
                self.prefix(),
                va.unit,
                vb.unit
            ),
            &format!("({a} {op} {b})"),
        )
        .with_dims(va.unit, vb.unit)
        .with_note("+, -, min and max require both sides to have the same unit")
    }

    /// Division by zero: the denominator is exactly zero at one sample.
    fn zero_denominator_error(
        &self,
        numerator: &Expr,
        denominator: &Expr,
        index: usize,
    ) -> Diagnostic {
        self.sample_error(
            Code::Value,
            format!(
                "{}division by zero in `({numerator} / {denominator})`{}: `{denominator}` is 0",
                self.prefix(),
                self.location(index)
            ),
            &denominator.to_string(),
            index,
        )
        .with_note(
            "no epsilon is applied: a zero denominator is reported, never evaluated to an infinity",
        )
    }

    /// `gain_db` at a sample whose ratio has zero magnitude: log10(0) is -inf,
    /// which is not a dB value.
    fn zero_gain_error(&self, numerator: &Expr, denominator: &Expr, index: usize) -> Diagnostic {
        self.sample_error(
            Code::Value,
            format!(
                "{}gain_db({numerator}, {denominator}) has no dB value{}: the magnitude of the ratio is 0, and log10(0) is -inf",
                self.prefix(),
                self.location(index)
            ),
            &numerator.to_string(),
            index,
        )
        .with_note(
            "dB is logarithmic: a zero-magnitude sample has no finite dB value, so it is reported instead of exporting -inf",
        )
    }
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

/// The first sample of `data` that is not finite, with a description of what
/// is wrong with it.
///
/// Real data has one component; complex data needs *both* components finite, so
/// a non-finite imaginary part is the same class of error as a non-finite real
/// part, and the detail says which component is which.
fn first_non_finite(data: &Data) -> Option<(usize, String)> {
    match data {
        Data::Real(values) => values.iter().position(|x| !x.is_finite()).map(|index| {
            (
                index,
                format!("the sample is {}", number_detail(values[index])),
            )
        }),
        Data::Complex(values) => values.iter().enumerate().find_map(|(index, z)| {
            let mut problems = Vec::new();
            if !z.re.is_finite() {
                problems.push(format!("the real part is {}", number_detail(z.re)));
            }
            if !z.im.is_finite() {
                problems.push(format!("the imaginary part is {}", number_detail(z.im)));
            }
            (!problems.is_empty()).then(|| (index, problems.join(" and ")))
        }),
    }
}

/// `NaN`, `+inf` or `-inf`, for a diagnostic about one non-finite value.
fn number_detail(x: f64) -> &'static str {
    if x.is_nan() {
        "NaN"
    } else if x.is_sign_positive() {
        "+inf"
    } else {
        "-inf"
    }
}

/// The coordinate of sample `index`, for diagnostics.
///
/// An analysis with an axis names the sample by its axis value (`time = 0.02`);
/// an operating point has no independent variable, so the index is all there is
/// to name.
fn sample_coordinate(dataset: &Dataset, index: usize) -> String {
    match dataset.axis.samples().get(index) {
        Some(x) => format!("{} = {}", dataset.axis.kind_name(), format_number(*x)),
        None => format!("sample {index}"),
    }
}

/// The first broadcast sample index at which `data` is exactly zero.
///
/// `n` is the length of the combination the caller is about to make; a length-1
/// operand repeats over every sample, exactly as `combine` does. The test is
/// exact equality, never a threshold: a tiny but non-zero denominator must keep
/// working, and no epsilon may turn it into an error.
fn first_zero_sample(data: &Data, n: usize) -> Option<usize> {
    let at = |i: usize| if data.len() == 1 && n > 1 { 0 } else { i };
    (0..n).find(|&i| match data {
        Data::Real(values) => values.get(at(i)).is_some_and(|x| *x == 0.0),
        Data::Complex(values) => values.get(at(i)).is_some_and(|z| z.is_zero()),
    })
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
    use circuit_core::id::{DeviceId, NodeId};
    use circuit_core::plan::{ExprIr, Probe, ProbeRef};
    use circuit_core::span::SourceSpan;
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

    /// Like `dataset`, but with a chosen analysis identity, so the tests that
    /// assert a diagnostic naming the analysis use a name a user would see.
    fn dataset_in(analysis: &str, kind: &str, axis: Axis, signals: Vec<Signal>) -> Dataset {
        Dataset::new(
            "exp",
            analysis,
            kind,
            axis,
            signals,
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("test dataset must be well formed")
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
            implicit_only: Vec::new(),
            diagnostics: Vec::new(),
            backend: BackendInfo::new("test", "0"),
        };
        let err = eval(&Expr::differential("a", "b"), &ds).expect_err("3 samples vs 2");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("3 samples"), "{}", err.message);
        assert!(err.message.contains("2 samples"), "{}", err.message);
    }

    fn context_value(err: &Diagnostic, key: &str) -> Option<String> {
        err.context
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }

    fn probe_ir(name: &str, probe: Probe) -> ExprIr {
        ExprIr::Probe(ProbeRef::new(name, probe, SourceSpan::synthetic()))
    }

    /// Round 3: the plan layer describes the expression, the results layer
    /// lowers it. Every IR variant must land on the variant the evaluator
    /// implements, including a differential probe, which arrives as the name
    /// of the saved differential signal.
    #[test]
    fn from_ir_lowers_every_variant() {
        let a = || probe_ir("v(a)", Probe::NodeVoltage(NodeId(1)));
        let b = || probe_ir("v(b)", Probe::NodeVoltage(NodeId(2)));
        let current = || probe_ir("i(r1)", Probe::DeviceCurrent(DeviceId(0)));
        let differential = || {
            probe_ir(
                "v(a,b)",
                Probe::DifferentialVoltage {
                    pos: NodeId(1),
                    neg: NodeId(2),
                },
            )
        };

        assert_eq!(from_ir(&ExprIr::Number(2.5)), Expr::Number(2.5));
        assert_eq!(from_ir(&a()), Expr::Signal("v(a)".into()));
        assert_eq!(from_ir(&b()), Expr::Signal("v(b)".into()));
        assert_eq!(from_ir(&current()), Expr::Signal("i(r1)".into()));
        assert_eq!(from_ir(&differential()), Expr::Signal("v(a,b)".into()));
        assert_eq!(from_ir(&ExprIr::Neg(Box::new(a()))), -Expr::voltage("a"));
        assert_eq!(
            from_ir(&ExprIr::Add(Box::new(a()), Box::new(b()))),
            Expr::voltage("a") + Expr::voltage("b")
        );
        assert_eq!(
            from_ir(&ExprIr::Sub(Box::new(a()), Box::new(b()))),
            Expr::voltage("a") - Expr::voltage("b")
        );
        assert_eq!(
            from_ir(&ExprIr::Mul(Box::new(a()), Box::new(b()))),
            Expr::voltage("a") * Expr::voltage("b")
        );
        assert_eq!(
            from_ir(&ExprIr::Div(Box::new(a()), Box::new(b()))),
            Expr::voltage("a") / Expr::voltage("b")
        );
        assert_eq!(
            from_ir(&ExprIr::Abs(Box::new(a()))),
            Expr::voltage("a").abs()
        );
        assert_eq!(
            from_ir(&ExprIr::Sqrt(Box::new(a()))),
            Expr::voltage("a").sqrt()
        );
        assert_eq!(
            from_ir(&ExprIr::Min(Box::new(a()), Box::new(b()))),
            Expr::min(Expr::voltage("a"), Expr::voltage("b"))
        );
        assert_eq!(
            from_ir(&ExprIr::Max(Box::new(a()), Box::new(b()))),
            Expr::max(Expr::voltage("a"), Expr::voltage("b"))
        );
        assert_eq!(
            from_ir(&ExprIr::GainDb {
                numerator: Box::new(a()),
                denominator: Box::new(b()),
            }),
            Expr::gain_db(Expr::voltage("a"), Expr::voltage("b"))
        );
    }

    #[test]
    fn from_ir_lowers_into_evaluable_expressions() {
        let ds = dataset(
            "ac",
            Axis::Frequency(vec![1e3]),
            vec![
                Signal::real("v(out)", VOLTAGE, vec![2.0]),
                Signal::real("v(in)", VOLTAGE, vec![1.0]),
            ],
        );
        let ir = ExprIr::GainDb {
            numerator: Box::new(probe_ir("v(out)", Probe::NodeVoltage(NodeId(2)))),
            denominator: Box::new(probe_ir("v(in)", Probe::NodeVoltage(NodeId(1)))),
        };
        let value = eval(&from_ir(&ir), &ds).expect("both signals are saved");
        assert_close(
            value.as_real().expect("real")[0],
            6.020_599_913_279_624,
            1e-12,
        );

        // A probe outside the read set keeps the existing Name diagnostic:
        // the read set is the session's job, the evaluator still reports what
        // it cannot find.
        let missing = ExprIr::Probe(ProbeRef::new(
            "v(gone)",
            Probe::NodeVoltage(NodeId(9)),
            SourceSpan::synthetic(),
        ));
        let err = eval(&from_ir(&missing), &ds).expect_err("not in the read set");
        assert_eq!(err.code, Code::Name);
        assert!(err.message.contains("v(gone)"), "{}", err.message);
        assert!(
            err.notes.iter().any(|n| n.contains("available signals")),
            "{:?}",
            err.notes
        );
    }

    /// Round 3: a zero denominator is a definite error naming the analysis,
    /// the expression and the sample coordinate, not an infinity.
    #[test]
    fn division_by_zero_is_a_definite_error_with_analysis_and_sample() {
        let ds = dataset_in(
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1.0, 2.0]),
            vec![
                Signal::real("v(out)", VOLTAGE, vec![1.0, 2.0, 3.0]),
                Signal::real("v(in)", VOLTAGE, vec![1.0, 1.0, 0.0]),
            ],
        );
        let err = eval(&(Expr::voltage("out") / Expr::voltage("in")), &ds)
            .expect_err("v(in) is zero at the last sample");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("division by zero"), "{}", err.message);
        assert!(err.message.contains("v(in)"), "{}", err.message);
        assert!(err.message.contains("tran1"), "{}", err.message);
        assert!(err.message.contains("time = 2"), "{}", err.message);
        assert_eq!(context_value(&err, "analysis").as_deref(), Some("tran1"));
        assert_eq!(context_value(&err, "kind").as_deref(), Some("tran"));
        assert_eq!(context_value(&err, "signal").as_deref(), Some("v(in)"));
        assert_eq!(context_value(&err, "sample").as_deref(), Some("time = 2"));
        assert_eq!(context_value(&err, "index").as_deref(), Some("2"));
        // The value is a diagnostic, not an infinity to carry onwards.
        assert!(!err.message.contains("inf"), "{}", err.message);
    }

    #[test]
    fn division_by_zero_without_an_axis_names_the_index() {
        let ds = dataset_in(
            "op1",
            "op",
            Axis::None,
            vec![
                Signal::real("v(out)", VOLTAGE, vec![2.0]),
                Signal::real("v(in)", VOLTAGE, vec![0.0]),
            ],
        );
        let err = eval(&(Expr::voltage("out") / Expr::voltage("in")), &ds)
            .expect_err("an operating point has only a sample index");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("sample 0"), "{}", err.message);
        assert_eq!(context_value(&err, "sample").as_deref(), Some("sample 0"));
        assert_eq!(context_value(&err, "index").as_deref(), Some("0"));
        assert_eq!(context_value(&err, "analysis").as_deref(), Some("op1"));
    }

    #[test]
    fn division_by_zero_covers_broadcast_and_complex_samples() {
        // A zero literal broadcasts over the whole numerator; the first
        // broadcast sample is where it is reported.
        let ds = dataset_in(
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1.0]),
            vec![Signal::real("v(out)", VOLTAGE, vec![1.0, 2.0])],
        );
        let err = eval(&(Expr::voltage("out") / Expr::number(0.0)), &ds)
            .expect_err("a dimensionless zero is still zero");
        assert_eq!(err.code, Code::Value);
        assert_eq!(context_value(&err, "sample").as_deref(), Some("time = 0"));

        // Complex division by zero is the same error, not a NaN sample.
        let ds = dataset_in(
            "ac1",
            "ac",
            Axis::Frequency(vec![100.0]),
            vec![
                Signal::complex("v(out)", VOLTAGE, vec![Complex::new(1.0, 1.0)]),
                Signal::complex("v(in)", VOLTAGE, vec![Complex::ZERO]),
            ],
        );
        let err =
            eval(&(Expr::voltage("out") / Expr::voltage("in")), &ds).expect_err("0 + 0i is zero");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("division by zero"), "{}", err.message);
        assert_eq!(
            context_value(&err, "sample").as_deref(),
            Some("frequency = 100")
        );
    }

    /// No epsilon: a tiny denominator is a denominator.
    #[test]
    fn tiny_but_non_zero_denominators_keep_working() {
        let ds = dataset_in(
            "op1",
            "op",
            Axis::None,
            vec![Signal::real("v(out)", VOLTAGE, vec![1.0])],
        );
        let value =
            eval(&(Expr::voltage("out") / Expr::number(1e-300)), &ds).expect("1e-300 is not zero");
        assert_close(value.as_real().expect("real")[0], 1e300, 1e290);

        let ds = dataset_in(
            "ac1",
            "ac",
            Axis::Frequency(vec![1e3]),
            vec![
                Signal::real("v(out)", VOLTAGE, vec![1e-300]),
                Signal::real("v(in)", VOLTAGE, vec![1e-300]),
            ],
        );
        let value = eval(
            &Expr::gain_db(Expr::voltage("out"), Expr::voltage("in")),
            &ds,
        )
        .expect("a tiny but non-zero ratio has a dB value");
        assert_close(value.as_real().expect("real")[0], 0.0, 1e-12);
    }

    /// Round 3: log10(0) is -inf, and -inf must not reach a measurement or an
    /// exported column.
    #[test]
    fn gain_db_of_a_zero_magnitude_is_a_definite_error() {
        let ds = dataset_in(
            "ac1",
            "ac",
            Axis::Frequency(vec![100.0, 200.0]),
            vec![
                Signal::real("v(out)", VOLTAGE, vec![1.0, 0.0]),
                Signal::real("v(in)", VOLTAGE, vec![1.0, 1.0]),
            ],
        );
        let err = eval(
            &Expr::gain_db(Expr::voltage("out"), Expr::voltage("in")),
            &ds,
        )
        .expect_err("the ratio is zero at the last sample");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("log10(0)"), "{}", err.message);
        assert!(err.message.contains("v(out)"), "{}", err.message);
        assert!(err.message.contains("ac1"), "{}", err.message);
        assert!(err.message.contains("frequency = 200"), "{}", err.message);
        assert_eq!(context_value(&err, "analysis").as_deref(), Some("ac1"));
        assert_eq!(context_value(&err, "signal").as_deref(), Some("v(out)"));
        assert_eq!(
            context_value(&err, "sample").as_deref(),
            Some("frequency = 200")
        );
        assert_eq!(context_value(&err, "index").as_deref(), Some("1"));
        assert!(
            err.notes.iter().any(|n| n.contains("-inf")),
            "{:?}",
            err.notes
        );

        // A zero denominator is reported as the division it is.
        let ds = dataset_in(
            "ac1",
            "ac",
            Axis::Frequency(vec![100.0]),
            vec![
                Signal::real("v(out)", VOLTAGE, vec![1.0]),
                Signal::real("v(in)", VOLTAGE, vec![0.0]),
            ],
        );
        let err = eval(
            &Expr::gain_db(Expr::voltage("out"), Expr::voltage("in")),
            &ds,
        )
        .expect_err("a zero denominator is not an infinite gain");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("division by zero"), "{}", err.message);

        // A complex zero numerator has zero magnitude too.
        let ds = dataset_in(
            "ac1",
            "ac",
            Axis::Frequency(vec![100.0]),
            vec![
                Signal::complex("v(out)", VOLTAGE, vec![Complex::ZERO]),
                Signal::complex("v(in)", VOLTAGE, vec![Complex::ONE]),
            ],
        );
        let err = eval(
            &Expr::gain_db(Expr::voltage("out"), Expr::voltage("in")),
            &ds,
        )
        .expect_err("|0 + 0i| is 0");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("log10(0)"), "{}", err.message);
    }

    // -----------------------------------------------------------------------
    // Round 4: the value policy, the sites, and constant evaluation
    // -----------------------------------------------------------------------

    /// A transient dataset whose second sample is a negative V^2 value, so
    /// `sqrt` has a coordinate and a dimension it can halve.
    fn negative_transient() -> Dataset {
        dataset_in(
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 0.02]),
            vec![Signal::real(
                "p(r1)",
                Dimension::new(2, 0, 0),
                vec![4.0, -4.0],
            )],
        )
    }

    /// `sqrt(-1)` used to evaluate to NaN, which an enclosing `min` could
    /// then hide: the domain rule is exact and it fails at the `sqrt`.
    #[test]
    fn sqrt_of_a_negative_sample_is_a_value_error() {
        let ds = negative_transient();
        let err = eval_at(
            &Expr::signal("p(r1)").sqrt(),
            &ds,
            &EvalSite::measure("vrms"),
        )
        .expect_err("sqrt(-4 V^2) is not a real voltage");
        assert_eq!(err.code, Code::Value, "{}", err.render_plain());
        let text = err.render_plain();
        assert!(text.contains("sqrt"), "{text}");
        assert!(text.contains("time = 0.02"), "the sample is named: {text}");
        assert!(text.contains("measure `vrms`"), "the site is named: {text}");
        assert_eq!(context_value(&err, "analysis").as_deref(), Some("tran1"));
        assert_eq!(context_value(&err, "kind").as_deref(), Some("tran"));
        assert_eq!(context_value(&err, "signal").as_deref(), Some("vrms"));
        assert_eq!(
            context_value(&err, "sample").as_deref(),
            Some("time = 0.02")
        );
        assert_eq!(context_value(&err, "index").as_deref(), Some("1"));
    }

    #[test]
    fn sqrt_of_minus_one_is_a_value_error_not_a_nan() {
        let ds = real_op();
        let err = eval(&Expr::number(-1.0).sqrt(), &ds).expect_err("sqrt(-1) has no real value");
        assert_eq!(err.code, Code::Value, "{}", err.render_plain());
        assert!(err.message.contains("sqrt"), "{}", err.message);
        assert!(err.message.contains("-1"), "{}", err.message);
        assert!(err.message.contains("negative"), "{}", err.message);
        // Without a site the signal context stays the offending operand,
        // exactly as in round 3 (the denominator of a division, the inner
        // expression of a sqrt).
        assert_eq!(
            context_value(&err, "signal").as_deref(),
            Some("-1"),
            "an anonymous site names the operand that produced the value"
        );
        // The operating point has no axis, and the diagnostic says so.
        assert!(
            err.notes.iter().any(|n| n.contains("scalar")),
            "a scalar expression must say so: {:?}",
            err.notes
        );
    }

    /// The illegal intermediate value fails the expression: an enclosing
    /// `min`/`max` cannot return the other operand.
    #[test]
    fn an_illegal_intermediate_value_cannot_be_masked_by_min_or_max() {
        let ds = real_op();
        let illegal = || Expr::number(-1.0).sqrt();
        for nested in [
            Expr::min(illegal(), Expr::number(2.0)),
            Expr::max(illegal(), Expr::number(2.0)),
            Expr::max(
                Expr::min(Expr::min(illegal(), Expr::number(2.0)), Expr::number(1.0)),
                Expr::number(3.0),
            ),
        ] {
            let err = eval(&nested, &ds).expect_err("the clamp must not rescue an illegal operand");
            assert_eq!(err.code, Code::Value, "{}", err.render_plain());
            assert!(err.message.contains("sqrt"), "{}", err.message);
            assert!(
                !err.message.contains("min(") && !err.message.contains("max("),
                "the failure is reported at the sqrt, not at the clamp: {}",
                err.message
            );
        }
    }

    /// Arithmetic overflow is the same class of error as a domain violation:
    /// the result is not representable as a sample.
    #[test]
    fn a_non_finite_arithmetic_result_is_a_value_error() {
        let ds = real_op();
        let product = Expr::number(1e308) * Expr::number(1e308);
        let err = eval(&product, &ds).expect_err("1e308 * 1e308 overflows to +inf");
        assert_eq!(err.code, Code::Value, "{}", err.render_plain());
        let text = err.render_plain();
        assert!(
            text.contains("multiplication"),
            "the operation is named: {text}"
        );
        assert!(text.contains("+inf"), "the value is named: {text}");
        assert!(
            text.contains("1e308"),
            "a huge literal is quoted compactly: {text}"
        );

        // The nested clamp cannot hide it either.
        let err = eval(&Expr::max(product, Expr::number(2.0)), &ds)
            .expect_err("max(+inf, 2) must not return 2");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("multiplication"), "{}", err.message);

        // Addition overflows with the same policy.
        let err =
            eval(&(Expr::number(1e308) + Expr::number(1e308)), &ds).expect_err("the sum is +inf");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("addition"), "{}", err.message);

        // And so does a quotient whose result leaves the double range.
        let err = eval(&(Expr::number(1.0) / Expr::number(1e-320)), &ds)
            .expect_err("1.0 / 1e-320 overflows");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("division"), "{}", err.message);
    }

    /// A sample that is already non-finite is refused by the operation that
    /// reads it, whatever produces it: a saved signal, an `abs`, or a
    /// literal.
    #[test]
    fn non_finite_signal_samples_are_refused_by_the_read() {
        let ds = dataset_in(
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 0.02]),
            vec![Signal::real("v(bad)", VOLTAGE, vec![1.0, f64::INFINITY])],
        );
        let err = eval_at(&Expr::voltage("bad"), &ds, &EvalSite::derive("g"))
            .expect_err("an infinite sample is not a value");
        assert_eq!(err.code, Code::Value, "{}", err.render_plain());
        let text = err.render_plain();
        assert!(text.contains("signal read"), "{text}");
        assert!(text.contains("v(bad)"), "{text}");
        assert!(text.contains("+inf"), "{text}");
        assert!(text.contains("time = 0.02"), "{text}");
        assert!(text.contains("derive `g`"), "{text}");
        assert_eq!(context_value(&err, "analysis").as_deref(), Some("tran1"));
        assert_eq!(context_value(&err, "kind").as_deref(), Some("tran"));
        assert_eq!(context_value(&err, "signal").as_deref(), Some("g"));
        assert_eq!(
            context_value(&err, "sample").as_deref(),
            Some("time = 0.02")
        );
        assert_eq!(context_value(&err, "index").as_deref(), Some("1"));

        // abs() cannot launder it: the read fails first.
        let err = eval(&Expr::voltage("bad").abs(), &ds).expect_err("abs(inf) is still inf");
        assert_eq!(err.code, Code::Value);

        // A NaN literal is refused by the number node itself.
        let err = eval(&Expr::number(f64::NAN), &ds).expect_err("NaN is not a sample");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("number literal"), "{}", err.message);
        assert!(err.message.contains("NaN"), "{}", err.message);
    }

    /// Complex data follows the same policy: both components are validated,
    /// and an overflowing complex product is reported, never carried.
    #[test]
    fn non_finite_complex_components_and_complex_overflow_share_one_policy() {
        let ds = dataset_in(
            "ac1",
            "ac",
            Axis::Frequency(vec![100.0]),
            vec![Signal::complex(
                "v(out)",
                VOLTAGE,
                vec![Complex::new(1.0, f64::NAN)],
            )],
        );
        let err = eval(&Expr::voltage("out"), &ds).expect_err("a NaN imaginary part is refused");
        assert_eq!(err.code, Code::Value, "{}", err.render_plain());
        assert!(
            err.message.contains("imaginary part is NaN"),
            "{}",
            err.message
        );

        // (1e308 + 1e308i) * (1 + i) has real part 1e308 - 1e308 = 0 and
        // imaginary part 1e308 + 1e308 = +inf.
        let ds = dataset_in(
            "ac1",
            "ac",
            Axis::Frequency(vec![100.0]),
            vec![
                Signal::complex("v(a)", VOLTAGE, vec![Complex::new(1e308, 1e308)]),
                Signal::complex("v(b)", VOLTAGE, vec![Complex::new(1.0, 1.0)]),
            ],
        );
        let err = eval(&(Expr::voltage("a") * Expr::voltage("b")), &ds)
            .expect_err("the complex product is not representable");
        assert_eq!(err.code, Code::Value);
        assert!(
            err.message.contains("imaginary part is +inf"),
            "{}",
            err.message
        );

        // A magnitude that overflows is the same error: hypot(MAX, MAX) is
        // +inf even though both components are finite, so abs() reports it.
        let ds = dataset_in(
            "ac1",
            "ac",
            Axis::Frequency(vec![100.0]),
            vec![Signal::complex(
                "v(c)",
                VOLTAGE,
                vec![Complex::new(f64::MAX, f64::MAX)],
            )],
        );
        let read = eval(&Expr::voltage("c"), &ds).expect("both components are finite");
        assert!(read.is_complex(), "the read itself is legal");
        let err = eval(&Expr::voltage("c").abs(), &ds).expect_err("the magnitude overflows");
        assert_eq!(err.code, Code::Value, "{}", err.render_plain());
        assert!(err.message.contains("abs"), "{}", err.message);
        assert!(err.message.contains("+inf"), "{}", err.message);
    }

    /// The runtime dimension guard: V^127 is representable, V^128 is not, and
    /// the same for the negative edge. This exercises the `None` branch of
    /// `Dimension::checked_mul`/`checked_div` on a user-written expression.
    #[test]
    fn a_128_factor_voltage_product_is_a_dimension_error_at_runtime() {
        let ds = dataset_in(
            "op1",
            "op",
            Axis::None,
            vec![Signal::real("v(in)", VOLTAGE, vec![1.0])],
        );
        let product = |factors: usize| {
            let mut expr = Expr::signal("v(in)");
            for _ in 1..factors {
                expr = expr * Expr::signal("v(in)");
            }
            expr
        };

        // 127 factors are V^127: the largest exponent the representation holds.
        let value = eval(&product(127), &ds).expect("V^127 is representable");
        assert_eq!(value.unit, Dimension::new(127, 0, 0));
        assert_eq!(value.as_real(), Some(&[1.0][..]));

        // The 128th factor has nowhere to go: a diagnostic, not a panic and
        // not a wrapped exponent.
        let err = eval_at(&product(128), &ds, &EvalSite::derive("over"))
            .expect_err("V^128 leaves the exponent range");
        assert_eq!(err.code, Code::Dimension, "{}", err.render_plain());
        let text = err.render_plain();
        assert!(
            text.contains("dimension outside the representable range"),
            "{text}"
        );
        assert!(
            text.contains("V^127"),
            "the operands are named by dimension: {text}"
        );
        assert!(text.contains("derive `over`"), "{text}");
        assert_eq!(context_value(&err, "analysis").as_deref(), Some("op1"));
        assert_eq!(context_value(&err, "signal").as_deref(), Some("over"));

        // The negative edge is the `checked_div` branch: V^-128 is the
        // smallest exponent, and one more division leaves the range.
        let mut quotient = Expr::number(1.0);
        for _ in 0..128 {
            quotient = quotient / Expr::signal("v(in)");
        }
        let value = eval(&quotient, &ds).expect("V^-128 is representable");
        assert_eq!(value.unit, Dimension::new(-128, 0, 0));
        let err = eval(&(quotient / Expr::signal("v(in)")), &ds)
            .expect_err("V^-129 leaves the exponent range");
        assert_eq!(err.code, Code::Dimension, "{}", err.render_plain());
    }

    /// `eval` keeps its public behaviour: it is `eval_at` with an anonymous
    /// site, and a named site only adds text.
    #[test]
    fn eval_is_eval_at_with_an_anonymous_site() {
        let ds = real_op();
        let expression = Expr::voltage("a") - Expr::voltage("b");
        let plain = eval(&expression, &ds).expect("eval works");
        assert_eq!(
            plain,
            eval_at(&expression, &ds, &EvalSite::anonymous()).expect("eval_at")
        );
        assert_eq!(
            plain,
            eval_at(&expression, &ds, &EvalSite::derive("vdiff")).expect("a named site")
        );

        let zero = Expr::voltage("a") / Expr::number(0.0);
        let anon = eval(&zero, &ds).expect_err("zero denominator");
        let named = eval_at(&zero, &ds, &EvalSite::derive("g")).expect_err("zero denominator");
        assert!(
            named.message.starts_with("derive `g`: "),
            "{}",
            named.message
        );
        assert!(
            named.message.ends_with(&anon.message),
            "a site only prefixes the text: {} vs {}",
            named.message,
            anon.message
        );
        assert_eq!(anon.code, named.code);
        assert_eq!(context_value(&anon, "signal").as_deref(), Some("0"));
        assert_eq!(context_value(&named, "signal").as_deref(), Some("g"));
    }

    #[test]
    fn is_constant_sees_through_every_node() {
        assert!(is_constant(&Expr::number(2.0)));
        assert!(is_constant(&Expr::number(9.0).sqrt()));
        assert!(is_constant(&(Expr::number(1.0) / Expr::number(0.5))));
        assert!(is_constant(&Expr::gain_db(
            Expr::number(2.0),
            Expr::number(1.0)
        )));
        assert!(is_constant(&Expr::min(
            Expr::number(1.0).abs(),
            -Expr::number(2.0)
        )));

        assert!(!is_constant(&Expr::voltage("out")));
        assert!(!is_constant(&Expr::differential("a", "b")));
        assert!(!is_constant(&(Expr::number(2.0) * Expr::voltage("out"))));
        assert!(!is_constant(&Expr::gain_db(
            Expr::voltage("out"),
            Expr::number(1.0)
        )));
        assert!(!is_constant(&Expr::voltage("out").sqrt()));
    }

    #[test]
    fn eval_constant_evaluates_a_constant_and_rejects_a_signal() {
        // sqrt(9) = 3, with no dataset at all.
        let value = eval_constant(&Expr::number(9.0).sqrt()).expect("sqrt(9) is constant");
        assert_eq!(value.unit, DIMENSIONLESS);
        assert_eq!(value.as_real(), Some(&[3.0][..]));

        // gain_db(10, 1) = 20 dB.
        let value =
            eval_constant(&Expr::gain_db(Expr::number(10.0), Expr::number(1.0))).expect("20 dB");
        assert_close(value.as_real().expect("real")[0], 20.0, 1e-12);

        // A signal-dependent expression cannot be decided before a run.
        let err = eval_constant(&Expr::voltage("out")).expect_err("not constant");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("not a constant"), "{}", err.message);
        assert!(err.message.contains("v(out)"), "{}", err.message);
    }

    #[test]
    fn eval_constant_reports_the_same_violations_as_a_run() {
        // The constant text is the runtime text minus the analysis/sample
        // context, so check and run report the same error.
        let negative_sqrt = Expr::number(-1.0).sqrt();
        let constant = eval_constant(&negative_sqrt).expect_err("sqrt(-1)");
        let ds = dataset_in(
            "op1",
            "op",
            Axis::None,
            vec![Signal::real("v(a)", VOLTAGE, vec![1.0])],
        );
        let runtime = eval(&negative_sqrt, &ds).expect_err("sqrt(-1)");
        assert_eq!(constant.code, runtime.code);
        assert_eq!(constant.code, Code::Value);
        assert_eq!(
            runtime
                .message
                .replace(" at sample 0 of analysis `op1`", ""),
            constant.message,
            "the constant text is the runtime text minus its context"
        );
        assert!(constant.message.contains("sqrt"), "{}", constant.message);

        // 1e308 * 1e308, x/0 and gain_db(0) are refused at check time too.
        let err = eval_constant(&(Expr::number(1e308) * Expr::number(1e308)))
            .expect_err("the product overflows");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("multiplication"), "{}", err.message);

        let err = eval_constant(&(Expr::number(1.0) / Expr::number(0.0))).expect_err("x/0");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("division by zero"), "{}", err.message);

        let err = eval_constant(&Expr::gain_db(Expr::number(0.0), Expr::number(1.0)))
            .expect_err("a zero ratio has no dB value");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("log10(0)"), "{}", err.message);

        // No dataset: the diagnostic says the value is one scalar sample, and
        // it claims no analysis identity.
        assert!(
            err.notes.iter().any(|n| n.contains("scalar")),
            "{:?}",
            err.notes
        );
        assert!(
            !err.context.iter().any(|(key, _)| key == "analysis"),
            "{:?}",
            err.context
        );
    }

    // -----------------------------------------------------------------------
    // Round 4, FINDING-1: the evaluator depth guard
    // -----------------------------------------------------------------------

    /// `levels` nested negations around a leaf, so the tree is `levels` deep
    /// for `levels >= 1` (`neg_chain(1)` is the bare leaf). Built in a loop:
    /// constructing a deep tree must not need a deep stack either.
    fn neg_chain(levels: usize) -> Expr {
        let mut expr = Expr::signal("v(a)");
        for _ in 1..levels {
            expr = -expr;
        }
        expr
    }

    /// Run `f` on the kind of stack `cdsl` evaluates on
    /// (`circuit-cli/src/main.rs` uses 64 MiB): a deep but legal expression
    /// needs that room, and a test thread's default stack is much smaller.
    fn on_worker_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(f)
            .expect("the worker thread starts")
            .join()
            .expect("the worker thread must not panic")
    }

    #[test]
    fn depth_counts_every_operand_and_operator() {
        assert_eq!(Expr::number(1.0).depth(), 1);
        assert_eq!(Expr::signal("v(a)").depth(), 1);
        assert_eq!(Expr::differential("a", "b").depth(), 1);
        assert_eq!((-Expr::signal("v(a)")).depth(), 2);
        assert_eq!(Expr::min(Expr::number(1.0), Expr::number(2.0)).depth(), 2);
        assert_eq!(Expr::number(4.0).sqrt().abs().depth(), 3);
        assert_eq!(
            Expr::gain_db(Expr::signal("v(a)"), Expr::signal("v(b)")).depth(),
            2
        );

        // A flat chain is one level per operand, not a bush: this left-nested
        // product of four leaves is four levels deep.
        let flat = Expr::signal("v(a)") * Expr::number(1.0) * Expr::number(2.0) * Expr::number(3.0);
        assert_eq!(flat.depth(), 4);

        // The numbers the guard compares against.
        assert_eq!(neg_chain(1).depth(), 1);
        assert_eq!(neg_chain(2).depth(), 2);
        assert_eq!(neg_chain(MAX_EXPR_DEPTH).depth(), MAX_EXPR_DEPTH);
        assert_eq!(neg_chain(MAX_EXPR_DEPTH + 1).depth(), MAX_EXPR_DEPTH + 1);
    }

    #[test]
    fn a_chain_at_the_limit_still_evaluates() {
        let expr = neg_chain(MAX_EXPR_DEPTH);
        assert_eq!(expr.depth(), MAX_EXPR_DEPTH, "the limit itself is allowed");
        // 255 negations above the leaf: the sign flips an odd number of times,
        // so +3 V becomes -3 V.
        let value = on_worker_stack(move || {
            let ds = real_op();
            eval(&expr, &ds).expect("depth equal to the limit must evaluate")
        });
        assert_eq!(value.unit, VOLTAGE);
        assert_eq!(value.as_real(), Some(&[-3.0][..]));
    }

    #[test]
    fn a_chain_one_past_the_limit_is_refused_before_evaluation() {
        let ds = real_op();
        let expr = neg_chain(MAX_EXPR_DEPTH + 1);
        let expected = format!(
            "expression is {} levels deep, which is deeper than the {MAX_EXPR_DEPTH} level limit",
            MAX_EXPR_DEPTH + 1
        );

        let err = eval(&expr, &ds).expect_err("one level over the limit");
        assert_eq!(err.code, Code::Limit, "{}", err.render_plain());
        assert_eq!(err.message, expected);
        assert!(
            !err.message.contains("v(a)"),
            "the guard must not render the deep expression: {}",
            err.message
        );

        // The other entry points refuse it the same way. In particular
        // `eval_constant` checks the depth before `is_constant`, which walks the
        // tree recursively.
        let err = eval_at(&expr, &ds, &EvalSite::derive("deep")).expect_err("refused");
        assert_eq!(err.code, Code::Limit);
        assert_eq!(err.message, expected);
        let err = eval_constant(&expr).expect_err("refused before the constant walk");
        assert_eq!(err.code, Code::Limit);
    }

    /// The guard is the thing that must not overflow: it measures 100_000
    /// levels and refuses them without recursing, on a stack a recursive walk
    /// could not survive.
    #[test]
    fn the_depth_guard_itself_never_recurses() {
        let (depth, code, message) = on_worker_stack(|| {
            let ds = real_op();
            let mut expr = Expr::signal("v(a)");
            for _ in 1..100_000 {
                expr = -expr;
            }
            let depth = expr.depth();
            let error = eval(&expr, &ds).expect_err("100_000 levels is far over the limit");
            (depth, error.code, error.message)
        });
        assert_eq!(depth, 100_000);
        assert_eq!(code, Code::Limit);
        assert_eq!(
            message,
            format!(
                "expression is 100000 levels deep, which is deeper than the {MAX_EXPR_DEPTH} level limit"
            ),
            "the message names the observed depth and the limit, and nothing else"
        );
    }

    /// The depth guard must not change the cases the value and dimension rules
    /// already decide: a 128-factor product is still `E_DIMENSION` (its depth,
    /// 128, is inside the limit), 127 factors still evaluate.
    #[test]
    fn the_depth_guard_leaves_the_dimension_priority_alone() {
        let ds = dataset_in(
            "op1",
            "op",
            Axis::None,
            vec![Signal::real("v(in)", VOLTAGE, vec![1.0])],
        );
        let product = |factors: usize| {
            let mut expr = Expr::signal("v(in)");
            for _ in 1..factors {
                expr = expr * Expr::signal("v(in)");
            }
            expr
        };

        let expr = product(128);
        assert_eq!(expr.depth(), 128, "a flat product is one level per operand");
        let err = eval(&expr, &ds).expect_err("V^128 is not representable");
        assert_eq!(err.code, Code::Dimension, "{}", err.render_plain());
        assert_ne!(
            err.code,
            Code::Limit,
            "128 levels are inside the limit, so the dimension rule decides"
        );

        let value = eval(&product(127), &ds).expect("V^127 is representable");
        assert_eq!(value.unit, Dimension::new(127, 0, 0));
        assert_eq!(value.as_real(), Some(&[1.0][..]));

        // A chain longer than the limit is refused by the depth guard instead
        // (the parser refuses the same shape written in source), and never
        // aborts.
        let err = eval(&product(300), &ds).expect_err("300 levels is over the limit");
        assert_eq!(err.code, Code::Limit, "{}", err.render_plain());
    }
}
