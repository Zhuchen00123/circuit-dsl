//! Session state and the commands a session understands.
//!
//! A session is a set of definitions plus a table of variables, and a way to
//! change either. It knows nothing about terminals: the REPL binary feeds it
//! lines and prints what it replies, and the tests drive it directly.
//!
//! # What a session guarantees
//!
//! - **Definition replacement is atomic.** An input that defines something is
//!   checked by compiling the whole resulting state; only if that succeeds is
//!   the state replaced. A failed definition leaves the session exactly as it
//!   was, so the previous definitions and every variable are still usable.
//! - **The committed state always compiles.** Because each input is checked
//!   against the whole state, a redefinition that would break an existing
//!   experiment is refused rather than stored.
//! - **Session variables never reach a circuit.** Elaboration starts from an
//!   empty parameter scope, so a circuit that mentions a session variable
//!   without declaring `param` gets `E_NAME`. The only way a session value
//!   enters a run is an explicit override on `:run`, evaluated once and fixed
//!   for that run.
//! - **Runs use the current definitions.** Every run re-elaborates; nothing is
//!   cached, so editing a definition and running again cannot show stale
//!   results.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use circuit_backend::thevenin::TheveninBackend;
use circuit_core::diagnostic::{Code, Diagnostic, Diagnostics};
use circuit_core::span::SourceId;
use circuit_core::units::Quantity;
use circuit_core::{Limits, SourceMap};
use circuit_dsl::ast::{CircuitDef, ExperimentDef, Program};
use circuit_dsl::complete::{Completeness, assess};
use circuit_dsl::eval::{self, Value, Variables};
use circuit_dsl::parse;
use circuit_dsl::parser::{Input, parse_input};

use crate::execute::{self, Format, RunRequest};

/// What the session did with an input.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// The input is not finished; the prompt should show a continuation.
    Continue { reason: String },
    /// Nothing to report: an empty input or a comment.
    Nothing,
    /// A value, from `name = expr` (which carries the name) or an expression.
    Value {
        name: Option<String>,
        text: String,
        kind: &'static str,
    },
    /// Definitions were stored.
    Defined {
        accepted: Vec<String>,
        replaced: Vec<String>,
    },
    /// Free text: command output, run summaries.
    Message(String),
    /// The session was asked to end.
    Quit,
}

/// The commands a session understands.
///
/// Kept as one list so `feed` (which decides whether a line is a command at
/// all) and `command` (which dispatches it) cannot disagree: a word this list
/// does not contain is a symbol, not a command.
pub const COMMANDS: &[&str] = &[
    ":help", ":load", ":list", ":quit", ":exit", ":reset", ":run",
];

/// Whether a line is a `:command` rather than language input.
fn is_command(line: &str) -> bool {
    let word = line.split_whitespace().next().unwrap_or_default();
    COMMANDS.contains(&word)
}

/// Session options.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub limits: Limits,
}

pub struct Session {
    sources: SourceMap,
    /// Session variables. Separate namespace from the definitions, so a
    /// variable and a circuit may share a name without either hiding the other.
    vars: BTreeMap<String, Quantity>,
    circuits: BTreeMap<String, CircuitDef>,
    experiments: BTreeMap<String, ExperimentDef>,
    backend: TheveninBackend,
    limits: Limits,
    /// Text of the input being accumulated across continuation lines.
    pending: String,
    /// Session line number the pending input started on.
    pending_line: u32,
    /// Next session line number, 1-based, so diagnostics say `<repl:7>`.
    line: u32,
}

impl Default for Session {
    fn default() -> Self {
        Self::new(Options::default())
    }
}

impl Session {
    pub fn new(options: Options) -> Self {
        Self {
            sources: SourceMap::new(),
            vars: BTreeMap::new(),
            circuits: BTreeMap::new(),
            experiments: BTreeMap::new(),
            backend: TheveninBackend::new(),
            limits: options.limits,
            pending: String::new(),
            pending_line: 1,
            line: 1,
        }
    }

