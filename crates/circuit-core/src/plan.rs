//! The analysis plan: what to run, over which probes, along which sweep.
//!
//! A plan is produced by elaboration and consumed by a backend. It is
//! deliberately independent of the circuit *structure* (brief §7.2): the
//! circuit says what the network is, the plan says what to measure on it.
//!
//! Probes are resolved to typed references ([`NodeId`] / [`DeviceId`]) during
//! elaboration, so a backend never sees a user-written name.

use crate::id::{AnalysisId, DeviceId, NodeId};
use crate::span::SourceSpan;
use crate::units::Quantity;

/// What a saved signal refers to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Probe {
    /// Voltage of `pos` relative to ground.
    NodeVoltage(NodeId),
    /// Voltage `pos - neg`, matching the brief's definition of `v(a, b)`.
    DifferentialVoltage { pos: NodeId, neg: NodeId },
    /// Current through a device, positive in the device's `p -> n` direction.
    DeviceCurrent(DeviceId),
}

impl Probe {
    /// The nodes whose voltages this probe reads.
    ///
    /// Empty for a current probe: which nodes a branch current depends on is
    /// a property of the circuit topology, not of the probe, so the circuit
    /// IR is needed to answer it.
    pub fn voltage_nodes(&self) -> Vec<NodeId> {
        match self {
            Probe::NodeVoltage(n) => vec![*n],
            Probe::DifferentialVoltage { pos, neg } => vec![*pos, *neg],
            Probe::DeviceCurrent(_) => Vec::new(),
        }
    }
}

/// A probe together with the name the user gave it and where it was written.
#[derive(Clone, Debug)]
pub struct NamedProbe {
    /// Display name, e.g. `v(out)` or `i(r1)`.
    pub name: String,
    pub probe: Probe,
    pub span: SourceSpan,
}

// ---------------------------------------------------------------------------
// Result expressions (plan layer)
// ---------------------------------------------------------------------------

/// A resolved probe read inside a result expression.
///
/// The name is the signal name the backend is asked for and the name the
/// evaluator looks up later (`v(out)`, `v(a,b)`, `i(r1)`); the probe itself is
/// the typed identity elaboration resolved.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProbeRef {
    pub name: String,
    pub probe: Probe,
    pub span: SourceSpan,
}

impl ProbeRef {
    pub fn new(name: impl Into<String>, probe: Probe, span: SourceSpan) -> Self {
        Self {
            name: name.into(),
            probe,
            span,
        }
    }

    /// The dimension a probe of this kind reads.
    pub fn dimension(&self) -> crate::units::Dimension {
        match self.probe {
            Probe::NodeVoltage(_) | Probe::DifferentialVoltage { .. } => crate::units::VOLTAGE,
            Probe::DeviceCurrent(_) => crate::units::CURRENT,
        }
    }
}

/// A result expression as the plan layer describes it: resolved probes,
/// dimensionless literals and the operators the evaluator supports.
///
/// This is deliberately *not* the front-end AST: elaboration lowers a written
/// expression into this form, and `circuit-results` lowers it again into its
/// runtime AST. No parser type reaches the backend, and `circuit-core` never
/// depends on `circuit-results`.
#[derive(Clone, PartialEq, Debug)]
pub enum ExprIr {
    /// A dimensionless literal.
    Number(f64),
    /// A probe read; the signal name is the one the backend produces.
    Probe(ProbeRef),
    Neg(Box<ExprIr>),
    Add(Box<ExprIr>, Box<ExprIr>),
    Sub(Box<ExprIr>, Box<ExprIr>),
    Mul(Box<ExprIr>, Box<ExprIr>),
    Div(Box<ExprIr>, Box<ExprIr>),
    /// Absolute value (magnitude for complex data), keeping the unit.
    Abs(Box<ExprIr>),
    /// Square root; every dimension exponent must be even.
    Sqrt(Box<ExprIr>),
    Min(Box<ExprIr>, Box<ExprIr>),
    Max(Box<ExprIr>, Box<ExprIr>),
    /// `20*log10(abs(numerator / denominator))`; both sides share a dimension.
    GainDb {
        numerator: Box<ExprIr>,
        denominator: Box<ExprIr>,
    },
}

impl ExprIr {
    /// The span the expression was written at, for diagnostics.
    pub fn span(&self) -> SourceSpan {
        match self {
            Self::Number(_) => SourceSpan::synthetic(),
            Self::Probe(p) => p.span,
            Self::Neg(x) | Self::Abs(x) | Self::Sqrt(x) => x.span(),
            Self::Add(a, b)
            | Self::Sub(a, b)
            | Self::Mul(a, b)
            | Self::Div(a, b)
            | Self::Min(a, b)
            | Self::Max(a, b) => a.span().merge(b.span()),
            Self::GainDb {
                numerator,
                denominator,
            } => numerator.span().merge(denominator.span()),
        }
    }

