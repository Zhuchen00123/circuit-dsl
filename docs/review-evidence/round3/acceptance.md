# Round 3 acceptance (measured on the final snapshot)

All numbers below were produced by running the commands shown, on the working tree at
F:\codexprojects\dsl000 after every writer stopped. Nothing was committed, pushed or published.
The repository is the workspace root; the CLI is reached with `cargo run --quiet -p circuit-cli -- ...`.

## 1. Workspace gates (lead-run, full logs in target/round3/)

| gate | command | result | log |
|---|---|---|---|
| tests | `cargo test --workspace` | **exit 0, 554 tests passed, 0 failed** (baseline 457) | `target/round3/gate5-test.log` |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 | `target/round3/gate5-clippy.log` |
| format | `cargo fmt --all -- --check` | exit 0 | `target/round3/gate5-fmt.log` |

The 554 is the sum of every `test result: ok. N passed` line of the workspace run (28 test
targets, including 1 doctest). Per target: core 65, results 92 + 1 doctest, dsl 88 + 67 elaborate +
6 phase_syntax + 8 reference_path + 34 result_expressions (new) + 15 tran_option_validation = 218,
backend 15 + 21 adapter + 6 output_interval + 6 phase + 6 source_breakpoint + 4 transient_reference
= 58, session 18 + 6 expression_flow (new) + 25 session + 5 tran_output_interval = 54, cli 9 + 22 e2e
+ 22 expression_qa (new) + 13 repl = 66. The round added 97 tests (457 -> 554), seven of them the
regression tests written after the two independent reviews (B1, B2, N3, NEW-1/NEW-2); the README
now quotes this measured number and points here instead of keeping its own drifting count.

## 2. Lead acceptance runs (real CLI, real files)

The inputs live under `target/round3/lead-acceptance/` (scratch, not part of the repository).

### 2.1 RC corner on an exact sweep grid (scenario 1)