    // ---- inspection ------------------------------------------------------

    /// The source map holding every input, so diagnostics render with
    /// locations exactly as they do for a file.
    pub fn sources(&self) -> &SourceMap {
        &self.sources
    }

    pub fn variables(&self) -> impl Iterator<Item = (&String, &Quantity)> {
        self.vars.iter()
    }

    pub fn circuit_names(&self) -> Vec<&str> {
        self.circuits.keys().map(String::as_str).collect()
    }

    pub fn subcircuit_names(&self) -> Vec<&str> {
        self.circuits
            .values()
            .filter(|c| c.is_subcircuit)
            .map(|c| c.name.as_str())
            .collect()
    }

    pub fn experiment_names(&self) -> Vec<&str> {
        self.experiments.keys().map(String::as_str).collect()
    }

    /// Whether an input is waiting to be completed.
    pub fn is_continuing(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Throw away a half-typed input, keeping every definition and variable.
    ///
    /// This is what `Ctrl+C` does: cancelling an input must never cost the
    /// work already done in the session.
    pub fn cancel_pending(&mut self) {
        self.pending.clear();
    }

    /// The current definitions, assembled into one program.
    pub fn program(&self) -> Program {
        Program {
            circuits: self.circuits.values().cloned().collect(),
            experiments: self.experiments.values().cloned().collect(),
        }
    }

    // ---- input -----------------------------------------------------------

    /// Feed one line of input.
    ///
    /// The line is appended to any pending continuation, and the accumulated
    /// buffer is classified: unfinished input asks for more, anything else is
    /// handled and the buffer cleared.
    pub fn feed(&mut self, line: &str) -> Result<Reply, Diagnostics> {
        // Commands are recognised only at the start of an input, never inside
        // a continuation, and only for the fixed list of command words: `:vin`
        // is a symbol, and a symbol is a value like any other.
        if self.pending.is_empty() {
            let trimmed = line.trim();
            if is_command(trimmed) {
                self.line += 1;
                return self.command(trimmed);
            }
            if trimmed.is_empty() {
                self.line += 1;
                return Ok(Reply::Nothing);
            }
        }

        if self.pending.is_empty() {
            self.pending_line = self.line;
        }
        self.pending.push_str(line);
        self.pending.push('\n');
        self.line += 1;

        let name = format!("<repl:{}>", self.pending_line);
        // Registering the buffer before classifying it keeps every span the
        // parser produces valid against this map. A buffer that turns out to
        // be unfinished is registered again (extended) on the next line; the
        // earlier entry is simply never referenced.
        let source = self.sources.add(name, self.pending.clone());

        match assess(source, &self.pending) {
            Completeness::Incomplete { reason, .. } => Ok(Reply::Continue {
                reason: reason.to_string(),
            }),
            Completeness::Invalid(diagnostics) => {
                self.pending.clear();
                Err(diagnostics)
            }
            Completeness::Complete => {
                let text = std::mem::take(&mut self.pending);
                self.accept(&text, source)
            }
        }
    }

    /// Handle a completed input.
    fn accept(&mut self, text: &str, source: SourceId) -> Result<Reply, Diagnostics> {
        let tokens = circuit_dsl::lex(source, text)?;
        let parsed = parse_input(&tokens)?;

        match parsed {
            Input::Empty => Ok(Reply::Nothing),
            Input::Expr(expr) => {
                let v = eval::eval(&expr, &self.var_view())?;
                Ok(Reply::Value {
                    name: None,
                    text: crate::format::value(&v),
                    kind: crate::format::type_name(&v),
                })
            }
            Input::Assign { name, value, .. } => {
                let v = eval::eval(&value, &self.var_view())?;
                let Some(q) = v.as_num() else {
                    return Err(Diagnostics::single(
                        Diagnostic::error(
                            Code::Type,
                            format!("a variable must be a number, found {}", v.type_name()),
                        )
                        .at(value.span)
                        .with_note("e.g. `r = 1.kohm`, or `tau = r * c`"),
                    ));
                };
                if !q.is_finite() {
                    return Err(Diagnostics::single(
                        Diagnostic::error(Code::Value, "a variable must be a finite number")
                            .at(value.span),
                    ));
                }
                self.vars.insert(name.clone(), q);
                Ok(Reply::Value {
                    name: Some(name),
                    text: crate::format::value(&Value::Num(q)),
                    kind: "number",
                })
            }
            Input::Program(program) => self.define(program),
        }
    }

    /// Store the definitions of `program`, replacing same-named ones.
    ///
    /// The whole state is compiled first, so a definition that does not
    /// elaborate changes nothing.
    fn define(&mut self, program: Program) -> Result<Reply, Diagnostics> {
        let mut circuits = self.circuits.clone();
        let mut experiments = self.experiments.clone();
        let mut accepted = Vec::new();
        let mut replaced = Vec::new();

        for def in program.circuits {
            let name = def.name.clone();
            let label = format!("circuit `{name}`");
            if circuits.insert(name.clone(), def).is_some() {
                replaced.push(label);
            } else {
                accepted.push(label);
            }
        }
        for def in program.experiments {
            let name = def.name.name.clone();
            let label = format!("experiment `{name}`");
            if experiments.insert(name.clone(), def).is_some() {
                replaced.push(label);
            } else {
                accepted.push(label);
            }
        }

        let candidate = Program {
            circuits: circuits.values().cloned().collect(),
            experiments: experiments.values().cloned().collect(),
        };

        // A definition that mentions something not yet defined is refused
        // here: the session checks an input against the state it has now.
        let hint = missing_reference_hint(&candidate);
        circuit_dsl::compile(&candidate, &self.limits).map_err(|d| with_hint(d, hint.clone()))?;

        self.circuits = circuits;
        self.experiments = experiments;
        Ok(Reply::Defined { accepted, replaced })
    }

    // ---- commands --------------------------------------------------------

    /// Handle a `:command`. Terminal commands are not part of the language:
    /// they are recognised here, at the start of an input, by a fixed list.
    pub fn command(&mut self, line: &str) -> Result<Reply, Diagnostics> {
        let mut parts = line.split_whitespace();
        let name = parts.next().unwrap_or_default();
        let rest: Vec<&str> = parts.collect();

        match name {
            ":help" => Ok(Reply::Message(self.help())),
            ":list" => Ok(Reply::Message(self.list())),
            ":reset" => Ok(Reply::Message(self.reset())),
            ":quit" | ":exit" => Ok(Reply::Quit),
            ":load" => self.load_command(&rest),
            ":run" => self.run_command(&rest),
            other => Err(Diagnostics::single(
                Diagnostic::error(Code::Syntax, format!("unknown command `{other}`"))
                    .with_note(format!("commands are {}", COMMANDS.join(", ")))
                    .with_note("anything else is read as the language"),
            )),
        }
    }

    fn help(&self) -> String {
        [
            "commands:",
            "  :help                     this text",
            "  :load <file> [--replace]  load a whole file (default: refuse name clashes)",
            "  :list                     show definitions and variables",
            "  :run <exp> [name=expr ...] [--out DIR]",
            "                            run an experiment; overrides are evaluated here",
            "  :reset                    clear definitions and variables",
            "  :quit                     leave (Ctrl+D does the same)",
            "",
            "language:",
            "  r = 1.kohm                a session variable (numbers only)",
            "  tau = r * 100.nF          arithmetic keeps units",
            "  circuit :d do ... end      define a circuit, subcircuit or experiment",
            "  Expressions and definitions are checked as you go; a definition is",
            "  stored only if the whole session still compiles.",
        ]
        .join("\n")
    }

    fn list(&self) -> String {
        let mut out = Vec::new();
        out.push(format!(
            "{} definition(s), {} variable(s)",
            self.circuits.len() + self.experiments.len(),
            self.vars.len()
        ));
        for def in self.circuits.values() {
            let kind = if def.is_subcircuit {
                "subcircuit"
            } else {
                "circuit"
            };
            out.push(format!(
                "  {kind} :{} ({} device statement(s))",
                def.name,
                def.body.len()
            ));
        }
        for def in self.experiments.values() {
            out.push(format!(
                "  experiment :{} on circuit :{}",
                def.name.name, def.circuit.name
            ));
        }
        for (name, value) in &self.vars {
            out.push(format!(
                "  {name} = {}",
                circuit_core::format_quantity(*value)
            ));
        }
        out.join("\n")
    }

    fn reset(&mut self) -> String {
        let definitions = self.circuits.len() + self.experiments.len();
        let variables = self.vars.len();
        self.circuits.clear();
        self.experiments.clear();
        self.vars.clear();
        self.pending.clear();
        format!("cleared {definitions} definition(s) and {variables} variable(s)")
    }

    fn load_command(&mut self, args: &[&str]) -> Result<Reply, Diagnostics> {
        let mut path: Option<&str> = None;
        let mut replace = false;
        for arg in args {
            match *arg {
                "--replace" => replace = true,
                other if other.starts_with('-') => {
                    return Err(Diagnostics::single(
                        Diagnostic::error(
                            Code::Argument,
                            format!("unknown option `{other}` for `:load`"),
                        )
                        .with_note("`:load <file> [--replace]`"),
                    ));
                }
                other => {
                    if path.is_some() {
                        return Err(Diagnostics::single(Diagnostic::error(
                            Code::Argument,
                            "`:load` takes one file",
                        )));
                    }
                    path = Some(other);
                }
            }
        }
        let Some(path) = path else {
            return Err(Diagnostics::single(
                Diagnostic::error(Code::Argument, "`:load` needs a file name")
                    .with_note("e.g. `:load examples/rc_filter.cdsl`"),
            ));
        };
        self.load(Path::new(path), replace)
    }

    /// Load a file's definitions into the session, atomically.
    pub fn load(&mut self, path: &Path, replace: bool) -> Result<Reply, Diagnostics> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            Diagnostics::single(
                Diagnostic::error(Code::Io, format!("cannot read `{}`: {e}", path.display()))
                    .with_note("the path is resolved relative to the working directory"),
            )
        })?;

        let source = self.sources.add(path.display().to_string(), text.clone());
        let tokens = circuit_dsl::lex(source, &text)?;
        let program = parse(&tokens)?;

        // Name clashes are reported rather than resolved silently: the user
        // must say `--replace` to overwrite a definition.
        let mut clashes = Vec::new();
        for def in &program.circuits {
            if self.circuits.contains_key(&def.name) {
                clashes.push(format!("circuit `{}`", def.name));
            }
        }
        for def in &program.experiments {
            if self.experiments.contains_key(&def.name.name) {
                clashes.push(format!("experiment `{}`", def.name.name));
            }
        }
        if !clashes.is_empty() && !replace {
            return Err(Diagnostics::single(
                Diagnostic::error(
                    Code::Duplicate,
                    format!(
                        "`{}` defines {} name(s) this session already has: {}",
                        path.display(),
                        clashes.len(),
                        clashes.join(", ")
                    ),
                )
                .with_note("nothing was loaded")
                .with_note("run `:load <file> --replace` to overwrite them"),
            ));
        }

        let reply = self.define(program)?;
        self.pending.clear();
        Ok(reply)
    }

    fn run_command(&mut self, args: &[&str]) -> Result<Reply, Diagnostics> {
        let mut experiment: Option<&str> = None;
        let mut out: Option<PathBuf> = None;
        let mut overrides = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = args[i];
            if arg == "--out" {
                i += 1;
                let Some(dir) = args.get(i) else {
                    return Err(Diagnostics::single(
                        Diagnostic::error(Code::Argument, "`--out` needs a directory")
                            .with_note("e.g. `:run divider --out results`"),
                    ));
                };
                out = Some(PathBuf::from(dir));
            } else if arg.starts_with('-') {
                return Err(Diagnostics::single(
                    Diagnostic::error(Code::Argument, format!("unknown option `{arg}` for `:run`"))
                        .with_note("`:run <experiment> [name=expr ...] [--out DIR]`"),
                ));
            } else if arg.contains('=') {
                overrides.push(arg.to_string());
            } else if experiment.is_none() {
                experiment = Some(arg);
            } else {
                return Err(Diagnostics::single(
                    Diagnostic::error(
                        Code::Argument,
                        format!("unexpected `{arg}`; a run takes one experiment"),
                    )
                    .with_note("overrides are written `name=expr`"),
                ));
            }
            i += 1;
        }

        let Some(experiment) = experiment else {
            return Err(Diagnostics::single(
                Diagnostic::error(Code::Argument, "`:run` needs an experiment name")
                    .with_note(format!("defined: {:?}", self.experiment_names())),
            ));
        };
        let experiment = experiment.to_string();

        // Overrides are ordinary assignments, so they get the same syntax,
        // the same name rules and the same evaluation as `name = expr`.
        let mut fixed = Vec::new();
        for text in &overrides {
            let source = self.sources.add("<override>", text.clone());
            let tokens = circuit_dsl::lex(source, text)?;
            let Input::Assign { name, value, .. } = parse_input(&tokens)? else {
                return Err(Diagnostics::single(
                    Diagnostic::error(
                        Code::Argument,
                        format!("`{text}` is not an override; write `name=expr`"),
                    )
                    .with_note("e.g. `:run divider r1=2.kohm`"),
                ));
            };
            let v = eval::eval(&value, &self.var_view())?;
            let Some(q) = v.as_num() else {
                return Err(Diagnostics::single(
                    Diagnostic::error(
                        Code::Type,
                        format!(
                            "override `{name}` must be a number, found {}",
                            v.type_name()
                        ),
                    )
                    .at(value.span),
                ));
            };
            // Evaluated once, here: the value a run uses is the value shown in
            // its summary, so the run can be read back and reproduced.
            fixed.push((name, q));
        }

        self.run(&experiment, &fixed, out.as_deref())
    }

    /// Run an experiment with the current definitions.
    pub fn run(
        &mut self,
        experiment: &str,
        overrides: &[(String, Quantity)],
        out: Option<&Path>,
    ) -> Result<Reply, Diagnostics> {
        if !self.experiments.contains_key(experiment) {
            return Err(Diagnostics::single(
                Diagnostic::error(
                    Code::Name,
                    format!("no experiment named `{experiment}` in this session"),
                )
                .with_note(if self.experiments.is_empty() {
                    "define one with `experiment :name, circuit: :name do ... end`".to_string()
                } else {
                    format!("defined: {}", self.experiment_names().join(", "))
                }),
            ));
        }

        let program = self.program();
        let request = RunRequest {
            program: &program,
            experiment,
            overrides,
            limits: &self.limits,
        };
        let outcome = execute::execute(&request, &mut self.backend, &self.sources)?;

        let mut lines = Vec::new();
        lines.push(format!(
            "experiment `{experiment}` (backend {} {})",
            outcome.datasets[0].backend.name, outcome.datasets[0].backend.version
        ));
        for (name, value) in overrides {
            lines.push(format!(
                "  override {name} = {}",
                circuit_core::format_quantity(*value)
            ));
        }
        for s in outcome.summaries() {
            lines.push(format!("  {s}"));
        }
        // A scalar analysis is short enough to show in full, and showing it is
        // the point of running one interactively. A sweep or a transient is
        // not: those get a pointer to `--out` instead of a page of numbers.
        // The output view is used so the point count matches what `--out`
        // writes (raw and output differ when `output_interval:` is set).
        for d in &outcome.output_datasets {
            if matches!(d.axis, circuit_results::dataset::Axis::None) {
                for signal in &d.signals {
                    if let circuit_results::dataset::Data::Real(values) = &signal.data
                        && let [value] = values.as_slice()
                    {
                        lines.push(format!(
                            "  {} = {}",
                            signal.name,
                            circuit_core::format_quantity(Quantity::new(*value, signal.unit))
                        ));
                    }
                }
            } else {
                lines.push(format!(
                    "  {} has {} points; pass `--out <dir>` to write them",
                    d.analysis,
                    d.signals.first().map(|s| s.len()).unwrap_or(0)
                ));
            }
        }
        for w in &outcome.warnings {
            lines.push(format!("  warning: {w}"));
        }
        for m in &outcome.measures {
            // Rendered by the results layer so the REPL and a file run print
            // the same text, including which analysis the value came from: in
            // a multi-analysis experiment a name alone does not say what was
            // measured. The run only reaches this point when every requested
            // measure evaluated, so no measure can be missing from here.
            lines.push(format!("  measure {}", m.render_with_analysis()));
        }

        if let Some(dir) = out {
            let written =
                execute::write_datasets(dir, Format::Both, &outcome.output_datasets, &mut |_| {
                    Ok(())
                })?;
            for path in &written.paths {
                lines.push(format!("  wrote {}", path.display()));
            }
            // What a file run prints for the same experiment, in the same
            // words: the renderer warnings are built by warning_lines, so the
            // REPL and the file run cannot disagree about what is missing from
            // the file that was just written.
            lines.extend(written.warning_lines());
        }

        Ok(Reply::Message(lines.join("\n")))
    }

    /// A view of the variables for the shared evaluator.
    fn var_view(&self) -> VarView<'_> {
        VarView { vars: &self.vars }
    }

    /// Render diagnostics against the session's sources.
    pub fn render(&self, diagnostics: &Diagnostics) -> String {
        diagnostics.render(&self.sources)
    }
}

