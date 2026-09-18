# QA: `_probe` `breakpoint_study` warning cleanup (task-8, Part A)

Status: **PASS** — the two local source warnings are gone, the numeric output is
byte-for-byte identical before and after, and the exit code is 0 in both runs.

## 1. Baseline warnings (BEFORE)

Command (own target dir; `_probe` is a separate project and is **not** covered by
any workspace gate — `cargo test/clippy/fmt --workspace` from the repository root
does not see it):

```
cd _probe
cargo build --bin breakpoint_study 2>&1
```

Recorded: exit code 0, **3 warnings**, of which **2 are source warnings**:

| # | warning | location |
|---|---|---|
| 1 | `unused_mut`: variable does not need to be mutable | `src/bin/breakpoint_study.rs:805` — `let mut check = \|t: f64, worst_residual: &mut f64, worst_residual_at: &mut f64\| ...` |
| 2 | `dead_code`: field `t_prev` is never read | `src/bin/breakpoint_study.rs:643` — in `struct BpRow` |
| 3 | `linker_messages`: `linker stdout: 正在创建库 ...breakpoint_study.lib 和对象 ...breakpoint_study.exp` | msvc link step, **not** a source warning |

The same 3 warnings appear for `cargo build --release --bin breakpoint_study`
(exit 0), so the profile makes no difference.

## 2. The fix (warning-only, no global allow)

Three added/removed lines in `_probe/src/bin/breakpoint_study.rs` only; **no**
`#[allow]`, no `#![allow]`, no `Cargo.toml` `[lints]` change, and no other
`_probe` binary touched.

```diff
@@ -641,7 +641,6 @@ struct BpRow {
     /// `0` for the first period, then the pulse index.
     k: u64,
     kind: &'static str,
-    t_prev: f64,
     h_before: f64,
     h1: f64,
@@ -708,7 +707,6 @@
             rows.push(BpRow {
                 bp,
                 k,
                 kind,
-                t_prev: t[i],
                 h_before,
                 h1,
@@ -803,7 +801,7 @@
     // (where the 2nd-order cancellation would show up if the reference used
     // the naive ramp form).
     let mut worst_residual = 0.0_f64;
     let mut worst_residual_at = 0.0_f64;
     let mut worst_input = 0.0_f64;
-    let mut check = |t: f64, worst_residual: &mut f64, worst_residual_at: &mut f64| {
+    let check = |t: f64, worst_residual: &mut f64, worst_residual_at: &mut f64| {
```

Rationale, per warning:

* `unused_mut`: `check` is called only, never reassigned, and it captures
  nothing mutably (it takes the accumulators as `&mut` arguments), so `mut` is
  dead. Dropping it is what `cargo fix` suggests.
* `dead_code` field `t_prev`: `t_prev` was written once and read nowhere
  (`grep t_prev _probe/src` matches only the declaration and the initialiser).
  It is a copy of `t[i]`, which is still available through `bp` on the same row,
  so removing the field removes no information that the report uses.
  It is **not** printed anywhere, so no output line can change.

## 3. AFTER: build diagnostics

```
cd _probe
cargo build --bin breakpoint_study 2>&1          -> exit 0, "generated 1 warning"
cargo build --release --bin breakpoint_study 2>&1 -> exit 0, "generated 1 warning"
```

The single remaining warning is the pre-existing `linker_messages` line emitted
by the MSVC linker (`正在创建库 ... .lib 和对象 ... .exp`); it is rustc reporting
the linker's stdout, not a warning about this source file. The task scope was the
local `unused_mut` / unused-field warnings, so it is deliberately left alone.

Absent from the AFTER builds: `warning: variable does not need to be mutable`
and `warning: field `t_prev` is never read`.

## 4. BEFORE / AFTER numeric comparison

The binary is deterministic (identical hash across two independent runs of the
debug and of the release build), so the two builds were compared byte for byte.

```
# BEFORE (captured before the edit)
_probe\target\debug\breakpoint_study.exe   > target/round3/qa/bp-debug-before.out     exit 0   27560 bytes
_probe\target\release\breakpoint_study.exe > target/round3/qa/bp-release-before.out   exit 0   27560 bytes

# AFTER (rebuilt after the edit)
_probe\target\debug\breakpoint_study.exe   > target/round3/qa/bp-debug-after.out      exit 0   27560 bytes
_probe\target\release\breakpoint_study.exe > target/round3/qa/bp-release-after.out    exit 0   27560 bytes
```

Result:

* debug before vs debug after: **byte-identical** (`-ceq` true, 27560 == 27560 bytes, 133 lines)
* release before vs release after: **byte-identical** (`-ceq` true)
* debug before vs release before: **byte-identical** (same SHA-256
  `01ccaed6b048a9cbe756de4f00d4e6f611df692cb05b2815f70e8549bffadd32`) — the
  numbers do not depend on the profile either.
* every run exits **0** and prints `RESULT: ALL EXPERIMENTS RAN, ALL CONTRACT
  CHECKS MET (exit 0)`; all **14** contract checks pass.

**Which build was used:** both. The release build was available (1 m 29 s from
cold, 2.15 s incremental), so no fallback to debug-only was needed. The captured
artefacts are the release ones above; the debug run is the same bytes.

### Headline numbers (unchanged, quoted from the AFTER release run)

```
  returned points = 3129, dt_min = 2.500e-10 s, dt_max = 1.000e-7 s
  ODE residual max |tau*y' + y - v_in| = 5.773e-15 V at t=2.209220e-4 s
  |reference.v_in - engine PULSE replica| max = 6.450e-14 V (independent evaluator)
  h1(1st breakpoint) = 1.0000e-8 s  vs  0.1*h_max = 1.0000e-8 s (rel.diff 5.178e-13)
  |engine - BE| = 5.175e-19 V, |engine - TRAP| = 4.999e-7 V
  contract checks: 14 total, 14 passed, 0 failed
```

### The two `[NOT-MET]` rows (known limitation, preserved verbatim)

These two rows are printed **on purpose**: they are the measured §17 misses of
the kernel's forced-Backward-Euler restart step, kept as findings. They are not
a regression and they do **not** flip the exit code (the bin documents this in
its header). Both survived the cleanup with identical numbers:

```
[NOT-MET] tau/50 (h_max = 2.0000e-6 s): 250/309 points exceed §17, max |err| = 7.3318e-4 V at t = 2.802000e-4 s, h1 = 2.0000e-7 s
[NOT-MET] tau/200 (h_max = 5.0000e-7 s): 3/729 points exceed §17, max |err| = 1.2490e-5 V at t = 1.000500e-4 s, h1 = 5.0000e-8 s
```

Count check: BEFORE and AFTER both print exactly 2 `[NOT-MET]` summary rows
(plus the per-row detail lines), so nothing was silenced by the cleanup.

## 5. Scope notes (what this does NOT cover)

* `_probe` is an **independent project with its own `target/`** (`_probe/Cargo.toml`
  depends on published `thevenin 0.5.0` / `cirq-ir 0.5.0`, not on this workspace).
  The workspace gates run by the Lead (`cargo test/clippy/fmt --workspace`) do not
  build it and therefore say nothing about it. `breakpoint_study` was run
  explicitly here.
* No other `_probe` binary (`currents`, `robustness`, `tran_contract`) was built,
  edited or verified.
* The remaining `linker_messages` warning is untouched by design.
