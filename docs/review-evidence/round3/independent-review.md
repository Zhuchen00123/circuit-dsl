# Round 3 independent review — result expressions, derived signals, analysis binding

Reviewer workstream (team-board row 8). Read-only round: the only file written is this one.
Nothing was committed, reverted, reformatted or edited outside `target/round3/reviewer/`.

- Snapshot reviewed: working tree at HEAD `8a4d569` **with the uncommitted round-3 diff**
  (27 tracked files modified, +3453/-191, plus the new untracked test files
  `crates/circuit-cli/tests/expression_qa.rs`, `crates/circuit-dsl/tests/result_expressions.rs` and the
  round-3 evidence directory), i.e. exactly what `git status` shows now.
- Contract reviewed against: `docs/review-evidence/round3/design-contract.md` (frozen).
- Method: read the full `git diff` of every changed file, then reproduce behaviour with the real
  binary built from the same working tree.
- Commands used (own target dir, as instructed):
  ```
  $env:CARGO_TARGET_DIR="F:\codexprojects\dsl000\target\round3\w-reviewer"
  cargo build -p circuit-cli --tests
  cargo test -p circuit-core | -p circuit-results | -p circuit-dsl | -p circuit-backend | -p circuit-session | -p circuit-cli
  & target\round3\w-reviewer\debug\cdsl.exe check|run <file> --out ... --format csv|json
  node target\round3\reviewer\analyse.js / analyse2.js     # my own numeric references
  ```
- Scratch inputs and outputs: `target/round3/reviewer/*.cdsl`, `target/round3/reviewer/out/**`.
  No file was created in the repository root, in `examples/`, or in the source tree.

---

## 1. Verdict per focus area

### 1.1 Layering (core must not depend on results; no AST leak; no second engine) — **VERIFIED**

- `git status` shows **no `Cargo.toml` modified** in the round, so no dependency edge was added.
  `crates/circuit-core/Cargo.toml` still depends only on `thiserror`; the expression IR
  (`ExprIr`, `ProbeRef`, `AnalysisBinding`, `DeriveRequest`) lives in `circuit-core::plan`
  (`crates/circuit-core/src/plan.rs:53-300`) and is expressed with core types only.
- `grep` over `crates/circuit-backend/src`: no `circuit_dsl`, no `ExprIr`, no front-end AST type.
  The backend sees `circuit_core::plan::{AnalysisTask, AnalysisPlan}` and
  `circuit_results::{Dataset, Signal}` only (`thevenin.rs:46`, `backend.rs:16`).
- Exactly one lowering exists: `circuit_results::expr::from_ir` (`crates/circuit-results/src/expr.rs:285`),
  re-exported once (`lib.rs:67`); the only other `ExprIr` users are the front end (which builds it),
  the session (which hands it to `from_ir`), and tests.
- Exactly one evaluator: the DSL lowers `ast::Expr` to `ExprIr` without evaluating
  (`elaborate.rs:2311-2540`); `circuit-dsl/src/eval.rs` remains the *parameter* evaluator, untouched
  by this round and never called from the result-expression path.
- Confirmed by reading that no result value is threaded through the backend: the derived column is
  appended in the session (`execute.rs:386-390`), after the backend has returned.

### 1.2 Analysis binding (task↔dataset identity, sweeps, silent drops) — **PARTIALLY VERIFIED**

Verified:
- Identity, not name-guessing: the round-2 code that recovered a transient task by parsing
  `dataset.analysis.strip_prefix("tran")` is **gone**; `output_view` now resolves the task through
  `bindings` (`execute.rs:285-288`). The remaining `dataset.analysis` reads are reporting only.
- One dataset per task in plan order: `binding_table` (`execute.rs:232-260`) maps `tasks[i] -> i`
  and the backend pushes in plan order. Reproduced: `rank_tie.cdsl` (two DC sweeps) exported
  `dc1` then `dc2`, and the two stitched files hold the two different sweeps.