    /// Every probe this expression reads, distinct by name, in first-seen order.
    ///
    /// This is the automatic dependency set: an expression needs no `save`
    /// statement to be evaluable, and the collected probes are read from the
    /// backend without becoming part of the exported signal set.
    pub fn probes(&self) -> Vec<ProbeRef> {
        let mut out: Vec<ProbeRef> = Vec::new();
        self.collect_probes(&mut out);
        out
    }

    fn collect_probes(&self, out: &mut Vec<ProbeRef>) {
        match self {
            Self::Number(_) => {}
            Self::Probe(p) => {
                if !out.iter().any(|q| q.name == p.name) {
                    out.push(p.clone());
                }
            }
            Self::Neg(x) | Self::Abs(x) | Self::Sqrt(x) => x.collect_probes(out),
            Self::Add(a, b)
            | Self::Sub(a, b)
            | Self::Mul(a, b)
            | Self::Div(a, b)
            | Self::Min(a, b)
            | Self::Max(a, b) => {
                a.collect_probes(out);
                b.collect_probes(out);
            }
            Self::GainDb {
                numerator,
                denominator,
            } => {
                numerator.collect_probes(out);
                denominator.collect_probes(out);
            }
        }
    }

    /// True when the expression is exactly one probe read.
    ///
    /// Used for the documented legacy selection rule: a `measure` over a plain
    /// probe without `analysis:` keeps searching analyses, while a compound
    /// expression must be bound explicitly in a multi-analysis experiment.
    pub fn is_plain_probe(&self) -> bool {
        matches!(self, Self::Probe(_))
    }

    /// The dimension of the result when it is statically known.
    ///
    /// `None` means "not provable from the written expression" (an
    /// inconsistent combination is reported separately by
    /// [`ExprIr::static_dimension_error`]), never "no dimension".
    pub fn static_dimension(&self) -> Option<crate::units::Dimension> {
        use crate::units::DIMENSIONLESS;
        match self {
            Self::Number(_) => Some(DIMENSIONLESS),
            Self::Probe(p) => Some(p.dimension()),
            Self::Neg(x) | Self::Abs(x) => x.static_dimension(),
            Self::Sqrt(x) => half_dimension(x.static_dimension()?),
            Self::Add(a, b) | Self::Sub(a, b) | Self::Min(a, b) | Self::Max(a, b) => {
                let (x, y) = (a.static_dimension()?, b.static_dimension()?);
                (x == y).then_some(x)
            }
            // A product whose exponents leave the `i8` range has no known
            // dimension either; it is reported by `static_dimension_error`.
            Self::Mul(a, b) => a.static_dimension()?.checked_mul(b.static_dimension()?),
            Self::Div(a, b) => a.static_dimension()?.checked_div(b.static_dimension()?),
            Self::GainDb {
                numerator,
                denominator,
            } => {
                let (x, y) = (
                    numerator.static_dimension()?,
                    denominator.static_dimension()?,
                );
                (x == y).then_some(DIMENSIONLESS)
            }
        }
    }

    /// A message when the written expression is dimensionally inconsistent
    /// *without running anything*, so `cdsl check` can reject it.
    pub fn static_dimension_error(&self) -> Option<String> {
        // A dimension that does not fit is checked first: it is a fact about
        // the written units, not about how they combine at run time, and it is
        // what `cdsl check` must reject (round-4 R4-02).
        if let Some(message) = self.exponent_overflow_error() {
            return Some(message);
        }
        let binary = |op: &str, a: &ExprIr, b: &ExprIr| -> Option<String> {
            let (x, y) = (a.static_dimension()?, b.static_dimension()?);
            (x != y).then(|| {
                format!(
                    "cannot apply `{op}` to `{}` ({x}) and `{}` ({y}): the units differ",
                    a.render(),
                    b.render()
                )
            })
        };
        match self {
            Self::Number(_) | Self::Probe(_) => None,
            Self::Neg(x) | Self::Abs(x) => x.static_dimension_error(),
            Self::Sqrt(x) => {
                if let Some(message) = x.static_dimension_error() {
                    return Some(message);
                }
                match x.static_dimension() {
                    Some(d) if half_dimension(d).is_none() => Some(format!(
                        "sqrt of a quantity in {d} is not representable: every dimension exponent must be even"
                    )),
                    _ => None,
                }
            }
            Self::Add(a, b) => binary("+", a, b).or_else(|| {
                a.static_dimension_error()
                    .or_else(|| b.static_dimension_error())
            }),
            Self::Sub(a, b) => binary("-", a, b).or_else(|| {
                a.static_dimension_error()
                    .or_else(|| b.static_dimension_error())
            }),
            Self::Mul(a, b) => a
                .static_dimension_error()
                .or_else(|| b.static_dimension_error()),
            Self::Div(a, b) => a
                .static_dimension_error()
                .or_else(|| b.static_dimension_error()),
            Self::Min(a, b) => binary("min", a, b).or_else(|| {
                a.static_dimension_error()
                    .or_else(|| b.static_dimension_error())
            }),
            Self::Max(a, b) => binary("max", a, b).or_else(|| {
                a.static_dimension_error()
                    .or_else(|| b.static_dimension_error())
            }),
            Self::GainDb {
                numerator,
                denominator,
            } => {
                if let Some(message) = numerator
                    .static_dimension_error()
                    .or_else(|| denominator.static_dimension_error())
                {
                    return Some(message);
                }
                match (numerator.static_dimension(), denominator.static_dimension()) {
                    (Some(x), Some(y)) if x != y => Some(format!(
                        "gain `{} / {}` must be a ratio of like quantities, but the units are {x} and {y}",
                        numerator.render(),
                        denominator.render()
                    )),
                    _ => None,
                }
            }
        }
    }

