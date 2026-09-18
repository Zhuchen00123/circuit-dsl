# Round 3 front-end reconnaissance: parser / AST / elaborate entry points for `derive` and expression measures

Owner: explore-frontend (read-only). Task: `task-1`. This file is the only file this agent wrote.

## 0. Snapshot and scope

Two revisions were visible while observing, because workstreams 1 and 3-4 are landing in parallel:

- `crates/circuit-core/src/plan.rs` (922 lines at observation) **already contains the frozen round-3 IR**:
  `ProbeRef` (58-81), `ExprIr` (90-112), `ExprIr::probes/static_dimension/static_dimension_error/is_statically_real/render` (139-308),
  `AnalysisTask::implicit_probes` + `read_probes()` + `exported_names()` (474-511), `AnalysisBinding` (553-569), `DeriveRequest` (571-582),
  `MeasureRequest { expr, binding, source, span, kind_span }` (584-598), `AnalysisPlan::derives` (600-615), `result_name` / `analysis_id_by_name` (622-652).
  `crates/circuit-core/src/lib.rs:38` re-exports `AnalysisBinding`, `DeriveRequest`, `ExprIr`.
- The DSL crate is **still at the pre-round-3 revision**: no `derive` anywhere in `crates/circuit-dsl` (grep for `derive|ExprIr|AnalysisBinding` finds nothing there),
  and `elaborate.rs:1992-1999` still builds the *old* `MeasureRequest { target, target_name }`, while `plan.rs:584-598` no longer has those fields.
  Same mismatch in `crates/circuit-session/src/execute.rs` (`m.target` / `m.target_name`) and `crates/circuit-cli/src/check.rs:167` (`m.target_name`).
  So the workspace is mid-refactor by construction; I did not run cargo (out of scope for this task), so this is a code-reading fact, not a measured build failure.

Line numbers below are from that snapshot. `crates/circuit-session/src/execute.rs` grew from 639 to ~850 lines *during* this reconnaissance, so session lines are the least stable; DSL lines were re-checked immediately before writing.

## 1. How `save` and `measure` are lexed and parsed

### 1.1 There is no keyword token

- `token.rs:1-8` states the design: statement keywords arrive as `TokenKind::Ident`; the parser dispatches on text at statement position.
- `token.rs:188-220` is the single keyword table: `save` at 209, `measure` at 210. `is_keyword` is `token.rs:227-229`.
- The only consumer of `is_keyword` is the REPL assignment-name check `parser.rs:671` (`is_keyword(&name) || is_reserved_name(&name)`);
  `RESERVED_IN_NAME_POSITION` is a *separate* list (`token.rs:225-233`, `is_reserved_name` 231-233) and blocks declared names (`parser.rs:671`, `parser.rs:1065`).
- `lexer.rs:195-203` (`ident`) emits `Ident`; `save`/`measure` take no special lexer path.
- Newline/continuation: `lexer.rs:143-163` drops a newline after any token where `TokenKind::is_continuation_after` is true (`token.rs:89-108`), which includes `Comma` and `Colon`. That is what lets `measure :m,` be broken across lines.
- `:gain` is `TokenKind::Symbol` via `lexer.rs:205-218`; the dotted-path join for `:stage1.r1` is in the *parser* atom (`parser.rs:1499-1522`), not the lexer.

### 1.2 Dispatch chain

- `parser.rs:553-615` `program` -> `Some("experiment")` at 572 -> `experiment` (`parser.rs:1204-1268`, body via `exp_block` 1270-1296).
- `parser.rs:1298-1404` `exp_stmt` is the dispatch table: `op` 1310, `dc`/`ac`/`tran` 1315-1317, `save` 1318-1325, `param` 1326-1347, `measure` 1348-1372, unknown 1373-1402.
- `save`: `bump()` then `expr_list` (`parser.rs:531-549`, comma-separated `self.expr()`, trailing comma allowed) then `finish_stmt` (`parser.rs:350-366`); span = keyword.merge(prev_end) (1322).
- `measure`: `expect_symbol` for `:name` (`parser.rs:374-404`), mandatory `Comma` (1353), `skip_newlines` (1360), `arg_label` (`parser.rs:407-428`) for the kind word, then `self.expr()` (1362), then `finish_stmt` (1364).
  Only **one** labelled argument is accepted, and it must be the measure kind: a second `, analysis: :ac1` is rejected by `finish_stmt` with 'unexpected `,`; expected the end of the statement' (`parser.rs:360-365`).