- A parameter sweep delivering one dataset named `dc_param_*` for a task the plan calls `dc1` binds
  to the swept task: `sweep_two.cdsl` experiment `sweepr` prints
  `dc_param_r: 4 sweep points; signals: v(out), d` and `measure m = 2.25 V (dc_param_r)`, with
  `d = 2*v(out)` appended; the independent divider value at r = 0.5 kohm is 3*1.5/(0.5+1.5) = 2.25 V.
- A binding to a non-swept analysis is refused **before** the solve with `E_UNSUPPORTED`
  (`check_sweep_bindings`, `execute.rs:187-215`) — reproduced with `sweep_op.cdsl` (op + swept dc).
- `E_AMBIGUOUS` for an omitted binding in a multi-analysis experiment, and `E_NAME` listing the
  available ids for an unknown one, both fire at check stage (reproduced by the QA file, re-read here).

**Not verified / broken:** with **two** parameter-sweep analyses the two functions that answer
"which analysis does the sweep deliver?" disagree — see finding **B2**, which lets a derive bound to
`analysis: :dc1` be evaluated against the dataset produced by sweeping the *other* parameter.

### 1.3 Execution order and grid discipline — **VERIFIED** (export-set composition excepted, see B1)

- Derived signals are evaluated on the raw solver grid, before any resampling: `attach_derived`
  runs before `output_view` (`execute.rs:154-165`), the output view is resampled last
  (`execute.rs:317-320`).
- Numerical proof that the order is right, with a deliberately non-linear expression
  (`power = v(:vin,:vout) * i(:r1)`, quadratic in the probe values):
  - the coarse (20 us) and fine (1 us) views agree at all 51 shared times to **4.3e-23** absolute;
  - the *wrong* order (interpolate `v` and `i` first, then multiply, on the same view)
    differs by **8.4e-11** absolute = 8.4e-8 relative to the 1 mW peak.
- `output_interval` never reaches the solver: `backend.settings` for the raw/fine/coarse runs of the
  same circuit are identical except for `tran.output_interval`/`tran.output_grid`/`tran.output_points`
  — `tran.solve_points = 10018` and `tran.waveform_bound = 1e-9` in all three.
- Measures do not move with `output_interval`: `avg_power = 0.000049999859482767916 V*A` identical
  for raw, 1 us and 20 us output grids, and the exported point counts are 10018 / 1001 / 51.
- Implicit probes are read without a `save` (the expression evaluates) — verified; they are **not**
  exported when a `save` exists — verified. They **are** exported when there is no `save`: B1.
- A failing expression/measure leaves no partial export: every negative case below produced
  `EXIT=1` and an empty/absent output directory (`FILES=none`), including a failure in a *derive*
  (`err_div_zero_derive.cdsl`) and a failure raised only while building the output view
  (`derive_const.cdsl`).

### 1.4 Error contract (design-contract section 5, row by row) — **VERIFIED**, two caveats

Every row was exercised through the real CLI (check stage and/or run stage), each in its own file
so the file-cumulative error gate could not hide anything. Observed code, stage, exit code:

