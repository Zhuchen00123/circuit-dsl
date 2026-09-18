# Round 3 recon — session / CLI / REPL: analysis identity, dataset naming, output view, export

Read-only reconnaissance (task-3, owner explore-session). No cargo, git or build command was run;
every claim below is from reading the source at the cited line. The tree is **mid-migration**: the
frozen round-3 plan IR has landed in `crates/circuit-core/src/plan.rs` (731 lines, incl. `ExprIr`,
`AnalysisBinding`, `DeriveRequest`, `implicit_probes`, `result_name`) but the DSL front end, the
backend and the session/CLI have **not** been adapted to it yet (migration sites listed at the end
of §8). Line numbers are as read in
this session; files owned by other workers may move.

## 1. How the backend stamps analysis identity and dataset names

* Naming is per-analysis-kind ordinal, computed over `plan.tasks` in plan order:
  `crates/circuit-backend/src/thevenin.rs:339-347` (`per_kind: HashMap<&str,u32>`, first `ac` → 1,
  second → 2), then `Dataset::new(plan.name.clone(), format!("{kind}{ordinal}"), kind, ...)` at
  `thevenin.rs:546-560` (name literal at `thevenin.rs:548`).
* Push order: one `run_task` per task, results pushed in the same order
  (`thevenin.rs:341-347`), so `SimulationResults.datasets[i]` corresponds to `plan.tasks[i]`
  (`crates/circuit-backend/src/backend.rs:47-50`). `SimulationResults::dataset_for` looks a dataset
  up **by its stamped name** (`backend.rs:59-61`).
* The dataset carries three distinct strings: `experiment` = plan name, `analysis` = the identity
  (`ac1`), `kind` = `"op"|"dc"|"ac"|"tran"` (`crates/circuit-results/src/dataset.rs:471-484`).
* The identity is also the engine-plot-independent name: the adapter runs one analysis per call and
  selects the plot by kind prefix (`thevenin.rs:385-410`, `thevenin.rs:1075-1085`), because
  `simulate_tran` returns `[op1, tran1]`; the internal op plot is *not* a task, so a tran-only
  experiment produces exactly one dataset `tran1`, never an `op1`.
* `AnalysisId` is a dense 0-based counter assigned in experiment-body order across all kinds
  (`crates/circuit-dsl/src/elaborate.rs:1940` and `:2004-2047`); the plan now computes the stamped
  name from it (`crates/circuit-core/src/plan.rs:625-644` `result_name`, `:647-652`
  `analysis_id_by_name`, `:655-660` `analysis_names`). The two orderings agree today because
  `tasks.push` happens in body order.
* The backend's test that pins the distinct-name rule: `crates/circuit-backend/tests/adapter.rs:943-1011`,
  asserting `["ac1","ac2"]` at `adapter.rs:997`.

## 2. The default probe set (a task with an empty probe list)

* Empty `task.probes` ⇒ expose every engine vector of the selected plot, translated into DSL names:
  `thevenin.rs:493-505`; the axis vector is skipped by name comparison (`thevenin.rs:499-501`).
* Translation table (`thevenin.rs:1134-1155`): `v(x)`/`V(x)` → `("v(x)", VOLTAGE)`;
  `<device>#branch` → `("i(device)", CURRENT)` **only when the device exists in the circuit**
  (`thevenin.rs:1146-1153`); everything else (e.g. `@src[dc]`, `v-sweep`) returns `None` and is dropped.
* Axis names the skip compares against come from `build_axis` (`thevenin.rs:1441-1492`): `"time"`
  (`:1460`), `"frequency"` (`:1466`), `"v-sweep"`/`@...` for DC (`:1474-1476`), `""` for OP (`:1457`).
* Explicit probes take the other branch and are materialised one by one
  (`thevenin.rs:506-528`, `materialise_probe` `thevenin.rs:1161-1279`); a probe the engine did not
  report is an error that aborts the whole task (`thevenin.rs:510-527`, `:530-532`).
