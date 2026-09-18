# Round 4 acceptance — phase A and phase B

> Phase B result (added after `independent-review.md` §11): **accepted**. The reviewer ran a
> 22-row matrix with its own inputs and hand derivations, verified a 15-file frozen revision
> (hashes stable), and closed every plan §5.3 row: forward / multi-level / diamond chains, unknown
> name vs self cycle vs multi-node cycle with spans, nested same-name parameters, override
> recomputation and REPL transactionality, direct and indirect topology sweeps refused at check
> time with the `scanned -> intermediate -> use point` path (each step carrying a real span), plain
> value sweeps still running with hand-checked numbers, the runtime `E_TOPO_PARAM` defence still
> present, and the round-2/3 regressions green. Three non-blocking P3 findings were dispatched
> (docs wording, an outdated test comment, a QA evidence note) and are fixed.
>
> Lead verification of the same rows is in `target/round4/lead-verify/`: a forward-reference
> divider runs to `measure vout = 2 V` (3 V · 2k/(1k+2k)), a two-node cycle reports
> `E_PARAM_CYCLE: a -> b -> a`, an *indirect* topology sweep reports
> `E_TOPO_PARAM ... path: k -> mode -> \`if\` condition` with four source spans, and a plain
> component-value sweep still runs four points to `2.25 V`.
>
> Phase B frozen revision: `crates/circuit-dsl/src/param_graph.rs` (new module),
> `crates/circuit-dsl/src/elaborate.rs`, `crates/circuit-dsl/src/lib.rs`; hashes in the phase-B
> hand-off recorded in `team-board.md`.

# Phase A

Frozen revision: see the hash list handed to the reviewer (`target/round4/reviewer/t8/hashes-start.txt`).
Independent review: `independent-review.md` (task-7 §0-§9 reconnaissance + task-8 §10 closure).

## Result

Phase A is **accepted**. The reviewer closed all nine rows of the design-contract §3 acceptance
matrix with its own inputs, and the round-4 defects R4-01, R4-02 and R4-03 are reproduced as fixed
in debug *and* release, together with the deep-expression regression the fixes had introduced
(FINDING-1).

## What was fixed

| id | defect | fix | owner |
|---|---|---|---|
| R4-01 | an illegal intermediate value was masked: `sqrt(-1)`, `min(sqrt(-1), 2)` and `1e308 * 1e308` ran to exit 0 with a plausible measure and a null signal | every operation validates its own samples (real `is_finite`, both complex components), `sqrt` has a domain check, division and `gain_db` keep their exact-zero rules, and the diagnostic names the derive/measure, the analysis, the sample coordinate and its index. Constant expressions are refused by `cdsl check` before a run | results worker (`expr.rs`, `measure.rs`), session worker (`check.rs`, `execute.rs`) |
| R4-02 | 128 voltage factors overflowed the `i8` dimension exponents: debug panicked at `units.rs:49`, release wrapped silently, `check` accepted the file | the unchecked dimension arithmetic is gone; `Dimension::{checked_mul,checked_div,checked_pow}` and `Quantity::{checked_mul,checked_div}` return `None`, every call site turns that into `E_DIMENSION`, and `ExprIr::static_dimension_error` reports the overflow so `check` refuses the file | lead (`units.rs`, `plan.rs`, `eval.rs`, `format.rs`) |
| R4-03 | the session export path used the text-only renderers and threw the non-finite warnings away | `write_datasets` renders through `to_*_with_diagnostics`, returns `Written { paths, warnings }`, deduplicates per dataset across formats, and both front ends print the warnings | session worker (`execute.rs`, `run.rs`, `session.rs`) |
| FINDING-1 | a deep expression aborted the process (`0xC00000FD`, no diagnostic); after the R4-01 work the run-time cliff fell from >126 to <96 nodes and `check` gained an abort path | parser nesting guard + AST-shape guard, evaluator depth guard, shared `MAX_EXPR_DEPTH = 256`, and a 64 MiB work stack for every `cdsl` subcommand | lead (`parser.rs`, `limits.rs`, `main.rs`), results worker (`expr.rs`) |

