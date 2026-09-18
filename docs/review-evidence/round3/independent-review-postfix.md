# Round 3 independent re-verification (post-fix)

Independent re-verifier workstream. **Read-only round: the only file written is this one.**
All throwaway inputs/outputs live under `target/round3/reverify/`.

- Snapshot: working tree at HEAD `8a4d569` **with the uncommitted round-3 diff and the lead's
  post-review fixes** (the same 27 modified files plus the round-3 test/evidence files that
  `git status` shows now).
- Contract re-checked against: `docs/review-evidence/round3/design-contract.md` (frozen).
- Claims re-checked against: `docs/review-evidence/round3/acceptance.md` §4b, and the findings
  B1/B2/N1/N2/N3/L2 of `docs/review-evidence/round3/independent-review.md`.
- Method: read the code paths first, then reproduce every claim with the real CLI built from this
  same working tree, and cross-check the *original reviewer's own inputs*
  (`target/round3/reviewer/*.cdsl`) with my binary.
- Own target dir and commands used throughout (never the workspace gates):

  ```powershell
  $env:CARGO_TARGET_DIR="F:\codexprojects\dsl000\target\round3\w-reverify"
  cargo build -p circuit-cli
  cargo test -p circuit-core | -p circuit-results | -p circuit-dsl | -p circuit-backend | -p circuit-session | -p circuit-cli
  $c = "target\round3\w-reverify\debug\cdsl.exe"     # every "cdsl" below is this binary
  & $c check|run <file> [--experiment E] [--out DIR] [--format csv|json]
  ```

  Throwaway inputs: `target/round3/reverify/*.cdsl`; outputs `target/round3/reverify/out/**`.

---

## 1. B1 — expression-only probes were exported when there is no `save`

### Verdict: **verified for ordinary analyses; BROKEN (still leaks) on the parameter-sweep path**

The provenance mechanism itself is right and every non-sweep case I could construct behaves as the
frozen contract §4 requires.

**Command / observed (the deciding runs)**

| # | command (experiment in `target/round3/reverify/b1_set.cdsl`) | observed CSV header |
|---|---|---|
| 1 | `& $c run ...b1_set.cdsl --experiment plain --out ...out\b1_plain --format csv` | `time,v(vout),v(vin),i(input)` |
| 2 | `--experiment nosave_expr` (`measure :avg_power, avg: v(:vin, :vout) * i(:r1)`, no `save`) | `time,v(vout),v(vin),i(input)` — **identical**, `measure avg_power = 0.0002454263496237123 V*A (tran1)` |
| 3 | `--experiment nosave_derive` (`derive :vr, expr: v(:vin, :vout)`) | `time,v(vout),v(vin),i(input),vr` (default set + derived only) |
| 4 | `--experiment nosave_default_read` (`measure :peak, max: v(:vout)`) | `time,v(vout),v(vin),i(input)` — an implicit probe the backend reports **by default stays exported** |
| 5 | `--experiment save_expr` (`save v(:vin)`, `derive :vr`, `measure ... v(:vin,:vout)*i(:r1)`) | `time,v(vin),vr` — exactly the saved signal plus the derived one |
| 6 | `& $c run ...b1_same.cdsl --experiment save_and_read_same --format csv` (`save v(:vout), v(:vin)` + `derive :sum` + `measure ... v(:vout) + v(:vin)`) | `time,v(vout),v(vin),sum` — a signal in **both** save and implicit appears **once** |
| 7 | `--experiment op_expr` vs `op_plain` (`b1_more.cdsl`) | `v(vout),v(vin),i(input)` vs `v(vout),v(vin),i(input),p` |
| 8 | `--experiment interval_expr` vs `interval_plain` (`output_interval: 10.us`, 11 rows each) | `time,v(vout),v(vin),i(input)` vs `...,vr` — the resample path does not reintroduce them |

JSON export did **not** gain keys (both files, run 1 vs run 2):

```
json_plain\plain.tran1.json        top=[analysis,axis,backend,diagnostics,experiment,kind,schema,signals] sig=[name,type,unit,values] signals=3
json_expr\nosave_expr.tran1.json   top=[analysis,axis,backend,diagnostics,experiment,kind,schema,signals] sig=[name,type,unit,values] signals=3
```

