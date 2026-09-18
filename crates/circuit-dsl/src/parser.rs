//! Recursive-descent parser: tokens in, [`Program`](crate::ast::Program) out.
//!
//! The parser is purely syntactic. Names are kept as strings, no dimension is
//! checked, and nothing is looked up — declaration order, duplicate names and
//! unknown references are elaboration's business. What this module does own is
//! *shape*: that `1 + 2 * 3` groups as `1 + (2 * 3)`, that every expression
//! carries a span covering its own text, and that a syntax error inside a
//! statement does not stop the rest of the file from being checked.
//!
//! Recovery follows one rule: an error is reported, then tokens are skipped to
//! the end of the statement, so a file with three bad lines produces three
//! diagnostics rather than one.

use circuit_core::diagnostic::{Code, Diagnostic, Diagnostics};
use circuit_core::span::SourceSpan;

use crate::ast::{
    AnalysisCall, Arg, BinaryOp, Call, CircuitDef, DeviceStmt, DeviceStmtKind, DictEntry, ExpStmt,
    ExperimentDef, Expr, ExprKind, ForIter, ForStmt, IfStmt, InstanceStmt, ModelDecl, NodeDecl,
    ParamDecl, Program, SpannedName, Stmt, UnaryOp,
};
use crate::token::{Token, TokenKind, is_reserved_name};

/// Parse a whole file.
///
/// Returns every syntax error found, or the program when the file is clean.
pub fn parse(tokens: &[Token]) -> Result<Program, Diagnostics> {
    let mut parser = Parser::new(tokens);
    let program = parser.program();
    if parser.diagnostics.is_empty() {
        Ok(program)
    } else {
        Err(parser.diagnostics)
    }
}

/// Stand-in for "past the end of the token slice", so that every loop has a
/// terminating token to look at even for an empty slice.
const EOF_TOKEN: Token = Token {
    kind: TokenKind::Eof,
    span: SourceSpan::synthetic(),
};

