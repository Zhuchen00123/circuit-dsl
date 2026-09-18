# Round-3 acceptance QA (task-8, Part B)

Independent end-to-end QA of result expressions, derived signals, expression
measurements, analysis binding and export. Everything below was run and observed
by qa-worker; nothing is copied from another worker's summary.

| artefact | path |
|---|---|
| CLI acceptance suite (new) | `crates/circuit-cli/tests/expression_qa.rs` — 22 tests |
| session/export flow suite (new) | `crates/circuit-session/tests/expression_flow.rs` — 6 tests |
| `_probe` warning cleanup | `docs/review-evidence/round3/qa-probe-warnings.md` |

Target dir for every command here: `CARGO_TARGET_DIR=F:\codexprojects\dsl000\target\round3\w-qa`
(never the shared one). Scratch files are written under `CARGO_TARGET_TMPDIR`
(`target/round3/w-qa/tmp`) and under `target/round3/qa/` — never into the
repository or `examples/`.

---

## 1. Result

```
cargo test -p circuit-cli --test expression_qa     -> 22 passed; 0 failed   (exit 0)
cargo test -p circuit-session --test expression_flow -> 6 passed; 0 failed  (exit 0)
cargo test -p circuit-cli                          -> 66 passed; 0 failed   (exit 0)
cargo test -p circuit-session                      -> 50 passed; 0 failed   (exit 0)
cargo test -p circuit-backend --test output_interval_regression \
                             --test transient_reference_regression -> 10 passed; 0 failed (exit 0)
```

Every contract §7 scenario is covered by a test that asserts numbers, exit codes
or file contents. Two contract rows are **not reachable from the DSL surface**
(§7 finding F1 below); that is reported, not papered over.

---

## 2. Scenario by scenario

### §7.1 RC gain at the cut-off frequency — PASS

Source: R = 1 kohm, C = 159.15494309189535 nF (`C = 1/(2*pi*R*1 kHz)`), AC sweep
100 Hz -> 10 kHz at 40 points/decade, so sample **40** lands on Fc:

```
frequency,v(vin)_re,v(vin)_im,v(vout)_re,v(vout)_im,gain_re,gain_im,gain_db
...
1000.0000000000003,1,0,0.4999999999999997,-0.49999999999999994,0.4999999999999997,-0.49999999999999994,-3.010299956639815
```

