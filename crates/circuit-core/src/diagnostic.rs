//! Diagnostics: the user-facing error type.
//!
//! The rendered form follows the layout fixed in the language specification:
//!
//! ```text
//! error[E_DIMENSION]: resistor.value requires resistance, found time
//!   --> examples/filter.cdsl:8:48
//!    |
//!  8 | resistor :r1, p: :vin, n: :out, value: 10.ms
//!    |                                         ^^^^^
//!    = instance: top.filter.r1
//!    = expected: ohm
//!    = received: s
//! ```

use std::fmt;

use crate::span::{SourceMap, SourceSpan};

/// How serious a diagnostic is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        }
    }
}

/// Stable machine-readable diagnostic identifiers.
///
/// These appear in `error[E_...]` and are part of the CLI contract, so they
/// must not be renamed once released.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Code {
    /// Lexing or parsing failure.
    Syntax,
    /// A name was not declared, or is used in a context where it has no meaning.
    Name,
    /// Duplicate definition of a name in the same namespace.
    Duplicate,
    /// Dimensional mismatch in an expression or device parameter.
    Dimension,
    /// A device/parameter value is outside its contract (for example R <= 0).
    Value,
    /// Wrong argument count, unknown named argument, or missing required argument.
    Argument,
    /// A value has the wrong type (symbol where number expected, and so on).
    Type,
    /// Cyclic parameter dependency.
    ParamCycle,
    /// Recursive subcircuit instantiation.
    Recursion,
    /// Port binding problems: missing, unknown, or duplicated ports.
    Port,
    /// A parameter that affects topology was used where topology must be fixed.
    TopologyParam,
    /// An elaboration limit (device count, expansion steps, depth) was exceeded.
    Limit,
    /// A construct the project does not support (yet, or by design).
    Unsupported,
    /// The backend rejected the circuit or the analysis plan.
    Backend,
    /// A numerical solve failed to converge.
    Convergence,
    /// The system matrix was singular.
    Singular,
    /// A sweep specification is inconsistent (bad step, reversed direction).
    Sweep,
    /// File system or other I/O problem.
    Io,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Code::Syntax => "E_SYNTAX",
            Code::Name => "E_NAME",
            Code::Duplicate => "E_DUPLICATE",
            Code::Dimension => "E_DIMENSION",
            Code::Value => "E_VALUE",
            Code::Argument => "E_ARGUMENT",
            Code::Type => "E_TYPE",
            Code::ParamCycle => "E_PARAM_CYCLE",
            Code::Recursion => "E_RECURSION",
            Code::Port => "E_PORT",
            Code::TopologyParam => "E_TOPO_PARAM",
            Code::Limit => "E_LIMIT",
            Code::Unsupported => "E_UNSUPPORTED",
            Code::Backend => "E_BACKEND",
            Code::Convergence => "E_CONVERGE",
            Code::Singular => "E_SINGULAR",
            Code::Sweep => "E_SWEEP",
            Code::Io => "E_IO",
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One annotated source region.
#[derive(Clone, Debug)]
pub struct Label {
    pub span: SourceSpan,
    pub message: Option<String>,
}

impl Label {
    pub fn new(span: SourceSpan) -> Self {
        Self {
            span,
            message: None,
        }
    }

    pub fn with_message(span: SourceSpan, message: impl Into<String>) -> Self {
        Self {
            span,
            message: Some(message.into()),
        }
    }
}

/// A single user-facing diagnostic.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: Code,
    pub message: String,
    /// The location the error is "about".
    pub primary: Option<Label>,
    /// Supporting locations, for example the first definition of a
    /// duplicate name.
    pub secondary: Vec<Label>,
    /// Trailing `= key: value` lines.
    pub notes: Vec<String>,
    /// Free-form key/value context, rendered as `= key: value`.
    pub context: Vec<(String, String)>,
}

impl Diagnostic {
    pub fn new(severity: Severity, code: Code, message: impl Into<String>) -> Self {
        Self {
            severity,
            code,
            message: message.into(),
            primary: None,
            secondary: Vec::new(),
            notes: Vec::new(),
            context: Vec::new(),
        }
    }

