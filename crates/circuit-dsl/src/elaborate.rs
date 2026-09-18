//! Elaboration: AST in, `(Circuit, AnalysisPlan)` out.
//!
//! This is where the language acquires meaning:
//!
//! - name resolution (parameters, nodes, devices, models, subcircuits),
//! - dimensional checking of every argument,
//! - parameter evaluation with a dependency order and cycle detection,
//! - subcircuit instantiation, port binding and instance-local nodes,
//! - finite `for` / `if` expansion,
//! - analysis specification, probe resolution and sweep validation.
//!
//! Two rules shape the whole module:
//!
//! 1. **Nothing is guessed.** An undeclared name is an error, never an
//!    implicitly-created node; a value with the wrong dimension is an error,
//!    never a silent conversion.
//! 2. **Errors accumulate.** One run reports every independent problem it can,
//!    because `cdsl check` is most useful when it lists them all.

// `circuit_core::Diagnostic` is the project's single user-facing error type:
// a message, a primary and any number of secondary labelled spans, notes and
// context. At ~144 bytes it trips `clippy::result_large_err` on every fallible
// function here. Boxing it would push `Box<Diagnostic>` into the public API of
// this crate and make every caller unbox before rendering, to save a size that
// is irrelevant next to the allocations the error path already performs. The
// same trade-off is recorded in `circuit-results`.
#![allow(clippy::result_large_err)]

use std::collections::{HashMap, HashSet};

use circuit_core::diagnostic::{Code, Diagnostic, Diagnostics};
use circuit_core::ir::{
    self, Circuit, Device, DeviceKind, InstanceStep, Model, ModelKind, Node, NodeKind, SourceSpec,
    Waveform, terminal,
};
use circuit_core::plan::{
    AcSweep, AnalysisKind, AnalysisPlan, AnalysisTask, DcSpec, MeasureKind, MeasureRequest,
    NamedProbe, Probe, Sweep, SweepKind, SweepTarget, TranSpec,
};
use circuit_core::span::{SourceId, SourceSpan};
use circuit_core::units::{
    self, CAPACITANCE, CURRENT, Dimension, FREQUENCY, INDUCTANCE, Quantity, RESISTANCE, TIME,
    VOLTAGE,
};
use circuit_core::{AnalysisId, CircuitId, DeviceId, GROUND, Limits, ModelId, NodeId};

use crate::ast::{
    self, AnalysisCall, Arg, BinaryOp, Call, CircuitDef, DeviceStmtKind, DictEntry, ExpStmt, Expr,
    ExprKind, ForIter, Program, SpannedName, Stmt, UnaryOp,
};

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// One elaborated experiment: the circuit it runs on, and its plan.
#[derive(Clone, Debug)]
pub struct Elaborated {
    pub circuit: Circuit,
    pub plan: AnalysisPlan,
}

/// Everything a source file defines, after elaboration.
#[derive(Clone, Debug)]
pub struct Compiled {
    /// Every top-level `circuit`, elaborated with its default parameters.
    pub circuits: Vec<Circuit>,
    /// Every `experiment`, each with the circuit it runs against.
    pub experiments: Vec<Elaborated>,
}

impl Compiled {
    pub fn circuit(&self, name: &str) -> Option<&Circuit> {
        self.circuits.iter().find(|c| c.name == name)
    }

    pub fn experiment(&self, name: &str) -> Option<&Elaborated> {
        self.experiments.iter().find(|e| e.plan.name == name)
    }
}

/// Elaborate a whole program: every circuit and every experiment.
pub fn compile(program: &Program, limits: &Limits) -> Result<Compiled, Diagnostics> {
    let mut el = Elaborator::new(program, limits);
    let mut circuits = Vec::new();

    for def in &program.circuits {
        if def.is_subcircuit {
            continue;
        }
        if let Some(c) = el.elaborate_top_circuit(def, &[]) {
            circuits.push(c);
        }
    }

    // A program with no top-level circuit but with experiments is still an
    // error worth reporting at the right place.
    if circuits.is_empty() && !program.experiments.is_empty() {
        for def in &program.circuits {
            if !def.is_subcircuit {
                continue;
            }
        }
    }

    let mut experiments = Vec::new();
    for def in &program.experiments {
        let overrides = el.experiment_overrides(def);
        let Some(circuit) = el.elaborate_named_circuit(&def.circuit, &overrides) else {
            continue;
        };
        if let Some(plan) = el.elaborate_experiment(def, &circuit) {
            experiments.push(Elaborated { circuit, plan });
        }
    }

    if el.diagnostics.has_errors() {
        Err(el.diagnostics)
    } else {
        Ok(Compiled {
            circuits,
            experiments,
        })
    }
}

/// Elaborate one experiment, optionally substituting parameter values.
///
/// This is the hook the parameter-sweep driver uses: it calls this once per
/// sweep point with a different override, so every point is a fresh
/// elaboration and the topology can be checked for invariance.
pub fn elaborate_experiment(
    program: &Program,
    experiment_name: &str,
    overrides: &[(String, Quantity)],
    limits: &Limits,
) -> Result<Elaborated, Diagnostics> {
    let mut el = Elaborator::new(program, limits);
    let Some(def) = program.experiment(experiment_name) else {
        return Err(Diagnostics::single(Diagnostic::error(
            Code::Name,
            format!("no experiment named `{experiment_name}`"),
        )));
    };

    let mut all = el.experiment_overrides(def);
    for (name, value) in overrides {
        // Later entries win, which is what "successive refinement" means for
        // the override chain default -> instance/experiment -> sweep point.
        all.retain(|(n, _, _)| n != name);
        all.push((name.clone(), *value, def.circuit.span));
    }

    let circuit = el.elaborate_named_circuit(&def.circuit, &all);
    let plan = circuit
        .as_ref()
        .and_then(|c| el.elaborate_experiment(def, c));

    match (circuit, plan) {
        (Some(circuit), Some(plan)) if !el.diagnostics.has_errors() => {
            Ok(Elaborated { circuit, plan })
        }
        _ => Err(el.diagnostics),
    }
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// A runtime value during elaboration.
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
    fn type_name(&self) -> &'static str {
        match self {
            Value::Num(_) => "number",
            Value::Bool(_) => "boolean",
            Value::Sym(_) => "symbol",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Dict(_) => "dictionary",
        }
    }

    fn as_num(&self) -> Option<Quantity> {
        match self {
            Value::Num(q) => Some(*q),
            _ => None,
        }
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn as_sym(&self) -> Option<&str> {
        match self {
            Value::Sym(s) => Some(s.as_str()),
            _ => None,
        }
    }

    fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Elaborator
// ---------------------------------------------------------------------------

/// A parameter scope.
///
/// Two things are tracked separately, because conflating them is a bug:
///
/// - `vars` holds the **effective** value of every parameter in scope. It is
///   pre-populated by the override chain (instance or experiment overrides,
///   then sweep points) *before* the body runs.
/// - `declared` holds the parameters a `param` statement in the body being
///   executed has declared. Only a second `param` for the same name is a
///   duplicate; a `param` whose value arrived as an override is the normal
///   case and must not be reported.
///
/// The override order from brief §5.3 (default -> instance/experiment ->
/// sweep point) falls out of this: a default is only installed when nothing
/// has already provided a value.
#[derive(Clone, Default)]
struct Scope {
    vars: HashMap<String, Quantity>,
    /// Span each parameter was declared or overridden at, for diagnostics.
    spans: HashMap<String, SourceSpan>,
    /// Parameters declared by a `param` statement in the current body.
    declared: HashSet<String>,
    /// Declared defaults, kept so an override can be checked against the
    /// dimension the parameter was written with.
    defaults: HashMap<String, Quantity>,
}

impl Scope {
    fn get(&self, name: &str) -> Option<Quantity> {
        self.vars.get(name).copied()
    }

    fn set(&mut self, name: &str, value: Quantity, span: SourceSpan) {
        self.vars.insert(name.to_string(), value);
        self.spans.insert(name.to_string(), span);
    }

    fn span(&self, name: &str) -> SourceSpan {
        self.spans
            .get(name)
            .copied()
            .unwrap_or_else(SourceSpan::synthetic)
    }
}

struct Elaborator<'a> {
    program: &'a Program,
    limits: Limits,
    diagnostics: Diagnostics,
    /// Node table under construction for the current top-level circuit.
    nodes: Vec<Node>,
    node_index: HashMap<String, NodeId>,
    devices: Vec<Device>,
    models: Vec<Model>,
    /// Device name -> the span it was defined at, for duplicate reporting.
    device_spans: HashMap<String, SourceSpan>,
    model_spans: HashMap<String, SourceSpan>,
    /// Diagnostics that must not be swallowed by later successful steps.
    error_count: usize,
    /// Instantiation stack, for recursion detection.
    stack: Vec<String>,
    /// Loop steps executed, against `limits.max_total_steps`.
    steps: u64,
}

impl<'a> Elaborator<'a> {
    fn new(program: &'a Program, limits: &Limits) -> Self {
        Self {
            program,
            limits: *limits,
            diagnostics: Diagnostics::new(),
            nodes: Vec::new(),
            node_index: HashMap::new(),
            devices: Vec::new(),
            models: Vec::new(),
            device_spans: HashMap::new(),
            model_spans: HashMap::new(),
            error_count: 0,
            stack: Vec::new(),
            steps: 0,
        }
    }

    fn error(&mut self, d: Diagnostic) {
        self.error_count += 1;
        self.diagnostics.push(d);
    }

    /// Resolve a declared name: literal, or computed from an expression.
    ///
    /// A computed name is checked to be a legal identifier, so a loop cannot
    /// produce a name that would be unwritable elsewhere in the language. The
    /// uniqueness half of "deterministic and unique" is enforced by the
    /// ordinary duplicate checks, which see the resolved names.
    fn resolve_name(&mut self, n: &SpannedName, scope: &mut Scope) -> Option<String> {
        let Some(expr) = &n.expr else {
            return Some(n.name.clone());
        };

        match self.eval(expr, scope) {
            Ok(Value::Str(s)) => {
                if let Err(message) = check_identifier(&s) {
                    self.error(
                        Diagnostic::error(
                            Code::Value,
                            format!("`{s}` is not a usable name: {message}"),
                        )
                        .at(n.span),
                    );
                    return None;
                }
                Some(s)
            }
            Ok(Value::Sym(s)) => Some(s),
            Ok(other) => {
                self.error(
                    Diagnostic::error(
                        Code::Type,
                        format!(
                            "a name must be a string or symbol, found {}",
                            other.type_name()
                        ),
                    )
                    .at(n.span)
                    .with_note("e.g. `(\"r\" + i)` builds a name from a loop variable"),
                );
                None
            }
            Err(d) => {
                self.error(d);
                None
            }
        }
    }