| contract row | file I ran | observed |
|---|---|---|
| unknown probe / node | QA `an_unknown_node_is_a_name_error` (re-run) | `E_NAME`, exit 1 |
| unknown function | `err_unknown_fn.cdsl` | `E_NAME` + available list, exit 1 |
| non-dimensionless literal | `err_unit_literal.cdsl` | `E_TYPE`, exit 1 |
| bare identifier | `err_bare_ident.cdsl` | `E_TYPE`, exit 1 |
| derive references derive | `err_derive_ref.cdsl` | `E_TYPE`, exit 1 |
| `+`/`-`/`min`/`max` static dimension mismatch | `err_dim_add.cdsl` | `E_DIMENSION`, exit 1 |
| `gain_db` static dimension mismatch | `err_gaindb_dim.cdsl` | `E_DIMENSION`, exit 1 |
| unknown `analysis: :id` | QA + `sweep_two_d2.cdsl` (`E_UNSUPPORTED` row) | `E_NAME` with ids, exit 1 |
| ambiguous binding | QA `an_omitted_binding...` (re-run) | `E_AMBIGUOUS`, exit 1 |
| duplicate derive name | `err_dup_derive.cdsl` | `E_DUPLICATE` + "first defined here", exit 1 |
| `avg`/`rms` bound to a non-time analysis | `err_avg_op_bound.cdsl` | `E_TYPE` at check, exit 1 |
| `max`/`min` on AC over a possibly-complex expr | `err_ac_max.cdsl` | `E_TYPE` + `abs(...)` hint, exit 1 |
| `max`/`min` complex at runtime (legacy) | `legacy_avg_op.cdsl` path (QA for max) | `E_TYPE` + hint, exit 1 |
| division by zero sample | `err_div_zero_derive.cdsl` | `E_VALUE` + analysis/kind/signal/sample/index, exit 1 |
| `gain_db` zero-magnitude sample | `err_gaindb_zero2.cdsl` | `E_VALUE` at `parameter = 0`, index 2, exit 1 |
| no candidate has a time axis (runtime) | `legacy_avg_op.cdsl`, `legacy_avg_op_dc.cdsl` | `E_TYPE` listing what was tried, exit 1 |
| sweep binding to a non-swept analysis | `sweep_op.cdsl` | `E_UNSUPPORTED` before the solve, exit 1 |
| failed expression aborts before writing | all of the above | no file, `EXIT=1` |

Caveats: the runtime no-time-axis **message is garbled** (N1), and one contract row is not reachable
from the language at all (L1). No path was found in which a requested measure vanishes silently:
`evaluate_measures` now returns `Result` and the legacy loop only skips a candidate that cannot
answer (`execute.rs:682-760`). "No epsilon rescue" also holds: `1e-300` denominators keep working
(results-layer test) and only exact zeros are reported.

### 1.5 Resource boundaries — **PARTIALLY VERIFIED**

- Bounds that still hold: sweep points are checked against `max_sweep_points` before any solve
  (`execute.rs:436-448`, unchanged); `resample_time` checks `point_count * signals.len()` against
  `max_result_values` (`resample.rs:163-184`) and the view it receives now **includes the derived
  columns**, so derived signals are counted; `Dataset::new` is now called on every output view
  (`execute.rs:303-311`), which re-runs length/duplicate/limit validation over the exported set.
- No unbounded broadcast was introduced: `combine` still only accepts equal lengths or a length-1
  operand (`expr.rs:616-626` for the new pre-check, `combine` for the combination), and every
  operand length is bounded by the axis, which the backend already validated.
- Cost of the new code (bounded but worth knowing): `output_view` now materialises an intermediate
  `cloned()` copy of the selected signal set before resampling it (N4), and derived signals are
  appended to the raw dataset without a re-validation, so the first enforcement point for a derived
  signal's size is the view build — after the allocation.
- Not verified by execution: see section 3.

### 1.6 Backwards compatibility — **PARTIALLY VERIFIED** (two deliberate changes)

- All 7 files in `examples/` still `check` with exit 0.
- Per-crate suites, all green, 0 failures: circuit-core 65, circuit-results 93, circuit-dsl 215,
  circuit-backend 58 (including `output_interval_regression`, `phase_regression`,
  `source_breakpoint_regression`, `transient_reference_regression`, `adapter`),
  circuit-session 50 (including the round-2 `tran_output_interval` suite), circuit-cli 66
  (e2e, repl, expression_qa) — **547 tests, 0 failed** (baseline was 457).