`Select-String -Path ...json -Pattern implicit` finds nothing, and the writer is untouched by the
round: `git diff --stat HEAD -- crates/circuit-results/src/export.rs` prints **nothing**
(the round-3 `dataset.rs` diff adds only the `implicit_only` field, `dataset.rs:480-488`,
`516`). The new `implicit_probes` key exists only in `check --json`, which is a plan dump, not a
result export — and it is documented (`docs/language.md:607-608`).

The reviewer's own repro also passes now:

```
& $c run target\round3\reviewer\power_set.cdsl --experiment plain        -> HEADER: time,v(vout),v(vin),i(input)
& $c run target\round3\reviewer\power_set.cdsl --experiment nosave_expr  -> HEADER: time,v(vout),v(vin),i(input)
   measure avg_power = 0.000049999859482767916 V*A (tran1)
& $c check target\round3\reviewer\power_set.cdsl
   experiment `nosave_expr` ... : reads v(vin,vout), i(r1) (expression inputs, not exported)
```

### The hole: a parameter sweep loses the provenance, so the leak is still there

```
& $c run target\round3\reverify\b1_sweep.cdsl --experiment sweep_plain  --out ...\b1sw_sweep_plain  --format csv
  dc_param_r: 8 sweep points; signals: v(out), v(in), i(src)
  HEADER: parameter,v(out),v(in),i(src)

& $c run target\round3\reverify\b1_sweep.cdsl --experiment sweep_expr   --out ...\b1sw_sweep_expr   --format csv
  dc_param_r: 8 sweep points; signals: v(out), v(in), i(src), v(in,out), i(r1)
  HEADER: parameter,v(out),v(in),i(src),"v(in,out)",i(r1)      <-- two columns nobody wrote

& $c check target\round3\reverify\b1_sweep.cdsl
      reads v(in,out), i(r1) (expression inputs, not exported)   <-- now untrue
```

`--experiment sweep_derive` shows the same leak plus the derived column
(`parameter,v(out),v(in),i(src),"v(in,out)",i(r1),p`), and the JSON export of the same run carries
the same extra signal entries (`SWEEP-JSON-SIGNALS: v(out),v(in),i(src),v(in,out),i(r1) COUNT=5`
versus 3 for `sweep_plain`); the JSON top-level key set is unchanged, so this is extra data, not a
schema break. Details and the code line in §5 (NEW-1).

---

## 2. B2 — two parameter sweeps could target the wrong analysis

### Verdict: **verified** (front-end refusal + single-sweep end-to-end), with one dead-code caveat

```
& $c check target\round3\reverify\b2_two_dc.cdsl        -> EXIT=1
error[E_UNSUPPORTED]: experiment `dup_sweep` declares 2 parameter sweeps (dc1, dc2); a run can drive only one
  --> target\round3\reverify\b2_two_dc.cdsl:12:3
   = a parameter sweep re-elaborates and re-runs the design once per point, so two sweeps cannot share one run;
     put each sweep in its own experiment

& $c run  ...b2_two_dc.cdsl --experiment dup_sweep     --out ...\out\b2_two  --format csv  -> EXIT=1, OUTDIR=False
& $c run  ...b2_two_dc.cdsl --experiment dup_sweep_rev --out ...\out\b2_twor --format csv  -> EXIT=1, OUTDIR=False
```

The reviewer's original B2 inputs are refused identically, both stages, no output directory:
`target\round3\reviewer\sweep_two_d1.cdsl`, `sweep_two_d2.cdsl`, `sweep_both.cdsl`
(`CHECK ... EXIT=1`, `RUN ... EXIT=1 OUTDIR=False`). The input order does not matter:
`dup_sweep_rev` (`dc param: :c` first, `derive ... analysis: :dc2`) is refused too.

**A single parameter sweep still runs end to end and binds to the stitched dataset**

```
& $c run target\round3\reverify\b2_one_sweep.cdsl --experiment one --out ...\out\b2_one --format csv
  dc_param_r: 8 sweep points; signals: v(out), v(in), i(src), d
  measure vmax = 2.25 V (dc_param_r)
  measure vmin = 0.8181818181818182 V (dc_param_r)
  HEADER: parameter,v(out),v(in),i(src),d
  ROW: 500,2.25,3,-0.0015,4.5      ROW: 1000,1.8,3,-0.0012,3.6     ROW: 1500,1.5,3,-0.001,3
  ROW: 4000,0.8181818181818182,3,-0.0005454545454545455,1.6363636363636365
```