    pub fn error(code: Code, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, code, message)
    }

    pub fn warning(code: Code, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, code, message)
    }

    /// Attach the primary location, unless the span is synthetic (in which
    /// case there is no honest location to point at).
    pub fn at(mut self, span: SourceSpan) -> Self {
        if !span.is_synthetic() {
            self.primary = Some(Label::new(span));
        }
        self
    }

    pub fn at_with(mut self, span: SourceSpan, message: impl Into<String>) -> Self {
        if !span.is_synthetic() {
            self.primary = Some(Label::with_message(span, message));
        }
        self
    }

    pub fn with_label(mut self, label: Label) -> Self {
        if self.primary.is_none() && !label.span.is_synthetic() {
            self.primary = Some(label);
        } else if !label.span.is_synthetic() {
            self.secondary.push(label);
        }
        self
    }

    pub fn with_secondary(mut self, span: SourceSpan, message: impl Into<String>) -> Self {
        if !span.is_synthetic() {
            self.secondary.push(Label::with_message(span, message));
        }
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.push((key.into(), value.into()));
        self
    }

    /// Convenience for the common "expected / received" pair.
    pub fn with_dims(self, expected: impl fmt::Display, received: impl fmt::Display) -> Self {
        self.with_context("expected", expected.to_string())
            .with_context("received", received.to_string())
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// Render without source excerpts (used when no `SourceMap` is available).
    pub fn render_plain(&self) -> String {
        let mut out = format!(
            "{}[{}]: {}",
            self.severity.as_str(),
            self.code.as_str(),
            self.message
        );
        for (k, v) in &self.context {
            out.push_str(&format!("\n   = {k}: {v}"));
        }
        for n in &self.notes {
            out.push_str(&format!("\n   = {n}"));
        }
        out
    }

    /// Render with source excerpts against a source map.
    pub fn render(&self, sources: &SourceMap) -> String {
        let mut out = format!(
            "{}[{}]: {}",
            self.severity.as_str(),
            self.code.as_str(),
            self.message
        );

        if let Some(label) = &self.primary {
            out.push_str(&format!("\n  --> {}", sources.location(label.span)));
            if let Some(excerpt) = sources.render_excerpt(label.span, label.message.as_deref()) {
                out.push_str("\n   |\n");
                out.push_str(&excerpt);
            }
        }

        for label in &self.secondary {
            out.push_str(&format!("\n  --> {}", sources.location(label.span)));
            // The main label already carries the message when there is one;
            // secondary labels can carry their own.
            if let Some(excerpt) = sources.render_excerpt(label.span, label.message.as_deref()) {
                out.push_str("\n   |\n");
                out.push_str(&excerpt);
            }
        }

        for (k, v) in &self.context {
            out.push_str(&format!("\n   = {k}: {v}"));
        }
        for n in &self.notes {
            out.push_str(&format!("\n   = {n}"));
        }
        out
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render_plain())
    }
}

impl std::error::Error for Diagnostic {}

/// A non-empty list of diagnostics.
///
/// Errors are collected rather than short-circuiting so that one run can
/// report several independent problems, which is what users expect from a
/// `check` command.
#[derive(Clone, Debug)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn single(d: Diagnostic) -> Self {
        Self { items: vec![d] }
    }

    pub fn push(&mut self, d: Diagnostic) {
        self.items.push(d);
    }

    pub fn extend(&mut self, other: Diagnostics) {
        self.items.extend(other.items);
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn has_errors(&self) -> bool {
        self.items.iter().any(|d| d.is_error())
    }

    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter()
    }

    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.items
    }

    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter().filter(|d| d.is_error())
    }

    /// Render all diagnostics, one per block.
    pub fn render(&self, sources: &SourceMap) -> String {
        self.items
            .iter()
            .map(|d| d.render(sources))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Render without source excerpts.
    pub fn render_plain(&self) -> String {
        self.items
            .iter()
            .map(|d| d.render_plain())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Diagnostic> for Diagnostics {
    fn from(d: Diagnostic) -> Self {
        Self::single(d)
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render_plain())
    }
}

impl std::error::Error for Diagnostics {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::SourceMap;

    #[test]
    fn renders_location_and_caret() {
        let mut sm = SourceMap::new();
        let id = sm.add("t.cdsl", "resistor :r1, value: 10.ms\n");
        // "10.ms" occupies bytes 21..26, i.e. column 22.
        let d = Diagnostic::error(Code::Dimension, "resistor.value requires resistance")
            .at(SourceSpan::new(id, 21, 26))
            .with_context("expected", "ohm")
            .with_context("received", "s");
        let out = d.render(&sm);
        assert!(out.starts_with("error[E_DIMENSION]: "), "{out}");
        assert!(out.contains("t.cdsl:1:22"), "{out}");
        assert!(out.contains("^^^^^"), "{out}");
        assert!(out.contains("= expected: ohm"), "{out}");
        assert!(out.contains("= received: s"), "{out}");
    }

    #[test]
    fn synthetic_span_does_not_become_a_location() {
        let mut sm = SourceMap::new();
        sm.add("t.cdsl", "x\n");
        let d = Diagnostic::error(Code::Name, "boom").at(SourceSpan::synthetic());
        assert!(d.primary.is_none());
        let out = d.render(&sm);
        assert!(!out.contains("-->"), "{out}");
    }

    #[test]
    fn diagnostics_aggregate() {
        let mut ds = Diagnostics::new();
        ds.push(Diagnostic::error(Code::Syntax, "a"));
        ds.push(Diagnostic::warning(Code::Value, "b"));
        assert_eq!(ds.len(), 2);
        assert_eq!(ds.errors().count(), 1);
        assert!(ds.has_errors());
    }
}
