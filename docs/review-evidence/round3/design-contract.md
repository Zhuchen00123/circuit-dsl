# Round 3 design contract (frozen 2026-xx, lead-owned)

Scope: result expressions, derived signals, power/gain measurement, DSL+CLI+REPL+export.
Out of scope this round: parameter DAG, new devices, GUI/LSP, backend replacement, second expression engine.

## 1. Frozen DSL syntax

```ruby
experiment :response, circuit: :rc_filter do
  ac from: 100.Hz, to: 100.kHz, points_per_decade: 40

  derive :gain,    expr: v(:vout) / v(:vin)
  derive :gain_db, expr: gain_db(v(:vout), v(:vin))
  measure :peak_gain, max: abs(v(:vout) / v(:vin))
end

experiment :power, circuit: :rc_filter do
  tran stop: 1.ms, max_step: 100.ns, output_interval: 10.us
  measure :avg_power, avg: v(:vin, :vout) * i(:r1)
end

experiment :two, circuit: :rc_filter do
  ac from: 100.Hz, to: 100.kHz, points_per_decade: 40
  op
  derive :g, expr: v(:vout) / v(:vin), analysis: :ac1
  measure :gm, max: abs(v(:vout) / v(:vin)), analysis: :ac1
end
```

Statement grammar (only extensions):

* `derive :name, expr: <result-expr>` and
  `derive :name, expr: <result-expr>, analysis: :<analysis-id>`
* `measure :name, <max|min|avg|rms>: <result-expr>` and the same statement with a
  trailing `analysis: :<analysis-id>`
* `<analysis-id>` is `{kind}{ordinal}` with a 1-based per-kind ordinal in declaration
  order: `op1`, `dc1`, `ac1`, `ac2`, `tran1`, ... This is exactly the analysis
  identity the backend stamps on the result dataset (`thevenin.rs:548`). `:ac`
  without an ordinal is NOT accepted; the diagnostic lists the available ids.

Result expressions (inside `expr:` and inside a measure target) accept:

| form | meaning |
|---|---|
| `v(:node)`, `v(:a, :b)`, `i(:device)` | probe reads, resolved to typed probes |
| dimensionless number literals (`1`, `2.5`) | broadcast scalar |
| `( )`, unary `+`/`-`, `+`, `-`, `*`, `/` | arithmetic |
| `abs(x)`, `sqrt(x)`, `min(a,b)`, `max(a,b)`, `gain_db(a,b)` | existing evaluator functions |

Rejected by design: comparisons/logic, arrays, dicts, dimensions other than
dimensionless literals, bare identifiers, and a derive referring to another derive
("first version: derive signals do not feed other derives"). Function names and
signal names live in different namespaces.

## 2. Frozen plan-layer IR (`circuit-core::plan`)

