//! Tokens produced by the lexer.
//!
//! There is deliberately **no keyword token kind**: statement keywords such as
//! `circuit`, `do`, and `end` arrive as [`TokenKind::Ident`] and the parser
//! dispatches on their text at statement position. This keeps words like
//! `value`, `from`, and `points` usable as named-argument labels without a
//! special case, and it means a device or node named `end` is still writable
//! as a symbol (`:end`).

use circuit_core::span::SourceSpan;
use circuit_core::units::Dimension;
use std::fmt;

/// A quantity literal: the SI value plus the text the user wrote.
///
/// `text` is kept so diagnostics can echo `1.kohm` rather than `1000`.
#[derive(Clone, PartialEq, Debug)]
pub struct QuantityLiteral {
    /// Value in SI base units.
    pub value: f64,
    pub dimension: Dimension,
    /// Verbatim source text, e.g. `100.nF`.
    pub text: String,
}

#[derive(Clone, PartialEq, Debug)]
pub enum TokenKind {
    // ---- literals -------------------------------------------------------
    /// Integer literal without a unit suffix.
    Int(i64),
    /// Floating point literal without a unit suffix.
    Float(f64),
    /// A literal with a unit suffix, e.g. `1.kohm`.
    Quantity(QuantityLiteral),
    /// A double-quoted string.
    Str(String),
    /// A `:symbol`.
    Symbol(String),
    /// A bare identifier: a variable, function name, or statement keyword.
    Ident(String),

    // ---- layout ---------------------------------------------------------
    /// End of a logical line. The parser treats this as a statement
    /// terminator; the lexer suppresses it inside brackets and after
    /// continuation points (see `docs/language.md` §1.8).
    Newline,

    // ---- punctuation ----------------------------------------------------
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Dot,
    /// `..`
    DotDot,
    /// `=>`
    FatArrow,

    // ---- operators ------------------------------------------------------
    Plus,
    Minus,
    Star,
    Slash,
    /// `!`
    Bang,
    Lt,
    Le,
    Gt,
    Ge,
    EqEq,
    BangEq,
    AmpAmp,
    PipePipe,

    /// End of input.
    Eof,
}

impl TokenKind {
    /// Whether this token can end a statement, which is what the lexer checks
    /// when deciding whether a newline is significant.
    pub fn is_continuation_after(&self) -> bool {
        matches!(
            self,
            TokenKind::Comma
                | TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Star
                | TokenKind::Slash
                | TokenKind::AmpAmp
                | TokenKind::PipePipe
                | TokenKind::EqEq
                | TokenKind::BangEq
                | TokenKind::Lt
                | TokenKind::Le
                | TokenKind::Gt
                | TokenKind::Ge
                | TokenKind::Colon
        )
    }

    /// Short description used in `expected ...` diagnostics.
    pub fn describe(&self) -> String {
        match self {
            TokenKind::Int(v) => format!("integer `{v}`"),
            TokenKind::Float(v) => format!("number `{v}`"),
            TokenKind::Quantity(q) => format!("quantity `{}`", q.text),
            TokenKind::Str(s) => format!("string \"{s}\""),
            TokenKind::Symbol(s) => format!("symbol `:{s}`"),
            TokenKind::Ident(s) => format!("identifier `{s}`"),
            TokenKind::Newline => "end of line".to_string(),
            TokenKind::Eof => "end of file".to_string(),
            other => format!("`{}`", other.symbol_text()),
        }
    }

    /// The literal punctuation for symbol-like tokens.
    pub fn symbol_text(&self) -> &'static str {
        match self {
            TokenKind::LParen => "(",
            TokenKind::RParen => ")",
            TokenKind::LBracket => "[",
            TokenKind::RBracket => "]",
            TokenKind::LBrace => "{",
            TokenKind::RBrace => "}",
            TokenKind::Comma => ",",
            TokenKind::Colon => ":",
            TokenKind::Dot => ".",
            TokenKind::DotDot => "..",
            TokenKind::FatArrow => "=>",
            TokenKind::Plus => "+",
            TokenKind::Minus => "-",
            TokenKind::Star => "*",
            TokenKind::Slash => "/",
            TokenKind::Bang => "!",
            TokenKind::Lt => "<",
            TokenKind::Le => "<=",
            TokenKind::Gt => ">",
            TokenKind::Ge => ">=",
            TokenKind::EqEq => "==",
            TokenKind::BangEq => "!=",
            TokenKind::AmpAmp => "&&",
            TokenKind::PipePipe => "||",
            _ => "?",
        }
    }

    /// Whether a statement may start with this token.
    ///
    /// Used by the parser to decide whether a construct is finished, and by
    /// elaboration to reject trailing garbage.
    pub fn starts_statement(&self) -> bool {
        matches!(self, TokenKind::Ident(_))
    }
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub span: SourceSpan,
}

impl Token {
    pub fn new(kind: TokenKind, span: SourceSpan) -> Self {
        Self { kind, span }
    }
}

/// Statement keywords, recognised at statement position by the parser.
///
/// Kept here (rather than as tokens) so both the parser and elaboration agree
/// on one list.
pub const KEYWORDS: &[&str] = &[
    "circuit",
    "subcircuit",
    "experiment",
    "param",
    "node",
    "instance",
    "model",
    "do",
    "end",
    "else",
    "elsif",
    "if",
    "for",
    "in",
    "true",
    "false",
    "op",
    "dc",
    "ac",
    "tran",
    "save",
    "measure",
    "resistor",
    "capacitor",
    "inductor",
    "voltage_source",
    "current_source",
    "diode",
    "pulse",
    "sin",
    "pwl",
];

/// Words that may not be used as a declared name (parameter, node, device,
/// model, instance, circuit, experiment) because they would make statement
/// parsing ambiguous.
pub const RESERVED_IN_NAME_POSITION: &[&str] = &["do", "end", "else", "elsif", "if", "for", "in"];

pub fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

pub fn is_reserved_name(s: &str) -> bool {
    RESERVED_IN_NAME_POSITION.contains(&s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use circuit_core::units::{RESISTANCE, TIME};

    #[test]
    fn reserved_words_are_reported() {
        assert!(is_keyword("circuit"));
        assert!(!is_keyword("r1"));
        assert!(is_reserved_name("end"));
        assert!(!is_reserved_name("value"));
    }

    #[test]
    fn continuation_points_cover_the_documented_cases() {
        assert!(TokenKind::Comma.is_continuation_after());
        assert!(TokenKind::Plus.is_continuation_after());
        assert!(TokenKind::Colon.is_continuation_after());
        assert!(!TokenKind::LParen.is_continuation_after());
        assert!(!TokenKind::Ident("x".into()).is_continuation_after());
    }

    #[test]
    fn describes_tokens_for_diagnostics() {
        assert_eq!(TokenKind::Symbol("vin".into()).describe(), "symbol `:vin`");
        assert_eq!(TokenKind::Eof.describe(), "end of file");
        let q = TokenKind::Quantity(QuantityLiteral {
            value: 1000.0,
            dimension: RESISTANCE,
            text: "1.kohm".into(),
        });
        assert_eq!(q.describe(), "quantity `1.kohm`");
        let t = TokenKind::Quantity(QuantityLiteral {
            value: 1e-6,
            dimension: TIME,
            text: "1.us".into(),
        });
        assert!(t.describe().contains("1.us"));
    }

    #[test]
    fn only_idents_start_statements() {
        assert!(TokenKind::Ident("node".into()).starts_statement());
        assert!(!TokenKind::Newline.starts_statement());
        assert!(!TokenKind::Symbol("x".into()).starts_statement());
    }
}
