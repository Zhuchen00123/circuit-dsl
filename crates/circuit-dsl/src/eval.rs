//! The expression evaluator, shared by file elaboration and the REPL.
//!
//! This module owns the language's *value* semantics: what a literal means,
//! how dimensions propagate, which conversions are allowed, and what each
//! operator and built-in does. There is deliberately only one implementation —
//! a REPL that evaluated expressions its own way would eventually disagree
//! with the simulator about what a program means, which is the one thing an
//! interactive front end must not do.
//!
//! Name lookup goes through [`Variables`], so the elaborator can pass its
//! parameter scope (with declaration spans for diagnostics) and the session
//! can pass its variable table, without either owning a copy of the rules.

// Same trade-off as `elaborate.rs`: `circuit_core::Diagnostic` is ~144 bytes,
// and boxing it here would push `Box<Diagnostic>` into this crate's public API
// to save a size that is irrelevant next to the allocations the error path
// already performs. The longer note lives in `elaborate.rs`.
#![allow(clippy::result_large_err)]

use circuit_core::diagnostic::{Code, Diagnostic};
use circuit_core::span::SourceSpan;
use circuit_core::units::{self, Quantity};

use crate::ast::{BinaryOp, Call, Expr, ExprKind, UnaryOp};

/// A runtime value during evaluation.
///
/// Deliberately small: only what the language can actually compute with.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Num(Quantity),
    Bool(bool),
    Sym(String),
    Str(String),
    Array(Vec<Value>),
    Dict(Vec<(String, Value)>),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Num(_) => "number",
            Value::Bool(_) => "boolean",
            Value::Sym(_) => "symbol",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Dict(_) => "dictionary",
        }
    }

    pub fn as_num(&self) -> Option<Quantity> {
        match self {
            Value::Num(q) => Some(*q),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_sym(&self) -> Option<&str> {
        match self {
            Value::Sym(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    /// Convenience for tests and callers that know a number is there.
    pub fn num(&self) -> Option<Quantity> {
        self.as_num()
    }
}

/// Where the evaluator looks names up.
///
/// Implemented by the elaborator's parameter scope and by the session's
/// variable table. `declared_span` is optional and only improves diagnostics.
pub trait Variables {
    fn lookup(&self, name: &str) -> Option<Quantity>;

    /// Where `name` was declared, so "not declared" can point at a similar
    /// name that *is* in scope.
    fn declared_span(&self, _name: &str) -> Option<SourceSpan> {
        None
    }
}

/// A variable table with nothing in it, for evaluating constant expressions.
pub struct NoVariables;

impl Variables for NoVariables {
    fn lookup(&self, _name: &str) -> Option<Quantity> {
        None
    }
}

/// Evaluate an expression.
pub fn eval(expr: &Expr, vars: &dyn Variables) -> Result<Value, Diagnostic> {
    match &expr.kind {
        ExprKind::Int(i) => Ok(Value::Num(Quantity::scalar(*i as f64))),
        ExprKind::Float(f) => Ok(Value::Num(Quantity::scalar(*f))),
        ExprKind::Quantity(q) => Ok(Value::Num(Quantity::new(q.value, q.dimension))),
        ExprKind::Bool(b) => Ok(Value::Bool(*b)),
        ExprKind::Str(s) => Ok(Value::Str(s.clone())),
        ExprKind::Symbol(s) => Ok(Value::Sym(s.clone())),

        ExprKind::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(eval(item, vars)?);
            }
            Ok(Value::Array(out))
        }

        ExprKind::Dict(entries) => {
            let mut out = Vec::with_capacity(entries.len());
            for entry in entries {
                out.push((entry.key.clone(), eval(&entry.value, vars)?));
            }
            Ok(Value::Dict(out))
        }

        ExprKind::Var(name) => match vars.lookup(name) {
            Some(q) => Ok(Value::Num(q)),
            None => {
                let mut d = Diagnostic::error(Code::Name, format!("`{name}` is not declared"))
                    .at(expr.span)
                    .with_note(
                        "an undeclared name is never treated as a node, device or function call",
                    );
                if let Some(src) = vars.declared_span(name) {
                    d = d.with_secondary(src, "a parameter with this name is declared here");
                }
                Err(d)
            }
        },

        ExprKind::Unary { op, rhs } => {
            let v = eval(rhs, vars)?;
            match (op, v) {
                (UnaryOp::Neg, Value::Num(q)) => Ok(Value::Num(-q)),
                (UnaryOp::Pos, Value::Num(q)) => Ok(Value::Num(q)),
                (UnaryOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                (UnaryOp::Not, other) => Err(Diagnostic::error(
                    Code::Type,
                    format!("`!` needs a boolean, found {}", other.type_name()),
                )
                .at(expr.span)),
                (_, other) => Err(Diagnostic::error(
                    Code::Type,
                    format!("cannot negate {}", other.type_name()),
                )
                .at(expr.span)),
            }
        }

        ExprKind::Binary { op, lhs, rhs } => {
            let a = eval(lhs, vars)?;
            let b = eval(rhs, vars)?;
            eval_binary(*op, a, b, expr.span, lhs.span, rhs.span)
        }

        ExprKind::Call(call) => eval_call(call, vars),
    }
}

fn eval_binary(
    op: BinaryOp,
    a: Value,
    b: Value,
    span: SourceSpan,
    a_span: SourceSpan,
    b_span: SourceSpan,
) -> Result<Value, Diagnostic> {
    use BinaryOp::*;

    // `+` also builds names: if either side is text, the result is text.
    // This is what lets a loop generate `("r" + i)`.
    //
    // The trigger is a *string*, not a symbol: `:a + "b"` joins, `:a + :b`
    // does not. Concatenation never converts a dimensioned number, because
    // `"r" + 1.kohm` would be indistinguishable from `"r" + 1000`.
    if op == Add && (matches!(a, Value::Str(_)) || matches!(b, Value::Str(_))) {
        let (Some(x), Some(y)) = (stringify(&a), stringify(&b)) else {
            return Err(Diagnostic::error(
                Code::Type,
                format!(
                    "cannot join {} and {} into text",
                    a.type_name(),
                    b.type_name()
                ),
            )
            .at(span)
            .with_note(
                "only a dimensionless number, symbol, boolean or string becomes text; \
                 a value with a unit does not",
            ));
        };
        return Ok(Value::Str(x + &y));
    }

    if op.is_logical() {
        let (Some(x), Some(y)) = (a.as_bool(), b.as_bool()) else {
            return Err(Diagnostic::error(
                Code::Type,
                format!(
                    "`{}` needs booleans, found {} and {}",
                    op.symbol(),
                    a.type_name(),
                    b.type_name()
                ),
            )
            .at(span));
        };
        return Ok(Value::Bool(if op == And { x && y } else { x || y }));
    }

    if op.is_comparison() {
        // Only a single numeric comparison could be ambiguous; handle the
        // common Equals/NotEquals for booleans and symbols first.
        if let (Some(x), Some(y)) = (a.as_bool(), b.as_bool()) {
            return match op {
                Eq => Ok(Value::Bool(x == y)),
                Ne => Ok(Value::Bool(x != y)),
                _ => Err(Diagnostic::error(
                    Code::Type,
                    format!("`{}` cannot compare booleans", op.symbol()),
                )
                .at(span)),
            };
        }
        if let (Some(x), Some(y)) = (a.as_sym(), b.as_sym()) {
            return match op {
                Eq => Ok(Value::Bool(x == y)),
                Ne => Ok(Value::Bool(x != y)),
                _ => Err(Diagnostic::error(
                    Code::Type,
                    format!("`{}` cannot order symbols", op.symbol()),
                )
                .at(span)),
            };
        }
        let (Some(x), Some(y)) = (a.as_num(), b.as_num()) else {
            return Err(Diagnostic::error(
                Code::Type,
                format!(
                    "`{}` needs two numbers, found {} and {}",
                    op.symbol(),
                    a.type_name(),
                    b.type_name()
                ),
            )
            .at(span));
        };
        if x.dimension != y.dimension {
            return Err(Diagnostic::error(
                Code::Dimension,
                format!("cannot compare {} with {}", x.dimension, y.dimension),
            )
            .at(span)
            .with_secondary(a_span, format!("this is {}", x.dimension))
            .with_secondary(b_span, format!("this is {}", y.dimension))
            .with_dims(x.dimension, y.dimension));
        }
        let (x, y) = (x.value, y.value);
        return Ok(Value::Bool(match op {
            Eq => x == y,
            Ne => x != y,
            Lt => x < y,
            Le => x <= y,
            Gt => x > y,
            Ge => x >= y,
            _ => unreachable!(),
        }));
    }

    let (Some(x), Some(y)) = (a.as_num(), b.as_num()) else {
        return Err(Diagnostic::error(
            Code::Type,
            format!(
                "`{}` needs two numbers, found {} and {}",
                op.symbol(),
                a.type_name(),
                b.type_name()
            ),
        )
        .at(span));
    };

    let result = match op {
        Add | Sub => {
            if x.dimension != y.dimension {
                return Err(Diagnostic::error(
                    Code::Dimension,
                    format!("cannot {} {} and {}", op.symbol(), x.dimension, y.dimension),
                )
                .at(span)
                .with_secondary(a_span, format!("this is {}", x.dimension))
                .with_secondary(b_span, format!("this is {}", y.dimension))
                .with_dims(x.dimension, y.dimension));
            }
            Quantity::new(
                if op == Add {
                    x.value + y.value
                } else {
                    x.value - y.value
                },
                x.dimension,
            )
        }
        Mul => x * y,
        Div => {
            if y.value == 0.0 {
                return Err(Diagnostic::error(Code::Value, "division by zero").at(b_span));
            }
            x / y
        }
        _ => unreachable!("logical and comparison handled above"),
    };

    Ok(Value::Num(result))
}

fn eval_call(call: &Call, vars: &dyn Variables) -> Result<Value, Diagnostic> {
    // Built-in numeric functions.
    match call.name.as_str() {
        "str" => {
            if call.positional.len() != 1 || !call.named.is_empty() {
                return Err(Diagnostic::error(
                    Code::Argument,
                    format!("`str` takes one argument, found {}", call.positional.len()),
                )
                .at(call.span));
            }
            let v = eval(&call.positional[0], vars)?;
            let Some(text) = stringify(&v) else {
                return Err(Diagnostic::error(
                    Code::Type,
                    format!("`str` cannot convert {}", v.type_name()),
                )
                .at(call.span));
            };
            Ok(Value::Str(text))
        }
        "abs" | "sqrt" | "min" | "max" => {
            let mut nums = Vec::new();
            for arg in &call.positional {
                match eval(arg, vars)? {
                    Value::Num(q) => nums.push(q),
                    other => {
                        return Err(Diagnostic::error(
                            Code::Type,
                            format!("`{}` takes numbers, found {}", call.name, other.type_name()),
                        )
                        .at(arg.span));
                    }
                }
            }
            builtin_numeric(&call.name, nums, call)
        }
        "v" | "i" => Err(Diagnostic::error(
            Code::Name,
            format!("`{}` can only be used in `save` and `measure`", call.name),
        )
        .at(call.span)
        .with_note("it names a result signal, not a value available during elaboration")),
        "pulse" | "sin" | "pwl" => Err(Diagnostic::error(
            Code::Name,
            format!("`{}` can only be used as a `waveform:` argument", call.name),
        )
        .at(call.span)),
        other => Err(
            Diagnostic::error(Code::Name, format!("unknown function `{other}`"))
                .at(call.name_span)
                .with_note("available: abs, sqrt, min, max, str, pulse, sin, pwl, v, i"),
        ),
    }
}

fn builtin_numeric(name: &str, args: Vec<Quantity>, call: &Call) -> Result<Value, Diagnostic> {
    let want = |n: usize| -> Result<(), Diagnostic> {
        if args.len() == n {
            Ok(())
        } else {
            Err(Diagnostic::error(
                Code::Argument,
                format!("`{name}` takes {n} argument(s), found {}", args.len()),
            )
            .at(call.span))
        }
    };

    match name {
        "abs" => {
            want(1)?;
            Ok(Value::Num(Quantity::new(
                args[0].value.abs(),
                args[0].dimension,
            )))
        }
        "sqrt" => {
            want(1)?;
            let d = args[0].dimension;
            // Only an even root of an even-powered dimension is meaningful;
            // require dimensionless for simplicity and say so.
            if !d.is_dimensionless() {
                return Err(Diagnostic::error(
                    Code::Dimension,
                    format!("`sqrt` needs a dimensionless value, found {}", d),
                )
                .at(call.span)
                .with_dims(units::DIMENSIONLESS, d));
            }
            if args[0].value < 0.0 {
                return Err(
                    Diagnostic::error(Code::Value, "`sqrt` of a negative number").at(call.span),
                );
            }
            Ok(Value::Num(Quantity::scalar(args[0].value.sqrt())))
        }
        "min" | "max" => {
            want(2)?;
            if args[0].dimension != args[1].dimension {
                return Err(Diagnostic::error(
                    Code::Dimension,
                    format!(
                        "`{name}` needs two values of the same kind, found {} and {}",
                        args[0].dimension, args[1].dimension
                    ),
                )
                .at(call.span)
                .with_dims(args[0].dimension, args[1].dimension));
            }
            let pick = if name == "min" {
                args[0].value.min(args[1].value)
            } else {
                args[0].value.max(args[1].value)
            };
            Ok(Value::Num(Quantity::new(pick, args[0].dimension)))
        }
        _ => unreachable!("caller filters the name"),
    }
}

/// Turn a value into text, for string concatenation and `str(x)`.
///
/// The rule, stated in `docs/language.md`: a dimensionless number becomes a
/// decimal (integers without a decimal point), a symbol becomes its name, a
/// boolean becomes `true`/`false`. A value with a dimension does *not* convert,
/// and neither do arrays or dictionaries.
pub fn stringify(v: &Value) -> Option<String> {
    Some(match v {
        Value::Str(s) => s.clone(),
        Value::Sym(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Num(q) => {
            if !q.dimension.is_dimensionless() {
                return None;
            }
            if q.value.fract() == 0.0 && q.value.abs() < 1e15 {
                format!("{}", q.value as i64)
            } else {
                format!("{}", q.value)
            }
        }
        Value::Array(_) | Value::Dict(_) => return None,
    })
}