- Expression entry point for that value: `expr` = `binary(1)` (`parser.rs:1417-1419`), `binary` 1423-1458, `unary` 1460-1478, `atom` 1480-1578, `call` 1681-1741 (`gain_db(v(:vout), v(:vin))` parses as a plain call with two positional args).
- AST nodes: `ExpStmt::Save { probes, span }` `ast.rs:252-256`; `ExpStmt::Measure { name: SpannedName, kind: String, kind_span, target: Expr, span }` `ast.rs:263-271`; `ExpStmt::span()` `ast.rs:274-283`; `ExpStmt::keyword()` `ast.rs:285-295`.

### 1.3 REPL-side lists that name these statements

- `parser.rs:123-131` `body_only_keyword`: `"op" | "dc" | "ac" | "tran" | "save" | "measure" => "experiment"` (line 127). Used at `parser.rs:695-717`.
- `parser.rs:1393-1401` unknown-statement message lists the allowed experiment statements (1394-1397).
- `complete.rs` has **no keyword table**: its structural scan only knows `do`/`end` (`complete.rs:125-129`), and incompleteness is decided by parser failure plus trailing-token class (`complete.rs:64-91`, 100-189).
- Completion names come from `circuit_session::completions` (`crates/circuit-session/src/session.rs:720-736`) which lists commands, circuit names, experiment names and variables - no statement keywords.

## 2. How elaboration turns them into the plan (pre-round-3 code)

- Entry points: `compile(program, limits)` `elaborate.rs:88-130`; public `elaborate_experiment(program, name, overrides, limits)` `elaborate.rs:137-170` (used by the session and by the sweep driver); inner `Elaborator::elaborate_experiment(def, circuit)` `elaborate.rs:1932-2105`.
- Error accumulation: `Elaborator::error` `elaborate.rs:276-279` bumps `error_count` (`declared` at 243-244). `compile` returns `Err(self.diagnostics)` when `diagnostics.has_errors()` (122-129); the inner elaborator returns `None` when `self.error_count > 0` (2093-2095).
  Note the counter is **cumulative for the whole file**: an error in experiment 1 makes every later experiment's plan `None` too (117-119 iterates and pushes only `Some(plan)`).
- `experiment_overrides` `elaborate.rs:1890-1930`: collects `ExpStmt::Param` values, last one for a name wins (1903-1911).
- Inner loop `elaborate.rs:1955-2049`:
  - `Save` arm 1959-1971: a second `save` in one experiment is `Code::Duplicate` 'an experiment may have only one `save` statement' (1961-1968) and is dropped (`continue`), so the **first** save wins; the AST expressions are stored in `pending_probes` (1970).
  - `Measure` arm 1973-2001: `MeasureKind::parse` (called at `elaborate.rs:1980`, defined at `crates/circuit-core/src/plan.rs:534-542`; unknown kind -> `Code::Unsupported` + note 'available: max, min, avg, rms' 1981-1989), then `resolve_probe_expr(target, circuit, &mut [])` (1991) and the old `MeasureRequest` push (1992-1999).
  - analysis arms 2003-2047: each successful `*_spec` pushes `AnalysisTask { id: AnalysisId(id), ... }` and `id += 1`; a task whose spec fails consumes **no** id.
  - 'declares no analysis' error 2051-2061.
- Probe resolution happens **after** the whole task list exists, once: 2063-2087. Dedupe is a `HashSet<String>` on the resolved name (`p.name`), duplicate -> `Code::Duplicate` 'probe `X` is saved twice' (2071-2080).
- Then the same `Vec<NamedProbe>` is cloned into **every** task: 2089-2091 (comment cites spec 8.5 'one `save` applies to all analyses').
- Plan literal 2097-2104 (no `derives` field existed then).

### 2.1 Probe resolution primitives (reusable by `derive` lowering)

- `probe_target` `elaborate.rs:2112-2138`: accepts only `ExprKind::Symbol` or `ExprKind::Str`; anything else -> `Code::Type` '`v` takes node symbols...'; empty name -> `Code::Value`.
- `resolve_probe_expr` `elaborate.rs:2141-2299`: non-call -> `Code::Type` 'a probe must be `v(:node)` or `i(:device)`' (2147-2153);
  - `v`: 1 node -> `NamedProbe { name: "v(<node>)", probe: Probe::NodeVoltage }` (2204-2208); 2 nodes -> `name: "v(a,b)"`, `Probe::DifferentialVoltage` (2209-2213); other arity -> `Code::Argument` (2214-2226).
  - `i`: exactly one device arg -> `name: "i(<dev>)"`, `Probe::DeviceCurrent` (2230-2288).
  - any other function name -> `Code::Name` 'unknown probe' + note 'available probes: v(node), v(a, b), i(device)' (2290-2297).
