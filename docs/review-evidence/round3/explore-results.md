# Round 3 reconnaissance — results layer (evaluator, reduction, dataset, export)

Read-only. No cargo/git/build was run; every claim below is from reading the tree at
the current working copy. Citations are `file:line`. Anything I could not verify is
marked **unverified**.

Scope: `crates/circuit-results/src/{expr.rs,measure.rs,dataset.rs,resample.rs,export.rs,lib.rs}`,
plus the call sites that decide today's behaviour (`circuit-session/src/execute.rs`,
`circuit-cli/src/run.rs`, `circuit-backend/src/thevenin.rs`) and `circuit-core`
(`diagnostic.rs`, `plan.rs`, `units.rs`, `format.rs`, `limits.rs`, `span.rs`).

## 0. The connection gap this round closes

`circuit_results::expr::eval` has exactly one production caller: `measure::measure`
(`expr.rs:269`; `measure.rs:112`). `measure_signal` is used once in the session
(`execute.rs:423`) and always with a bare probe name (`m.target_name`,
`execute.rs:413-417`). No DSL-, session- or CLI-side code constructs an `Expr`
(only `expr.rs`/`measure.rs` tests do). So today `Div`, `Mul`, `Min`, `Max`,
`Sqrt`, `GainDb` and `Abs` are unreachable from the product; only the
`Expr::Signal`-shaped path is exercised (`measure.rs:118-125`).

## 1. `expr.rs` inventory

### 1.1 Value model
- `Value { unit: Dimension, data: Data }` (`expr.rs:46-50`), `Clone + PartialEq + Debug`.
- `Value::scalar(x)` = `Data::Real(vec![x])` + `DIMENSIONLESS` (`expr.rs:53-59`) — the only
  broadcast source. `real`/`complex`/`from_signal` at `expr.rs:61-81`; accessors
  `len/is_empty/is_complex/magnitudes/as_real` at `expr.rs:83-103`.

### 1.2 AST (`expr.rs:115-145`)
`Number(f64)` 117 · `Signal(String)` 119 · `Differential{pos,neg}` 125-128 ·
`Neg` 129 · `Add`/`Sub`/`Mul`/`Div` 130-133 · `Abs` 135 · `Sqrt` 137 ·
`Min`/`Max` 138-139 · `GainDb{numerator,denominator}` 141-144.
Constructors: `voltage`→`"v(node)"` (158-160), `current`→`"i(device)"` (163-165),
`differential` (168-173), `gain_db` (176-181), `min/max` (184-191), `abs` (194-196),
`sqrt` (199-201); operator impls 204-237. `Display` 239-262 renders
`v(pos, neg)` (244) and `20*log10(abs(a / b))` (256-259) — usable verbatim in messages.

### 1.3 Evaluation and every error path
`eval(expr, dataset) -> Result<Value, Diagnostic>` (`expr.rs:269-306`).

| # | condition | code | site |
|---|---|---|---|
| E1 | signal not in dataset | `Name` + context `analysis`/`kind` + note "available signals: …" | `expr.rs:309-328` |
| E2 | `v(a)-v(b)` with different units | `Dimension` via `dimension_error` (+`with_dims`) | `expr.rs:341-349` |
| E3 | `+`/`-` with different units | `Dimension` | `expr.rs:377-379` |
| E4 | `min`/`max` with different units | `Dimension` | `expr.rs:439-441` |
| E5 | `min`/`max` of complex data | `Type` + note "apply abs(…)" | `expr.rs:442-448` |
| E6 | `sqrt` of complex data | `Type` + note | `expr.rs:410-416` |
| E7 | `sqrt` with an odd exponent | `Dimension` + `with_dims(unit, default)` | `expr.rs:417-426` |
| E8 | `gain_db` with different units | `Dimension` + `with_dims` + note | `expr.rs:469-481` |
| E9 | vector lengths neither equal nor 1 | `Value` "cannot combine a value with {a} samples and one with {b} samples" — **no analysis context** | `expr.rs:551-564` |

No other error path exists. `dimension_error` (`expr.rs:508-518`) always attaches
`expected`/`received` dims and one note.

