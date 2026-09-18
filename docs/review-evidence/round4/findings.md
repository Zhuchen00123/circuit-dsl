# Round 4 findings (lead-owned)

Issues found while running the round-4 acceptance work that are **not** part of the plan's
phase-A acceptance matrix (R4-01/R4-02/R4-03) or of phase B (parameter DAG). They are recorded
with the same evidence standard as the in-scope fixes. FINDING-1 turned out to be a *regression*
the round-4 fix introduced, so it was fixed before the phase-A gate closed; FINDING-2 was a wording
defect found by the reviewer's mid-term evidence and fixed on the spot.

## FINDING-1 — a deeply nested expression aborts the process (stack overflow, no diagnostic)

| field | value |
|---|---|
| severity | medium: a *legal* program kills the process; no diagnostic, no output, no exit code the CLI controls |
| observed by | QA worker (task-5) and reviewer (task-7), independently, with different triggers |
| pre-existing | yes: the recursion depth of parser/elaborator/evaluator is unchanged by round 4 |
| in plan scope | no; it is adjacent to R4-02 ("user-controlled input must not abort") but not in its acceptance matrix |

Reproductions (all with the debug CLI unless noted):

- A `derive` whose expression is a chain of `v(:vin) * ... * v(:vin)` with 96, 110, 127 or 200
  factors: `cdsl run` exits with `-1073741571` (`0xC00000FD`, `STATUS_STACK_OVERFLOW`) and prints
  only `thread 'main' has overflowed its stack`. 64 and 80 factors run normally.
- The same with an addition chain (110/127/200 factors) — so it is expression *depth*, not
  dimensions.
- `cdsl check` on the same files exits 0: the crash is in the run-time evaluation path.
- `reviewer`: 96 nested `abs(...)` calls pass; 112/120/124/126/200 abort in debug **and in
  `check`**, which points at the parser/lowering recursion as well. In release, 512 nested
  `abs` pass and 1024/2048/4096 abort.

Evidence: `target/round4/qa/depth-probe.md`, `target/round4/qa/probe-depth/`,
`target/round4/reviewer/abs/`, `target/round4/reviewer/logs/`.

Interaction with the phase-A fix that must be preserved by any later fix:

- 128 voltage factors must keep reporting `E_DIMENSION` (checked statically, before evaluation),
  never `E_LIMIT`.
- 127 voltage factors are legal today (V^127 is representable) and must stop aborting: either they
  evaluate, or they are refused with an explicit `E_LIMIT` diagnostic naming the nesting depth.

Candidate fixes (not decided here): (a) run front-end work on a dedicated thread with a large
stack, (b) a configured maximum expression depth reported as `E_LIMIT` (the codebase already has
`Limits` for exactly this kind of bound), (c) making the evaluator/lowering iterative. A final
choice must be verified in debug *and* release, and must not weaken the `E_DIMENSION` path.

## FINDING-1 status after T9 (lead) + T10 (results worker)

Fixed in this round, before the phase-A gate closed, because the reviewer's S1/S2 comparison showed
the round-4 expression policy had made it *worse* in two places: `check` gained an abort path
(constant expressions are now evaluated there) and the run-time cliff moved from >126 to <96 nodes.

| guard | where | behaviour |
|---|---|---|
| parser recursion guard | `crates/circuit-dsl/src/parser.rs` (`unary` wrapper + `depth` counter) | bounds the parser's own recursion; 300 nested `abs`/`(` now report `E_LIMIT` instead of aborting |
| parser tree-shape guard | `parser.rs` (`check_expression_depth`, explicit stack, top-level expressions only) | bounds what every later pass recurses over: an expression deeper than 256 nodes is `E_LIMIT` |
| evaluator guard (defence in depth) | `crates/circuit-results/src/expr.rs` (T10) | a hand-built tree deeper than the limit cannot be evaluated |
| 64 MiB work stack | `crates/circuit-cli/src/main.rs` | every subcommand runs there, so the accepted depth cannot exhaust the stack even in an unoptimised build |
| boundary | measured | the limit is only *safe* together with a stack of that size: on the test harness's 2 MiB thread, 300 nested calls still abort before any guard can run. The CLI is the product surface and it always uses the work stack; an embedder on a small stack must provide one too (documented on `MAX_EXPR_DEPTH`) |
| shared limit | `circuit-core/src/limits.rs` `pub const MAX_EXPR_DEPTH: usize = 256` | one number for both sides |

Measured by the lead after the fix (debug CLI, `target/round4/lead-verify.ps1`):

| input | before | after |
|---|---|---|
| `lad96`..`lad127` run | `96`+ aborted (`0xC00000FD`) | exit 0 |
| `mul96`, `mul112`, `add128` run | aborted | exit 0 |
| `nest120`/`nest200` check | 200 aborted | exit 0 |
| `nest300` abs and paren check | aborted | exit 1, `E_LIMIT`, no abort |
| `const96`/`const256` check | 96 aborted | exit 0 |
| `const512`/`const1024` check | aborted | exit 1 `E_LIMIT` (documented depth limit) |
| 128 voltage factors check | `E_DIMENSION` | unchanged `E_DIMENSION` |

## FINDING-2 — resolved in round 4

`crates/circuit-dsl/src/eval.rs`'s new dimension-overflow note contained ten literal spaces
("in magnitude;          shorten ..."), a copy of a lost line continuation. Fixed by the lead
before the phase-A gate; found by the reviewer's mid-term evidence.
