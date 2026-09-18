//! Abstract syntax tree for the DSL.
//!
//! The AST is a *syntax* tree: names are unresolved strings, every node keeps
//! its source span, and no dimensional checking has happened yet. Elaboration
//! turns this into `circuit_core::ir::Circuit` plus `circuit_core::plan::AnalysisPlan`.

use circuit_core::span::SourceSpan;
use circuit_core::units::Dimension;

use crate::token::QuantityLiteral;

/// A whole source file.
#[derive(Clone, Debug, Default)]
pub struct Program {
    /// Top-level circuits and subcircuits, in source order.
    pub circuits: Vec<CircuitDef>,
    pub experiments: Vec<ExperimentDef>,
}

impl Program {
    pub fn circuit(&self, name: &str) -> Option<&CircuitDef> {
        self.circuits.iter().find(|c| c.name == name)
    }

    pub fn experiment(&self, name: &str) -> Option<&ExperimentDef> {
        self.experiments.iter().find(|e| e.name.name == name)
    }
}

// ---------------------------------------------------------------------------
// Circuit definitions
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct CircuitDef {
    pub name: String,
    /// `true` for `subcircuit`, `false` for `circuit`.
    pub is_subcircuit: bool,
    /// Declared ports. Non-empty only for subcircuits.
    pub ports: Vec<SpannedName>,
    pub body: Vec<Stmt>,
    pub span: SourceSpan,
    pub name_span: SourceSpan,
}

/// A name with the span it was written at.
///
/// A name is usually a literal (`:r1` or `"r1"`). It may also be a
/// parenthesised expression, which is what lets a loop generate distinct
/// device names — the brief requires such names to be deterministic and
/// unique, and without this a loop could only ever declare one device.
#[derive(Clone, Debug)]
pub struct SpannedName {
    /// Literal text. Empty when `expr` is present.
    pub name: String,
    /// `Some` when the name is computed, e.g. `resistor ("r" + i), ...`.
    pub expr: Option<Box<Expr>>,
    pub span: SourceSpan,
}

impl SpannedName {
    pub fn new(name: impl Into<String>, span: SourceSpan) -> Self {
        Self {
            name: name.into(),
            expr: None,
            span,
        }
    }

    /// A name computed from an expression.
    pub fn expressed(expr: Expr, span: SourceSpan) -> Self {
        Self {
            name: String::new(),
            expr: Some(Box::new(expr)),
            span,
        }
    }

    /// Whether this name is a plain literal.
    pub fn is_literal(&self) -> bool {
        self.expr.is_none()
    }

    /// The literal text, if this is a literal name.
    pub fn literal(&self) -> Option<&str> {
        if self.expr.is_none() {
            Some(self.name.as_str())
        } else {
            None
        }
    }
}

/// A statement inside a `circuit` or `subcircuit` body.
#[derive(Clone, Debug)]
pub enum Stmt {
    Param(ParamDecl),
    Node(NodeDecl),
    Device(DeviceStmt),
    Model(ModelDecl),
    Instance(InstanceStmt),
    For(ForStmt),
    If(IfStmt),
}

impl Stmt {
    pub fn span(&self) -> SourceSpan {
        match self {
            Stmt::Param(s) => s.span,
            Stmt::Node(s) => s.span,
            Stmt::Device(s) => s.span,
            Stmt::Model(s) => s.span,
            Stmt::Instance(s) => s.span,
            Stmt::For(s) => s.span,
            Stmt::If(s) => s.span,
        }
    }