## Evidence

| row | command (representative) | expected | observed |
|---|---|---|---|
| `sqrt(-1)` derive | `cdsl check target/round4/repro/r4-01.cdsl` | exit 1, structured `E_VALUE` | exit 1, `error[E_VALUE]: sqrt of a negative sample in \`sqrt(-1)\`` |
| masked measure | `cdsl run ... --experiment bad_sqrt` | exit 1, nothing written | exit 1, `derive \`invalid\`: ...`, output directory absent |
| signal-dependent domain error | `cdsl run target/round4/repro/r4-01d.cdsl --experiment neg_signal` | exit 1 at `sqrt`, not masked by `min` | exit 1, `measure \`masked_neg\`: sqrt of a negative sample ... sample 0 of analysis \`op1\`` |
| overflow | `cdsl run ... --experiment overflow` | exit 1 | exit 1, `multiplication \`*\` produced a non-finite sample ... +inf` |
| complex non-finite | `cdsl run ... --experiment complex_overflow` | exit 1 | exit 1, `... at frequency = 1000 of analysis \`ac1\`: the real part is +inf` |
| 128 voltage factors | `cdsl check/run target/round4/repro/r4-02.cdsl` (debug + release) | exit 1 `E_DIMENSION`, no panic | exit 1 `E_DIMENSION` in both profiles |
| dimension division boundary | `V^-129` by 131 divisions | exit 1 | exit 1 `E_DIMENSION` (reviewer) |
| export warnings | hand-built Dataset with NaN/Inf through `write_datasets` | files keep empty/null, warnings returned once per dataset | CSV empty field, JSON null, `Written { paths: 2, warnings: 2 }` |
| CLI vs REPL | same illegal expression in both | same error class and text | byte-identical stderr (session worker and reviewer) |
| failure leaves no file | valid derive then illegal derive | exit 1, no output file, no deletion | output directory absent, sentinel file untouched |
| deep expression | `nest300` check/run | diagnostic, no abort | exit 1 `E_LIMIT` |
| 96..127 node chains | ladder/deep fixtures | run | exit 0 (restored to pre-fix behaviour) |
| round-2/3 regressions | workspace suite | green | 618 passed / 0 failed |

## Gates (lead, on the frozen revision)

```text
cargo test --workspace                                  exit 0, 618 passed / 0 failed (33 binaries)
cargo clippy --workspace --all-targets -- -D warnings   exit 0
cargo fmt --all -- --check                              exit 0
```

Logs: `target/round4/lead-gate/`. The reviewer independently re-ran the per-crate suites and
reproduced the 618/0 summary, and re-derived the divider and RC numbers by hand.

## Deviations and limits recorded

- The export-warning contract sentence "the returned warnings are the same diagnostics as the JSON
  file's array" was wrong; both the contract §1.4 and the code comment now state the real
  relationship (dataset diagnostics vs renderer warnings). Reviewer NEEDS_FIX P3(a), fixed.
- The nesting guard's boundary and message were off by one against the shape guard; fixed
  (NEEDS_FIX P3(b)). The exponent diagnostic said "at most 127 in magnitude" while `-128` is legal;
  every occurrence now says `-128..=127` (NEEDS_FIX P3(c)).
- The depth limit is only *safe* together with a large stack; `cdsl` runs its work on 64 MiB and the
  constant documents that an embedder must provide one (see `findings.md`).
- `expr::is_constant` and `expr::from_ir` remain recursive; on the product path they are reached
  only through the parser's depth limit, but a hand-built 100k-deep tree passed to the public API
  would still abort. Recorded as a limit, not claimed as fixed.
- Not verified: interactive TTY line editing of the REPL, disk-full/write-failure paths, and the
  `_probe` project (unchanged this round).