* Coverage gap: **no test in the suite exercises the empty-probe path** (grep `probes: vec![]` = 0 hits;
  `probes: Vec::new()` only in freshly migrated `plan.rs:790` and the four `elaborate.rs` task pushes),
  and all seven `examples/*.cdsl` experiments carry an explicit `save` (e.g.
  `examples/voltage_divider.cdsl:26`, `examples/rc_filter.cdsl:56`). The only observations of this
  path are indirect via `translate_default_name`.
* The backend still reads **only** `task.probes`: `thevenin.rs:231` (validate) and `thevenin.rs:495,507`
  (materialise). `AnalysisTask::implicit_probes`/`read_probes()`/`exported_names()` exist
  (`plan.rs:479-510`) but are not referenced anywhere under `crates/circuit-backend` or
  `crates/circuit-session` (grep), and `AnalysisPlan::derives` is not read anywhere.

## 3. Parameter-sweep path (execute.rs + sweep.rs)

* Detection: `execute` branches on `parameter_sweep_of(&elaborated.plan)`
  (`crates/circuit-session/src/execute.rs:107-108`, helper `:198-208`); the plan layer now duplicates
  that helper as `AnalysisPlan::parameter_sweep` (`plan.rs:667-677`).
* `sweep_experiment` (`execute.rs:211-267`): re-elaborates, checks `max_sweep_points`
  (`:227-240`), then `run_parameter_sweep` (`:248`) with a closure that pushes the swept value into
  the overrides, re-elaborates, and converts the DC task to an OP for the single point
  (`:248-258`, `as_single_point_plan` `:270-288`). The topology-invariance check lives in
  `crates/circuit-backend/src/sweep.rs:227-253` (`Code::TopologyParam`).
* **How many tasks it corresponds to**: N backend runs, one per coordinate, each producing exactly one
  OP dataset (`sweep.rs:227-267`); the plan keeps 1 DC task. `run_parameter_sweep` returns
  `SweepOutcome { coordinates, results: Vec<SimulationResults> }` (`sweep.rs:117-124`).
* The stitched dataset is built by `stitch` (`execute.rs:291-394`):
  name `format!("dc_param_{parameter}")` (`:387`), kind `"dc"` (`:388`), axis
  `Axis::Parameter(coordinates)` (`:389`), `experiment` = experiment name (`:386`), backend
  `name/version` copied from the first point plus settings `sweep`/`points` (`:309-311`).
  Every point must report the same signal names in the same order (`:315-345`) and each signal
  exactly one sample (`:356-368`); a complex sample is `Code::Unsupported` (`:370-379`).
* Consequence for identity: this experiment's single dataset is **not** named by the
  `{kind}{ordinal}` rule — `result_name(AnalysisId(0))` would say `dc1` (`plan.rs:625-644`) while the
  artifact is `dc_param_r`. Both names are pinned by tests: `crates/circuit-cli/tests/e2e.rs:534`
  (`sweep.dc_param_r.csv`) and `crates/circuit-session/tests/session.rs:463` (`dc_param_r`).
* `execute` returns exactly one dataset for this shape (`:125-130` rejects an empty result;
  `run.datasets` at `:121`).

## 4. execute / output_view / evaluate_measures / write_datasets

* `RunOutcome` (`execute.rs:34-43`): `datasets` (raw solver grid), `output_datasets` (display/export
  view, same order and names), `measures`, `warnings`. `summaries()` renders the **output** view and
  prints `d.analysis` (`execute.rs:50-72`, format at `:60-70`).
* Order of operations inside `execute` (`:95-147`): backend run (`:110`) → param override metadata
  (`:113-120`) → non-empty check (`:125-130`) → warnings from dataset diagnostics (`:132-135`) →
  **measures on the raw datasets** (`:138`) → **output view / resampling** (`:139`). So a coarse
  `output_interval` can never move an `avg`/`rms`/`max`/`min` (comment `:27-33`, `:136-137`).
* `output_view` (`:155-195`) collects one `output_interval` per TRAN task in plan order
  (`:163-170`) and finds a dataset's own interval by **parsing the dataset name text**:
  `dataset.analysis.strip_prefix("tran")` then `parse::<usize>()` (`:174-186`), falling back to
  ordinal 1. Non-tran datasets are cloned unchanged (`:191`).