    /// Statement keyword, for diagnostics.
    pub fn keyword(&self) -> &'static str {
        match self {
            Stmt::Param(_) => "param",
            Stmt::Node(_) => "node",
            Stmt::Device(d) => match d.kind {
                DeviceStmtKind::Resistor => "resistor",
                DeviceStmtKind::Capacitor => "capacitor",
                DeviceStmtKind::Inductor => "inductor",
                DeviceStmtKind::VoltageSource => "voltage_source",
                DeviceStmtKind::CurrentSource => "current_source",
                DeviceStmtKind::Diode => "diode",
            },
            Stmt::Model(_) => "model",
            Stmt::Instance(_) => "instance",
            Stmt::For(_) => "for",
            Stmt::If(_) => "if",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ParamDecl {
    pub name: SpannedName,
    /// `default:` expression. `None` means the value must be supplied by an
    /// instance or experiment override.
    pub default: Option<Expr>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug)]
pub struct NodeDecl {
    pub names: Vec<SpannedName>,
    pub span: SourceSpan,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceStmtKind {
    Resistor,
    Capacitor,
    Inductor,
    VoltageSource,
    CurrentSource,
    Diode,
}

impl DeviceStmtKind {
    pub fn keyword(self) -> &'static str {
        match self {
            DeviceStmtKind::Resistor => "resistor",
            DeviceStmtKind::Capacitor => "capacitor",
            DeviceStmtKind::Inductor => "inductor",
            DeviceStmtKind::VoltageSource => "voltage_source",
            DeviceStmtKind::CurrentSource => "current_source",
            DeviceStmtKind::Diode => "diode",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DeviceStmt {
    pub kind: DeviceStmtKind,
    pub name: SpannedName,
    /// Named arguments in source order, e.g. `p:`, `n:`, `value:`.
    /// Duplicates are a semantic error, checked during elaboration.
    pub args: Vec<Arg>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug)]
pub struct ModelDecl {
    pub name: SpannedName,
    pub args: Vec<Arg>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug)]
pub struct InstanceStmt {
    pub name: SpannedName,
    /// `of: :subcircuit_name`.
    pub of: SpannedName,
    /// `ports: { name: node_symbol, ... }`, in source order.
    pub ports: Vec<DictEntry>,
    /// `params: { name: value, ... }`, in source order.
    pub params: Vec<DictEntry>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug)]
pub struct ForStmt {
    pub var: SpannedName,
    pub iter: ForIter,
    pub body: Vec<Stmt>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug)]
pub enum ForIter {
    /// `for i in [a, b, c]`
    List(Expr),
    /// `for i in a..b`, inclusive of both ends.
    Range { start: Expr, end: Expr },
}

#[derive(Clone, Debug)]
pub struct IfStmt {
    /// `if`/`elsif` arms, at least one.
    pub arms: Vec<(Expr, Vec<Stmt>)>,
    pub else_body: Option<Vec<Stmt>>,
    pub span: SourceSpan,
}

// ---------------------------------------------------------------------------
// Experiments
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ExperimentDef {
    pub name: SpannedName,
    /// `circuit: :name`.
    pub circuit: SpannedName,
    pub body: Vec<ExpStmt>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug)]
pub enum ExpStmt {
    Op {
        span: SourceSpan,
    },
    Dc(AnalysisCall),
    Ac(AnalysisCall),
    Tran(AnalysisCall),
    /// `save v(:a), v(:b), i(:d)`
    Save {
        probes: Vec<Expr>,
        span: SourceSpan,
    },
    /// `param :r, value: 2.kohm` — experiment-level override.
    Param {
        name: SpannedName,
        value: Expr,
        span: SourceSpan,
    },
    /// `measure :name, max: v(:out)`, optionally `max: <expr>, analysis: :ac1`.
    Measure {
        name: SpannedName,
        /// `max` / `min` / `avg` / `rms`
        kind: String,
        kind_span: SourceSpan,
        target: Expr,
        /// `analysis: :ac1`: the analysis the target is evaluated on.
        ///
        /// `None` leaves the choice to elaboration: one analysis in the
        /// experiment binds to it, several make the statement ambiguous, and a
        /// lone probe read keeps the documented legacy search order.
        analysis: Option<SpannedName>,
        span: SourceSpan,
    },
    /// `derive :name, expr: <result-expr>`, optionally `, analysis: :ac1`.
    ///
    /// A derived signal is a new column computed from probe reads. It is not a
    /// parameter, and in this version it cannot be referenced by another
    /// expression (see `docs/language.md`).
    Derive {
        name: SpannedName,
        expr: Expr,
        /// `analysis: :ac1`, as on `measure`.
        analysis: Option<SpannedName>,
        span: SourceSpan,
    },
}