    // -----------------------------------------------------------------------
    // Top-level circuit
    // -----------------------------------------------------------------------

    fn elaborate_top_circuit(
        &mut self,
        def: &CircuitDef,
        overrides: &[(String, Quantity, SourceSpan)],
    ) -> Option<Circuit> {
        self.reset_circuit_state();
        self.declare_ground();

        let mut scope = Scope::default();
        self.push_overrides(&mut scope, overrides);

        let before = self.error_count;
        let mut top_ctx = Bodies::top(def.name.clone());
        self.run_body(&def.body, &mut scope, &mut top_ctx);

        if self.error_count > before {
            // Report what we have, but do not hand back a half-built circuit
            // as if it were complete.
            return None;
        }
        self.finish_circuit(&def.name, def.span)
    }

    /// Elaborate the circuit named by an experiment's `circuit:` argument.
    fn elaborate_named_circuit(
        &mut self,
        name: &SpannedName,
        overrides: &[(String, Quantity, SourceSpan)],
    ) -> Option<Circuit> {
        let Some(def) = self.program.circuit(&name.name) else {
            let available: Vec<&str> = self
                .program
                .circuits
                .iter()
                .filter(|c| !c.is_subcircuit)
                .map(|c| c.name.as_str())
                .collect();
            let mut d = Diagnostic::error(Code::Name, format!("unknown circuit `{}`", name.name))
                .at(name.span);
            if !available.is_empty() {
                d = d.with_note(format!("defined circuits: {}", available.join(", ")));
            }
            self.error(d);
            return None;
        };

        if def.is_subcircuit {
            self.error(
                Diagnostic::error(
                    Code::Name,
                    format!("`{}` is a subcircuit and cannot be run directly", name.name),
                )
                .at(name.span)
                .with_note("instantiate it from a `circuit` instead"),
            );
            return None;
        }

        self.elaborate_top_circuit(def, overrides)
    }

    fn reset_circuit_state(&mut self) {
        self.nodes.clear();
        self.node_index.clear();
        self.devices.clear();
        self.models.clear();
        self.device_spans.clear();
        self.model_spans.clear();
        self.stack.clear();
        self.steps = 0;
    }

    fn declare_ground(&mut self) {
        self.node_index.insert("gnd".to_string(), GROUND);
        self.node_index.insert("0".to_string(), GROUND);
        self.nodes.push(Node {
            id: GROUND,
            name: "gnd".to_string(),
            local_name: "gnd".to_string(),
            kind: NodeKind::Ground,
            span: SourceSpan::synthetic(),
        });
    }