### 1.4 Unit/dimension algebra
- `+ - min max`: strict unit equality (`expr.rs:377`, `439`); `-` on differentials 341.
- `*`/`/`: exponents add/subtract, no constraint (`expr.rs:394-405` → `Dimension::mul`/
  `div`, `units.rs:47-61`). `V/A = ohm` is a tested derivation (`expr.rs:942-956`).
- `abs` keeps the unit, real→real, complex→real magnitudes (`expr.rs:285-297`).
- `sqrt`: every exponent must be even; `half_dimension` (`expr.rs:496-499`) halves
  (V,A,s) exponents (`units.rs:43-45`).
- `gain_db`: requires **equal** units (not `is_dimensionless`), result `DIMENSIONLESS`
  (`expr.rs:469`, `488`). So `gain_db` of two V² signals is accepted.

### 1.5 Broadcasting
`combine` (`expr.rs:524-549`): `broadcast_len` accepts equal or one length-1
(551-563); real+real stays `Data::Real`, anything complex promotes both sides
(531-548). `sample_real`/`sample_complex` fall back to `0.0`/`Complex::ZERO` on an
out-of-range index (`expr.rs:567-576`) — unreachable after `broadcast_len`, but it is a
silent zero if an invariant ever breaks. Empty ∩ empty = empty (length 0) and flows
through as an empty Value; the reduction layer then reports "no samples" (`measure.rs:152-160`).

### 1.6 The three behaviours the round intends to change (today, exactly)
1. **Division by zero — silent IEEE, no diagnostic.** `Div` → `eval_product` (`expr.rs:399-401`)
   applies `|x,y| x/y` (real) or `Complex::div` (`dataset.rs:137-146`, which divides by
   `rhs.norm_sqr()`, so a zero denominator yields ±inf or NaN in **both** components).
   Nothing inspects the denominator. Downstream: `extreme` returns NaN as NaN
   (`measure.rs:161-163`), ±inf passes the NaN gate and wins the fold (`measure.rs:166-177`);
   `Measured::render` prints `NaN`/`inf` (`measure.rs:95-103`+`format.rs:32-39`,
   `format.rs:79-87`) and `format_quantity` drops the unit for non-finite values
   (`format.rs:48-51`) — so the CLI prints `measure x = inf` (`cli/run.rs:163-165`,
   `session.rs:639-647`) and exports an empty CSV field / JSON `null` plus a warning
   (`export.rs:144-149`, `322-327`, `338-388`).
2. **`gain_db` with a zero-magnitude sample → `-inf` (or NaN), no diagnostic.**
   `eval_gain_db` (`expr.rs:462-489`) computes the ratio, takes `magnitudes()`
   (`dataset.rs:242-247`) and maps `20.0 * m.log10()` (483-487). log10(0)=−inf;
   numerator 0 / denominator 0 gives 0/0=NaN, so the two zero cases differ. The result is
   `Value::real(DIMENSIONLESS, db)` (488) and is never checked again.
3. **`min`/`max` on a complex value: two different answers depending on the layer.**
   As an *expression* it is rejected (E5, `expr.rs:442-448`). As a *measurement* over a
   complex signal it silently reduces magnitudes: `extremes` uses `value.magnitudes()`
   (`measure.rs:151` → `dataset.rs:242-247`) and takes the largest/smallest |z|
   (`measure.rs:166-178`, asserted at `measure.rs:418-421`). The asymmetry is documented in
   both module headers (`expr.rs:17`, `measure.rs:19-27`).
   Dead code worth knowing: `eval_elementwise`'s complex closure
   (`expr.rs:449-455`) compares magnitudes but returns the **complex operand** — it is
   unreachable because of the E5 gate; it is the natural home for a
   "magnitude comparison" policy, and it would have to be rewritten to return `f64`.

## 2. `measure.rs`
- `Measurement` (`measure.rs:37-42`), `name` 46-53, `parse` case-insensitive 56-64,
  `needs_time_axis` 68-70 (true for Avg/Rms).
- `Measured { name, value, unit }` (`measure.rs:75-83`) — **no analysis/source field**;
  `Measured::new` 86-92; `render` = `"name = value unit"` 95-103.
- `measure(kind,name,expr,dataset)` = `expr::eval` then `reduce(kind,name,&value,&dataset.axis,&dataset.analysis)`
  (`measure.rs:106-114`). `reduce` already receives the analysis string
  (`measure.rs:131-142`) but only uses it in the no-time-axis message (189-200).