R = 1 kohm, C = 1.5915494 uF => Fc = 1/(2*pi*R*C) = 100 Hz exactly. `ac from: 10.Hz, to: 1.kHz,`
points_per_decade: 1` gives the grid 10, 100, 1000 Hz, so the corner is the middle sample
(no borrowing of a neighbouring point):

| frequency | gain = v(vout)/v(vin) | |gain| | gain_db |
|---|---|---|---|
| 10 Hz | 0.9900990102818735 - 0.09900989910472678j | 0.9950371904013807 | -0.04321373615572819 |
| **100.00000000000001 Hz** | **0.5000000097134758 - 0.4999999999999999j** | **0.707106788055012** | **-3.0102998722696332** |
| 1000.0000000000003 Hz | 0.009900990479893183 - 0.09900990287547125j | 0.0995037209349137 | -20.043213570756766 |

Reference values 0.5 - 0.5j, 0.7071067811865476, -3.010299956639812 are met within 7e-9 (magnitude)
and 8.8e-8 (dB). The residual is the declared-component rounding of Fc (the grid point is
100.00000000000001 Hz), not solver error; the QA suite recomputes the reference from R and C
in-test and lands at 0.4999999999999997 - 0.49999999999999994j, |gain| = 0.7071067811865472,
gain_db = -3.010299956639815 (docs/review-evidence/round3/qa-acceptance.md).

### 2.2 Resistor power (scenario 2)

R = 1 kohm, C = 100 nF, `tran stop: 1.ms, max_step: 1.us`, no `output_interval` (export = raw grid,
1018 samples, width exactly 1 ms), `save v(:vin), v(:vout), i(:r1)`,
`measure :avg_power, avg: v(:vin, :vout) * i(:r1)`, `measure :peak_vr, max: abs(v(:vin, :vout))`,
`derive :vr, expr: v(:vin, :vout)`.

| quantity | value |
|---|---|
| reported `avg_power` | 0.000050001053065728614 V*A |
| independent trapezoid of v_r * i(r1) on the exported raw grid | 0.000050001053065728614 |
| independent trapezoid of i(r1)^2 * R | 0.000050001053065728614 |
| independent trapezoid of v_r^2 / R | 0.000050001053065728614 |
| analytic C/2 * (1 - exp(-2T/tau)) / T | 0.00004999999989694232 |
| max abs difference between the derived column and v(vin) - v(vout) | 0 |
| reported `peak_vr` | 0.9999946875214843 V |

All three independent routes agree bit-for-bit with the reported measure, so the power sign
convention (a resistor absorbs, so the value is positive) and the V*A unit are right; the
2.1e-8 relative gap to the analytic value is the 1 us trapezoid discretisation.

### 2.3 output_interval does not move a measure (scenario 3)

Same circuit, `output_interval: 2.us` vs `output_interval: 50.us` with `max_step: 1.us`:

| run | exported points | measure `vr_avg` | measure `vr_rms` |
|---|---|---|---|
| fine (2 us) | 501 | 0.09999542911366777 V | 0.22360915246413465 V |
| coarse (50 us) | 21 | 0.09999542911366777 V | 0.22360915246413465 V |

The two measure texts are byte-identical while the exported point count follows the requested
grid, which is the contract: measures (and derived signals) are computed on the raw solver grid
and only the output view is resampled. The QA suite checks the same property across 1001 / 51 /
10018 points and additionally verifies that the derived column agrees at every shared output
time (docs/review-evidence/round3/qa-acceptance.md).

### 2.4 Implicit probes are read, not exported (scenario 4)

`save v(:vin)` only, with `derive :vr, expr: v(:vin) - v(:vout)` and `measure :vr_avg, avg: v(:vin, :vout)`:

```
`experiment `subset` on circuit `rc` (backend thevenin 0.5.0)`
`  tran1: 218 time points; signals: v(vin), vr`
`  measure vr_avg = 0.4323330052751151 V (tran1)`
`wrote target\\round3\\lead-acceptance\\out_subset\\subset.tran1.csv`
`CSV header: time,v(vin),vr``
```

So the expression read v(:vout) (the run succeeded without saving it), v(:vout) never appears in
the export, and the derived column does. `cdsl check` prints the dependency explicitly:
`reads v(vin), v(vout) (expression inputs, not exported)`.

### 2.5 Negative contract (scenario 7 and 5), exit codes and no partial export

| input | stage | diagnostic | exit |
|---|---|---|---|
| `derive :g` in an ac+op experiment with no `analysis:` | check | `error[E_AMBIGUOUS]: `g` could be evaluated on any of 2 analyses, so this derive needs `analysis:`` | 1 |
| `measure :m, max: v(:vout) + i(:r1)` | check | `error[E_DIMENSION]: cannot apply `+` to `v(vout)` (V) and `i(r1)` (A): the units differ` | 1 |
| `measure :m, max: v(:vout) / 0` | run | `error[E_VALUE]: division by zero in `(v(vout) / 0)` at time = 0 of analysis `tran1`: `0` is 0` (context: analysis tran1, kind tran, sample time = 0, index 0, measure m) | 1 |
| `measure :m, max: v(:vout) / v(:vin)` on an AC-only experiment (explicit binding) | check | `error[E_TYPE]: `max` of `(v(vout) / v(vin))` is not ordered: the samples of `ac1` are complex` | 1 |
| legacy `measure :m, max: v(:vout)` on an AC-only experiment | check 0, run | `error[E_TYPE]: cannot take the max of `m`: the value is complex, and a complex number has no ordering` (analysis ac1, measure m) | 1 |

This table was re-run on the fixed snapshot (post-review); the codes, stages and exit codes are
identical, and the only text that changed is the two diagnostic sentences the review asked to fix
(section 4b). After all five failures the output directory `target/round3/lead-acceptance/out_neg_final` does not exist,
i.e. a failed expression or measure aborts the run before any file is written. The legacy
complex case shows the intended split: a bare-probe measure without `analysis:` is exempt from the
static AC rule (its analysis is not known at check time) and is reported at run time with the
analysis that was actually selected.