- Legacy measure order reproduced independently: `rank_tie.cdsl` has two DC sweeps of equal rank
  where the *second* would give the larger number; the tool reports `measure vm = 1.5 V (dc1)` and
  `measure vm2 = 0.5 V (dc1)` — TRAN→AC→DC→OP with **declaration order** breaking the tie, exactly
  the documented rule (the old stable sort by rank over plan-ordered datasets is correctly replaced
  by `sort_by_key(|i| (rank, i))`, `execute.rs:708`).
- Legacy `save`/`measure` syntax unchanged; `check` output keeps the old lines and only appends
  (`check.rs:97-152`).
- Deliberate public behaviour changes, both pinned by tests and documented:
  1. complex `max`/`min` is now `E_TYPE` (`measure.rs:185-198`) instead of silently ranking
     magnitudes; the pre-existing test `complex_measurements_use_the_magnitude` was rewritten into
     `complex_max_and_min_are_rejected_with_an_abs_hint`, which the frozen contract section 5 requires.
  2. the REPL run summary now prints measures through `Measured::render_with_analysis()`
     (`session.rs:645`), i.e. raw SI plus the analysis id, instead of the engineering-prefixed value
     it printed before; pinned by `repl.rs`, `e2e.rs` and `expression_qa.rs`.
- Unpinned shape change caused by **B1**: for a no-`save` experiment that uses an expression, the
  exported CSV/JSON gains columns it never had before.

---

## 2. Findings

### BLOCKING

#### B1. Implicit expression inputs are exported when the experiment has no `save`, and `check` says the opposite
- Code: `crates/circuit-backend/src/thevenin.rs:506-526` adds `task.implicit_probes` to the
  "no explicit probe list: expose everything" signal set; `crates/circuit-session/src/execute.rs:290`
  + `294-301` then keeps **every** signal of the dataset because `exported_names()` is `None`
  (`crates/circuit-core/src/plan.rs:519-527`). The function's own doc comment
  (`execute.rs:270-271`) states the opposite: "Implicit expression dependencies never reach the
  export unless the user also saved them", and `cdsl check` prints
  `reads v(vin,vout), i(r1) (expression inputs, not exported)` (`check.rs:131`).
- Contract row broken: design-contract section 4, "implicit probes never enter the export unless
  they were also saved" (design-contract.md:186-188); matrix item 4 (design-contract.md:266).
- Why it matters: the export is the user's declared contract. A file that only *measures* something
  gains columns nobody wrote, in CSV, JSON and the REPL summary — e.g. `v(vin,vout)` and `i(r1)`
  appear from nowhere, and a downstream CSV reader keyed on column names breaks. It also makes
  `cdsl check` untrue, which is worse than the leak itself. Note `docs/language.md` §7.5 currently
  documents the leak as intended, so code and docs agree with each other and disagree with the
  frozen contract; the contract is the frozen artefact.
- Smallest reproduction (run):
  ```
  & target\round3\w-reviewer\debug\cdsl.exe run target\round3\reviewer\power_set.cdsl --experiment plain --out out\plain --format csv
  & ... --experiment nosave_expr --out out\nosave_expr --format csv
  & ... check target\round3\reviewer\power_set.cdsl
  ```
  `plain` (identical circuit, no expression, no `save`) writes
  `time,v(vout),v(vin),i(input)`; `nosave_expr` (same circuit plus
  `measure :avg_power, avg: v(:vin, :vout) * i(:r1)`) writes
  `time,v(vout),v(vin),i(input),"v(vin,vout)",i(r1)`, while `check` prints the "not exported"
  line for exactly those two signals. The REPL shows the same leaked set in its `:run` summary:
  `tran1: 10018 time points; signals: v(vout), v(vin), i(input), v(vin,vout), i(r1)`,
  and writes the same value as file mode (`avg_power = 0.000049999859482767916 V*A (tran1)`).