- Name lookup: `node_lookup` `elaborate.rs:2836-2851` (`gnd`/`0` -> ground; full name; else unique leaf name), `device_lookup` 2868-2880, `ambiguous_paths` 2856-2865 (two matches -> `Code::Name` listing full paths).
- These take `&Circuit`, so any `derive` lowering that must resolve probes can reuse them verbatim - but only after the circuit for the experiment exists (`compile` 114-117).

### 2.2 Gaps that matter for round 3

- **No duplicate check for measure names**, and no collision check between a measure name and a saved probe name: 1973-2001 pushes unconditionally.
- **`measure` names are used as `name.name` without `resolve_name`** (1993). `resolve_name` (`elaborate.rs:287+`) is only used for circuit-body names; a computed name (`measure ("m" + i), ...`, `ast.rs:71-77` sets `name = ""` for `SpannedName::expressed`) therefore silently elaborates to the empty string. Same at 1910 and 1948 for `param` overrides.
- `self.eval` (`elaborate.rs:1536-1540`) must **not** be pointed at a derive/measure expression: `eval.rs:372-377` rejects `v`/`i` with '`v` can only be used in `save` and `measure`', and `gain_db` is not in the evaluator's function table (`eval.rs:335-389`, note the 'available:' list at 386).

## 3. What `derive :name, expr: ..., analysis: :ac1` needs at the front end

### 3.1 Registration sites for a new statement keyword (complete list found by grep)

1. `token.rs:188-220` `KEYWORDS` - adding `"derive"` only changes `is_keyword`, whose sole use is `parser.rs:671` (REPL `name = ...` target). It does **not** reserve the word as a declared name (`is_reserved_name`, `token.rs:225-233`).
2. `parser.rs:123-131` `body_only_keyword` - add `"derive"` to the `"experiment"` arm so a bare `derive` typed at the prompt gets the 'is a body statement' hint (`parser.rs:695-717`).
3. `parser.rs:1298-1404` `exp_stmt` - add a `"derive"` arm; the fallback message at 1394-1397 should list it.
4. `ast.rs:245-272` `ExpStmt` - new variant; extend `span()` 274-283 and `keyword()` 285-295.
5. `docs/language.md:23-33` (keyword list) and `docs/language.md:307-316` (experiment statement grammar) are the documentation half; `docs/language.md:372-387` (§5.2 probes) and 461-477 (§7 measurements) describe today's behaviour.
6. Nothing in `complete.rs` needs to change (no keyword table, 1.3 above).

### 3.2 Grammar plumbing that already works and that does not

- `derive :gain` -> `expect_symbol` (`parser.rs:374-404`) works unchanged; `expr:` / `analysis:` -> `arg_label` (`parser.rs:407-428`) + `self.expr()` (`parser.rs:1417-1419`) work unchanged.
- Optional **trailing** `analysis:` does *not* work with the current hand-rolled shape: `measure` reads exactly one label then calls `finish_stmt` (1353-1364). A loop in the style of `arg_list` (`parser.rs:445-461`) is required, or a hand-rolled 'if eat(Comma) then arg_label ...'. `arg_list_open` (465-487) cannot be reused because it demands a label before the first comma.
- The `expr:` value is a generic `ast::Expr` (`ast.rs:416-420`, every node spans) - it must be lowered to `circuit_core::plan::ExprIr` (`plan.rs:90-112`), not evaluated.

### 3.3 Lowering map `ast::Expr` -> `ExprIr` (observable rules from the code)

| AST (`ast.rs:386-409`) | ExprIr (`plan.rs:90-112`) | note |
|---|---|---|
| `Int`/`Float` | `Number(f64)` | dimensionless by construction |
| `Quantity(q)` | `Number(q.value)` | only if `q.dimension.is_dimensionless()` (contract rejects other dimensions) |
| `Str` / `Symbol` / `Var` / `Bool` / `Array` / `Dict` | reject | contract: bare identifiers, strings, arrays, dicts are Type errors |
| `Call` named `v`/`i` | `Probe(ProbeRef)` via `resolve_probe_expr` (`elaborate.rs:2141-2299`) then convert `NamedProbe` -> `ProbeRef` (`plan.rs:58-72`) | positional args must be Symbol/Str (`elaborate.rs:2112-2138`) |
| `Call abs/sqrt/min/max/gain_db` | `Abs/Sqrt/Min/Max/GainDb` | matches the runtime AST (`crates/circuit-results/src/expr.rs:115-145`); `expr.rs:176-181` builds `GainDb` |
| `Unary Neg` | `Neg` | `Unary Pos` -> identity; `Unary Not` -> reject |
| `Binary Add/Sub/Mul/Div` | `Add/Sub/Mul/Div` | `Eq/Ne/Lt/Le/Gt/Ge/And/Or` -> reject (`ast.rs:340-353`) though the parser accepts them today |
| any other call name | `Code::Name` listing functions | mirror `eval.rs:386`'s wording; contract wants 'unknown function in a result expression' = `Name` |