### 2.6 Legacy syntax and round-2 behaviour (scenarios 6 and 11)

`cdsl run examples/rc_filter.cdsl --experiment response` still exits 0 with the documented values
(vfinal = 0.9932620899316276 V, vavg = 0.8013465976386364 V, vrms = 0.8382662216736428 V) and the
legacy measure selection still prefers TRAN over AC over DC over OP (vfinal is the transient max,
not the OP value). The round-2 regression suites (output_interval_regression,
transient_reference_regression, source_breakpoint_regression, phase_regression, tran_output_interval,
session) all pass unchanged.

## 3. Independent QA (owned by qa-worker, details in qa-acceptance.md)

- crates/circuit-cli/tests/expression_qa.rs: 22 tests, real binary, asserts numbers, exit codes and
  file contents; crates/circuit-session/tests/expression_flow.rs: 6 tests.
- Covered: corner gain at the sample that lands on Fc, power against an independent v^2/R trapezoid
  (relative difference 1.3e-16, sign checked), fine/coarse measure invariance with derived-sample
  agreement, implicit-probe export rule, ambiguous/unknown binding, all negative codes with exit 1
  and no output directory, CSV/JSON real-vs-complex + units + analysis identity, file mode vs REPL
  equality, parameter-sweep binding and its refusal, and the round-2 regressions.

## 3b. _probe checks after the warning cleanup (lead)

| command | result |
|---|---|
| `cargo fmt --manifest-path _probe/Cargo.toml -- --check` | exit 0 |
| `cargo build --bin breakpoint_study` | exit 0 (only the MSVC `linker_messages` warning) |
| `cargo run --quiet --bin breakpoint_study` | exit 0 |

Logs: `target/round3/probe-fmt.log`, `probe-build.log`, `probe-run.log`. The before/after
byte comparison of the tool output (identical, both profiles, both deliberate NOT-MET rows preserved)
is in `qa-probe-warnings.md`.

## 4. _probe (outside the workspace gates)

Breakpoint_study kept its numbers byte-for-byte after the local-warning cleanup (27560 bytes, 133
lines, SHA 01ccaed6..., 14/14 contract checks, both deliberate [NOT-MET] rows preserved), in both
debug and release. See docs/review-evidence/round3/qa-probe-warnings.md. The coarse-step
breakpoint accuracy limit of the third-party kernel is unchanged and still documented.

## 4b. Fixes from the independent review, re-verified

The independent reviewer (docs/review-evidence/round3/independent-review.md) found two blocking
issues; both were fixed and re-verified end to end with the real CLI before the final gates ran.

### B1 — an expression-only probe was exported when the experiment had no `save`

Fixed by giving the provenance a home: the backend marks the signals it added *only* because an
expression reads them (`Dataset::implicit_only`), and the output view drops exactly those when the
experiment declares no `save`. The backend's own signal set is untouched, so adding an expression
neither removes a column nor adds one. Verified on `target/round3/lead-acceptance/b1_only.cdsl`:

| experiment | exported header | measure |
|---|---|---|
| `plain` (no save, no expression) | `time,v(vout),v(vin),i(input)` | - |
| `nosave_expr` (+ `measure :avg_power, avg: v(:vin,:vout) * i(:r1)`) | `time,v(vout),v(vin),i(input)` (identical) | 0.000432341666219801 V*A (tran1) |

and `cdsl check` prints `reads v(vin,vout), i(r1) (expression inputs, not exported)`, which is now
literally true. Regression test: `execute::tests::an_expression_only_signal_is_read_but_not_exported_without_a_save`.

### B2 — two parameter sweeps let a binding target the wrong analysis