/// Why a block stopped being read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockStop {
    /// The `end` keyword is current and has not been consumed.
    End,
    /// An `elsif` arm follows.
    Elsif,
    /// An `else` arm follows.
    Else,
    /// Input ran out before the block was closed.
    Eof,
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    diagnostics: Diagnostics,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self {
            tokens,
            pos: 0,
            diagnostics: Diagnostics::new(),
        }
    }

    // ---- cursor ----------------------------------------------------------

    fn current(&self) -> &'a Token {
        self.tokens.get(self.pos).unwrap_or(&EOF_TOKEN)
    }

    fn kind(&self) -> &'a TokenKind {
        &self.current().kind
    }

    fn span(&self) -> SourceSpan {
        self.current().span
    }

    fn peek_kind(&self, n: usize) -> &'a TokenKind {
        self.tokens
            .get(self.pos + n)
            .map(|token| &token.kind)
            .unwrap_or(&TokenKind::Eof)
    }

    fn at_eof(&self) -> bool {
        matches!(self.kind(), TokenKind::Eof)
    }

    /// Consume the current token. `Eof` is never consumed, so a loop that
    /// checks [`Parser::at_eof`] always terminates.
    fn bump(&mut self) -> Token {
        let token = self.current().clone();
        if !self.at_eof() {
            self.pos += 1;
        }
        token
    }

    fn ident_text(&self) -> Option<&'a str> {
        match self.kind() {
            TokenKind::Ident(text) => Some(text.as_str()),
            _ => None,
        }
    }

    fn at_ident(&self, word: &str) -> bool {
        self.ident_text() == Some(word)
    }

    fn at_token(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(self.kind()) == std::mem::discriminant(kind)
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.at_token(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_ident(&mut self, word: &str) -> bool {
        if self.at_ident(word) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.kind(), TokenKind::Newline) {
            self.bump();
        }
    }

    /// Tokens that end a statement: a line break, end of file, or the word
    /// that closes the enclosing block.
    fn at_stmt_end(&self) -> bool {
        match self.kind() {
            TokenKind::Newline | TokenKind::Eof => true,
            TokenKind::Ident(word) => matches!(word.as_str(), "end" | "else" | "elsif"),
            _ => false,
        }
    }

    /// Span of the last consumed token, skipping any line breaks.
    fn prev_end(&self) -> SourceSpan {
        let mut i = self.pos;
        while i > 0 {
            i -= 1;
            if !matches!(self.tokens[i].kind, TokenKind::Newline) {
                return self.tokens[i].span;
            }
        }
        SourceSpan::synthetic()
    }

    // ---- diagnostics and recovery ---------------------------------------

    fn report(&mut self, span: SourceSpan, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(Code::Syntax, message).at(span));
    }

    fn error<T>(&mut self, span: SourceSpan, message: impl Into<String>) -> Option<T> {
        self.report(span, message);
        None
    }

    /// Skip to the end of the current statement, keeping bracket nesting so a
    /// newline inside `(` ... `)` is not mistaken for a statement end.
    ///
    /// The line break itself is left in place, so a caller that goes on to
    /// check for a statement terminator still sees it.
    fn skip_to_stmt_end(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.kind() {
                TokenKind::Eof => break,
                TokenKind::Newline if depth == 0 => break,
                TokenKind::Ident(word)
                    if depth == 0 && matches!(word.as_str(), "end" | "else" | "elsif") =>
                {
                    break;
                }
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                    depth += 1;
                    self.bump();
                }
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    depth = depth.saturating_sub(1);
                    self.bump();
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    /// Run a statement parser, recovering at the end of the statement if it
    /// fails. The extra bump guarantees progress even for a parser that
    /// returned before consuming anything.
    fn guarded<T>(&mut self, parse: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        let before = self.pos;
        let result = parse(&mut *self);
        if result.is_none() {
            if self.pos == before && !self.at_eof() {
                self.bump();
            }
            self.skip_to_stmt_end();
        }
        result
    }

    /// A statement ends at a line break, at the end of the file, or just
    /// before the word that closes the enclosing block.
    fn finish_stmt(&mut self) -> Option<()> {
        if matches!(self.kind(), TokenKind::Newline) {
            self.skip_newlines();
            return Some(());
        }
        if self.at_stmt_end() {
            return Some(());
        }
        let token = self.current().clone();
        let message = format!(
            "unexpected {}; expected the end of the statement",
            token.kind.describe()
        );
        self.error(token.span, message)
    }

    // ---- shared pieces ---------------------------------------------------

    /// A `:name` token, a `"name"` string, or a parenthesised expression.
    ///
    /// The string and expression forms exist so that a loop can generate
    /// distinct names: `resistor ("r" + i), p: :a, n: :b, value: 1.kohm`.
    fn expect_symbol(&mut self, what: &str) -> Option<SpannedName> {
        match self.kind().clone() {
            TokenKind::Symbol(name) => {
                let token = self.bump();
                Some(SpannedName::new(name, token.span))
            }
            TokenKind::Str(name) => {
                let token = self.bump();
                Some(SpannedName::new(name, token.span))
            }
            TokenKind::LParen => {
                let open = self.span();
                self.bump();
                self.skip_newlines();
                let inner = self.expr()?;
                self.skip_newlines();
                if !self.eat(&TokenKind::RParen) {
                    let found = self.kind().clone();
                    let message = format!("expected `)`, found {}", found.describe());
                    return self.error(open, message);
                }
                let span = open.merge(self.prev_end());
                Some(SpannedName::expressed(inner, span))
            }
            other => {
                let span = self.span();
                let message = format!("expected {what}, found {}", other.describe());
                self.error(span, message)
            }
        }
    }

    /// An argument label: a bare word followed by `:`.
    fn arg_label(&mut self) -> Option<(String, SourceSpan)> {
        match self.kind().clone() {
            TokenKind::Ident(name) => {
                let token = self.bump();
                if !self.eat(&TokenKind::Colon) {
                    let found = self.kind().clone();
                    let message =
                        format!("expected `:` after `{name}`, found {}", found.describe());
                    return self.error(token.span, message);
                }
                Some((name, token.span))
            }
            other => {
                let span = self.span();
                let message = format!(
                    "unexpected {}; expected an argument of the form `name: value`",
                    other.describe()
                );
                self.error(span, message)
            }
        }
    }

    fn arg(&mut self) -> Option<Arg> {
        let (name, name_span) = self.arg_label()?;
        let value = self.expr()?;
        Some(Arg {
            name,
            name_span,
            value,
        })
    }

    /// `, label: value, ...`, the argument tail of a statement whose head
    /// already ended, as in `resistor :r1, p: :a`.
    ///
    /// A trailing comma is accepted, which is what makes multi-line argument
    /// lists convenient.
    fn arg_list(&mut self) -> Vec<Arg> {
        let mut args = Vec::new();
        while self.eat(&TokenKind::Comma) {
            self.skip_newlines();
            if self.at_stmt_end() {
                break;
            }
            match self.arg() {
                Some(arg) => args.push(arg),
                None => {
                    self.skip_to_stmt_end();
                    break;
                }
            }
        }
        args
    }

    /// Arguments that follow a bare keyword with no comma before the first
    /// one, as in `ac from: 10.Hz, to: 10.MHz`.
    fn arg_list_open(&mut self) -> Vec<Arg> {
        let mut args = Vec::new();
        if self.at_stmt_end() {
            return args;
        }
        loop {
            match self.arg() {
                Some(arg) => args.push(arg),
                None => {
                    self.skip_to_stmt_end();
                    break;
                }
            }
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            self.skip_newlines();
            if self.at_stmt_end() {
                break;
            }
        }
        args
    }

    /// `[:a, :b]`, used by `subcircuit ... ports:`.
    fn symbol_list(&mut self) -> Option<Vec<SpannedName>> {
        if !self.eat(&TokenKind::LBracket) {
            let token = self.current().clone();
            let message = format!(
                "expected `[` to start the port list, found {}",
                token.kind.describe()
            );
            return self.error(token.span, message);
        }
        let mut names = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::RBracket) {
                break;
            }
            if self.at_eof() {
                let span = self.span();
                return self.error(span, "expected `]` to close the port list");
            }
            names.push(self.expect_symbol("a port name (a symbol such as `:input`)")?);
            self.skip_newlines();
            if self.eat(&TokenKind::Comma) {
                continue;
            }
            if self.eat(&TokenKind::RBracket) {
                break;
            }
            let token = self.current().clone();
            let message = format!(
                "expected `,` or `]` in the port list, found {}",
                token.kind.describe()
            );
            return self.error(token.span, message);
        }
        if names.is_empty() {
            let span = self.span();
            return self.error(span, "expected at least one port name in the port list");
        }
        Some(names)
    }

    /// Comma-separated expressions, as used by `save`.
    fn expr_list(&mut self, what: &str) -> Option<Vec<Expr>> {
        let mut items = Vec::new();
        if self.at_stmt_end() {
            let span = self.span();
            return self.error(span, format!("expected {what}"));
        }
        loop {
            items.push(self.expr()?);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            self.skip_newlines();
            if self.at_stmt_end() {
                break; // trailing comma
            }
        }
        Some(items)
    }

    // ---- top level -------------------------------------------------------

    fn program(&mut self) -> Program {
        let mut program = Program::default();
        loop {
            self.skip_newlines();
            if self.at_eof() {
                break;
            }
            let before = self.pos;
            match self.ident_text() {
                Some("circuit") => {
                    if let Some(def) = self.guarded(|p| p.circuit(false)) {
                        program.circuits.push(def);
                    }
                }
                Some("subcircuit") => {
                    if let Some(def) = self.guarded(|p| p.circuit(true)) {
                        program.circuits.push(def);
                    }
                }
                Some("experiment") => {
                    if let Some(def) = self.guarded(Self::experiment) {
                        program.experiments.push(def);
                    }
                }
                _ => {
                    let token = self.current().clone();
                    let message = format!(
                        "unexpected {}; expected `circuit`, `subcircuit`, or `experiment`",
                        token.kind.describe()
                    );
                    self.report(token.span, message);
                    self.skip_to_stmt_end();
                }
            }
            if self.pos == before && !self.at_eof() {
                self.bump();
            }
        }
        program
    }

    fn circuit(&mut self, is_subcircuit: bool) -> Option<CircuitDef> {
        let keyword = self.bump();
        let word = if is_subcircuit {
            "subcircuit"
        } else {
            "circuit"
        };

        let Some(name) =
            self.expect_symbol(&format!("a name after `{word}`, as in `{word} :name`"))
        else {
            self.skip_to_stmt_end();
            return None;
        };

        let mut ports: Vec<SpannedName> = Vec::new();
        while self.eat(&TokenKind::Comma) {
            self.skip_newlines();
            let Some((label, label_span)) = self.arg_label() else {
                self.skip_to_stmt_end();
                return None;
            };
            if label != "ports" {
                self.report(
                    label_span,
                    format!("unexpected argument `{label}`; `{word}` accepts only `ports:`"),
                );
                self.skip_to_stmt_end();
                return None;
            }
            let Some(list) = self.symbol_list() else {
                self.skip_to_stmt_end();
                return None;
            };
            ports = list;
        }

        if is_subcircuit && ports.is_empty() {
            self.report(
                name.span,
                "a `subcircuit` must declare its ports, as in `subcircuit :name, ports: [:a, :b]`",
            );
        }

        if !self.eat_ident("do") {
            let token = self.current().clone();
            let message = format!(
                "expected `do` after `{word} :{}`, found {}",
                name.name,
                token.kind.describe()
            );
            self.report(token.span, message);
            self.skip_to_stmt_end();
            return None;
        }

        let (body, stop) = self.block(false);
        let end = if stop == BlockStop::End {
            self.bump().span
        } else {
            self.report(
                keyword.span,
                format!("expected `end` to close `{word} :{}`", name.name),
            );
            self.prev_end()
        };

        Some(CircuitDef {
            name: name.name,
            is_subcircuit,
            ports,
            body,
            span: keyword.span.merge(end),
            name_span: name.span,
        })
    }

    /// Circuit statements up to the token that ends the block.
    fn block(&mut self, allow_else: bool) -> (Vec<Stmt>, BlockStop) {
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_eof() {
                return (stmts, BlockStop::Eof);
            }
            match self.ident_text() {
                Some("end") => return (stmts, BlockStop::End),
                Some("elsif") if allow_else => return (stmts, BlockStop::Elsif),
                Some("else") if allow_else => return (stmts, BlockStop::Else),
                Some("else" | "elsif") => {
                    let token = self.current().clone();
                    let message = format!(
                        "unexpected {}; there is no matching `if`",
                        token.kind.describe()
                    );
                    self.report(token.span, message);
                    self.bump();
                    self.skip_to_stmt_end();
                }
                _ => {
                    if let Some(stmt) = self.guarded(Self::circuit_stmt) {
                        stmts.push(stmt);
                    }
                }
            }
        }
    }

    // ---- circuit statements ---------------------------------------------

    fn circuit_stmt(&mut self) -> Option<Stmt> {
        let Some(word) = self.ident_text().map(str::to_string) else {
            let token = self.current().clone();
            let message = format!(
                "unexpected {}; expected a statement such as `node` or `param`",
                token.kind.describe()
            );
            self.report(token.span, message);
            self.skip_to_stmt_end();
            return None;
        };
        match word.as_str() {
            "param" => self.param_stmt(),
            "node" => self.node_stmt(),
            "resistor" => self.device_stmt(DeviceStmtKind::Resistor),
            "capacitor" => self.device_stmt(DeviceStmtKind::Capacitor),
            "inductor" => self.device_stmt(DeviceStmtKind::Inductor),
            "voltage_source" => self.device_stmt(DeviceStmtKind::VoltageSource),
            "current_source" => self.device_stmt(DeviceStmtKind::CurrentSource),
            "diode" => self.device_stmt(DeviceStmtKind::Diode),
            "model" => self.model_stmt(),
            "instance" => self.instance_stmt(),
            "for" => self.for_stmt(),
            "if" => self.if_stmt(),
            other => {
                let token = self.current().clone();
                let message = format!(
                    "unexpected `{other}`; expected a circuit statement (`param`, `node`, \
                     `resistor`, `capacitor`, `inductor`, `voltage_source`, `current_source`, \
                     `diode`, `model`, `instance`, `for`, or `if`)"
                );
                self.report(token.span, message);
                self.bump();
                self.skip_to_stmt_end();
                None
            }
        }
    }

    fn param_stmt(&mut self) -> Option<Stmt> {
        let keyword = self.bump();
        let name = self
            .expect_symbol("a parameter name after `param`, as in `param :r, default: 1.kohm`")?;
        let mut default = None;
        if self.eat(&TokenKind::Comma) {
            self.skip_newlines();
            let (label, label_span) = self.arg_label()?;
            if label != "default" {
                let message =
                    format!("unexpected argument `{label}`; `param` accepts only `default:`");
                return self.error(label_span, message);
            }
            default = Some(self.expr()?);
        }
        let span = keyword.span.merge(self.prev_end());
        self.finish_stmt()?;
        Some(Stmt::Param(ParamDecl {
            name,
            default,
            span,
        }))
    }

    fn node_stmt(&mut self) -> Option<Stmt> {
        let keyword = self.bump();
        let mut names = Vec::new();
        loop {
            names.push(self.expect_symbol("a node name after `node`, as in `node :vin, :vout`")?);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            self.skip_newlines();
            if self.at_stmt_end() {
                break; // trailing comma
            }
        }
        let span = keyword.span.merge(self.prev_end());
        self.finish_stmt()?;
        Some(Stmt::Node(NodeDecl { names, span }))
    }

    fn device_stmt(&mut self, kind: DeviceStmtKind) -> Option<Stmt> {
        let keyword = self.bump();
        let word = kind.keyword();
        let name = self.expect_symbol(&format!(
            "a name after `{word}`, as in `{word} :name, p: :a, n: :b`"
        ))?;
        let args = self.arg_list();
        let span = keyword.span.merge(self.prev_end());
        self.finish_stmt()?;
        Some(Stmt::Device(DeviceStmt {
            kind,
            name,
            args,
            span,
        }))
    }

    fn model_stmt(&mut self) -> Option<Stmt> {
        let keyword = self.bump();
        let name =
            self.expect_symbol("a model name after `model`, as in `model :d1, type: :diode`")?;
        let args = self.arg_list();
        let span = keyword.span.merge(self.prev_end());
        self.finish_stmt()?;
        Some(Stmt::Model(ModelDecl { name, args, span }))
    }

    fn instance_stmt(&mut self) -> Option<Stmt> {
        let keyword = self.bump();
        let name = self.expect_symbol(
            "an instance name after `instance`, as in `instance :stage1, of: :lowpass`",
        )?;
        let args = self.arg_list();

        let mut of: Option<SpannedName> = None;
        let mut ports: Vec<DictEntry> = Vec::new();
        let mut params: Vec<DictEntry> = Vec::new();
        for arg in args {
            match arg.name.as_str() {
                "of" => match &arg.value.kind {
                    ExprKind::Symbol(subcircuit) => {
                        of = Some(SpannedName::new(subcircuit.clone(), arg.value.span));
                    }
                    _ => {
                        let message = format!(
                            "`of:` requires a subcircuit name (a symbol such as `:lowpass`), found {}",
                            describe_expr(&arg.value)
                        );
                        self.report(arg.value.span, message);
                    }
                },
                "ports" | "params" => match &arg.value.kind {
                    ExprKind::Dict(entries) => {
                        if arg.name == "ports" {
                            ports = entries.clone();
                        } else {
                            params = entries.clone();
                        }
                    }
                    _ => {
                        let message = format!(
                            "`{}:` requires a dictionary such as `{{ input: :vin }}`, found {}",
                            arg.name,
                            describe_expr(&arg.value)
                        );
                        self.report(arg.value.span, message);
                    }
                },
                other => {
                    let name_span = arg.name_span;
                    let message = format!(
                        "unexpected argument `{other}`; `instance` accepts `of:`, `ports:`, and `params:`"
                    );
                    self.report(name_span, message);
                }
            }
        }

        let Some(of) = of else {
            let span = keyword.span;
            return self.error(span, "expected `of: :subcircuit` in `instance`");
        };
        let span = keyword.span.merge(self.prev_end());
        self.finish_stmt()?;
        Some(Stmt::Instance(InstanceStmt {
            name,
            of,
            ports,
            params,
            span,
        }))
    }

    fn for_stmt(&mut self) -> Option<Stmt> {
        let keyword = self.bump();
        let var = match self.kind().clone() {
            TokenKind::Ident(text) if !is_reserved_name(&text) => {
                let token = self.bump();
                SpannedName::new(text, token.span)
            }
            other => {
                let span = self.span();
                let message = match &other {
                    TokenKind::Ident(text) => format!(
                        "expected a loop variable after `for`, found `{text}`; `{text}` is a reserved word"
                    ),
                    _ => format!(
                        "expected a loop variable after `for`, found {}",
                        other.describe()
                    ),
                };
                self.report(span, message);
                self.skip_to_stmt_end();
                return None;
            }
        };
        if !self.eat_ident("in") {
            let token = self.current().clone();
            let message = format!(
                "expected `in` after `for {}`, found {}",
                var.name,
                token.kind.describe()
            );
            self.report(token.span, message);
            self.skip_to_stmt_end();
            return None;
        }

        let first = self.expr()?;
        let iter = if self.eat(&TokenKind::DotDot) {
            let last = self.expr()?;
            ForIter::Range {
                start: first,
                end: last,
            }
        } else {
            ForIter::List(first)
        };

        if !self.eat_ident("do") {
            let token = self.current().clone();
            let message = format!(
                "expected `do` after `for {} in ...`, found {}",
                var.name,
                token.kind.describe()
            );
            self.report(token.span, message);
            self.skip_to_stmt_end();
            return None;
        }

        let (body, stop) = self.block(false);
        let end = if stop == BlockStop::End {
            self.bump().span
        } else {
            self.report(
                keyword.span,
                format!("expected `end` to close `for {}`", var.name),
            );
            self.prev_end()
        };
        Some(Stmt::For(ForStmt {
            var,
            iter,
            body,
            span: keyword.span.merge(end),
        }))
    }

    fn if_stmt(&mut self) -> Option<Stmt> {
        let keyword = self.bump();
        let condition = self.expr()?;
        if !self.eat_ident("do") {
            let token = self.current().clone();
            let message = format!(
                "expected `do` after the `if` condition, found {}",
                token.kind.describe()
            );
            self.report(token.span, message);
            self.skip_to_stmt_end();
            return None;
        }

        let mut arms = Vec::new();
        let mut else_body = None;
        let (body, mut stop) = self.block(true);
        arms.push((condition, body));
        let mut end = None;

        loop {
            match stop {
                BlockStop::End => {
                    end = Some(self.bump().span);
                    break;
                }
                BlockStop::Eof => {
                    self.report(keyword.span, "expected `end` to close `if`");
                    break;
                }
                BlockStop::Elsif => {
                    self.bump();
                    let condition = self.expr()?;
                    if !self.eat_ident("do") {
                        let token = self.current().clone();
                        let message = format!(
                            "expected `do` after the `elsif` condition, found {}",
                            token.kind.describe()
                        );
                        self.report(token.span, message);
                        self.skip_to_stmt_end();
                        return None;
                    }
                    let (body, next) = self.block(true);
                    arms.push((condition, body));
                    stop = next;
                }
                BlockStop::Else => {
                    self.bump();
                    let (body, next) = self.block(false);
                    else_body = Some(body);
                    stop = next;
                }
            }
        }

        let end = end.unwrap_or_else(|| self.prev_end());
        Some(Stmt::If(IfStmt {
            arms,
            else_body,
            span: keyword.span.merge(end),
        }))
    }

    // ---- experiments -----------------------------------------------------

    fn experiment(&mut self) -> Option<ExperimentDef> {
        let keyword = self.bump();
        let name = self.expect_symbol(
            "a name after `experiment`, as in `experiment :response, circuit: :rc_filter`",
        )?;

        let mut circuit: Option<SpannedName> = None;
        while self.eat(&TokenKind::Comma) {
            self.skip_newlines();
            let Some((label, label_span)) = self.arg_label() else {
                self.skip_to_stmt_end();
                return None;
            };
            if label != "circuit" {
                self.report(
                    label_span,
                    format!("unexpected argument `{label}`; `experiment` accepts only `circuit:`"),
                );
                self.skip_to_stmt_end();
                return None;
            }
            let Some(target) =
                self.expect_symbol("a circuit name after `circuit:`, as in `circuit: :rc_filter`")
            else {
                self.skip_to_stmt_end();
                return None;
            };
            circuit = Some(target);
        }

        let Some(circuit) = circuit else {
            let span = keyword.span;
            return self.error(span, "expected `circuit: :name` in `experiment`");
        };

        if !self.eat_ident("do") {
            let token = self.current().clone();
            let message = format!(
                "expected `do` after `experiment :{}`, found {}",
                name.name,
                token.kind.describe()
            );
            self.report(token.span, message);
            self.skip_to_stmt_end();
            return None;
        }

        let (body, stop) = self.exp_block();
        let end = if stop == BlockStop::End {
            self.bump().span
        } else {
            self.report(
                keyword.span,
                format!("expected `end` to close `experiment :{}`", name.name),
            );
            self.prev_end()
        };

        Some(ExperimentDef {
            name,
            circuit,
            body,
            span: keyword.span.merge(end),
        })
    }

    fn exp_block(&mut self) -> (Vec<ExpStmt>, BlockStop) {
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_eof() {
                return (stmts, BlockStop::Eof);
            }
            match self.ident_text() {
                Some("end") => return (stmts, BlockStop::End),
                Some("else" | "elsif") => {
                    let token = self.current().clone();
                    let message = format!(
                        "unexpected {}; there is no matching `if`",
                        token.kind.describe()
                    );
                    self.report(token.span, message);
                    self.bump();
                    self.skip_to_stmt_end();
                }
                _ => {
                    if let Some(stmt) = self.guarded(Self::exp_stmt) {
                        stmts.push(stmt);
                    }
                }
            }
        }
    }

    fn exp_stmt(&mut self) -> Option<ExpStmt> {
        let Some(word) = self.ident_text().map(str::to_string) else {
            let token = self.current().clone();
            let message = format!(
                "unexpected {}; expected a statement such as `op` or `save`",
                token.kind.describe()
            );
            self.report(token.span, message);
            self.skip_to_stmt_end();
            return None;
        };
        match word.as_str() {
            "op" => {
                let token = self.bump();
                self.finish_stmt()?;
                Some(ExpStmt::Op { span: token.span })
            }
            "dc" => Some(ExpStmt::Dc(self.analysis_call()?)),
            "ac" => Some(ExpStmt::Ac(self.analysis_call()?)),
            "tran" => Some(ExpStmt::Tran(self.analysis_call()?)),
            "save" => {
                let keyword = self.bump();
                let probes =
                    self.expr_list("at least one probe after `save`, as in `save v(:out)`")?;
                let span = keyword.span.merge(self.prev_end());
                self.finish_stmt()?;
                Some(ExpStmt::Save { probes, span })
            }
            "param" => {
                let keyword = self.bump();
                let name = self.expect_symbol(
                    "a parameter name after `param`, as in `param :r, value: 2.kohm`",
                )?;
                if !self.eat(&TokenKind::Comma) {
                    let span = self.span();
                    return self
                        .error(span, "expected `, value: <expression>` after `param :name`");
                }
                self.skip_newlines();
                let (label, label_span) = self.arg_label()?;
                if label != "value" {
                    let message =
                        format!("unexpected argument `{label}`; `param` accepts only `value:`");
                    return self.error(label_span, message);
                }
                let value = self.expr()?;
                let span = keyword.span.merge(value.span);
                self.finish_stmt()?;
                Some(ExpStmt::Param { name, value, span })
            }
            "measure" => {
                let keyword = self.bump();
                let name = self.expect_symbol(
                    "a measurement name after `measure`, as in `measure :vmax, max: v(:out)`",
                )?;
                if !self.eat(&TokenKind::Comma) {
                    let span = self.span();
                    return self.error(
                        span,
                        "expected `, <kind>: <probe>` after `measure :name`, as in `measure :vmax, max: v(:out)`",
                    );
                }
                self.skip_newlines();
                let (kind, kind_span) = self.arg_label()?;
                let target = self.expr()?;
                let span = keyword.span.merge(target.span);
                self.finish_stmt()?;
                Some(ExpStmt::Measure {
                    name,
                    kind,
                    kind_span,
                    target,
                    span,
                })
            }
            other => {
                let token = self.current().clone();
                let message = format!(
                    "unexpected `{other}`; expected an experiment statement (`op`, `dc`, `ac`, \
                     `tran`, `save`, `param`, or `measure`)"
                );
                self.report(token.span, message);
                self.bump();
                self.skip_to_stmt_end();
                None
            }
        }
    }

    /// `dc`/`ac`/`tran`: a keyword plus named arguments.
    fn analysis_call(&mut self) -> Option<AnalysisCall> {
        let keyword = self.bump();
        let args = self.arg_list_open();
        let span = keyword.span.merge(self.prev_end());
        self.finish_stmt()?;
        Some(AnalysisCall { args, span })
    }

    // ---- expressions -----------------------------------------------------

    fn expr(&mut self) -> Option<Expr> {
        self.binary(1)
    }

    /// Precedence climbing over the table in `docs/language.md` §2.1; every
    /// operator is left-associative.
    fn binary(&mut self, min_level: u8) -> Option<Expr> {
        let mut lhs = self.unary()?;
        while let Some((op, level)) = binary_op(self.kind()) {
            if level < min_level {
                break;
            }
            self.bump();
            let rhs = self.binary(level + 1)?;
            let span = lhs.span.merge(rhs.span);
            lhs = Expr::new(
                ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            );
        }
        Some(lhs)
    }

    fn unary(&mut self) -> Option<Expr> {
        let op = match self.kind() {
            TokenKind::Minus => UnaryOp::Neg,
            TokenKind::Plus => UnaryOp::Pos,
            TokenKind::Bang => UnaryOp::Not,
            _ => return self.atom(),
        };
        let op_span = self.span();
        self.bump();
        let rhs = self.unary()?;
        let span = op_span.merge(rhs.span);
        Some(Expr::new(
            ExprKind::Unary {
                op,
                rhs: Box::new(rhs),
            },
            span,
        ))
    }

    fn atom(&mut self) -> Option<Expr> {
        let token = self.current().clone();
        match token.kind {
            TokenKind::Int(value) => {
                self.bump();
                Some(Expr::new(ExprKind::Int(value), token.span))
            }
            TokenKind::Float(value) => {
                self.bump();
                Some(Expr::new(ExprKind::Float(value), token.span))
            }
            TokenKind::Quantity(literal) => {
                self.bump();
                Some(Expr::new(ExprKind::Quantity(literal), token.span))
            }
            TokenKind::Str(value) => {
                self.bump();
                Some(Expr::new(ExprKind::Str(value), token.span))
            }
            TokenKind::Symbol(name) => {
                self.bump();
                let start = token.span;
                // A hierarchy path such as `:stage1.r1` names one thing: the
                // flattened name the elaborator already prints in diagnostics
                // and CSV headers. Reading it back as separate tokens would
                // make the language unable to ask about a subcircuit's innards.
                let mut name = name;
                let mut end = start;
                loop {
                    let part = match (self.kind(), self.peek_kind(1)) {
                        (TokenKind::Dot, TokenKind::Ident(text)) => text.clone(),
                        (TokenKind::Dot, TokenKind::Int(value)) => value.to_string(),
                        _ => break,
                    };
                    self.bump(); // the `.`
                    let segment = self.bump();
                    name.push('.');
                    name.push_str(&part);
                    end = segment.span;
                }
                let span = SourceSpan::new(start.source, start.start, end.end);
                Some(Expr::new(ExprKind::Symbol(name), span))
            }
            TokenKind::Ident(text) => {
                if text == "true" {
                    self.bump();
                    Some(Expr::new(ExprKind::Bool(true), token.span))
                } else if text == "false" {
                    self.bump();
                    Some(Expr::new(ExprKind::Bool(false), token.span))
                } else if matches!(self.peek_kind(1), TokenKind::LParen) {
                    let name_span = token.span;
                    self.bump();
                    self.call(text, name_span)
                } else if matches!(text.as_str(), "end" | "else" | "elsif") {
                    // These words close a block. Treating them as a variable
                    // reference would swallow the `end` of the enclosing
                    // definition, so they are refused here.
                    let span = token.span;
                    let message =
                        format!("expected an expression, found `{text}`; this word closes a block");
                    self.error(span, message)
                } else {
                    // A bare identifier is a reference, never a call.
                    self.bump();
                    Some(Expr::new(ExprKind::Var(text), token.span))
                }
            }
            TokenKind::LParen => self.paren(),
            TokenKind::LBracket => self.array(),
            TokenKind::LBrace => self.dict(),
            other => {
                let span = token.span;
                let message = format!("expected an expression, found {}", other.describe());
                self.error(span, message)
            }
        }
    }

    fn paren(&mut self) -> Option<Expr> {
        let open = self.bump();
        self.skip_newlines();
        let inner = self.expr()?;
        self.skip_newlines();
        if !self.eat(&TokenKind::RParen) {
            let token = self.current().clone();
            let message = format!(
                "expected `)` to close the parenthesised expression, found {}",
                token.kind.describe()
            );
            return self.error(token.span, message);
        }
        let span = open.span.merge(self.prev_end());
        Some(Expr::new(inner.kind, span))
    }

    fn array(&mut self) -> Option<Expr> {
        let open = self.bump();
        let mut items = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::RBracket) {
                break;
            }
            if self.at_eof() {
                let span = self.span();
                return self.error(span, "expected `]` to close the array");
            }
            items.push(self.expr()?);
            self.skip_newlines();
            if self.eat(&TokenKind::Comma) {
                continue;
            }
            if self.eat(&TokenKind::RBracket) {
                break;
            }
            let token = self.current().clone();
            let message = format!(
                "expected `,` or `]` in the array, found {}",
                token.kind.describe()
            );
            return self.error(token.span, message);
        }
        let span = open.span.merge(self.prev_end());
        Some(Expr::new(ExprKind::Array(items), span))
    }

    fn dict(&mut self) -> Option<Expr> {
        let open = self.bump();
        let mut entries = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::RBrace) {
                break;
            }
            if self.at_eof() {
                let span = self.span();
                return self.error(span, "expected `}` to close the dictionary");
            }
            let (key, key_span) = match self.kind().clone() {
                TokenKind::Ident(text) => {
                    let token = self.bump();
                    (text, token.span)
                }
                other => {
                    let span = self.span();
                    let message = format!(
                        "expected a dictionary key (a bare identifier such as `input`), found {}",
                        other.describe()
                    );
                    return self.error(span, message);
                }
            };
            if !self.eat(&TokenKind::Colon) {
                let token = self.current().clone();
                let message = format!(
                    "expected `:` after the dictionary key `{key}`, found {}",
                    token.kind.describe()
                );
                return self.error(token.span, message);
            }
            let value = self.expr()?;
            entries.push(DictEntry {
                key,
                key_span,
                value,
            });
            self.skip_newlines();
            if self.eat(&TokenKind::Comma) {
                continue;
            }
            if self.eat(&TokenKind::RBrace) {
                break;
            }
            let token = self.current().clone();
            let message = format!(
                "expected `,` or `}}` in the dictionary, found {}",
                token.kind.describe()
            );
            return self.error(token.span, message);
        }
        let span = open.span.merge(self.prev_end());
        Some(Expr::new(ExprKind::Dict(entries), span))
    }

    fn call(&mut self, name: String, name_span: SourceSpan) -> Option<Expr> {
        let open = self.bump(); // the `(`
        let mut positional = Vec::new();
        let mut named: Vec<Arg> = Vec::new();

        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::RParen) {
                break;
            }
            if self.at_eof() {
                let message = format!("expected `)` to close the call to `{name}`");
                return self.error(open.span, message);
            }

            let named_arg_here = matches!(self.kind(), TokenKind::Ident(_))
                && matches!(self.peek_kind(1), TokenKind::Colon);

            if named_arg_here {
                let arg = self.arg()?;
                named.push(arg);
            } else {
                if !named.is_empty() {
                    let token = self.current().clone();
                    let message = format!(
                        "expected a named argument `name: value` here: positional arguments of \
                         `{name}(...)` must come before the named ones, found {}",
                        token.kind.describe()
                    );
                    return self.error(token.span, message);
                }
                positional.push(self.expr()?);
            }

            self.skip_newlines();
            if self.eat(&TokenKind::Comma) {
                continue;
            }
            if self.eat(&TokenKind::RParen) {
                break;
            }
            let token = self.current().clone();
            let message = format!(
                "expected `,` or `)` after an argument of `{name}`, found {}",
                token.kind.describe()
            );
            return self.error(token.span, message);
        }

        let span = name_span.merge(self.prev_end());
        Some(Expr::new(
            ExprKind::Call(Call {
                name,
                name_span,
                positional,
                named,
                span,
            }),
            span,
        ))
    }
}