Spans: `ExprIr::span()` (`plan.rs:116-132`) merges child spans but `Number` is `SourceSpan::synthetic()` (118), so a literal's written position is only available from the AST. `DeriveRequest.source` / `MeasureRequest.source` (`plan.rs:578-579`, 594-595) are `String`s: `compile(program, limits)` (`elaborate.rs:88`) has **no `SourceMap`**, so elaboration cannot slice the original text; the only text available at that layer is structural rendering (`ExprIr::render`, `plan.rs:290-308`).

### 3.4 Dependency collection and binding

- `ExprIr::probes()` `plan.rs:139-171` is the automatic dependency set (distinct by name, first-seen order). It must land in the bound task's `implicit_probes` (`plan.rs:482-500`): `read_probes()` = explicit `save` probes first, then implicit ones, deduped by name; `exported_names()` is `None` when there is no explicit `save` (`plan.rs:502-510`).
- Today nothing consumes `implicit_probes` or `read_probes`: the backend reads only `task.probes` (`crates/circuit-backend/src/thevenin.rs:495-505` for the 'no explicit probe list: expose everything' default, 506-528 otherwise), and `materialise_probe` names each signal `probe.name` (1176-1180 node, 1191-1195 differential, 1236/1250 current).
- Binding identifiers: ids are assigned in source order to *successful* tasks only (`elaborate.rs:2004-2047`), the backend stamps `{kind}{ordinal}` per kind (`thevenin.rs:339-345` and `format!("{kind}{ordinal}")` at 548), and `plan.rs:625-652` (`result_name`, `analysis_id_by_name`) re-derives the same identity. `:ac` without an ordinal is not derivable from either.
- Static checks already exist on the new IR: `static_dimension` (187-208), `static_dimension_error` (212-267), `is_statically_real` (273-287), `is_plain_probe` (178-180) - the legacy-binding rule keys on `is_plain_probe` plus `AnalysisBinding::LegacyPreferred` (`plan.rs:553-569`).

## 4. Existing tests that assert today's parse/elaborate behaviour

Will need editing when a new statement kind / new plan fields land:

`crates/circuit-dsl/src/parser.rs` (inline tests):
- `experiment_statements_cover_every_documented_form` 2519-2594: `body.len() == 11` (2539), the exact keyword vector (2541-2547), `ExpStmt::Measure` destructure incl. `target` (2576-2584), `ExpStmt::Save` destructure (2586-2593).
- `save_probe_expressions_keep_their_spans` 2597-2613 (snippet assertions 2606-2612).
- `malformed_headers_are_reported_once_and_recovered` 2772-2816: the `save` message (2808-2811) and the `measure` message 'expected `, <kind>: <probe>` after `measure :name`' (2812-2815) - the latter text must change if the head becomes `<kind>: <result-expr>`. Both strings live at `parser.rs:1351-1358`.
- `interactive_mistakes_name_their_fix` 2944-2965 (body-statement hints 2945-2946).
- `a_dotted_symbol_is_one_name_covering_the_whole_path` 3024-3059 (save probes 3029-3034).

`crates/circuit-dsl/tests/elaborate.rs` (helpers `ok`/`err` at the top; 1621 lines):
- `voltage_divider_elaborates` 105-152 - probe count and exact names `["v(vin)", "v(vout)", "i(r1)"]` (144-151).
- `rc_filter_example_elaborates` 156-257 - 'probes must reach every task' (252-256).
- `differential_probe_is_typed_as_such` 261-277 - name `v(a,b)` and `Probe::DifferentialVoltage` (274-276).
- `an_experiment_without_an_analysis_is_reported` 918-930 (save-only experiment, line 925).
- `measurements_resolve_their_target` 999-1022 (four measures, 1016-1021).
- `an_unknown_measurement_kind_is_reported` 1025-1038 (asserts note 'available: max, min, avg, rms', 1037).
- `a_probe_on_an_unknown_node_is_reported_with_the_available_ones` 1041-1054, `a_probe_on_an_unknown_device_is_reported` 1057-1070,
  `saving_the_same_probe_twice_is_reported` 1073-1085 (`Code::Duplicate`).