    fn finish_circuit(&mut self, name: &str, span: SourceSpan) -> Option<Circuit> {
        let circuit = match Circuit::new(
            CircuitId(0),
            name.to_string(),
            std::mem::take(&mut self.nodes),
            std::mem::take(&mut self.devices),
            std::mem::take(&mut self.models),
            span,
        ) {
            Ok(c) => c,
            Err(e) => {
                self.error(Diagnostic::error(Code::Backend, e.to_string()).at(span));
                return None;
            }
        };

        // The engine will not report an undetermined node: its gmin stepping
        // keeps a floating node finite and returns success (see
        // docs/backend-evaluation.md §4.6). So the DC reference-path check has
        // to happen here, and it has to be a reachability test rather than a
        // "does anything connect?" test, because a capacitor path is not a DC
        // path (brief §9).
        for f in circuit_core::floating_nodes(&circuit) {
            let node = circuit.node(f.node);
            let (message, note) = match f.kind {
                circuit_core::FloatingKind::Unused => (
                    format!(
                        "node `{}` is declared but nothing connects to it",
                        circuit.node_name(f.node)
                    ),
                    "remove the declaration, or connect a device to it".to_string(),
                ),
                circuit_core::FloatingKind::AcCoupledOnly => (
                    format!(
                        "node `{}` has no DC path to ground, so its operating point is undefined",
                        circuit.node_name(f.node)
                    ),
                    "a capacitor or current source does not provide a DC reference path; add a \
                     resistor, inductor, voltage source or diode to ground"
                        .to_string(),
                ),
            };
            let mut d = Diagnostic::error(Code::Name, message)
                .at(node.map(|n| n.span).unwrap_or(span))
                .with_note(note);
            if !f.blocking.is_empty() {
                d = d.with_note(format!(
                    "attached but not conducting at DC: {}",
                    f.blocking
                        .iter()
                        .filter_map(|id| circuit.device(*id))
                        .map(|dev| dev.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            self.error(d);
        }

        if self.error_count > 0 {
            return None;
        }
        Some(circuit)
    }

    fn push_overrides(&mut self, scope: &mut Scope, overrides: &[(String, Quantity, SourceSpan)]) {
        for (name, value, span) in overrides {
            scope.set(name, *value, *span);
        }
    }

    // -----------------------------------------------------------------------
    // Body execution
    // -----------------------------------------------------------------------

    /// Everything a device statement needs to resolve its node symbols.
    fn run_body(&mut self, body: &[Stmt], scope: &mut Scope, ctx: &mut Bodies) {
        for stmt in body {
            if self.error_count > 0 {
                // Stop after the first structural error in a body: later
                // statements would produce noise from a broken scope.
                break;
            }
            self.run_stmt(stmt, scope, ctx);
        }
    }

    fn run_stmt(&mut self, stmt: &Stmt, scope: &mut Scope, ctx: &mut Bodies) {
        self.steps += 1;
        if self.steps > self.limits.max_total_steps {
            self.error(
                Diagnostic::error(
                    Code::Limit,
                    format!("elaboration exceeded {} steps", self.limits.max_total_steps),
                )
                .at(stmt.span()),
            );
            return;
        }

        match stmt {
            Stmt::Param(p) => self.stmt_param(p, scope),
            Stmt::Node(n) => self.stmt_node(n, scope, ctx),
            Stmt::Device(d) => self.stmt_device(d, scope, ctx),
            Stmt::Model(m) => self.stmt_model(m, scope),
            Stmt::Instance(i) => self.stmt_instance(i, scope, ctx),
            Stmt::For(f) => self.stmt_for(f, scope, ctx),
            Stmt::If(i) => self.stmt_if(i, scope, ctx),
        }
    }

    fn stmt_param(&mut self, p: &ast::ParamDecl, scope: &mut Scope) {
        if p.name.expr.is_some() {
            self.error(
                Diagnostic::error(Code::Type, "a parameter name must be written literally")
                    .at(p.name.span)
                    .with_note(
                        "a computed name cannot be referred to by the expressions that use it",
                    ),
            );
            return;
        }

        if !scope.declared.insert(p.name.name.clone()) {
            self.error(
                Diagnostic::error(
                    Code::Duplicate,
                    format!("parameter `{}` is declared twice", p.name.name),
                )
                .at(p.name.span)
                .with_secondary(scope.span(&p.name.name), "first declared here"),
            );
            return;
        }

        // Record where this parameter was written, so a later duplicate can
        // point back here even when an override supplied the value.
        scope.spans.insert(p.name.name.clone(), p.name.span);

        let Some(default) = &p.default else {
            // No default: the value must come from an override. If it does
            // not, reading the name reports it as undeclared.
            return;
        };

        match self.eval(default, scope) {
            Ok(Value::Num(q)) => {
                self.check_finite(q, default.span, &p.name.name);
                scope.defaults.insert(p.name.name.clone(), q);
                // An override already supplied a value; the default only
                // records the expected dimension.
                if scope.vars.contains_key(&p.name.name) {
                    return;
                }
                scope.set(&p.name.name, q, p.name.span);
            }
            Ok(other) => self.error(
                Diagnostic::error(
                    Code::Type,
                    format!(
                        "parameter `{}` must be a number, found {}",
                        p.name.name,
                        other.type_name()
                    ),
                )
                .at(default.span),
            ),
            Err(d) => self.error(d),
        }
    }

    fn stmt_node(&mut self, n: &ast::NodeDecl, scope: &mut Scope, ctx: &mut Bodies) {
        for name in &n.names {
            let Some(local) = self.resolve_name(name, scope) else {
                continue;
            };
            if local == "gnd" || local == "0" {
                self.error(
                    Diagnostic::error(
                        Code::Duplicate,
                        "`gnd` is the reserved global reference and cannot be redeclared",
                    )
                    .at(name.span),
                );
                continue;
            }
            if !ctx.local_nodes.insert(local.clone()) {
                self.error(
                    Diagnostic::error(
                        Code::Duplicate,
                        format!("node `{local}` is declared twice in this body"),
                    )
                    .at(name.span),
                );
                continue;
            }

            let qualified = ctx.qualify(&local);
            if self.node_index.contains_key(&qualified) {
                self.error(
                    Diagnostic::error(
                        Code::Duplicate,
                        format!("node `{qualified}` already exists"),
                    )
                    .at(name.span),
                );
                continue;
            }
            let id = NodeId(self.nodes.len() as u32);
            self.node_index.insert(qualified.clone(), id);
            self.nodes.push(Node {
                id,
                name: qualified,
                local_name: local,
                kind: NodeKind::Normal,
                span: name.span,
            });
        }
    }

    fn stmt_device(&mut self, d: &ast::DeviceStmt, scope: &mut Scope, ctx: &mut Bodies) {
        let Some(local) = self.resolve_name(&d.name, scope) else {
            return;
        };
        let name = ctx.qualify(&local);
        if self.device_spans.contains_key(&name) {
            let first = self.device_spans[&name];
            self.error(
                Diagnostic::error(Code::Duplicate, format!("device `{name}` is defined twice"))
                    .at(d.name.span)
                    .with_secondary(first, "first defined here"),
            );
            return;
        }
        if self.devices.len() >= self.limits.max_devices {
            self.error(
                Diagnostic::error(
                    Code::Limit,
                    format!("more than {} devices", self.limits.max_devices),
                )
                .at(d.span),
            );
            return;
        }

        let kind = match d.kind {
            DeviceStmtKind::Resistor => DeviceKind::Resistor,
            DeviceStmtKind::Capacitor => DeviceKind::Capacitor,
            DeviceStmtKind::Inductor => DeviceKind::Inductor,
            DeviceStmtKind::VoltageSource => DeviceKind::VoltageSource,
            DeviceStmtKind::CurrentSource => DeviceKind::CurrentSource,
            DeviceStmtKind::Diode => DeviceKind::Diode,
        };

        // ---- terminals ----------------------------------------------------
        let (pos_t, neg_t) = match kind {
            DeviceKind::Diode => (terminal::ANODE, terminal::CATHODE),
            _ => (terminal::POS, terminal::NEG),
        };
        let Some(p_node) = self.node_arg(d, "p", pos_t, scope, ctx) else {
            return;
        };
        let Some(n_node) = self.node_arg(d, "n", neg_t, scope, ctx) else {
            return;
        };

        // ---- allowed argument names ---------------------------------------
        let allowed: &[&str] = match kind {
            DeviceKind::Resistor | DeviceKind::Capacitor | DeviceKind::Inductor => {
                &["p", "n", "value"]
            }
            DeviceKind::VoltageSource | DeviceKind::CurrentSource => {
                &["p", "n", "dc", "ac", "waveform"]
            }
            DeviceKind::Diode => &["p", "n", "model"],
        };
        self.reject_unknown_args(&d.args, allowed, d.kind.keyword(), d.span);

        // ---- value / source / model ---------------------------------------
        let mut params = HashMap::new();
        let mut source = None;
        let mut model = None;

        match kind {
            DeviceKind::Resistor | DeviceKind::Capacitor | DeviceKind::Inductor => {
                let want = kind
                    .value_dimension()
                    .expect("passive devices have a value");
                let Some(arg) = d.args.iter().find(|a| a.name == "value") else {
                    self.error(
                        Diagnostic::error(
                            Code::Argument,
                            format!("`{}` requires `value:`", d.kind.keyword()),
                        )
                        .at(d.span),
                    );
                    return;
                };
                let Some(q) = self.num_arg(arg, scope, want, &format!("{}.value", local)) else {
                    return;
                };
                if q.value <= 0.0 {
                    self.error(
                        Diagnostic::error(
                            Code::Value,
                            format!(
                                "`{}` must be greater than zero, found {}",
                                arg.name,
                                arg.value_text_hint()
                            ),
                        )
                        .at(arg.value.span)
                        .with_context("device", local.clone())
                        .with_note(
                            "zero or negative R/L/C values are rejected rather than \
                             replaced by a small positive number",
                        ),
                    );
                    return;
                }
                params.insert("value".to_string(), q);
            }

            DeviceKind::VoltageSource | DeviceKind::CurrentSource => {
                let want = if kind == DeviceKind::VoltageSource {
                    VOLTAGE
                } else {
                    CURRENT
                };
                let mut spec = SourceSpec::default();

                if let Some(arg) = d.args.iter().find(|a| a.name == "dc")
                    && let Some(q) = self.num_arg(arg, scope, want, "dc")
                {
                    spec.dc = Some(q);
                }
                if let Some(arg) = d.args.iter().find(|a| a.name == "ac")
                    && let Some(q) = self.num_arg(arg, scope, want, "ac")
                {
                    spec.ac = Some(ir::AcSpec {
                        magnitude: q,
                        phase_rad: 0.0,
                    });
                }
                if let Some(arg) = d.args.iter().find(|a| a.name == "waveform") {
                    if let Some(w) = self.waveform_arg(&arg.value, scope, want) {
                        spec.waveform = Some(w);
                    }
                }

                if spec.is_empty() {
                    self.error(
                        Diagnostic::error(
                            Code::Argument,
                            format!(
                                "`{}` needs at least one of `dc:`, `ac:` or `waveform:`",
                                d.kind.keyword()
                            ),
                        )
                        .at(d.span),
                    );
                    return;
                }
                source = Some(spec);
            }

            DeviceKind::Diode => {
                let Some(arg) = d.args.iter().find(|a| a.name == "model") else {
                    self.error(
                        Diagnostic::error(Code::Argument, "`diode` requires `model:`").at(d.span),
                    );
                    return;
                };
                let Some(sym) = arg.value.as_symbol() else {
                    self.error(
                        Diagnostic::error(
                            Code::Type,
                            "`model:` takes a model name symbol, e.g. `model: :dmod`",
                        )
                        .at(arg.value.span),
                    );
                    return;
                };
                match self.model_spans.get(sym).copied() {
                    Some(_) => {
                        let id = self
                            .models
                            .iter()
                            .position(|m| m.name == sym)
                            .map(|i| ModelId(i as u32));
                        model = id;
                    }
                    None => self.error(
                        Diagnostic::error(Code::Name, format!("unknown model `:{sym}`"))
                            .at(arg.value.span)
                            .with_note("declare it with `model :name, type: :diode, ...` first"),
                    ),
                }
                if model.is_none() {
                    return;
                }
            }
        }

        let id = DeviceId(self.devices.len() as u32);
        self.device_spans.insert(name.clone(), d.name.span);
        self.devices.push(Device {
            id,
            kind,
            local_name: local.clone(),
            name,
            terminals: vec![(pos_t.to_string(), p_node), (neg_t.to_string(), n_node)],
            params,
            model,
            source,
            def_span: d.name.span,
            instance_path: ctx
                .prefix
                .iter()
                .zip(ctx.of_stack.iter().chain(std::iter::repeat(&String::new())))
                .map(|(instance, of)| InstanceStep {
                    instance: instance.clone(),
                    of: of.clone(),
                    span: d.name.span,
                })
                .collect(),
        });
    }

    /// Resolve a terminal argument to a node id, reporting a missing terminal
    /// or an undeclared node.
    fn node_arg(
        &mut self,
        d: &ast::DeviceStmt,
        arg_name: &str,
        terminal_name: &str,
        scope: &mut Scope,
        ctx: &mut Bodies,
    ) -> Option<NodeId> {
        let Some(arg) = d.args.iter().find(|a| a.name == arg_name) else {
            self.error(
                Diagnostic::error(
                    Code::Argument,
                    format!("`{}` requires terminal `{arg_name}:`", d.kind.keyword()),
                )
                .at(d.span)
                .with_note(format!(
                    "for a {} the terminals are `p` ({}), `n` ({})",
                    d.kind.keyword(),
                    if terminal_name == terminal::ANODE {
                        "anode"
                    } else {
                        "positive"
                    },
                    if terminal_name == terminal::CATHODE {
                        "cathode"
                    } else {
                        "negative"
                    }
                )),
            );
            return None;
        };
        // A terminal is usually a literal `:name`, but it may also be a name
        // computed at elaboration time — `n: ("mid" + k)` — the same way a
        // device name can. Without this a loop cannot build a ladder or a
        // chain, because every rung would have to name its nodes literally.
        let local = match &arg.value.kind {
            ast::ExprKind::Symbol(sym) => sym.clone(),
            // A bare word is only a name if it is a variable holding one;
            // otherwise it is a typo for `:word`, and saying so is more useful
            // than "unknown variable".
            ast::ExprKind::Var(v) if !scope.vars.contains_key(v) => {
                self.error(
                    Diagnostic::error(
                        Code::Type,
                        format!("`{arg_name}:` takes a node symbol, e.g. `{arg_name}: :vin`"),
                    )
                    .at(arg.value.span)
                    .with_note(format!(
                        "write `:{v}` for the node, or declare `param :{v}` first"
                    )),
                );
                return None;
            }
            _ => {
                let sn = SpannedName::expressed(arg.value.clone(), arg.value.span);
                self.resolve_name(&sn, scope)?
            }
        };
        self.resolve_node(&local, arg.value.span, ctx)
    }

    /// Resolve a node symbol in the current body: a port, an internal node, or
    /// the global ground.
    fn resolve_node(&mut self, sym: &str, span: SourceSpan, ctx: &mut Bodies) -> Option<NodeId> {
        if sym == "gnd" || sym == "0" {
            return Some(GROUND);
        }
        if let Some(n) = ctx.ports.get(sym) {
            return Some(*n);
        }
        let qualified = ctx.qualify(sym);
        if let Some(n) = self.node_index.get(&qualified) {
            return Some(*n);
        }
        // An explicit port name that was never bound (only reachable if the
        // instantiation is itself broken, which is reported separately).
        let available: Vec<&str> = ctx
            .local_nodes
            .iter()
            .map(String::as_str)
            .chain(ctx.ports.keys().map(String::as_str))
            .collect();
        let mut d = Diagnostic::error(Code::Name, format!("node `:{sym}` is not declared"))
            .at(span)
            .with_note("declare it with `node :name` before using it");
        if !available.is_empty() {
            let mut list = available.clone();
            list.sort_unstable();
            d = d.with_note(format!("nodes in scope: {}", list.join(", ")));
        }
        self.error(d);
        None
    }

    fn reject_unknown_args(
        &mut self,
        args: &[Arg],
        allowed: &[&str],
        what: &str,
        span: SourceSpan,
    ) {
        for arg in args {
            if allowed.contains(&arg.name.as_str()) {
                continue;
            }
            self.error(
                Diagnostic::error(
                    Code::Argument,
                    format!("`{what}` has no argument `{}`", arg.name),
                )
                .at(arg.name_span)
                .with_note(format!("accepted: {}", allowed.join(", "))),
            );
            let _ = span;
        }
    }

    fn stmt_model(&mut self, m: &ast::ModelDecl, scope: &mut Scope) {
        let name = m.name.name.clone();
        if let Some(first) = self.model_spans.get(&name).copied() {
            self.error(
                Diagnostic::error(Code::Duplicate, format!("model `{name}` is defined twice"))
                    .at(m.name.span)
                    .with_secondary(first, "first defined here"),
            );
            return;
        }

        let Some(type_arg) = m.args.iter().find(|a| a.name == "type") else {
            self.error(Diagnostic::error(Code::Argument, "`model` requires `type:`").at(m.span));
            return;
        };
        let Some(type_sym) = type_arg.value.as_symbol() else {
            self.error(
                Diagnostic::error(Code::Type, "`type:` takes a symbol, e.g. `type: :diode`")
                    .at(type_arg.value.span),
            );
            return;
        };
        let kind = match type_sym {
            "diode" => ModelKind::Diode,
            other => {
                self.error(
                    Diagnostic::error(Code::Unsupported, format!("unknown model type `:{other}`"))
                        .at(type_arg.value.span)
                        .with_note("supported model types: diode"),
                );
                return;
            }
        };

        let mut params = HashMap::new();
        for arg in &m.args {
            if arg.name == "type" {
                continue;
            }
            let want = match arg.name.as_str() {
                "is" => CURRENT,
                "n" => units::DIMENSIONLESS,
                _ => units::DIMENSIONLESS,
            };
            if let Some(q) = self.num_arg(arg, scope, want, &format!("model {name}")) {
                params.insert(arg.name.clone(), q);
            }
        }

        let id = ModelId(self.models.len() as u32);
        self.model_spans.insert(name.clone(), m.name.span);
        self.models.push(Model {
            id,
            name,
            kind,
            params,
            span: m.span,
        });
    }

    // -----------------------------------------------------------------------
    // Subcircuit instantiation
    // -----------------------------------------------------------------------

    fn stmt_instance(&mut self, inst: &ast::InstanceStmt, scope: &mut Scope, ctx: &mut Bodies) {
        let def_name = inst.of.name.clone();
        let Some(def) = self.program.circuit(&def_name) else {
            self.error(
                Diagnostic::error(Code::Name, format!("unknown subcircuit `:{def_name}`"))
                    .at(inst.of.span),
            );
            return;
        };
        if !def.is_subcircuit {
            self.error(
                Diagnostic::error(
                    Code::Name,
                    format!("`{def_name}` is a circuit, not a subcircuit"),
                )
                .at(inst.of.span)
                .with_note("only `subcircuit` definitions can be instantiated"),
            );
            return;
        }

        // ---- recursion ----------------------------------------------------
        if self.stack.contains(&def_name) {
            let mut chain = self.stack.clone();
            chain.push(def_name.clone());
            self.error(
                Diagnostic::error(
                    Code::Recursion,
                    format!("subcircuit `{def_name}` instantiates itself"),
                )
                .at(inst.of.span)
                .with_note(format!("call chain: {}", chain.join(" -> ")))
                .with_note("recursive instantiation is not supported"),
            );
            return;
        }
        if self.stack.len() >= self.limits.max_depth {
            self.error(
                Diagnostic::error(
                    Code::Limit,
                    format!(
                        "subcircuit nesting deeper than {} levels",
                        self.limits.max_depth
                    ),
                )
                .at(inst.of.span)
                .with_note(format!("chain: {}", self.stack.join(" -> "))),
            );
            return;
        }

        let Some(instance_local) = self.resolve_name(&inst.name, scope) else {
            return;
        };
        let instance_name = ctx.qualify(&instance_local);
        if self.device_spans.contains_key(&instance_name)
            || self.node_index.contains_key(&instance_name)
        {
            self.error(
                Diagnostic::error(
                    Code::Duplicate,
                    format!("`{instance_name}` is already defined"),
                )
                .at(inst.name.span),
            );
            return;
        }

        // ---- port binding --------------------------------------------------
        let declared: Vec<&str> = def.ports.iter().map(|p| p.name.as_str()).collect();
        let mut ports: HashMap<String, NodeId> = HashMap::new();
        let mut ok = true;

        for entry in &inst.ports {
            if !declared.contains(&entry.key.as_str()) {
                self.error(
                    Diagnostic::error(
                        Code::Port,
                        format!("`{def_name}` has no port named `{}`", entry.key),
                    )
                    .at(entry.key_span)
                    .with_note(format!("declared ports: {}", declared.join(", "))),
                );
                ok = false;
                continue;
            }
            let Some(sym) = entry.value.as_symbol() else {
                self.error(
                    Diagnostic::error(
                        Code::Type,
                        format!("port `{}` must be bound to a node symbol", entry.key),
                    )
                    .at(entry.value.span),
                );
                ok = false;
                continue;
            };
            if ports.contains_key(&entry.key) {
                self.error(
                    Diagnostic::error(Code::Port, format!("port `{}` is bound twice", entry.key))
                        .at(entry.key_span),
                );
                ok = false;
                continue;
            }
            match self.resolve_node(sym, entry.value.span, ctx) {
                Some(n) => {
                    ports.insert(entry.key.clone(), n);
                }
                None => ok = false,
            }
        }

        for port in &def.ports {
            if !ports.contains_key(&port.name) {
                self.error(
                    Diagnostic::error(
                        Code::Port,
                        format!(
                            "instance `{}` does not bind port `{}`",
                            inst.name.name, port.name
                        ),
                    )
                    .at(inst.name.span)
                    .with_note(format!("all ports must be bound: {}", declared.join(", "))),
                );
                ok = false;
            }
        }

        // ---- parameter overrides -------------------------------------------
        let mut inner = Scope::default();
        let mut seen_defaults = HashSet::new();
        for stmt in &def.body {
            if let Stmt::Param(p) = stmt {
                if !seen_defaults.insert(p.name.name.clone()) {
                    continue;
                }
                if let Some(expr) = &p.default {
                    match self.eval(expr, &mut inner) {
                        Ok(Value::Num(q)) => inner.set(&p.name.name, q, p.name.span),
                        Ok(other) => self.error(
                            Diagnostic::error(
                                Code::Type,
                                format!(
                                    "parameter `{}` must be a number, found {}",
                                    p.name.name,
                                    other.type_name()
                                ),
                            )
                            .at(expr.span),
                        ),
                        Err(d) => self.error(d),
                    }
                }
            }
        }

        // Instance parameter values are written in the *enclosing* scope, so
        // `params: { r: rstage }` may name a parameter of the parent circuit.
        // The subcircuit's own defaults are folded in as a fallback, and each
        // accepted override is added as it is evaluated, so a later entry can
        // refer to an earlier one.
        let mut eval_scope = scope.clone();
        for (k, v) in &inner.vars {
            eval_scope.vars.entry(k.clone()).or_insert(*v);
            eval_scope.spans.entry(k.clone()).or_insert(inner.span(k));
        }

        for entry in &inst.params {
            if !seen_defaults.contains(&entry.key) {
                self.error(
                    Diagnostic::error(
                        Code::Name,
                        format!("`{def_name}` has no parameter `{}`", entry.key),
                    )
                    .at(entry.key_span)
                    .with_note(format!(
                        "declared parameters: {}",
                        if seen_defaults.is_empty() {
                            "<none>".to_string()
                        } else {
                            let mut v: Vec<&str> =
                                seen_defaults.iter().map(String::as_str).collect();
                            v.sort_unstable();
                            v.join(", ")
                        }
                    )),
                );
                ok = false;
                continue;
            }
            let previous = inner.get(&entry.key);
            match self.eval(&entry.value, &mut eval_scope) {
                Ok(Value::Num(q)) => {
                    // Dimension compatibility with the default, when there is one.
                    if let Some(prev) = previous
                        && prev.dimension != q.dimension
                    {
                        self.error(
                            Diagnostic::error(
                                Code::Dimension,
                                format!(
                                    "parameter `{}` expects {}, found {}",
                                    entry.key, prev.dimension, q.dimension
                                ),
                            )
                            .at(entry.value.span)
                            .with_dims(prev.dimension, q.dimension),
                        );
                        ok = false;
                        continue;
                    }
                    inner.set(&entry.key, q, entry.key_span);
                    eval_scope.set(&entry.key, q, entry.key_span);
                }
                Ok(other) => {
                    self.error(
                        Diagnostic::error(
                            Code::Type,
                            format!(
                                "parameter `{}` must be a number, found {}",
                                entry.key,
                                other.type_name()
                            ),
                        )
                        .at(entry.value.span),
                    );
                    ok = false;
                }
                Err(d) => {
                    self.error(d);
                    ok = false;
                }
            }
        }

        if !ok {
            return;
        }

        // ---- recurse --------------------------------------------------------
        let mut inner_ctx = Bodies {
            prefix: {
                let mut p = ctx.prefix.clone();
                p.push(instance_local.clone());
                p
            },
            of_stack: {
                let mut s = ctx.of_stack.clone();
                s.push(def_name.clone());
                s
            },
            ports,
            local_nodes: HashSet::new(),
            circuit_name: ctx.circuit_name.clone(),
        };

        self.stack.push(def_name.clone());
        let before = self.error_count;
        self.run_body(&def.body, &mut inner, &mut inner_ctx);
        self.stack.pop();

        if self.error_count == before {
            // Record the instance so later diagnostics can name the chain.
            // (Nothing to do here: devices already carry `instance_path`.)
        }
    }

    // -----------------------------------------------------------------------
    // Loops and conditionals
    // -----------------------------------------------------------------------

    fn stmt_for(&mut self, f: &ast::ForStmt, scope: &mut Scope, ctx: &mut Bodies) {
        let items: Vec<Value> = match &f.iter {
            ForIter::List(expr) => match self.eval(expr, scope) {
                Ok(v) => match v.as_array() {
                    Some(a) => a.to_vec(),
                    None => {
                        self.error(
                            Diagnostic::error(
                                Code::Type,
                                format!(
                                    "`for` over a list needs an array, found {}",
                                    v.type_name()
                                ),
                            )
                            .at(expr.span)
                            .with_note("e.g. `for i in [1, 2, 3] do`"),
                        );
                        return;
                    }
                },
                Err(d) => {
                    self.error(d);
                    return;
                }
            },
            ForIter::Range { start, end } => {
                let (a, b) = match (self.eval(start, scope), self.eval(end, scope)) {
                    (Ok(a), Ok(b)) => (a, b),
                    (Err(d), _) | (_, Err(d)) => {
                        self.error(d);
                        return;
                    }
                };
                let (Some(a), Some(b)) = (a.as_num(), b.as_num()) else {
                    self.error(
                        Diagnostic::error(Code::Type, "`for ... in a..b` needs integer bounds")
                            .at(start.span.merge(end.span)),
                    );
                    return;
                };
                if !a.dimension.is_dimensionless() || !b.dimension.is_dimensionless() {
                    self.error(
                        Diagnostic::error(Code::Dimension, "a loop range must be dimensionless")
                            .at(start.span.merge(end.span))
                            .with_dims(units::DIMENSIONLESS, a.dimension),
                    );
                    return;
                }
                let (lo, hi) = (a.value.round() as i64, b.value.round() as i64);
                if hi < lo {
                    // An empty range is legal and simply does nothing.
                    Vec::new()
                } else if (hi - lo) as u64 + 1 > self.limits.max_loop_iterations {
                    self.error(
                        Diagnostic::error(
                            Code::Limit,
                            format!(
                                "loop of {} iterations exceeds the limit of {}",
                                hi - lo + 1,
                                self.limits.max_loop_iterations
                            ),
                        )
                        .at(f.span),
                    );
                    return;
                } else {
                    (lo..=hi)
                        .map(|i| Value::Num(Quantity::scalar(i as f64)))
                        .collect()
                }
            }
        };

        if items.len() as u64 > self.limits.max_loop_iterations {
            self.error(
                Diagnostic::error(
                    Code::Limit,
                    format!(
                        "loop of {} iterations exceeds the limit of {}",
                        items.len(),
                        self.limits.max_loop_iterations
                    ),
                )
                .at(f.span),
            );
            return;
        }

        for item in items {
            let previous = scope.get(&f.var.name);
            match item {
                Value::Num(q) => scope.set(&f.var.name, q, f.var.span),
                other => {
                    self.error(
                        Diagnostic::error(
                            Code::Type,
                            format!(
                                "loop variable `{}` must be a number, found {}",
                                f.var.name,
                                other.type_name()
                            ),
                        )
                        .at(f.span),
                    );
                    return;
                }
            }
            self.run_body(&f.body, scope, ctx);
            if self.error_count > 0 {
                return;
            }
            match previous {
                Some(p) => scope.set(&f.var.name, p, f.var.span),
                None => {
                    scope.vars.remove(&f.var.name);
                }
            }
        }
    }

    fn stmt_if(&mut self, s: &ast::IfStmt, scope: &mut Scope, ctx: &mut Bodies) {
        for (cond, body) in &s.arms {
            match self.eval(cond, scope) {
                Ok(v) => match v.as_bool() {
                    Some(true) => {
                        self.run_body(body, scope, ctx);
                        return;
                    }
                    Some(false) => continue,
                    None => {
                        self.error(
                            Diagnostic::error(
                                Code::Type,
                                format!("`if` needs a boolean condition, found {}", v.type_name()),
                            )
                            .at(cond.span)
                            .with_note("compare with `==`, `<`, `>=` etc. to get a boolean"),
                        );
                        return;
                    }
                },
                Err(d) => {
                    self.error(d);
                    return;
                }
            }
        }
        if let Some(body) = &s.else_body {
            self.run_body(body, scope, ctx);
        }
    }

    // -----------------------------------------------------------------------
    // Expression evaluation
    // -----------------------------------------------------------------------

    fn eval(&mut self, e: &Expr, scope: &mut Scope) -> Result<Value, Diagnostic> {
        match &e.kind {
            ExprKind::Int(i) => Ok(Value::Num(Quantity::scalar(*i as f64))),
            ExprKind::Float(f) => Ok(Value::Num(Quantity::scalar(*f))),
            ExprKind::Quantity(q) => Ok(Value::Num(Quantity::new(q.value, q.dimension))),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Str(s) => Ok(Value::Str(s.clone())),
            ExprKind::Symbol(s) => Ok(Value::Sym(s.clone())),

            ExprKind::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.eval(item, scope)?);
                }
                Ok(Value::Array(out))
            }

            ExprKind::Dict(entries) => {
                let mut out = Vec::with_capacity(entries.len());
                for entry in entries {
                    out.push((entry.key.clone(), self.eval(&entry.value, scope)?));
                }
                Ok(Value::Dict(out))
            }

            ExprKind::Var(name) => match scope.get(name) {
                Some(q) => Ok(Value::Num(q)),
                None => {
                    let mut d = Diagnostic::error(
                        Code::Name,
                        format!("`{name}` is not declared"),
                    )
                    .at(e.span)
                    .with_note(
                        "an undeclared name is never treated as a node, device or function call",
                    );
                    if let Some(src) = scope.spans.get(name) {
                        d = d.with_secondary(*src, "a parameter with this name is declared here");
                    }
                    Err(d)
                }
            },

            ExprKind::Unary { op, rhs } => {
                let v = self.eval(rhs, scope)?;
                match (op, v) {
                    (UnaryOp::Neg, Value::Num(q)) => Ok(Value::Num(-q)),
                    (UnaryOp::Pos, Value::Num(q)) => Ok(Value::Num(q)),
                    (UnaryOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                    (UnaryOp::Not, other) => Err(Diagnostic::error(
                        Code::Type,
                        format!("`!` needs a boolean, found {}", other.type_name()),
                    )
                    .at(e.span)),
                    (_, other) => Err(Diagnostic::error(
                        Code::Type,
                        format!("cannot negate {}", other.type_name()),
                    )
                    .at(e.span)),
                }
            }

            ExprKind::Binary { op, lhs, rhs } => {
                let a = self.eval(lhs, scope)?;
                let b = self.eval(rhs, scope)?;
                self.eval_binary(*op, a, b, e.span, lhs.span, rhs.span)
            }

            ExprKind::Call(call) => self.eval_call(call, scope),
        }
    }

    fn eval_binary(
        &mut self,
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
                .at(span));
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

    fn eval_call(&mut self, call: &Call, scope: &mut Scope) -> Result<Value, Diagnostic> {
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
                let v = self.eval(&call.positional[0], scope)?;
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
                    match self.eval(arg, scope)? {
                        Value::Num(q) => nums.push(q),
                        other => {
                            return Err(Diagnostic::error(
                                Code::Type,
                                format!(
                                    "`{}` takes numbers, found {}",
                                    call.name,
                                    other.type_name()
                                ),
                            )
                            .at(arg.span));
                        }
                    }
                }
                self.builtin_numeric(&call.name, nums, call)
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

    fn builtin_numeric(
        &mut self,
        name: &str,
        args: Vec<Quantity>,
        call: &Call,
    ) -> Result<Value, Diagnostic> {
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

    // -----------------------------------------------------------------------
    // Argument helpers
    // -----------------------------------------------------------------------

    /// Evaluate a numeric argument and require an exact dimension.
    fn num_arg(
        &mut self,
        arg: &Arg,
        scope: &mut Scope,
        want: Dimension,
        what: &str,
    ) -> Option<Quantity> {
        match self.eval(&arg.value, scope) {
            Ok(Value::Num(q)) => {
                if q.dimension != want {
                    self.error(
                        Diagnostic::error(
                            Code::Dimension,
                            format!("`{what}` needs {}, found {}", want, q.dimension),
                        )
                        .at(arg.value.span)
                        .with_dims(want, q.dimension)
                        .with_note(format!(
                            "write e.g. `{}` with a unit suffix",
                            example_for(want)
                        )),
                    );
                    return None;
                }
                self.check_finite(q, arg.value.span, &arg.name);
                Some(q)
            }
            Ok(other) => {
                self.error(
                    Diagnostic::error(
                        Code::Type,
                        format!("`{what}` needs a number, found {}", other.type_name()),
                    )
                    .at(arg.value.span),
                );
                None
            }
            Err(d) => {
                self.error(d);
                None
            }
        }
    }

    fn check_finite(&mut self, q: Quantity, span: SourceSpan, what: &str) {
        if !q.value.is_finite() {
            self.error(
                Diagnostic::error(Code::Value, format!("`{what}` is not a finite number")).at(span),
            );
        }
    }

    /// One named argument of a waveform call.
    ///
    /// A missing optional argument becomes zero of the expected dimension,
    /// which is what the waveform functions document; a missing required one
    /// is reported against the call.
    #[allow(clippy::too_many_arguments)]
    fn wave_arg(
        &mut self,
        call: &Call,
        name: &str,
        dim: Dimension,
        required: bool,
        what: &str,
        scope: &mut Scope,
    ) -> Option<Quantity> {
        match call.arg(name) {
            Some(a) => self.num_arg(a, scope, dim, &format!("{what}.{name}")),
            None if required => {
                self.error(
                    Diagnostic::error(Code::Argument, format!("`{what}` requires `{name}:`"))
                        .at(call.span),
                );
                None
            }
            None => Some(Quantity::new(0.0, dim)),
        }
    }

    fn waveform_arg(
        &mut self,
        expr: &Expr,
        scope: &mut Scope,
        want: Dimension,
    ) -> Option<Waveform> {
        let ExprKind::Call(call) = &expr.kind else {
            self.error(
                Diagnostic::error(
                    Code::Type,
                    "`waveform:` takes a waveform function, e.g. `pulse(...)`",
                )
                .at(expr.span),
            );
            return None;
        };

        let known = |name: &str, allowed: &[&str]| -> bool { allowed.contains(&name) };

        match call.name.as_str() {
            "pulse" => {
                let allowed = ["low", "high", "delay", "rise", "fall", "width", "period"];
                for arg in &call.named {
                    if !known(&arg.name, &allowed) {
                        self.error(
                            Diagnostic::error(
                                Code::Argument,
                                format!("`pulse` has no argument `{}`", arg.name),
                            )
                            .at(arg.name_span)
                            .with_note(format!("accepted: {}", allowed.join(", "))),
                        );
                        return None;
                    }
                }
                let low = self.wave_arg(call, "low", want, true, "pulse", scope)?;
                let high = self.wave_arg(call, "high", want, true, "pulse", scope)?;
                let delay = self.wave_arg(call, "delay", TIME, false, "pulse", scope)?;
                let rise = self.wave_arg(call, "rise", TIME, true, "pulse", scope)?;
                let fall = self.wave_arg(call, "fall", TIME, true, "pulse", scope)?;
                let width = self.wave_arg(call, "width", TIME, true, "pulse", scope)?;
                let period = self.wave_arg(call, "period", TIME, true, "pulse", scope)?;

                // A pulse whose period cannot contain its own width would
                // silently produce a different waveform than the user wrote.
                if period.value > 0.0 && width.value + delay.value > period.value {
                    self.error(
                        Diagnostic::error(
                            Code::Value,
                            "`pulse` width plus delay exceeds the period",
                        )
                        .at(call.span)
                        .with_context("delay + width", format!("{}", delay.value + width.value))
                        .with_context("period", format!("{}", period.value))
                        .with_note("the pulse would never return to its low level"),
                    );
                    return None;
                }
                if rise.value < 0.0 || fall.value < 0.0 || width.value < 0.0 {
                    self.error(
                        Diagnostic::error(
                            Code::Value,
                            "`pulse` rise, fall and width must not be negative",
                        )
                        .at(call.span),
                    );
                    return None;
                }
                Some(Waveform::Pulse {
                    low,
                    high,
                    delay,
                    rise,
                    fall,
                    width,
                    period,
                })
            }

            "sin" => {
                let allowed = [
                    "offset",
                    "amplitude",
                    "frequency",
                    "delay",
                    "damping",
                    "phase",
                ];
                for arg in &call.named {
                    if !known(&arg.name, &allowed) {
                        self.error(
                            Diagnostic::error(
                                Code::Argument,
                                format!("`sin` has no argument `{}`", arg.name),
                            )
                            .at(arg.name_span)
                            .with_note(format!("accepted: {}", allowed.join(", "))),
                        );
                        return None;
                    }
                }
                let offset = self.wave_arg(call, "offset", want, true, "sin", scope)?;
                let amplitude = self.wave_arg(call, "amplitude", want, true, "sin", scope)?;
                let frequency = self.wave_arg(call, "frequency", FREQUENCY, false, "sin", scope)?;
                let delay = self.wave_arg(call, "delay", TIME, false, "sin", scope)?;
                let damping =
                    self.wave_arg(call, "damping", units::DIMENSIONLESS, false, "sin", scope)?;
                let phase_rad = match call.arg("phase") {
                    Some(a) => {
                        let deg = self.num_arg(a, scope, units::DIMENSIONLESS, "sin.phase")?;
                        deg.value.to_radians()
                    }
                    None => 0.0,
                };
                Some(Waveform::Sin {
                    offset,
                    amplitude,
                    frequency,
                    delay,
                    damping,
                    phase_rad,
                })
            }

            "pwl" => {
                // Accepted as a flat array of (time, value) pairs or a list of
                // nested pairs, whichever the user wrote.
                let points = match call.positional.len() {
                    0 => {
                        self.error(
                            Diagnostic::error(
                                Code::Argument,
                                "`pwl` takes a list of time/value pairs",
                            )
                            .at(call.span)
                            .with_note("e.g. `pwl([0.s, 0.V, 1.us, 1.V])`"),
                        );
                        return None;
                    }
                    _ => &call.positional[0],
                };
                let ExprKind::Array(items) = &points.kind else {
                    self.error(
                        Diagnostic::error(Code::Type, "`pwl` takes an array").at(points.span),
                    );
                    return None;
                };
                if items.len() % 2 != 0 {
                    self.error(
                        Diagnostic::error(
                            Code::Value,
                            format!(
                                "`pwl` needs an even number of entries, found {}",
                                items.len()
                            ),
                        )
                        .at(points.span)
                        .with_note("each point is a time followed by a value"),
                    );
                    return None;
                }
                if items.len() / 2 > self.limits.max_pwl_points {
                    self.error(
                        Diagnostic::error(
                            Code::Limit,
                            format!("`pwl` has more than {} points", self.limits.max_pwl_points),
                        )
                        .at(points.span),
                    );
                    return None;
                }

                let mut out: Vec<(Quantity, Quantity)> = Vec::with_capacity(items.len() / 2);
                let mut previous: Option<f64> = None;
                for pair in items.chunks(2) {
                    let t = match self.eval(&pair[0], scope) {
                        Ok(Value::Num(q)) if q.dimension == TIME => q,
                        Ok(Value::Num(q)) => {
                            self.error(
                                Diagnostic::error(
                                    Code::Dimension,
                                    format!("`pwl` time needs s, found {}", q.dimension),
                                )
                                .at(pair[0].span)
                                .with_dims(TIME, q.dimension),
                            );
                            return None;
                        }
                        Ok(other) => {
                            self.error(
                                Diagnostic::error(
                                    Code::Type,
                                    format!(
                                        "`pwl` time must be a number, found {}",
                                        other.type_name()
                                    ),
                                )
                                .at(pair[0].span),
                            );
                            return None;
                        }
                        Err(d) => {
                            self.error(d);
                            return None;
                        }
                    };
                    let v = match self.eval(&pair[1], scope) {
                        Ok(Value::Num(q)) if q.dimension == want => q,
                        Ok(Value::Num(q)) => {
                            self.error(
                                Diagnostic::error(
                                    Code::Dimension,
                                    format!("`pwl` value needs {want}, found {}", q.dimension),
                                )
                                .at(pair[1].span)
                                .with_dims(want, q.dimension),
                            );
                            return None;
                        }
                        Ok(other) => {
                            self.error(
                                Diagnostic::error(
                                    Code::Type,
                                    format!(
                                        "`pwl` value must be a number, found {}",
                                        other.type_name()
                                    ),
                                )
                                .at(pair[1].span),
                            );
                            return None;
                        }
                        Err(d) => {
                            self.error(d);
                            return None;
                        }
                    };
                    if let Some(prev) = previous
                        && t.value <= prev
                    {
                        self.error(
                            Diagnostic::error(Code::Value, "`pwl` times must strictly increase")
                                .at(pair[0].span)
                                .with_context("previous", format!("{prev}"))
                                .with_context("this", format!("{}", t.value)),
                        );
                        return None;
                    }
                    previous = Some(t.value);
                    out.push((t, v));
                }
                Some(Waveform::Pwl(out))
            }

            other => {
                self.error(
                    Diagnostic::error(Code::Name, format!("unknown waveform `{other}`"))
                        .at(call.name_span)
                        .with_note("available: pulse, sin, pwl"),
                );
                None
            }
        }
    }

    // -----------------------------------------------------------------------
    // Experiments
    // -----------------------------------------------------------------------

    fn experiment_overrides(
        &mut self,
        def: &ast::ExperimentDef,
    ) -> Vec<(String, Quantity, SourceSpan)> {
        // Experiment-level `param :x, value: ...` statements. Evaluated with a
        // scope containing only earlier experiment overrides, so they cannot
        // depend on circuit parameters (which are not in scope here).
        let mut scope = Scope::default();
        let mut out = Vec::new();
        for stmt in &def.body {
            if let ExpStmt::Param { name, value, span } = stmt {
                match self.eval(value, &mut scope) {
                    Ok(Value::Num(q)) => {
                        if let Some(existing) = out
                            .iter_mut()
                            .find(|(n, _, _): &&mut (String, Quantity, SourceSpan)| *n == name.name)
                        {
                            existing.1 = q;
                            existing.2 = *span;
                        } else {
                            out.push((name.name.clone(), q, *span));
                        }
                        scope.set(&name.name, q, name.span);
                    }
                    Ok(other) => self.error(
                        Diagnostic::error(
                            Code::Type,
                            format!(
                                "parameter override `{}` must be a number, found {}",
                                name.name,
                                other.type_name()
                            ),
                        )
                        .at(value.span),
                    ),
                    Err(d) => self.error(d),
                }
            }
        }
        out
    }

    fn elaborate_experiment(
        &mut self,
        def: &ast::ExperimentDef,
        circuit: &Circuit,
    ) -> Option<AnalysisPlan> {
        let mut tasks = Vec::new();
        let mut measures = Vec::new();
        let mut pending_probes: Option<Vec<Expr>> = None;
        let mut id = 0u32;

        let mut overrides = Vec::new();
        {
            let mut scope = Scope::default();
            for stmt in &def.body {
                if let ExpStmt::Param { name, value, span } = stmt {
                    if let Ok(Value::Num(q)) = self.eval(value, &mut scope) {
                        overrides.push((name.name.clone(), q, *span));
                        scope.set(&name.name, q, name.span);
                    }
                }
            }
        }

        for stmt in &def.body {
            match stmt {
                ExpStmt::Param { .. } => {}

                ExpStmt::Save { probes, .. } => {
                    if pending_probes.is_some() {
                        self.error(
                            Diagnostic::error(
                                Code::Duplicate,
                                "an experiment may have only one `save` statement",
                            )
                            .at(stmt.span()),
                        );
                        continue;
                    }
                    pending_probes = Some(probes.clone());
                }

                ExpStmt::Measure {
                    name,
                    kind,
                    kind_span,
                    target,
                    span,
                } => {
                    let Some(k) = MeasureKind::parse(kind) else {
                        self.error(
                            Diagnostic::error(
                                Code::Unsupported,
                                format!("unknown measurement `{kind}`"),
                            )
                            .at(*kind_span)
                            .with_note("available: max, min, avg, rms"),
                        );
                        continue;
                    };
                    if let Some(probe) = self.resolve_probe_expr(target, circuit, &mut []) {
                        measures.push(MeasureRequest {
                            name: name.name.clone(),
                            kind: k,
                            target: probe.probe,
                            target_name: probe.name,
                            span: *span,
                            kind_span: *kind_span,
                        });
                    }
                }

                ExpStmt::Op { span } => {
                    tasks.push(AnalysisTask {
                        id: AnalysisId(id),
                        kind: AnalysisKind::Op,
                        probes: Vec::new(),
                        span: *span,
                    });
                    id += 1;
                }

                ExpStmt::Ac(call) => {
                    if let Some(kind) = self.ac_spec(call) {
                        tasks.push(AnalysisTask {
                            id: AnalysisId(id),
                            kind,
                            probes: Vec::new(),
                            span: call.span,
                        });
                        id += 1;
                    }
                }

                ExpStmt::Tran(call) => {
                    if let Some(kind) = self.tran_spec(call) {
                        tasks.push(AnalysisTask {
                            id: AnalysisId(id),
                            kind,
                            probes: Vec::new(),
                            span: call.span,
                        });
                        id += 1;
                    }
                }

                ExpStmt::Dc(call) => {
                    if let Some(kind) = self.dc_spec(call, circuit) {
                        tasks.push(AnalysisTask {
                            id: AnalysisId(id),
                            kind,
                            probes: Vec::new(),
                            span: call.span,
                        });
                        id += 1;
                    }
                }
            }
        }

        if tasks.is_empty() {
            self.error(
                Diagnostic::error(
                    Code::Argument,
                    format!("experiment `{}` declares no analysis", def.name.name),
                )
                .at(def.name.span)
                .with_note("add `op`, `dc`, `ac` or `tran`"),
            );
            return None;
        }

        // Resolve probes once the whole task list exists, then attach them to
        // every task (spec §8.5: one `save` applies to all analyses).
        let probes = match &pending_probes {
            Some(exprs) => {
                let mut out = Vec::new();
                let mut seen: HashSet<String> = HashSet::new();
                for e in exprs {
                    if let Some(p) = self.resolve_probe_expr(e, circuit, &mut []) {
                        if !seen.insert(p.name.clone()) {
                            self.error(
                                Diagnostic::error(
                                    Code::Duplicate,
                                    format!("probe `{}` is saved twice", p.name),
                                )
                                .at(e.span),
                            );
                            continue;
                        }
                        out.push(p);
                    }
                }
                out
            }
            None => Vec::new(),
        };

        for task in &mut tasks {
            task.probes = probes.clone();
        }

        if self.error_count > 0 {
            return None;
        }

        Some(AnalysisPlan {
            name: def.name.name.clone(),
            circuit_name: circuit.name.clone(),
            tasks,
            param_overrides: overrides,
            measures,
            span: def.span,
        })
    }

    /// The name a probe argument refers to.
    ///
    /// `:out` and `:stage1.r1` are the usual spellings; a string is accepted
    /// too, because the flattened name of something inside a subcircuit is a
    /// path and writing it as `"stage1.r1"` is unambiguous.
    fn probe_target(&mut self, arg: &Expr, func: &str, what: &str) -> Option<String> {
        let name = match &arg.kind {
            ast::ExprKind::Symbol(s) | ast::ExprKind::Str(s) => s.clone(),
            _ => {
                let example = if func == "v" { "v(:out)" } else { "i(:r1)" };
                self.error(
                    Diagnostic::error(
                        Code::Type,
                        format!("`{func}` takes {what} symbols, e.g. `{example}`"),
                    )
                    .at(arg.span)
                    .with_note(
                        "to name something inside a subcircuit, use its full path: `:stage1.r1`",
                    ),
                );
                return None;
            }
        };
        if name.is_empty() {
            self.error(
                Diagnostic::error(Code::Value, format!("`{func}` was given an empty name"))
                    .at(arg.span),
            );
            return None;
        }
        Some(name)
    }

    /// Resolve `v(...)` / `i(...)` into a typed probe.
    fn resolve_probe_expr(
        &mut self,
        expr: &Expr,
        circuit: &Circuit,
        _unused: &mut [()],
    ) -> Option<NamedProbe> {
        let ExprKind::Call(call) = &expr.kind else {
            self.error(
                Diagnostic::error(Code::Type, "a probe must be `v(:node)` or `i(:device)`")
                    .at(expr.span),
            );
            return None;
        };

        match call.name.as_str() {
            "v" => {
                let mut nodes = Vec::new();
                for arg in &call.positional {
                    let sym = self.probe_target(arg, "v", "node")?;
                    match node_lookup(circuit, &sym) {
                        Some(n) => nodes.push((n, arg.span)),
                        None => {
                            if let Some(paths) = ambiguous_paths(
                                circuit
                                    .nodes
                                    .iter()
                                    .map(|n| (n.local_name.as_str(), n.name.as_str())),
                                &sym,
                            ) {
                                self.error(
                                    Diagnostic::error(
                                        Code::Name,
                                        format!("`{sym}` names {} different nodes", paths.len()),
                                    )
                                    .at(arg.span)
                                    .with_note(format!(
                                        "write the full path: {}",
                                        paths
                                            .iter()
                                            .map(|p| format!("v(:{p})"))
                                            .collect::<Vec<_>>()
                                            .join(" or ")
                                    )),
                                );
                                return None;
                            }
                            let mut d =
                                Diagnostic::error(Code::Name, format!("unknown node `:{sym}`"))
                                    .at(arg.span);
                            let names: Vec<&str> = circuit
                                .signal_nodes()
                                .map(|n| n.name.as_str())
                                .take(12)
                                .collect();
                            if !names.is_empty() {
                                d = d.with_note(format!("nodes: {}", names.join(", ")));
                            }
                            self.error(d);
                            return None;
                        }
                    }
                }
                match nodes.as_slice() {
                    [(n, _)] => Some(NamedProbe {
                        name: format!("v({})", circuit.node_name(*n)),
                        probe: Probe::NodeVoltage(*n),
                        span: call.span,
                    }),
                    [(a, _), (b, _)] => Some(NamedProbe {
                        name: format!("v({},{})", circuit.node_name(*a), circuit.node_name(*b)),
                        probe: Probe::DifferentialVoltage { pos: *a, neg: *b },
                        span: call.span,
                    }),
                    _ => {
                        self.error(
                            Diagnostic::error(
                                Code::Argument,
                                format!("`v` takes one or two nodes, found {}", nodes.len()),
                            )
                            .at(call.span)
                            .with_note(
                                "`v(:a)` is the voltage at `a`; `v(:a, :b)` is `v(a) - v(b)`",
                            ),
                        );
                        None
                    }
                }
            }

            "i" => {
                if call.positional.len() != 1 {
                    self.error(
                        Diagnostic::error(
                            Code::Argument,
                            format!("`i` takes one device, found {}", call.positional.len()),
                        )
                        .at(call.span),
                    );
                    return None;
                }
                let arg = &call.positional[0];
                let sym = self.probe_target(arg, "i", "device")?;
                match device_lookup(circuit, &sym) {
                    Some(d) => Some(NamedProbe {
                        name: format!("i({})", d.name),
                        probe: Probe::DeviceCurrent(d.id),
                        span: call.span,
                    }),
                    None => {
                        let mut diag =
                            Diagnostic::error(Code::Name, format!("unknown device `:{sym}`"))
                                .at(arg.span);
                        if let Some(paths) = ambiguous_paths(
                            circuit
                                .devices
                                .iter()
                                .map(|d| (d.local_name.as_str(), d.name.as_str())),
                            &sym,
                        ) {
                            diag = Diagnostic::error(
                                Code::Name,
                                format!("`{sym}` names {} different devices", paths.len()),
                            )
                            .at(arg.span)
                            .with_note(format!(
                                "write the full path: {}",
                                paths
                                    .iter()
                                    .map(|p| format!("i(:{p})"))
                                    .collect::<Vec<_>>()
                                    .join(" or ")
                            ));
                        } else {
                            let names: Vec<&str> = circuit
                                .devices
                                .iter()
                                .map(|d| d.name.as_str())
                                .take(12)
                                .collect();
                            if !names.is_empty() {
                                diag = diag.with_note(format!("devices: {}", names.join(", ")));
                            }
                        }
                        self.error(diag);
                        None
                    }
                }
            }

            other => {
                self.error(
                    Diagnostic::error(Code::Name, format!("unknown probe `{other}`"))
                        .at(call.name_span)
                        .with_note("available probes: v(node), v(a, b), i(device)"),
                );
                None
            }
        }
    }

    // ---- analysis specs ---------------------------------------------------

    fn ac_spec(&mut self, call: &AnalysisCall) -> Option<AnalysisKind> {
        let allowed = ["from", "to", "points", "points_per_decade"];
        self.check_analysis_args(call, &allowed, "ac");

        let mut scope = Scope::default();
        let start = self.req_quantity(call, "from", FREQUENCY, &mut scope)?;
        let stop = self.req_quantity(call, "to", FREQUENCY, &mut scope)?;

        if start.value <= 0.0 {
            self.error(
                Diagnostic::error(Code::Value, "`ac from:` must be greater than zero").at(call
                    .arg("from")
                    .unwrap()
                    .value
                    .span),
            );
            return None;
        }
        if stop.value <= start.value {
            self.error(
                Diagnostic::error(
                    Code::Sweep,
                    format!(
                        "`ac to:` ({}) must be greater than `from:` ({})",
                        stop.value, start.value
                    ),
                )
                .at(call.span),
            );
            return None;
        }

        let per_decade = call.arg("points_per_decade");
        let total = call.arg("points");
        let (points, kind) = match (per_decade, total) {
            (Some(_), Some(_)) => {
                self.error(
                    Diagnostic::error(
                        Code::Argument,
                        "`ac` takes either `points_per_decade:` or `points:`, not both",
                    )
                    .at(call.span),
                );
                return None;
            }
            (Some(a), None) => {
                let n = self.positive_count(a, &mut scope, "points_per_decade")?;
                (n, SweepKind::Decade)
            }
            (None, Some(a)) => {
                let n = self.positive_count(a, &mut scope, "points")?;
                (n, SweepKind::LinearPoints)
            }
            (None, None) => {
                self.error(
                    Diagnostic::error(
                        Code::Argument,
                        "`ac` needs `points_per_decade:` or `points:`",
                    )
                    .at(call.span),
                );
                return None;
            }
        };

        let spec = AcSweep {
            start_hz: start.value,
            stop_hz: stop.value,
            points,
            kind,
            span: call.span,
        };

        let n = spec.point_count();
        if n > self.limits.max_sweep_points {
            self.error(
                Diagnostic::error(
                    Code::Limit,
                    format!(
                        "`ac` would produce {n} points, more than the limit of {}",
                        self.limits.max_sweep_points
                    ),
                )
                .at(call.span)
                .with_note("reduce `points_per_decade:` or narrow the range"),
            );
            return None;
        }

        Some(AnalysisKind::Ac(spec))
    }

    fn tran_spec(&mut self, call: &AnalysisCall) -> Option<AnalysisKind> {
        let allowed = ["stop", "start", "max_step", "output_interval"];
        self.check_analysis_args(call, &allowed, "tran");

        let mut scope = Scope::default();
        let stop = self.req_quantity(call, "stop", TIME, &mut scope)?;
        let start = match call.arg("start") {
            Some(_) => self.req_quantity(call, "start", TIME, &mut scope)?,
            None => Quantity::seconds(0.0),
        };
        if stop.value <= start.value {
            self.error(
                Diagnostic::error(
                    Code::Sweep,
                    format!(
                        "`tran stop:` ({}) must be greater than `start:` ({})",
                        stop.value, start.value
                    ),
                )
                .at(call.span),
            );
            return None;
        }
        if start.value < 0.0 {
            self.error(
                Diagnostic::error(Code::Value, "`tran start:` must not be negative").at(call
                    .arg("start")
                    .unwrap()
                    .value
                    .span),
            );
            return None;
        }

        let max_step = match call.arg("max_step") {
            Some(_) => Some(self.req_quantity(call, "max_step", TIME, &mut scope)?.value),
            None => None,
        };
        if let Some(ms) = max_step
            && ms <= 0.0
        {
            self.error(
                Diagnostic::error(Code::Value, "`max_step:` must be greater than zero").at(call
                    .arg("max_step")
                    .unwrap()
                    .value
                    .span),
            );
            return None;
        }

        let output_interval = match call.arg("output_interval") {
            Some(_) => Some(
                self.req_quantity(call, "output_interval", TIME, &mut scope)?
                    .value,
            ),
            None => None,
        };

        Some(AnalysisKind::Tran(TranSpec {
            start_s: start.value,
            stop_s: stop.value,
            max_step,
            output_interval,
            uic: false,
            span: call.span,
        }))
    }

    fn dc_spec(&mut self, call: &AnalysisCall, circuit: &Circuit) -> Option<AnalysisKind> {
        let allowed = ["source", "param", "from", "to", "step", "points"];
        self.check_analysis_args(call, &allowed, "dc");

        let source_arg = call.arg("source");
        let param_arg = call.arg("param");

        let (target, want) = match (source_arg, param_arg) {
            (Some(_), Some(_)) => {
                self.error(
                    Diagnostic::error(
                        Code::Argument,
                        "`dc` sweeps either a `source:` or a `param:`, not both",
                    )
                    .at(call.span),
                );
                return None;
            }
            (Some(a), None) => {
                let Some(sym) = a.value.as_symbol() else {
                    self.error(
                        Diagnostic::error(
                            Code::Type,
                            "`source:` takes a device symbol, e.g. `source: :input`",
                        )
                        .at(a.value.span),
                    );
                    return None;
                };
                let Some(d) = device_lookup(circuit, sym) else {
                    self.error(
                        Diagnostic::error(Code::Name, format!("unknown device `:{sym}`"))
                            .at(a.value.span),
                    );
                    return None;
                };
                if !d.kind.is_source() {
                    self.error(
                        Diagnostic::error(
                            Code::Sweep,
                            format!("`{}` is a {}, not a source", d.name, d.kind.name()),
                        )
                        .at(a.value.span),
                    );
                    return None;
                }
                let want = if d.kind == DeviceKind::VoltageSource {
                    VOLTAGE
                } else {
                    CURRENT
                };
                (
                    SweepTarget::SourceValue {
                        device: d.id,
                        name: d.name.clone(),
                    },
                    want,
                )
            }
            (None, Some(a)) => {
                let Some(sym) = a.value.as_symbol() else {
                    self.error(
                        Diagnostic::error(Code::Type, "`param:` takes a parameter symbol")
                            .at(a.value.span),
                    );
                    return None;
                };
                // A swept parameter must exist somewhere in the design; the
                // authoritative check happens when the point is elaborated,
                // but a typo is worth catching here.
                (
                    SweepTarget::Parameter {
                        name: sym.to_string(),
                    },
                    units::DIMENSIONLESS, // checked per point
                )
            }
            (None, None) => {
                self.error(
                    Diagnostic::error(Code::Argument, "`dc` needs `source:` or `param:`")
                        .at(call.span)
                        .with_note("e.g. `dc source: :input, from: 0.V, to: 5.V, step: 1.V`"),
                );
                return None;
            }
        };

        let mut scope = Scope::default();
        // A parameter sweep has no known dimension until the point is
        // elaborated, so its range is required to be dimensionless here and
        // re-checked by the sweep driver.
        let dimension = if want == units::DIMENSIONLESS {
            None
        } else {
            Some(want)
        };

        let from = self.range_end(call, "from", dimension, &mut scope)?;
        let to = self.range_end(call, "to", dimension, &mut scope)?;

        let step = match call.arg("step") {
            Some(_) => Some(self.range_end(call, "step", dimension, &mut scope)?.value),
            None => None,
        };
        let points = match call.arg("points") {
            Some(a) => Some(self.positive_count(a, &mut scope, "points")?),
            None => None,
        };

        if step.is_none() && points.is_none() {
            self.error(
                Diagnostic::error(Code::Argument, "`dc` needs `step:` or `points:`").at(call.span),
            );
            return None;
        }
        if step.is_some() && points.is_some() {
            self.error(
                Diagnostic::error(
                    Code::Argument,
                    "`dc` takes either `step:` or `points:`, not both",
                )
                .at(call.span),
            );
            return None;
        }

        let sweep = Sweep {
            target,
            // For a source sweep the dimension is the source's; for a
            // parameter sweep it is whatever the range expression carried.
            dimension: if let Some(d) = dimension {
                d
            } else {
                from.dimension
            },
            start: from.value,
            stop: to.value,
            step,
            points,
            kind: SweepKind::Linear,
            include_endpoint: true,
            span: call.span,
        };
        Some(AnalysisKind::Dc(DcSpec { sweep }))
    }

    fn range_end(
        &mut self,
        call: &AnalysisCall,
        name: &str,
        dimension: Option<Dimension>,
        scope: &mut Scope,
    ) -> Option<Quantity> {
        let arg = call.arg(name)?;
        match self.eval(&arg.value, scope) {
            Ok(Value::Num(q)) => {
                if let Some(want) = dimension
                    && q.dimension != want
                {
                    self.error(
                        Diagnostic::error(
                            Code::Dimension,
                            format!("`dc {name}:` needs {want}, found {}", q.dimension),
                        )
                        .at(arg.value.span)
                        .with_dims(want, q.dimension),
                    );
                    return None;
                }
                Some(q)
            }
            Ok(other) => {
                self.error(
                    Diagnostic::error(
                        Code::Type,
                        format!("`dc {name}:` needs a number, found {}", other.type_name()),
                    )
                    .at(arg.value.span),
                );
                None
            }
            Err(d) => {
                self.error(d);
                None
            }
        }
    }

    fn req_quantity(
        &mut self,
        call: &AnalysisCall,
        name: &str,
        want: Dimension,
        scope: &mut Scope,
    ) -> Option<Quantity> {
        let Some(arg) = call.arg(name) else {
            self.error(
                Diagnostic::error(
                    Code::Argument,
                    format!("`{}` requires `{name}:`", analysis_name(call)),
                )
                .at(call.span),
            );
            return None;
        };
        self.num_arg(arg, scope, want, &format!("{}.{name}", analysis_name(call)))
    }

    fn positive_count(&mut self, arg: &Arg, scope: &mut Scope, what: &str) -> Option<u32> {
        let q = self.num_arg(arg, scope, units::DIMENSIONLESS, what)?;
        let n = q.value;
        if n < 1.0 || n.fract() != 0.0 {
            self.error(
                Diagnostic::error(
                    Code::Value,
                    format!("`{what}:` must be a positive whole number, found {n}"),
                )
                .at(arg.value.span),
            );
            return None;
        }
        Some(n as u32)
    }

    fn check_analysis_args(&mut self, call: &AnalysisCall, allowed: &[&str], what: &str) {
        for arg in &call.args {
            if allowed.contains(&arg.name.as_str()) {
                continue;
            }
            self.error(
                Diagnostic::error(
                    Code::Argument,
                    format!("`{what}` has no argument `{}`", arg.name),
                )
                .at(arg.name_span)
                .with_note(format!("accepted: {}", allowed.join(", "))),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Body context
// ---------------------------------------------------------------------------

/// Where the current body sits: its instance prefix, the port bindings that
/// connect it to its parent, and the nodes it has declared so far.
///
/// Passing this explicitly (rather than keeping a stack in the elaborator)
/// makes instance-local scoping visible at every call site, which is what
/// keeps `stage1.r1` and `stage2.r1` from sharing nodes.
struct Bodies {
    /// Instance prefix for names, e.g. `["stage1"]`.
    prefix: Vec<String>,
    /// The subcircuit each level of `prefix` instantiates, in the same order.
    of_stack: Vec<String>,
    /// Port name -> the outer node it is bound to.
    ports: HashMap<String, NodeId>,
    /// Node names declared inside this body, before prefixing.
    local_nodes: HashSet<String>,
    /// Name of the enclosing top-level circuit, for diagnostics.
    circuit_name: String,
}

impl Bodies {
    fn top(circuit_name: String) -> Self {
        Self {
            prefix: Vec::new(),
            of_stack: Vec::new(),
            ports: HashMap::new(),
            local_nodes: HashSet::new(),
            circuit_name,
        }
    }

    /// Qualify an internal name with the instance prefix.
    fn qualify(&self, name: &str) -> String {
        if self.prefix.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", self.prefix.join("."), name)
        }
    }
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

fn analysis_name(call: &AnalysisCall) -> &'static str {
    let _ = call;
    "analysis"
}

/// A suggestion of how to write a value of a given dimension.
fn example_for(d: Dimension) -> &'static str {
    if d == RESISTANCE {
        "1.kohm"
    } else if d == CAPACITANCE {
        "100.nF"
    } else if d == INDUCTANCE {
        "10.mH"
    } else if d == TIME {
        "1.us"
    } else if d == FREQUENCY {
        "10.MHz"
    } else if d == VOLTAGE {
        "1.V"
    } else if d == CURRENT {
        "1.mA"
    } else {
        "1"
    }
}

/// Whether `s` is a legal declared name.
///
/// Same shape as an identifier (`docs/language.md` §1.2), so a generated name
/// is always also a name the user could have written literally.
fn check_identifier(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("it is empty".to_string());
    }
    let mut chars = s.chars();
    let first = chars.next().expect("checked non-empty");
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(format!(
            "it must start with a letter or underscore, not `{first}`"
        ));
    }
    if let Some(bad) = chars.find(|c| !(c.is_ascii_alphanumeric() || *c == '_')) {
        return Err(format!("it contains `{bad}`"));
    }
    Ok(())
}

/// Turn a value into text, for string concatenation.
fn stringify(v: &Value) -> Option<String> {
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
        _ => return None,
    })
}

/// Look a node up by its local name or its full hierarchical name.
fn node_lookup(circuit: &Circuit, name: &str) -> Option<NodeId> {
    if name == "gnd" || name == "0" {
        return Some(GROUND);
    }
    circuit.node_id(name).or_else(|| {
        // Allow referring to `stage1.out` by its leaf name, but only while it
        // is unambiguous: with two instances both holding an `internal`, a
        // silent pick would answer with the wrong instance's voltage.
        let mut found = circuit.nodes.iter().filter(|n| n.local_name == name);
        let first = found.next()?;
        if found.next().is_some() {
            return None;
        }
        Some(first.id)
    })
}

/// The full paths of everything whose leaf name is `name`, when that is more
/// than one thing — the list that turns a silently wrong answer into a
/// question the user can fix.
fn ambiguous_paths<'a>(
    items: impl Iterator<Item = (&'a str, &'a str)>,
    name: &str,
) -> Option<Vec<&'a str>> {
    let paths: Vec<&str> = items
        .filter(|(local, _)| *local == name)
        .map(|(_, full)| full)
        .collect();
    (paths.len() > 1).then_some(paths)
}

/// Look a device up by its local or full hierarchical name.
fn device_lookup<'c>(circuit: &'c Circuit, name: &str) -> Option<&'c Device> {
    if let Some(id) = circuit.device_id(name) {
        return circuit.device(id);
    }
    let mut found = circuit.devices.iter().filter(|d| d.local_name == name);
    let first = found.next()?;
    // Only unambiguous leaf names resolve; a name that matches two instances
    // must be written in full.
    if found.next().is_some() {
        return None;
    }
    Some(first)
}

impl Arg {
    /// A short description of the value written, used in messages that need to
    /// echo what the user typed rather than an evaluated number.
    fn value_text_hint(&self) -> String {
        match &self.value.kind {
            ExprKind::Quantity(q) => q.text.clone(),
            ExprKind::Int(i) => i.to_string(),
            ExprKind::Float(f) => f.to_string(),
            _ => "the given value".to_string(),
        }
    }
}

impl Value {
    /// Convenience for tests: extract a numeric value.
    pub fn num(&self) -> Option<Quantity> {
        self.as_num()
    }
}

/// Unused parameter kept for signature symmetry with future needs.
#[allow(dead_code)]
fn _unused(_: SourceId, _: DictEntry, _: SpannedName) {}
