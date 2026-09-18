# Round 3 final summary

## What was delivered

The round-3 objective - connect the existing result-expression evaluator to the DSL, CLI, REPL
and export - is implemented and verified end to end. No commit, push, publish or deploy was made;
every pre-existing uncommitted change in the working tree was preserved.

### New language surface

```ruby
derive :gain,    expr: v(:vout) / v(:vin)
derive :gain_db, expr: gain_db(v(:vout), v(:vin))
measure :peak_gain, max: abs(v(:vout) / v(:vin))
measure :avg_power, avg: v(:vin, :vout) * i(:r1)
measure :gm, max: abs(v(:vout) / v(:vin)), analysis: :ac1
```

Expressions: probes, dimensionless constants, parentheses, unary +/- , + - * /, and the
functions the evaluator already had (abs, sqrt, min, max, gain_db). Probes an expression reads
are collected automatically (no `save` needed) and are read from the backend without entering the
exported signal set. Multi-analysis experiments bind with `analysis: :{kind}{ordinal}` (`ac1`,
`tran2`); one analysis may omit it; an omitted binding that cannot be resolved is refused as
`E_AMBIGUOUS`, never guessed. Legacy direct-probe `save`/`measure` keeps its documented
TRAN -> AC -> DC -> OP preference order, and round-2 transient behaviour is untouched.

### Implementation map

| layer | file | change |
|---|---|---|
| plan IR (core) | crates/circuit-core/src/plan.rs | `ProbeRef`, `ExprIr`, `AnalysisBinding`, `DeriveRequest`, expression-based `MeasureRequest`, `AnalysisTask::implicit_probes` + `read_probes()`/`exported_names()`, `AnalysisPlan::{derives, result_name, analysis_id_by_name, analysis_names, parameter_sweep}` |
| diagnostics | crates/circuit-core/src/diagnostic.rs | new stable code `E_AMBIGUOUS` |
| front end | crates/circuit-dsl/src/{ast,parser,token,complete,elaborate}.rs | `derive` statement and trailing `analysis:` on `measure`; lowering of the written expression into `ExprIr`; binding resolution; implicit-probe collection; duplicate/name, dimension, complex-AC and time-axis rejections |
| evaluator | crates/circuit-results/src/{expr,measure,lib}.rs | `expr::from_ir`; division-by-zero and zero-amplitude `gain_db` as runtime errors with analysis + sample coordinate; complex `max`/`min` refused with an `abs(...)` hint; `Measured.analysis` + `render_with_analysis()` |
| backend | crates/circuit-backend/src/thevenin.rs | reads `implicit_probes` (widening the default set without changing it, adding them to an explicit list) |
| driver | crates/circuit-session/src/execute.rs | task <-> dataset binding table, derived signals computed on the raw grid and appended before the output view, fallible measure evaluation with the legacy selection separated from "cannot apply", pre-run refusal of parameter-sweep bindings, no partial export on failure |
| session/CLI | crates/circuit-session/src/{session,lib}.rs, crates/circuit-cli/src/{run,check}.rs | analysis-aware measure lines, `derive`/`reads`/binding output in `check` (text and JSON), `evaluate_measures` re-exported as the public seam |
| docs | docs/language.md, docs/repl.md, docs/architecture.md, docs/testing.md, README.md | specification, REPL examples, architecture, and the measured counts |

### Verification (all measured, logs under target/round3/)

- `cargo test --workspace` exit 0: **554 tests passed, 0 failed** (baseline 457, +97).
- `cargo clippy --workspace --all-targets -- -D warnings` exit 0; `cargo fmt --all -- --check` exit 0.
- Real CLI acceptance (lead + independent QA): RC corner on an exact sweep point
  (gain = 0.5 - 0.5j, |gain| = 0.70710678..., dB = -3.01029996), resistor power identical to three
  independent trapezoids (`v*i`, `i^2R`, `v^2/R`) and within 2.1e-8 of the analytic value,
  fine/coarse `output_interval` measures byte-identical with different export sizes, implicit
  probes read but not exported, legacy example unchanged, and every negative case exiting 1 with
  no output directory.
- `_probe` (outside the workspace gates): `breakpoint_study` output byte-identical after the local
  warning cleanup; the two deliberate NOT-MET coarse-step rows are preserved.

