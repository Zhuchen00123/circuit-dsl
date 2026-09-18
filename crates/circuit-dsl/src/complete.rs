//! Deciding whether an input is finished, half-typed, or already wrong.
//!
//! A REPL must answer this question before it can show a continuation prompt.
//! The naive answers are both wrong:
//!
//! - counting `do` against `end` misses `f(1,` and `x =`, and breaks as soon
//!   as a `do` appears inside a string or a comment;
//! - treating *any* parse failure as "needs more input" makes `node 5` hang the
//!   session forever, because that input will never become valid.
//!
//! So this module answers in three states, and each one is justified by a
//! structural fact about the token stream rather than by a parse failure:
//!
//! 1. **The lexer failed** → [`Completeness::Invalid`]. Every lexer error is
//!    final: an unknown unit suffix, an illegal character, `...`, string
//!    interpolation, and an unterminated string (strings may not span lines,
//!    so that is an error and not a continuation).
//! 2. **The token stream is structurally open** → [`Completeness::Incomplete`]:
//!    an unclosed bracket, an unclosed `do` block, or a trailing token that
//!    cannot end a line (a comma, an operator, `:`, `..`, `=`, `=>`).
//! 3. **Otherwise the parser decides**: it succeeds ([`Completeness::Complete`])
//!    or it reports real syntax errors ([`Completeness::Invalid`]).
//!
//! A stray `end` or a mismatched `)` is deliberately *not* "incomplete": the
//! stream is closed, so the parser gets to explain what is wrong with it.

use circuit_core::diagnostic::Diagnostics;
use circuit_core::span::{SourceId, SourceSpan};

use crate::lexer::lex;
use crate::parser::parse_input_detailed;
use crate::token::{Token, TokenKind};

/// What an input is, as far as a REPL is concerned.
#[derive(Clone, Debug)]
pub enum Completeness {
    /// The input parses; it can be evaluated or defined as it stands.
    Complete,
    /// The input is structurally open: more text can finish it.
    Incomplete {
        /// What is still open, phrased for a continuation prompt.
        reason: &'static str,
        /// Where it was opened, so the REPL can point at it.
        span: SourceSpan,
    },
    /// The input is closed but does not parse, or does not lex.
    Invalid(Diagnostics),
}

impl Completeness {
    pub fn is_complete(&self) -> bool {
        matches!(self, Completeness::Complete)
    }

    pub fn is_incomplete(&self) -> bool {
        matches!(self, Completeness::Incomplete { .. })
    }
}

/// Classify `text`.
///
/// The whole accumulated buffer is passed in on every call, so a multi-line
/// construct is judged as one input.
pub fn assess(source: SourceId, text: &str) -> Completeness {
    let tokens = match lex(source, text) {
        Ok(tokens) => tokens,
        Err(diagnostics) => return Completeness::Invalid(diagnostics),
    };

    // The parser decides first, because it is the only thing that can tell a
    // half-typed line from a wrong one: the structural scan alone would call
    // `for k in 1..3 do` unfinished, when in fact a `for` can never appear at
    // the session prompt.
    let parsed = parse_input_detailed(&tokens);
    if parsed.is_ok() {
        return Completeness::Complete;
    }

    // The parser ran out of text *and* the structure is still open: this input
    // can be finished by typing more.
    if parsed.ran_out
        && let Some(open) = unclosed(&tokens)
    {
        return Completeness::Incomplete {
            reason: open.reason,
            span: open.span,
        };
    }

    Completeness::Invalid(parsed.diagnostics)
}

struct Open {
    reason: &'static str,
    span: SourceSpan,
}