```rust
pub struct ProbeRef { pub name: String, pub probe: Probe, pub span: SourceSpan }

pub enum ExprIr {
    Number(f64),
    Probe(ProbeRef),
    Neg(Box<ExprIr>),
    Add(Box<ExprIr>, Box<ExprIr>),
    Sub(Box<ExprIr>, Box<ExprIr>),
    Mul(Box<ExprIr>, Box<ExprIr>),
    Div(Box<ExprIr>, Box<ExprIr>),
    Abs(Box<ExprIr>),
    Sqrt(Box<ExprIr>),
    Min(Box<ExprIr>, Box<ExprIr>),
    Max(Box<ExprIr>, Box<ExprIr>),
    GainDb { numerator: Box<ExprIr>, denominator: Box<ExprIr> },
}

impl ExprIr {
    pub fn span(&self) -> SourceSpan;
    /// Dependency set: distinct probes in first-seen order (identity = ProbeRef::name).
    pub fn probes(&self) -> Vec<ProbeRef>;
    /// Dimension when it is statically known (probe kind / number / arithmetic).
    pub fn static_dimension(&self) -> Option<Dimension>;
    /// True when the expression can only produce real samples: numbers, abs(),
    /// gain_db() and arithmetic over such values. A bare probe is NOT known real.
    pub fn is_statically_real(&self) -> bool;
    /// The expression as the user would read it, for diagnostics.
    pub fn render(&self) -> String;
    /// True for a single probe read (`ExprIr::Probe`).
    pub fn is_plain_probe(&self) -> bool;
}

pub enum AnalysisBinding { Analysis(AnalysisId), LegacyPreferred }

pub struct DeriveRequest {
    pub name: String,
    pub expr: ExprIr,
    pub binding: AnalysisBinding,
    /// The expression in canonical rendered form, for reporting and metadata
    /// (elaboration has spans but no source text).
    pub source: String,
    pub span: SourceSpan,
    pub name_span: SourceSpan,
}

pub struct MeasureRequest {
    pub name: String,
    pub kind: MeasureKind,
    pub expr: ExprIr,
    pub binding: AnalysisBinding,
    pub source: String,
    pub span: SourceSpan,
    pub kind_span: SourceSpan,
}

pub struct AnalysisTask {
    pub id: AnalysisId,
    pub kind: AnalysisKind,
    /// Explicit `save` probes. Empty = backend default (unchanged).
    pub probes: Vec<NamedProbe>,
    /// Expression dependencies the user did not ask to export. Read, never exported.
    pub implicit_probes: Vec<NamedProbe>,
    pub span: SourceSpan,
}

pub struct AnalysisPlan {
    pub name: String, pub circuit_name: String,
    pub tasks: Vec<AnalysisTask>,
    pub param_overrides: Vec<(String, Quantity, SourceSpan)>,
    pub derives: Vec<DeriveRequest>,
    pub measures: Vec<MeasureRequest>,
    pub span: SourceSpan,
}

impl AnalysisPlan {
    pub fn task(&self, id: AnalysisId) -> Option<&AnalysisTask>;
    /// `{kind}{ordinal}` — the analysis identity and result-dataset name of a task.
    pub fn result_name(&self, id: AnalysisId) -> Option<String>;
    /// Look an analysis identity up as written in `analysis: :ac1`.
    pub fn analysis_id_by_name(&self, name: &str) -> Option<AnalysisId>;
    /// All analysis identities, in plan order, for diagnostics.
    pub fn analysis_names(&self) -> Vec<String>;
    /// `Some((parameter, sweep))` when the plan contains a DC parameter sweep.
    pub fn parameter_sweep(&self) -> Option<(String, Sweep)>;
}
```

Dependency direction is unchanged and enforced: `core <- results <- session/backend/cli`.
`circuit-core` never depends on `circuit-results`. The runtime expression AST stays
`circuit_results::expr::Expr`; lowering is a method of the results crate:

```rust
// circuit-results/src/expr.rs
pub fn from_ir(ir: &circuit_core::plan::ExprIr) -> Expr;
```

## 3. Frozen binding rules

For a `derive` (always an expression) and for a compound `measure` target:

1. explicit `analysis: :id` must name an existing analysis of the experiment,
   otherwise `Code::Name` listing the available ids;
2. with one analysis in the experiment and no `analysis:`, bind to it;
3. with more than one analysis and no `analysis:`, report `Code::Ambiguous:`
   never guess from which run happens to succeed.

Legacy exception, preserved exactly: a `measure` whose target is a **plain probe**
with no `analysis:` keeps `LegacyPreferred` (TRAN -> AC -> DC -> OP rank order,
then declaration order), which is today's documented behaviour.

Unsupported combination, rejected before any solve: a parameter DC sweep produces
exactly one stitched dataset, so binding a derive/measure to any other analysis of
that experiment is an explicit capability error, not a silent drop.

## 4. Execution order (unchanged shape, expression inserted)

```
elaborate + static checks
  -> per-analysis read set = save probes (or backend default) + implicit probes
  -> backend solve (raw solver grid, output_interval never reaches it)
  -> evaluate derive expressions on the RAW dataset   (non-linear value first)
  -> evaluate measures on the RAW dataset             (coarse output cannot move them)
  -> build the output view = exported signals + derived signals
  -> resample the output view (linear interpolation of already-computed derived data)
  -> CLI/REPL display, CSV/JSON export
```

Derived signals are appended to the raw dataset (so measurements and later derives
see them) and to the output view. Exported signals = explicit `save` names, or the
backend's own signal set when the experiment has no `save`; implicit probes never
enter the export unless they were also saved. The backend records the names it added
*only* for an expression in `Dataset::implicit_only`, and the output view removes
exactly those from a `save`-less export, so adding an expression can neither drop a
column the analysis reported nor add one nobody wrote (round-3 review B1).

## 5. Frozen error contract

