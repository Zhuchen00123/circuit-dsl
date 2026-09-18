# Round 4 team board

Lead: `lead`. Scope basis: `docs/round3-review-and-round4-plan.md`, contract:
`docs/review-evidence/round4/design-contract.md`.

## Ownership (one writer per file)

| owner | write set | task |
|---|---|---|
| lead | `crates/circuit-core/src/units.rs`, `crates/circuit-core/src/plan.rs`, `crates/circuit-core/src/format.rs`, `crates/circuit-dsl/src/eval.rs`, docs/board/evidence, final gates | T1 |
| results worker | `crates/circuit-results/src/expr.rs`, `measure.rs`, `export.rs` | T2 |
| session worker | `crates/circuit-session/src/execute.rs`, `crates/circuit-session/src/session.rs` (only the `run --out` branch, ~648-656), `crates/circuit-session/tests/expression_flow.rs` (only the mechanical `written.paths` adaptation), `crates/circuit-cli/src/{main,run,check,repl}.rs` | T3, T4 |
| QA worker | new files only: `crates/circuit-cli/tests/r4_*.rs`, `crates/circuit-session/tests/r4_*.rs`, `crates/circuit-results/tests/r4_*.rs` | T5 |
| DAG explorer | none (read-only report) | T6 |
| reviewer | none (read-only report) | T7, T8 |

Shared build resources: one `target/` directory for everyone; local `cargo test -p <crate>` only.
The lead alone runs `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo fmt --all`. No worker runs `cargo fmt`.

## Log

| date | event |
|---|---|
| start | baseline confirmed: workspace tests pass; `cdsl check` accepts `sqrt(-1)`; debug `cdsl run` panics at `units.rs:49` on 128 voltage factors (repro under `target/round4/repro/`) |
| start | phase A interfaces frozen in `design-contract.md`; core write set landed by the lead |
| + | R4-02 verified end to end by the lead: `cdsl check` and `cdsl run` on the 128-factor input now exit 1 with `E_DIMENSION` (no panic, no wrap); `cargo test -p circuit-core -p circuit-dsl` all green |
| + | session worker authorized to touch `circuit-session/src/session.rs` (`run --out` branch only) for the R4-03 wiring |
| + | note: `target/release/cdsl.exe` is a stale round-2 binary (no `derive` support); release verification rebuilds it |
| + | T2 complete: results worker landed the finite/domain policy + `eval_at`/`eval_constant`; `cargo test -p circuit-results` 103 lib + 12 QA tests green |
| + | T6 complete: `dag-recon.md` (24 compatibility rules, 18 live probes); phase-B semantics frozen in `design-contract.md` §4 |
| + | FINDING-1 (deep-expression stack overflow, pre-existing) recorded in `findings.md`; fix decision deferred until both phases pass |
| + | T4 complete: `check` now rejects constant illegal expressions (exit 1) and execution uses named `EvalSite`s; lead verified by hand: `sqrt(-1)` (check+run), `min(sqrt(v/abs(v)),2)` with a negative signal, `1e308*1e308`, 128-factor `E_DIMENSION`, AC complex overflow — all exit 1 with a structured diagnostic and no output directory |
| + | lead gates so far: `cargo clippy --workspace --all-targets -- -D warnings` exit 0; `cargo fmt --all -- --check` reports drift in two worker files, to be formatted once after task-5 |
| + | T1 complete: checked dimension API + static overflow analysis landed; `cargo test -p circuit-core -p circuit-dsl` all green; reviewer reproduced the debug *and* release fix independently |
| + | T2/T3/T4/T5 complete: R4-01/02/03 fixed and covered by 42 new independent QA tests (6 files); CLI and REPL diagnostics byte-identical |
| + | FINDING-1 fixed before the gate (T9 lead + T10 results worker): parser nesting guard, AST depth guard, evaluator guard, shared `MAX_EXPR_DEPTH = 256`, 64 MiB work stack in `main.rs`; 96..127 chains run again, 300-deep nesting is `E_LIMIT`, 128 factors stay `E_DIMENSION` |
| + | lead gates on the frozen phase-A revision: `cargo test --workspace` 618 passed / 0 failed (exit 0), `cargo clippy --workspace --all-targets -- -D warnings` exit 0, `cargo fmt --all -- --check` exit 0; logs in `target/round4/lead-gate/` |
| + | phase-B tasks created (T11 DAG worker, T12 QA, T13 docs, T14 reviewer, T15 lead gate), all gated on T8 |
| + | T8 complete: reviewer closed phase A PASS (58-case matrix, 20/20 hashes stable, 618/0 reproduced). Three P3 NEEDS_FIX recorded and fixed by the lead (Written struct comment, nesting-guard boundary, `-128..=127` wording); gates re-run green |
| + | **phase A accepted** — evidence in `acceptance.md`; phase B started: T11 dag-worker (param_graph.rs + elaborate.rs) and T12 qa-worker (r4b_* tests) running in parallel with disjoint write sets |
| + | T11 + T12 complete: parameter DAG (scope paths, deterministic topological order, cycle paths with spans, effective-definition priority, transitive topology propagation) and check-time `E_TOPO_PARAM` rejection; 92 new phase-B tests |
| + | decision: the topology-sweep refusal lives in `compile()` (contract §4.6), so `:load`/`:define` refuse such a program before any solve; QA rewrote its session fixture accordingly |
| + | lead gates after phase B: `cargo test --workspace` exit 0 / **660 passed, 0 failed** (37 binaries), `clippy -D warnings` exit 0, `fmt --check` exit 0; logs in `target/round4/lead-gate/*-b.log` |
| + | T13 docs-worker spawned (language/architecture/repl/testing/README); T14 reviewer running the phase-B matrix |
| + | T14 complete: reviewer closed phase B PASS (22-row matrix, own inputs and hand derivations, 15/15 hashes stable); the three P3 items (docs wording, outdated test comment, QA evidence note) are fixed |
| + | T13 complete: language/architecture/REPL/testing/README synced to the round-4 behaviour and to 660 tests; docs worker verified 22 CLI commands against the written examples |
| + | user requested git management after the round closed: everything uncommitted since `8a4d569` (rounds 2-4: 80 files) committed as `097e955` and pushed to `origin/main`; remote ref verified equal to local HEAD |
| + | **round 4 closed** — final gates on the frozen revision: `cargo test --workspace` 660 passed / 0 failed (exit 0), `clippy -D warnings` exit 0, `fmt --check` exit 0, all 7 examples `check` exit 0; logs in `target/round4/lead-gate/final-*.log`; summary in `final-summary.md`. No commit/push was made |
