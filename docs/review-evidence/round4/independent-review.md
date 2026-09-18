# 第 4 轮阶段 A+B 独立复核报告（reviewer: task-7 + task-8 + task-14）

本文件分三部分：

- **第一部分（task-7，§0–§9）**：缺陷独立复现与契约审计，快照 S1（实现中途，§1.3/§1.5 尚未落地）与
  S2（T2–T5 落地后）。
- **第二部分（task-8，§10）**：阶段 A **冻结快照**（T2–T10，哈希清单见 §10.1）的闭门复核与
  design-contract §3 验收矩阵逐行核对。
- **第三部分（task-14，§11）**：阶段 B **冻结快照**（参数 DAG，哈希清单见 §11.1）的闭门复核、
  计划 §5.3 验收矩阵、阶段 A 三处 P3 复验与 58 用例回归重跑。

**阶段 B 复核结论（task-14）：阶段 B 可以关闭。** 契约 §4.1–§4.7 与计划 §5.3 逐条通过；
环诊断的闭合路径与逐声明 span、E_TOPO_PARAM 的三段解释路径 span 均落到真实声明/使用点；
阶段 A 无回归；仅 3 项 P3 文档/证据问题（§11.6）。

**冻结复核结论（task-8）：阶段 A 可以关闭。** R4-01 / R4-02 / R4-03 三条缺陷闭环；契约
§1.1–§1.5 与 §3 验收矩阵逐行通过；FINDING-1（深嵌套 abort）已修复并在 debug 与 release 双档验证。
另有 3 项 P3 级文档/边界一致性问题（§10.7），不阻塞关闭。task-7 阶段的 NEEDS_FIX（契约 §1.4 措辞）
已由 lead 按裁决 A 改写，复验通过（§10.5、§10.7）。

- 写集边界：只写了本文件与 `target/round4/reviewer/**`；未修改生产代码、测试或他人文件；未执行任何 git 写命令。
- 全部复现输入为本人自建，未复用 `target/round4/repro/`。
## 0. 快照、哈希与并发说明

取证期间（本会话）其他 worker 正在并行编辑。以下哈希为 `Get-FileHash -Algorithm SHA256`（小写）。
S1 = 实现中途（§1.3/§1.5 未落地）；S2 = 最终快照。S2 的哈希在我全部命令执行**前后各取一次，逐字节一致**
（`target/round4/reviewer/final-hashes-before.txt` / `final-hashes-after.txt`），因此下文 S2 结论对应固定版本。

| 文件 | S1（中途） | S2（最终） |
|---|---|---|
| `crates/circuit-core/src/units.rs` | 921a6ae3f1a89a5577c9afddbb9f9b0cc189b211c719e379cc58f510a3e9e743 | 同左 |
| `crates/circuit-core/src/plan.rs` | fa042d72…（中途）→ 0f8e2f45403c9d94db490dc751c6fecd257d37ced481b5fdcc9aa7b518ddfea0 | 0f8e2f45…（同） |
| `crates/circuit-core/src/format.rs` | adf067669f830ecb02bc73a3fbdae5bf86f9ae17262e8c7a0f0422846638cacf | 同左 |
| `crates/circuit-dsl/src/eval.rs` | 5a7b0890…→4f48f7cb6bc3a6286eba2eb4696a524d3aed2328124b2a1b60c81e39f8367faa | 4f48f7cb…（同） |
| `crates/circuit-dsl/src/elaborate.rs` | – | aab383334d33f481d359ce143f81a6cd877418d34f4730e89f93026a7cb281c5 |
| `crates/circuit-dsl/src/parser.rs` | – | a461fce204302d600ebde59cb6351af3d21c77e82ef3e2556fa1a790a0746b71 |
| `crates/circuit-results/src/expr.rs` | 732e4184…→1dddf4f2…（§1.3 未落地） | c5eb03a95bc18f868d65c0c12ba610e6cd9a5a8b96aea31fb36b5384e697d15c |
| `crates/circuit-results/src/measure.rs` | – | ec6dfe70123d86926e3ba849ad05d6c2cedf00d83cdc5d1c7fd1630c133e610f |
| `crates/circuit-results/src/export.rs` | dc2bdf4cc019b5166d4e98b66002076a63bee203c24c471e62285ae7a58585e7 | 同左 |
| `crates/circuit-session/src/execute.rs` | 0bcb4355…（§1.4 前）→ ab60b8b2…（§1.4 后） | f751c12cab2707e6f36c84e500c5b2122ee7de6f4205f94ac56e1f7290aae8f7 |
| `crates/circuit-cli/src/check.rs` | 0e13ade980a616a4516abd14c3ee277fdb6471bb8d13bbd570b60dc35b7b5db0 | 31da28a6b0bb28bc157a28e44ea0559d531ede20865973c1dda42f821fd2737d |
| `crates/circuit-cli/src/run.rs` | – | 5ec5ba98d1ef62e5c0ac334291a532af4910e45aa9a12c661be1dcc8c764d6d9 |
| `crates/circuit-cli/src/repl.rs` | 1243d2306042215f5961d94273e16ff7a62aa93f0d563136f609258bbe8d2072 | 同左（REPL 接线在 `session.rs`，见 §4/§1.4） |
| `crates/circuit-cli/src/main.rs` | – | 6c1b1680156e9c911ad3844396cbb21a80ee8aa76c343f27027b61fe0bd74cda |

环境：Windows + PowerShell 7；debug 用 `cargo build -p circuit-cli` 产出的 `target/debug/cdsl.exe`，
release 用 `cargo build -p circuit-cli --release` 产出的 `target/release/cdsl.exe`。
全部 `run` 都用显式 `--out`，输出目录均在 `target/round4/reviewer/` 下。
每条命令的原始 stdout/stderr 与退出码保存在 `target/round4/reviewer/logs/<case>.log`（文件名见各节）。
复现输入哈希见 `target/round4/reviewer/input-hashes.txt`。

---

## 1. R4-01 非法中间值被掩盖 —— PASS（S2）

### 1.1 `sqrt(-1)` 与 `min(sqrt(-1), 2)`

输入：`target/round4/reviewer/rev_sqrt_masked.cdsl`（自建分压电路 `:rev_ldr`，3 kΩ / 1 kΩ）：

```ruby
experiment :sqrt_domain, circuit: :rev_ldr do
  op
  save v(:in), v(:out), i(:r_hi)
  derive :root_of_negative, expr: sqrt(-1)
  measure :min_cannot_hide, max: min(sqrt(-1), 2)
end
```

**S1（缺陷复现，证据 `logs/run_sqrt.log`）**

```
> target\debug\cdsl.exe run target/round4/reviewer/rev_sqrt_masked.cdsl --experiment sqrt_domain --out target/round4/reviewer/out-sqrt
exit 0
  op1: scalar; signals: v(in), v(out), i(r_hi), root_of_negative
  measure min_cannot_hide = 2 dimensionless (op1)          <-- 非法中间值被 min 掩盖
  wrote .../sqrt_domain.op1.csv
  wrote .../sqrt_domain.op1.json
  warning[E_VALUE]: signal `root_of_negative` has 1 non-finite sample(s)   <-- 只是导出警告，退出码仍为 0
```

即：运行成功、测量返回正常数字 2、派生列写成空值/null，仅有一条警告。R4-01 的两条症状（掩盖 + 不拒绝）在此快照成立。

**S2（修复后验证）**

```
> target\debug\cdsl.exe check target/round4/reviewer/rev_sqrt_masked.cdsl              -> exit 1
> target\debug\cdsl.exe run   ...rev_sqrt_masked.cdsl --experiment sqrt_domain --out .../out2/sqrt  -> exit 1
```

check（常量路径，§1.5）原文（`logs/f_check_sqrt.log`）：