| case | stage | code |
|---|---|---|
| unknown probe / unknown node / unknown device | check (elaborate) | `Name` |
| unknown function in a result expression | check | `Name` (lists available) |
| non-dimensionless literal, bare identifier, derive-references-derive | check | `Type` |
| `+`/`-`/`min`/`max` with statically different dimensions | check | `Dimension` |
| `gain_db` with statically different numerator/denominator dimensions | check | `Dimension` |
| unknown `analysis: :id` | check | `Name` |
| ambiguous binding in a multi-analysis experiment | check | `Ambiguous` |
| duplicate derive name | check | `Duplicate` |
| a derived name the backend already returned as a signal | runtime, before export | `Duplicate` |
| `avg`/`rms` bound to an analysis without a time axis | check | `Type` |
| `max`/`min` bound to an AC analysis over a possibly-complex expression | check | `Type`, "use abs(...)" |
| `max`/`min` of a value that turns out complex | runtime | `Type`, "use abs(...)" |
| division by zero sample | runtime | `Value`, with analysis, signal and sample index |
| `gain_db` with a zero-magnitude sample | runtime | `Value`, with analysis, signal and sample index |
| selected analysis fails to evaluate | runtime | the failure propagates: the requested measure never disappears silently |
| `avg`/`rms` where no candidate has a time axis | runtime | `Type` listing what was tried |
| a parameter-sweep experiment with a binding to a non-swept analysis | pre-run capability check | `Unsupported` |

No epsilon rescue: an illegal value is a diagnostic, never a plausible finite number.
A runtime expression/measure failure aborts the run before any file is written; the
CLI exits non-zero (`EXIT_USER_ERROR`) and no partial export exists.

Analysis selection for the legacy path is documented per candidate:
`LegacyPreferred` skips a dataset only when the signal is absent or when the
reduction cannot apply there (no time axis for avg/rms); an evaluation failure on the
selected dataset is an error, not a reason to try the next one.

## 6. Frozen result-layer additions

```rust
// circuit-results/src/measure.rs
pub struct Measured {
    pub name: String, pub value: f64, pub unit: Dimension,
    /// Analysis identity the value was taken from (`ac1`), empty when unknown.
    pub analysis: String,
}
impl Measured {
    pub fn new(name, value, unit) -> Self;          // analysis = ""
    pub fn with_analysis(self, analysis: impl Into<String>) -> Self;
    pub fn render(&self) -> String;                 // unchanged: "name = value unit"
    pub fn render_with_analysis(&self) -> String;   // appends "(ac1)" when known
}
// circuit-results/src/expr.rs
pub fn from_ir(ir: &circuit_core::plan::ExprIr) -> Expr;
```

`circuit_session::RunOutcome` keeps its fields and gains:

```rust
pub struct RunOutcome {
    pub datasets: Vec<Dataset>,          // raw grid, one per task, plan order (+ derived signals)
    pub output_datasets: Vec<Dataset>,   // exported signals (+ derived), resampled
    pub measures: Vec<Measured>,
    /// Raw dataset index each analysis task produced (one entry per task).
    pub bindings: Vec<(AnalysisId, usize)>,
    pub warnings: Vec<String>,
}
```

`circuit_session::execute` keeps its signature and now returns `Err` for a failed
expression or measure instead of returning a silently shortened measure list.
`evaluate_measures(plan, datasets) -> Result<Vec<Measured>, Diagnostics>` is the
public seam and keeps that name with the new `Result` return type.

## 7. Matrix of invariants the QA must be able to check

1. RC at the cut-off frequency: `gain = 0.5 - 0.5j`, `|gain| = 0.70710678`,
   `gain_db = -3.01029996` on the sample that actually lands on Fc.
2. `avg: v(:vin, :vout) * i(:r1)` equals an independent `v^2/R` computation over
   the raw grid, with the same sign convention.
3. Fine vs coarse `output_interval:`: identical measure values, different exported
   point counts, identical derived values at shared time points.
4. With `save v(:vin)` only, an expression using `v(:vout)` still evaluates,
   `v(:vout)` is not exported, and the derived column is exported.
5. Multi-analysis binding works, unknown `analysis:` and omitted binding are refused.
6. Legacy `measure :vmax, max: v(:out)` selection order unchanged.
7. Unit error, division by zero, zero-amplitude dB, unavailable current, unknown node,
   complex reduction all produce the contract's diagnostic and exit code.
8. Duplicate derive names and derive/save collisions are refused at check time.
9. CSV/JSON keep real vs complex, units and analysis identity, including derived columns.
10. File mode and REPL produce the same values and the same errors.
11. Single-parameter DC sweep binds correctly or is explicitly refused.
12. All pre-existing tests keep passing.