Observed on **that** sample (the test finds the nearest sample and then asserts
`|f - Fc|/Fc < 1e-12`, so it cannot borrow a neighbour's value):

| quantity | observed | reference recomputed from R, C and f |
|---|---|---|
| `gain_re` | 0.4999999999999997 | `1/(1+(wRC)^2)` = 0.4999999999999998 |
| `gain_im` | -0.49999999999999994 | `-wRC/(1+(wRC)^2)` = -0.49999999999999999 |
| `\|gain\|` | 0.7071067811865472 | `1/sqrt(1+(wRC)^2)` = 0.7071067811865472 |
| `gain_db` | -3.010299956639815 | `20*log10(\|H\|)` = -3.010299956639814 |

Contract numbers: `gain = 0.5 - 0.5j` (1e-9), `|gain| = 0.70710678` (1e-8),
`gain_db = -3.01029996` (1e-8) — all met. The test also re-derives `gain_db`
from `|gain|` at every one of the 81 samples.

`measure :peak_gain, max: abs(v(:vout)/v(:vin))` -> `0.9950371902099894 dimensionless (ac1)`,
matching an independent maximum over the CSV, and reported with its analysis id.

### §7.2 Resistor power in a transient run — PASS

```
cdsl run ... --format csv
  measure avg_power = 0.000049999859482767916 V*A (tran1)
```

Raw grid: 10018 samples, total width exactly 1e-3 s (no `output_interval:`, so
the CSV **is** the raw grid). Independent trapezoids over that file:

| reference | value | agreement with the measure |
|---|---|---|
| `∫(vin-vout)^2/R dt / ∫dt` | 4.9999859482767922e-05 | rel 1.32e-16 |
| `∫(vin-vout)*i(r1) dt / ∫dt` | 4.9999859482767916e-05 | rel < 1e-15 |

Sign convention checked explicitly: `max \|v*i - v^2/R\| = 2.17e-19` over every
sample (so `i(r1) = v(vin,vout)/R` exactly), the minimum sample product is >= 0,
and the average is positive — the resistor absorbs. Unit is `V*A`.

### §7.3 Fine vs coarse `output_interval:` — PASS

| | exported points | measure printed |
|---|---|---|
| `output_interval: 1.us` | 1001 | `measure vavg = 0.9000040403243671 V (tran1)` |
| `output_interval: 20.us` | 51 | `measure vavg = 0.9000040403243671 V (tran1)` |
| no `output_interval:` (raw) | 10018 | same value |

* The two measure lines are **byte-identical**, and equal to a trapezoid over the
  raw grid (rel < 1e-12).
* Derived `vdiff` agrees at every shared time point (all 51 coarse times are on
  the fine grid; max delta < 1e-12).
* Observation: a trapezoid over the **1 us exported view** is 0.9000032116024738,
  i.e. 9.2e-7 (relative) below the measure. That is the correct reading of the
  contract — measurements are computed on the raw grid *before* resampling — and
  the test asserts the drift stays in (1e-9, 1e-5) so an implementation that
  measured after resampling would fail.

### §7.4 `save v(:vin)` only, expression reads `v(:vout)` — PASS

```
experiment e   : time,v(vin),vdiff          (1018 rows)  <- v(vout) absent
experiment both: time,v(vin),v(vout),vdiff  (1018 rows)
```

`vdiff = v(vin) - v(vout)` holds on every row of the exporting run (and equals
the unsaved-run column value on every row), so the implicit probe really was
solved and read; it is simply not exported. The in-memory side is checked in
`expression_flow.rs`: the raw dataset carries `v(vin), v(vout), vdiff`, the
output view carries `v(vin), vdiff` only.

### §7.5 Multi-analysis binding — PASS

| case | observed |
|---|---|
| `derive :g, ..., analysis: :ac1` + `derive :gdc, ..., analysis: :op1` | `two.op1.csv` = `v(vout),gdc` (gdc = 1), `two.ac1.csv` = `frequency,v(vout)_re,v(vout)_im,g_re,g_im` |
| `measure :gm, max: abs(...), analysis: :ac1` | `measure gm = 0.998031904503645 dimensionless (ac1)` = max over the CSV |
| `measure :vop, max: v(:vout), analysis: :op1` | `measure vop = 1 V (op1)` |
| omitted binding, 2 analyses | exit 1, `E_AMBIGUOUS: `g` could be evaluated on any of 2 analyses, so this derive needs `analysis:`` + `= available analyses: op1, ac1` + `= add `analysis: :op1` to choose one` (also refused by `check`) |
| unknown id `analysis: :ac9` | exit 1, `E_NAME: `ac9` is not an analysis of this experiment` + `= available analyses: op1` + `= an analysis identity is `{kind}{ordinal}`, e.g. `:ac1`` |
| bare `analysis: :ac` (no ordinal), with an AC analysis present | exit 1, same `E_NAME`, `= available analyses: op1, ac1` |
| legacy bare probe, `op + dc` | `measure vm = 1.5 V (dc1)` — DC before OP |
| legacy bare probe, `op + dc + ac + tran` | `measure vm = 0.632118751197524 V (tran1)` = the transient maximum; all four analyses still exported (`e.ac1.csv, e.dc1.csv, e.op1.csv, e.tran1.csv`) |

### §7.6 Negative cases — PASS

Each has its own test asserting the exact code, the message body, the exit code 1
and that no output exists afterwards.

| case | code | observed message (excerpt) | exit |
|---|---|---|---|
| `v(a) + i(b)` | `E_DIMENSION` | `cannot apply `+` to `v(out)` (V) and `i(r1)` (A): the units differ` | 1 (`run` and `check`) |
| `v(:out)/0` at run time | `E_VALUE` | `division by zero in `(v(out) / 0)` at sample 0 of analysis `op1`: `0` is 0` + analysis/signal/sample/index contexts + `no epsilon is applied` | 1 |
| `gain_db(v(:in), v(:out))` with v(out) = 0 | `E_VALUE` | `division by zero in `(v(in) / v(out))` ... `v(out)` is 0` | 1 |
| `i(:c1)` (capacitor) | `E_BACKEND` | `no branch current reported for `c1`` + `the engine reports branch currents only for elements that own a branch unknown` | 1 |
| `i(:nope)` (unknown device) | `E_NAME` | `unknown device `:nope`` + `devices: v1, r1, r2` | 1 |
| `v(:nope)` | `E_NAME` | `unknown node `:nope`` + `nodes: in, out` | 1 |
| `max: v(:vout)` bound to `ac1` | `E_TYPE` (check stage) | ``max` of `v(vout)` is not ordered: the samples of `ac1` are complex` + `apply `abs(...)` first, e.g. `max: abs(v(:out))`` | 1 |
| `max: abs(v(:vout))` (the advice) | — | `measure m = ... (ac1)` = 0.9980319045036448, matching `1/sqrt(1+(2*pi*100*RC)^2)` | 0 |
| `avg`/`rms` bound to `op1` | `E_TYPE` | ``avg` needs a time axis, but analysis `op1` has none` + `bind it to a transient analysis` | 1 |
| duplicate `derive :same` | `E_DUPLICATE` | ``same` is already defined as a derived signal` + `first defined here` | 1 |
| `derive :same` + `measure :same` | `E_DUPLICATE` | same message (one result namespace) | 1 |
| any run-time failure | — | no `wrote` line, **no output directory created**, no `measure` line printed | 1 |

```
$ cdsl run ... --out <dir>            # division by zero
$ echo $?            -> 1
$ ls <dir>           -> does not exist
```

### §7.7 / §7.9 CSV and JSON export of derived signals — PASS

```
e.ac1.csv : frequency,v(vin)_re,v(vin)_im,v(vout)_re,v(vout)_im,gain_re,gain_im,gain_db
e.ac1.json : analysis "ac1", kind "ac", axis {type frequency, unit Hz, 81 values},
             signals: v(vin) complex V, v(vout) complex V, gain complex dimensionless,
                      gain_db real dimensionless
```

* The real derived signal has **no** `gain_db_im` column; the complex one keeps
  both parts.
* Every CSV field was compared against the **in-memory** `f64` of
  `RunOutcome.output_datasets` with `format_number`: text-for-text equality for
  all 81 rows x 4 derived columns, and `field.parse::<f64>() == value` for the
  re/im parts (a written float reads back bit-identically).
* The JSON text must contain the same decimal field text for the derived
  columns — checked on sampled rows.
* The analysis identity travels: JSON `analysis: "ac1"`, CSV file name
  `e.ac1.csv`, stdout `(ac1)`.
* Provenance is recorded in the file: `backend.settings` carries
  `derive.gain_db = gain_db(v(vout), v(vin))`.

### §7.10 File mode vs REPL — PASS

Same source through `cdsl run` and through `cdsl repl` (`:run e`):

```
file: measure peak_gain = 0.9950371902099894 dimensionless (ac1)
repl: measure peak_gain = 0.9950371902099894 dimensionless (ac1)      <- identical line
both: ac1: 81 frequency points; signals: v(vin), v(vout), gain, gain_db
```

For the bad source both exit 1 and the first error block is identical
(code + message + notes); only the location differs (`ambiguous.cdsl:13:3` vs
`<repl:9>:5:3`), which is expected. The REPL reports diagnostics on **stderr**,
the transcript on stdout.

### §7.11 Single-parameter DC sweep — PASS

```
sweep.dc_param_r.csv: parameter,v(out),scaled
500,2.25,4.5        1000,1.8,3.6        1500,1.5,3        2000,1.2857142857142858,2.5714285714285716
measure vmax = 2.25 V (dc_param_r)      measure vmin = 1.2857142857142858 V (dc_param_r)
```

* Both the explicit `analysis: :dc1` derive and the legacy bare-probe measure
  bind to the **single stitched dataset**; it is named `dc_param_r` in the file
  and on the measure lines, while `analysis: :dc1` addresses it.
* `v(out) = 3 V * 1.5k/(r+1.5k)` recomputed per point, and `scaled = 2*v(out)`.
* A binding to a non-swept analysis (`analysis: :op1` in a `op` + parameter-sweep
  experiment) is refused **before the run**:
  `E_UNSUPPORTED: `x` is bound to analysis `op1`, but this experiment sweeps the parameter `r` and only the swept DC analysis produces a result` +
  `= bind it to `dc1` or run the sweep without it`; exit 1, no output directory.

### §7.12 Regression — PASS (referenced, not duplicated)

```
cargo test -p circuit-session       -> 50 passed (session 25, expression_flow 6, tran_output_interval 5, lib 14)
cargo test -p circuit-cli           -> 66 passed (e2e 22, expression_qa 22, repl 13, lib 9)
cargo test -p circuit-backend --test output_interval_regression --test transient_reference_regression -> 10 passed
```

The round-2 invariants are owned by
`crates/circuit-session/tests/tran_output_interval.rs` and
`crates/circuit-backend/tests/output_interval_regression.rs` /
`transient_reference_regression.rs`; they were run, not copied. The round-3
suite adds only what is new: a negative `output_interval:` is still refused
(exit 1, diagnostic naming the option, no output), and fine/coarse agreement is
covered by §7.3.

---

## 3. Findings

### F1 (contract vs surface) — the "derive colliding with a saved signal" row is unreachable

Contract §5 lists `duplicate derive name, or derive colliding with a saved
signal | check | Duplicate`. The guard exists
(`crates/circuit-dsl/src/elaborate.rs:2211`, comparing the derive name with each
saved probe's name), but no source text can reach it:

* a probe name is always `v(...)` or `i(...)`;
* a `derive` name must be a bare symbol: `derive :v(out), expr: ...` and
  `derive :"v(out)", expr: ...` are both rejected with
  `E_SYNTAX: expected ... after `derive :name`, as in `derive :gain, expr: v(:out) / v(:vin)``
  (exit 1).

So the contract row is implemented but not exercised by the language. The
reachable half of the same namespace rule **is** enforced and tested
(`derive :same` twice, and `derive :same` + `measure :same`, both
`E_DUPLICATE`). Test:
`a_derive_name_can_never_collide_with_a_saved_probe`. Reported for the lead to
decide: either drop the row, or give `derive` a name form that can collide
(e.g. a quoted name) so the guard is reachable. No action taken here.

### F2 (nit, no impact) — `evaluate_measures` is not re-exported at the crate root

Contract §6 calls `evaluate_measures(plan, datasets)` "the public seam". It is
public at `circuit_session::execute::evaluate_measures` but missing from the
`pub use` list in `circuit-session/src/lib.rs`, so callers need the deeper path.
Used from that path in `expression_flow.rs`; the seam itself behaves correctly
(it reproduces every run measure exactly).

### F3 (test-harness note, not a product defect) — reading JSON back with serde_json is off by one ulp

Comparing `e.ac1.csv` and `e.ac1.json` **numerically after parsing** showed
`105.92537251772889` vs `105.92537251772887` for the same AC axis sample. The
files themselves agree to the last digit (checked by text: the JSON literally
contains the CSV field), and the discrepancy is `serde_json`'s default float
parser, which is one ulp off unless the `float_roundtrip` feature is on
(`serde_json = "1"` with no features in the workspace `Cargo.toml`). The export
tests therefore compare decimal **text** exactly and numbers at 1e-12.
No product change is warranted; noting it so a future test does not chase it.

### Verification log (final pass)

```
cargo test -p circuit-cli --test expression_qa         exit 0   22 passed; 0 failed
cargo test -p circuit-session --test expression_flow   exit 0    6 passed; 0 failed
cargo test -p circuit-cli                              exit 0   66 passed; 0 failed
cargo test -p circuit-session                          exit 0   50 passed; 0 failed
cargo test -p circuit-backend --test output_interval_regression \\
                             --test transient_reference_regression  exit 0  10 passed; 0 failed
cd _probe; cargo build --bin breakpoint_study          exit 0   1 warning (linker noise only)
```

Exit codes above were captured without merging stderr into the pipeline
(`cargo ... > out 2> err; $LASTEXITCODE`), because a merged stream makes
PowerShell report a spurious 1 for a successful cargo run.

Both new test files compile with **no source warning**; the only warnings in
their builds are the pre-existing MSVC `linker_messages` lines.

### Not covered

* `--verbose` output, GUI/LSP, and the round-2 kernel study (`_probe`) beyond
  the warning cleanup — out of scope for this task.
* The workspace-wide gates (`--workspace`) are the lead's; only crate-level
  commands were run here.