impl ExpStmt {
    pub fn span(&self) -> SourceSpan {
        match self {
            ExpStmt::Op { span }
            | ExpStmt::Save { span, .. }
            | ExpStmt::Param { span, .. }
            | ExpStmt::Measure { span, .. }
            | ExpStmt::Derive { span, .. } => *span,
            ExpStmt::Dc(c) | ExpStmt::Ac(c) | ExpStmt::Tran(c) => c.span,
        }
    }

    pub fn keyword(&self) -> &'static str {
        match self {
            ExpStmt::Op { .. } => "op",
            ExpStmt::Dc(_) => "dc",
            ExpStmt::Ac(_) => "ac",
            ExpStmt::Tran(_) => "tran",
            ExpStmt::Save { .. } => "save",
            ExpStmt::Param { .. } => "param",
            ExpStmt::Measure { .. } => "measure",
            ExpStmt::Derive { .. } => "derive",
        }
    }
}

/// An analysis statement with its named arguments.
#[derive(Clone, Debug)]
pub struct AnalysisCall {
    pub args: Vec<Arg>,
    pub span: SourceSpan,
}

impl AnalysisCall {
    pub fn arg(&self, name: &str) -> Option<&Arg> {
        self.args.iter().find(|a| a.name == name)
    }

    pub fn args_named(&self, name: &str) -> impl Iterator<Item = &Arg> {
        self.args.iter().filter(move |a| a.name == name)
    }

    /// Argument names present, in source order, for "unknown argument"
    /// diagnostics that list what was actually given.
    pub fn arg_names(&self) -> Vec<&str> {
        self.args.iter().map(|a| a.name.as_str()).collect()
    }
}

// ---------------------------------------------------------------------------
// Arguments and expressions
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Arg {
    pub name: String,
    pub name_span: SourceSpan,
    pub value: Expr,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnaryOp {
    Neg,
    Pos,
    Not,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl BinaryOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::And => "&&",
            BinaryOp::Or => "||",
        }
    }

    /// Whether this operator compares rather than computes.
    pub fn is_comparison(self) -> bool {
        matches!(
            self,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        )
    }

    pub fn is_logical(self) -> bool {
        matches!(self, BinaryOp::And | BinaryOp::Or)
    }
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Quantity(QuantityLiteral),
    Bool(bool),
    Str(String),
    Symbol(String),
    Array(Vec<Expr>),
    /// Dictionary literal; keys are bare identifiers.
    Dict(Vec<DictEntry>),
    /// A bare identifier: a parameter reference.
    Var(String),
    Unary {
        op: UnaryOp,
        rhs: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Call(Call),
}

/// An expression together with the source text it came from.
///
/// Carrying the span on the wrapper (rather than on selected variants) means
/// every sub-expression can be pointed at, which is what the brief requires
/// for locatable dimension and name errors.
#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: SourceSpan,
}

#[derive(Clone, Debug)]
pub struct DictEntry {
    pub key: String,
    pub key_span: SourceSpan,
    pub value: Expr,
}

#[derive(Clone, Debug)]
pub struct Call {
    pub name: String,
    pub name_span: SourceSpan,
    /// Arguments without a label, e.g. `v(:vin)` or `sqrt(2)`.
    pub positional: Vec<Expr>,
    /// Arguments with a label, e.g. `pulse(low: 0.V, ...)`.
    pub named: Vec<Arg>,
    pub span: SourceSpan,
}

impl Call {
    pub fn arg(&self, name: &str) -> Option<&Arg> {
        self.named.iter().find(|a| a.name == name)
    }
}

impl Expr {
    pub fn new(kind: ExprKind, span: SourceSpan) -> Self {
        Self { kind, span }
    }

    pub fn span(&self) -> SourceSpan {
        self.span
    }

    /// Convenience for building a dimensionless literal in tests.
    pub fn quantity(value: f64, dimension: Dimension) -> Self {
        Expr::new(
            ExprKind::Quantity(QuantityLiteral {
                value,
                dimension,
                text: format!("{value}"),
            }),
            SourceSpan::synthetic(),
        )
    }

    /// Convenience for building a symbol in tests.
    pub fn symbol(name: &str) -> Self {
        Expr::new(ExprKind::Symbol(name.to_string()), SourceSpan::synthetic())
    }

    pub fn var(name: &str) -> Self {
        Expr::new(ExprKind::Var(name.to_string()), SourceSpan::synthetic())
    }