```
error[E_VALUE]: sqrt of a negative sample in `sqrt(-1)`: `-1` is -1 there, and a negative real number has no real square root
  = signal: -1
  = index: 0
  = derive: root_of_negative
  = no dataset and no analysis axis: this is one scalar sample of a constant expression
  = sqrt is defined for samples >= 0: the illegal sample is reported, never evaluated to NaN
error[E_VALUE]: sqrt of a negative sample in `sqrt(-1)` ... (同一表达式作为 measure 再报一次)
  = measure: min_cannot_hide
```

run（运行期路径）原文（`logs/f_run_sqrt.log`，`exit 1`，无 `wrote` 行）：

```
error[E_VALUE]: derive `root_of_negative`: sqrt of a negative sample in `sqrt(-1)` at sample 0 of analysis `op1`: `-1` is -1 there, and a negative real number has no real square root
  = analysis: op1
  = kind: op
  = signal: root_of_negative
  = sample: sample 0
  = index: 0
  = expression: sqrt(-1)
  = this analysis has no axis: the value is one scalar sample, named by its index
  = sqrt is defined for samples >= 0: the illegal sample is reported, never evaluated to NaN
```

### 1.2 `1e308 * 1e308` 与嵌套 min/max

输入 `rev_overflow_masked.cdsl`：`derive :huge_product, expr: 1e308 * 1e308` +
`measure :min_cannot_hide_huge, max: min(1e308 * 1e308, 5)`。

- S1（`logs/run_overflow.log`）：`exit 0`，`measure min_cannot_hide_huge = 5 dimensionless (op1)` —— 同样被掩盖。
- S2：`check` `exit 1`（`logs/f_check_overflow.log`）、`run` `exit 1`（`logs/f_run_overflow.log`），
  原文：`error[E_VALUE]: multiplication `*` produced a non-finite sample in `(1e308 * 1e308)`: the sample is +inf`，
  运行期带 `analysis/kind/signal/sample/index/expression` 上下文。

### 1.3 负信号开方（我构造的无量纲 -1，绕过静态量纲检查）

`rev_negative_signal_sqrt2.cdsl`：`derive :root_of_ratio, expr: sqrt(v(:vm) / abs(v(:vm)))`，其中 `v(:vm) = -4 V`，
表达式量纲为 V/V = 无量纲、值为 -1，**静态量纲检查不可能是拒绝它的原因**。

- S2 `run` `exit 1`（`logs/f_run_negsig2.log`）：
  `error[E_VALUE]: derive `root_of_ratio`: sqrt of a negative sample in `sqrt((v(vm) / abs(v(vm))))` at sample 0 of analysis `op1`: `(v(vm) / abs(v(vm)))` is -1 there, ...`
- 对照 `rev_negative_signal_sqrt.cdsl`（`sqrt(v(:vm))`，量纲 V）在 check 与 run 都 `exit 1`，
  报 `error[E_DIMENSION]: sqrt of a quantity in V is not representable: every dimension exponent must be even`（`logs/f_check_negsig.log`、`logs/f_run_negsig.log`）。

### 1.4 复数非有限分量

`rev_complex_overflow.cdsl`：AC、`ac: 1e200.V`、`derive :squared, expr: v(:vout) * v(:vout)`。

- S1（`logs/run_complex.log`）：`exit 0`，两个复数样本非有限，仅导出警告。
- S2：`check exit 0`（非常量，只做静态检查，符合 §1.5），`run exit 1`（`logs/f_run_complex.log`）：
  `error[E_VALUE]: derive `squared`: multiplication `*` produced a non-finite sample in `(v(vout) * v(vout))` at frequency = 1000 of analysis `ac1`: the real part is NaN and the imaginary part is -inf`
  并给出 `= sample: frequency = 1000`（轴坐标）与 `analysis/kind/signal/index/expression`。

### 1.5 失败不落盘

`rev_partial_fail.cdsl` 同一实验内先 `derive :fine, expr: v(:out) * 2` 后 `derive :bad, expr: sqrt(-1)`。

- S2 `run` `exit 1`（`logs/f_run_partial.log`），报 `derive `bad`` 的诊断；
- `target/round4/reviewer/out2/partial` **不存在**（整轮 15 条命令跑完后 `out2` 下文件数为 0），
  即本次失败没有产生任何成功输出文件，也没有删除已有文件。

**R4-01 结论：PASS**（S2）。严重级别不适用（已闭环）。

---

## 2. R4-02 量纲指数溢出 —— PASS（S2，debug + release）

输入 `rev_dim128.cdsl`：`derive :chain, expr:` 128 个 `v(:vin)` 相乘（`:vin` 为 1 V 直流源）。

| 命令 | S1 | S2 |
|---|---|---|
| debug `cdsl check` | exit 1（单元测试/前端口径已落地时） | **exit 1**，`E_DIMENSION` |
| debug `cdsl run` | exit 1（不再 panic） | **exit 1**，同一诊断，无 panic |
| release `cdsl check` / `run` | exit 1 / exit 1 | **exit 1 / exit 1** |

S2 原文（`logs/f_check_dim128.log`、`logs/f_run_dim128.log`）：