/// The evaluator's view of the session variables.
struct VarView<'a> {
    vars: &'a BTreeMap<String, Quantity>,
}

impl Variables for VarView<'_> {
    fn lookup(&self, name: &str) -> Option<Quantity> {
        self.vars.get(name).copied()
    }
}

/// A note for the case a session hits most often: something refers to a
/// definition that has not been entered yet.
fn missing_reference_hint(program: &Program) -> Option<String> {
    for def in &program.experiments {
        if program.circuit(&def.circuit.name).is_none() {
            return Some(format!(
                "the session checks a definition when you enter it: define `circuit :{}` first, \
                 enter both in one input, or load a whole file with `:load` (there, order does \
                 not matter)",
                def.circuit.name
            ));
        }
    }
    None
}

fn with_hint(diagnostics: Diagnostics, hint: Option<String>) -> Diagnostics {
    let Some(hint) = hint else {
        return diagnostics;
    };
    let mut items = diagnostics.into_vec();
    if let Some(first) = items.first_mut() {
        first.notes.push(hint);
    }
    let mut out = Diagnostics::new();
    for d in items {
        out.push(d);
    }
    out
}

/// Re-exported so a caller does not need to depend on `circuit-results` for
/// the output format choice.
pub use crate::execute::{Format as OutputFormat, write_datasets};

/// Names useful for completion, in a stable order.
pub fn completions(session: &Session) -> Vec<String> {
    let mut out = vec![
        ":help".to_string(),
        ":list".to_string(),
        ":load".to_string(),
        ":quit".to_string(),
        ":reset".to_string(),
        ":run".to_string(),
    ];
    out.extend(session.circuit_names().into_iter().map(String::from));
    out.extend(session.subcircuit_names().into_iter().map(String::from));
    out.extend(session.experiment_names().into_iter().map(String::from));
    out.extend(session.variables().map(|(name, _)| name.clone()));
    out.sort();
    out.dedup();
    out
}

/// Keep `sanitise` reachable for callers that name their own output files.
pub use execute::sanitise as sanitise_name;
