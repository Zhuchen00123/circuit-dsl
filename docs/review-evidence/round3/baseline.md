# Round 3 baseline (measured before any round-3 change)

Working tree: F:\codexprojects\dsl000, HEAD 8a4d569 ("Separate transient output sampling from
the solver step, and cover source breakpoints"). No commits, pushes or reverts were made in
this round; every pre-existing uncommitted change was preserved.

## Pre-existing state at the start of round 3

Untracked files that were already in the working tree and are still untouched:
AGENT_TEAM_EXECUTION_PROMPT.md, ROUND3_AGENT_TEAM_PROMPT.md, agent-team-switch.md,
docs/round2-acceptance-and-round3-plan.md.

New round-3 evidence lives in docs/review-evidence/round3/; nothing under
docs/review-evidence/round2/ was modified.

## Workspace gates at the baseline (full log: target/round3/baseline-workspace-test.log)

| gate | command | result |
|---|---|---|
| tests | `cargo test --workspace` | exit 0, **457** tests passed, 0 failed (summed from the per-suite `test result:` lines) |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 (re-verified at the final gate) |
| format | `cargo fmt --all -- --check` | exit 0 (re-verified at the final gate) |

The 457 count was taken from the actual run, not copied from the round-2 report; README.md
still claimed 456 and is corrected in this round.

## What round 2 left in place (re-verified by reading, not by trusting the report)

- Raw solver grid and output view are separate: `circuit_session::execute` measures on the
  raw datasets and resamples only the output view (`execute.rs`), and `_probe`'s
  `tran_contract` / `breakpoint_study` cover the sampling contracts.
- The two transient fixes (fine/coarse `output_interval` agreement, negative
  `output_interval` rejected at check time) are covered by
  `crates/circuit-session/tests/tran_output_interval.rs` and stay untouched.
- Known open limitation, still open after this round: coarse-step accuracy around source
  breakpoints in the third-party kernel, and long-simulation resource behaviour.
- `_probe` is not covered by the workspace gates: it has its own `Cargo.toml`, target
  directory and `.gitignore`, and is run separately.

## Machine and toolchain

Windows, MSVC linker (it prints its own stdout warnings during builds; they are not project
warnings). Rust toolchain as resolved by the workspace lock file; `_probe` builds against
the same registry cache.

## Baseline conclusion

The repository was green on all three workspace gates before round 3 began, with 457 tests.
Every gate is re-run from scratch at the end of this round on the frozen snapshot, and the
new count is reported as measured.