- `min`/`max` (`extremes`, 150-179): magnitudes of the value; empty → `Value` "no samples"
  (152-160); any NaN → `Ok(NaN)` deliberately (161-163, tested 592-606); fold with
  ±inf identity (166-177); unit preserved.
- `avg`/`rms` (`integral`, 182-256): axis must be `Axis::Time` else `Type` naming the
  analysis and `axis.describe()` + 2 notes (189-200); length mismatch → `Value` (203-212);
  fewer than 2 points → `Value` (213-221); trapezoid (226-233); non-advancing/`width<=0`
  → `Value` (237-246); result unit = the signal's unit (255).
- `reduce`'s `Max|Min`/`Avg|Rms` cross-calls are guarded by unreachable! branches
  (`measure.rs:169-176`, `253`).

## 3. `dataset.rs`
- `Complex` 37-171: `magnitude` = hypot (68-70), `phase_rad` = atan2 (74-76), `is_zero`
  (87-89), `norm_sqr` (93-95), `Div` divides by `norm_sqr` **without a zero guard**
  (137-146), `Display` `"{re}{:+}i"` (167-171).
- `Data` 183-267: real/complex kept apart (183-186); `sample` promotes real→complex
  (229-234); `magnitudes` = **identity for real**, |z| for complex (236-247) — this is why
  `max` of a negative voltage is that negative voltage (doc 236-241);
  `non_finite_indices` 252-267.
- `Axis` 292-366: `None`/`Time`/`Frequency`/`Parameter`; `samples()` empty for `None` (322-327);
  `kind_name` 331-338; `unit` (`None` for Parameter) 344-350; `describe` 361-366.
- `Signal { name, unit, data }` 375-411.
- `normalize_signal_name` = lowercase + strip whitespace (`dataset.rs:415-420`); lookups
  are normalized (590-595, tested 765-774).
- `Dataset` fields are **all public**: `experiment, analysis, kind, axis, signals,
  diagnostics, backend` (`dataset.rs:471-484`).
- `Dataset::new` builds then `validate(limits)` (492-512). `validate` (518-587) checks:
  per-signal length vs axis (OP expects exactly 1) 523-549; duplicate **normalized** names
  → `Duplicate` 551-569; `scalar_value_count > limits.max_result_values` → `Limit` 571-584;
  returns **all** problems as `Diagnostics` (586). `scalar_value_count` counts a complex
  sample twice (618-629); `sample_count` (607-613) treats an OP as "max signal length".
  `Limits::default` `max_result_values = 50_000_000` (`limits.rs:38`), `for_tests` 10_000
  (`limits.rs:55`).
- **Rebuilding with extra signals**: there is no `push_signal`/`with_signals`; the only
  mutator is `push_diagnostic` (641-643). A derived signal must be added either by
  mutating `signals` in place (public field) or by calling `Dataset::new` again — only the
  latter re-runs the duplicate/length/limit checks. Shape rule consequence: a length-1
  derived value is **invalid** on an axis-backed dataset (523-549) unless broadcast first;
  a duplicate name (normalized!) is rejected (551-569).

## 4. `resample.rs` (`output_interval` view)
- `OutputGrid::from_axis` guards: time axis, ≥2 points, finite interval>0, `last>first`
  (`resample.rs:62-75`). Grid = first, interior `first+k*iv < last`, last raw point
  (120-131); `point_count` matches the built grid (85-113).
- `resample_time` (139-228): non-finite/non-positive interval → `Value` (144-152); non-time
  axis → clone unchanged (154-156); degenerate axis → clone unchanged (157-161);
  **size check counts `point_count × dataset.signals.len().max(1)`** (163-166) → `Limit`;
  every signal is interpolated (real, or complex component-wise) preserving name and unit
  (186-205); backend settings `tran.output_grid`/`tran.output_points` added, `output_interval`
  deliberately not duplicated (209-213, test 473-485); rebuilt through `Dataset::new`
  (215-223) so duplicate/shape/limit validation runs **again**; diagnostics copied (224-226).
- `interpolate` (235-270): clamps to the first/last raw value (242-249), bisection
  (251-256), degenerate `t1<=t0` → `raw[lo]` (258-263). Nothing is extrapolated.
