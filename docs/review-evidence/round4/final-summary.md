# Round 4 final summary

Two phases, both closed by independent review: **A** (R4-01/R4-02/R4-03 plus the deep-expression
regression the fixes exposed) and **B** (parameter DAG, cycles, topology propagation, check-time
rejection). No commit, push or publish was made; every pre-existing uncommitted change was kept.

## What was delivered

### Phase A — correctness gaps (accepted)

| id | before | after |
|---|---|---|
| R4-01 | `sqrt(-1)`, `min(sqrt(-1), 2)` and `1e308 * 1e308` ran to exit 0, printed a plausible measure and wrote null signals | every operation validates its own samples (real `is_finite`, both complex components); `sqrt` has a domain rule; division and `gain_db` keep exact-zero rules; the diagnostic names the derive/measure, analysis, sample coordinate and index; constant expressions are refused by `cdsl check` before a run |
| R4-02 | 128 voltage factors overflowed the `i8` dimension exponents: debug panic at `units.rs:49`, release wrap, `check` accepted the file | checked dimension arithmetic everywhere on user-reachable paths (the unchecked operators are gone, so the compiler enumerates call sites), overflow reported as `E_DIMENSION` by `check` and by the evaluator; debug and release both diagnostic, never panic or wrap |
| R4-03 | the export path used the text-only renderers and dropped the non-finite warnings | `write_datasets` renders through `to_*_with_diagnostics`, returns `Written { paths, warnings }`, deduplicates per dataset across formats, and `cdsl run` plus the REPL print them |
| FINDING-1 | a deep expression aborted the process (`0xC00000FD`, no diagnostic); the round-4 fixes had made the run-time cliff fall from >126 to <96 nodes and added a `check` abort path | parser nesting guard + AST-shape guard, evaluator depth guard, shared `MAX_EXPR_DEPTH = 256` (`E_LIMIT`), and a 64 MiB work stack for every `cdsl` subcommand; 96..127 node chains run again, 300-deep nesting is a diagnostic, 128 factors stay `E_DIMENSION` |

### Phase B — parameter DAG (accepted)

- New `crates/circuit-dsl/src/param_graph.rs`: nodes are parameters **in one body instance**
  (`top`, `top.stage1`), never bare strings across scopes; deterministic topological order with
  source order breaking ties; explicit cycle reports with the closed path and a span per
  declaration; unknown names stay distinct from cycles.
- `elaborate.rs` collects a body's declarations in a pre-pass, evaluates them in dependency order
  with the existing evaluator, and installs the values before the body runs. Forward references
  inside one body are legal; an overridden parameter does not evaluate its default and contributes
  no edges; the default → instance `params:` → experiment `param` → sweep point → `:run`
  precedence is unchanged.
- Topology use points are `if` conditions, `for` iteration sources and computed names/terminals;
  the affected set is the transitive closure backwards through the graph, including instance
  `params:` bindings as the one cross-scope channel. An experiment that sweeps such a parameter is
  refused during elaboration, so `cdsl check`, `cdsl run` and the REPL's `:load` all report
  `E_TOPO_PARAM` with a `scanned parameter -> intermediate parameter(s) -> use point` path and a
  span per step. Plain component-value sweeps keep working, and the per-point runtime defence in
  `circuit-backend/src/sweep.rs` was not touched.

## Verification (all measured)

| gate | command | result |
|---|---|---|
| workspace tests | `cargo test --workspace` | exit 0, **660 passed / 0 failed** (37 test binaries; round-3 baseline 554) |
| lint | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| formatting | `cargo fmt --all -- --check` | exit 0 |
| examples | `cdsl check examples/*.cdsl` | all 7 exit 0 |
| phase A, independent | reviewer's 58-case matrix, debug + release | PASS on all nine acceptance rows; 20/20 frozen hashes stable |
| phase B, independent | reviewer's 22-row matrix with own inputs and hand derivations | PASS on every plan §5.3 row; 15/15 frozen hashes stable |
| round-3 regressions | RC corner, resistor power, multi-analysis binding, implicit probes, resampling, sweep stitching | green (QA, with its own derivations) |
| release | `cargo test --release -p circuit-cli --test r4_repro_cli`, release CLI runs of the reproductions | 8/8, reproductions exit 1 in release too |

Logs: `target/round4/lead-gate/` (workspace/clippy/fmt), `target/round4/reviewer/` (both
matrices), `target/round4/qa*/`, `target/round4/dag-worker/`, `target/round4/docs-worker/`.

## Evidence documents

`design-contract.md` (frozen interfaces + phase-B semantics), `repro-baseline.md` (the four
reproductions before any fix), `dag-recon.md` (parameter scope/override/topology reconnaissance
with 24 compatibility rules and 18 live probes), `findings.md` (FINDING-1/2), `acceptance.md`,
`independent-review.md` (three parts), `qa-acceptance.md`, `qa-acceptance-phase-b.md`,
`team-board.md` (ownership, dependencies, decisions).

## Deviations, conflicts and limits (recorded, not hidden)

- Plan §5.1.3 vs code: `compile()` elaborates each top-level circuit with an empty override chain,
  so a circuit whose own default cannot be evaluated is still reported even when an experiment
  overrides it. Phase B keeps that (documented in contract §4.3) because the alternative would make
  `check`'s verdict depend on which experiment is read last.
- Plan §5.1.1: parameter identity is a bare string today; the DAG introduces scoped node identity
  without changing the observable override chain.
- The task brief's "session.rs param command" does not exist; the session's override surface is
  ` :run <exp> name=expr`, verified transactional.
- The expression depth limit is only safe together with the 64 MiB work stack `cdsl` uses; the
  constant documents that an embedder must provide a comparable stack. `expr::is_constant` and
  `expr::from_ir` remain recursive and rely on the parser's limit on the product path.
- The export-warning contract sentence and the nesting-boundary/exponent wording were wrong at first
  and were corrected (reviewer P3 items, all fixed and re-verified).
- Repeated `params:` keys and repeated experiment `param` statements keep "last wins, no
  diagnostic" — deliberately unchanged and now documented.
- Not verified: interactive TTY line editing, disk-full/write-failure paths, the `_probe` project
  (untouched), and performance.

## Next round (unchanged priority)

1. Run budget, cancellation and result-memory governance (including the declared `max_step`
   long-simulation limit).
2. Mixed parameter sweeps: the known repeated-execution limit stays documented.
3. Only then: plotting, more devices, model import, cross-platform release.