- hierarchy-path tests that go through `save`: 1351, 1375, 1401, 1430, 1445; and other `save` fixtures at 181, 1182, 1383/1387, 1418, 1436, 1450, 1581, 1616.

Outside the DSL crate (these break because the plan fields changed, even though the files belong to other workstreams):
- `crates/circuit-session/src/execute.rs`: `evaluate_measures` currently reads `m.target` / `m.target_name` (observed 615-646 at snapshot) and the test `measure_request_round_trips_through_the_mapper` builds the old literal (observed ~982-1000; the file was still being edited when this report was written).
- `crates/circuit-cli/src/check.rs:167` prints `[m.name, m.kind.name(), m.target_name]`; `check.rs:97-102` counts measures.
- `crates/circuit-cli/tests/e2e.rs:76-95` (check --json shape), 385 and 391 (measure output lines); `crates/circuit-cli/tests/repl.rs:86`, 160 (`save` fixtures).
- `crates/circuit-session/tests/session.rs:46`, 284-298, 313-317, 332, 453 (`save` fixtures).
- `crates/circuit-dsl/tests/tran_option_validation.rs:274`, `crates/circuit-dsl/tests/phase_syntax_regression.rs:97` (fixtures with `save`).
- `crates/circuit-backend/tests/*` build `AnalysisPlan`/`AnalysisTask` literals directly (adapter.rs:120-135, 980, 1039, 1045; output_interval_regression.rs:176-190; phase_regression.rs:252-266; source_breakpoint_regression.rs:202-216; transient_reference_regression.rs:195-209) - the contract allows mechanical field additions there.

Not covered by any test today (grep found no assertion):
- the single-`save` rule (`elaborate.rs:1959-1971`) - the message text at 1964 appears nowhere in tests;
- duplicate measure names / measure-vs-save collisions - no code and no test;
- `derive` anything (does not exist).

## 5. Compatibility risks (facts, not predictions)

1. **One `save` per experiment, first wins** (`elaborate.rs:1959-1971`, `Code::Duplicate`, `continue`). Any new per-analysis read set must keep this and must keep the dedupe-by-name rule at 2068-2080.
2. **Probes are copied to every task** (2089-2091). With no explicit `save`, `probes` is empty and the backend exports *everything it solved* (`thevenin.rs:495-505`), which is exactly the pre-existing 'no `save`' behaviour asserted in `plan.rs:899-906` (`exported_names() == None`). Folding implicit probes into `probes` instead of `implicit_probes` would silently narrow that default export set.
3. **A task with no `save` can still need extra reads**: `read_probes()` (`plan.rs:492-500`) is the only place that expresses 'read these, export only the default/selected set' - and no backend consumes it yet (`thevenin.rs:495-528` reads `task.probes` only).
4. **Name resolution order and error accumulation**: probes are resolved after all tasks exist (2063-2087) but measures are resolved inside the statement loop (1991), i.e. measures are resolved *before* probes. `error_count` is file-cumulative (`elaborate.rs:243-244`, 276-279) and gates the whole plan (2093-2095), so any single expression error discards that experiment - and later experiments - from `Compiled` (`compile` 111-120).
5. **Computed names are silently empty** in experiment statements (2.2). A `derive` name that goes through `SpannedName::expressed` must be resolved or explicitly rejected, otherwise duplicate detection (`Code::Duplicate`, contract §5) has nothing to compare.
6. **Legacy measure selection is not 'try until success'**: `evaluate_measures` skips a dataset only when `d.signal(&target).is_none()` and otherwise breaks on the first `Ok`, while swallowing *every* `Err` and moving on (observed 615-646). The contract keeps 'signal absent -> skip; reduction unsupported -> skip; evaluation failure on the *selected* dataset -> error', so that loop must distinguish the error kinds when it is rewritten to evaluate `ExprIr` instead of `measure_signal`.
7. **Analysis identity is duplicated logic**: `{kind}{ordinal}` is computed by the backend per experiment (`thevenin.rs:339-345`, 548) and by `plan.rs:625-652` in core. `analysis: :ac1` must resolve through `analysis_id_by_name` (or the ids will not match what `Dataset.analysis` says); a drift between the two ordinal rules is a silent mis-binding, not an error.
8. **`ProbeRef` cannot express a differential fallback**: `ProbeRef` carries `name: String` + typed `Probe` + span (`plan.rs:58-63`), while the runtime AST's `Expr::Differential { pos, neg }` needs node *names* (`crates/circuit-results/src/expr.rs:125-128`; helper 168-173). `Probe::DifferentialVoltage` holds `NodeId`s (`plan.rs:15-23`). The backend already materialises the difference under `probe.name` (`thevenin.rs:1183-1196` prefers `v(a,b)`, else subtracts), so lowering should use `Expr::Signal(name)` (`expr.rs:119`, 153) and never parse the display name.
9. **Export is per-dataset signals only**: CSV/JSON iterate `dataset.signals` (`crates/circuit-results/src/export.rs:59-109`); `Dataset::validate` re-checks duplicate names case-insensitively and counts `max_result_values` (`crates/circuit-results/src/dataset.rs:551-584`, limit at `crates/circuit-core/src/limits.rs:24`). Derived columns must be appended before export and the size limit counts them too.
10. **`Measured.analysis` does not exist yet** in the results layer (`crates/circuit-results/src/measure.rs:74-103` has name/value/unit only; `render` 95-102), so today a measure's analysis identity is unrepresentable at the display layer (`session.rs:639-647`, `run.rs:163-165`).