- **Appended signals resample correctly iff they are appended before resampling** and on
  the raw grid: resample iterates `dataset.signals` (188-204) and keeps names/units, so a
  derived column is interpolated too. Two consequences: (a) a signal appended **after**
  the resample keeps the raw length and no longer matches the output axis — nothing
  re-validates it; (b) extra signals count towards the resample limit (163-166), so a
  derive can newly trip `max_result_values`. Interpolation is linear on the *derived
  value* (e.g. dB), not a recomputation at the new grid points — a semantics decision.

## 5. `export.rs` (how a derived signal reaches files)
- CSV: axis column then one column per signal in dataset order (`export.rs:64-109`);
  complex → `name_re`/`name_im` (120-128); headers quoted when they contain `,"\n\r"`
  (135-141, tested 557-579); non-finite → empty field (144-149); **no duplicate-name check
  here** (validation happens in `Dataset::validate`).
- JSON: `schema`/`experiment`/`analysis`/`kind`/`axis`/`signals`/`backend`/`diagnostics`
  (175-198), per-signal name/unit/type/values (227-248); non-finite → `null` (322-327);
  `SCHEMA = "circuit-dsl.result/1"` (201).
- `non_finite_diagnostics` emits one warning per affected signal/axis with context
  `analysis` and notes "sample indices: …" + "exported as an empty CSV field and as JSON
  null" (338-388) — the existing precedent for a coordinate-carrying message.
- A derived signal added to `signals` therefore exports automatically in both formats,
  including its unit and its `_re`/`_im` split.

## 6. Test inventory relevant to the changes

Assertions of behaviour we intend to change / that will break:
- complex max/min = magnitude: `measure.rs:396-428` (asserts `max=5.0`, `min=1.0` at
  418-421), `measure.rs:430-450` (complex rms on magnitudes).
- complex `Expr::min/max` is a `Type` error: `expr.rs:840-856` (847-855).
- `magnitudes` identity for real, |z| for complex: `dataset.rs:712-741` (727-731).
- division by zero: the **only** test is `export.rs:790-797`
  (`Complex::div` by `ZERO` → non-finite → JSON `null`). No test calls `Expr::Div` with a
  zero denominator (the only division test divides by 1e-3: `expr.rs:942-956`).
- `gain_db` of a zero magnitude: **no test**; all gains are 10×, 1×, 0.5/0.5
  (`expr.rs:858-879`), complex (881-919), dimensional rejection (921-940), dB clamp
  (816-838).
- `Measured` shape/`render` text: `measure.rs:305`, CLI `e2e.rs:384-394`
  (`"measure vfinal = 0.9"`, `"measure vavg = 0.80"`).
- consumers of `RunOutcome.measures` that must keep compiling: `tran_output_interval.rs:220-227`,
  `458-489` (bit-equality fine vs coarse), `491-517` (raw-grid avg ≠ output-view avg —
  the round-2 invariant), `cli/run.rs:163-165`, `session.rs:639-647`.
- `Measured` is only built via `Measured::new` (`measure.rs:162,178,255`) and
  `RunOutcome` only via its single literal (`execute.rs:141`), so adding fields to either
  is cheap — but `extreme` does not receive `analysis` today (`measure.rs:150`), so filling
  a new `Measured.analysis` for max/min needs that argument threaded or the value stamped
  in `reduce` (which already has it, `measure.rs:131-142`).
- Failure is currently swallowed: `evaluate_measures` tries candidates and does
  `Err(_) => continue` (`execute.rs:419-432`), so a measure that never evaluates simply
  disappears from `RunOutcome.measures` and from the output. Any move to a `Result` seam
  must still distinguish "this analysis cannot support the reduction" (today `Code::Type`
  from `measure.rs:189-200`) from "the selected analysis failed" (contract §5). `Code` is
  the only discriminator in the current error channel, and `Type` is also used for
  genuinely broken expressions (E5/E6), so Code-based discrimination is ambiguous.
- Ordering is already correct for "no file written on failure": the CLI writes only after
  `execute` returns `Ok` (`cli/run.rs:87-94` then `105-120`), and `output_view` (resample)
  runs inside `execute` (`execute.rs:139`).