    /// Whether this expression is a bare symbol, returning its name.
    pub fn as_symbol(&self) -> Option<&str> {
        match &self.kind {
            ExprKind::Symbol(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Whether this expression is a dictionary literal.
    pub fn as_dict(&self) -> Option<&[DictEntry]> {
        match &self.kind {
            ExprKind::Dict(d) => Some(d.as_slice()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circuit_core::units::{FREQUENCY, RESISTANCE};

    fn sp() -> SourceSpan {
        SourceSpan::synthetic()
    }

    #[test]
    fn program_lookup_by_name() {
        let p = Program {
            circuits: vec![CircuitDef {
                name: "rc".into(),
                is_subcircuit: false,
                ports: Vec::new(),
                body: Vec::new(),
                span: sp(),
                name_span: sp(),
            }],
            experiments: vec![ExperimentDef {
                name: SpannedName::new("x", sp()),
                circuit: SpannedName::new("rc", sp()),
                body: Vec::new(),
                span: sp(),
            }],
        };
        assert!(p.circuit("rc").is_some());
        assert!(p.circuit("nope").is_none());
        assert!(p.experiment("x").is_some());
    }

    #[test]
    fn analysis_call_finds_named_args() {
        let call = AnalysisCall {
            args: vec![
                Arg {
                    name: "from".into(),
                    name_span: sp(),
                    value: Expr::quantity(10.0, FREQUENCY),
                },
                Arg {
                    name: "to".into(),
                    name_span: sp(),
                    value: Expr::quantity(1e6, FREQUENCY),
                },
            ],
            span: sp(),
        };
        assert!(call.arg("from").is_some());
        assert!(call.arg("points").is_none());
        assert_eq!(call.arg_names(), vec!["from", "to"]);
    }

    #[test]
    fn binary_op_classification() {
        assert!(BinaryOp::Lt.is_comparison());
        assert!(!BinaryOp::Lt.is_logical());
        assert!(BinaryOp::And.is_logical());
        assert_eq!(BinaryOp::Add.symbol(), "+");
    }

    #[test]
    fn stmt_keywords_come_from_device_kind() {
        let s = Stmt::Device(DeviceStmt {
            kind: DeviceStmtKind::Resistor,
            name: SpannedName::new("r1", sp()),
            args: Vec::new(),
            span: sp(),
        });
        assert_eq!(s.keyword(), "resistor");
        assert_eq!(DeviceStmtKind::VoltageSource.keyword(), "voltage_source");
    }

    /// Every expression must be able to report a span, because dimension and
    /// name errors point at sub-expressions.
    #[test]
    fn expressions_carry_spans() {
        let e = Expr::new(ExprKind::Var("r".into()), sp());
        assert_eq!(e.span(), sp());

        let nested = Expr::new(
            ExprKind::Binary {
                op: BinaryOp::Add,
                lhs: Box::new(Expr::quantity(1.0, RESISTANCE)),
                rhs: Box::new(Expr::var("r")),
            },
            sp(),
        );
        assert_eq!(nested.span(), sp());
    }

    #[test]
    fn expr_helpers_round_trip() {
        assert_eq!(Expr::symbol("vin").as_symbol(), Some("vin"));
        assert_eq!(Expr::var("r").as_symbol(), None);
        assert!(Expr::var("r").as_dict().is_none());

        let d = Expr::new(
            ExprKind::Dict(vec![DictEntry {
                key: "input".into(),
                key_span: sp(),
                value: Expr::symbol("vin"),
            }]),
            sp(),
        );
        assert_eq!(d.as_dict().unwrap().len(), 1);
        assert_eq!(d.as_dict().unwrap()[0].key, "input");
    }

    #[test]
    fn call_lookup_uses_named_args() {
        let c = Call {
            name: "pulse".into(),
            name_span: sp(),
            positional: Vec::new(),
            named: vec![Arg {
                name: "low".into(),
                name_span: sp(),
                value: Expr::quantity(0.0, circuit_core::units::VOLTAGE),
            }],
            span: sp(),
        };
        assert!(c.arg("low").is_some());
        assert!(c.arg("high").is_none());
    }
}