Fixed at the front end: an experiment that declares two `dc param:` tasks is refused at check time,
and the driver refuses a hand-built plan with the same shape before any solve. Verified:
`cdsl check` on an experiment with `dc param: :r` and `dc param: :c` now exits 1 with
`` error[E_UNSUPPORTED]: experiment `dup_sweep` declares 2 parameter sweeps (dc1, dc2); a run can drive only one ``
plus the note "put each sweep in its own experiment". Regression tests:
`two_parameter_sweeps_are_refused_at_check_time`, `one_parameter_sweep_is_accepted`,
`a_source_sweep_is_not_a_parameter_sweep`. A source sweep next to a parameter sweep still compiles, because the
backend runs it natively in one call.

### N1, N2, N3 and L2 (non-blocking, fixed in the same pass)

- N1: the runtime "no time axis" message now reads
  `` error[E_TYPE]: `avg` needs a time axis, but no analysis here provides one for `v(vout)`; tried: op1 (no axis (operating point)) ``
- N2: a runtime expression failure prints `= analysis:` exactly once (the results layer owns that context; the
  driver adds only `expression` / `measure`).
- N3: a constant expression is a broadcast scalar: `derive :k, expr: 2` exports a 118-sample column of 2 and
  `measure :m, max: 2` reports `2 dimensionless`, instead of failing a shape check after the
  expression had already succeeded. Regression test:
  `execute::tests::a_constant_derived_signal_is_broadcast_over_the_axis`.
- L2: a parameter-sweep experiment that also declares another analysis now says so in the run
  warnings ("analysis `op1` is not exported: a parameter sweep delivers only the swept dataset
  `dc_param_r`") instead of dropping it silently. The pre-existing behaviour itself is unchanged.

### NEW-1 and NEW-2 (blocking, found by the post-fix re-verification and fixed)

A second read-only subagent re-checked the fixed snapshot
(`docs/review-evidence/round3/independent-review-postfix.md`) and found two more blocking defects in the
parameter-sweep path, both fixed and re-verified against its own reproduction files:

- **NEW-1**: the stitched dataset did not carry `implicit_only`, so on a `dc param:` experiment the
  expression-only probes were exported again. Fixed in `stitch` (the provenance is copied and every point
  must agree on it). Verified with the reviewer's `b1_sweep.cdsl`:
  `sweep_plain` and `sweep_expr` now both export `parameter,v(out),v(in),i(src)` while the measure still
  evaluates (`measure p = 0.0015 V*A (dc_param_r)`), and `sweep_derive` adds exactly the derived column
  `parameter,v(out),v(in),i(src),p`.
- **NEW-2**: `stitch` stitched the FIRST task's per-point dataset instead of the swept task's, so an
  experiment with `op` before the swept `dc` (which `check` accepts) failed at run time with a
  confusing `E_NAME` about the other task's read set. Fixed by passing the swept task's position
  into the stitcher and indexing every per-point dataset with it. Verified: the reviewer's
  `sweep_op_first.cdsl` now exits 0, exports the swept dataset with the derived column and warns
  "analysis `op1` is not exported ..."; `sweep_tran_first.cdsl` exits 0 too (it used to fail with
  `E_BACKEND` "returned 1003 samples, expected 1"). Regression test:
  `execute::tests::the_stitch_takes_the_swept_dataset_and_keeps_its_provenance`.

Not fixed, and reported as limitations instead: N4 (one extra dataset-sized allocation while
building the output view), L1 (the derive-vs-backend-name collision guard is unreachable from the
DSL and is defensive code only), L4 (kernel first-step error dominating transient cross-checks).

## 5. What is deliberately not covered here

- Interactive TTY behaviour of the REPL (pre-existing boundary; the tests drive the session API).
- release-profile CLI runs (all numbers above are debug builds; the gates and QA ran debug too).
- Non-Windows platforms, large-simulation resource behaviour, and the third-party breakpoint
  accuracy limitation: unchanged and still listed in README "已知限制".