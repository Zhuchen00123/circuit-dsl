# R1 内核契约取证：Thevenin 0.5.0 瞬态求解内核与用户参数的交互（task-1）

- 任务：`r1-kernel-recon` / 共享任务 `task-1`
- 模式：**只读取证**。本轮未修改任何产品代码/Cargo.toml/Cargo.lock，未运行任何测试，未 commit。
- 时间锚：本报告所有 workspace 侧引用均为「读取时刻」快照；内核侧（registry 源码）行号由下方 SHA256 锚定。

## 0. 审查对象与版本锚定

|crate|版本|Cargo.lock checksum|
|---|---|---|
|thevenin|0.5.0|`90e5ea0cb6ecb5f6f469175bf4aecf369546102830223d1dbeec7118148535ad`|
|cirq-ir|0.5.0|`07a56f0d72194574bb00e815f32ed7b73eac35cd137a42c8a10afe288b099146`|
|thevenin-types|0.5.0|`33553d02a4aae8317673ea4fe8e4115d67619ca94be4015e0daf532395959915`|

来源：`Cargo.lock`（`name = "thevenin"` / `version = "0.5.0"` / `source = "registry+https://github.com/rust-lang/crates.io-index"`）。
依赖声明：`Cargo.toml:41-43`（`cirq-ir = "0.5.0"` / `thevenin = "0.5.0"` / `thevenin-types = "0.5.0"`），
`crates/circuit-backend/Cargo.toml` 用 `thevenin.workspace = true`。
只读校验命令：`cargo tree --locked --offline -p circuit-backend --depth 1` → 输出 `thevenin v0.5.0`、`cirq-ir v0.5.0`，**EXIT=0**。

上游出处（`thevenin-0.5.0/.cargo_vcs_info.json`）：
`{"git":{"sha1":"cfcccc9846624aa0f32bbebbe2aaa7791575f054"},"path_in_vcs":"thevenin"}`，
`Cargo.toml` 中 `repository = "https://github.com/cramt/thevenin"`。