    /// A short rendering of an operand, for the overflow message.
    ///
    /// A user can write an expression with hundreds of factors (the round-4
    /// reproduction has 128), and pasting the whole tree into a diagnostic
    /// buries the one fact that matters. The full source line is printed by the
    /// CLI's span rendering anyway.
    fn brief(expr: &ExprIr) -> String {
        const LIMIT: usize = 48;
        let text = expr.render();
        if text.chars().count() <= LIMIT {
            return text;
        }
        let head: String = text.chars().take(LIMIT).collect();
        format!("{head}... ({} characters)", text.chars().count())
    }

    /// The first product or quotient in the tree whose exponents leave the
    /// representable range, rendered for the user.
    ///
    /// Children are visited before their parent so the message points at the
    /// *first* operation that cannot be represented, which is the one the user
    /// has to shorten. `static_dimension` returns `None` for such a tree
    /// instead of wrapping the exponent (release) or panicking (debug).
    fn exponent_overflow_error(&self) -> Option<String> {
        use crate::units::Dimension;
        let child = |a: &ExprIr, b: &ExprIr| {
            a.exponent_overflow_error()
                .or_else(|| b.exponent_overflow_error())
        };
        let overflow = |op: &str, a: &ExprIr, b: &ExprIr| -> Option<String> {
            let (x, y) = (a.static_dimension()?, b.static_dimension()?);
            let fits = if op == "*" {
                x.checked_mul(y).is_some()
            } else {
                x.checked_div(y).is_some()
            };
            (!fits).then(|| {
                let (low, high) = (Dimension::MIN_EXPONENT, Dimension::MAX_EXPONENT);
                format!(
                    "the dimension of `({} {op} {})` is {x} {op} {y}, which leaves the representable \
                     exponent range (an exponent is held as a signed 8-bit integer, {low}..={high})",
                    ExprIr::brief(a),
                    ExprIr::brief(b)
                )
            })
        };
        match self {
            Self::Number(_) | Self::Probe(_) => None,
            Self::Neg(x) | Self::Abs(x) | Self::Sqrt(x) => x.exponent_overflow_error(),
            Self::Add(a, b) | Self::Sub(a, b) | Self::Min(a, b) | Self::Max(a, b) => child(a, b),
            Self::Mul(a, b) => child(a, b).or_else(|| overflow("*", a, b)),
            Self::Div(a, b) => child(a, b).or_else(|| overflow("/", a, b)),
            Self::GainDb {
                numerator,
                denominator,
            } => child(numerator, denominator),
        }
    }

    /// True when every sample this expression can produce is real.
    ///
    /// A bare probe is *not* known real: an AC analysis reports complex
    /// samples, and the language has no implicit magnitude ordering.
    pub fn is_statically_real(&self) -> bool {
        match self {
            Self::Number(_) => true,
            Self::Probe(_) => false,
            // abs() returns the magnitude and gain_db() the dB value: both real.
            Self::Abs(_) | Self::GainDb { .. } => true,
            Self::Neg(x) | Self::Sqrt(x) => x.is_statically_real(),
            Self::Add(a, b)
            | Self::Sub(a, b)
            | Self::Mul(a, b)
            | Self::Div(a, b)
            | Self::Min(a, b)
            | Self::Max(a, b) => a.is_statically_real() && b.is_statically_real(),
        }
    }