## 7. Building a runtime error with the sample coordinate and the analysis

Available API (all in `circuit-core`): `Diagnostic { severity, code, message, primary,
secondary, notes, context: Vec<(String,String)> }` (`diagnostic.rs:136-151`);
`Diagnostic::error` 166-168; `.at(span)` 176-181 (**silently drops a synthetic span**),
`.at_with` 183-188, `.with_label` 190-197, `.with_secondary` 199-204, `.with_note`
206-209, `.with_context` 211-214, `.with_dims` 217-220; rendering as `= key: value` lines
in `render_plain` 227-241 and `render` 244-277. `SourceSpan::synthetic`/`is_synthetic`
(`span.rs:39,47`).

So a results-layer error can carry everything the contract asks for as **context pairs +
notes**, exactly like today's best examples:
`Diagnostic::error(Code::Value, msg).with_context("analysis", dataset.analysis.clone())
.with_context("kind", dataset.kind.clone()).with_context("signal", name)
.with_context("sample", i.to_string())
.with_context("time", circuit_core::format_number(dataset.axis.samples()[i]))
.with_note(...)`
(`Dataset.analysis/kind` at `dataset.rs:473-476`; `Axis::samples` 322-327;
`format_number` re-exported at `lib.rs:74`). Precedents: `analysis` context + "sample
indices" note in `export.rs:351-364,372-384`; `analysis`/`kind` context in
`expr.rs:317-326`.

Two limits observed in code:
1. `combine` — the shared element-wise kernel — takes only `&Data` (`expr.rs:524-529`)
   and knows the index but not the dataset, the signal name or the sub-expression. A
   sample-index-carrying diagnostic must therefore be built either by pre-checking the
   operands in `eval_product`/`eval_gain_db` (which do have `dataset` and the `Expr`s) or
   by threading context into `combine` — its six call sites are `expr.rs:351`, `381-382`,
   `397`, `401`, `449`, `482`.
2. A caret pointing at the DSL statement is **not** producible inside `circuit-results`:
   `Dataset` carries no span (`dataset.rs:471-484`) and expr errors are span-free. The spans
   exist one layer up: `MeasureRequest.span`/`kind_span` (`plan.rs:253-261`),
   `AnalysisTask.span` (`plan.rs:204-209`), `AnalysisPlan.span` (`plan.rs:275`),
   `NamedProbe.span` (`plan.rs:42-47`), `Sweep.span` (`plan.rs:83-98`) — so the session/CLI
   must attach `.at(measure.span)` to an error returned from the results layer, and must
   not rely on the results layer to do it.

## Interface recommendations

D1. **Freeze where the zero-denominator check lives and what it returns.** Observed
options: (a) pre-check the evaluated denominator `Data` in `eval_product`/`eval_gain_db`
(`expr.rs:391-406`, `462-489`), where `dataset`, both `Expr`s and `Display` are available,
so the message can name the sub-expression and cite analysis + first offending index;
(b) check inside `combine` (`expr.rs:524-549`), which has the index but neither the
dataset nor the expression, and whose six call sites would all need new arguments;
(c) a post-pass over the produced `Value` (detects ±inf/NaN but cannot say which operand
or index caused it, and would also fire on legitimate input infinities that already reach
`Dataset::validate`/`export.rs:338-388`). Today the *only* precedent in the codebase is the
elaboration evaluator's `Code::Value "division by zero"` at the operand span
(`circuit-dsl/src/eval.rs:323-328`). Recommendation: (a) with `Code::Value`, and decide
separately whether `gain_db`'s zero numerator (log10(0) = −∞) and the 0/0 = NaN case get
the same code/message (they are different values today: −inf vs NaN).

D2. **Freeze the complex min/max policy as one story, not two.** Today an expression
`max(a,b)` over complex data is `E_TYPE` (`expr.rs:442-448`) while
`measure :x, max: v(complex_signal)` silently reduces magnitudes
(`measure.rs:151`+`166-178`, test `measure.rs:418-421`). Keeping both means documenting a
deliberate asymmetry and keeping the dead magnitude-comparison branch honest
(`expr.rs:449-455`); unifying on magnitude means changing `expr.rs:442-448` into a
real-valued magnitude comparison (the branch returns `Data::Complex`, so it must be
rewritten), which changes the meaning of `max(x,y)` as a *shape* operation and would make
`max(abs(a), b)`-style user code no longer obviously needed; unifying on error means
changing `measure.rs:396-428` and the §7 magnitude definition in `measure.rs:19-27`.
Note `gain_db` is already real-valued (`expr.rs:488`), and `Dataset::validate` keeps real
and complex apart (`dataset.rs:183-186`), so any policy must state whether a mixed
real/complex operand pair is promoted before the comparison.