```
error[E_DIMENSION]: the dimension of `((((... (1392 characters) * v(vin))` is V^127 * V, which leaves the representable exponent range
(an exponent is held as a signed 8-bit integer, at most 127 in magnitude)
  --> target/round4/reviewer/rev_dim128.cdsl:10:24
```

边界对照（我自己加的 127 因子文件）：

- `cdsl check rev_dim127.cdsl` `exit 0`（V^127 可表示，无假阳性）；
- release `cdsl run rev_dim127.cdsl` `exit 0`（`logs/fin_rel_dim127.log`）；
- debug `cdsl run rev_dim127.cdsl` **abort（0xC00000FD）** —— 见 §6 FINDING，这不是量纲溢出，而是求值递归深度。

另一条 DSL 可达路径（参数表达式，走 `circuit-dsl/src/eval.rs` 的受检运算）：

- `rev_param128.cdsl`：`param :p_long, default:` 128 个 `1.V` 相乘 → `cdsl check` `exit 1`（`logs/f_check_param128.log`）：
  `error[E_DIMENSION]: `*` on a quantity in V^127 and one in V has a dimension outside the representable range`，span 指向运算符。
- 该 note 的 10 个多余空格已按 lead 说明修复，S2 输出为
  `= a dimension exponent is held as a signed 8-bit integer, at most 127 in magnitude; shorten the product or split it into named parameters`
  （`crates/circuit-dsl/src/eval.rs:361-364`，哈希 4f48f7cb…）。**复验通过。**

**R4-02 结论：PASS**（128 因子在 debug/release 都得到诊断而非 panic/回绕；127 边界合法且 release 正常；
debug 127 的 abort 属 §6 FINDING，不改变量纲判定）。

---

## 3. R4-03 会话导出丢弃警告 —— PASS（S2），但契约 §1.4 有一处表述与实现不符

### 3.1 代码证据（S1：缺陷存在）

S1 的 `crates/circuit-session/src/execute.rs`（哈希 0bcb4355…）中：

- `pub fn write_datasets(...) -> Result<Vec<PathBuf>, Diagnostics>`（`:984-989`）
- 渲染走文本接口 `circuit_results::to_csv(d)?`（`:1002`）、`circuit_results::to_json(d)?`（`:1007`），
  返回值里没有诊断；
- 而渲染器本身有 `Export { text, diagnostics }`（`crates/circuit-results/src/export.rs:31-34`），
  `to_csv`/`to_json` 只是 `Ok(to_csv_with_diagnostics(d)?.text)`（`export.rs:59-61`、`:156-158`），
  唯一携带诊断的 `*_with_diagnostics` 从未被会话层调用 → 警告被丢弃。这就是 R4-03 的代码级证明。

### 3.2 独立复现：手工构造 Dataset（S2）

我不复用任何人的测试，自建独立 crate `target/round4/reviewer/probe/`（独立 `[workspace]`，不会被
`cargo test --workspace` 拾取），依赖 `circuit-core / circuit-results / circuit-session`，构造
`Axis::None`、三个实信号 `v(nan)=[NaN]`、`v(inf)=[+inf]`、`v(ok)=[1.5]` 的合法 Dataset，
调用 `circuit_session::execute::write_datasets`。

```
> cargo run --offline --quiet --manifest-path target/round4/reviewer/probe/Cargo.toml   (exit 0)
dataset.has_non_finite = true
dataset.diagnostics (from Dataset::new) = 0
write_datasets returned: Ok(Written { paths: ["...rev_export.op1.csv", "...rev_export.op1.json"],
  warnings: [Diagnostic { severity: Warning, code: Value, message: "signal `v(nan)` has 1 non-finite sample(s)",
    notes: ["sample indices: 0", "exported as an empty CSV field and as JSON null"], context: [("analysis","op1")] },
             Diagnostic { severity: Warning, code: Value, message: "signal `v(inf)` has 1 non-finite sample(s)", ... }] })
--- rev_export.op1.csv ---
v(nan),v(inf),v(ok)
,,1.5
--- rev_export.op1.json ---  (节选)
  "diagnostics": [],
  "signals": [ { "name": "v(nan)", "type": "real", "unit": "V", "values": [ null ] }, ... ]
```

验证到的事实：`Written { paths, warnings }` 返回 2 条警告（**每 dataset 去重**：CSV+JSON 两格式各命中一次，
最终只 2 条，而不是 4 条）；CSV 空单元格、JSON `null` 的既有渲染未变；文件照常写出。
完整原文见 `logs/export_probe_final.log`；探针源码 `target/round4/reviewer/probe/src/main.rs`。

### 3.3 与契约不符之处（NEEDS_FIX，P3）

`design-contract.md` §1.4 写道：

> the JSON file keeps its own `diagnostics` array (that is the file's self-description); the returned
> warnings are the same diagnostics as values, deduplicated by `render_plain()` text

**实现不支持这半句**：

- JSON 的 `diagnostics` 数组来自 `dataset.diagnostics`（`crates/circuit-results/src/export.rs:193-196` 的
  `to_json_value`），而返回的 warnings 来自渲染期计算的 `non_finite_diagnostics(dataset)`
  （`export.rs:169`、`:338`）；两者是不同的集合。
- 我的手建 Dataset 里 `dataset.diagnostics` 为空 → 文件里 NaN/Inf 已写成 `null`，但
  `"diagnostics": []`；返回的 warnings 非空。**文件并不"自我描述"它为什么缺值。**
- 同类现象在真实 CLI 输出上也可复现：S1 的 `target/round4/reviewer/out-complex/complex_overflow.ac1.json`
  含 2 个 null 样本，但 `"diagnostics": []`（第 25 行）。

建议由 lead 二选一：(a) 在写出 JSON 时把渲染期诊断并入文件数组；(b) 把契约改成
"JSON 的 diagnostics 是 dataset 级诊断；渲染期警告由调用方展示"。二者都属契约/实现一致性，非数值缺陷。

### 3.4 用户可见性

- CLI：`crates/circuit-cli/src/run.rs:161-168` 打印 `outcome.warnings` 与 `written.warning_lines()`；
- REPL：`crates/circuit-session/src/session.rs:648-660` —— `:run <exp> --out DIR` 在写完文件后
  `lines.extend(written.warning_lines())`，与文件运行共用同一渲染（`repl.rs` 本身不含写盘逻辑，
  所以 `repl.rs` 哈希未变不构成缺口）。

**R4-03 结论：PASS**（警告不再丢失，且不重复刷屏）；附带 **NEEDS_FIX(P3)**：契约 §1.4 关于 JSON
`diagnostics` 数组的表述与实现不符。

---

## 4. 契约 §1.1–§1.5 逐条对照

| 条款 | 结论 | 证据（file:line） |
|---|---|---|
| §1.1 `Dimension::MAX_EXPONENT` | PASS | `crates/circuit-core/src/units.rs:49` = `i8::MAX` |
| §1.1 `checked_mul/div/pow` const fn → `Option` | PASS | `units.rs:61`、`:74`、`:90`；内部用 `i8::checked_add/sub/mul` |
| §1.1 `mul/div/pow` 已删除 | PASS | 全仓 grep `pub fn mul\|pub fn div\|pub fn pow` 在 `units.rs` 无命中（残留的 `.mul(`/`.div(` 只在 backend 测试里操作 `Cx` 复数） |
| §1.1 `Quantity::checked_mul/div`，`Mul/Div` 运算符删除，`Neg` 保留 | PASS | `units.rs:277`、`:289`、`:297`；`impl Mul/Div for ...` 无命中 |
| §1.1 调用点：plan/eval 用受检形式 + `Code::Dimension` | PASS | `plan.rs:200-201`、`:305-321`；`eval.rs:326-334` + `dimension_exponent_overflow` `:347-365`（`E_DIMENSION`，span 在运算符） |
| §1.1 `format.rs` 与测试用 `checked_*` + 显式 expect | PASS | `format.rs:234-236`、`:292-294` |
| §1.2 `static_dimension` 签名不变、越界返回 `None` | PASS | `plan.rs:187-213`（`:200-201` 用 checked，溢出即 `None`） |
| §1.2 `static_dimension_error` 先报溢出再报单位不匹配 | PASS | `plan.rs:217-223`（`:221` 先于 `binary(...)`）；`exponent_overflow_error` `:299-334` |
| §1.2 被 `elaborate::lower_result_expr` 消费 → `cdsl check` 直接拒绝 | PASS | `crates/circuit-dsl/src/elaborate.rs:2359-2366`；derive `:2238`、measure `:2265` 都经过它；运行验证见 §2 |
| §1.3 `EvalKind` / `EvalSite{kind,name}` / `derive` / `measure` | PASS | `crates/circuit-results/src/expr.rs:345`、`:371-386`（另有 `:396` `anonymous()`，契约 §1.3 的 `eval` 等价形式所需） |
| §1.3 `eval` 保留且等于 `eval_at(.., anonymous)` | PASS | `expr.rs:420-422` |
| §1.3 `eval_at` / `is_constant` / `eval_constant` | PASS | `expr.rs:428`、`:439`、`:465` |
| §1.3 策略 1：每个运算校验自身结果（实/复有限） | PASS | `expr.rs:608-617`（`produced a non-finite sample`）、`:1125-1136`（`first_non_finite`，实 `is_finite`、复两分量）；运行验证见 §1.2/§1.4 |
| §1.3 策略 2：sqrt 负值 / 分母精确 0 / gain_db 零幅值，无 epsilon | PASS | `expr.rs:952`、`:1063-1080`、`:1083-1097`；`:1171` 注释亦声明精确等值 |
| §1.3 策略 3：输入样本本身非有限也拒绝 | 部分验证（读代码）+ 运行验证间接 | `expr.rs:608-617` 覆盖读取运算；我没有构造"信号本身就是 NaN/Inf 后进入表达式"的 CLI 用例（见 §8） |
| §1.3 策略 4：上下文 analysis/kind/signal/sample/index + 无轴时说明标量 | PASS | 运行输出见 §1.1；常量文本 `expr.rs:668` |
| §1.3 策略 5：常量表达式在 check 用同一错误文本 | PASS | `check.rs:176-209` 与 §1.1 原文对照（只在运行期文本上追加 `derive/measure` 名） |
| §1.3 运行期量纲守卫改为 checked | PASS | `expr.rs:731-732` |
| §1.3 `measure.rs` 传递 site | PASS（读代码） | `crates/circuit-results/src/measure.rs:144` `eval_at(expr, dataset, &EvalSite::measure(name))`；未做 measure 的运行期诊断用例 |
| §1.4 `Written { paths, warnings }` + `write_datasets` 新签名 | PASS | `crates/circuit-session/src/execute.rs:989-995`、`:1040-1043` |
| §1.4 先渲染完再写、失败不落盘 | PASS | `execute.rs:1047-1059`（先 `*_with_diagnostics` 收集，再 `:1075-1084` 写）；手工 Dataset 验证 + `:1641` 附近单测 |
| §1.4 按 `render_plain()` 每 dataset 去重 | PASS | `execute.rs:1061-1073`；探针输出 2 条而非 4 条 |
| §1.4 JSON `diagnostics` 数组与返回 warnings 是同一批诊断 | **NEEDS_FIX(P3)** | 见 §3.3：`export.rs:169/193-196` |
| §1.4 CLI/REPL 展示 | PASS | `run.rs:161-168`、`session.rs:648-660` |
| §1.5 check 求值常量 derive/measure，非常量只做静态检查 | PASS | `check.rs:176-209`（`:191` 跳过非常量，`:194-201` 求值并带 `derive/measure` 上下文）；运行验证 §1.1/§1.2 |

补充（非缺陷，措辞层面）：`MAX_EXPONENT = i8::MAX = 127`，但指数下界是 −128，诊断文本
"at most 127 in magnitude"（`plan.rs:313-317`、`eval.rs:361-364`）在符号方向上不精确；
`Quantity::checked_mul` 在量纲越界时仍先算数值乘积再丢弃（`units.rs:277-282`），无用户可见影响。

---

## 5. FINDING A：深嵌套调用在 **parse 阶段** abort（pre-existing）

- 现象：`cdsl check` / `cdsl run` 直接终止，退出码 **−1073741571 = 0xC00000FD（STATUS_STACK_OVERFLOW）**，
  stderr 只有 `thread 'main' has overflowed its stack`，**没有任何诊断**。
- 最小复现：`target/round4/reviewer/abs/abs8.cdsl` … `abs126.cdsl`（`derive :chain, expr: abs(abs(...(v(:vin))...))`）
  与 `target/round4/reviewer/deep/devabs112.cdsl`（同样的嵌套放在器件 `value:` 位置，仍 abort）。
- debug 阈值：96 层 `exit 0`；112/120/124/126/200 层 **abort**（`logs/abscheck_*.log`）。
- release 阈值：512 层 `exit 0`；1024/2048/4096 层 abort（512 → 0xC00000FD）。
- 阶段定位（自建 `target/round4/reviewer/parseprobe/`，只依赖 core+dsl，逐步打印阶段标记）：
  ```
  > rev-stage-probe.exe target/round4/reviewer/abs/abs112.cdsl
  stage=read bytes=749
  stage=lex tokens=388
  (abort，未打印 stage=parse ok)
  ```
  即 abort 发生在 `circuit_dsl::parse` 内部（lex 已成功），与结果表达式求值无关；递归下降解析器
  `parser.rs`（`expr`:1480、`unary`:1523、`call`:1744 → `arg_list` → `expr`）无深度上限。
- 归因：S1 与 S2 的阈值一致（96 通过 / 112 abort），`parser.rs` 在本轮未被本 FINDING 涉及的行为改变，
  属本轮之前的既有问题。
- 严重级别：**P1**（合法程序杀进程、无诊断、退出码不受 CLI 契约控制）。计划内 128 因子量纲输入已被
  §1.2 在**求值前**拦下，故本 FINDING 不影响 R4-02 的判定，但它是"用户可达输入不得 abort"的同类缺口。

## 6. FINDING B：结果表达式的递归深度——**本轮引入的退化**（check 常量求值 + run 求值）

同一组文件在 S1 与 S2 的行为对比（文件内容未变，哈希见 `input-hashes.txt`）：

| 输入 | S1 行为 | S2 行为 |
|---|---|---|
| `check` `target/round4/reviewer/iso/iso_const127.cdsl`（127 个 `1.0` 相乘，纯常量） | **exit 0** | **abort 0xC00000FD** |
| `check` `iso_const200.cdsl`（200 个 `1.0` 相乘） | **exit 0** | **abort 0xC00000FD** |
| `run` `ladder/lad112.cdsl`（112 个 `v(:vin)` 相乘） | **exit 0** | **abort 0xC00000FD** |
| `run` `ladder/lad120/124/126.cdsl` | **exit 0** | **abort 0xC00000FD** |
| `run` `ladder/lad127.cdsl` | abort | abort |

S2 的系统阈值（全部 debug，`check exit 0` 但 `run` abort；`logs/thr_run_*.log`、`logs/cst_*.log`）：

| 形状 | debug check | debug run | release check | release run |
|---|---|---|---|---|
| 常量乘法链 32/64 | 0 | 0 | 0 | 0 |
| 常量乘法链 96…512 | **abort** | **abort** | 0 | 0 |
| 常量乘法链 1024 | abort | abort | **abort** | **abort** |
| `v(:vin)` 乘法链 80 | 0 | 0 | – | – |
| `v(:vin)` 乘法链 96…127 | 0 | **abort** | – |（127 的 release run = 0） |
| `v(:vin)` 加法链 96 / 128 | 0 / 0 | 0 / **abort** | – | – |

结论与归因：

1. **check 期常量求值是本轮新增**（§1.5 `check.rs:176-209` 用 `eval_constant` 递归求值）。
   S1 的 check.rs（0e13ade9…）没有该函数，因此同文件 S1 `exit 0`、S2 abort。
   这是本轮引入的**新失败模式**：一个纯常量表达式可以让 `cdsl check` 直接 abort。
2. **run 期阈值下降**（126 → 80/96 之间，同一批 `ladder/*.cdsl` 文件直接对比），
   与 §1.3 重写的递归 `Evaluator`（`expr.rs:549` 起）逐层栈占用增大一致；
   根因归属为"求值器递归 + debug 大栈帧"，具体改动影响面未逐帧剖析。
3. release 阈值远高于 debug（512 通过、1024 abort），但**同样没有上限保护**。

严重级别：**P1**（合法且被 check 接受的输入使进程 abort、无诊断；debug 阈值已低到 96 个运算）。
与 `docs/review-evidence/round4/findings.md` FINDING-1 的关系：该文把本条标记为
"pre-existing / 递归深度未变"。我的取证**部分纠正**这一点——parse 阶段（FINDING A）确为既有、阈值未变；
但 check 期常量 abort 是本轮 §1.5 新引入，run 期阈值也在本轮从"126 可运行"退化为"96 abort"。
任何修法都必须保留 128 因子的 `E_DIMENSION` 静态拒绝（当前 `exit 1` 且无 abort，已由本报告 §2 确认）。
未在本报告给出修法建议，仅提供证据。

---

## 7. CLI 与 REPL 一致性（同表达式）

命令（管道输入）：

```
:load target/round4/reviewer/rev_sqrt_masked.cdsl --replace
:run sqrt_domain --out target/round4/reviewer/out3/repl
:quit
```

结果 `exit 1`（`logs/fin_repl.log`），REPL 文本与 CLI `run` 逐字同类：
`error[E_VALUE]: derive `root_of_negative`: sqrt of a negative sample in `sqrt(-1)` at sample 0 of analysis `op1`: ...`
并带同一组 `analysis/kind/signal/sample/index/expression` 上下文与标量说明。
**结论：PASS**（错误类别与文本一致；未比较数值路径的逐字输出）。

---

## 8. 我没有验证的范围

1. **全量门禁**：未运行 `cargo test --workspace`、`cargo clippy --workspace`、`cargo fmt`（按分工属 lead）；
   我只跑了 `cargo build -p circuit-cli`（debug/release）与一次 `cargo test -p circuit-core --lib`
   （早期快照：67 passed / 0 failed；随后 lead 编辑 `plan.rs` 期间该 crate 一度编译失败，属并发中间态，未作为结论）。
2. **phase B（参数 DAG）**：完全未审（本轮范围外）。
3. **QA worker 的测试文件**：未读取、未运行、未采纳其结论；本报告只用自己的输入与探针。
4. **§1.3 策略 3 的完整运行级验证**：未构造"原始信号本身即 NaN/Inf 后进入表达式求值"的 CLI 用例；
   复制的项目是：`check` 时是常量、运行期信号非有限来自非法表达式本身。
5. **measure 路径的 site 贯穿**：只有代码级证据（`measure.rs:144`），未构造运行期失败的 measure 用例。
6. **REPL 交互式路径**：只测管道（非 TTY）会话；未测历史、补全、取消输入。
7. **越界嵌套的精确阈值**：debug 只测到 80 通过 / 96 abort、release 512 通过 / 1024 abort 的区间；
   未二分到精确层数，也未测 release 下 127 因子以外的乘法链。
8. **写盘失败路径**（磁盘错误、`approve` 拒绝、目录不可写）：只读了代码与既有单测，未实测。
9. **非有限轴**（`Axis` 含 NaN/Inf 的导出警告）：`non_finite_diagnostics` 也覆盖轴，我只测了信号列。
10. **第 3 轮基线对照**：无法在不做 git checkout 的前提下恢复 pre-round-4 二进制，故 S1 只代表"本轮中途"状态；
    §6 的"新引入"结论建立在"check.rs 在 S1 没有 `eval_constant`"这一事实链上。
11. **性能/大文件**：未测导出体积、运行时间。

---

## 9. 证据文件清单（全部在仓库内，只读引用）

- 输入（自建）：`target/round4/reviewer/rev_*.cdsl`、`ladder/lad*.cdsl`、`iso/*.cdsl`、`abs/abs*.cdsl`、
  `deep/dabs*.cdsl`、`thr/mul*.cdsl`、`thr/add*.cdsl`、`thr/const*.cdsl`；哈希 `target/round4/reviewer/input-hashes.txt`
- 原始输出：`target/round4/reviewer/logs/*.log`（命名见正文）、`logs/export_probe_final.log`
- 快照哈希：`target/round4/reviewer/final-hashes-before.txt`、`final-hashes-after.txt`
- 独立探针：`target/round4/reviewer/probe/`（手工 Dataset → `write_datasets`）、
  `target/round4/reviewer/parseprobe/`（前端阶段定位）；两者都是独立 `[workspace]`，不进入主工作区构建
- 输出目录（用于验证"不落盘"）：`target/round4/reviewer/out2/`（整轮为空）、`out3/`、`probe-out/`

---

# 第二部分：阶段 A 冻结复核（task-8）

- 任务：task-8 *phase-A closure review (frozen revision)*，owner reviewer（只读）。
- 冻结哈希：lead 给出的 20 个文件（SHA-256 前 16 位）在**复核开始与结束各校验一次：20/20 一致**
  （`target/round4/reviewer/t8/hashes-start.txt`、`hashes-end.txt`）。
- 一条命令跑完整矩阵：`& target/round4/reviewer/reverify.ps1 -Tag reverify-t8`
  （先 `cargo build -p circuit-cli` 与 `cargo build -p circuit-cli --release`，均 exit 0），
  **58 个用例**，每例 exit code 与首行摘要见 `target/round4/reviewer/reverify-t8/logs/<case>.log`，
  整轮摘要 `target/round4/reviewer/t8/reverify-full.txt`；脚本自检哈希在运行期间不变
  （`hashes stable during the run: True`）。
- 总结论：**阶段 A 可以关闭**，理由见 §10.3–§10.6；不阻塞关闭的 3 项 P3 见 §10.7。

## 10.1 冻结哈希校验（20/20）

lead 给出的 20 个文件（units/plan/format/limits/eval/parser/expr/measure/execute/session/check/run/main +
expression_flow + 6 个 r4 测试文件）在复核开始与结束两次 `Get-FileHash -Algorithm SHA256` 全部命中：
复核开始时 20/20，结束时 20/20（脚本运行期间亦自检一致）。原始清单见
`target/round4/reviewer/t8/hashes-start.txt` 与 `hashes-end.txt`。

## 10.2 全矩阵实测（我的命令与观察，节选原文）

### R4-01 运行期表达式策略

| 用例 | 命令 | exit | 观察 |
|---|---|---|---|
| sqrt_check | cdsl check rev_sqrt_masked.cdsl | 1 | error[E_VALUE]: sqrt of a negative sample in sqrt(-1)（常量路径，带 derive/measure 名） |
| sqrt_run | cdsl run ... --experiment sqrt_domain | 1 | derive root_of_negative ... at sample 0 of analysis op1 + analysis/kind/signal/sample/index + 标量说明；无 measure 输出 |
| overflow_check / overflow_run | check / run rev_overflow_masked.cdsl | 1 / 1 | multiplication * produced a non-finite sample in (1e308 * 1e308): the sample is +inf |
| negsqrt_check / negsqrt_run | rev_negative_signal_sqrt.cdsl | 1 / 1 | E_DIMENSION: sqrt of a quantity in V is not representable（静态量纲先拒绝，无 panic） |
| negsqrt_dimless | run rev_negative_signal_sqrt2.cdsl | 1 | E_VALUE: ... (v(vm) / abs(v(vm))) is -1 there —— 无量纲构造，证明运行期 sqrt 域检查独立成立 |
| complex_check | check rev_complex_overflow.cdsl | 0 | 读信号 → 非常量，只做静态检查（符合 §1.5） |
| complex_run | run rev_complex_overflow.cdsl | 1 | the real part is NaN and the imaginary part is -inf，sample: frequency = 1000 |
| partial_run | run rev_partial_fail.cdsl | 1 | 失败诊断；`reverify-t8/o/partial` **不存在**（本次失败未写任何文件） |

### R4-02 量纲边界（debug 与 release）

| 用例 | exit | 观察 |
|---|---|---|
| dim128 check/run（debug） | 1 / 1 | E_DIMENSION ... V^127 * V ... at most 127 in magnitude，无 panic |
| dim128 check/run（release） | 1 / 1 | 同一诊断 |
| dim127 check（debug） | 0 | V^127 合法，无假阳性 |
| dim127 run（debug / release） | 0 / 0 | 现在可正常运行并写文件（修复前 debug abort） |
| param128 check | 1 | E_DIMENSION: * on a quantity in V^127 and one in V（crates/circuit-dsl/src/eval.rs 路径，span 在运算符） |
| dimbound neg130 check（本次新增） | 0 | V^(2-130) = V^-128 接受（合法下界） |
| dimbound neg131 check（本次新增） | 1 | E_DIMENSION: / on a quantity in V^-128 and one in V —— 除法/减法边界正确，无回绕 |

### 深度护栏（FINDING-1）

| 用例 | exit | 观察 |
|---|---|---|
| lad96 / lad112 / lad120 / lad124 / lad126 / lad127 run（debug） | 全部 0 | 修复前 96 以上即 abort，现均正常写文件 |
| lad126 / lad127 run（release） | 0 / 0 | 同上 |
| mul80 / mul96 / add96 / add128 run（debug） | 全部 0 | 运行期递归退化已消除 |
| const96 / const127 / const256 check（debug） | 0 | check 期常量求值不再 abort |
| const512 check（release）/ const1024 check（debug） | 1 / 1 | E_LIMIT: expression is 257 levels deep, which is deeper than the 256 level limit |
| add256 check / run（debug） | 1 / 1 | 同上（E_LIMIT，非 abort） |
| nest120 check / run（debug） | 0 / 0 | |
| nest200 check（debug / release）/ run（debug） | 0 / 0 / 0 | 200 层嵌套可解析、可运行 |
| nest250 / nest254 check（debug） | 0 / 0 | 上限内的最深处（见 §10.7 第 2 条） |
| nest255 / nest256 / nest257 / nest260 / nest300 check（debug） | 全部 1 | E_LIMIT: expression nests deeper than 256 levels; the parser stops here |
| nest300 paren check（debug）、nest300 check（release）、abs512 check（release） | 1 / 1 / 1 | 三种形态均 E_LIMIT，无 abort |
| abs112 check（debug）、devabs200 check（debug） | 0 / 0 | parse 阶段 abort 已消除（含器件 value: 位置） |

### CLI 脚手架（clap 未被改动破坏）

| 用例 | exit | 观察 |
|---|---|---|
| cdsl --help | 0 | usage 与子命令列表正常 |
| cdsl capabilities | 0 | backend thevenin 0.5.0、分析/器件清单正常 |
| cdsl（无参数） | 2 | clap 用法错误（exit 2，符合 CLI 契约） |

## 10.3 design-contract §3 验收矩阵逐行核对

| §3 行 | 我用的独立输入 | 观察 | 判定 |
|---|---|---|---|
| sqrt(-1)、负信号开方 → 结构化错误，无成功测量 | rev_sqrt_masked.cdsl、rev_negative_signal_sqrt{,2}.cdsl | check/run exit 1；无 measure 行；负信号在无量纲构造下也被运行期拒绝 | **PASS** |
| min(sqrt(-1),2)、嵌套 max → 非法中间值不可隐藏 | 同上（measure 为 max: min(sqrt(-1),2)） | 错误在 sqrt 处报出，measure 从未产出数字 | **PASS** |
| 1e308*1e308、嵌套 min/max → 溢出诊断 | rev_overflow_masked.cdsl | exit 1，E_VALUE +inf，measure 无输出 | **PASS** |
| 非有限复数分量 / 复数运算溢出 → 统一策略 | rev_complex_overflow.cdsl（ac: 1e200.V） | run exit 1，报实部 NaN / 虚部 -inf，同一 E_VALUE 类 | **PASS** |
| 128 因子与量纲减法边界 → check/run 诊断，无 panic、无回绕（debug+release） | rev_dim128.cdsl、rev_dim127.cdsl、dimbound/neg{130,131}.cdsl | 128 → E_DIMENSION（两档）；127 与 V^-128 接受；V^-129 → E_DIMENSION | **PASS** |
| 原始 Dataset 含 NaN/Inf 的 CSV/JSON 导出 → 约定空值 + 可见警告 | 自建 probe crate 手工 Dataset | Written{warnings: 2}（每 dataset 去重）；CSV 空字段、JSON null；JSON diagnostics: []（与改写后的 §1.4 一致）；CLI/REPL 打印警告 | **PASS** |
| 同一表达式 CLI 与 REPL → 数值、错误类别一致 | :load + :run sqrt_domain（管道） | REPL 报同一条 E_VALUE 文本（analysis/kind/signal/sample/index 相同），exit 1 | **PASS** |
| 先有效 derive 后非法 derive → 本次失败不产生成功输出文件 | rev_partial_fail.cdsl | exit 1；输出目录不存在（Test-Path False） | **PASS** |
| RC 增益、功率、多分析绑定、隐式探针、重采样 → 原有行为保持 | 6 个 crate 的全部测试 + 两个示例工程 | 逐 crate 测试合计 618 passed / 0 failed（§10.6）；示例数值独立复算一致 | **PASS** |

## 10.4 FINDING-1 关闭情况（三个快照对照，输入文件内容未变）

| 输入 | S1（§1.3/§1.5 前，本轮中途） | S2（T2–T5 后，修复前） | 冻结（T9+T10） |
|---|---|---|---|
| lad112/120/124/126 run（debug） | exit 0 | abort 0xC00000FD | **exit 0** |
| lad127 run（debug / release） | abort / – | abort / 0 | **0 / 0** |
| mul96 run（debug） | exit 0 | abort | **0** |
| add128 run（debug） | exit 0 | abort | **0** |
| const96 check（debug） | exit 0 | abort | **0** |
| const256 check（debug） | – | abort（同类） | **0** |
| const512 check（release） | – | abort | **exit 1 E_LIMIT** |
| nest112 check（debug） | abort（parse） | abort | **0** |
| nest300 check（debug / release） | abort | abort | **exit 1 E_LIMIT** |
| abs512 check（release） | abort | abort | **exit 1 E_LIMIT** |
| 128 因子 check/run（debug+release） | E_DIMENSION | E_DIMENSION | **E_DIMENSION（优先级未被护栏破坏）** |

即：本轮引入的两处退化（check 期常量 abort、run 期阈值 <96）都被消除，更深输入变成 E_LIMIT 诊断，
且 128 因子的 E_DIMENSION 优先级保持不变。

## 10.5 契约 §1.1–§1.5 在冻结哈希下的复审计

| 条款 | 判定 | 冻结 revision 的证据（file:line） |
|---|---|---|
| §1.1 checked 量纲 API | PASS | units.rs:49 MAX_EXPONENT；:61/:74/:90 checked_*；:277/:289 Quantity::checked_*；:297 Neg；无 pub fn mul/div/pow，无 impl Mul/Div |
| §1.1 调用点（plan/eval/format） | PASS | plan.rs:200-201；eval.rs:326-334 与 :347-365（E_DIMENSION、operator span）；format.rs 测试用 checked + expect |
| §1.2 静态量纲分析 | PASS | plan.rs:187 / :217 签名不变；:221 先报溢出；:315-341 exponent_overflow_error；elaborate.rs 经 lower_result_expr 消费 |
| §1.3 结果表达式求值 | PASS | expr.rs:390/:416-441 EvalKind/EvalSite；:466 eval；:476 eval_at；:513 is_constant；:540 eval_constant；:246 迭代式 depth；:494 check_depth；:808-809 checked 量纲 |
| §1.4 导出诊断（改写后） | PASS | execute.rs:1022-1044 write_datasets 文档与新语义一致；Written{warnings}、按 dataset 去重、先渲染后写；CLI run.rs 与 REPL session.rs 共用 warning_lines()；probe 实测 2 条警告 + JSON null |
| §1.5 check 期常量拒绝 | PASS | check.rs:190-201：from_ir → is_constant 跳过非常量 → eval_constant 失败即 exit 1，并补 derive/measure 上下文 |
| task-7 的 NEEDS_FIX（§1.4 措辞） | PASS（已修） | 契约 §1.4 第 148-157 行明确「两批互补」；实测 JSON diagnostics: [] 与 warnings 非空同时成立 |

## 10.6 对 lead 自报门禁的独立核对

1. **workspace 测试 618 passed / 0 failed** —— 角色约束只允许逐 crate 运行，我按
   `cargo test -p <crate>` 对 6 个 crate 各跑一次，把每个 test binary 的 test result 行相加：
   circuit-core 67、circuit-dsl 220、circuit-results 125、circuit-session 63、circuit-cli 85、
   circuit-backend 58 = **618 passed / 0 failed**（33 行 test result，所有命令 exit 0）。
   与自报数字一致。原始日志：`target/round4/reviewer/t8/tests-<crate>.log`。
2. **clippy -D warnings、cargo fmt --check、cargo test --workspace** —— **我未运行、未验证**：
   角色约束把 workspace 级门禁留给 lead，我只有 `-p <crate>` 权限；这三项仍属自报。
3. **数值回归（我独立复算）**：
   - examples/voltage_divider.cdsl（exit 0）：v(in)=5、v(out)=3、i(r1)=0.002、i(v1)=-0.002，
     与示例头注释 v(out)=3.000 V、i(r1)=+2.000 mA 一致。
   - examples/rc_filter.cdsl（exit 0）：121 个 AC 点、1019 个 tran 点；vfinal=0.9932620899316276 V、
     vavg=0.8013465976386364 V、vrms=0.8382662216736428 V；AC 网格上最接近解析截止频率
     1/(2*pi*1kohm*100nF)=1591.54943091895 Hz 的点为 f=1584.893192461108 Hz，
     实测 |v(vout)|=0.70858696835616，解析 1/sqrt(1+(f/fc)^2)=0.70858696835616（差 0），
     相位 -44.8799368167267 度亦与解析值逐位一致。
   - 原始输出：`target/round4/reviewer/t8/ex_divider.log`、`ex_rc.log`、`examples/*.csv`。

## 10.7 NEEDS_FIX（均为 P3，不阻塞阶段 A 关闭）

1. **crates/circuit-session/src/execute.rs:990-999 的 Written 结构体文档注释仍是旧表述**：
   称 warnings 是「the same non-finite-sample warnings that the JSON file already keeps in its own
   diagnostics array」。这与同一文件已被改写的 write_datasets 文档（:1028-1039）和契约 §1.4
   （:148-157）直接矛盾，也被 probe 实测反驳（JSON diagnostics 为空、warnings 非空）。
   裁决 A 要求「同步修正 execute.rs 的文档注释」，实际只改了函数注释、漏了结构体注释。
2. **嵌套护栏在恰好 256 层触发，消息多报一层**：parser.rs:1616 用 self.depth >= MAX_EXPR_DEPTH，
   而形态护栏 :1550 用 deepest <= MAX_EXPR_DEPTH。实测 nest254 check=0、nest255 check=1
   （消息「nests deeper than 256 levels」）；const256 check=0、const512 报「257 levels deep」。
   即 256 上限对平链与嵌套的计数不同，limits.rs:44 「every depth up to the limit is accepted there」
   对嵌套调用不成立。建议改用 > 或修正文案/文档（无任何安全性影响）。
3. **指数下界措辞**：units.rs:48-49 等处的诊断写「at most 127 in magnitude」，但 -128 是合法下界
   （实测 V^-128 check=0、V^-129 exit 1）。建议写 -128..=127。
4. 观察（未列为 NEEDS_FIX）：expr::is_constant（:513）与 expr::from_ir 是公开 API、递归实现且无深度护栏；
   CLI 路径安全，因为 parser 先把 AST 限深（check.rs:190-194 的顺序因此无害）。若未来有嵌入方直接
   用手工 Expr 调用 is_constant，建议同样先做 depth() 检查。

## 10.8 我在 task-8 中未验证的范围

1. workspace 级门禁：cargo test --workspace、cargo clippy --workspace --all-targets -- -D warnings、
   cargo fmt --all -- --check（角色约束禁止执行，属 lead 自报）。
2. 6 个新 r4_*.rs 测试文件的逐行审阅、断言强度与是否存在弱断言（我只运行了它们）。
3. phase B（参数 DAG）的任何内容。
4. 非 TTY 之外的 REPL 行为（历史、补全、取消输入）与交互式 TTY 路径。
5. 性能/大文件、导出体积；非有限 Axis 的导出警告（只测了信号列）。
6. release 下更深的精确阈值（只测得 const512 / abs512 → E_LIMIT，未测极端值）。
7. 小栈 embedder 场景（findings.md 自述「2 MiB 线程上 300 层仍 abort」）——我未在自建线程上复现。
8. target/round4/lead-gate/ 与 lead-verify.ps1 的日志我未逐条复核。

---

# 第三部分：阶段 B 冻结复核（task-14）

- 任务：task-14 *phase-B closure review (frozen revision)*，owner reviewer（只读）。
- 冻结哈希：lead 给出的 15 个文件（SHA-256 前 16 位）在**复核开始与结束各校验一次：15/15 一致**
  （`target/round4/reviewer/t14/hashes-start.txt`、`hashes-end.txt`）。
- 输入全部自建（`target/round4/reviewer/phaseB/p01..p21*.cdsl`、`t14/depth/*.cdsl`、
  `phaseB/cf255..257.cdsl`），未复用 QA/DAG worker 用例；原始输出在
  `target/round4/reviewer/t14/logs/`。
- **总结论：阶段 B PASS，可以关闭。** 契约 §4.1–§4.7 与计划 §5.3 的验收项逐条通过（§11.2/§11.3）；
  阶段 A 的 58 用例矩阵在本修订版上重跑无回归（§11.4）；lead 自报的 660/0 我用逐 crate 运行复现一致（§11.5）。
  仅有 3 项 P3 级文档/证据问题（§11.6），不阻塞关闭。

## 11.1 冻结哈希校验（15/15）

复核开始与结束两次 `Get-FileHash -Algorithm SHA256`：param_graph.rs 9FA7C804B84AC037、
elaborate.rs 575471D091505B3E、lib.rs B40967F8EA2780B2、parser.rs 75D0F2B5B40DF06F、
limits.rs 5E54BDB24F373F8B、units.rs 3F9E3592BA3B6B78、plan.rs BB368AC948F67431、
main.rs 47F2B6DC95814D0E、execute.rs 8D51FAB47B7CFBA9、expr.rs 35FDCDD586C92563、
r4b_regressions.rs 2FAF6F0C4E3608FD、r4b_topo_sweep.rs BD1C6E32C345E237、
r4b_param_graph.rs BEBB63BF2776EF7A、r4b_session_dag.rs A163338EAD8D066D、
elaborate.rs(测试) 3EA06B51E9ECC6BE —— 全部命中，复核期间未变。

## 11.2 计划 §5.3 验收矩阵（我的独立输入与手算）

| # | 验收项 | 我的输入 | 观察（原始输出摘录） | 判定 |
|---|---|---|---|---|
| 1 | 前向引用正确 | p01_forward.cdsl：b=2*a 声明在 a 之前 | check 0；run 0；measure vo = 1.3333333333333333 V = 4*1/(1+2) | PASS |
| 2 | 多层链正确 | p02_chain.cdsl：c=b+a, b=2*a, a=1k | vo = 1 V = 4*1/(1+3) | PASS |
| 3 | 重复运行顺序确定 | p02 两次 run 到不同目录 | CSV 与 JSON 逐行 Compare-Object 无差异 | PASS |
| 4 | 菱形图正确 | p03_diamond.cdsl：d=b+c, b=2a, c=3a | vo = 0.6666666666666666 V = 4*1/(1+5) | PASS |
| 5 | 未知名称诊断 | p04_unknown.cdsl（default: nope * 1.kohm） | exit 1，`error[E_NAME]: nope is not declared`，span 3:22 指向引用，note 与原规则一致 | PASS |
| 6 | 自环诊断 | p05_self_cycle.cdsl（param :a, default: a） | exit 1，`E_PARAM_CYCLE: parameter a depends on its own definition: a -> a`；primary 3:22（闭合引用）+ secondary 3:9（声明） | PASS |
| 7 | 多节点环诊断 | p06_cycle3.cdsl（a=b, b=c, c=a） | exit 1，`these parameters depend on each other: a -> b -> c -> a`；primary 5:22 + 三个 secondary 3:9/4:9/5:9，每个参与声明各一条 | PASS |
| 8 | 嵌套同名参数互不污染 | p07_scopes.cdsl：top.r=2k、实例 u1 的 r=3k（字面量绑定） | op7 vo = 2.4 V = 4*3/(2+3)（实例用 3k）；sweep7（扫 top.r，只达数值位置）check+run 0 | PASS |
| 9 | 实例 params: 跨作用域通道 | p08_inst_params_edge.cdsl：params: { mode: k }，子电路内 if mode > 0 | check 1，`E_TOPO_PARAM: parameter k reaches a topology use point`，`= path: k -> mode -> if condition` | PASS |
| 10 | 传递路径跨作用域 | p18_transitive_edge.cdsl：k -> mid=k+1 -> mode -> if | exit 1，`= path: k -> mid -> mode -> if condition` | PASS |
| 11 | 覆盖后依赖重算 | p13_override_recompute.cdsl：twice=2*base；实验覆盖 base=2k | defaults vo = 4 V；override vo = 4.8 V（twice 随之变为 4k） | PASS |
| 12 | REPL 事务性 | 管道会话：:load p13；:run defaults；:run defaults base=2.kohm；:run nosuch；:run defaults | 4 V → 4.8 V → E_NAME（无状态变更）→ 再次 4 V，与失败前逐字一致；失败 :load（E_TOPO_PARAM）后 :run defaults 仍为 4 V | PASS |
| 13 | 直接拓扑扫描拒绝 | p09_topo_direct.cdsl（if m > 0） | exit 1，`= path: m -> if condition`，span 12:13（dc param）、3:9（声明）、6:6（use point） | PASS |
| 14 | 计算名/端子是拓扑点 | p16（for k in 1..count）、p20（("m" + k) 作端子） | 均 exit 1；p20 报 `topology use point: computed terminal name`，span 6:28 | PASS |
| 15 | 无数值影响的扫描仍合法 | p10_value_sweep.cdsl；p11_indirect_numeric.cdsl（k→mode→(mode+1)*1k） | check+run 0；p11 三点 v(out) = 2 / 2.6666666666666665 / 3，与 4*R/(1k+R)（R=1k,2k,3k）逐点吻合 | PASS |
| 16 | 重复 params: 键与实验覆盖 | p12_dup_params.cdsl（params: { r: 2k, r: 3k }；两次 param :rs） | check 0（无诊断）；run 打印 `overrides: rs=2000, rs=1000`，vo = 3 V = 4*3/(1+3)（last wins） | PASS |
| 17 | 覆盖只查存在性、量纲在消费点 | p14_override_dim.cdsl（param :r, value: 1.V） | exit 1，`E_DIMENSION: rl.value needs ohm, found V` | PASS |
| 18 | 实例覆盖压掉默认（有效定义优先） | p19_override_hides_cycle.cdsl：子电路 a=b,b=a 但实例同时覆盖两者 | check 0、run 0，vo = 2.6666666666666665 V = 4*2/(1+2)（默认的环未被求值） | PASS |
| 19 | 分析规模位置不读参数 | p21_ac_points_param.cdsl（ac points: n） | exit 1，`E_NAME: n is not declared`（与 §4.5 记录一致，未扩展） | PASS |
| 20 | 运行期 C7 防线仍在 | 代码 + 测试 | `circuit-backend/src/sweep.rs:209-249` 仍做逐点拓扑比较；单测 `changing_only_a_value_keeps_the_topology`(:399)、`adding_a_device_is_a_topology_change`(:416)、`rewiring_is_a_topology_change`(:435)、`changing_a_device_kind_is_a_topology_change`(:453)；circuit-backend 58 tests 全绿 | PASS |
| 21 | 既有示例与扫描不回归 | examples/parameter_sweep.cdsl、examples/rc_filter.cdsl | check 均 0；sweep 8 点 v(out) = 3*1.5/(r+1.5) 逐点吻合（2.25/1.8/1.5/1.2857142857142858/…） | PASS |
| 22 | release 档 | p01/p13/p08/p18/p12 | 1.3333333333333333 / 4.8 / E_TOPO_PARAM / E_TOPO_PARAM / 3 V，与 debug 一致 | PASS |

## 11.3 lead 指定重点项的独立结论

**(1) 环诊断的闭合路径与逐声明 span —— PASS。** p05 与 p06 的原始输出给出闭合路径
（`a -> a`、`a -> b -> c -> a`），primary span 落在**闭合那条引用**上（p06 为 5:22，
即 `param :c, default: a` 里的 `a`），每个参与声明各有一条 secondary
（`a takes part in the cycle` 等）。契约 §4.4 的三项要求（闭合路径、闭合处 primary、逐声明 secondary）都成立。

**(2) E_TOPO_PARAM 三处 span —— PASS。** p08 给出三段 span：`dc param: :k` 的 `:k`（20:13）、
`param :k` 声明（12:9，`k is the swept parameter`）、子电路 `param :mode` 声明（3:9，
`mode follows from the swept parameter`）、`if mode > 0` 条件（4:6，
`topology use point: if condition`），并渲染 `= path: k -> mode -> if condition`。
p18 的传递路径为 `k -> mid -> mode -> if condition`，四步各有 span。**span 确实落在声明/使用点，
不是只写了消息文本。**

**(3) 实例 `params:` 作为跨作用域通道：我按"读法 A"，并认为可接受。**
- 读法 A（我采用，也是契约 §4.1/§4.5 的字面读法）：`params: { x: <读 p 的表达式> }` 建立
  `父作用域 p -> 实例作用域 x` 的**值边**；当实例内的 `x` 传递到达拓扑使用点时，
  扫描 `p` 必须被 check 拒绝。实现即如此：`param_graph.rs:26-32` 明确
  "The one cross-scope channel is an instance `params:` binding … gives the instance-local `x`
  the parent's `p` as a dependency"，并有 `bind_from_parent`。
- 读法 B（把拓扑闭包停在作用域边界）会让 p08 通过 check，而该扫描在 mode 跨过 0 时确实会改变子电路
  拓扑——只能靠 `sweep.rs` 的逐点比较在运行期失败。计划 §5.3 明确要求"直接和间接拓扑扫描
  都在 check 被拒绝"，所以读法 B 不可接受。
- 实现的两点边界也符合契约，且我用输入证明：**字面量绑定不产生边**（p07 的 `params: { r: 3.kohm }`，
  扫 top.r 合法）；**绑定是数值链时不误报**（p11，k→mode→`value:` 合法）。

**(4) 嵌套同名参数 —— PASS**（p07，见 §11.2 第 8 行）。
**(5) 覆盖后联动重算 —— PASS**（p13 + REPL，见第 11/12 行）。
**(6) REPL/会话事务性 —— PASS**（第 12 行；失败运行与失败 `:load` 都不污染后续成功运行）。
**(7) 普通数值扫描仍可用 —— PASS**（第 15/21 行）。
**(8) 运行期 C7 防线仍在 —— PASS**（第 20 行）。
**(9) 阶段 A 三个 P3 复验 —— PASS：**
- (a) `crates/circuit-session/src/execute.rs:990-999` 的 `Written` 结构体注释已改为
  "renderer's diagnostics … not a dataset's own `diagnostics`（the provenance from the plan and the backend…）"，
  与改写后的契约 §1.4 一致；探针实测 JSON diagnostics 为空数组 + warnings 非空仍成立。
- (b) 嵌套护栏由 `>=` 改为 `>`（`parser.rs:1616`）。实测：nest254 check=0；
  nest255 报形态护栏 `expression is 257 levels deep…`；nest256/257 报嵌套护栏
  `expression nests deeper than 256 levels`。**我另用自建 probe（`parseprobe/src/bin/astdepth.rs`，
  与护栏同一遍历、显式栈 + 64 MiB 工作栈）独立测量 AST 深度**：nest254 = 256 层（= 上限，接受）、
  nest255 = 257 层（消息数字准确，因为 `v(:vin)` 本身是 Call + 符号两层）、
  平链 256 个字面量接受、257 个拒绝、255 个 abs 包一个数（= 256 层）接受。
  即：**没有低于文档上限的输入被拒，消息数字与实际树深一致。**
- (c) `units.rs:48-54` 新增 `MIN_EXPONENT = i8::MIN`；`plan.rs:332` 渲染
  `(an exponent is held as a signed 8-bit integer, -128..=127)`，`eval.rs:357` 渲染