* `evaluate_measures(plan, datasets) -> Vec<Measured>` (`:404-435`) is today **infallible and
  silent**: it sorts datasets by `measurement_rank` (TRAN 0, AC 1, DC 2, OP 3 — `:437-445`,
  stable sort so ties keep dataset order), and for each `MeasureRequest` it walks the ordered
  datasets; a missing signal `continue`s (`:420-422`) and a failing reduction `continue`s (`:429-431`).
  A measure that no dataset can satisfy is **dropped without a diagnostic** and the out-of-range
  case never reaches a caller.
* `write_datasets(out, format, datasets, approve)` (`:493-531`): `create_dir_all`, then per dataset
  stem `{sanitise(experiment)}.{sanitise(analysis)}` (`:508`), calling `to_csv`/`to_json`
  (`:511-517`), `approve(&path)?` before **each** `std::fs::write` (`:519-528`). One file per dataset
  per format; writing is incremental, so a refusal or I/O error on the k-th file leaves 1..k-1 files
  on disk. `sanitise` maps everything outside `[A-Za-z0-9_-]` to `_` (`:458-468`).
* Export serialisation itself: CSV header = axis kind name + one column per signal (complex → `_re`/`_im`,
  quoted when it contains `,`, which a differential probe name does) — `crates/circuit-results/src/export.rs:64-109`,
  `:120-128`, `:135-141`; JSON keys `schema/experiment/analysis/kind/axis/signals/backend/diagnostics`
  — `export.rs:175-197`. A derived signal reaches a file only if it is appended as a `Signal` of a
  dataset that is passed to `write_datasets`.
* `RunOutcome` consumers:
  * `crates/circuit-session/src/session.rs:594-659` — `datasets[0].backend` (`:599`),
    `summaries()` (`:607`), `output_datasets` (`:615`, `:651`), `warnings` (`:636`),
    `measures` (`:639`).
  * `crates/circuit-cli/src/run.rs:76-140` — `output_datasets` (`:109`), `datasets[0]` (`:149`),
    `summaries()` (`:157`), `warnings` (`:160`), `measures` (`:163`).
  * Re-exported at `crates/circuit-session/src/lib.rs:18`.
  * Tests: `crates/circuit-session/tests/tran_output_interval.rs:51,155,218` (helper type, helper
    return, `measure_of`) and the four tests using the fields (§8).
* `Measured` today is `{ name, value, unit }` (`crates/circuit-results/src/measure.rs:74-103`), with
  `render()` = `"name = value unit"` (`:95-102`). It carries **no analysis identity**, so neither
  `run.rs:164` nor `session.rs:643-646` can say which analysis a value came from; only the dataset's
  own name is printed elsewhere.
* `measure_signal` builds `Expr::signal(name)` and evaluates against a dataset
  (`measure.rs:118-125`); the evaluator's missing-signal error already names the analysis
  (`crates/circuit-results/src/expr.rs:309-328`), and `measure` passes `dataset.analysis` into
  `reduce` (`measure.rs:106-114`, `:131-142`), so identity is available at the failure site — it is
  the *success* path that drops it.

## 5. Session / REPL

* `Session::run` (`crates/circuit-session/src/session.rs:567-660`): name check (`:573-585`) →
  `RunRequest` (`:588-593`) → `execute::execute` (`:594`) → message assembly: header with
  backend name/version (`:597-600`), overrides (`:601-606`), `summaries()` (`:607-609`), then
  either OP scalars printed as `"  {signal.name} = {value} {unit}"` **with no analysis label**
  (`:615-627`, only for `Axis::None` datasets) or `"  {d.analysis} has N points; pass --out <dir>"`
  (`:628-635`), warnings as message lines (`:636-638`), then `"  measure {name} = {value unit}"`
  rendered with `circuit_core::format_quantity` (`:639-647`) — deliberately **not**
  `Measured::render()` (comment `:640-641`). With `--out` it writes `Format::Both` through
  `execute::write_datasets` with a no-op `approve` (`:649-657`).
* Errors: a command/definition/run failure returns `Err(Diagnostics)` from `feed` and the REPL prints
  it to **stderr** (`crates/circuit-cli/src/repl.rs:100-107`); a `Reply::Message` (all run output,
  including warnings) goes to **stdout** (`repl.rs:95-98`). A piped session records `Step::Failed` and
  exits 1 at end of input (`repl.rs:112-148`, `:144-148`).