**注释**：vendored 目录（`C:\Users\15185\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\thevenin-0.5.0\`）只有 `.cargo-ok` 与 `.cargo_vcs_info.json`，**没有 `.cargo-checksum.json`**，因此无法用 Cargo.lock 的 checksum 复核本地文件完整性。改用事前 SHA256 锚定本轮所引文件：

```
D85171C4ED11D49FC0D0D8A90104B032E4C3EBB1024F67F33BE4DD6C83532D5F  thevenin-0.5.0/src/transient.rs
3FA87E3DBB90467868A5AA60091989C9EFC2752CF1E2D3F30FB353F11F8EDA6F  thevenin-0.5.0/src/waveform.rs
8EB05D22F9485F1A20B1F6A62F245753B5B9FF31916D4108390A3192AA50E6D7  thevenin-0.5.0/src/newton.rs
6390471F813053521FD7446F0BBE3E49E0D9799E6116ED34C67021A61257FF32  thevenin-0.5.0/src/mna_ir.rs
AEF98B272FC8A57C0E4984AA3FB9E79B8507BAEFD86C3F536F73D6441928360D  cirq-ir-0.5.0/src/lib.rs
```

以下若写 `transient.rs:NNN` 均指 `thevenin-0.5.0/src/transient.rs`，`waveform.rs`/`newton.rs`/`mna_ir.rs` 同理，`cirq-ir/lib.rs` 指 `cirq-ir-0.5.0/src/lib.rs`。

调用链（Circuit 路径，本仓库走的路径）：
`thevenin::circuit::simulate_tran`（`thevenin-0.5.0/src/circuit.rs:134-150`：先跑 OP 再 `run_tran`）
→ `mna_ir::tran_params_from_circuit`（`mna_ir.rs:581-640`）
→ `transient::run_tran`（`transient.rs:775`）。

---

## 1. `h_max` / `h_min` / `h_print` 的确切公式与来源

### 1.1 入参与唯一守卫（`transient.rs:776-799`）

```rust
776:    let TranRunParams {
777:        t_step: h_print,
...
792:    if h_print <= 0.0 || t_stop <= 0.0 {
793:        return Err(MnaError::UnsupportedElement(
794:            "invalid .tran parameters".to_string(),
795:        ));
796:    }
797:
798:    // Maximum internal timestep: tmax if specified, otherwise min(tstep, tstop/50).
799:    let h_max = t_max.unwrap_or_else(|| h_print.min(t_stop / 50.0));
```

- `h_print` 就是 `TranRunParams.t_step`（`:663` 定义；实际值来自 `TranAnalysis.step`，见 §6）。
- `h_max = t_max.unwrap_or(min(h_print, t_stop/50))`。**`tmax` 存在时 `tstep` 完全不进入 `h_max`**；`tmax` 缺失时 `tstep` 与 `tstop/50` 取小者。
- 内核**没有**对 `t_max` 做 `> 0` 校验（`t_max` 在 `transient.rs` 只出现 3 次：`:780` 解构、`:799` 使用、`:2308` 写快照）。

来源结构（`transient.rs:660-701`）：
```rust
662: pub struct TranRunParams {
663:     pub t_step: f64,
664:     pub t_stop: f64,
665:     pub t_start: f64,
666:     pub t_max: Option<f64>,
667:     pub uic: bool,
...
672:     pub nr_opts: crate::newton::NrOptions,
```

### 1.2 `h_min` 与初始步长（`transient.rs:1403-1409`）

```rust
1403:    // Internal timestep — start small and let doubling grow it to h_max.
1404:    // ngspice starts at h_max/400 and doubles each step (matching its adaptive
1405:    // initial-step algorithm).  For reactive circuits the LTE will control growth;
1406:    // for purely resistive circuits the step doubles freely until reaching h_max.
1407:    let h_min = h_print * 1e-9; // Absolute minimum timestep.
1408:    let mut h = (h_max / 400.0).max(h_min);
1409:    let mut h_prev = h; // No previous step yet; use h as initial estimate.
```

- `h_min = h_print * 1e-9`：**唯一**来源就是 `tstep`，与 `tmax` 无关。
- 初值 `h = max(h_max/400, h_min)`；`:1404-1406` 的注释独立确认「h_max/400 起、每步最多 ×2」，常数在 `:366-369`：`MIN_SHRINK = 0.125`、`MAX_GROW = 2.0`。
- `h_min` 同时是循环下界与重试阈值（见 §7.5），并未被 `min(h_max, …)` 夹住，因此 `h_min > h_max` 是可能的（`tstep` 过大 + `tmax` 极小）。

### 1.3 `h_print` 在整条步长链中的**全部**出现位置（9 行，其中代码 8 行）

|行|角色|代码|
|---|---|---|
|`777`|解构重命名|`t_step: h_print,`|
|`792`|守卫|`if h_print <= 0.0 \|\| t_stop <= 0.0`|
|`799`|`h_max` 回退|`t_max.unwrap_or_else(\|\| h_print.min(t_stop / 50.0))`|
|`1328`|写入波形参数|`let tran_params = TranParams { tstep: h_print, tstop: t_stop };`|
|`1407`|`h_min`|`let h_min = h_print * 1e-9;`|
|`1408`|初值（经 `h_min`）|`let mut h = (h_max / 400.0).max(h_min);`|
|`1690`|（注释）|`// but cap at h_print so output points stay dense enough for`|
|`1694`|**唯一的步长硬上限**|`h = (step_h * MAX_GROW).min(h_max).min(h_print).max(h_min);`|
|`2307`|pause 快照|`t_step: h_print,`|

> 与 round-1 文档 `docs/review-evidence/backend-contract.md:99` 的差异：该处写「grep `h_print` 全部 7 处：777/792/799/1328/1407/1694/2307」，实际含 `:1408`（经 `h_min` 影响初始步长）与 `:1690`（注释）共 9 行。行为结论不变，计数需更正。

### 1.4 `:1694` 的 `.min(h_print)` 属于哪个条件分支

循环内步长决策是 4 段链（`transient.rs:1620-1696`），`.min(h_print)` 位于最后的 `else`：

```rust
1620:        if method == IntegrationMethod::Trapezoidal && (has_reactive || has_device_charges) {
1654:            h = new_h.min(step_h * MAX_GROW).min(h_max).max(h_min);
1655:            force_be = false;
1656:        } else if method == IntegrationMethod::BackwardEuler && (has_reactive || has_device_charges)
1657:        {
...
1681:            h = trap_h.min(step_h * MAX_GROW).min(h_max).max(h_min);
1682:        } else if (at_breakpoint || force_be) && (has_ltra || has_txl || has_cpl) {
1686:            h = (step_h * MAX_GROW).min(h_max).max(h_min);
1687:            force_be = false;
1688:        } else {
1689:            // No LTE control and not at a breakpoint — grow toward h_max,
1690:            // but cap at h_print so output points stay dense enough for
1691:            // waveform fidelity.  Without growth here, circuits with only
1692:            // transmission lines (no caps/inductors/BJTs) stay stuck at
1693:            // the initial tiny h and never make progress.
1694:            h = (step_h * MAX_GROW).min(h_max).min(h_print).max(h_min);
1695:            force_be = false;
1696:        }
```

即 `else` 被到达的充要条件是：**LTE 不可用**（`!(method==Trap && (has_reactive||has_device_charges))` 且 `!(method==BE && (has_reactive||has_device_charges))`）**且不落在传输线 BE 特例**（`!( (at_breakpoint||force_be) && (has_ltra||has_txl||has_cpl) )`）。`has_reactive`/`has_device_charges` 定义：

```rust
1324:    let has_nonlinear = mna.has_nonlinear();
1325:    let has_reactive = !mna.capacitors.is_empty() || !mna.inductors.is_empty();
1618:        let has_device_charges =
1619:            !mna.bjts.is_empty() || !mna.vbics.is_empty() || !mna.bsim3soi_dds.is_empty();
```

**结论**：只有「既无 C/L，也无 BJT/VBIC/BSIM3SOI 电荷」的电路（纯电阻/源、外加非 LTE 器件）才会走 `else`；在该分支里 `tstep`（经 `h_print`）是**除 `h_max` 之外的第二个硬步长上限**。这类电路里 `tstep` 极小 ⇒ 步数爆炸，**即使显式传了 `tmax`**（`min` 里 `h_print` 更小者生效）。有 LTE 的电路（本仓库 RC 就是）不会到这一行。

---

## 2. `tstep`（`TranParams.tstep`）的**全部**使用点清单

`TranParams` 定义（`waveform.rs:10-17`）：
```rust
12: pub struct TranParams {
13:     /// The timestep from .tran (used as default for rise/fall times).
14:     pub tstep: f64,
15:     /// The final simulation time (used as default for SIN freq, SFFM freqs).
16:     pub tstop: f64,
17: }
```
Circuit 路径的构造点：`transient.rs:1327-1330`（`tstep: h_print`）；DC/OP 路径另有一个硬编码副本 `mna_ir.rs:391-399`（`TranParams { tstep: 1e-9, tstop: 1.0 }`，只用于 t=0 处波形求值，**不是** `TranAnalysis.step`）。

### 2.1 引擎侧（步长/断点）

|位置|性质|代码|
|---|---|---|
|`transient.rs:308`|断点去重窗口（**硬下限来源**）|`let min_break = tran.tstep * 5e-5;`|
|`transient.rs:799`|`h_max` 回退（仅 `tmax=None`）|`t_max.unwrap_or_else(\|\| h_print.min(t_stop / 50.0))`|
|`transient.rs:1407`|`h_min`|`let h_min = h_print * 1e-9;`|
|`transient.rs:1408`|初始步长（经 `h_min`）|`(h_max / 400.0).max(h_min)`|
|`transient.rs:1694`|无 LTE 分支的步长上限|`.min(h_max).min(h_print)`|

### 2.2 波形求值（`waveform.rs`）——"默认值"与"硬 clamp"必须分开看

|波形|行|代码|性质|
|---|---|---|---|
|PULSE `tr`|37|`tr.unwrap_or(tran.tstep).max(tran.tstep)`|**默认值 + 硬 clamp（双侧）**|
|PULSE `tf`|38|`tf.unwrap_or(tran.tstep).max(tran.tstep)`|**默认值 + 硬 clamp（双侧）**|
|PULSE `pw`|39|`pw.unwrap_or(tran.tstop).max(0.0)`|默认 `tstop`，**与 tstep 无关**|
|PULSE `per`|42, 135|`per.unwrap_or(tstop).max(tr + pw + tf).max(tstep)`|默认 `tstop`，**`tstep` 是硬下限**|
|SIN `freq`|55-59|`freq.unwrap_or(if tstop>0 {1.0/tstop} else {1.0})`|默认来自 `tstop`，**无 tstep**|
|SIN `td/theta/phi`|60-62|`unwrap_or(0.0)`|无 tstep|
|EXP `td1`|75|`td1.unwrap_or(tran.tstep)`|默认 = `tstep`（无 clamp）|
|EXP `tau1`|76|`tau1.unwrap_or(tran.tstep).max(tran.tstep)`|**默认值 + 硬 clamp**|
|EXP `td2`|77|`td2.unwrap_or(td1.unwrap_or(tran.tstep) + tran.tstep)`|默认依赖 `tstep`|
|EXP `tau2`|78|`tau2.unwrap_or(tran.tstep).max(tran.tstep)`|**默认值 + 硬 clamp**|
|SFFM|83-95|`fc.unwrap_or(5/tstop)`、`fs.unwrap_or(500/tstop)`、`md.clamp(0.0, fc/fs)`|**无 tstep**|
|AM|98|`eval_am(*va,*vo,*fc,*fs, td.unwrap_or(0.0), t)`|**无 tstep，`fc/fs` 必填无默认**|
|PWL|81|`eval_pwl(points, t)`|**无 tstep**|

断点收集侧是同一套公式的复刻（**展宽同时改变断点位置**）：
```rust
271:            let tr_val = tr.unwrap_or(tran.tstep).max(tran.tstep);
272:            let tf_val = tf.unwrap_or(tran.tstep).max(tran.tstep);
273:            let pw_val = pw.unwrap_or(tran.tstop).max(0.0);
275:            let period = per
276:                .unwrap_or(tran.tstop)
277:                .max(tr_val + pw_val + tf_val)
278:                .max(tran.tstep);
...
311:            let td1_val = td1.unwrap_or(tran.tstep);
312:            let td2_val = td2.unwrap_or(td1_val + tran.tstep);
```

### 2.3 与 `tstep` 同名但不同路径（避免误读）

`expr.rs:1229-1243` 的 `tstep` 是 `thevenin_types::Analysis::Tran` 的**表达式字段**（`.tran` 卡解析路径，`thevenin-types-0.5.0/src/lib.rs:994-999`），用于 `try_resolve_expr(tstep, ctx)`（`:1236`），与 `TranParams.tstep` 无关。本仓库走 cirq-ir Circuit 路径，不经过它。

### 2.4 `min_break`（`transient.rs:286-308`）

```rust
286:    min_break: f64,
...
308:        let min_break = tran.tstep * 5e-5;
...
313:            min_break,
```
`tstep` 越大，断点去重/"已越过"窗口越大：当 `5e-5·tstep ≥ 断点间距` 时，`next_after` 会把后续断点一并吞掉（见 §3）。

---

## 3. 断点（breakpoint）机制：检测、BE 回退、重启步长

### 3.1 断点表的来源与检测

```rust
289: impl BreakpointTable {
291:    fn from_mna(mna: &MnaSystem, tran: &TranParams) -> Self {
292:        let mut times = Vec::new();
294:        for vs in &mna.voltage_sources {
295:            if let Some(ref wf) = vs.waveform {
296:                times.extend(waveform::breakpoints(wf, tran));
...
299:        for cs in &mna.current_sources {
...
305:        times.sort_by(|a, b| a.total_cmp(b));
306:        times.dedup_by(|a, b| (*a - *b).abs() < 1e-15);
308:        let min_break = tran.tstep * 5e-5;
```

`waveform::breakpoints`（`waveform.rs:257-343`）逐波形给出断点时间：
- PULSE：每周期 4 个边 `[0, tr, tr+pw, tr+pw+tf]`（`:282`），按 `td + k*period` 展开（`:284-301`，`k > 1_000_000` 安全上限）；
- PWL：每个点（`:303-309`）；EXP：`td1`、`td2`（`:310-319`）；SIN/AM：`td`（仅 `>0`，`:320-331`）；
- **SFFM 无断点**（`:332-334`）；未知变体 `_ => {}`（`:335-337`）。

判定：
```rust
318:    fn next_after(&mut self, current_time: f64) -> Option<f64> {
320:        while self.next_idx < self.times.len()
321:            && self.times[self.next_idx] <= current_time + self.min_break
323:            self.next_idx += 1;
...
350:    fn is_at_breakpoint(&self, t: f64) -> bool {
351:        if self.next_idx < self.times.len() {
352:            let bp = self.times[self.next_idx];
353:            (bp - t).abs() < self.min_break
```

循环内（**顺序很关键**）：
```rust
1432:        // Breakpoint handling: don't cross the next breakpoint.
1433:        let at_breakpoint = breakpoints.is_at_breakpoint(t);
1434:        if let Some(bp) = breakpoints.next_after(t) {
1435:            let dist = bp - t;
1436:            if step_h > dist {
1437:                step_h = dist;
1438:            }
1439:        }
1441:        // At breakpoints, reduce step for stability (ngspice uses 0.1×).
1442:        if at_breakpoint {
1443:            step_h = step_h.min(h * 0.1).max(h_min);
1444:        }
```

### 3.2 "断点后第一个接受步是否被强制回退为 Backward-Euler"

**准确表述**：不是"断点之后的第一步"，而是**起点落在断点上（或断点前 `min_break` 窗口内）的那一步**被判为 `at_breakpoint`，从而强制 BE：

```rust
1459:        let gear_bootstrap_only = prefer_method == IntegrationMethod::Gear
1460:            && gear_bootstrap_steps_remaining > 0
1461:            && !is_first_step
1462:            && !at_breakpoint
1463:            && !force_be;
1464:        let method = if is_first_step || at_breakpoint || force_be || gear_bootstrap_only {
1465:            IntegrationMethod::BackwardEuler
1466:        } else {
1467:            prefer_method
1468:        };
```

`at_breakpoint` 用的是**本步起点 `t`**（`:1433`），且 `:1433` 早于 `:1434` 的 `next_after` 推进 `next_idx`。因此：
- 到达断点的那一步：只有当它的起点已在 `min_break` 窗口内时才是 BE；否则它是普通步，但会被 `:1436-1438` 夹到恰好落在断点上（`step_h = bp - t`，IEEE 下 `t += (bp - t)` 精确回到 `bp`）。
- 从断点出发的第一步：`|bp - t| = 0 < min_break` ⇒ `at_breakpoint = true` ⇒ **BE**，同时步长被压到 `min(h*0.1, dist)`。
- 窗口内消费的边角：若起点已在窗口内（`t ∈ (bp-min_break, bp)`），`:1434` 的 `next_after` 会先把该断点消费掉，于是 `:1435-1438` 的 clamp 指向的是**下一个**断点，本步可能跨过 `bp`（跨幅 ≤ 到下一断点的距离）。这是"不精确落点 + 不触发 BE"的罕见路径，`min_break` 越小越可忽略。

**实证佐证（二次引用 Lead 的 round2 repro，非本轮独立重跑）**：`target/round2-evidence/repro/out-coarse/coarse.tran1.csv` 前 11 个时间点为
`0, 2.5e-13, 7.5e-13, 1.75e-12, 3.75e-12, 7.75e-12, 1.575e-11, 3.175e-11, 6.375e-11, 1.2775e-10` ⇒ 首步 `2.5e-13 = (h_max/400)·0.1`（`h_max = tmax = 1ns`，PULSE `delay:0.s` 产生 t=0 断点 → BE + 0.1×），随后每步步长 ×2（`MAX_GROW=2.0`）直到 `dt_max = 1e-9 = tmax`。与 `:1408`、`:1443`、`:1681`、`:369` 完全一致。

### 3.3 重启步长 `h1` 从哪来？有没有 `tmax/10` 之类上限？

- **本内核没有 `h1` 变量**：`grep '\bh1\b'` 在 `transient.rs` 只有 `:2235` 的 LTRA 卷积累加注释（无关）。
- 重启步长就是 `:1443`：`step_h.min(h * 0.1).max(h_min)`，其中 `h` 是上一轮步长决策给出的建议值（`h ≤ h_max`，且已被 `:1681/:1686/:1694` 夹过），`step_h` 已被 `t_stop` 与断点距离夹过。
- **没有** `tmax/10`、`h_max/10` 之类的独立重启上限（`grep 't_max /' | 'h_max /'` 只命中 `:1404`/`:1408` 的 `h_max/400`）。0.1 是"现场 `h` 的 0.1 倍"，不是 `tmax` 的 0.1 倍。
- 断点后的增长从被压缩后的 `step_h` 重新起算（`:1681` 用 `step_h * MAX_GROW`，不是用 `h`），所以 0.1× 会真实体现在后续爬升上（实证见 §3.2 的 ×2 序列）。

### 3.4 其他强制 BE 路径（与断点并列）

- 第一步：`is_first_step`（`:1464`，初值 `:1410`）。
- NR 失败：`:1591-1610`（`force_be = true; h = (step_h * MIN_SHRINK).max(h_min); continue;`）。
- 阶数升级检查：`:1678` `force_be = trap_h <= 1.05 * step_h;`（ngspice dctran 风格）。
- Gear 自举：`:1459-1463`（前 2 步 BE，`:1415` `gear_bootstrap_steps_remaining: u32 = 2`）。

---

## 4. 输出时间轴录制：是否抽样、`start` 的作用、单调性、点数

### 4.1 每条被接受的内步都写，没有抽样

录制点只有两处：
```rust
1332:    // Record initial point at t=0 (fresh runs only). On resume the paused
1333:    // leg already recorded a sample at t_paused; the new leg picks up from
1334:    // the next accepted step.
1335:    let mut t = start_state.as_ref().map(|s| s.t_initial).unwrap_or(0.0);
1336:    if start_state.is_none() && t >= t_start {
...
1372:        record_point(
```
```rust
2271:        // Record output point.
2272:        if t >= t_start {
2273:            record_point(
```
`record_point`（`:2388-2420`）**无条件 push**，无 stride/抽样/上限：
```rust
2400:    time_vec.data.as_real_mut().push(t);
2402:    for (idx, (_name, node_idx)) in sorted_nodes.iter().enumerate() {
2403:        node_vecs[idx].data.as_real_mut().push(solution[*node_idx]);
```
拒绝步（NR 失败 `:1591-1611`、LTE 拒绝 `:1636-1651`）在 `record_point` 之前 `continue`，**不入结果**。
负证据：`transient.rs` 内 `decimat|subsampl|sample|stride|max_points|truncate|retain` 只命中注释（`:1217/:1279/:1333/:1389/:2287/:2290`），无任何抽样实现；`h_print` 的 9 处出现（§1.3）无一处涉及输出抽样；`thevenin-0.5.0/src/output.rs` 内 `tstep|h_print|decimat|stride|print_step` 仅 2 处无关命中（`:115` 注释、`:2285` 的 `".tran"` 字符串）。

### 4.2 `start`(tstart) 对输出点的影响

- 只做**门槛**：`if t >= t_start`（`:2272`）。不插入、不重采样、不生成 `t_start` 处的点。
- `t_start > 0` 时 `:1336` 的 t=0 初值点也不记录 ⇒ 时间轴首点 = **第一个 `t >= t_start` 的被接受步**（可能严格大于 `t_start`）。
- `t_start` 不参与步长/断点决策（`t_start` 在 `transient.rs` 只有 `:1336`、`:2272` 两处使用）。

### 4.3 时间点是否可能重复或非严格递增

代码论证：`t += step_h`（`:1700`），而各条路径下 `step_h > 0`：
- `h ≥ h_min > 0`（`:1407`、`:1408`、各分支 `.max(h_min)`），`h_print > 0` 由 `:792` 保证；
- `step_h = h.min(h_max)`（`:1425`）；越界时 `step_h = t_stop - t > 0`（`:1428-1430`）；断点距离 `dist > min_break ≥ 0`（`:1436`，`next_after` 只返回 `> t + min_break` 的点）；`:1443` 的 `.max(h_min)` 兜底为正。

**唯一例外**：`t_max = Some(0.0)`（或 `t_max < 0`）⇒ `h_max = 0`，`step_h = h.min(0) = 0`；若同时不在断点分支，`:1700` 的 `t += 0` 使 `while t < t_stop - h_min`（`:1422`）**死循环**（内核不校验 `t_max > 0`，见 §1.1）。这不是重复时间点，而是挂死。

实测（二次引用同一 CSV）：`out-coarse` 与 `out-fine` 均 `n=2015`、`strictlyIncreasing=True`、`dup=0`、`t0=0`、`tN=2e-6`（= `t_stop` 精确）、`dt_min=2.5e-13`、`dt_max=1e-9`。

### 4.4 返回点数与什么量成正比

- 点数 = 被接受的内步数 + 1（初值点，若 `t_start ≤ 0` 且非 resume），与 `t_stop / h_eff` 成正比；`h_eff` 由 `h_max`（显式 `tmax`，否则 `min(tstep, tstop/50)`）与 LTE（`estimate_new_timestep`，`:377-434`）共同决定，另有 `h_max/400 → ×2` 的爬升段。
- **与 `tstep` 无直接比例关系**（有 LTE 且给了 `tmax` 时，实测 coarse/fine 点数完全相同），除非 (a) `tmax` 缺省（则 `tstep` 进 `h_max`），或 (b) 电路无 LTE 源（则 `:1694` 的 `.min(h_print)` 生效）。
- 注意 Circuit 路径返回 `[op1, tran1]` 两个 plot（`circuit.rs:134-149`），瞬态点数取 `tran1`。

---

## 5. 容差：默认值、传入结构、可用通道、`uic` 作用点

### 5.1 结构与方法来源

```rust
newton.rs:34: #[derive(Debug, Clone)]
newton.rs:35: pub struct NrOptions {
newton.rs:36:     /// Absolute current tolerance (ngspice ABSTOL, default 1e-12).
newton.rs:37:     pub abstol: f64,
newton.rs:38:     /// Relative tolerance (ngspice RELTOL, default 1e-3).
newton.rs:39:     pub reltol: f64,
newton.rs:40:     /// Absolute voltage tolerance (ngspice VNTOL, default 1e-6).
newton.rs:41:     pub vntol: f64,
...
newton.rs:78:     pub chgtol: f64,
...
newton.rs:114:     pub trtol: f64,
```
默认值（`newton.rs:166-232`）：
```rust
169:            abstol: 1e-12,
170:            reltol: 1e-3,
171:            vntol: 1e-6,
172:            itl1: 100,
173:            itl2: 200,
174:            itl4: 10,
178:            itl5: 0,
183:            gmin: 1e-12,
184:            diag_gmin: 1e-12,
185:            chgtol: 1e-14,
198:            gminsteps: 10,
204:            trtol: 7.0,
```

### 5.2 可传通道（传递链）

1. `cirq_ir::Circuit.options: Vec<(String, Value)>`（`cirq-ir/lib.rs:57-58`：`/// Simulation options (e.g. GMIN, ABSTOL, RELTOL).`）。
2. `mna_ir::nr_options_from_circuit(circuit)`（`mna_ir.rs:107-…`；`"ABSTOL" => opts.abstol`(`:131`)、`"RELTOL" => opts.reltol`(`:132`)、`"VNTOL" => opts.vntol`(`:133`)、`"CHGTOL" => opts.chgtol`(`:140`)、`"TRTOL" => opts.trtol`(`:145`)、`"GMIN"`(`:130`)、`ITL1/2/4/5/6|SRCSTEPS`(`:134-139`)、`RSHUNT/GSHUNT/GMINSTEPS/NOOPITER/PIVTOL/PIVREL/…`(`:141-164`)。未识别键静默忽略（`:103-106` 注释）。
3. `TranRunParams.nr_opts`（`transient.rs:672`，填充点 `mna_ir.rs:632`）→ `run_tran` 内 `nr_options = circuit_nr_opts`（`:1326`）。
4. 消费点：LTE 只用 `reltol/abstol/chgtol/trtol`（`:1630-1633`、`:1673-1676` → `estimate_new_timestep` 形参 `:386-389`，容差公式 `:413-415`、`:457-459`）；`gmin` 用于器件 stamp（`:1815/:1873/:2142/:2608/:3522`）；`itl5` 用于总迭代上限（`:1581-1588`）。
5. **`vntol` 不参与 LTE**，只在 NR 收敛判据里：`newton.rs:246-256`
   ```rust
   254:            options.reltol * new[i].abs().max(old[i].abs()) + options.vntol
   256:            options.reltol * new[i].abs().max(old[i].abs()) + options.abstol
   ```
6. `TranAnalysis`（`.tran` 参数）里**没有**容差字段（§6）⇒ 改容差必须走 `circuit.options`。
7. 阈值形态的第二个通道是 `circuit.options` 里的 `METHOD`（`mna_ir.rs:646-655` → `Trapezoidal` 默认 / `euler` / `gear`）。

### 5.3 `uic` 的作用点

```rust
818:    } else if uic {
819:        // When UIC (Use Initial Conditions) is set, skip the DC operating
820:        // point and start from zero with explicit .ic node voltages applied.
821:        vec![0.0; dim]
822:    } else {
823:        // Otherwise, compute the normal DC OP as the starting point.
```
```rust
833:    if start_state.is_none() {
834:        // Apply pre-resolved .ic node voltage overrides.
835:        for (idx, val) in &ic_overrides {
836:            solution[*idx] = *val;
837:        }
...
840:        for cap in &mna.capacitors { if let Some(ic_v) = cap.ic { ... } }   // 841-851
854:        for ind in &mna.inductors { if let Some(ic_i) = ind.ic { ... } }   // 854-858
```
⇒ `uic=true` 只跳过 DC OP（起点为零向量），`.ic` 覆盖仍然生效（`:833` 的条件是 `start_state.is_none()`，与 `uic` 无关）。历史量初始化（`:861-891`）都以该起点为准，电容 `current=0`、`charge_prev=charge`（DC 稳态假设）。

---

## 6. cirq-ir `TranAnalysis` 完整字段定义

```rust
cirq-ir/lib.rs:1695: #[derive(Debug, Clone)]
cirq-ir/lib.rs:1696: pub struct TranAnalysis {
cirq-ir/lib.rs:1697:     pub step: f64,
cirq-ir/lib.rs:1698:     pub stop: f64,
cirq-ir/lib.rs:1699:     pub start: f64,
cirq-ir/lib.rs:1700:     pub uic: bool,
cirq-ir/lib.rs:1701:     /// Maximum internal timestep. `None` means the solver picks automatically.
cirq-ir/lib.rs:1702:     pub tmax: Option<f64>,
cirq-ir/lib.rs:1703: }
```

- **只有这 5 个字段**，`grep TranAnalysis` 在 `cirq-ir-0.5.0/src` 仅 3 处（模块文档 `:21`、枚举分支 `:1655`、结构体 `:1696`）。**没有** `Default` 实现 ⇒ 外部只能用全字段字面量构造（不能 `..Default::default()`）。
- 会影响输出网格的字段只有 `tmax`（→ `h_max`）与 `step`（→ 波形下限、`h_min`、`min_break`、无-LTE 上限、`h_max` 回退）；`start` 仅是输出门槛；`uic` 只影响初值。
- 对比 SPICE 路径（`thevenin-types-0.5.0/src/lib.rs:994-999`）：`Tran { tstep: Expr, tstop: Expr, tstart: Option<Expr>, tmax: Option<Expr>, uic: bool }` —— 同一语义，`tstart`/`tmax` 可选。
- 枚举外层 `Analysis` 是 `#[non_exhaustive]`（`cirq-ir/lib.rs:1650`），`TranAnalysis` 本身**不是**。

---

## 7. 结论与判据（rise=10ns / fall=10ns / max_step=1ns / stop=2us）

### 7.1 不让 tstep 展宽边沿的约束

PULSE 的有效边沿是 `tr_eff = max(tr, tstep)`（`waveform.rs:37`，`tf` 同理 `:38`）：

> **`tr_eff == tr` ⟺ `tstep ≤ tr`；`tf_eff == tf` ⟺ `tstep ≤ tf`。**

对判据组：`tstep ≤ 10 ns`（`1e-8 s`，`> 0` 由 `:792` 保证）。同时 `per` 有 `.max(tstep)` 下限（`:135`），也需 `tstep ≤ per`（本例 `per=20us`，宽松）。

第二个（次级）约束是断点分辨率：`min_break = 5e-5·tstep` 必须小于最小断点间距，否则 `next_after`（`:320-323`）会把后续断点吞掉。本例最小间距 = `tr=10ns` ⇒ `tstep < 10ns/5e-5 = 200 µs`，非约束。

### 7.2 `tstep = 0` 会怎样

`:792-796` 立即返回 `Err(MnaError::UnsupportedElement("invalid .tran parameters"))`，整次瞬态失败（**不是**回退到默认值）。
适配层目前用 `.filter(|s| *s > 0.0).unwrap_or_else(|| span / 1000.0)`（`crates/circuit-backend/src/thevenin.rs:771-775`，我读取时刻的状态）把 0 换成 `span/1000`，所以 `output_interval: 0.s` 到不了引擎（Lead 的 `target/round2-evidence/repro/zero-interval.cdsl` 实测仍产出 2015 点）。若改动后直接写 `step: 0.0`，就会变成硬错误。

### 7.3 `tstep = 1e-15` 会怎样（按是否有 `tmax`、是否有 LTE 分档）

| 场景 | `h_max` | 后果 |
|---|---|---|
| 无 `tmax`，任意电路 | `min(1e-15, tstop/50) = 1e-15`（`:799`） | 步数 ≈ `2e-6/1e-15 = 2e9`，每步对每个节点/支路 push 一个 `f64`（`:2400-2411`）⇒ 实践上挂死/OOM |
| 有 `tmax=1ns`，电路含 C/L/器件电荷（有 LTE，本仓库 RC 属此类） | `1ns` | 网格由 `tmax` 与 LTE 决定（预测与 `tstep=1ns` 的 fine 跑一致）；`tstep` 仅剩 `h_min=1e-24`（`:1407`）、`min_break=5e-20`（`:308`）、初值 `max(h_max/400, h_min)`（`:1408`，此例不改变）的次级影响 ⇒ **安全但无必要** |
| 有 `tmax=1ns`，电路**无** LTE 源（纯电阻/仅源） | `1ns` | `:1694` 的 `.min(h_print)` 生效 ⇒ `h = 1e-15` ⇒ **仍然 2e9 步爆炸**（这条最易被忽略：`tmax` 救不了） |
| NR 失败重试 | — | `:1591` `Err(e) if step_h > h_min * 2.0`：`h_min` 极小时几乎总能重试（此档反而更宽容） |

其它次级影响：`min_break = 5e-20` 使"断点前窗口"几乎为零，但因为 `step_h = bp - t` 是精确减法、`t + (bp - t)` 精确回到 `bp`（Sterbenz），`|bp - t| = 0 < min_break` 仍成立 ⇒ **BE 回退仍会触发**；`times.dedup_by(... < 1e-15)`（`:306`）不随 `tstep` 变，不受影响。

**结论**：`0` 与 `1e-15` 都不可取。安全的选择是让 `tstep` 同时满足"≤ 最小声明边沿"与"≥ 安全下限（避免无-LTE 分支与 `h_max` 回退爆炸）"。对本判据组，`tstep = 1 ns = max_step`（同时显式传 `tmax = 1 ns`）即可：`1ns ≤ 10ns` 不展宽，且 `h_print == h_max` 时 `:1694` 的上限与 `h_max` 等价，不产生额外步数。

### 7.4 `tstep` 取值速查（本判据组，stop=2µs）

| `tstep` | `tr_eff`(声明 10ns) | `min_break` | `h_min` | `h_max`(无 tmax) | 预估步数 | `h_max`(显式 tmax=1ns) |
|---|---|---|---|---|---|---|
| 100ns（当前缺陷映射） | **100ns（展宽 ×10）** | 5ps | 1e-16 | 40ns | ~50+ | 1ns |
| 10ns | 10ns ✓ | 500fs | 1e-17 | 10ns | ~200+ | 1ns |
| 1ns | 10ns ✓ | 50fs | 1e-18 | 1ns | ~2000+ | 1ns |
| 1e-15 | 10ns ✓ | 5e-20 | 1e-24 | 1e-15（**爆炸**） | ~2e9（无 LTE 分支同） | 1ns（有 LTE）/ 1e-15（无 LTE ⇒ 爆炸） |
| 0 | — | — | — | — | `Err`（`:792`） | `Err` |

### 7.5 显式传 `tmax` 时，`h_print` 仍会从哪些路径影响积分步

（A）直接：
1. `h_min = h_print*1e-9`（`:1407`），并被 `:1408/:1443/:1654/:1681/:1686/:1694` 的 `.max(h_min)` 用作步长下限、`:1422` 用作循环下界（`while t < t_stop - h_min`）、`:1591` 用作 NR 重试阈值（`step_h > h_min*2.0`）。
2. 初始步长（`:1408`）：仅当 `h_print*1e-9 > h_max/400`，即 `h_print > 2.5e6·h_max` 时才抬高初值（本例不触发）。
3. `min_break = h_print*5e-5`（`:308`）—— `:1443` 的 `.max(h_min)` 还能把步长抬到超过断点距离，当断点间距 < `h_min` 时断点被跳过。
4. `:1694` 无-LTE 分支的硬上限（与 `h_max` 取小）。

（B）间接（**即使给了 `tmax` 也存在**）：`h_print → TranParams.tstep`（`:1328`）→ 波形求值/断点（`waveform.rs:37-38,75-78,135,271-278,311-312`）→ 断点时间移动 → `:1434-1439` 的距离 clamp 与 `:350-353` 的 BE 判定 → 接受步网格整体移动。

**实测佐证**（二次引用 `target/round2-evidence/repro/`）：`pulse-coarse.cdsl`（`output_interval: 100.ns`）与 `pulse-fine.cdsl`（`1.ns`），其余相同（`rise/fall=10ns, max_step=1ns, stop=2us`）：
- 两者均 `n=2015`、`dt_min=2.5e-13`、`dt_max=1e-9`、首末点相同，但**时间列逐点不同**（间接效应真实存在）；
- `v(vin)` 在 t≈50ns：coarse = **0.5002 V**（被展宽到 `tr_eff=100ns`）；fine = **1.0 V**（`tr_eff=10ns`）。
- 这同时证明：(a) `tstep` 不抽样输出；(b) 展宽效应与网格效应可分离；(c) `max_step` 确实到达引擎（`h_max=1ns=dt_max`）。

### 7.6 两条附带的产品侧风险（不属于本报告任务，交修复任务）

1. **`tmax` 无正值校验**：`:799` 直接使用 `t_max`；`t_max=0` ⇒ `h_max=0`、`:1694/1425` 得到 `step_h=0` ⇒ 非断点分支死循环（§4.3）。适配层必须保证 `max_step > 0`（DSL 已有校验：`crates/circuit-dsl/src/elaborate.rs:2433-2438`）。
2. **EXP 的 `td1` 缺省是 `tstep` 而非 0**（`waveform.rs:75`、断点侧 `:311`）：`tr/tf/tau` 是"默认 + clamp"，而 `td1` 是纯默认；把 `tstep` 调小会顺带把 EXP 的起始延迟调小。此处是否与 ngspice 语义一致，本轮**未核实**（见限制）。

---

## 8. `Waveform` 枚举：各参数 Option 与否、`#[non_exhaustive]` 的影响

定义（`cirq-ir/lib.rs:356-408`）：

```rust
356: /// Transient waveform for voltage/current sources.
357: ///
358: /// `#[non_exhaustive]` — new waveform shapes may be added in any 1.x.
359: #[derive(Debug, Clone)]
360: #[non_exhaustive]
361: pub enum Waveform {
362:     /// `PULSE(v1 v2 [td [tr [tf [pw [per]]]]])`
363:     Pulse { v1: f64, v2: f64,
366:         td: Option<f64>, tr: Option<f64>, tf: Option<f64>, pw: Option<f64>, per: Option<f64> },
372:     /// `SIN(v0 va [freq [td [theta [phi]]]])`
373:     Sin { v0: f64, va: f64,
376:         freq: Option<f64>, td: Option<f64>, theta: Option<f64>, phi: Option<f64> },
381:     /// `EXP(v1 v2 [td1 [tau1 [td2 [tau2]]]])`
382:     Exp { v1: f64, v2: f64,
385:         td1: Option<f64>, tau1: Option<f64>, td2: Option<f64>, tau2: Option<f64> },
390:     /// `PWL(t1 v1 t2 v2 ...)` — piecewise linear.
391:     Pwl(Vec<(f64, f64)>),
392:     /// `SFFM(v0 va [fc [fs [md]]])`
393:     Sffm { v0: f64, va: f64,
396:         fc: Option<f64>, fs: Option<f64>, md: Option<f64> },
400:     /// `AM(va vo fc fs [td])`
401:     Am { va: f64, vo: f64, fc: f64, fs: f64, td: Option<f64> },
408: }
```

|变体|必填|可选（`Option<f64>`）|
|---|---|---|
|`Pulse`|`v1,v2`|`td,tr,tf,pw,per`|
|`Sin`|`v0,va`|`freq,td,theta,phi`|
|`Exp`|`v1,v2`|`td1,tau1,td2,tau2`|
|`Pwl`|`Vec<(f64,f64)>`（无命名字段）|—|
|`Sffm`|`v0,va`|`fc,fs,md`|
|`Am`|`va,vo,fc,fs`|`td`（**注意 `fc/fs` 必填**，与其它波形不同）|

`#[non_exhaustive]` 的影响：
- 只约束**外部 crate 的 match 必须带兜底分支**；thevenin 已按此写：`waveform.rs:99-101` `_ => 0.0`（求值静默返回 0），`:335-337` `_ => {}`（无断点）。⇒ 上游新增波形时，本仓库会**静默得到 0 V / 无边沿断点**，不会编译报错、不会报运行时错误。
- 不影响构造：`#[non_exhaustive]` 标在**枚举**上，各变体自身没有 `non_exhaustive`，外部可直接构造现有变体（本仓库适配层就是这么做的）。
- 同文件 `Analysis` 枚举（`:1649-1651`）也是 `#[non_exhaustive]`，适配层 match 必须带 `_` 分支。

---

## 附：本轮实际执行的命令（全部只读）

|命令|退出码|用途|
|---|---|---|
|`Get-ChildItem`（registry src / workspace / target）|0|定位 vendored 源码与已存在的证据产物|
|`Select-String Cargo.lock`|0|thevenin/cirq-ir 版本与 checksum|
|`Get-Content .cargo_vcs_info.json` / `Cargo.toml`|0|上游 commit 与仓库出处|
|`Get-FileHash -Algorithm SHA256`（5 个源文件）|0|行号锚定|
|`cargo tree --locked --offline -p circuit-backend --depth 1`|**0**|确认解析到 thevenin/cirq-ir 0.5.0（不改 Cargo.lock）|
|`Get-Content target\round2-evidence\repro\*.csv` + 只读统计|0|二次核验时间轴（单调性/步长分布/50ns 处 vin）|

未执行：任何 `cargo test` / `cargo build` / `cargo run`（按任务约定由主代理统一负责），未写 `target` 以外任何临时文件。

---

## 结论

**PASS**（task-1 取证完成：8 个问题全部有 `文件:行号 + 代码摘录` 级证据，且关键推断有 Lead repro 的 CSV 作二次佐证）。

产品侧 NEEDS_FIX 危险点（供修复任务使用，本报告不修改产品代码）：

1. `tstep` 承担了 4 种互不相干的语义（波形默认/下限、`h_max` 回退、`h_min`/`min_break` 尺度、无-LTE 分支硬上限）⇒ 不能再用 `output_interval` 映射它；最小约束 `tstep ≤ min(声明 tr,tf,tau…)`，同时 `tstep ≥` 安全下限，绝不可 0 或 1e-15（§7.1-7.4）。
2. 显式传 `tmax` 时 `tstep` 仍经"波形→断点→距离 clamp"间接移动网格，且对无-LTE 电路仍是硬上限（§7.5）。
3. `t_max` 无正值校验（`t_max=0` ⇒ 死循环）；EXP `td1` 缺省 = `tstep`（§7.6）。

### 未验证项与限制

- **未实测**：`tstep=0`（除适配层过滤外）、`tstep=1e-15`、`t_max=0` 三种情形的运行结果均为**代码推导**，本轮受"只读取证、不跑测试"约束未编译运行任何探针。
- 实证表格引用的 `target/round2-evidence/repro/*` 由 Lead 的 round2 repro 产生，我只做只读统计，**未独立复跑**，也无法排除适配层/CLI 中间层的影响（但 `max_step→tmax`、`dt_max=1e-9`、`dt_min=h_max/4000` 等数据与内核公式逐条吻合）。
- 未核实 ngspice 上游对 EXP `td1` 缺省值与断点算法的实际实现（本轮只用 thevenin 源码）；§7.6 第 2 条只陈述 thevenin 的行为。
- vendored 目录无 `.cargo-checksum.json`，无法用 Cargo.lock 的 sha256 复核本地文件；改用 `.cargo_vcs_info.json` + 事前 SHA256。
- workspace 侧引用（`crates/circuit-backend/src/thevenin.rs:758-782`）是我读取时刻的快照，该文件可能正被其他代理修改；行号以 kernel 侧（registry）为准。
- 未覆盖 `.control` 解释器路径与 `.options` 之外的容差入口（`thevenin-types` SPICE 解析路径），本仓库 Circuit 路径不经过它们。