#### B2. `swept_task()` and `AnalysisPlan::parameter_sweep()` disagree, so a derive can be evaluated against the wrong analysis
- Code: `crates/circuit-core/src/plan.rs:682-692` returns the **last** parameter sweep of the
  experiment (it keeps overwriting `found`), and that is what `sweep_experiment` re-derives and
  actually sweeps (`execute.rs:414-416`, `431-432`). `crates/circuit-session/src/execute.rs:176-185`
  (`swept_task`) returns the **first** parameter sweep, and that is what the capability check
  (`check_sweep_bindings`, `execute.rs:187-215`) and the binding table (`binding_table`,
  `execute.rs:243-249`) treat as the only analysis that produces a result.
- Contract rules broken: section 3 and section 5's row "a parameter-sweep experiment with a binding
  to a non-swept analysis | pre-run capability check | `Unsupported`" — the check refuses the
  analysis that *is* swept and blesses the one that is not; section 2's "no binding derived from a
  name guess" is then violated in effect, because the stitched dataset carries the *other*
  parameter's name while `bindings` claims it is the first sweep's result.
- Why it matters: a user who writes two parameter sweeps in one experiment (which `check` accepts
  without complaint, even though `docs/language.md` §11 lists joint multi-parameter sweeps as
  unsupported) either gets a value taken from the wrong analysis, or is told to bind to the analysis
  that is not being swept.
- Smallest reproduction (run): `target/round3/reviewer/sweep_two_d1.cdsl` — `dc param: :r` (dc1)
  then `dc param: :c` (dc2), `derive :d1 ... analysis: :dc1`, `measure :m1 ... analysis: :dc1`:
  ```
  experiment `both` ...  dc_param_c: 4 sweep points; signals: v(out), d1
    measure m1 = 1.8 V (dc_param_c)
    wrote out\sw_d1\both.dc_param_c.csv
  parameter,v(out),d1
  0.000000050000000000000004,1.8,3.6      <- the axis is c (5e-8 .. 2e-7 F), r was never swept
  ```
  and `sweep_two_d2.cdsl` (same file with `:dc2`) is refused:
  ```
  error[E_UNSUPPORTED]: `d2` is bound to analysis `dc2`, but this experiment sweeps the parameter `c` ... bind it to `dc1`
  ```
  That message is the proof of the inconsistency: the tool names the analysis it actually sweeps
  (`c`, i.e. dc2) and then tells the user to bind to dc1.

### NON-BLOCKING

#### N1. The runtime "no analysis has a time axis" message is malformed
- `execute.rs:716-727`: each candidate is pushed as `format!(", {} ({})", analysis, axis.describe())`
  and then joined into the sentence `"... no analysis here has one: {signal} is {}"`, producing:
  ```
  error[E_TYPE]: `avg` needs a time axis, and no analysis here has one: v(out) is , op1 (no axis (operating point))
  ```
  and for op+dc: `v(out) is , dc1 (a parameter axis), op1 (no axis (operating point))`.
- Why it matters: the code, stage and exit code are right (contract row holds), but the sentence
  reads as if the signal were "` is , op1 ...`". Tests only assert the substring "time axis".
- Reproduction: `legacy_avg_op.cdsl`, `legacy_avg_op_dc.cdsl` (run, exit 1). The README-form
  design is otherwise fine: listing what was tried is exactly what design-contract.md:209 asks for.

#### N2. Every expression/measure failure renders `= analysis:` twice
- `expr.rs:629-641` (`sample_error`) already attaches `analysis`/`kind`/`signal`/`sample`/`index`;
  `execute.rs:399-407` (`expression_context`) and `execute.rs:770-776` (`measure_context`) add
  `analysis` a second time. Observed verbatim (also reproduced in `docs/repl.md` §4.5):
  ```
  = analysis: op1
  = kind: op
  = signal: 0
  = sample: sample 0
  = index: 0
  = analysis: op1        <- duplicate key
  = expression: (v(out) / 0)
  ```