/// The binary operator a token spells, with its precedence level.
fn binary_op(kind: &TokenKind) -> Option<(BinaryOp, u8)> {
    Some(match kind {
        TokenKind::PipePipe => (BinaryOp::Or, 1),
        TokenKind::AmpAmp => (BinaryOp::And, 2),
        TokenKind::EqEq => (BinaryOp::Eq, 3),
        TokenKind::BangEq => (BinaryOp::Ne, 3),
        TokenKind::Lt => (BinaryOp::Lt, 4),
        TokenKind::Le => (BinaryOp::Le, 4),
        TokenKind::Gt => (BinaryOp::Gt, 4),
        TokenKind::Ge => (BinaryOp::Ge, 4),
        TokenKind::Plus => (BinaryOp::Add, 5),
        TokenKind::Minus => (BinaryOp::Sub, 5),
        TokenKind::Star => (BinaryOp::Mul, 6),
        TokenKind::Slash => (BinaryOp::Div, 6),
        _ => return None,
    })
}

/// How an expression is described in a `found ...` diagnostic.
fn describe_expr(expr: &Expr) -> &'static str {
    match &expr.kind {
        ExprKind::Int(_) => "an integer",
        ExprKind::Float(_) => "a number",
        ExprKind::Quantity(_) => "a quantity",
        ExprKind::Bool(_) => "a boolean",
        ExprKind::Str(_) => "a string",
        ExprKind::Symbol(_) => "a symbol",
        ExprKind::Array(_) => "an array",
        ExprKind::Dict(_) => "a dictionary",
        ExprKind::Var(_) => "a name",
        ExprKind::Unary { .. } | ExprKind::Binary { .. } => "an expression",
        ExprKind::Call(_) => "a call",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use circuit_core::span::{SourceId, SourceMap};
    use circuit_core::units::{CAPACITANCE, RESISTANCE, TIME};

    /// The first full program from the language specification: an RC filter
    /// and the experiment that drives it.
    const RC_FILTER: &str = r#"circuit :rc_filter do
  param :r, default: 1.kohm
  param :c, default: 100.nF

  node :vin, :vout

  voltage_source :input, p: :vin, n: :gnd,
    dc: 0.V,
    ac: 1.V,
    waveform: pulse(low: 0.V, high: 1.V,
                    delay: 1.us, rise: 10.ns,
                    fall: 10.ns, width: 5.us,
                    period: 10.us)

  resistor :r1, p: :vin, n: :vout, value: r
  capacitor :c1, p: :vout, n: :gnd, value: c
end

experiment :response, circuit: :rc_filter do
  op
  ac from: 10.Hz, to: 10.MHz, points_per_decade: 50
  tran stop: 30.us, max_step: 50.ns

  save v(:vin), v(:vout), i(:input)
end
"#;

    /// The hierarchical example: a subcircuit with ports plus a circuit that
    /// instantiates it twice with `ports:`/`params:` dictionaries.
    const TWO_STAGE: &str = r#"subcircuit :lowpass, ports: [:input, :output, :ground] do
  param :r, default: 1.kohm
  param :c, default: 100.nF

  resistor :r1, p: :input, n: :output, value: r
  capacitor :c1, p: :output, n: :ground, value: c
end

circuit :two_stage do
  node :vin, :mid, :out
  voltage_source :src, p: :vin, n: :gnd, dc: 1.V

  instance :stage1, of: :lowpass,
    ports: { input: :vin, output: :mid, ground: :gnd },
    params: { r: 2.kohm, c: 47.nF }

  instance :stage2, of: :lowpass,
    ports: { input: :mid, output: :out, ground: :gnd }
end
"#;

    fn parse_source(src: &str) -> Program {
        let tokens = lex(SourceId(0), src)
            .unwrap_or_else(|d| panic!("lexing should succeed:\n{}", d.render_plain()));
        parse(&tokens).unwrap_or_else(|d| panic!("parsing should succeed:\n{}", d.render_plain()))
    }

    fn parse_errors(src: &str) -> Diagnostics {
        let tokens = lex(SourceId(0), src)
            .unwrap_or_else(|d| panic!("lexing should succeed:\n{}", d.render_plain()));
        match parse(&tokens) {
            Ok(_) => panic!("expected parse errors for:\n{src}"),
            Err(diagnostics) => diagnostics,
        }
    }

    fn messages(diagnostics: &Diagnostics) -> Vec<String> {
        diagnostics.iter().map(|d| d.message.clone()).collect()
    }

    /// Parse, require an error whose message contains `needle`, and check it is
    /// a syntax error.
    fn expect_error(src: &str, needle: &str) -> Diagnostics {
        let diagnostics = parse_errors(src);
        let found = diagnostics
            .iter()
            .find(|d| d.message.contains(needle) && d.code == Code::Syntax);
        assert!(
            found.is_some(),
            "wanted `{needle}` among {:?}",
            messages(&diagnostics)
        );
        diagnostics
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-12 * b.abs().max(1.0)
    }

    /// The `default:`/`value:` expression of a device argument.
    fn arg<'e>(stmt: &'e Stmt, name: &str) -> &'e Expr {
        match stmt {
            Stmt::Device(device) => device
                .args
                .iter()
                .find(|a| a.name == name)
                .map(|a| &a.value)
                .unwrap_or_else(|| panic!("no `{name}:` argument")),
            other => panic!("expected a device, got {other:?}"),
        }
    }

    fn device(stmt: &Stmt, kind: DeviceStmtKind) -> &DeviceStmt {
        match stmt {
            Stmt::Device(device) if device.kind == kind => device,
            other => panic!("expected a {kind:?}, got {other:?}"),
        }
    }

    fn as_binary(expr: &Expr) -> (&Expr, BinaryOp, &Expr) {
        match &expr.kind {
            ExprKind::Binary { op, lhs, rhs } => (lhs, *op, rhs),
            other => panic!("expected a binary expression, got {other:?}"),
        }
    }

    fn as_int(expr: &Expr) -> i64 {
        match expr.kind {
            ExprKind::Int(value) => value,
            ref other => panic!("expected an integer, got {other:?}"),
        }
    }

    fn as_quantity(expr: &Expr) -> &crate::token::QuantityLiteral {
        match &expr.kind {
            ExprKind::Quantity(literal) => literal,
            other => panic!("expected a quantity, got {other:?}"),
        }
    }

    // ---- the two example programs ---------------------------------------

    #[test]
    fn rc_filter_example_parses_into_the_expected_shape() {
        let mut sources = SourceMap::new();
        let id = sources.add("rc_filter.cdsl", RC_FILTER);
        let tokens = lex(id, RC_FILTER).expect("lexes");
        let program = parse(&tokens).expect("parses");

        assert_eq!(program.circuits.len(), 1);
        assert_eq!(program.experiments.len(), 1);

        let circuit = program.circuit("rc_filter").expect("the circuit");
        assert!(!circuit.is_subcircuit);
        assert!(circuit.ports.is_empty());
        assert_eq!(
            sources.snippet(circuit.name_span),
            ":rc_filter",
            "a symbol token spans its leading colon"
        );
        let definition = sources.snippet(circuit.span);
        assert!(
            definition.starts_with("circuit :rc_filter do"),
            "{definition}"
        );
        assert!(definition.ends_with("end"), "{definition}");
        assert!(
            !definition.contains("experiment"),
            "the circuit definition stops at its own `end`"
        );
        assert_eq!(circuit.body.len(), 6, "body: {:?}", circuit.body);

        // param :r, default: 1.kohm
        let Stmt::Param(r) = &circuit.body[0] else {
            panic!("expected a param, got {:?}", circuit.body[0])
        };
        assert_eq!(r.name.name, "r");
        let default = r.default.as_ref().expect("default");
        assert!(close(as_quantity(default).value, 1000.0));
        assert_eq!(as_quantity(default).dimension, RESISTANCE);
        assert_eq!(as_quantity(default).text, "1.kohm");
        assert_eq!(sources.snippet(default.span), "1.kohm");

        let Stmt::Param(c) = &circuit.body[1] else {
            panic!("expected a param")
        };
        assert_eq!(c.name.name, "c");
        assert!(close(
            as_quantity(c.default.as_ref().unwrap()).value,
            100e-9
        ));

        // node :vin, :vout
        let Stmt::Node(node) = &circuit.body[2] else {
            panic!("expected a node statement")
        };
        let names: Vec<&str> = node.names.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["vin", "vout"]);
        assert_eq!(sources.snippet(node.span), "node :vin, :vout");

        // voltage_source with a multi-line argument list and a waveform call.
        let source = device(&circuit.body[3], DeviceStmtKind::VoltageSource);
        assert_eq!(source.name.name, "input");
        let labels: Vec<&str> = source.args.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(labels, ["p", "n", "dc", "ac", "waveform"]);
        assert_eq!(arg(&circuit.body[3], "p").as_symbol(), Some("vin"));
        assert!(close(as_quantity(arg(&circuit.body[3], "ac")).value, 1.0));

        let ExprKind::Call(waveform) = &arg(&circuit.body[3], "waveform").kind else {
            panic!("expected a call")
        };
        assert_eq!(waveform.name, "pulse");
        assert!(waveform.positional.is_empty());
        let waveform_labels: Vec<&str> = waveform.named.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            waveform_labels,
            ["low", "high", "delay", "rise", "fall", "width", "period"]
        );
        assert!(close(as_quantity(&waveform.named[6].value).value, 10e-6));
        assert_eq!(as_quantity(&waveform.named[5].value).dimension, TIME);

        // resistor/capacitor values reference the declared parameters.
        let Stmt::Device(r1) = &circuit.body[4] else {
            panic!("expected a device")
        };
        assert_eq!(r1.name.name, "r1");
        let ExprKind::Var(name) = &arg(&circuit.body[4], "value").kind else {
            panic!("expected a variable reference")
        };
        assert_eq!(name, "r");

        // experiment :response, circuit: :rc_filter
        let experiment = program.experiment("response").expect("the experiment");
        assert_eq!(experiment.circuit.name, "rc_filter");
        assert_eq!(experiment.body.len(), 4, "body: {:?}", experiment.body);
        assert!(matches!(experiment.body[0], ExpStmt::Op { .. }));

        let ExpStmt::Ac(ac) = &experiment.body[1] else {
            panic!("expected an ac analysis")
        };
        assert!(close(
            as_quantity(&ac.arg("from").unwrap().value).value,
            10.0
        ));
        assert!(close(as_quantity(&ac.arg("to").unwrap().value).value, 10e6));
        assert_eq!(
            ac.arg("to").unwrap().value.span,
            ac.arg("to").unwrap().value.span
        );
        assert_eq!(as_int(&ac.arg("points_per_decade").unwrap().value), 50);

        let ExpStmt::Tran(tran) = &experiment.body[2] else {
            panic!("expected a tran analysis")
        };
        assert_eq!(arg_names_of(&tran.args), ["stop", "max_step"]);
        assert!(close(
            as_quantity(&tran.arg("stop").unwrap().value).value,
            30e-6
        ));
        assert!(close(
            as_quantity(&tran.arg("max_step").unwrap().value).value,
            50e-9
        ));

        let ExpStmt::Save { probes, .. } = &experiment.body[3] else {
            panic!("expected a save statement")
        };
        assert_eq!(probes.len(), 3);
        for probe in probes {
            assert!(matches!(probe.kind, ExprKind::Call(_)));
        }
        assert_eq!(sources.snippet(probes[0].span), "v(:vin)");
        assert_eq!(sources.snippet(probes[2].span), "i(:input)");
    }

    fn arg_names_of(args: &[Arg]) -> Vec<&str> {
        args.iter().map(|a| a.name.as_str()).collect()
    }

    #[test]
    fn two_stage_example_parses_subcircuits_and_instances() {
        let mut sources = SourceMap::new();
        let id = sources.add("two_stage.cdsl", TWO_STAGE);
        let tokens = lex(id, TWO_STAGE).expect("lexes");
        let program = parse(&tokens).expect("parses");

        assert_eq!(program.circuits.len(), 2);
        assert!(program.experiments.is_empty());

        let lowpass = program.circuit("lowpass").expect("the subcircuit");
        assert!(lowpass.is_subcircuit);
        let ports: Vec<&str> = lowpass.ports.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(ports, ["input", "output", "ground"]);
        assert_eq!(sources.snippet(lowpass.ports[0].span), ":input");
        assert_eq!(sources.snippet(lowpass.span), LOWPASS_TEXT.trim());

        let two_stage = program.circuit("two_stage").expect("the circuit");
        assert!(!two_stage.is_subcircuit);
        assert_eq!(two_stage.body.len(), 4);

        let Stmt::Node(nodes) = &two_stage.body[0] else {
            panic!("expected a node statement")
        };
        let names: Vec<&str> = nodes.names.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["vin", "mid", "out"]);

        let Stmt::Instance(stage1) = &two_stage.body[2] else {
            panic!("expected an instance, got {:?}", two_stage.body[2])
        };
        assert_eq!(stage1.name.name, "stage1");
        assert_eq!(stage1.of.name, "lowpass");
        assert_eq!(sources.snippet(stage1.of.span), ":lowpass");

        let port_names: Vec<&str> = stage1.ports.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(port_names, ["input", "output", "ground"]);
        assert_eq!(stage1.ports[1].value.as_symbol(), Some("mid"));
        assert_eq!(sources.snippet(stage1.ports[1].value.span), ":mid");
        assert_eq!(sources.snippet(stage1.ports[1].key_span), "output");

        let param_names: Vec<&str> = stage1.params.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(param_names, ["r", "c"]);
        assert!(close(as_quantity(&stage1.params[0].value).value, 2000.0));
        assert_eq!(as_quantity(&stage1.params[0].value).dimension, RESISTANCE);
        assert!(close(as_quantity(&stage1.params[1].value).value, 47e-9));
        assert_eq!(as_quantity(&stage1.params[1].value).dimension, CAPACITANCE);

        let Stmt::Instance(stage2) = &two_stage.body[3] else {
            panic!("expected an instance")
        };
        assert_eq!(stage2.name.name, "stage2");
        assert_eq!(stage2.ports.len(), 3);
        assert!(stage2.params.is_empty());
        assert!(sources.snippet(stage2.span).starts_with("instance :stage2"));
    }

    const LOWPASS_TEXT: &str = r#"subcircuit :lowpass, ports: [:input, :output, :ground] do
  param :r, default: 1.kohm
  param :c, default: 100.nF

  resistor :r1, p: :input, n: :output, value: r
  capacitor :c1, p: :output, n: :ground, value: c