* What showing analysis identity would touch: the header at `:597-600` (one identity for the run is
  currently inferred from `datasets[0]`), the OP scalar lines at `:621-626` (need the dataset's
  `analysis`, otherwise two OP datasets with the same signal name print identical lines), and the
  measure lines at `:642-646` (need `Measured.analysis`/`render_with_analysis`). `:help` text
  (`:337-356`) and `completions` (`:720-736`) do not mention analyses or derive/analysis keywords.
* `COMMANDS` is a fixed list (`:70-72`); anything else is language input (`:75-78`), so a REPL
  surface for analysis identity must be an existing command's output or a new command added there.

## 6. CLI

* Exit codes: `EXIT_OK=0`, `EXIT_USER_ERROR=1`, `EXIT_INTERNAL=2` (`crates/circuit-cli/src/main.rs:24-26`),
  dispatched at `:103-125`. **`EXIT_INTERNAL` is declared and never returned** (grep: no other use), so
  the documented "2 for an internal failure" (`main.rs:9`) is currently unreachable.
* `check` prints human output: file/definition counts (`crates/circuit-cli/src/check.rs:86-91`), one
  line per circuit (`:92-94`), then per experiment `tasks.len()` and `measures.len()` (`:95-102`)
  and one line **per task** with `t.kind.name()` and the saved probes, or empty when there are none
  (`:103-114`). It shows **no analysis identity** (`AnalysisId`/`ac1`) anywhere; the JSON dump has
  `analyses: [{kind, probes}]` and `measures: [[name, kind, target_name]]` and likewise no id
  (`:162-168`). Backend validation runs inside the front end for every experiment
  (`check.rs:55-65`); a requirement on `avg`/`rms` having a time axis is *not* checked here
  (`needs_time_axis` is referenced only in tests: `crates/circuit-dsl/tests/elaborate.rs:1020-1021`).
* `run` (`crates/circuit-cli/src/run.rs:17-141`): front end → experiment choice (errors `E_NAME`/`E_ARGUMENT`,
  `:29-70`) → `execute` with `overrides: &[]` and `Limits::default()` (`:76-94`) → format map
  (`:97-101`) → `write_datasets` guarded by `guard_output` (`:102-130`) → summary (`:132-140`,
  `print_summary` `:143-169`).
* Measures are printed to **stdout** at `run.rs:163-165` via `m.render()`; warnings to **stderr**
  (`:160-162`). Measures are printed **after** the files were written (`:105` vs `:163`).
* Does a failed measure abort before files are written? **Today a failed measure cannot fail the run**:
  `evaluate_measures` returns a plain `Vec` and silently drops it (`execute.rs:404-435`), and `run`
  writes whatever `output_datasets` holds (`run.rs:105-130`). Concretely,
  `measure :m, avg: v(:out)` in an OP-only experiment passes `check` (counted at `check.rs:101`) and
  produces no measure line and exit code 0. There is therefore **no code path today that stops the
  export because of a measure**, which is exactly what the contract's §5 ("a runtime expression/measure
  failure aborts the run before any file is written") changes.
* Partial-output hazard in the existing refusal path: `write_datasets` writes incrementally
  (`execute.rs:519-528`) while the guard is evaluated per file (`run.rs:110-119`,
  `main.rs:142-156`). When the k-th path is refused, `run.rs:121-123` returns `EXIT_USER_ERROR`
  immediately after the call, so datasets 1..k-1 are already on disk. A pre-write validation pass is
  the only way to make "no partial export exists" true for both failures and refusals.
* `capabilities` prints "parameter DC sweep: yes (one elaboration per point, via the CLI)" whenever
  `caps.parameter_sweep` is false (`check.rs:188-195`), i.e. it does not echo the flag
  (`thevenin.rs:103` declares `parameter_sweep: false`).
* Defaults: `--out results` (`main.rs:67`), `--format both` (`main.rs:70-71`).

## 7. Where a derived signal can reach the user (export facts)