`d = 2*v(:out)` bound with `analysis: :dc1` matches the stitched column row by row, and the two
measures are the independent values of the swept divider `3*1.5/(r+1.5)`
(max at r = 0.5 kohm = 2.25 V, min at r = 4 kohm = 0.818181... V). The reviewer's
`target\round3\reviewer\sweep_op.cdsl` (op + swept dc, `save v(:out)`, derive bound to `:dc1`)
also runs: `HEADER: parameter,v(out),scaled`, `scaled` = 4.5 / 3.6 / 3 / 2.5714285, `vmax = 2.25 V`.

**A source sweep plus a parameter sweep still compiles** (and runs):

```
& $c check target\round3\reverify\b2_src_and_param.cdsl -> EXIT=0
    - dc    save v(out)
    - dc    save v(out)
& $c run ...b2_src_and_param.cdsl --experiment src_and_param --format csv -> EXIT=0
  dc_param_r: 8 sweep points; signals: v(out)
  warning: analysis `dc1` is not exported: a parameter sweep delivers only the swept dataset `dc_param_r`
```

**Hand-built plan with two sweeps — code path read, not executed.** `check_sweep_capabilities`
(`crates/circuit-session/src/execute.rs:216-241`) is called at `execute.rs:120`, i.e. **before** the
backend call at `execute.rs:128`, and reproduces the same predicate on a plan. However `execute`
**always re-elaborates from source first** (`execute.rs:107-112`) and the front end already refuses
two sweeps (`elaborate.rs:2316-2342`), so this guard is unreachable through the public API
(`circuit_session::execute`, and the REPL, which calls it at `session.rs:594`); no test covers it
(`crates/circuit-session/src/execute.rs` test list has no two-sweep case). It is defensive code of
the same class as L1, not a checkable behaviour — see §6.