end"#;

    // ---- expressions -----------------------------------------------------

    #[test]
    fn multiplication_binds_tighter_than_addition() {
        let program = parse_source("circuit :x do\n  resistor :r1, value: 1 + 2 * 3\nend\n");
        let circuit = program.circuit("x").unwrap();
        let value = arg(&circuit.body[0], "value");
        let (lhs, op, rhs) = as_binary(value);
        assert_eq!(op, BinaryOp::Add);
        assert_eq!(as_int(lhs), 1);
        let (lhs, op, rhs) = as_binary(rhs);
        assert_eq!(op, BinaryOp::Mul);
        assert_eq!(as_int(lhs), 2);
        assert_eq!(as_int(rhs), 3);
    }

    #[test]
    fn operators_are_left_associative_and_ordered_by_level() {
        let program = parse_source("circuit :x do\n  resistor :r1, value: 1 - 2 - 3\nend\n");
        let value = arg(&program.circuit("x").unwrap().body[0], "value");
        let (lhs, op, rhs) = as_binary(value);
        assert_eq!((op, as_int(rhs)), (BinaryOp::Sub, 3));
        let (lhs, op, rhs) = as_binary(lhs);
        assert_eq!((op, as_int(lhs), as_int(rhs)), (BinaryOp::Sub, 1, 2));

        // `==` is looser than `<`, `&&` looser than `==`, `||` loosest of all.
        let program = parse_source(
            "circuit :x do\n  resistor :r1, value: 1 + 2 < 3 * 4 == true || false && true\nend\n",
        );
        let value = arg(&program.circuit("x").unwrap().body[0], "value");
        let (lhs, op, rhs) = as_binary(value);
        assert_eq!(op, BinaryOp::Or);
        assert!(matches!(
            rhs.kind,
            ExprKind::Binary {
                op: BinaryOp::And,
                ..
            }
        ));
        let (lhs, op, rhs) = as_binary(lhs);
        assert_eq!(op, BinaryOp::Eq);
        assert!(matches!(rhs.kind, ExprKind::Bool(true)));
        let (lhs, op, rhs) = as_binary(lhs);
        assert_eq!(op, BinaryOp::Lt);
        let (_, op, _) = as_binary(lhs);
        assert_eq!(op, BinaryOp::Add);
        let (_, op, _) = as_binary(rhs);
        assert_eq!(op, BinaryOp::Mul);
    }

    #[test]
    fn unary_operators_nest_and_bind_tighter_than_multiplication() {
        let program = parse_source("circuit :x do\n  resistor :r1, value: -2 * 3\nend\n");
        let value = arg(&program.circuit("x").unwrap().body[0], "value");
        let (lhs, op, rhs) = as_binary(value);
        assert_eq!((op, as_int(rhs)), (BinaryOp::Mul, 3));
        match &lhs.kind {
            ExprKind::Unary { op, rhs } => {
                assert_eq!(*op, UnaryOp::Neg);
                assert_eq!(as_int(rhs), 2);
            }
            other => panic!("expected a unary minus, got {other:?}"),
        }

        let program = parse_source("circuit :x do\n  resistor :r1, value: - -3 + !true\nend\n");
        let value = arg(&program.circuit("x").unwrap().body[0], "value");
        let (lhs, op, rhs) = as_binary(value);
        assert_eq!(op, BinaryOp::Add);
        match (&lhs.kind, &rhs.kind) {
            (
                ExprKind::Unary {
                    op: UnaryOp::Neg,
                    rhs: inner,
                },
                ExprKind::Unary {
                    op: UnaryOp::Not,
                    rhs: flag,
                },
            ) => {
                assert!(matches!(
                    inner.kind,
                    ExprKind::Unary {
                        op: UnaryOp::Neg,
                        ..
                    }
                ));
                assert!(matches!(flag.kind, ExprKind::Bool(true)));
            }
            other => panic!("unexpected shapes {other:?}"),
        }
    }

    #[test]
    fn every_expression_carries_a_span_covering_its_text() {
        let src = "circuit :x do\n  resistor :r1, value: 1.kohm + 2 * r\nend\n";
        let mut sources = SourceMap::new();
        let id = sources.add("t.cdsl", src);
        let tokens = lex(id, src).expect("lexes");
        let program = parse(&tokens).expect("parses");
        let value = arg(&program.circuit("x").unwrap().body[0], "value");
        assert_eq!(sources.snippet(value.span), "1.kohm + 2 * r");
        let (lhs, _, rhs) = as_binary(value);
        assert_eq!(sources.snippet(lhs.span), "1.kohm");
        assert_eq!(sources.snippet(rhs.span), "2 * r");
        assert!(!sources.snippet(value.span).is_empty());
        assert!(!value.span.is_synthetic());
    }

    #[test]
    fn parenthesised_expressions_keep_the_inner_kind_but_widen_the_span() {
        let src = "circuit :x do\n  resistor :r1, value: (1 + 2) * 3\nend\n";
        let mut sources = SourceMap::new();
        let id = sources.add("t.cdsl", src);
        let tokens = lex(id, src).expect("lexes");
        let program = parse(&tokens).expect("parses");
        let value = arg(&program.circuit("x").unwrap().body[0], "value");
        let (lhs, op, _) = as_binary(value);
        assert_eq!(op, BinaryOp::Mul);
        assert!(matches!(
            lhs.kind,
            ExprKind::Binary {
                op: BinaryOp::Add,
                ..
            }
        ));
        assert_eq!(sources.snippet(lhs.span), "(1 + 2)");
    }

    #[test]
    fn atoms_cover_literals_symbols_arrays_dicts_and_calls() {
        let src = "circuit :x do\n  model :m, a: [1, 2.5, 1.kohm], b: { input: :vin, n: 2 }, c: \"text\", d: :sym, e: f(1, :v, g(2)), bools: true\nend\n";
        let program = parse_source(src);
        let circuit = program.circuit("x").unwrap();
        let Stmt::Model(model) = &circuit.body[0] else {
            panic!("expected a model")
        };
        let find = |name: &str| {
            model
                .args
                .iter()
                .find(|a| a.name == name)
                .unwrap_or_else(|| panic!("no `{name}:`"))
        };

        let ExprKind::Array(items) = &find("a").value.kind else {
            panic!("expected an array")
        };
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0].kind, ExprKind::Int(1)));
        assert!(matches!(items[1].kind, ExprKind::Float(_)));
        assert!(matches!(items[2].kind, ExprKind::Quantity(_)));

        let ExprKind::Dict(entries) = &find("b").value.kind else {
            panic!("expected a dictionary")
        };
        let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["input", "n"]);
        assert_eq!(entries[0].value.as_symbol(), Some("vin"));
        assert!(matches!(entries[1].value.kind, ExprKind::Int(2)));

        assert!(matches!(&find("c").value.kind, ExprKind::Str(s) if s == "text"));
        assert!(matches!(&find("d").value.kind, ExprKind::Symbol(s) if s == "sym"));
        assert!(matches!(&find("bools").value.kind, ExprKind::Bool(true)));

        let ExprKind::Call(call) = &find("e").value.kind else {
            panic!("expected a call")
        };
        assert_eq!(call.name, "f");
        assert_eq!(call.positional.len(), 3);
        assert!(call.named.is_empty());
        assert_eq!(call.positional[1].as_symbol(), Some("v"));
        let ExprKind::Call(inner) = &call.positional[2].kind else {
            panic!("expected a nested call")
        };
        assert_eq!(inner.name, "g");
        assert_eq!(inner.positional.len(), 1);
    }

    #[test]
    fn a_bare_identifier_is_a_variable_not_a_call() {
        let program = parse_source("circuit :x do\n  resistor :r1, value: r\nend\n");
        let value = arg(&program.circuit("x").unwrap().body[0], "value");
        assert!(matches!(&value.kind, ExprKind::Var(name) if name == "r"));

        // with parentheses it is a call
        let program = parse_source("circuit :x do\n  resistor :r1, value: r()\nend\n");
        let value = arg(&program.circuit("x").unwrap().body[0], "value");
        assert!(matches!(&value.kind, ExprKind::Call(call) if call.name == "r"));
    }

    #[test]
    fn calls_take_positional_then_named_arguments() {
        let program = parse_source(
            "circuit :x do\n  voltage_source :v1, waveform: pulse(low: 0.V, high: 1.V)\nend\n",
        );
        let value = arg(&program.circuit("x").unwrap().body[0], "waveform");
        let ExprKind::Call(call) = &value.kind else {
            panic!("expected a call")
        };
        assert!(call.positional.is_empty());
        let names: Vec<&str> = call.named.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["low", "high"]);
        assert_eq!(sources_of_call(call), "pulse(low: 0.V, high: 1.V)");

        // `v(:a, :b)` is two positional symbol arguments.
        let program = parse_source(
            "experiment :e, circuit: :x do\n  save v(:a, :b)\nend\ncircuit :x do\nend\n",
        );
        let experiment = program.experiment("e").unwrap();
        let ExpStmt::Save { probes, .. } = &experiment.body[0] else {
            panic!("expected save")
        };
        let ExprKind::Call(call) = &probes[0].kind else {
            panic!("expected a call")
        };
        assert_eq!(call.name, "v");
        assert_eq!(call.positional.len(), 2);
        assert_eq!(call.positional[0].as_symbol(), Some("a"));
        assert_eq!(call.positional[1].as_symbol(), Some("b"));
    }

    /// The span of a call covers its whole text; checked with a fresh map
    /// because the call above has no source map handy.
    fn sources_of_call(call: &Call) -> String {
        let src =
            "circuit :x do\n  voltage_source :v1, waveform: pulse(low: 0.V, high: 1.V)\nend\n";
        let mut sources = SourceMap::new();
        let id = sources.add("t.cdsl", src);
        let tokens = lex(id, src).expect("lexes");
        let program = parse(&tokens).expect("parses");
        let value = arg(&program.circuit("x").unwrap().body[0], "waveform");
        let ExprKind::Call(parsed) = &value.kind else {
            panic!("expected a call")
        };
        assert_eq!(parsed.name, call.name);
        assert_eq!(sources.snippet(parsed.span), "pulse(low: 0.V, high: 1.V)");
        sources.snippet(parsed.span).to_string()
    }

    #[test]
    fn positional_argument_after_a_named_one_is_rejected() {
        let diagnostics = expect_error(
            "circuit :x do\n  resistor :r1, value: f(a: 1, 2)\nend\n",
            "must come before",
        );
        assert_eq!(diagnostics.len(), 1);
    }

    // ---- statements ------------------------------------------------------

    #[test]
    fn for_over_a_range_and_over_a_list() {
        let program = parse_source("circuit :x do\n  for i in 1..4 do\n    node :n\n  end\nend\n");
        let circuit = program.circuit("x").unwrap();
        let Stmt::For(loop_stmt) = &circuit.body[0] else {
            panic!("expected a for statement")
        };
        assert_eq!(loop_stmt.var.name, "i");
        match &loop_stmt.iter {
            ForIter::Range { start, end } => {
                assert_eq!(as_int(start), 1);
                assert_eq!(as_int(end), 4);
                // Exactly `..` sits between the two bounds, with no token lost
                // to the quantity parser.
                assert_eq!(end.span.start - start.span.end, 2);
            }
            other => panic!("expected a range, got {other:?}"),
        }
        assert_eq!(loop_stmt.body.len(), 1);
        assert_eq!(circuit.body.len(), 1);

        let program =
            parse_source("circuit :x do\n  for i in [1, 2, 3] do\n    node :n\n  end\nend\n");
        let Stmt::For(loop_stmt) = &program.circuit("x").unwrap().body[0] else {
            panic!("expected a for statement")
        };
        let ForIter::List(list) = &loop_stmt.iter else {
            panic!("expected a list, got {:?}", loop_stmt.iter)
        };
        let ExprKind::Array(items) = &list.kind else {
            panic!("expected an array")
        };
        assert_eq!(items.len(), 3);

        // A range over parameters keeps them as variables.
        let program = parse_source("circuit :x do\n  for i in 0..n do\n    node :n\n  end\nend\n");
        let Stmt::For(loop_stmt) = &program.circuit("x").unwrap().body[0] else {
            panic!("expected a for statement")
        };
        match &loop_stmt.iter {
            ForIter::Range { start, end } => {
                assert!(matches!(start.kind, ExprKind::Int(0)));
                assert!(matches!(&end.kind, ExprKind::Var(name) if name == "n"));
            }
            other => panic!("expected a range, got {other:?}"),
        }
    }

    #[test]
    fn if_elsif_else_arms_are_collected_in_order() {
        let src = "circuit :x do\n  if a do\n    node :p\n  elsif b do\n    node :q\n  else\n    node :r\n  end\nend\n";
        let program = parse_source(src);
        let Stmt::If(if_stmt) = &program.circuit("x").unwrap().body[0] else {
            panic!("expected an if statement")
        };
        assert_eq!(if_stmt.arms.len(), 2);
        assert!(matches!(if_stmt.arms[0].0.kind, ExprKind::Var(_)));
        assert_eq!(if_stmt.arms[0].1.len(), 1);
        assert_eq!(if_stmt.arms[1].1.len(), 1);
        assert_eq!(if_stmt.else_body.as_ref().map(Vec::len), Some(1));
        let Stmt::Node(elsif_node) = &if_stmt.arms[1].1[0] else {
            panic!("expected a node")
        };
        assert_eq!(elsif_node.names[0].name, "q");
        let else_node = &if_stmt.else_body.as_ref().unwrap()[0];
        let Stmt::Node(else_node) = else_node else {
            panic!("expected a node")
        };
        assert_eq!(else_node.names[0].name, "r");
    }

    #[test]
    fn nested_blocks_belong_to_their_own_statement() {
        let src = "circuit :x do\n  for i in 1..2 do\n    if i == 1 do\n      node :a\n    else\n      node :b\n    end\n    node :c\n  end\n  node :after\nend\n";
        let program = parse_source(src);
        let circuit = program.circuit("x").unwrap();
        assert_eq!(circuit.body.len(), 2);
        let Stmt::For(loop_stmt) = &circuit.body[0] else {
            panic!("expected a for statement")
        };
        assert_eq!(loop_stmt.body.len(), 2);
        assert!(matches!(loop_stmt.body[0], Stmt::If(_)));
        assert!(matches!(loop_stmt.body[1], Stmt::Node(_)));
        assert!(matches!(circuit.body[1], Stmt::Node(_)));
    }

    #[test]
    fn if_condition_can_use_operators() {
        let src = "circuit :x do\n  if r > 1.kohm && c != 0.F do\n    node :a\n  end\nend\n";
        let program = parse_source(src);
        let Stmt::If(if_stmt) = &program.circuit("x").unwrap().body[0] else {
            panic!("expected an if statement")
        };
        let (lhs, op, rhs) = as_binary(&if_stmt.arms[0].0);
        assert_eq!(op, BinaryOp::And);
        assert_eq!(as_binary(lhs).1, BinaryOp::Gt);
        assert_eq!(as_binary(rhs).1, BinaryOp::Ne);
        assert!(close(as_quantity(as_binary(lhs).2).value, 1000.0));
    }

    #[test]
    fn every_device_keyword_from_the_spec_parses() {
        let src = "circuit :x do\n  inductor :l1, p: :a, n: :b, value: 1.mH\n  \
                   current_source :i1, p: :a, n: :gnd, dc: 1.mA\n  \
                   diode :d1, p: :a, n: :b, model: :dm\n  \
                   model :dm, type: :diode, is: 1e-14, n: 2\nend\n";
        let program = parse_source(src);
        let circuit = program.circuit("x").unwrap();
        assert_eq!(circuit.body.len(), 4);

        let kinds: Vec<DeviceStmtKind> = circuit
            .body
            .iter()
            .filter_map(|stmt| match stmt {
                Stmt::Device(device) => Some(device.kind),
                Stmt::Model(_) => None,
                other => panic!("unexpected statement {other:?}"),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                DeviceStmtKind::Inductor,
                DeviceStmtKind::CurrentSource,
                DeviceStmtKind::Diode
            ]
        );

        let inductor = device(&circuit.body[0], DeviceStmtKind::Inductor);
        assert_eq!(inductor.name.name, "l1");
        assert!(close(as_quantity(&inductor.args[2].value).value, 1e-3));

        let diode = device(&circuit.body[2], DeviceStmtKind::Diode);
        assert_eq!(diode.name.name, "d1");
        assert_eq!(arg(&circuit.body[2], "model").as_symbol(), Some("dm"));

        let Stmt::Model(model) = &circuit.body[3] else {
            panic!("expected a model")
        };
        assert_eq!(model.name.name, "dm");
        let labels: Vec<&str> = arg_names_of(&model.args);
        assert_eq!(labels, ["type", "is", "n"]);
        assert_eq!(model.args[0].value.as_symbol(), Some("diode"));
        assert!(matches!(model.args[1].value.kind, ExprKind::Float(_)));
        assert_eq!(as_int(&model.args[2].value), 2);
    }

    #[test]
    fn experiment_statements_cover_every_documented_form() {
        let src = r#"circuit :x do
end

experiment :e, circuit: :x do
  op
  dc source: :v1, from: 0.V, to: 5.V, step: 1.V
  dc param: :r, from: 1.kohm, to: 5.kohm, step: 1.kohm
  ac from: 10.Hz, to: 10.MHz, points_per_decade: 50
  ac from: 10.Hz, to: 10.MHz, points: 100
  tran stop: 30.us, max_step: 50.ns
  tran start: 10.us, stop: 30.us, max_step: 50.ns
  param :r, value: 2.kohm
  measure :vmax, max: v(:out)
  measure :vavg, avg: v(:out)
  save v(:vin), v(:vout), v(:a, :b), i(:input)
end
"#;
        let program = parse_source(src);
        let experiment = program.experiment("e").expect("the experiment");
        assert_eq!(experiment.body.len(), 11);

        let keywords: Vec<&str> = experiment.body.iter().map(ExpStmt::keyword).collect();
        assert_eq!(
            keywords,
            [
                "op", "dc", "dc", "ac", "ac", "tran", "tran", "param", "measure", "measure", "save"
            ]
        );

        let ExpStmt::Dc(dc) = &experiment.body[1] else {
            panic!("expected dc")
        };
        assert_eq!(dc.arg_names(), ["source", "from", "to", "step"]);
        assert_eq!(dc.arg("source").unwrap().value.as_symbol(), Some("v1"));

        let ExpStmt::Ac(ac) = &experiment.body[3] else {
            panic!("expected ac")
        };
        assert_eq!(as_int(&ac.arg("points_per_decade").unwrap().value), 50);
        let ExpStmt::Ac(ac) = &experiment.body[4] else {
            panic!("expected ac")
        };
        assert_eq!(as_int(&ac.arg("points").unwrap().value), 100);

        let ExpStmt::Tran(tran) = &experiment.body[5] else {
            panic!("expected tran")
        };
        assert_eq!(tran.arg_names(), ["stop", "max_step"]);

        let ExpStmt::Param { name, value, .. } = &experiment.body[7] else {
            panic!("expected a param override")
        };
        assert_eq!(name.name, "r");
        assert!(close(as_quantity(value).value, 2000.0));
        assert_eq!(as_quantity(value).dimension, RESISTANCE);

        let ExpStmt::Measure {
            name, kind, target, ..
        } = &experiment.body[8]
        else {
            panic!("expected a measurement")
        };
        assert_eq!(name.name, "vmax");
        assert_eq!(kind, "max");
        assert!(matches!(target.kind, ExprKind::Call(_)));

        let ExpStmt::Save { probes, .. } = &experiment.body[10] else {
            panic!("expected save")
        };
        assert_eq!(probes.len(), 4);
        let ExprKind::Call(second) = &probes[2].kind else {
            panic!("expected a call")
        };
        assert_eq!(second.positional.len(), 2);
    }

    #[test]
    fn save_probe_expressions_keep_their_spans() {
        let src = "circuit :x do\nend\nexperiment :e, circuit: :x do\n  save v(:vin), v(:vout), i(:d)\nend\n";
        let mut sources = SourceMap::new();
        let id = sources.add("t.cdsl", src);
        let tokens = lex(id, src).expect("lexes");
        let program = parse(&tokens).expect("parses");
        let ExpStmt::Save { probes, .. } = &program.experiment("e").unwrap().body[0] else {
            panic!("expected save")
        };
        let text: Vec<&str> = probes.iter().map(|p| sources.snippet(p.span)).collect();
        assert_eq!(text, ["v(:vin)", "v(:vout)", "i(:d)"]);
        let ExprKind::Call(call) = &probes[2].kind else {
            panic!("expected a call")
        };
        assert_eq!(call.name, "i");
        assert_eq!(sources.snippet(call.positional[0].span), ":d");
    }

    #[test]
    fn string_and_symbol_literals_are_different_expressions() {
        let program = parse_source("circuit :x do\n  model :m, a: \"gnd\", b: :gnd\nend\n");
        let Stmt::Model(model) = &program.circuit("x").unwrap().body[0] else {
            panic!("expected a model")
        };
        assert!(matches!(&model.args[0].value.kind, ExprKind::Str(s) if s == "gnd"));
        assert!(matches!(&model.args[1].value.kind, ExprKind::Symbol(s) if s == "gnd"));
        assert_eq!(model.args[0].name, "a");
        assert_eq!(model.args[1].name, "b");
    }

    #[test]
    fn multi_line_argument_lists_and_trailing_commas_are_accepted() {
        // A comma at the end of a line continues the statement (§1.8), so a
        // trailing comma is only followed by the end of the block or by
        // further arguments.
        let src = "circuit :x do\n  node :first\n  resistor :r1,\n    p: :a,\n    n: :b,\n    value: 1.kohm,\nend\n";
        let program = parse_source(src);
        let circuit = program.circuit("x").unwrap();
        assert_eq!(circuit.body.len(), 2);
        let Stmt::Device(resistor) = &circuit.body[1] else {
            panic!("expected a resistor")
        };
        assert_eq!(arg_names_of(&resistor.args), ["p", "n", "value"]);

        // Arrays and dictionaries allow a trailing comma too, and may span
        // lines because line breaks inside brackets are ignored.
        parse_source(
            "circuit :x do\n  model :m,\n    a: [1, 2,],\n    b: {\n      k: 1,\n    }\nend\n",
        );
    }

    // ---- errors ----------------------------------------------------------

    #[test]
    fn missing_end_names_the_construct_and_points_at_its_header() {
        let diagnostics = expect_error("circuit :rc do\n  node :a\n", "expected `end` to close");
        assert_eq!(diagnostics.len(), 1);
        let first = diagnostics.iter().next().unwrap();
        assert_eq!(first.message, "expected `end` to close `circuit :rc`");
        let label = first.primary.as_ref().expect("a primary span");
        assert_eq!(
            (label.span.start, label.span.end),
            (0, 7),
            "points at `circuit`"
        );

        let diagnostics = expect_error(
            "circuit :x do\n  for i in 1..2 do\n    node :a\n  end\n",
            "expected `end` to close `circuit :x`",
        );
        assert_eq!(diagnostics.len(), 1);

        // A `for` whose body runs into end of file names the loop; the
        // enclosing definition is reported as unclosed as well, because the
        // same `end` was missing for both.
        let src = "circuit :x do\n  for i in 1..2 do\n    node :a\n";
        let diagnostics = expect_error(src, "expected `end` to close `for i`");
        let loop_error = diagnostics
            .iter()
            .find(|d| d.message.contains("for i"))
            .expect("the `for` diagnostic");
        let label = loop_error.primary.as_ref().expect("a primary span");
        assert_eq!(
            &src[label.span.start as usize..label.span.end as usize],
            "for",
            "points at the `for` keyword"
        );

        let diagnostics = expect_error(
            "experiment :e, circuit: :x do\n  op\n",
            "expected `end` to close `experiment :e`",
        );
        assert_eq!(diagnostics.len(), 1);

        let diagnostics = expect_error(
            "subcircuit :s, ports: [:a] do\n  if p do\n    node :a\n  end\n",
            "expected `end` to close `subcircuit :s`",
        );
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn missing_colon_reports_the_expected_label_syntax() {
        let src = "circuit :x do\n  resistor :r1, p 5\nend\n";
        let diagnostics = expect_error(src, "expected `:` after `p`, found integer `5`");
        assert_eq!(diagnostics.len(), 1);
        let label = diagnostics.iter().next().unwrap().primary.clone().unwrap();
        assert_eq!(
            &src[label.span.start as usize..label.span.end as usize],
            "p",
            "the label is reported at the argument name"
        );
    }

    #[test]
    fn a_positional_expression_where_a_label_belongs_is_rejected() {
        expect_error(
            "experiment :e, circuit: :x do\n  ac from: 1.Hz, 5\nend\n",
            "expected an argument of the form `name: value`",
        );
    }

    /// Error recovery must always make progress. Every prefix of a valid
    /// program is fed through lexer and parser: whatever the cut-off point,
    /// parsing has to terminate and report rather than loop.
    #[test]
    fn truncated_input_terminates() {
        for source in [RC_FILTER, TWO_STAGE] {
            for end in 0..source.chars().count() {
                let prefix: String = source.chars().take(end).collect();
                if let Ok(tokens) = lex(SourceId(0), &prefix) {
                    let _ = parse(&tokens);
                }
            }
        }
    }

    #[test]
    fn unexpected_tokens_are_reported_with_their_text() {
        let diagnostics = expect_error(
            "experiment :e, circuit: :x do\n  op,\nend\n",
            "unexpected `,`",
        );
        assert_eq!(diagnostics.len(), 1);

        // Junk at the top level.
        let diagnostics = expect_error(
            "node :a\n",
            "expected `circuit`, `subcircuit`, or `experiment`",
        );
        assert_eq!(diagnostics.len(), 1);

        // An unknown word inside a body.
        let diagnostics = expect_error(
            "circuit :x do\n  frobnicate :a\nend\n",
            "unexpected `frobnicate`",
        );
        assert_eq!(diagnostics.len(), 1);

        // An `else` with no `if`.
        expect_error("circuit :x do\n  else\nend\n", "no matching `if`");
    }

    #[test]
    fn several_independent_errors_are_reported_in_one_run() {
        let diagnostics =
            parse_errors("circuit :x do\n  node 5\n  resistor :r1, p 5\n  node :ok\nend\n");
        let messages = messages(&diagnostics);
        assert_eq!(diagnostics.len(), 2, "{messages:?}");
        assert!(messages[0].contains("expected a node name"));
        assert!(messages[1].contains("expected `:` after `p`"));
        // The third statement still parsed cleanly, which is why only two
        // diagnostics were produced.
    }

    #[test]
    fn malformed_headers_are_reported_once_and_recovered() {
        expect_error("circuit do\nend\n", "expected a name after `circuit`");
        expect_error(
            "circuit :x do\nend\ncircuit :y\n",
            "expected `do` after `circuit :y`",
        );
        expect_error("subcircuit :s do\nend\n", "must declare its ports");
        expect_error(
            "subcircuit :s, foo: [:a] do\nend\n",
            "unexpected argument `foo`",
        );
        expect_error(
            "circuit :x do\n  instance :i, ports: { a: :b }\nend\n",
            "expected `of: :subcircuit` in `instance`",
        );
        expect_error(
            "circuit :x do\n  instance :i, of: 5\nend\n",
            "`of:` requires a subcircuit name",
        );
        expect_error(
            "experiment :e do\n  op\nend\n",
            "expected `circuit: :name` in `experiment`",
        );
        expect_error(
            "circuit :x do\n  for 5 in 1..2 do\n    node :a\n  end\nend\n",
            "expected a loop variable after `for`",
        );
        expect_error(
            "circuit :x do\n  for do\n    node :a\n  end\nend\n",
            "expected a loop variable after `for`, found `do`",
        );
        expect_error(
            "circuit :x do\n  for i 1..2 do\n    node :a\n  end\nend\n",
            "expected `in` after `for i`",
        );
        expect_error(
            "experiment :e, circuit: :x do\n  save\nend\n",
            "expected at least one probe after `save`",
        );
        expect_error(
            "experiment :e, circuit: :x do\n  measure :m\nend\n",
            "expected `, <kind>: <probe>` after `measure :name`",
        );
    }

    #[test]
    fn unclosed_expressions_are_reported() {
        expect_error(
            "circuit :x do\n  resistor :r1, value: f(1, 2,\n",
            "expected `)` to close the call to `f`",
        );
        expect_error(
            "circuit :x do\n  resistor :r1, value: (1 + 2\nend\n",
            "expected `)` to close the parenthesised expression",
        );
        expect_error(
            "circuit :x do\n  resistor :r1, value: [1, 2,\n",
            "expected `]` to close the array",
        );
        expect_error(
            "circuit :x do\n  resistor :r1, value: { a: 1,\n",
            "expected `}` to close the dictionary",
        );
        // `value:` is followed by a line break, which is a continuation point,
        // so the next token is the `end` of the circuit: refusing to read it as
        // a variable keeps the block structure intact.
        expect_error(
            "circuit :x do\n  resistor :r1, value:\nend\n",
            "expected an expression, found `end`",
        );
        expect_error(
            "circuit :x do\n  resistor :r1, value: { :a: 1 }\nend\n",
            "expected a dictionary key",
        );
    }

    #[test]
    fn an_empty_token_stream_is_an_empty_program() {
        let tokens = lex(SourceId(0), "").expect("lexes");
        let program = parse(&tokens).expect("parses");
        assert!(program.circuits.is_empty());
        assert!(program.experiments.is_empty());
        assert!(
            parse(&[]).is_ok(),
            "no tokens at all behaves like an empty file"
        );
    }

    #[test]
    fn all_parse_errors_are_syntax_errors() {
        let diagnostics = parse_errors("circuit :x do\n  node 5\n  bogus :a\nend\n");
        assert!(diagnostics.iter().all(|d| d.code == Code::Syntax));
        assert!(diagnostics.has_errors());
    }

    #[test]
    fn a_dotted_symbol_is_one_name_covering_the_whole_path() {
        let src = "circuit :x do\n  node :a\nend\nexperiment :e, circuit: :x do\n  op\n  save v(:stage1.internal), i(:a.b.c)\nend\n";
        let tokens = lex(SourceId(0), src).expect("lexes");
        let program = parse(&tokens).expect("parses");

        let save = program.experiments[0]
            .body
            .iter()
            .find(|s| matches!(s, ExpStmt::Save { .. }))
            .expect("a save statement");
        let ExpStmt::Save { probes, .. } = save else {
            unreachable!()
        };
        // Each entry is a call such as `v(:stage1.internal)`; the path is its
        // argument, and that argument must be one symbol covering the path.
        let names: Vec<&str> = probes
            .iter()
            .map(|e| {
                let ExprKind::Call(call) = &e.kind else {
                    panic!("expected a probe call, found {:?}", e.kind);
                };
                call.positional[0].as_symbol().expect("a symbol")
            })
            .collect();
        assert_eq!(names, ["stage1.internal", "a.b.c"]);

        // The span must cover the whole path, so a diagnostic underlines all
        // of `:stage1.internal` and not just `:stage1`.
        let ExprKind::Call(first) = &probes[0].kind else {
            unreachable!()
        };
        let span = first.positional[0].span;
        assert_eq!(span.end - span.start, ":stage1.internal".len() as u32);
    }

    #[test]
    fn a_dot_that_is_not_a_path_is_still_left_alone() {
        // `1..3` is a range, not the name `1.` followed by `.3`.
        let src = "circuit :x do\n  param :n, default: 2\n  for k in 1..n do\n    node (\"m\" + k)\n  end\nend\n";
        let tokens = lex(SourceId(0), src).expect("lexes");
        assert!(parse(&tokens).is_ok(), "a range must still parse");
    }
}