    /// The expression as the user would read it, for diagnostics.
    pub fn render(&self) -> String {
        match self {
            Self::Number(x) => crate::format_number(*x),
            Self::Probe(p) => p.name.clone(),
            Self::Neg(x) => format!("-{}", x.render()),
            Self::Add(a, b) => format!("({} + {})", a.render(), b.render()),
            Self::Sub(a, b) => format!("({} - {})", a.render(), b.render()),
            Self::Mul(a, b) => format!("({} * {})", a.render(), b.render()),
            Self::Div(a, b) => format!("({} / {})", a.render(), b.render()),
            Self::Abs(x) => format!("abs({})", x.render()),
            Self::Sqrt(x) => format!("sqrt({})", x.render()),
            Self::Min(a, b) => format!("min({}, {})", a.render(), b.render()),
            Self::Max(a, b) => format!("max({}, {})", a.render(), b.render()),
            Self::GainDb {
                numerator,
                denominator,
            } => format!("gain_db({}, {})", numerator.render(), denominator.render()),
        }
    }
}

/// A dimension whose exponents are all even, halved; `None` otherwise.
fn half_dimension(d: crate::units::Dimension) -> Option<crate::units::Dimension> {
    let half = |e: i8| if e % 2 == 0 { Some(e / 2) } else { None };
    Some(crate::units::Dimension::new(
        half(d.volt)?,
        half(d.amp)?,
        half(d.second)?,
    ))
}

// ---------------------------------------------------------------------------
// Sweeps
// ---------------------------------------------------------------------------

/// How a sweep's points are generated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SweepKind {
    /// Fixed step from start to stop.
    Linear,
    /// `points` per decade, logarithmic.
    Decade,
    /// `points` per octave, logarithmic.
    Octave,
    /// `points` total, evenly spaced (used for frequency sweeps with a
    /// linear scale).
    LinearPoints,
}

/// What is being swept.
#[derive(Clone, Debug)]
pub enum SweepTarget {
    /// Source value, e.g. `dc from: 0.V, to: 5.V, step: 1.V` on a specific
    /// source. Reserved for DC sweeps of a source value.
    SourceValue { device: DeviceId, name: String },
    /// A non-topology parameter, swept over the experiment.
    Parameter { name: String },
}

/// A one-dimensional sweep axis.
///
/// Multi-dimensional sweeps are not implemented in this version; the type is
/// a single axis rather than a `Vec` so that a plan cannot silently claim
/// capability the elaborator does not have.
#[derive(Clone, Debug)]
pub struct Sweep {
    pub target: SweepTarget,
    pub start: f64,
    pub stop: f64,
    /// Dimension of the swept quantity, so a parameter sweep can rebuild a
    /// typed value for each point instead of guessing one.
    pub dimension: crate::units::Dimension,
    /// Step, for `Linear`.
    pub step: Option<f64>,
    /// Points per decade/octave, or total points for `LinearPoints`.
    pub points: Option<u32>,
    pub kind: SweepKind,
    /// Whether `stop` is included when it does not land exactly on a step.
    pub include_endpoint: bool,
    pub span: SourceSpan,
}

// ---------------------------------------------------------------------------
// Analysis specifications
// ---------------------------------------------------------------------------

/// A DC sweep specification.
#[derive(Clone, Debug)]
pub struct DcSpec {
    pub sweep: Sweep,
}

/// An AC small-signal specification.
///
/// `phase_rad` is stored in radians internally; the DSL accepts degrees and
/// converts at the boundary. Phase is always reported in the principal range
/// unless the exporter is explicitly asked to unwrap it.
#[derive(Clone, Debug)]
pub struct AcSweep {
    pub start_hz: f64,
    pub stop_hz: f64,
    pub points: u32,
    pub kind: SweepKind,
    pub span: SourceSpan,
}

impl AcSweep {
    /// Number of points this sweep will produce, computed the same way the
    /// backend does, so the front end can enforce result-size limits before
    /// running anything.
    pub fn point_count(&self) -> u64 {
        if self.stop_hz <= self.start_hz || self.start_hz <= 0.0 {
            return 0;
        }
        let decades = (self.stop_hz / self.start_hz).log10();
        match self.kind {
            // Points per decade/octave counts intervals, so the point count
            // is one more than the number of intervals.
            SweepKind::Decade => (decades * self.points as f64).round() as u64 + 1,
            SweepKind::Octave => {
                ((self.stop_hz / self.start_hz).log2() * self.points as f64).round() as u64 + 1
            }
            // A linear sweep's `points` is the total number of samples, which
            // the engine honours exactly. Verified against the engine in
            // `circuit-backend/tests/adapter.rs::two_analyses_of_the_same_kind_get_distinct_names`.
            SweepKind::LinearPoints | SweepKind::Linear => self.points as u64,
        }
    }
}