### Team and ownership

Three read-only reconnaissance agents (front end, results, session), three implementation workers
(front end, results, session/CLI) with disjoint file sets, a docs worker, a QA worker with its own
integration test files, and an independent reviewer with a separate report. The lead froze the
interface first (`docs/review-evidence/round3/design-contract.md`), owned `plan.rs`, the backend
adapter, the driver and the final gates, and re-checked every worker diff before the gates.

## Findings that changed the plan (and what happened to them)

| finding | resolution |
|---|---|
| **Independent review B1 (blocking)**: an implicit expression dependency was exported when the experiment had no `save`, contradicting `cdsl check` and contract section 4 | Fixed: the backend tags expression-only signals (`Dataset::implicit_only`) and the output view drops exactly those. Verified: a no-`save` file with and without an expression now exports the identical header `time,v(vout),v(vin),i(input)`. Regression test added |
| **Independent review B2 (blocking)**: `swept_task()` (first parameter sweep) and `AnalysisPlan::parameter_sweep()` (last) disagreed, so with two `dc param:` tasks a binding could be evaluated against the other sweep | Fixed: two parameter sweeps in one experiment are refused at check time (`E_UNSUPPORTED`, verified with the real CLI) and the driver refuses a hand-built plan of the same shape before the solve. Three regression tests added |
| **Post-fix re-verification NEW-1/NEW-2 (blocking)**: on a `dc param:` experiment the stitched dataset neither carried the expression-only provenance nor came from the swept task, so the export leak reappeared and an `op` declared before the sweep broke the run | Fixed in `stitch` (provenance copied and cross-checked per point; the swept task's position indexes every per-point dataset). Verified with the re-verifier's own files: `sweep_plain`/`sweep_expr` now export identical headers, `sweep_op_first`/`sweep_tran_first` exit 0 with the dropped-analysis warning. Regression test added |
| Review N1/N2/N3 and L2: garbled no-time-axis message, duplicated `= analysis:` context, constant-only `derive` failing at view build, dropped analysis in a sweep experiment going unmentioned | All fixed in the same pass (see acceptance.md section 4b for the before/after text); N4/L1/L4 are documented as limitations instead |
| The contract row "derive name colliding with a saved signal" is unreachable: probe names are always `v(...)`/`i(...)` while a derive name is a plain identifier | Contract amended; docs describe the reachable rule (duplicate derive/measure names are `E_DUPLICATE`; a runtime guard still catches a derived name that the backend actually returned) |
| `evaluate_measures` was not re-exported from the crate root | Re-exported from `circuit_session` |
| `clippy::approx_constant` on two new test literals and formatting drift across worker files | Fixed by the lead before the final gates (`std::f64::consts::FRAC_1_SQRT_2`; one `cargo fmt --all`) |
| `DeriveRequest.source` holds the canonical rendering, not the verbatim source text | Accepted and documented: elaboration carries spans but no source text; the rendering is unambiguous |
| The REPL used to print measures in engineering units (`993.262 mV`) while `run` printed SI (`0.9932620899316276 V`) | Deliberate, documented change: both now print the same `Measured::render_with_analysis()` line (value, unit, analysis identity), so a REPL run and a file run are textually identical. README and docs/repl.md were updated to the real output; ordinary REPL values (`r = 1 kohm`) keep their engineering form |

## Limits that remain (not claimed as fixed)

- Derived signals cannot feed other derived signals, and expressions never mix samples from two
  analyses or two axes (first-version boundary, in `docs/language.md` §7).
- Complex data has no implicit magnitude ordering: `max`/`min` need an explicit `abs(...)`.
- A `dc param:` sweep produces one stitched dataset, so bindings to any other analysis of that
  experiment are refused instead of dropped.
- Third-party kernel limitations stay as documented: coarse-step accuracy near source breakpoints,
  no tolerance channel, no run cancellation/budget, Windows-MSVC-only verification.
- The REPL is verified through the session API; interactive TTY line editing is still untested.

## Next round (unchanged from the round-2 roadmap)

1. Parameter DAG, cycle diagnostics, topology-dependency propagation and check-time rejection.
2. Run budget, cancellation and result-memory governance (including the declared `max_step`
   long-simulation limit).
3. Only then: plotting, more devices, model import, cross-platform release.