**Intentional behaviour change** (must be noted because it changes previously-accepted input): an
experiment with two `dc param:` used to `check` clean and `run` (reviewer's `sweep_two_d1.cdsl`);
it is now `E_UNSUPPORTED`. This is what the contract §3/§5 asks for and what
`docs/language.md:730` documents, and no file in `examples/` is affected.

---

## 3. N1 / N2 / N3 / L2

### N1 — the "no time axis" sentence is readable: **verified**

```
& $c run target\round3\reverify\n1_avg_op.cdsl --experiment op_only --format csv        -> EXIT=1
error[E_TYPE]: `avg` needs a time axis, but no analysis here provides one for `v(out)`; tried: op1 (no axis (operating point))

& $c run target\round3\reverify\n1_avg_op_dc.cdsl --experiment op_and_sweep ...          -> EXIT=1
error[E_TYPE]: `avg` needs a time axis, but no analysis here provides one for `v(out)`; tried: dc1 (a parameter axis), op1 (no axis ...

& $c run target\round3\reviewer\legacy_avg_op.cdsl    --experiment e ...  -> same sentence, tried: op1 (...)
& $c run target\round3\reviewer\legacy_avg_op_dc.cdsl --experiment e ...  -> tried: dc1 (a parameter axis), op1 (no axis (operating point))
```

The malformed `"v(out) is , op1 (...)"` form is gone; the candidate list is now a proper
`tried: a (…), b (…)` list (code: `execute.rs:835-849`).

### N2 — `= analysis:` exactly once: **verified**

Command / count (`[regex]::Matches($o, "= analysis:").Count` over the combined output):

| input | observed | count |
|---|---|---|
| `& $c run target\round3\reverify\n2_divzero.cdsl --experiment divzero_derive` | `error[E_VALUE]: division by zero in (v(vout) / 0) at time = 0 of analysis tran1: 0 is 0` + `= analysis: tran1 / = kind: tran / = signal: 0 / = sample: time = 0 / = index: 0 / = expression: (v(vout) / 0)` | **1** |
| `--experiment divzero_measure` | same + `= measure: m` | **1** |
| `& $c run target\round3\reviewer\err_div_zero_derive.cdsl --experiment e` (op) | `... at sample 0 of analysis op1 ...` + `= analysis: op1 / = kind: op / = signal: 0 / = sample: sample 0 / = index: 0 / = expression: ...` | **1** |

### N3 — constant expressions broadcast: **verified**

```
& $c run target\round3\reverify\n3_const.cdsl --experiment const_tran --format csv
  HEADER: time,v(vout),v(vin),i(input),k   ROWS=109   FIRST ROW k=2   LAST ROW k=2
  measure m = 2 dimensionless (tran1)
& ... --experiment const_op  -> HEADER v(vout),v(vin),i(input),k, ROWS=1, k=2
& ... --experiment const_ac  -> HEADER frequency,...,k, ROWS=3, k=2 (real column next to complex ones)
& $c run target\round3\reviewer\derive_const.cdsl --experiment e --format csv
  HEADER: frequency,v(vout)_re,v(vout)_im,k,half_re,half_im ; measure m = 2 dimensionless (ac1)
& $c run target\round3\reverify\sweep_const.cdsl --experiment sweep_const --format csv
  HEADER: parameter,v(out),k ; ROWS=4 ; k=2 ; measure m = 2 dimensionless (dc_param_r)
```

No shape error, no internal-sounding `E_VALUE`; `broadcast_value` (`execute.rs:467-483`) covers the
derive path, both measure paths and the stitched sweep axis.

### L2 — a sweep that declares another analysis mentions it: **verified for the shapes that survive; the claim does not hold in general**

```
& $c run target\round3\reverify\l2_warning.cdsl --experiment sweep_and_op --format csv   -> EXIT=0
  dc_param_r: 4 sweep points; signals: v(out)
  warning: analysis `op1` is not exported: a parameter sweep delivers only the swept dataset `dc_param_r`
& $c run target\round3\reviewer\sweep_op.cdsl --experiment sweep --format csv             -> EXIT=0 + same warning for `op1`
& $c run ...b2_src_and_param.cdsl --experiment src_and_param                                -> EXIT=0 + warning for `dc1`
```

Caveat: when the *other* analysis is declared **before** the sweep and does not survive the stitch,
the run dies inside `stitch` **before** warnings are computed, so nothing is said at all:

```
& $c run target\round3\reverify\sweep_tran_first.cdsl --experiment tran_first --format csv -> EXIT=1
error[E_BACKEND]: sweep point for `v(out)` returned 1003 samples, expected 1   (no warning printed)
```

That is a consequence of NEW-2 below; the L2 fix is correct only where the stitch itself succeeds.

---

## 4. Regression sweep of what the fixes touched

**Per-crate suites, own target dir** (`$env:CARGO_TARGET_DIR=...\w-reverify`, logs in
`target\round3\reverify\test-<crate>.log`):

| crate | targets | passed | failed |
|---|---|---|---|
| circuit-core | 2 | 65 | 0 |
| circuit-results | 2 | 93 (92 + 1 doctest) | 0 |
| circuit-dsl | 7 | 218 (88 + 67 + 6 + 8 + **34 result_expressions** + 15) | 0 |
| circuit-backend | 7 | 58 (15 + 21 + 6 output_interval + 6 phase + 6 source_breakpoint + 4 transient_reference) | 0 |
| circuit-session | 5 | 52 (16 + 6 expression_flow + 25 session + 5 tran_output_interval) | 0 |
| circuit-cli | 4 | 66 (9 + 22 e2e + 22 expression_qa + 13 repl) | 0 |
| **total** | **27** | **552** | **0** |

So the round-2 transient suites (`output_interval_regression`, `transient_reference_regression`,
`source_breakpoint_regression`, `phase_regression`, `tran_output_interval`) and the new
expression suites are all green; the claimed 552 total is real. (The per-target breakdown *inside*
`acceptance.md` §1 is stale — it sums to 547 and lists `dsl 31 result_expressions` /
`session 14`; the tree actually has 34 and 16. See §5 N-2.)

**Legacy measure selection order** — two equal-rank source sweeps and a TRAN/AC/DC mix:

```
& $c run target\round3\reviewer\rank_tie.cdsl --experiment e --format csv   -> EXIT=0
  measure vm  = 1.5 V (dc1)      <- declaration order breaks the DC/DC rank tie
  measure vm2 = 0.5 V (dc1)
& $c run target\round3\reverify\legacy_order.cdsl --experiment two_source_sweeps -> measure vm = 1.2 V (dc1)   (dc2 would give 2.4)
& $c run target\round3\reverify\legacy_order.cdsl --experiment tran_wins         -> measure vm = 0.6 V (tran1)  (dc1 would give 1.2)
```

**Examples and CSV/JSON shapes**

```
foreach examples\*.cdsl: cdsl check -> diode_rectifier 0, ladder 0, parameter_sweep 0, rc_filter 0,
                                       rlc 0, two_stage 0, voltage_divider 0
& $c run examples\rc_filter.cdsl --experiment response --format csv -> EXIT=0
  measure vfinal = 0.9932620899316276 V (tran1)   <- byte-equal to the acceptance value
  measure vavg   = 0.8013465976386364 V (tran1)
  measure vrms   = 0.8382662216736428 V (tran1)
  HEADER response.tran1.csv: time,v(vin),v(vout),i(input),i(r1)
  HEADER response.op1.csv:   v(vin),v(vout),i(input),i(r1)
  HEADER response.ac1.csv:   frequency,v(vin)_re,v(vin)_im,...,i(r1)_re,i(r1)_im
```

No existing behaviour was weakened beyond the two documented, intended changes (two parameter
sweeps refused; complex `max`/`min` rejected — both pre-existing round-3 contract changes, not
post-review regressions).

---

## 5. New defects found by this pass

### NEW-1 (BLOCKING) — the parameter-sweep path drops `implicit_only`, so B1 is only half-fixed

- **Repro**: `& $c run target\round3\reverify\b1_sweep.cdsl --experiment sweep_expr --out ... --format csv`
  → `HEADER: parameter,v(out),v(in),i(src),"v(in,out)",i(r1)` while
  `& $c run ... --experiment sweep_plain` → `HEADER: parameter,v(out),v(in),i(src)`, and
  `& $c check ...b1_sweep.cdsl` prints `reads v(in,out), i(r1) (expression inputs, not exported)`.
  Also in JSON (`COUNT=5` signals vs 3), and in `--experiment sweep_derive` (columns +`p`).
- **Code**: the backend does mark the provenance on every per-point dataset
  (`crates/circuit-backend/src/thevenin.rs:498,515-534,594`), but `stitch` builds the stitched
  dataset with `Dataset::new(...)` (`crates/circuit-session/src/execute.rs:688-696`) and never
  copies `implicit_only`, so the filter in `output_view`
  (`crates/circuit-session/src/execute.rs:353-362`, the `None =>` arm at `:356`) has nothing to drop.
- **Contract/docs**: design-contract §4 ("implicit probes never enter the export unless they were
  also saved", `design-contract.md:186-188`) and `docs/language.md:603-606` ("«加一个测量»…不会平白
  多出没人写过的列") are both untrue for `dc param:` experiments.
- **Reach**: the same `output_view` is used by the REPL (`session.rs:594` follows
  `execute::execute`), so `:run` of a no-`save` sweep in the REPL exports the same extra signals.

### NEW-2 (BLOCKING) — the stitched dataset is the **first task's** dataset, not the swept task's

- **Repro A** (`check` accepts, `run` dies):
  ```
  & $c check target\round3\reverify\sweep_op_first.cdsl   -> EXIT=0
      experiment `op_first_derive` ... reads v(in,out), i(r1) (expression inputs, not exported)
      derive :p, expr: (v(in,out) * i(r1)), analysis: :dc1
  & $c run ...sweep_op_first.cdsl --experiment op_first_derive --format csv -> EXIT=1
  error[E_NAME]: no signal named `v(in,out)` in analysis `dc_param_r`
     = analysis: dc_param_r / = kind: dc / = expression: (v(in,out) * i(r1))
     = available signals: v(out), v(in), i(src)          <- exactly the *op1* task's read set
  ```
  `--experiment op_first_saved` (`save v(:out), i(:r1)`) fails the same way with
  `available signals: v(out), i(r1)` — again the first task's read set, not the swept task's
  (which also read `v(in,out)`). The measure variant (`op_first_measure`) fails identically.
- **Repro B** (other analysis first, multi-sample): `& $c run ...sweep_tran_first.cdsl --experiment tran_first`
  → `EXIT=1 error[E_BACKEND]: sweep point for v(out) returned 1003 samples, expected 1` — the
  stitched columns are taken from the `tran` task.
- **Code**: `stitch` takes `outcome.results.first().and_then(|r| r.datasets.first())`
  (`execute.rs:601-604`) and then `let d = &r.datasets[0]` for every column
  (`execute.rs:654-656`) — index `0` is the first task in plan order, while
  `swept_task`/`binding_table`/`check_sweep_bindings` (`execute.rs:202-207, 244-282, 289-319`)
  map the swept analysis to *that* dataset. So the only binding the capability check permits
  (`analysis: :dc1`) is answered by a dataset that may not be the swept analysis' result.
- **Why it is (only) a failure, not a wrong number**: the per-point plan converts every `Dc` task
  to `Op` (`as_single_point_plan`, `execute.rs:572-591`), so the shapes that reach the stitch are
  homomorphic operating points; I could not construct a case where the exported numbers differ from
  the swept task's. The failure modes are a spurious `E_NAME`/`E_BACKEND` (exit 1, no file) for a
  check-accepted experiment, plus the misleading L2 warning ("analysis `op1` is not exported",
  while it is precisely `op1`'s dataset that is exported under the name `dc_param_r`).
- Acceptance §4b's claim "a single parameter sweep still runs end to end … evaluated against the
  stitched `dc_param_<parameter>` dataset" is true **only when the swept task is the first task**.

### NEW-3 (non-blocking, documentation) — `acceptance.md` §1 per-target table does not add up

The claimed workspace total (552) is correct — my own per-crate run reproduces 552/0 — but the
breakdown printed in `acceptance.md:16-21` sums to 547 and is stale relative to the tree it
describes (`result_expressions` 31 → 34, session unit tests 14 → 16, i.e. the five post-review
regression tests). README's "552" is right; the table beside it is not.

### NEW-4 (non-blocking, note) — the reviewer's B2 input no longer runs

`target/round3/reviewer/sweep_two_d1.cdsl` (the file that demonstrated the wrong-analysis
evaluation) is now refused, so the original B2 symptom can no longer be reproduced *by that file*;
my `b2_one_sweep.cdsl` and `sweep_op.cdsl` runs above replace it as positive evidence. This is
intended, but any future re-check should use a single-sweep file, not this one.

---

## 6. What I could not verify

- **The hand-built-plan guard** (`check_sweep_capabilities`, `execute.rs:216-241`): verified by
  reading that it runs before any `backend.run` (`:120` vs `:128`) and uses the same predicate as
  the front end, but it is unreachable from the public API because `execute` re-elaborates first
  (and elaboration refuses two sweeps). Exercising it would require a Rust test file, which this
  read-only pass may not add. No existing test covers it.
- **Interactive REPL**: I did not drive a TTY session. I verified the REPL shares
  `execute::execute` (`session.rs:594`) and that the CLI `repl` suite passes (13/13); the sweep
  leak in REPL `:run` (NEW-1) is therefore inferred from the shared code path, not observed.
- **Release-profile runs and non-Windows behaviour**: not run (debug only, as in the acceptance).
- **Workspace-wide gates** (`cargo test --workspace`, clippy, fmt): deliberately not run — my
  per-crate runs (27 targets, 552 tests) cover the test claim, but not clippy/fmt.
- **Peak-memory behaviour of the extra `cloned()` view (reviewer's N4)**: not measured; unchanged
  by the fixes and still bounded by `max_result_values`.

---

## 7. Bottom line

| item | verdict |
|---|---|
| B1 (no-save export provenance) | **verified for ordinary analyses; still broken on the `dc param:` path** (NEW-1) |
| B1 sub-checks: default-reported implicit stays, save+implicit once, JSON keys unchanged | verified |
| B2 (two sweeps refused, single sweep binds, source+param compiles) | **verified** (hand-built guard unreachable, §6) |
| N1 malformed "no time axis" sentence | verified fixed |
| N2 duplicate `= analysis:` | verified fixed (count = 1 in derive, measure and op cases) |
| N3 constant derive/measure broadcast | verified fixed (tran 109, op 1, ac 3, sweep 4 samples; measure = `2 dimensionless`) |
| L2 warning for a dropped sibling analysis | verified where the stitch succeeds; absent when it fails (NEW-2) |
| Regression sweep | 552 passed / 0 failed across 6 crates, 7 examples check clean, legacy order intact, CSV/JSON shapes unchanged |

Two blocking defects remain, both in the parameter-sweep export path: NEW-1 breaks the frozen
contract §4 exactly as the original B1 did (and makes `check`/`docs/language.md:603-606` untrue),
NEW-2 makes the one binding the capability check permits fail at run time. Both are fixable in
`crates/circuit-session/src/execute.rs`: carry the per-point `implicit_only` through `stitch`, and
build the stitch from the dataset of `swept_task(plan)` instead of `datasets[0]`.