/// A transient specification.
///
/// Three concepts are deliberately kept apart here (brief §8.4); conflating
/// them is what let a coarse output request silently rewrite a declared
/// source waveform in an earlier revision:
///
/// - **`max_step`** bounds the *integration* step. It is handed to the engine
///   as its step bound and never promises anything about the returned time
///   axis, which stays solver-chosen and generally non-uniform.
/// - **`output_interval`** requests *output sampling*. It is **not** a solver
///   parameter: the run is executed with the solver's own steps and the
///   returned trace is resampled afterwards (in `circuit-results`, which is
///   the layer above this one). `None` keeps the solver's own grid.
/// - The **source waveform** is whatever the circuit declares. No analysis
///   option may widen a declared edge: the backend picks a print step that
///   cannot clamp `rise`/`fall`/`period` (see
///   `circuit_backend::thevenin::print_step_for`).
#[derive(Clone, Debug)]
pub struct TranSpec {
    pub start_s: f64,
    pub stop_s: f64,
    pub max_step: Option<f64>,
    /// Requested output interval in seconds: a finite value greater than zero,
    /// or `None` for "whatever the solver produced". Validated at the input
    /// layer, and later used only to resample the delivered trace — never to
    /// choose an integration step.
    pub output_interval: Option<f64>,
    /// Use initial conditions instead of a DC operating point. Not exposed by
    /// the language yet; the backend path is unverified (see
    /// `docs/backend-evaluation.md`).
    pub uic: bool,
    pub span: SourceSpan,
}

/// Which analysis to run.
#[derive(Clone, Debug)]
pub enum AnalysisKind {
    Op,
    Dc(DcSpec),
    Ac(AcSweep),
    Tran(TranSpec),
}

impl AnalysisKind {
    pub fn name(&self) -> &'static str {
        match self {
            AnalysisKind::Op => "op",
            AnalysisKind::Dc(_) => "dc",
            AnalysisKind::Ac(_) => "ac",
            AnalysisKind::Tran(_) => "tran",
        }
    }
}

/// One analysis task: what to run and which signals to read.
#[derive(Clone, Debug)]
pub struct AnalysisTask {
    pub id: AnalysisId,
    pub kind: AnalysisKind,
    /// The `save` probes the user asked for. Empty keeps the backend
    /// default (every signal the engine produced), exactly as before.
    pub probes: Vec<NamedProbe>,
    /// Probes a result expression reads but the user did not ask to export.
    /// They are read from the backend and never enter the exported signal set;
    /// the backend falls back to its default set and adds these to it.
    pub implicit_probes: Vec<NamedProbe>,
    pub span: SourceSpan,
}

impl AnalysisTask {
    /// The probes the backend must read: the exported (or default) set plus the
    /// expression dependencies, distinct by name, explicit ones first.
    pub fn read_probes(&self) -> Vec<NamedProbe> {
        let mut out = self.probes.clone();
        for probe in &self.implicit_probes {
            if !out.iter().any(|p| p.name == probe.name) {
                out.push(probe.clone());
            }
        }
        out
    }

    /// The signal names the user asked to see. `None` means "whatever the
    /// backend returned", which is the pre-existing behaviour without `save`.
    pub fn exported_names(&self) -> Option<Vec<String>> {
        if self.probes.is_empty() {
            None
        } else {
            Some(self.probes.iter().map(|p| p.name.clone()).collect())
        }
    }
}

/// Which reduction a `measure` statement asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MeasureKind {
    Max,
    Min,
    /// Time integral of the signal divided by the elapsed time.
    Avg,
    /// Square root of the time integral of the square, over the elapsed time.
    Rms,
}

impl MeasureKind {
    pub fn name(self) -> &'static str {
        match self {
            MeasureKind::Max => "max",
            MeasureKind::Min => "min",
            MeasureKind::Avg => "avg",
            MeasureKind::Rms => "rms",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "max" => MeasureKind::Max,
            "min" => MeasureKind::Min,
            "avg" => MeasureKind::Avg,
            "rms" => MeasureKind::Rms,
            _ => return None,
        })
    }

    /// Whether this reduction needs a time axis.
    ///
    /// `avg` and `rms` are integrals over time, so an analysis with no time
    /// axis (OP, DC, AC) cannot support them (brief §8.5).
    pub fn needs_time_axis(self) -> bool {
        matches!(self, MeasureKind::Avg | MeasureKind::Rms)
    }
}

/// Which analysis a derive or measure expression is evaluated against.
///
/// The frozen rules (docs/review-evidence/round3/design-contract.md) are:
///
/// - an explicit `analysis: :ac1` binds to that analysis identity;
/// - with exactly one analysis, a new expression may omit the binding;
/// - with several analyses, a new expression must be bound explicitly;
/// - a measure over a bare probe with no binding keeps the documented legacy
///   TRAN -> AC -> DC -> OP search order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AnalysisBinding {
    /// Evaluate against exactly this analysis.
    Analysis(AnalysisId),
    /// The legacy rule: pick the first analysis, in the documented order, that
    /// has the signal and can support the reduction.
    LegacyPreferred,
}