D3. **Freeze how a failed measure surfaces, and how candidate-skipping is distinguished
from a real failure.** Today `evaluate_measures` swallows every error and returns a
shortened list (`execute.rs:419-432`), so a measure can vanish with exit code 0
(`cli/run.rs:163-165`, `session.rs:639-647`). The frozen contract wants a `Result` seam,
but the current error channel only exposes `Code`: the "cannot support the reduction"
case is `Type` (`measure.rs:189-200`) and `Type` is also what a broken complex min/max
raises (`expr.rs:442-448`), so a `Code`-based filter can swallow a genuine failure.
Options: (i) discriminate on `Code` only (smallest diff, brittle as shown);
(ii) add an explicit "inapplicable candidate" outcome to `circuit-results` (new public
type on `reduce`/`measure`, cleaner, touches `measure.rs`'s API and its callers);
(iii) pre-filter candidates structurally before evaluating using
`Measurement::needs_time_axis` (`measure.rs:68-70`) plus `Dataset.axis`/`kind`
(`dataset.rs:473-475`), so any error that *is* raised always propagates (no results-API
change, but the filter must be kept in sync with the reduction rules).
Also freeze whether a failed expression/measure aborts the whole run **before** files are
written: the CLI already only writes after a successful `execute` (`cli/run.rs:87-120`),
so this holds as long as the error is raised inside `execute` (before `execute.rs:139`).

D4. **Freeze how derived signals are appended and named.** No mutator exists beyond
`push_diagnostic` (`dataset.rs:641-643`) and `signals` is a public field (`479`): either
mutate in place (cheap, skips duplicate/length/`max_result_values` validation unless
`validate` is called explicitly, `518-587`) or rebuild via `Dataset::new` (re-runs every
check, needs `&Limits`). Consequences already visible in code: names are compared
normalized (`415-420`, `551-569`) so `Gain`/`gain` collide and a derive colliding with a
`save` probe is a `Duplicate`; a length-1 derived value is invalid on an axis-backed
dataset (`523-549`); complex samples count twice against the limit (`618-629`); appended
signals are resampled only if appended before `output_view` (`execute.rs:139`,
`resample.rs:188-204`) and they increase the resample size estimate
(`resample.rs:163-166`); they become CSV/JSON columns automatically with the
`_re`/`_im` split (`export.rs:75,88,120-128,190`). Freeze also whether a derived value is
interpolated as data (linear on dB, i.e. not recomputed at grid points —
`resample.rs:186-205`) or recomputed on the output grid; the current resampler can only
do the former without new API.

D5. **Freeze how `analysis: :ac1` is resolved and how the identity is reported.**
`Dataset.analysis`/`kind` already carry `ac1`/`ac` (`dataset.rs:473-476`) and the session
already parses the `tran{n}` ordinal back out for the resample mapping
(`execute.rs:174-183`); `Measured` has no analysis field and `RunOutcome` no bindings
(`measure.rs:75-83`, `execute.rs:34-43`). Two shapes: resolve the id by string match on
`Dataset.analysis` at evaluation time (no new fields, but nothing normalizes analysis ids
the way `normalize_signal_name` normalizes signal names — `dataset.rs:415-420` — so
casing/whitespace rules must be stated), or carry `RunOutcome.bindings`
(contract §6; exact and reorder-proof, cheap to add since `execute.rs:141` is the only
`RunOutcome` literal). For `Measured.analysis`, `reduce` already takes the analysis string
(`measure.rs:131-142`) so the avg/rms path can stamp it with no signature change, while
the max/min path does not (`measure.rs:150`) and needs the value threaded or stamped in
`reduce`; `Measured::render` must stay unchanged because CLI tests match its exact text
(`e2e.rs:384-394`) — the contract's separate `render_with_analysis` is the compatible
shape.
