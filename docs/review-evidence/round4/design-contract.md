# Round 4 design contract

Frozen by the lead before implementation. Every worker codes against this file; a change here is a
lead decision, not a worker decision.

Scope basis: `docs/round3-review-and-round4-plan.md` §3–§6 and `ROUND4_AGENT_TEAM_PROMPT.md`.
Round 3's contract (`docs/review-evidence/round3/design-contract.md`) still governs everything it
covered; this file only adds the round-4 decisions.

## 0. Baseline (measured in this round, not inherited)

| check | command | result |
|---|---|---|
| workspace tests | `cargo test --workspace --quiet` | all test binaries ok, 0 failed (log: `target/round4/baseline-test.log`) |
| R4-01 | `cdsl check target/round4/repro/r4-01.cdsl` | exit 0 — `sqrt(-1)` is accepted |
| R4-01 | `cdsl run ... --experiment bad_sqrt` | exit 0, `measure masked = 2 dimensionless`, derived signal null, no diagnostic |
| R4-02 | `cdsl check target/round4/repro/r4-02.cdsl` | exit 0 |
| R4-02 | debug `cdsl run ... --experiment dim_overflow` | panic `attempt to add with overflow` at `circuit-core/src/units.rs:49`, exit 101 |

The four required reproductions (lead, real CLI, debug build) are stored under
`target/round4/repro/` with their outputs.

## 1. Phase A — frozen interfaces

### 1.1 Dimension arithmetic — lead, `crates/circuit-core/src/units.rs`

Exponents stay `i8`; the type is not widened, wrapped, saturated, or guarded by
`catch_unwind`. The infallible operations are **removed**, so the compiler enumerates every call
site instead of a release build silently wrapping an exponent:

```rust
impl Dimension {
    pub const MAX_EXPONENT: i8 = i8::MAX;
    pub const fn checked_mul(self, rhs: Self) -> Option<Self>; // None when an exponent leaves i8
    pub const fn checked_div(self, rhs: Self) -> Option<Self>;
    pub const fn checked_pow(self, n: i8) -> Option<Self>;
    // mul / div / pow no longer exist
}
impl Quantity {
    pub fn checked_mul(self, rhs: Self) -> Option<Self>;
    pub fn checked_div(self, rhs: Self) -> Option<Self>;
    // impl Mul/Div for Quantity no longer exists; Neg is unchanged
}
```

Rule for callers: any operation whose operands can come from a written program must use the
`checked_*` form and turn `None` into a `Code::Dimension` diagnostic that names the operation and
the operands. `None` is never unwrapped on a user-reachable path.