/// A `derive :name, expr: ...` request: a named signal computed from a
/// result expression and exported as a column of the analysis it binds to.
#[derive(Clone, Debug)]
pub struct DeriveRequest {
    pub name: String,
    pub expr: ExprIr,
    pub binding: AnalysisBinding,
    /// The expression as written, for reporting and for the export metadata.
    pub source: String,
    pub span: SourceSpan,
    pub name_span: SourceSpan,
}

/// A `measure :name, <kind>: <expression>` request.
#[derive(Clone, Debug)]
pub struct MeasureRequest {
    pub name: String,
    pub kind: MeasureKind,
    /// The expression to reduce. A bare `ExprIr::Probe` is the historical
    /// `measure :vmax, max: v(:out)` form and keeps the legacy binding rule
    /// when no `analysis:` is written.
    pub expr: ExprIr,
    pub binding: AnalysisBinding,
    /// The expression as written, for reporting.
    pub source: String,
    pub span: SourceSpan,
    pub kind_span: SourceSpan,
}

/// A complete experiment: a circuit plus the analyses to run on it.
#[derive(Clone, Debug)]
pub struct AnalysisPlan {
    pub name: String,
    /// Name of the circuit definition this plan runs against.
    pub circuit_name: String,
    pub tasks: Vec<AnalysisTask>,
    /// Parameter overrides declared on the experiment itself, applied after
    /// subcircuit defaults and before sweep points (brief §5.3).
    pub param_overrides: Vec<(String, Quantity, SourceSpan)>,
    /// Named derived signals to compute and export.
    pub derives: Vec<DeriveRequest>,
    /// Measurements to evaluate against the results.
    pub measures: Vec<MeasureRequest>,
    pub span: SourceSpan,
}

impl AnalysisPlan {
    pub fn task(&self, id: AnalysisId) -> Option<&AnalysisTask> {
        self.tasks.iter().find(|t| t.id == id)
    }