* Output files: one per (experiment, analysis) per format, stem `{experiment}.{analysis}`
  (`execute.rs:508`); e.g. `divider.op1.csv`, `response.tran1.csv`, `response.ac1.csv`,
  `forward_drop.dc1.csv`, `sweep.dc_param_r.csv` — all pinned by CLI tests (§8).
* Therefore a *named derived signal* is visible only as a column/JSON signal of the dataset it was
  appended to; there is no per-signal file path and no per-signal name in the JSON root
  (`export.rs:175-197`).
* The REPL always writes both formats (`session.rs:651`); the CLI honours `--format` (`run.rs:97-101`).

## 8. Tests that depend on these surfaces

Analysis identity / dataset naming:
* `crates/circuit-backend/tests/adapter.rs:942-1011` — `two_analyses_of_the_same_kind_get_distinct_names`
  (asserts `["ac1","ac2"]` at `:997`, and 51/11 points at `:1010`).
* `crates/circuit-cli/tests/e2e.rs:246` (`divider.op1.csv`), `:290` (`response.tran1.csv`),
  `:346`/`:414` (`response.ac1.csv`), `:494` (`forward_drop.dc1.csv`),
  `:534` (`sweep.dc_param_r.csv`, 8 rows at `:550`), `:605` (`divider.op1.csv` exists),
  `:626-629` (`response.tran1.json`, `parsed["analysis"]=="tran1"`, `["kind"]=="tran"`).
* `crates/circuit-cli/tests/repl.rs:233` (`response.tran1.csv`).
* `crates/circuit-session/tests/session.rs:462-463` ("4 sweep points", `dc_param_r`).
* `crates/circuit-session/tests/tran_output_interval.rs:318` (`raw.analysis=="tran1"`).
* `crates/circuit-backend/tests/adapter.rs:1069-1109` — `only_requested_probes_are_returned`
  (explicit probes subset the result; the complementary default-set case is untested, §2).

RunOutcome / evaluate_measures / measures:
* `crates/circuit-session/tests/tran_output_interval.rs:51` (import), `:150-164` (`run` helper returns
  `RunOutcome`), `:218-234` (`measure_of` reads `outcome.measures` by name and `.value`),
  `:302-445` `the_output_view_is_the_requested_grid_and_the_raw_view_is_untouched`
  (`datasets`/`output_datasets` at `:315-318`, `:414`),
  `:446-526` `measurements_are_computed_on_the_raw_grid` (`.measures` at `:458`, `:462`, `:472`),
  `:527-573` `an_absent_output_interval_does_not_resample` (`:498`, `:535`),
  `:574-627` `an_oversized_output_grid_is_rejected_and_truncates_nothing` (`:587`, `:606`).