Landed call sites: `circuit-core/src/plan.rs` (static analysis), `circuit-dsl/src/eval.rs` (the
language's own expression evaluator, diagnostic at the operator span), `circuit-results/src/expr.rs`
(result expressions, runtime guard). `circuit-core/src/format.rs` and the unit tests use the
checked form with an explicit `expect` because their inputs are literals.

### 1.2 Static dimension analysis — lead, `crates/circuit-core/src/plan.rs`

```rust
impl ExprIr {
    /// Unchanged signature. Now also `None` when a product/quotient leaves the
    /// exponent range (it used to panic in debug / wrap in release).
    pub fn static_dimension(&self) -> Option<Dimension>;
    /// Unchanged signature. Reports exponent overflow first, then the existing
    /// unit-mismatch messages. Consumed by `elaborate::lower_result_expr`, so
    /// `cdsl check` rejects the overflow without any front-end change.
    pub fn static_dimension_error(&self) -> Option<String>;
}
```

### 1.3 Result-expression evaluation — results worker, `crates/circuit-results/src/expr.rs`

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EvalKind { Derive, Measure }

/// Who asked for the value, for diagnostics. The name is the `derive`/`measure` name.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EvalSite { pub kind: EvalKind, pub name: String }
impl EvalSite {
    pub fn derive(name: impl Into<String>) -> Self;
    pub fn measure(name: impl Into<String>) -> Self;
}

/// Existing entry point, kept: equivalent to `eval_at(expr, dataset, &EvalSite::anonymous())`.
pub fn eval(expr: &Expr, dataset: &Dataset) -> Result<Value, Diagnostic>;

/// Evaluation with a named site; every diagnostic it produces carries the site.
pub fn eval_at(expr: &Expr, dataset: &Dataset, site: &EvalSite) -> Result<Value, Diagnostic>;

/// True when the expression reads no signal, so it can be decided without a run.
pub fn is_constant(expr: &Expr) -> bool;

/// Evaluate a constant expression (check time). `Err` when the expression is
/// not constant or when it is illegal (sqrt(-1), 1e308*1e308, x/0, gain_db of 0).
pub fn eval_constant(expr: &Expr) -> Result<Value, Diagnostic>;
```

Policy (all in this file, one implementation for real and complex):

1. **Every operation validates its own result.** After `number/signal/neg/abs/sqrt/min/max/add/sub/
   mul/div/gain_db` produces samples, every sample must be finite: real `x.is_finite()`, complex
   both components finite. A non-finite sample is a `Code::Value` error naming the operation and the
   sub-expression that produced it — so `min(sqrt(-1), 2)` fails at `sqrt`, it cannot be masked.
2. **Domain rules stay exact, never epsilon:** `sqrt` of a negative real sample is an error;
   a denominator exactly 0 is an error (unchanged); `gain_db` with a zero-magnitude ratio is an
   error (unchanged). No saturation, no sample skipping, no `catch_unwind`.
3. **Finite inputs are checked too:** a signal sample that is already NaN/±Inf is refused by the
   operation that reads it, with the same error class.
4. **Diagnostic content:** context pairs `analysis`, `kind`, `signal` (the `derive`/`measure`
   name when the site is named), `sample`, `index`; the `sample` value is the axis coordinate
   (`time = 0.02`) when the analysis has an axis. A scalar expression (dataset `Axis::None` or no
   dataset at all) says so in a note: no axis coordinate exists, the value is one scalar sample.
5. **Constant expressions** (`is_constant`) are also evaluated by `eval_constant` at check time
   (§1.5) and produce the same error text as the runtime path, minus analysis/sample context.

Runtime dimension guard: multiplication/division of dimensions uses
`Dimension::checked_mul/checked_div`; `None` is a `Code::Dimension` diagnostic, never a panic and
never a wrapped exponent.

`crates/circuit-results/src/measure.rs` is in the same write set and passes the site through to
`expr::eval_at`.

### 1.4 Export diagnostics — session worker, `crates/circuit-session/src/execute.rs`

The renderer already returns `Export { text, diagnostics }` (`to_csv_with_diagnostics`,
`to_json_with_diagnostics`). The session writer must stop throwing that away:

```rust
/// What a write produced: the files, and the warnings the renderer raised while making them.
#[derive(Clone, Debug, Default)]
pub struct Written {
    pub paths: Vec<PathBuf>,
    /// User-visible warnings, deduplicated per dataset across formats.
    pub warnings: Vec<Diagnostic>,
}

pub fn write_datasets(
    out: &Path,
    format: Format,
    datasets: &[Dataset],
    approve: &mut dyn FnMut(&Path) -> Result<(), Diagnostics>,
) -> Result<Written, Diagnostics>;
```

Semantics:

- Each dataset is rendered through the `*_with_diagnostics` API; nothing is written before every
  render in that dataset succeeded.
- **Relationship between the JSON file's diagnostics and the returned warnings (corrected after
  the reviewer's NEEDS_FIX P3):** they are two different, complementary batches.
  `<file>.json` keeps the **dataset's own** `diagnostics` array — the provenance a
  `Dataset` carries from the plan/backend (`export.rs:193-196`). `Written::warnings` is the
  **renderer's** `non_finite_diagnostics` — one per non-finite signal sample, raised while
  rendering (`export.rs:169`, `:338`), which the file itself expresses only as an empty CSV field or
  a JSON `null`. Neither batch is a copy of the other, and the returned warnings are deduplicated by
  `render_plain()` text so exporting CSV+JSON of one dataset reports each renderer warning once.
  A caller that wants the whole picture shows `RunOutcome::warnings` (datasets) followed by
  `Written::warnings` (renderer), which is what both front ends do.
- The run path merges these into the user-visible output: `cdsl run` prints them like the existing
  dataset warnings (`warning: ...`), and the REPL shows the same warnings for the same experiment.
- Raw-data non-finite values keep their documented empty/null cell rendering; this contract does not
  change `Dataset`'s public API or the meaning of an empty cell.
- A failure earlier in the run (illegal expression, dimension overflow) still stops before any file
  is written; an existing file from a previous run is never deleted.

### 1.5 Check-time constant rejection — session worker, `crates/circuit-cli/src/check.rs`

`cdsl check` evaluates every `derive` and every expression-based `measure` whose `ExprIr` is
constant (`expr::is_constant(from_ir(&ir))`) with `expr::eval_constant` and fails the check on the
returned diagnostic. Non-constant expressions are only statically checked (dimension analysis);
they fail at run time.

## 2. Write sets (one writer per file)

| owner | files |
|---|---|
| lead | `circuit-core/src/units.rs`, `circuit-core/src/plan.rs`, `circuit-core/src/format.rs`, `circuit-dsl/src/eval.rs`, contract/board/evidence docs, final gates |
| results worker | `circuit-results/src/expr.rs`, `circuit-results/src/measure.rs`, `circuit-results/src/export.rs` |
| session worker | `circuit-session/src/execute.rs`, `circuit-cli/src/{run,check,repl}.rs`, `circuit-cli/src/main.rs` |
| QA worker | new test files only: `circuit-cli/tests/r4_*.rs`, `circuit-session/tests/r4_*.rs`, `circuit-results/tests/r4_*.rs` |
| DAG explorer / reviewer | none (read-only reports) |

No worker edits another worker's file. Cross-file needs are reported to the lead, who reassigns the
file or lands the change itself.

## 3. Phase A acceptance mapping (from plan §4)

| plan row | who covers it |
|---|---|
| `sqrt(-1)`, negative signal sqrt -> structured error, no successful measure | results worker (runtime) + session worker (check) + QA |
| `min(sqrt(-1),2)`, nested max -> illegal intermediate not hidden | results worker + QA |
| `1e308*1e308`, nested min/max -> overflow diagnostic | results worker + QA |
| non-finite complex components / complex operation overflow -> one policy | results worker + QA |
| 128 voltage factors, dimension subtraction boundary -> check/run diagnostic, no panic, no wrap | lead (core) + results worker + QA, debug and release |
| raw Dataset with NaN/Inf, CSV/JSON export -> documented empty format + visible warning | session worker (wiring) + results worker (renderer) + QA (independent Dataset) |
| same expression in CLI and REPL -> same numbers and error class | session worker + QA |
| valid derive then illegal derive in one experiment -> no successful output file from this run | session worker + QA |
| RC gain, power, multi-analysis binding, implicit probes, resampling -> unchanged | QA regression |

## 4. Phase B — parameter DAG (frozen from `dag-recon.md`)

Frozen by the lead on the evidence in `docs/review-evidence/round4/dag-recon.md` (every claim below
carries a `file:line` there). Phase B code does not start until phase A passes independent review.

### 4.1 Scope identity

A graph node is a **parameter in one body instance**, identified by
`ParamId { scope: ScopePath, name: String }`. `ScopePath` is the instantiation path the elaborator
already builds for node/device names (`Bodies.prefix`, `elaborate.rs:3407-3418`): the top-level
circuit is `top`, an instance is `top.stage1`, nested instances extend it. The bare name alone is
never a node id across scopes.

- Edges exist **only inside one body**: a `param` default may reference another `param` of the same
  body (today's scope rule, unchanged by phase B). A subcircuit default still cannot see the parent
  scope (`dag-recon.md` §1.1/§3.5, probe P) — that stays.
- The one cross-scope channel is `params:` on an instance: its values are evaluated in the **parent**
  scope (`elaborate.rs:1345-1397`) and become the instance-body parameter's effective definition. In
  the graph this is a value edge `(parent scope, name) -> (instance scope, name)`.
- Consequence for diagnostics: two same-named parameters in different scopes are different nodes, so
  a sweep of `top.r` can never implicate `top.stage1.r` unless the `params:` binding actually wires
  them (`dag-recon.md` B8; this is the "同名局部参数不得误报" requirement).

### 4.2 Effective-definition priority (unchanged)

`default -> instance params: -> experiment param -> sweep point -> session \`:run name=expr`
(`elaborate.rs:152-158`, `:620-624`, `:714-724`, `session.rs:484-564`; documented in
`docs/language.md` and `docs/repl.md`). Later entries replace earlier ones for the same name.

Two behaviours the recon found are **left exactly as they are**, because changing them would be a
behaviour change outside phase B's mandate; both are now recorded instead of being silent:

- a duplicated `params:` key inside one instance, and a duplicated `param` in an experiment body,
  both keep "last wins, no diagnostic" (`dag-recon.md` §3.8 d/e, probes M/N). Phase B must not
  start reporting them.
- a top-level override is still checked only for existence (E_NAME, `elaborate.rs:459-488`) and not
  for dimension; dimension errors keep surfacing at the device that consumes the value
  (`dag-recon.md` §3.8 c, probes R/O).

### 4.3 Effective definition first, then the graph

For every scope, the override chain selects each parameter's **effective definition** before any
dependency edge is built:

- a parameter with an override has no edges from its own default expression, its default is **not
  evaluated**, and it produces no diagnostic in that elaboration;
- a parameter without an override is defined by its `param` default, and its edges come from that
  expression;
- `param :r` with neither override nor default keeps today's behaviour: reading it is E_NAME with
  the "a parameter with this name is declared here" secondary span.

**Documented boundary (plan §5.1.3 vs. code, conflict resolved here).** `compile()` elaborates every
top-level circuit with an **empty** override chain (`elaborate.rs:97`), which is the pass `cdsl check`
uses. In that pass a circuit's own defaults are effective, so `param :r, default: nope` plus an
experiment `param :r, value: 1.kohm` still reports `E_NAME: nope is not declared` (probe I). Phase B
keeps that: the circuit is a reusable unit and `check` has to judge it, and
`docs/language.md` already documents the standalone rule. What phase B changes is the graph: with
that override in effect, `r` contributes no edge and no cycle, so the override is never damaged by
the default inside the elaboration that uses it. The alternative (accept a circuit because *some*
experiment's chain elaborates it) is recorded as a deliberate non-choice: it would make `check`'s
verdict depend on which experiment is read last and would hide real default errors.

### 4.4 Evaluation order, forward references, cycles, unknown names

- Declarations of one body are collected in a **pre-pass**, before the body runs
  (`elaborate.rs:631-664` currently evaluates them in source order as it walks).
- The graph is topologically sorted; the sort is **deterministic**: source order breaks every tie,
  so two runs of the same input elaborate identically (a stable priority queue over declaration
  index).
- A forward reference inside one body is **accepted**: `param :b, default: 2 * a` before `param :a`
  resolves through the graph. This is the one documented behaviour that changes
  (`docs/language.md:255`, `dag-recon.md` §3.3): the existing tests
  `a_forward_parameter_reference_is_rejected` and `a_self_referential_parameter_is_rejected`
  (`crates/circuit-dsl/tests/elaborate.rs:405-445`) are **rewritten to the new contract**, and
  `docs/language.md` is updated in the same change.
- An unknown name stays `E_NAME` at the reference, with today's note ("an undeclared name is never
  treated as a node, device or function call"). It is not a cycle.
- A cycle is `E_PARAM_CYCLE` (the code already exists, `circuit-core/src/diagnostic.rs:61-62`, and
  today has no producer) with: the closed path `a -> b -> a` in the message, the primary span at the
  declaration where the path closes, and a secondary span per participating declaration.
  Self-reference `param :a, default: a` is the one-node case of the same diagnostic.
- Evaluation uses the existing `eval::eval` and the existing units rules — no second evaluator and no
  new unit system (`eval.rs`, unchanged semantics apart from the phase-A overflow diagnostic).

### 4.5 Topology use points and their dependency propagation

Only the constructs the code really has count (`dag-recon.md` §1.4, §4):

| topology use | where |
|---|---|
| `if` condition | `elaborate.rs:1577-1611` |
| `for` iteration source (list, range) | `elaborate.rs:1456-1525` |
| computed names: device/node/instance name, device terminal | `elaborate.rs:376-415`, `:766`, `:793`, `:1036-1093`, `:1236` |

A parameter is **topology-affecting** when it is in the transitive closure computed from those use
sites by walking the graph **backwards** (a depends on b means sweeping b can move anything that uses
a):

- a use site that reads param `p` marks `p` and every param whose effective definition references
  `p` (transitively, in the same scope);
- an instance `params: { x: <expr reading p> }` marks `p`, and marks the instance-local `x` in that
  instance's scope; topology use sites inside that instance body then propagate the same way;
- numeric positions (`value:`, `dc:`, `ac:`, `waveform:`, model args) are **not** topology uses by
  themselves — a sweep over a plain component value stays legal, exactly as today.
- `dc param: :name` names the *swept* parameter (a symbol, not an expression,
  `elaborate.rs:3215-3232`); the analysis starts from that name and never invents an edge.
- Analysis-size positions (`ac ... points: n`) cannot reference a parameter at all today (empty
  scope, `elaborate.rs:2960-2966`) — not a topology use and not extended by phase B.

### 4.6 `check`-time rejection and the runtime defence

- When an experiment declares a parameter sweep, its elaboration refuses the plan if the swept
  parameter is topology-affecting, reporting the explanation path
  `scanned parameter -> intermediate parameter(s) -> topology use point` with the span of each
  step and of the use site. The diagnostic code is `E_TOPO_PARAM` (the runtime defence already uses
  it, `circuit-backend/src/sweep.rs:227-249`); the check-time message additionally says that a plain
  value sweep is still allowed when the parameter reaches no topology use.
- The experiment plan is built inside `compile()` (`elaborate.rs:118`), so `cdsl check` rejects it
  without any CLI change, and a session/REPL run refuses it before solving.
- The runtime per-point comparison in `sweep.rs` **stays** and keeps its test C7: check is a second
  defence, not a replacement.
- Every example and every existing sweep test must keep working: `examples/parameter_sweep.cdsl`
  (`e2e.rs:55-75`), the exact-number sweep test (`e2e.rs:520-551`), and the stitched-dataset tests
  (`expression_flow.rs:454-501`).

### 4.7 REPL failure state

Unchanged and re-verified rather than redesigned: the session holds no persistent parameter update
command (`session.rs:70-72`), `:run exp name=expr` evaluates the override in the session scope and
passes it down the same chain as a file run, and a failed run or a failed definition commit leaves
`circuits`/`experiments`/`vars` untouched (`dag-recon.md` §5, probes L). Phase B adds the case the
new DAG introduces: after a successful override of a base parameter, every parameter that depends on
it is recomputed (the plan's "覆盖后依赖重算"), and a *failed* override still leaves the next
successful run identical to the pre-failure result.

### 4.8 Phase B write set (one writer)

| owner | files |
|---|---|
| DAG worker | new `crates/circuit-dsl/src/param_graph.rs`, `crates/circuit-dsl/src/elaborate.rs`, `crates/circuit-dsl/src/lib.rs` |
| lead | `crates/circuit-core/**` (only if a core type is genuinely needed), contract/board/evidence, final gates |
| QA worker | new `crates/circuit-cli/tests/r4b_*.rs`, `crates/circuit-session/tests/r4b_*.rs`, `crates/circuit-dsl/tests/r4b_*.rs`, and the rewrite of the two phase-outdated tests in `crates/circuit-dsl/tests/elaborate.rs` |
| docs worker | `docs/language.md`, `docs/architecture.md`, `docs/repl.md`, `docs/testing.md`, `README.md` |
| reviewer | none (read-only report) |

The two existing tests that encode the old forward-reference rule move to the QA worker's write set
in the same change that makes forward references legal; nobody else touches
`crates/circuit-dsl/tests/elaborate.rs`.