- Reproduction: `err_div_zero_derive.cdsl`. Cosmetic, but the diagnostic is the user-facing contract.

#### N3. A constant-only `derive` fails at run time with an internal-sounding message
- Contract section 1 promises that a dimensionless literal is a "broadcast scalar", and
  `docs/language.md` §7.2 says literals broadcast "to every sample". That holds only when the
  expression also reads a probe: `derive :k, expr: 2` evaluates to a length-1 `Value`, and the view
  build rejects it (`execute.rs:303-311` → `Dataset::validate`):
  ```
  error[E_VALUE]: signal `k` has 1 samples, but the frequency axis has 3 samples
  ```
- Reproduction: `derive_const.cdsl` (in the same file, `derive :half, expr: v(:vout) / 2`,
  `measure :m, max: 2` are all fine). Nothing silent and no partial export, but the message does not
  say that a constant derived signal is unsupported, and the stage (output-view build) hides that
  the expression itself succeeded.

#### N4. `output_view` adds one full intermediate copy of the exported signal set
- `execute.rs:291-301` clones the selected signals and `execute.rs:318` then resamples that copy.
  Previously a non-transient dataset was moved into the view and only the resampled trace was built,
  so a transient with `output_interval` now holds raw + intermediate view + resampled output at once
  (≈ 1 extra dataset-sized allocation). It stays inside `max_result_values` (the view is validated),
  so this is a peak-memory note, not a bound violation.

#### N5. Team-board deliverables missing
- `docs/review-evidence/round3/team-board.md:14` lists `crates/circuit-session/tests/expression_flow.rs`
  as the QA worker's new session-level test; the file does not exist (`crates/circuit-session/tests/`
  holds only `session.rs` and `tran_output_interval.rs`). `docs/review-evidence/round3/acceptance.md`
  and `final-summary.md` (lead-owned) are also absent at review time.
- Behaviour is covered indirectly (the CLI e2e/REPL/expression_qa tests and my own runs exercise the
  same `execute` seam), so this is a delivery/documentation gap rather than an untested path.
- Note also `docs/review-evidence/round3/qa-*.md`: only `qa-probe-warnings.md` exists, and it
  covers only task 8 part A (the `_probe` warning cleanup), not the QA runs its own file claims.

#### N6. `docs/repl.md` §4.4 display rule no longer describes a measure line
- `docs/repl.md:212` still states that dimensioned values are shown in engineering notation with
  6 significant digits; since `session.rs:645` a `:run` measure line is
  `measure <name> = <raw SI value> <unit> (<analysis>)` (e.g. `measure vm = 1.5 V (dc1)`,
  or `0.000049999859482767916 V*A` where the old REPL printed an engineering form). The change is
  intentional (file/REPL consistency; §4.5 documents the new line) but §4.4 reads as if it still
  covered the run summary.

### LIMITATION (true and accepted)

#### L1. The contract's "derived name the backend already returned" row cannot be reached from the language
- `execute.rs:362-378` refuses a derived name that collides with a signal already in the dataset
  (`Code::Duplicate`), but every backend-produced name has probe form (`v(x)`, `i(x)` — see
  `thevenin.rs:1157-1178` and `materialise_probe`) while a derive name must be a plain symbol, so
  the collision is unreachable via the DSL. It is exercised only by a hand-built plan
  (`execute.rs` unit test `a_derived_name_that_collides_with_an_existing_signal_is_refused`).
  Contract matrix item 8's second half is therefore defensive code, not a checkable behaviour.
  The reachable half (`derive`/`measure` name duplicates, `derive` vs `save`) is checked at
  elaboration (`elaborate.rs:2541-2589`) and verified by `err_dup_derive.cdsl`.

