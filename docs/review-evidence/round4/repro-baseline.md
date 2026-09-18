# Round 4 reproduction baseline (lead, before any fix)

Date: this round. Build: `cargo build -p circuit-cli` (debug, unmodified tree).
Inputs and outputs are kept under `target/round4/repro/` (local evidence, not versioned).

## Files

- `target/round4/repro/r4-01.cdsl` — experiments `bad_sqrt` (`derive :invalid, expr: sqrt(-1)`,
  `measure :masked, max: min(sqrt(-1), 2)`) and `overflow` (`1e308 * 1e308` and its nested `min`),
  on the round-3 two-resistor divider.
- `target/round4/repro/r4-02.cdsl` — `derive :huge, expr: v(:vin) * ... * v(:vin)` with 128
  factors, plus `measure :m, max: abs(v(:out))`.

## R4-01 — an illegal intermediate value is masked

```text
> target/debug/cdsl.exe check target/round4/repro/r4-01.cdsl
... exit 0; the derive and the measure are listed as accepted

> target/debug/cdsl.exe run target/round4/repro/r4-01.cdsl --experiment bad_sqrt \
      --out target/round4/repro/out_bad_sqrt --format both
experiment `bad_sqrt` on circuit `divider` (backend thevenin 0.5.0)
  op1: scalar; signals: v(out), v(vin), i(src), invalid
  measure masked = 2 dimensionless (op1)
  wrote target/round4/repro/out_bad_sqrt/bad_sqrt.op1.csv
  wrote target/round4/repro/out_bad_sqrt/bad_sqrt.op1.json
... exit 0
```

The JSON carries `"diagnostics": []` and the derived signal `invalid` has no finite value (null),
while the measure reports 2. Nothing tells the user why. The same happens for `1e308 * 1e308`.

## R4-02 — a user-written expression panics the debug CLI

```text
> target/debug/cdsl.exe check target/round4/repro/r4-02.cdsl
... exit 0 (the 128-factor product is accepted)

> target/debug/cdsl.exe run target/round4/repro/r4-02.cdsl --experiment dim_overflow \
      --out target/round4/repro/out_dim
thread 'main' panicked at crates/circuit-core/src/units.rs:49:19:
attempt to add with overflow
... exit 101
```

`crates/circuit-core/src/units.rs` held the dimension exponents in `i8` and added them with the
unchecked operator, so `V^127 * V` overflowed: a panic under `debug-assertions`, a silently
wrapped exponent without them. `check` did not see it because `ExprIr::static_dimension_error`
never computed the dimension of a `Mul` node.

## R4-03 — the export path drops its diagnostics

Read from the pre-fix code: `circuit-session/src/execute.rs:1002` and `:1007` call
`circuit_results::to_csv(d)` / `to_json(d)`, the text-only wrappers that throw away
`Export.diagnostics` (`circuit-results/src/export.rs:59` and `:156`). The renderer itself already
reports non-finite samples through `to_csv_with_diagnostics` / `to_json_with_diagnostics`
(`export.rs:64`, `:161`, `non_finite_diagnostics` at `:338`), so a file with empty/null cells is
written without the user ever being told. Independent coverage is the QA worker's
`crates/circuit-session/tests/r4_*.rs` case that builds a Dataset with NaN/Inf directly.

## Release behaviour

`target/release/cdsl.exe` in this workspace is a stale round-2 binary (it does not even parse
`derive`: `error[E_SYNTAX]: unexpected \`derive\``), so it cannot serve as a pre-fix release
baseline. The pre-fix release semantics of the unchecked `i8` addition follow from the language
rather than from a measurement, and are therefore *not* claimed as observed evidence here. What is
verified and required for phase A is the release behaviour of the **fixed** tree: the 128-factor
input must be refused with a diagnostic and no panic in both debug and release
(`cargo test --release -p circuit-cli --test r4_* ` plus a direct release binary run).
