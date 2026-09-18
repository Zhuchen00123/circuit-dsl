//! Hand-written lexer for the circuit DSL.
//!
//! This module is the only place that decodes text into numbers, symbols and
//! quantity literals; the parser never re-scans source characters. Layout is
//! decided here too: `docs/language.md` §1.8 says a line break ends a
//! statement *unless* it falls inside brackets, follows a continuation point,
//! or belongs to a run of blank lines, and those three rules live in
//! [`Lexer::newline`].
//!
//! Errors are accumulated rather than fatal: an unknown unit suffix or an
//! illegal character is reported and lexing continues, so `cdsl check` can
//! list every independent problem in a file in one run.

use circuit_core::diagnostic::{Code, Diagnostic, Diagnostics};
use circuit_core::span::{SourceId, SourceSpan};
use circuit_core::units::{build_quantity, longest_unit_suffix};

use crate::token::{QuantityLiteral, Token, TokenKind};

/// Lex a whole source file.
///
/// On success the token vector always ends with [`TokenKind::Eof`]. On failure
/// every independent problem is in the returned [`Diagnostics`], and the
/// partial token vector is dropped.
pub fn lex(source: SourceId, text: &str) -> Result<Vec<Token>, Diagnostics> {
    Lexer::new(source, text).run()
}

/// Whether `c` can start an identifier (`[A-Za-z_]`).
fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

/// Whether `c` can continue an identifier (`[A-Za-z0-9_]`).
fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The MICRO SIGN and the GREEK SMALL LETTER MU, which both spell the
/// micro- prefix.
fn is_micro(c: char) -> bool {
    c == 'µ' || c == 'μ'
}

/// Whether `c` may start the unit suffix of a quantity literal.
///
/// This is the trigger rule: after the numeric part, a `.` followed by an
/// ASCII letter (or a micro sign) starts a quantity. The suffix itself is then
/// resolved by `circuit_core::units`, whose table also accepts `Ω`, so
/// `1.kΩ` resolves once the trigger character `k` has been seen.
fn is_unit_start(c: char) -> bool {
    c.is_ascii_alphabetic() || is_micro(c)
}

/// Characters that may belong to a unit suffix.
///
/// The whole run is grabbed before resolving so that `1.xV` is reported as one
/// unknown suffix instead of silently splitting into something else.
fn is_unit_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == 'Ω' || is_micro(c)
}

struct Lexer<'a> {
    source: SourceId,
    text: &'a str,
    pos: usize,
    /// Nesting depth of `(`, `[` and `{`; newlines are suppressed inside.
    bracket_depth: usize,
    tokens: Vec<Token>,
    diagnostics: Diagnostics,
}

impl<'a> Lexer<'a> {
    fn new(source: SourceId, text: &'a str) -> Self {
        Self {
            source,
            text,
            pos: 0,
            bracket_depth: 0,
            tokens: Vec::new(),
            diagnostics: Diagnostics::new(),
        }
    }

    fn run(mut self) -> Result<Vec<Token>, Diagnostics> {
        while let Some(c) = self.peek() {
            match c {
                ' ' | '\t' | '\r' => {
                    self.bump();
                }
                '\n' => self.newline(),
                '#' => self.skip_comment(),
                '"' => self.string(),
                ':' => self.colon_or_symbol(),
                '.' => self.dot(),
                c if c.is_ascii_digit() => self.number(),
                c if is_ident_start(c) => self.ident(),
                other => self.punctuation(other),
            }
        }

        let end = self.text.len() as u32;
        self.tokens.push(Token::new(
            TokenKind::Eof,
            SourceSpan::new(self.source, end, end),
        ));

        if self.diagnostics.is_empty() {
            Ok(self.tokens)
        } else {
            Err(self.diagnostics)
        }
    }

    // ---- cursor helpers --------------------------------------------------

    fn peek(&self) -> Option<char> {
        self.text[self.pos..].chars().next()
    }

    fn peek_nth(&self, n: usize) -> Option<char> {
        self.text[self.pos..].chars().nth(n)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    /// Source text between two byte offsets.
    fn slice(&self, range: std::ops::Range<usize>) -> &'a str {
        &self.text[range]
    }

    fn push(&mut self, kind: TokenKind, start: usize) {
        let span = SourceSpan::new(self.source, start as u32, self.pos as u32);
        self.tokens.push(Token::new(kind, span));
    }

    // ---- layout ----------------------------------------------------------