#### L2. An `op` declared next to a swept DC produces no result, file or warning
- `stitch` keeps only `outcome.results[..].datasets.first()` (`execute.rs:507-516`) and names the
  result `dc_param_<parameter>` (`execute.rs:594-602`), so the second analysis of the plan is
  dropped. Reproduced: `sweep_op.cdsl` (op + `dc param: :r`) prints only
  `dc_param_r: 4 sweep points` and writes one file; no `op1` output exists and nothing is said.
  Pre-existing round-2 behaviour (the old `execute` also collapsed to the stitched dataset); round 3
  only makes it visible, because `analysis: :op1` can now be written and is then refused pre-run.

#### L3. `check` accepts joint multi-parameter sweeps that the documentation calls unsupported
- `sweep_two.cdsl` / `sweep_both.cdsl` pass `check` with exit 0 while `docs/language.md` §11 lists
  multi-parameter sweeps as out of scope. This input is what triggers B2; rejecting two parameter
  sweeps at elaboration would close the hole independently of the binding fix.

#### L4. Transient accuracy at the first solver steps bounds any analytic cross-check
- Not a round-3 defect, but it must be stated because it dominates transient reference values:
  with the 1 ns pulse of `power_set.cdsl`, v(vout) is **50 % low at t = 1 ns**, 2.7e-3 relative at
  t = 185 ns, 4.6e-4 at 1 us and 1.9e-10 at 1 ms versus `1 - e^{-t/tau}`. The kernel's forced
  Backward-Euler restart is documented in the round-2 evidence; it means a power integral can only
  be cross-checked to ~1e-5 relative, not to solver tolerance (see the numbers below).

---

## 3. Numbers reproduced from scratch

Both references were computed by me (node, `target/round3/reviewer/analyse*.js`) from the component
values, not read back from a neighbouring sample.

### 3.1 RC corner gain on a sample that lands exactly on Fc
```
& target\round3\w-reviewer\debug\cdsl.exe run target\round3\reviewer\fc_exact.cdsl --out out\fc --format csv
```
Circuit: R = 1 kohm, C = 159.15494309189535 nF (so Fc = 1/(2*pi*R*C) = 1000.0 Hz exactly, printed by
my script as `1.00000000000000000e+3`), `ac from: 500.Hz, to: 2.kHz, points: 4` — a linear sweep
whose grid is [500, 1000, 1500, 2000] Hz, i.e. Fc is a sample by construction.

| quantity | exported by the tool | my independent value | difference |
|---|---|---|---|
| `gain_re` at 1000 Hz | `0.5` | 1/(1+w^2R^2C^2) = 0.5 | 1.1e-16 |
| `gain_im` at 1000 Hz | `-0.5` | -wRC/(1+w^2R^2C^2) = -0.5 | 1.1e-16 |
| `|gain|` at 1000 Hz | 0.7071067811865476 | 1/sqrt(2) = 0.7071067811865475 | 1.6e-16 |
| `gain_db` at 1000 Hz | `-3.0102999566398116` | 20*log10(|H|) = -3.0102999566398125 | 1.1e-15 rel |
| `measure peak_gain` | `0.8944271909999161 dimensionless (ac1)` | max over rows of sqrt(gain_re^2+gain_im^2) = 0.8944271909999161 (=2/sqrt(5)) | 0 |

The contract's frozen numbers (0.5 - 0.5j, 0.70710678, -3.01029996) are all reproduced on the sample
that really is Fc.

### 3.2 Transient resistor power, integral against an analytic value
```
& target\round3\w-reviewer\debug\cdsl.exe run target\round3\reviewer\power_set.cdsl --experiment raw|fine|coarse --out out\<exp> --format csv
```
Circuit: R = 1 kohm from vin to vout, C = 100 nF, 1 V pulse (rise 1 ns), `tran stop: 1.ms, max_step: 100.ns`;
measure: `avg: v(:vin, :vout) * i(:r1)`, tau = R*C = 100 us, T = 1 ms.