    /// The analysis identity the backend stamps on a task result:
    /// `{kind}{ordinal}` with a 1-based per-kind ordinal in declaration
    /// order (thevenin.rs). This is also what `analysis: :ac1` names.
    pub fn result_name(&self, id: AnalysisId) -> Option<String> {
        let mut per_kind: Vec<(&'static str, u32)> = Vec::new();
        for task in &self.tasks {
            let kind = task.kind.name();
            let ordinal = match per_kind.iter_mut().find(|(k, _)| *k == kind) {
                Some((_, n)) => {
                    *n += 1;
                    *n
                }
                None => {
                    per_kind.push((kind, 1));
                    1
                }
            };
            if task.id == id {
                return Some(format!("{kind}{ordinal}"));
            }
        }
        None
    }

    /// Resolve a written analysis identity (`ac1`) to a task.
    pub fn analysis_id_by_name(&self, name: &str) -> Option<AnalysisId> {
        self.tasks
            .iter()
            .find(|t| self.result_name(t.id).as_deref() == Some(name))
            .map(|t| t.id)
    }

    /// Every analysis identity, in plan order, for diagnostics.
    pub fn analysis_names(&self) -> Vec<String> {
        self.tasks
            .iter()
            .filter_map(|t| self.result_name(t.id))
            .collect()
    }

    /// `Some((parameter, sweep))` when the plan runs a DC parameter sweep.
    ///
    /// A parameter sweep re-elaborates and re-runs the design once per point,
    /// so the whole experiment produces exactly one stitched dataset for that
    /// single analysis; callers use this to refuse bindings it cannot honour.
    pub fn parameter_sweep(&self) -> Option<(String, Sweep)> {
        let mut found = None;
        for task in &self.tasks {
            if let AnalysisKind::Dc(spec) = &task.kind
                && let SweepTarget::Parameter { name } = &spec.sweep.target
            {
                found = Some((name.clone(), spec.sweep.clone()));
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ac(start: f64, stop: f64, points: u32, kind: SweepKind) -> AcSweep {
        AcSweep {
            start_hz: start,
            stop_hz: stop,
            points,
            kind,
            span: SourceSpan::synthetic(),
        }
    }

    #[test]
    fn decade_point_count_matches_ngspice_convention() {
        // 2 decades at 10 points/decade -> 20 intervals -> 21 points.
        assert_eq!(ac(10.0, 1000.0, 10, SweepKind::Decade).point_count(), 21);
        // 6 decades at 10/decade -> 61 points, matching the phase-0 probe.
        assert_eq!(ac(10.0, 1e7, 10, SweepKind::Decade).point_count(), 61);
    }

    #[test]
    fn linear_point_count_is_total_points() {
        // `points: n` on a linear sweep means n samples in total, which is
        // what the engine produces.
        assert_eq!(
            ac(100.0, 1000.0, 50, SweepKind::LinearPoints).point_count(),
            50
        );
        assert_eq!(
            ac(100.0, 1000.0, 11, SweepKind::LinearPoints).point_count(),
            11
        );
    }

    #[test]
    fn degenerate_ranges_report_zero() {
        assert_eq!(ac(1000.0, 10.0, 10, SweepKind::Decade).point_count(), 0);
        assert_eq!(ac(0.0, 10.0, 10, SweepKind::Decade).point_count(), 0);
    }

    fn probe_ref(name: &str, probe: Probe) -> ProbeRef {
        ProbeRef::new(name, probe, SourceSpan::synthetic())
    }

    fn voltage(name: &str) -> ExprIr {
        ExprIr::Probe(probe_ref(name, Probe::NodeVoltage(NodeId(1))))
    }

    #[test]
    fn expression_probe_set_is_deduplicated_in_first_seen_order() {
        // v(out)/v(in) * v(out) reads each probe once, in the order written.
        let expr = ExprIr::Mul(
            Box::new(ExprIr::Div(
                Box::new(voltage("v(out)")),
                Box::new(voltage("v(in)")),
            )),
            Box::new(voltage("v(out)")),
        );
        let names: Vec<String> = expr.probes().into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["v(out)".to_string(), "v(in)".to_string()]);
    }

    #[test]
    fn a_gain_ratio_of_two_voltages_is_dimensionless() {
        use crate::units::{CURRENT, DIMENSIONLESS, VOLTAGE};
        let gain = ExprIr::Div(Box::new(voltage("v(out)")), Box::new(voltage("v(in)")));
        assert_eq!(gain.static_dimension(), Some(DIMENSIONLESS));
        assert_eq!(gain.static_dimension_error(), None);

        // v(out) / i(r1) is an impedance, and dB of it is not defined.
        let current = ExprIr::Probe(probe_ref("i(r1)", Probe::DeviceCurrent(DeviceId(0))));
        let mixed = ExprIr::GainDb {
            numerator: Box::new(voltage("v(out)")),
            denominator: Box::new(current),
        };
        let message = mixed.static_dimension_error().expect("units differ");
        assert!(message.contains("ratio of like quantities"), "{message}");
        assert_eq!(mixed.static_dimension(), None);
        assert_eq!(voltage("v(out)").static_dimension(), Some(VOLTAGE));
        let _ = CURRENT;
    }

    #[test]
    fn adding_volts_and_amps_is_a_static_dimension_error() {
        let sum = ExprIr::Add(
            Box::new(voltage("v(out)")),
            Box::new(ExprIr::Probe(probe_ref(
                "i(r1)",
                Probe::DeviceCurrent(DeviceId(0)),
            ))),
        );
        let message = sum.static_dimension_error().expect("units differ");
        assert!(message.contains("cannot apply"), "{message}");
    }

    /// The round-4 R4-02 reproduction: 128 voltage factors overflow an `i8`
    /// exponent. The static analysis must say so — `cdsl check` rejects it —
    /// and must never panic or wrap.
    #[test]
    fn a_dimension_that_does_not_fit_is_a_static_error() {
        // 1 + 126 factors is exactly V^127; the next one cannot be held.
        let mut product = voltage("v(vin)");
        for _ in 0..126 {
            product = ExprIr::Mul(Box::new(product), Box::new(voltage("v(vin)")));
        }
        // V^127 is the last representable exponent.
        assert_eq!(
            product.static_dimension(),
            Some(crate::units::Dimension::new(127, 0, 0))
        );
        assert_eq!(product.static_dimension_error(), None);

        let overflowing = ExprIr::Mul(Box::new(product), Box::new(voltage("v(vin)")));
        assert_eq!(overflowing.static_dimension(), None);
        let message = overflowing
            .static_dimension_error()
            .expect("V^128 does not fit");
        assert!(
            message.contains("leaves the representable exponent range"),
            "{message}"
        );
    }

    #[test]
    fn only_abs_and_gain_db_are_statically_real() {
        // A bare probe may be complex (an AC analysis), so a reduction over it
        // must be rejected statically; abs(...) makes the value real.
        assert!(!voltage("v(out)").is_statically_real());
        assert!(ExprIr::Abs(Box::new(voltage("v(out)"))).is_statically_real());
        assert!(
            ExprIr::Neg(Box::new(ExprIr::Abs(Box::new(voltage("v(out)"))))).is_statically_real()
        );
        assert!(!ExprIr::Neg(Box::new(voltage("v(out)"))).is_statically_real());
    }

    fn task(id: u32, kind: AnalysisKind) -> AnalysisTask {
        AnalysisTask {
            id: AnalysisId(id),
            kind,
            probes: Vec::new(),
            implicit_probes: Vec::new(),
            span: SourceSpan::synthetic(),
        }
    }

    fn plan_with_tasks(tasks: Vec<AnalysisTask>) -> AnalysisPlan {
        AnalysisPlan {
            name: "e".into(),
            circuit_name: "c".into(),
            tasks,
            param_overrides: Vec::new(),
            derives: Vec::new(),
            measures: Vec::new(),
            span: SourceSpan::synthetic(),
        }
    }

    fn parameter_sweep_kind() -> AnalysisKind {
        AnalysisKind::Dc(DcSpec {
            sweep: Sweep {
                target: SweepTarget::Parameter { name: "r".into() },
                dimension: crate::units::RESISTANCE,
                start: 1.0,
                stop: 2.0,
                step: Some(1.0),
                points: None,
                kind: SweepKind::Linear,
                include_endpoint: true,
                span: SourceSpan::synthetic(),
            },
        })
    }

    #[test]
    fn result_names_follow_per_kind_ordinals() {
        // Two ACs and an op: identities are per-kind ordinals in plan order,
        // which is exactly what the backend stamps on each dataset.
        let ac = || {
            AnalysisKind::Ac(AcSweep {
                start_hz: 1.0,
                stop_hz: 10.0,
                points: 2,
                kind: SweepKind::Decade,
                span: SourceSpan::synthetic(),
            })
        };
        let plan = plan_with_tasks(vec![
            task(0, ac()),
            task(1, AnalysisKind::Op),
            task(2, ac()),
        ]);
        assert_eq!(plan.result_name(AnalysisId(0)).as_deref(), Some("ac1"));
        assert_eq!(plan.result_name(AnalysisId(1)).as_deref(), Some("op1"));
        assert_eq!(plan.result_name(AnalysisId(2)).as_deref(), Some("ac2"));
        assert_eq!(
            plan.analysis_names(),
            vec!["ac1".to_string(), "op1".to_string(), "ac2".to_string()]
        );
        assert_eq!(plan.analysis_id_by_name("ac2"), Some(AnalysisId(2)));
        assert_eq!(plan.analysis_id_by_name("tran1"), None);
        assert_eq!(plan.analysis_id_by_name("ac"), None, "no bare kind names");
        assert!(plan.parameter_sweep().is_none());
    }

    #[test]
    fn a_parameter_sweep_is_detected_and_a_source_sweep_is_not() {
        let sweeping = plan_with_tasks(vec![task(0, parameter_sweep_kind())]);
        let (name, sweep) = sweeping.parameter_sweep().expect("parameter sweep");
        assert_eq!(name, "r");
        assert_eq!(sweep.start, 1.0);

        let source = AnalysisKind::Dc(DcSpec {
            sweep: Sweep {
                target: SweepTarget::SourceValue {
                    device: DeviceId(0),
                    name: "v1".into(),
                },
                dimension: crate::units::VOLTAGE,
                start: 0.0,
                stop: 5.0,
                step: Some(1.0),
                points: None,
                kind: SweepKind::Linear,
                include_endpoint: true,
                span: SourceSpan::synthetic(),
            },
        });
        let native = plan_with_tasks(vec![task(0, source)]);
        assert!(native.parameter_sweep().is_none());
    }

    #[test]
    fn implicit_probes_are_read_but_not_exported() {
        let mut task = AnalysisTask {
            id: AnalysisId(0),
            kind: AnalysisKind::Op,
            probes: vec![NamedProbe {
                name: "v(in)".into(),
                probe: Probe::NodeVoltage(NodeId(1)),
                span: SourceSpan::synthetic(),
            }],
            implicit_probes: vec![NamedProbe {
                name: "v(out)".into(),
                probe: Probe::NodeVoltage(NodeId(2)),
                span: SourceSpan::synthetic(),
            }],
            span: SourceSpan::synthetic(),
        };
        let read: Vec<String> = task.read_probes().into_iter().map(|p| p.name).collect();
        assert_eq!(read, vec!["v(in)".to_string(), "v(out)".to_string()]);
        assert_eq!(task.exported_names(), Some(vec!["v(in)".to_string()]));

        // No save statement: the backend keeps its default set, and nothing is
        // excluded from the export.
        task.probes.clear();
        assert_eq!(task.exported_names(), None);
        // The implicit probe is still readable, and de-duplicated by name.
        task.implicit_probes.push(task.implicit_probes[0].clone());
        let read: Vec<String> = task.read_probes().into_iter().map(|p| p.name).collect();
        assert_eq!(read, vec!["v(out)".to_string()]);
    }

    #[test]
    fn differential_probe_touches_both_nodes() {
        let p = Probe::DifferentialVoltage {
            pos: NodeId(1),
            neg: NodeId(2),
        };
        assert_eq!(p.voltage_nodes(), vec![NodeId(1), NodeId(2)]);
        assert!(Probe::DeviceCurrent(DeviceId(0)).voltage_nodes().is_empty());
    }
}