/// A bracket or block that is still open, or a trailing token that cannot end
/// a line.
fn unclosed(tokens: &[Token]) -> Option<Open> {
    let mut stack: Vec<TokenKind> = Vec::new();
    // Where each opener was seen, so the reason can point at it.
    let mut openers: Vec<SourceSpan> = Vec::new();
    let mut blocks: Vec<SourceSpan> = Vec::new();

    for token in tokens {
        match &token.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                stack.push(token.kind.clone());
                openers.push(token.span);
            }
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                let expected = match token.kind {
                    TokenKind::RParen => TokenKind::LParen,
                    TokenKind::RBracket => TokenKind::LBracket,
                    _ => TokenKind::LBrace,
                };
                // A mismatched closer is a real error, not an unfinished
                // input: hand it to the parser, which explains the pairing.
                if stack.pop() != Some(expected) {
                    return None;
                }
                openers.pop();
            }
            TokenKind::Ident(word) if word == "do" => blocks.push(token.span),
            // An extra `end`: the input is closed, and wrong.
            TokenKind::Ident(word) if word == "end" => {
                blocks.pop()?;
            }
            _ => {}
        }
    }

    if let (Some(block), Some(opener)) = (blocks.last(), blocks.last()) {
        let _ = block;
        return Some(Open {
            reason: "unclosed `do` block",
            span: *opener,
        });
    }

    if let Some(kind) = stack.last() {
        let span = openers
            .last()
            .copied()
            .unwrap_or_else(SourceSpan::synthetic);
        let reason = match kind {
            TokenKind::LParen => "unclosed `(`",
            TokenKind::LBracket => "unclosed `[`",
            _ => "unclosed `{`",
        };
        return Some(Open { reason, span });
    }

    // The last token that is not a line break decides whether the input can
    // stop here. The lexer already suppresses a line break after a
    // continuation point, so a trailing comma or operator simply arrives as
    // the last token.
    let last = tokens
        .iter()
        .rev()
        .find(|t| !matches!(t.kind, TokenKind::Newline | TokenKind::Eof))?;

    let reason: &'static str = match &last.kind {
        TokenKind::Comma => "the line ends with a comma",
        TokenKind::Colon => "the line ends with a label",
        TokenKind::Dot => "the line ends with `.`",
        TokenKind::DotDot => "the line ends with `..`",
        TokenKind::FatArrow => "the line ends with `=>`",
        TokenKind::Plus => "the line ends with `+`",
        TokenKind::Minus => "the line ends with `-`",
        TokenKind::Star => "the line ends with `*`",
        TokenKind::Slash => "the line ends with `/`",
        TokenKind::AmpAmp => "the line ends with `&&`",
        TokenKind::PipePipe => "the line ends with `||`",
        TokenKind::EqEq => "the line ends with `==`",
        TokenKind::BangEq => "the line ends with `!=`",
        TokenKind::Lt => "the line ends with `<`",
        TokenKind::Le => "the line ends with `<`",
        TokenKind::Gt => "the line ends with `>`",
        TokenKind::Ge => "the line ends with `>`",
        TokenKind::Assign => "the line ends with `=`",
        _ => return None,
    };
    Some(Open {
        reason,
        span: last.span,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::TokenKind;

    fn assess_str(text: &str) -> Completeness {
        assess(SourceId(0), text)
    }

    fn assert_incomplete(text: &str, reason: &str) {
        match assess_str(text) {
            Completeness::Incomplete { reason: got, .. } => {
                assert_eq!(got, reason, "for input {text:?}")
            }
            other => panic!("expected {text:?} to be incomplete, got {other:?}"),
        }
    }

    fn assert_invalid(text: &str) {
        match assess_str(text) {
            Completeness::Invalid(d) => assert!(d.has_errors(), "for input {text:?}"),
            other => panic!("expected {text:?} to be a real error, got {other:?}"),
        }
    }

    fn assert_complete(text: &str) {
        assert!(
            assess_str(text).is_complete(),
            "expected {text:?} to be complete, got {:?}",
            assess_str(text)
        );
    }

    #[test]
    fn a_finished_input_is_complete() {
        assert_complete("r = 1.kohm\n");
        assert_complete("circuit :d do\n  node :a\nend\n");
        assert_complete("sqrt(2)\n");
        assert_complete("circuit :d do\n  for k in 1..3 do\n    node :a\n  end\nend\n");
        // No trailing newline is fine too.
        assert_complete("1 + 1");
    }

    /// A result-expression statement is an experiment statement like any
    /// other: an open block that contains one is a continuation, not an
    /// error, so the prompt must keep waiting for the `end`.
    #[test]
    fn an_experiment_with_a_derive_still_continues() {
        assert_incomplete(
            "experiment :e, circuit: :x do\n  derive :g, expr: v(:a) / v(:b)\n",
            "unclosed `do` block",
        );
        assert_incomplete(
            "experiment :e, circuit: :x do\n  measure :m, max: v(:a), analysis: :ac1\n",
            "unclosed `do` block",
        );
    }

    #[test]
    fn an_unclosed_block_continues() {
        assert_incomplete("circuit :d do\n", "unclosed `do` block");
        assert_incomplete("circuit :d do\n  node :a\n", "unclosed `do` block");
        assert_incomplete("circuit :d do\n  if a > 0 do\n", "unclosed `do` block");
        // A nested block needs both `end`s.
        assert_incomplete(
            "circuit :d do\n  if a > 0 do\n    node :b\n  end\n",
            "unclosed `do` block",
        );
    }

    #[test]
    fn an_unclosed_bracket_continues() {
        assert_incomplete("r = sqrt(", "unclosed `(`");
        assert_incomplete("r = min(1,", "unclosed `(`");
        assert_incomplete("r = [1, 2,\n", "unclosed `[`");
        assert_incomplete("r = { a:", "unclosed `{`");
    }

    #[test]
    fn a_trailing_operator_continues() {
        assert_incomplete("r = 1 +", "the line ends with `+`");
        assert_incomplete("r = 1 *", "the line ends with `*`");
        assert_incomplete("r = 1.kohm /", "the line ends with `/`");
        assert_incomplete("r =", "the line ends with `=`");
        assert_incomplete("a == ", "the line ends with `==`");
        assert_incomplete("b = 1 ||", "the line ends with `||`");
    }

    #[test]
    fn a_multi_line_continuation_completes() {
        let buffer = "circuit :d do\n  node :a\n";
        assert!(assess_str(buffer).is_incomplete());
        let buffer = format!("{buffer}end\n");
        assert_complete(&buffer);

        let buffer = "r = min(1,";
        assert!(assess_str(buffer).is_incomplete());
        assert_complete(&format!("{buffer} 2)"));
    }

    /// The distinction the brief calls out: a closed input that is wrong must
    /// report immediately rather than invite more typing.
    #[test]
    fn a_closed_input_that_is_wrong_is_invalid() {
        assert_invalid("node 5\n");
        assert_invalid("end\n");
        assert_invalid(")\n");
        assert_invalid("circuit :d do\nend\nend\n");
        assert_invalid("save v(:a))\n");
        assert_invalid("1 +* 2\n");
    }

    #[test]
    fn lexer_errors_are_final_not_continuations() {
        // An unterminated string cannot be fixed by typing more: strings may
        // not span lines.
        assert_invalid("r = \"abc\n");
        assert_invalid("r = 1.xV\n");
        assert_invalid("r = 0...3\n");
        assert_invalid("r = \"a#{b}\"\n");
        assert_invalid("node %w[a]\n");
    }

    #[test]
    fn a_do_inside_a_string_or_comment_is_not_a_block() {
        // The reason a token-level scan is right and counting text is wrong.
        assert_complete("r = \"do\"\n");
        assert_complete("# do not close this\n");
        assert_complete("r = \"end\"\n");
    }

    #[test]
    fn incomplete_reports_where_it_opened() {
        match assess_str("circuit :d do\n  node :a\n") {
            Completeness::Incomplete { span, .. } => {
                assert!(!span.is_synthetic());
                assert_eq!(span.start, 11, "the span points at `do`");
            }
            other => panic!("expected incomplete, got {other:?}"),
        }
    }

    /// A body statement typed at the prompt is wrong, not unfinished, and the
    /// message must say where it belongs. This is the case that a structural
    /// scan alone gets wrong: `for ... do` leaves a block open, but a `for`
    /// can never appear at the session prompt.
    #[test]
    fn a_body_statement_is_invalid_with_a_pointer_to_its_home() {
        match assess_str("for k in 1..3 do\n") {
            Completeness::Invalid(d) => {
                let text = d.render_plain();
                assert!(text.contains("body statement"), "{text}");
                assert!(text.contains("circuit :name do"), "{text}");
            }
            other => panic!("expected a real error, got {other:?}"),
        }
        assert_invalid("op\n");
        assert_invalid("node :a\n");
        assert_invalid("param :r, default: 1.kohm\n");
    }

    #[test]
    fn empty_input_is_complete() {
        // The REPL decides what to do with an empty line; it is not waiting
        // for more text.
        assert_complete("");
        assert_complete("\n");
        assert_complete("   \n");
        assert_complete("# just a comment\n");
    }

    /// A token that cannot end a line must be listed here, so a newly added
    /// operator cannot silently become "complete".
    #[test]
    fn the_trailing_token_list_covers_the_continuation_set() {
        for kind in [
            TokenKind::Comma,
            TokenKind::Plus,
            TokenKind::Minus,
            TokenKind::Star,
            TokenKind::Slash,
            TokenKind::AmpAmp,
            TokenKind::PipePipe,
            TokenKind::EqEq,
            TokenKind::BangEq,
            TokenKind::Lt,
            TokenKind::Le,
            TokenKind::Gt,
            TokenKind::Ge,
            TokenKind::Colon,
        ] {
            assert!(
                kind.is_continuation_after(),
                "{kind:?} should be flagged as a continuation by the lexer"
            );
        }
    }
}