* `crates/circuit-cli/tests/e2e.rs:381-394` — CLI stdout must contain `"measure vfinal = 0.9"` and
  `"measure vavg = 0.80"` (i.e. `Measured::render()`'s exact shape).
* `crates/circuit-results/src/lib.rs:108-149` `pipeline_from_dataset_to_files` (measure + CSV header
  `:139` + JSON `analysis` `:144`).
* `evaluate_measures` itself has **no direct test** (only the call at `execute.rs:138`), so changing
  its return type breaks no test directly — the four tests above are the real pins, via `RunOutcome`.
* `Measured`-shape tests: `crates/circuit-results/src/measure.rs:296-591` (unit/value assertions) and
  `crates/circuit-results/src/measure.rs:452-474` (`avg` on OP reports `Type` and names `op1`).

Migration sites that construct the changed structs (mid-round-3 state):
* `crates/circuit-dsl/src/elaborate.rs:1992-2000` (`MeasureRequest { name, kind, target,
  target_name, span, kind_span }` — the new struct has `expr`/`binding`/`source`, `plan.rs:586-598`),
  `:2004-2046` (four `AnalysisTask` literals without `implicit_probes`), `:2097-2104`
  (`AnalysisPlan` literal without `derives`).
* `crates/circuit-session/src/execute.rs:404-435` (reads `m.target`/`m.target_name`, which no longer
  exist), `:274-287` (`as_single_point_plan` copies `t.probes` but not `implicit_probes`),
  `:541-569` and `:622-629` (test literals).
* `crates/circuit-backend/tests/adapter.rs:124-131`, `:980-991`, `:1039-1049`;
  `tests/transient_reference_regression.rs:199-206`; `tests/source_breakpoint_regression.rs:206-213`;
  `tests/phase_regression.rs:256-263`; `tests/output_interval_regression.rs:180-187` — all struct
  literals for `AnalysisTask`/`AnalysisPlan` without the new fields.
* Not verified by compilation (no cargo run was permitted); the sites above were found by grep for
  `AnalysisTask {` / `AnalysisPlan {` / `MeasureRequest {`.

## Interface recommendations

1. **Freeze the analysis identity of a parameter-sweep experiment.** The stitched artifact is
   `dc_param_{parameter}` with kind `"dc"` (`execute.rs:386-393`), while `AnalysisPlan::result_name`
   computes `dc1` for its single DC task (`plan.rs:625-644`). Decide explicitly whether
   `analysis: :dc1` (a) resolves to that stitched dataset, (b) is rejected with `Unsupported`, or
   (c) `result_name` returns `dc_param_r` for sweep plans. Tradeoff observed: option (c) is the only
   one that makes one identity serve both display and file names, but it must keep
   `sweep.dc_param_r.csv` (`e2e.rs:534`) and the session's `dc_param_r` summary
   (`session.rs:463`) intact, and it must also state what `RunOutcome.bindings` reports for the
   N-per-point runs behind that one dataset (`sweep.rs:227-267`).
2. **Freeze the display seam for analysis identity.** The CLI renders measures with
   `Measured::render()` (`run.rs:164`) and the REPL with `format_quantity` (`session.rs:643-646`);
   OP signals print with no analysis label at all (`session.rs:621-626`) even when two OP datasets
   carry the same signal name. Either both surfaces switch to `render_with_analysis`, or the identity
   is CLI-only. Tradeoff: the CLI strings `"measure vfinal = 0.9"` and `"measure vavg = 0.80"` are
   asserted verbatim (`e2e.rs:381-394`); changing the seam changes those, and no session test pins the
   REPL measure line, so the REPL is the cheap surface to change and the CLI the expensive one.
3. **Freeze how derived signals are attached and what stays out of the export when there is no `save`.**
   Files are one-per-(experiment, analysis) and carry only `dataset.signals` (`execute.rs:508`,
   `export.rs:75-77`, `:188-191`), so a derive must be appended to a dataset. With empty `save` the
   backend currently builds the *default* set from every engine vector (`thevenin.rs:493-505`) and the
   backend never sees `implicit_probes` (`plan.rs:479-510` unreferenced outside core). Decide whether
   implicit probes are excluded by making the backend read `read_probes()` and filter by
   `exported_names()`, or by filtering after the run in `execute`. Tradeoff: the former changes the
   backend contract and its capability comments; the latter keeps the adapter untouched but needs the
   default text set to be recoverable, which today only the backend knows.
4. **Replace the name-string coupling in `output_view` with an explicit task→dataset binding.** The
   resampler finds a TRAN dataset's interval by parsing `dataset.analysis`
   (`execute.rs:174-186`), which is currently the only mapping from a dataset back to its task. The
   contract's `RunOutcome.bindings: Vec<(AnalysisId, usize)>` is exactly that mapping; using it (and
   keeping `{kind}{ordinal}` names stable) removes a class of silent mismatch if derives ever change
   dataset ordering. Tradeoff: the string parse works today and is pinned by
   `tran_output_interval.rs:302-445`; switching costs one migration in `execute.rs` and must keep the
   raw/output vectors in the same order and with the same names (documented at `execute.rs:37-39`).
5. **Freeze failure atomicity before the first write, including the existing refusal path.** The
   contract promises "no partial export exists", but `write_datasets` writes incrementally
   (`execute.rs:507-529`) and the CLI's overwrite guard runs per file, returning after earlier files
   were already written (`run.rs:102-130`, `main.rs:142-156`). Decide that all expression/measure
   evaluation (and, if cheap, all path approvals) happen before the first `std::fs::write`, so the
   0-file guarantee holds for runtime failures and refusals alike. Tradeoff: a pre-check pass over
   paths duplicates a little logic in `run.rs`, whereas leaving it means the guarantee is only true
   for the new failure mode the contract introduces.