    /// Emit a statement separator unless one of §1.8's continuation rules
    /// applies.
    fn newline(&mut self) {
        let start = self.pos;
        self.bump();

        if self.bracket_depth > 0 {
            return;
        }
        if self
            .tokens
            .last()
            .is_some_and(|previous| previous.kind.is_continuation_after())
        {
            return;
        }
        if self.only_line_breaks_ahead() {
            return;
        }
        self.push(TokenKind::Newline, start);
    }

    /// Whether the rest of the input up to the next real token is only blank
    /// lines and comments, in which case this line break is redundant.
    ///
    /// Scanning bytes is safe here: only ASCII decides the answer, and the
    /// first byte of any multi-byte character is not ASCII, so it reports a
    /// real token.
    fn only_line_breaks_ahead(&self) -> bool {
        let bytes = self.text.as_bytes();
        let mut i = self.pos;
        while i < bytes.len() {
            match bytes[i] {
                b' ' | b'\t' | b'\r' => i += 1,
                // A line break, a comment (which runs to the end of its line),
                // or the end of input: either way nothing that needs this
                // newline follows.
                b'\n' | b'#' => return true,
                _ => return false,
            }
        }
        true // end of input
    }

    fn skip_comment(&mut self) {
        while self.peek().is_some_and(|c| c != '\n') {
            self.bump();
        }
    }

    // ---- tokens ----------------------------------------------------------

    fn ident(&mut self) {
        let start = self.pos;
        self.bump();
        while self.peek().is_some_and(is_ident_continue) {
            self.bump();
        }
        let text = self.slice(start..self.pos).to_string();
        self.push(TokenKind::Ident(text), start);
    }

    fn colon_or_symbol(&mut self) {
        let start = self.pos;
        self.bump();
        if self.peek().is_some_and(is_ident_start) {
            self.bump();
            while self.peek().is_some_and(is_ident_continue) {
                self.bump();
            }
            let name = self.slice(start + 1..self.pos).to_string();
            self.push(TokenKind::Symbol(name), start);
        } else {
            self.push(TokenKind::Colon, start);
        }
    }

    fn string(&mut self) {
        let start = self.pos;
        self.bump(); // opening quote
        let mut value = String::new();
        loop {
            match self.peek() {
                Some('"') => {
                    self.bump();
                    self.push(TokenKind::Str(value), start);
                    return;
                }
                Some('\n') | None => {
                    // No escapes and no line continuation: the string must
                    // close on the line it opened on.
                    let span = SourceSpan::new(self.source, start as u32, self.pos as u32);
                    let diagnostic = Diagnostic::error(
                        Code::Syntax,
                        "unterminated string literal; add the closing `\"` before the end of the line",
                    )
                    .at(span);
                    self.diagnostics.push(diagnostic);
                    self.push(TokenKind::Str(value), start);
                    return;
                }
                // `#{...}` is Ruby string interpolation. Treating it as
                // ordinary text would silently name a device `r#{k}`, so it is
                // refused and the working alternative is named instead.
                Some('#') if self.peek_nth(1) == Some('{') => {
                    let span = SourceSpan::new(self.source, self.pos as u32, (self.pos + 2) as u32);
                    self.diagnostics.push(
                        Diagnostic::error(Code::Syntax, "string interpolation is not supported")
                            .at(span)
                            .with_note("build the name by concatenation instead, e.g. (\"r\" + k)"),
                    );
                    self.bump();
                    self.bump();
                    value.push('#');
                    value.push('{');
                }
                Some(c) => {
                    value.push(c);
                    self.bump();
                }
            }
        }
    }

    /// `.` on its own, `..`, or a float that starts with the dot (`.5`).
    fn dot(&mut self) {
        let start = self.pos;
        self.bump();
        if self.peek() == Some('.') {
            self.bump();
            // `...` is Ruby's exclusive range. Left alone it would lex as
            // `..` followed by the float `.3`, silently turning `0...3` into
            // a range from 0 to 0.3 — which then rounds to a single iteration.
            // Refusing it is the only honest option, since the language has
            // no exclusive range.
            if self.peek() == Some('.') {
                let span = SourceSpan::new(self.source, start as u32, (self.pos + 1) as u32);
                self.diagnostics.push(
                    Diagnostic::error(
                        Code::Syntax,
                        "`...` is not a range operator: this language has only `..`, which includes both ends",
                    )
                    .at(span)
                    .with_note("write `1..3` for 1, 2 and 3"),
                );
                self.bump();
                self.push(TokenKind::DotDot, start);
                return;
            }
            self.push(TokenKind::DotDot, start);
            return;
        }
        if self.peek().is_some_and(|c| c.is_ascii_digit()) {
            // `'.' digit+ exp?` is a float literal (docs/language.md §1.5).
            self.scan_digits();
            self.scan_exponent();
            let text = self.slice(start..self.pos);
            let value = text.parse::<f64>().unwrap_or(f64::NAN);
            self.push(TokenKind::Float(value), start);
            return;
        }
        self.push(TokenKind::Dot, start);
    }