| reference | value (W) | vs the CLI value |
|---|---|---|
| CLI `avg_power` (raw, 1 us and 20 us output grids all identical) | 4.9999859482767916e-5 | — |
| my trapezoid over the exported **raw** grid, of the `power` column | 4.9999859482767916e-5 | diff = **0.0** |
| my exact integrating-factor solution of tau*y' + y = v_in(t) with the real 1 ns ramp (1e-8 steps) | 4.999833327201336e-5 | 3.0e-5 relative |
| ideal-step analytic (1/(R*T))*(tau/2)*(1-e^{-2T/tau}) | 4.999999989694232e-5 | 2.8e-6 relative |

The residual against both analytic references is the kernel's first-step error quantified in L4
(v(vout) 50 % low at t = 1 ns), not an error of the expression/measure machinery: the trapezoid over
the exported derived column reproduces the CLI number bit for bit, and
`max |power - (v(vin)-v(vout))*i(r1)| = 0` on all 10018 raw samples (the derived column really is the
evaluated product, v^2/R to machine precision because i(r1) is derived as v/R), with
`min power sample = 0` (no negative power: the resistor absorbs, the sign convention is +).

Two further independent numbers from the same runs: 51 shared coarse/fine time points agree to
4.3e-23 (correct order) while "interpolate first, multiply after" differs by 8.4e-8 relative to the
peak; and integrating the **coarse exported column** instead of the raw grid gives 4.0665e-5 — a 19 %
error, which shows why the raw-grid rule matters.

---

## 4. What I could not verify, and why

1. **Workspace gates.** `cargo clippy --workspace --all-targets -D warnings` and
   `cargo fmt --all --check` were deliberately not run (lead-owned, and forbidden for this review).
   I ran per-crate tests only; formatting and lint cleanliness of the new code is unverified by me.
2. **`E_LIMIT` enforcement for derived signals, end to end.** `cdsl run` exposes no way to lower
   `Limits` and no existing test builds a plan with a limit small enough to trip on a derived column,
   so I verified by reading only: `resample_time` counts `signals.len()` including derived
   (`resample.rs:163-166`) and `Dataset::new` validates the view (`execute.rs:303-311`). I could not
   execute the failure path or observe its message.
3. **The sweep-point bound with a derived signal.** `max_sweep_points` is checked before the sweep
   (`execute.rs:436-448`); a sweep large enough to need it would exceed the CLI's only limits
   (defaults are 1e6 points / 5e7 values), so no run of mine reached it.
4. **Peak memory (N4) at scale.** No large-transient measurement was made; the extra copy is read
   off the code, not measured.
5. **Release profile, non-Windows platforms, TTY REPL editing, real large-scale circuits** — out of
   scope for this round's review, as in round 2.
6. **Whether B2's input is meant to be legal.** The contract does not say what two parameter sweeps
   in one experiment should do; I treated "accepted by check, then evaluated against the other
   analysis" as a defect regardless of the intended multi-sweep policy (fix the resolution or refuse
   the input — either closes it).
7. **`_probe`** (independent project): not built or run here.

---

## 5. Evidence index (all inside the workspace, nothing in the repo source tree)

- Inputs: `target/round3/reviewer/*.cdsl` — `fc_exact`, `power_set` (plain / nosave_expr / raw /
  fine / coarse), `repl_leak`, `sweep_two`, `sweep_two_d1`, `sweep_two_d2`, `sweep_both`,
  `sweep_op`, `rank_tie`, `legacy_avg_op`, `legacy_avg_op_dc`, `derive_const`, `derive_save_name`,
  `err_*` (12 negative cases).
- Outputs: `target/round3/reviewer/out/**` (CSV/JSON written by the real binary).
- My references: `target/round3/reviewer/analyse.js`, `analyse2.js`.
- Build/test logs: background job output for
  `cargo build -p circuit-cli --tests` and the six per-crate `cargo test` runs
  (547 passed, 0 failed); the binary is `target/round3/w-reviewer/debug/cdsl.exe`.