## Interface recommendations

Decisions the Lead should freeze before frontend-worker edits `crates/circuit-dsl`:

1. **Lower `Probe`-reads to `Expr::Signal(probe_ref.name)`; never split a differential display name.** Evidence: `ProbeRef` has no node names (`plan.rs:58-63`), `Probe::DifferentialVoltage` holds `NodeId`s (`plan.rs:20`), and the backend already emits the difference under exactly `probe.name` (`thevenin.rs:1183-1196`). The alternative (`Expr::Differential`) needs node names and would force a name parser in `circuit-results` - strictly worse.
2. **Implicit probes go to `AnalysisTask::implicit_probes`, never into `probes`; the backend/read path must switch to `read_probes()` and the export path must keep using `exported_names()`.** Evidence: `plan.rs:482-510`; backend reads `task.probes` only (`thevenin.rs:495-528`); no-`save` default export (`plan.rs:899-906`) would otherwise shrink. Tradeoff: whoever changes `thevenin.rs` must add the read set *without* changing signal order for the default case, or every existing CSV/JSON golden has a reordering risk.
3. **Freeze the parse shape of the optional trailing `analysis:` for both `derive` and `measure` as a hand-rolled label loop, not `arg_list`/`arg_list_open`.** Evidence: `measure` currently accepts exactly one label then `finish_stmt` (`parser.rs:1353-1364`), so `arg_list_open` (`parser.rs:465-487`) cannot start it; a loop keeps the existing error text, which an existing test asserts (`parser.rs:2812-2815`). Also decide whether `derive` joins `KEYWORDS` (`token.rs:188-220`) - observable effect is only `parser.rs:671` and the documented keyword list (`docs/language.md:23-33`).
4. **Decide the name rule for `derive`/`measure`: literal-only, or resolve `SpannedName::expr`.** Today `name.name` is used raw (`elaborate.rs:1993`), so a computed name becomes `""` (`ast.rs:71-77`) and the contract's `Code::Duplicate` checks (`design-contract.md:200`) would compare empty strings. Recommend literal-only with an explicit `Code::Type` (and a new duplicate check, which does not exist today - see §4 'Not covered').
5. **Freeze `DeriveRequest.source` / `MeasureRequest.source` as `ExprIr::render()` output, not verbatim user text.** Evidence: `compile(program, limits)` has no `SourceMap` (`elaborate.rs:88`), so the elaborator cannot slice the input; `ExprIr::render` already exists (`plan.rs:290-308`) and is what every static-check message quotes (`plan.rs:213-266`). Tradeoff: `render()` parenthesizes (`plan.rs:295-298`), so exported metadata reads `(v(vout) / v(vin))` rather than `v(:vout) / v(:vin)`; if verbatim text is required for QA item 9 (`design-contract.md:271`), the CLI/session layer must supply the snippet instead, and the field's contract should say so.

Cross-checks I could not perform (no cargo): that the DSL crate currently fails to compile against the new plan IR, and that the new `ExprIr` helpers are dimensionally correct - both verdicts belong to the Lead's gates and to the reviewer workstream.