    fn number(&mut self) {
        let start = self.pos;
        self.scan_digits();

        let mut is_float = false;
        if self.peek() == Some('.') && self.peek_nth(1).is_some_and(|c| c.is_ascii_digit()) {
            is_float = true;
            self.bump();
            self.scan_digits();
        }
        if self.scan_exponent() {
            is_float = true;
        }

        let numeric_end = self.pos;
        if self.starts_quantity() {
            let numeric = self.slice(start..numeric_end);
            self.quantity(start, numeric_end, numeric, is_float);
            return;
        }
        let numeric = self.slice(start..numeric_end);
        self.emit_plain_number(start, numeric_end, numeric, is_float);
    }

    fn scan_digits(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
        }
    }

    /// Consume `('e'|'E') ('+'|'-')? digit+` when it is really an exponent
    /// (`1e3` is one literal, `1end` is not).
    fn scan_exponent(&mut self) -> bool {
        if !matches!(self.peek(), Some('e' | 'E')) {
            return false;
        }
        let sign = match self.peek_nth(1) {
            Some('+' | '-') => 2,
            _ => 1,
        };
        if !self.peek_nth(sign).is_some_and(|c| c.is_ascii_digit()) {
            return false;
        }
        self.bump(); // e
        if sign == 2 {
            self.bump();
        }
        self.scan_digits();
        true
    }

    /// Whether a quantity literal starts here: the numeric part is followed by
    /// `.` and a character that could begin a unit suffix.
    fn starts_quantity(&self) -> bool {
        self.peek() == Some('.') && self.peek_nth(1).is_some_and(is_unit_start)
    }

    /// Lex the `.<unit>` tail of a quantity literal.
    ///
    /// The suffix is taken greedily and must resolve in full: `1.kohm` is a
    /// quantity, while the `xV` of `1.xV` is reported as an unknown unit and
    /// the number is kept so that lexing can continue.
    fn quantity(&mut self, start: usize, numeric_end: usize, numeric: &'a str, is_float: bool) {
        self.bump(); // the separating '.'
        let suffix_start = self.pos;
        while self.peek().is_some_and(is_unit_char) {
            self.bump();
        }
        let run = self.slice(suffix_start..self.pos);

        let suffix_len = longest_unit_suffix(run).filter(|len| *len == run.len());
        let Some(len) = suffix_len else {
            let span = SourceSpan::new(self.source, suffix_start as u32, self.pos as u32);
            let message = format!("`{run}` is not a known unit suffix");
            let diagnostic = Diagnostic::error(Code::Syntax, message).at(span).with_note(
                "units are V, A, s, Hz, ohm/Ohm/Ω, F and H, with an optional SI prefix \
                 (f, p, n, u, m, k, M, meg, g, t, a)",
            );
            self.diagnostics.push(diagnostic);
            self.emit_plain_number(start, numeric_end, numeric, is_float);
            return;
        };

        let suffix = &run[..len];
        match build_quantity(numeric, Some(suffix)) {
            Ok(quantity) => {
                let text = self.slice(start..self.pos).to_string();
                let literal = QuantityLiteral {
                    value: quantity.value,
                    dimension: quantity.dimension,
                    text,
                };
                self.push(TokenKind::Quantity(literal), start);
            }
            Err(error) => {
                let span = SourceSpan::new(self.source, start as u32, self.pos as u32);
                let message = format!(
                    "invalid quantity literal `{}`: {error}",
                    self.slice(start..self.pos)
                );
                let diagnostic = Diagnostic::error(Code::Syntax, message).at(span);
                self.diagnostics.push(diagnostic);
                self.emit_plain_number(start, numeric_end, numeric, is_float);
            }
        }
    }

    fn emit_plain_number(&mut self, start: usize, end: usize, numeric: &str, is_float: bool) {
        let span = SourceSpan::new(self.source, start as u32, end as u32);
        let kind = if is_float {
            // Unreachable in practice: the scanner only consumes decimal
            // literals, and overflow parses to infinity rather than failing.
            TokenKind::Float(numeric.parse::<f64>().unwrap_or(f64::NAN))
        } else {
            match numeric.parse::<i64>() {
                Ok(value) => TokenKind::Int(value),
                // Too large for `i64`: keep it as a float instead of dropping
                // the literal (the value range check belongs to elaboration).
                Err(_) => TokenKind::Float(numeric.parse::<f64>().unwrap_or(f64::NAN)),
            }
        };
        self.tokens.push(Token::new(kind, span));
    }

    fn punctuation(&mut self, c: char) {
        let start = self.pos;
        match c {
            '(' => {
                self.bump();
                self.bracket_depth += 1;
                self.push(TokenKind::LParen, start);
            }
            ')' => {
                self.bump();
                self.bracket_depth = self.bracket_depth.saturating_sub(1);
                self.push(TokenKind::RParen, start);
            }
            '[' => {
                self.bump();
                self.bracket_depth += 1;
                self.push(TokenKind::LBracket, start);
            }
            ']' => {
                self.bump();
                self.bracket_depth = self.bracket_depth.saturating_sub(1);
                self.push(TokenKind::RBracket, start);
            }
            '{' => {
                self.bump();
                self.bracket_depth += 1;
                self.push(TokenKind::LBrace, start);
            }
            '}' => {
                self.bump();
                self.bracket_depth = self.bracket_depth.saturating_sub(1);
                self.push(TokenKind::RBrace, start);
            }
            ',' => {
                self.bump();
                self.push(TokenKind::Comma, start);
            }
            '+' => {
                self.bump();
                self.push(TokenKind::Plus, start);
            }
            '-' => {
                self.bump();
                self.push(TokenKind::Minus, start);
            }
            '*' => {
                self.bump();
                self.push(TokenKind::Star, start);
            }
            '/' => {
                self.bump();
                self.push(TokenKind::Slash, start);
            }
            '!' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                    self.push(TokenKind::BangEq, start);
                } else {
                    self.push(TokenKind::Bang, start);
                }
            }
            '<' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                    self.push(TokenKind::Le, start);
                } else {
                    self.push(TokenKind::Lt, start);
                }
            }
            '>' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                    self.push(TokenKind::Ge, start);
                } else {
                    self.push(TokenKind::Gt, start);
                }
            }
            '=' => {
                self.bump();
                if self.peek() == Some('>') {
                    self.bump();
                    self.push(TokenKind::FatArrow, start);
                } else if self.peek() == Some('=') {
                    self.bump();
                    self.push(TokenKind::EqEq, start);
                } else {
                    self.report(
                        start,
                        "`=` is not an operator; the language has no assignment",
                    );
                }
            }
            '&' => {
                self.bump();
                if self.peek() == Some('&') {
                    self.bump();
                    self.push(TokenKind::AmpAmp, start);
                } else {
                    self.report(start, "`&` is not an operator; write `&&` for conjunction");
                }
            }
            '|' => {
                self.bump();
                if self.peek() == Some('|') {
                    self.bump();
                    self.push(TokenKind::PipePipe, start);
                } else {
                    self.report(start, "`|` is not an operator; write `||` for disjunction");
                }
            }
            other => {
                self.bump();
                let message = format!("unexpected character `{other}`");
                self.report(start, message);
            }
        }
    }

    /// Report an error covering the text from `start` to the cursor.
    fn report(&mut self, start: usize, message: impl Into<String>) {
        let span = SourceSpan::new(self.source, start as u32, self.pos as u32);
        self.diagnostics
            .push(Diagnostic::error(Code::Syntax, message).at(span));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circuit_core::units::{CAPACITANCE, FREQUENCY, INDUCTANCE, RESISTANCE, TIME, VOLTAGE};

    fn kinds(src: &str) -> Vec<TokenKind> {
        lex(SourceId(0), src)
            .unwrap_or_else(|d| panic!("lexing `{src}` should succeed, got:\n{d}"))
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    fn diagnostics(src: &str) -> Diagnostics {
        match lex(SourceId(0), src) {
            Ok(tokens) => panic!("expected an error for `{src}`, got {tokens:?}"),
            Err(diagnostics) => diagnostics,
        }
    }

    fn quantity(src: &str) -> QuantityLiteral {
        match kinds(src).as_slice() {
            [TokenKind::Quantity(q), TokenKind::Eof] => q.clone(),
            other => panic!("expected one quantity literal, got {other:?}"),
        }
    }

    /// Relative comparison; prefix scaling happens in binary floating point, so
    /// `100 * 1e-9` is not bit-identical to `1e-7`.
    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-12 * b.abs().max(1.0)
    }

    // ---- quantities ------------------------------------------------------

    #[test]
    fn quantity_literals_carry_value_dimension_and_text() {
        let q = quantity("1.kohm");
        assert_eq!(q.value, 1000.0);
        assert_eq!(q.dimension, RESISTANCE);
        assert_eq!(q.text, "1.kohm");

        let q = quantity("100.nF");
        assert!(close(q.value, 100e-9), "got {}", q.value);
        assert_eq!(q.dimension, CAPACITANCE);
        assert_eq!(q.text, "100.nF");

        let q = quantity("1.us");
        assert!(close(q.value, 1e-6), "got {}", q.value);
        assert_eq!(q.dimension, TIME);

        let q = quantity("10.Hz");
        assert_eq!(q.value, 10.0);
        assert_eq!(q.dimension, FREQUENCY);

        let q = quantity("1.V");
        assert_eq!(q.value, 1.0);
        assert_eq!(q.dimension, VOLTAGE);
        assert_eq!(q.text, "1.V");

        let q = quantity("1.mV");
        assert!(close(q.value, 1e-3), "got {}", q.value);
        assert_eq!(q.dimension, VOLTAGE);

        let q = quantity("1.MHz");
        assert!(close(q.value, 1e6), "got {}", q.value);
        assert_eq!(q.dimension, FREQUENCY);

        let q = quantity("1.megohm");
        assert!(close(q.value, 1e6), "got {}", q.value);
        assert_eq!(q.dimension, RESISTANCE);

        let q = quantity("1.mH");
        assert!(close(q.value, 1e-3), "got {}", q.value);
        assert_eq!(q.dimension, INDUCTANCE);
    }

    #[test]
    fn micro_spellings_agree() {
        for spelling in ["1.µs", "1.μs", "1.us"] {
            let q = quantity(spelling);
            assert!(close(q.value, 1e-6), "{spelling} gave {}", q.value);
            assert_eq!(q.dimension, TIME);
            assert_eq!(q.text, spelling);
        }
    }

    /// The documented case sensitivity rule: `m` is milli, `M` is mega.
    #[test]
    fn milli_and_mega_differ_by_case() {
        let milli = quantity("1.mF");
        let mega = quantity("1.MF");
        assert!(close(milli.value, 1e-3), "got {}", milli.value);
        assert!(close(mega.value, 1e6), "got {}", mega.value);
        assert_eq!(milli.dimension, mega.dimension);
        assert!(!close(milli.value, mega.value));

        let milli = quantity("1.mH");
        let mega = quantity("1.MH");
        assert!(close(milli.value, 1e-3));
        assert!(close(mega.value, 1e6));

        // `mHz` and `MHz` are both frequency but nine orders apart.
        assert!(!close(quantity("1.mHz").value, quantity("1.MHz").value));
    }

    #[test]
    fn quantity_stops_before_punctuation_and_identifiers() {
        assert_eq!(
            kinds("1.kohm, p: :a)"),
            vec![
                TokenKind::Quantity(QuantityLiteral {
                    value: 1000.0,
                    dimension: RESISTANCE,
                    text: "1.kohm".into(),
                }),
                TokenKind::Comma,
                TokenKind::Ident("p".into()),
                TokenKind::Colon,
                TokenKind::Symbol("a".into()),
                TokenKind::RParen,
                TokenKind::Eof,
            ]
        );

        // `kΩ` resolves even though the trigger character was `k`.
        let q = quantity("1.kΩ");
        assert_eq!(q.value, 1000.0);
        assert_eq!(q.dimension, RESISTANCE);
        assert_eq!(q.text, "1.kΩ");
    }

    #[test]
    fn unknown_unit_suffix_is_reported_at_the_suffix() {
        let d = diagnostics("1.xV");
        assert_eq!(d.len(), 1);
        let first = d.iter().next().unwrap();
        assert_eq!(first.code, Code::Syntax);
        assert!(
            first.message.contains("`xV` is not a known unit suffix"),
            "got {}",
            first.message
        );
        let label = first.primary.as_ref().expect("a primary span");
        // "1.xV": the suffix is bytes 2..4.
        assert_eq!(label.span.start, 2);
        assert_eq!(label.span.end, 4);

        for bad in ["1.m", "1.kg", "1.xV", "1.kohmQ"] {
            let d = diagnostics(bad);
            assert!(d.has_errors(), "{bad} should be rejected");
        }
    }

    #[test]
    fn lexing_continues_after_a_bad_unit_and_an_illegal_character() {
        let d = diagnostics("1.xV $\n@");
        let messages: Vec<&str> = d.iter().map(|x| x.message.as_str()).collect();
        assert_eq!(
            d.len(),
            3,
            "expected three independent errors: {messages:?}"
        );
        assert!(messages[0].contains("xV"));
        assert!(messages[1].contains('$'));
        assert!(messages[2].contains('@'));
        assert!(d.iter().all(|x| x.code == Code::Syntax));
    }

    // ---- numbers ---------------------------------------------------------

    #[test]
    fn integers_floats_and_scientific_notation() {
        assert_eq!(kinds("123")[0], TokenKind::Int(123));
        assert_eq!(kinds("1.5")[0], TokenKind::Float(1.5));
        assert_eq!(kinds("1e3")[0], TokenKind::Float(1000.0));
        assert_eq!(kinds("1.5e-6")[0], TokenKind::Float(1.5e-6));
        assert_eq!(kinds("1E+3")[0], TokenKind::Float(1000.0));
        assert_eq!(kinds(".5")[0], TokenKind::Float(0.5));
        // Too large for i64 becomes a float rather than being dropped.
        match kinds("9223372036854775808")[0] {
            TokenKind::Float(v) => assert!(close(v, 9.223372036854776e18)),
            ref other => panic!("expected a float, got {other:?}"),
        }
    }

    /// The critical range case: the quantity path must not swallow `..`.
    #[test]
    fn range_lexes_as_dot_dot() {
        assert_eq!(
            kinds("1..4"),
            vec![
                TokenKind::Int(1),
                TokenKind::DotDot,
                TokenKind::Int(4),
                TokenKind::Eof
            ]
        );
        // `1...4` deliberately does NOT appear here: see
        // `exclusive_range_is_refused_rather_than_misread`.
        assert_eq!(
            kinds("for i in 1..4 do"),
            vec![
                TokenKind::Ident("for".into()),
                TokenKind::Ident("i".into()),
                TokenKind::Ident("in".into()),
                TokenKind::Int(1),
                TokenKind::DotDot,
                TokenKind::Int(4),
                TokenKind::Ident("do".into()),
                TokenKind::Eof,
            ]
        );
        assert_eq!(
            kinds("a.b")[0..3],
            [
                TokenKind::Ident("a".into()),
                TokenKind::Dot,
                TokenKind::Ident("b".into())
            ]
        );
    }

    // ---- layout ----------------------------------------------------------

    #[test]
    fn newlines_are_suppressed_inside_brackets() {
        assert_eq!(
            kinds("f(1,\n  2)[3,\n4]{a: 1,\nb: 2}"),
            vec![
                TokenKind::Ident("f".into()),
                TokenKind::LParen,
                TokenKind::Int(1),
                TokenKind::Comma,
                TokenKind::Int(2),
                TokenKind::RParen,
                TokenKind::LBracket,
                TokenKind::Int(3),
                TokenKind::Comma,
                TokenKind::Int(4),
                TokenKind::RBracket,
                TokenKind::LBrace,
                TokenKind::Ident("a".into()),
                TokenKind::Colon,
                TokenKind::Int(1),
                TokenKind::Comma,
                TokenKind::Ident("b".into()),
                TokenKind::Colon,
                TokenKind::Int(2),
                TokenKind::RBrace,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn newlines_are_suppressed_after_continuation_points() {
        // After a comma, which is how `resistor :r1, p: :a,\n  n: :b` works.
        assert_eq!(
            kinds("a,\nb"),
            vec![
                TokenKind::Ident("a".into()),
                TokenKind::Comma,
                TokenKind::Ident("b".into()),
                TokenKind::Eof
            ]
        );
        // After a binary operator.
        assert_eq!(
            kinds("1 +\n2"),
            vec![
                TokenKind::Int(1),
                TokenKind::Plus,
                TokenKind::Int(2),
                TokenKind::Eof
            ]
        );
        // After a label colon.
        assert_eq!(
            kinds("value:\n1"),
            vec![
                TokenKind::Ident("value".into()),
                TokenKind::Colon,
                TokenKind::Int(1),
                TokenKind::Eof
            ]
        );
        // A comment does not stop the continuation.
        assert_eq!(
            kinds("a,\n# still the same statement\nb"),
            vec![
                TokenKind::Ident("a".into()),
                TokenKind::Comma,
                TokenKind::Ident("b".into()),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn newlines_end_statements_and_blank_runs_collapse() {
        assert_eq!(
            kinds("a\nb"),
            vec![
                TokenKind::Ident("a".into()),
                TokenKind::Newline,
                TokenKind::Ident("b".into()),
                TokenKind::Eof
            ]
        );
        // Three line breaks collapse into one separator, not three.
        assert_eq!(
            kinds("a\n\n\nb"),
            vec![
                TokenKind::Ident("a".into()),
                TokenKind::Newline,
                TokenKind::Ident("b".into()),
                TokenKind::Eof
            ]
        );
        // A trailing line break produces no separator at all.
        assert_eq!(
            kinds("a\n"),
            vec![TokenKind::Ident("a".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn carriage_returns_are_whitespace() {
        assert_eq!(
            kinds("a\r\nb\r\n"),
            vec![
                TokenKind::Ident("a".into()),
                TokenKind::Newline,
                TokenKind::Ident("b".into()),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn comments_run_to_the_end_of_the_line() {
        assert_eq!(
            kinds("x # comment with :symbols and 1.kohm\ny"),
            vec![
                TokenKind::Ident("x".into()),
                TokenKind::Newline,
                TokenKind::Ident("y".into()),
                TokenKind::Eof
            ]
        );
        // A comment-only file has no tokens besides `Eof`.
        assert_eq!(kinds("# nothing here\n"), vec![TokenKind::Eof]);
    }

    // ---- symbols and strings --------------------------------------------

    #[test]
    fn symbols_and_colons_are_distinguished() {
        assert_eq!(
            kinds("p: :vin"),
            vec![
                TokenKind::Ident("p".into()),
                TokenKind::Colon,
                TokenKind::Symbol("vin".into()),
                TokenKind::Eof
            ]
        );
        // A lone colon with no name after it, e.g. a label whose value is on
        // the next line.
        assert_eq!(
            kinds("p:\n5"),
            vec![
                TokenKind::Ident("p".into()),
                TokenKind::Colon,
                TokenKind::Int(5),
                TokenKind::Eof
            ]
        );
        assert_eq!(kinds(":gnd")[0], TokenKind::Symbol("gnd".into()));
    }

    /// Ruby's `...` must not silently become `..` followed by a `.3` float:
    /// `0...3` used to lex as a range from 0 to 0.3, which then rounded to a
    /// single loop iteration instead of reporting anything.
    #[test]
    fn exclusive_range_is_refused_rather_than_misread() {
        let (tokens, diags) = match lex(
            SourceId(0),
            "for k in 0...3 do
",
        ) {
            Ok(t) => (t, Vec::new()),
            Err(d) => (Vec::new(), d.into_vec()),
        };
        let _ = tokens;
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, Code::Syntax);
        assert!(
            diags[0].message.contains("`...` is not a range operator"),
            "{}",
            diags[0].message
        );
        // The note must name the form that does work.
        assert!(diags[0].render_plain().contains("1..3"), "{:?}", diags[0]);
    }

    /// `"r#{k}"` used to be accepted as the literal name `r#{k}`, so a single
    /// use produced a device with a nonsense name and no diagnostic at all.
    #[test]
    fn string_interpolation_is_refused_rather_than_taken_literally() {
        let diags = match lex(
            SourceId(0),
            "resistor \"r#{k}\", value: 1.ohm
",
        ) {
            Ok(_) => Vec::new(),
            Err(d) => d.into_vec(),
        };
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, Code::Syntax);
        assert!(
            diags[0].message.contains("interpolation is not supported"),
            "{}",
            diags[0].message
        );
        // The suggested alternative must be the one that actually works.
        let text = diags[0].render_plain();
        assert!(text.contains(r#"+ k)"#), "{text}");
        assert!(text.contains("instead"), "{text}");
    }

    /// A `#` that is not followed by `{` is still ordinary text, and `#{}`
    /// inside a comment is irrelevant (comments are skipped before strings).
    #[test]
    fn a_plain_hash_inside_a_string_is_still_allowed() {
        assert_eq!(kinds("\"a#b\"")[0], TokenKind::Str("a#b".into()));
    }

    #[test]
    fn strings_have_no_escapes_and_differ_from_symbols() {
        assert_eq!(
            kinds("\"gnd\" :gnd"),
            vec![
                TokenKind::Str("gnd".into()),
                TokenKind::Symbol("gnd".into()),
                TokenKind::Eof
            ]
        );
        // A backslash is just a character.
        assert_eq!(kinds(r#""a\b""#)[0], TokenKind::Str(r"a\b".into()));
        assert_eq!(kinds("\"\"")[0], TokenKind::Str(String::new()));
    }

    #[test]
    fn unterminated_string_is_reported_to_the_end_of_the_line() {
        let d = diagnostics("\"abc\nnext line");
        assert_eq!(d.len(), 1);
        let first = d.iter().next().unwrap();
        assert_eq!(first.code, Code::Syntax);
        assert!(
            first.message.contains("unterminated string"),
            "got {}",
            first.message
        );
        let label = first.primary.as_ref().expect("a primary span");
        assert_eq!((label.span.start, label.span.end), (0, 4));

        assert!(
            diagnostics("\"abc").has_errors(),
            "unterminated at end of input"
        );
    }

    // ---- operators -------------------------------------------------------

    #[test]
    fn operators_use_longest_match() {
        assert_eq!(
            kinds("<= >= == != && || => .."),
            vec![
                TokenKind::Le,
                TokenKind::Ge,
                TokenKind::EqEq,
                TokenKind::BangEq,
                TokenKind::AmpAmp,
                TokenKind::PipePipe,
                TokenKind::FatArrow,
                TokenKind::DotDot,
                TokenKind::Eof,
            ]
        );
        assert_eq!(
            kinds("+-*/!<>()[]{},.:"),
            vec![
                TokenKind::Plus,
                TokenKind::Minus,
                TokenKind::Star,
                TokenKind::Slash,
                TokenKind::Bang,
                TokenKind::Lt,
                TokenKind::Gt,
                TokenKind::LParen,
                TokenKind::RParen,
                TokenKind::LBracket,
                TokenKind::RBracket,
                TokenKind::LBrace,
                TokenKind::RBrace,
                TokenKind::Comma,
                TokenKind::Dot,
                TokenKind::Colon,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lone_assignment_operators_are_rejected() {
        let d = diagnostics("a = b");
        assert_eq!(d.len(), 1);
        let first = d.iter().next().unwrap();
        assert_eq!(first.code, Code::Syntax);
        assert!(
            first.message.contains("no assignment"),
            "got {}",
            first.message
        );
        let label = first.primary.as_ref().expect("a primary span");
        assert_eq!((label.span.start, label.span.end), (2, 3));

        assert!(diagnostics("a & b").has_errors());
        assert!(diagnostics("a | b").has_errors());
        // But the doubled forms are fine.
        assert_eq!(kinds("a && b")[1], TokenKind::AmpAmp);
        assert_eq!(kinds("a || b")[1], TokenKind::PipePipe);
    }

    #[test]
    fn illegal_characters_are_reported_at_their_own_span() {
        let d = diagnostics("node $ x");
        assert_eq!(d.len(), 1);
        let first = d.iter().next().unwrap();
        assert_eq!(first.code, Code::Syntax);
        assert!(first.message.contains('$'), "got {}", first.message);
        let label = first.primary.as_ref().expect("a primary span");
        assert_eq!((label.span.start, label.span.end), (5, 6));
    }

    // ---- eof -------------------------------------------------------------

    #[test]
    fn eof_is_always_last_and_sits_at_the_end_of_input() {
        let eof = |src: &str| {
            let tokens = lex(SourceId(0), src).unwrap_or_else(|d| panic!("{d}"));
            let last = tokens.last().expect("at least `Eof`").clone();
            assert_eq!(last.kind, TokenKind::Eof);
            assert_eq!(last.span.start as usize, src.len());
            assert_eq!(last.span.end as usize, src.len());
            assert_eq!(
                tokens.iter().filter(|t| t.kind == TokenKind::Eof).count(),
                1
            );
        };
        eof("");
        eof("node :a");
        eof("node :a\n");
        eof("# only a comment");
    }

    #[test]
    fn spans_cover_the_token_text() {
        let tokens = lex(SourceId(0), "resistor :r1, value: 1.kohm").unwrap();
        let text_of =
            |t: &Token| &"resistor :r1, value: 1.kohm"[t.span.start as usize..t.span.end as usize];
        assert_eq!(text_of(&tokens[0]), "resistor");
        assert_eq!(text_of(&tokens[1]), ":r1");
        assert_eq!(text_of(&tokens[2]), ",");
        assert_eq!(text_of(&tokens[3]), "value");
        assert_eq!(text_of(&tokens[4]), ":");
        assert_eq!(text_of(&tokens[5]), "1.kohm");
    }
}
