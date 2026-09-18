# R2 产品路径取证：`output_interval` / `max_step` 的 DSL → plan → backend → session → results → CLI 全链路

- 任务：共享任务 `task-2`（owner: `r2-product-path`，**只读取证**）
- 仓库：`F:\codexprojects\dsl000`（Windows / pwsh）
- 基线：`git rev-parse HEAD` = `cb5d8a212f66922181580900a05fb3d42abe32f2`（工作树已带他人未提交改动，我未做任何产品/测试/Cargo 改动）
- 取证时刻关键文件 SHA-256（前 16 位；完整值见 `target/round2-recon/r2/commands.txt`）：

| 文件 | SHA-256(16) |
|---|---|
| `crates/circuit-backend/src/thevenin.rs` | `CCB30627A0A5872F` |
| `crates/circuit-dsl/src/elaborate.rs` | `F95ECB7793F69C6D` |
| `crates/circuit-core/src/plan.rs` | `E562162A41A6B0F4` |
| `crates/circuit-session/src/execute.rs` | `1E906B9E5A668799` |
| `crates/circuit-results/src/dataset.rs` | `CEB84D6730EC946B` |
| `crates/circuit-results/src/measure.rs` | `E20BDB321180F9D2` |

> 行号锚点以该修订为准；`thevenin.rs`/`elaborate.rs` 正在被其他代理修改，**符号名比行号更稳**。

---

## 1. DSL 语法与校验（问题 1）

### 1.1 `tran` 参数解析与允许列表

`crates/circuit-dsl/src/elaborate.rs:2395-2462`（`fn tran_spec`）：

```rust
2395: fn tran_spec(&mut self, call: &AnalysisCall) -> Option<AnalysisKind> {
2396:     let allowed = ["stop", "start", "max_step", "output_interval"];
2397:     self.check_analysis_args(call, &allowed, "tran");
2400:     let stop  = self.req_quantity(call, "stop",  TIME, &mut scope)?;
2401:     let start = match call.arg("start") { ... None => Quantity::seconds(0.0) };
2405:     if stop.value <= start.value { ... Code::Sweep ... .at(call.span) ... }   // 2405-2417
2418:     if start.value < 0.0      { ... Code::Value "`tran start:` must not be negative"
2419:                                    .at(call.arg("start").unwrap().value.span) } // 2418-2427
```

- `output_interval` **已在 `allowed` 里**（:2396），因此语法层面早已接受，不需要改解析器；`check_analysis_args`（:2688-2702）只报"未知参数"（`Code::Argument`，`.at(arg.name_span)`，:2694-2700）。
- `req_quantity`（:2652-2670）：缺失参数 → `Code::Argument`，`.at(call.span)`；量纲/类型/有限性交给 `num_arg`（:2669 → :1541-1583）：
  - 量纲错 → `Code::Dimension`，`.at(arg.value.span)`（:1550-1563）；
  - 非数值 → `Code::Type`，`.at(arg.value.span)`（:1569-1576）；
  - **非有限值 → `Code::Value` "`…` is not a finite number"，`.at(arg.value.span)`**（`check_finite`，:1565 → :1585-1591）。
    即 `NaN` / `±inf` **已经被前端拒绝**（`num_arg` 调 `check_finite`），我实测读码确认；R4 报告里"未验证 NaN/inf"一项在此可结案：`check_finite` 覆盖。

### 1.2 `max_step` 的现有正值校验（模板）

`elaborate.rs:2429-2444`：

```rust
2429: let max_step = match call.arg("max_step") {
2430:     Some(_) => Some(self.req_quantity(call, "max_step", TIME, &mut scope)?.value),
2431:     None => None,
2432: };
2433: if let Some(ms) = max_step
2434:     && ms <= 0.0
2435: {
2436:     self.error(
2437:         Diagnostic::error(Code::Value, "`max_step:` must be greater than zero").at(call
2438:             .arg("max_step")
2439:             .unwrap()
2440:             .value
2441:             .span),
2442:     );
2443:     return None;      // ← 关键：立即 return None，不产生 TranSpec
2444: }
```

要点：诊断码 `Code::Value`；span 取**该参数的 value span**（不是 `call.span`）；`return None` 使该分析任务根本不进入 plan（下游拿不到半成品）。

### 1.3 `output_interval` 缺什么

`elaborate.rs:2446-2461`：

```rust
2446: let output_interval = match call.arg("output_interval") {
2447:     Some(_) => Some(
2448:         self.req_quantity(call, "output_interval", TIME, &mut scope)?
2449:             .value,
2450:     ),
2451:     None => None,
2452: };
2453:
2454: Some(AnalysisKind::Tran(TranSpec {
2455:     start_s: start.value,
2456:     stop_s: stop.value,
2457:     max_step,
2458:     output_interval,
2459:     uic: false,
2460:     span: call.span,
2461: }))
```

**只差一个 `<= 0` 的校验**（与 `:2433-2444` 完全对称即可）；其余（量纲、类型、非有限）由 `num_arg` 覆盖。`-1.ns` 今天能过 `check` 的唯一原因是这里没有正值判断，然后 `thevenin.rs:772-775` 的 `.filter(|s| *s > 0.0)` 把它吞掉。

### 1.4 诊断如何进入 `cdsl check`

- 前端错误累积在 `Elaborator.error_count/diagnostics`（:276-279），`compile` 与 `elaborate_experiment` 在 `diagnostics.has_errors()` 时返回 `Err`（:122-129、:164-170）→ `cdsl check`（`crates/circuit-cli/src/check.rs:45-51`）直接失败。
- 另有后端能力边界：`check.rs:55-62` 对每个 experiment 调 `backend.validate(&e.circuit, &e.plan)`。
  ⇒ **两层都要加校验**：DSL 层给用户源位置好的诊断，`TheveninBackend::validate` 层覆盖绕过 DSL 的 IR 调用者（后端测试、`_probe` 式直接构造）。

### 1.5 语言规范里的缺口（文档事实）

`docs/language.md:318-341` 的 `tran` 文档只列 `stop/start/max_step`，**全文没有 `output_interval`**（`:340-341` 只写"`max_step` 是最大内部步长，不是输出间隔"）。即该选项是"接受但未文档化"，修复时必须补 `docs/language.md`，否则语义无处可依。

---

## 2. 计划类型 `TranSpec`（问题 2）

### 2.1 定义与文档

`crates/circuit-core/src/plan.rs:148-165`：

```rust
148: /// A transient specification.
150: /// `max_step` is the maximum internal timestep, not an output interval: the
151: /// solver is free to take smaller steps and the returned time axis is
152: /// generally non-uniform (brief §8.4).
153: #[derive(Clone, Debug)]
154: pub struct TranSpec {
155:     pub start_s: f64,
156:     pub stop_s: f64,
157:     pub max_step: Option<f64>,
158:     /// Requested output interval. `None` means "whatever the solver produced".
159:     pub output_interval: Option<f64>,
163:     pub uic: bool,
164:     pub span: SourceSpan,
165: }
```

`AnalysisKind::Tran(TranSpec)` 在 `plan.rs:169-174`；`plan.rs:150-152` 的注释已经把"`max_step` 不是输出间隔"写对，但 `:158-159` 把 `output_interval` 定义成"请求的输出间隔"却从未被兑现——**类型注释与实现语义不一致**，修复后注释要同步（"求解后重采样"）。

### 2.2 全部构造点（grep `TranSpec {`，6 命中 = 1 定义 + 5 字面量）

| # | 位置 | 作用 |
|---|---|---|
| 1 | `crates/circuit-dsl/src/elaborate.rs:2454` | 唯一产品构造点（DSL） |
| 2 | `crates/circuit-backend/tests/adapter.rs:300-307` | `rc_transient_matches_analytic`，`max_step = output_interval = τ/200` |
| 3 | `crates/circuit-backend/tests/phase_regression.rs:668-675` | `sin` 相位回归，`output_interval = stop/1000` |
| 4 | `crates/circuit-backend/tests/phase_regression.rs:786-79x` | 第二个 tran 任务，`output_interval = stop/500` |
| 5 | `crates/circuit-backend/tests/transient_reference_regression.rs:274-281` | 共享 helper `tran_plan(...)`，4 个用例都经过它 |

`TranSpec` **没有 `Default`、没有 `#[non_exhaustive]`** ⇒ 任何新增字段都会立刻打断这 5 处字面量。冻结设计（`docs/review-evidence/round2/design-freeze.md:33`）保持 `output_interval: Option<f64>` 不变 ⇒ **这 5 处无需改动**，是最小改动路径。

### 2.3 全部读取点

| 位置 | 读什么 |
|---|---|
| `crates/circuit-backend/src/thevenin.rs:758-783` | `spec.stop_s/:start_s`（:771、:778-779）、`spec.output_interval`（:772-775）、`spec.max_step`（:781）、`spec.uic`（:780）——**唯一的运行时读者** |
| `crates/circuit-backend/src/thevenin.rs:362-367` | 仅按 `AnalysisKind::Tran(_)` 分派 `simulate_tran` |
| `crates/circuit-backend/src/thevenin.rs:1214-1218` | 仅按 `AnalysisKind::Tran(_)` 造 `Axis::Time` |
| `crates/circuit-dsl/tests/elaborate.rs:242-250` | 断言 `t.stop_s`、`t.max_step > 0`、`t.output_interval == None`（:247）；再断言一条"`max_step` 不得变成 output interval" |
| `crates/circuit-session/src/execute.rs:202-220` | `as_single_point_plan` 只把 `Dc` 改成 `Op`，`Tran` 原样 clone（参数扫描路径） |

⇒ 若要"拆语义"（新类型/新字段），必须改的是：**5 个构造点 + `thevenin.rs:758-783` 一个读取点**；`grep start_s|stop_s` 显示再也没有别的读者。我的建议：R2 不新增字段，只改 `thevenin.rs` 的映射与 `plan.rs:158-159` 注释。

---

## 3. 后端：映射、结果封装、trait、能否重采样（问题 3）

### 3.1 `map_analysis` 的 TRAN 分支（当前缺陷现场）

`crates/circuit-backend/src/thevenin.rs:743`（`fn map_analysis(circuit, task)`），TRAN 分支 `:758-783`：

```rust
758: AnalysisKind::Tran(spec) => {
759-770:  // 注释：tmax 是 h_max；step 不是输出间隔；engine 没有输出抽样；
          //       并明说 step 会被用作 PULSE tr/tf 的下限，"declared rise shorter than
          //       step is silently widened"
771:     let span = spec.stop_s - spec.start_s;
772:     let step = spec
773:         .output_interval
774:         .filter(|s| *s > 0.0)
775:         .unwrap_or_else(|| span / 1000.0);
776:     CqAnalysis::Tran(CqTran {
777:         step,                 // → CqTran.step  == 引擎 TranParams.tstep / h_print
778:         stop: spec.stop_s,
779:         start: spec.start_s,
780:         uic: spec.uic,
781:         tmax: spec.max_step,  // → 引擎 t_max == h_max
782:     })
783: }
```

- 调用链：`run`（:299-326，任务循环 :318-324）→ `run_task`（:330-390）→ `build_circuit`（:396-450）→ `map_analysis`（:426）；求解 `simulate_tran(&cq)`（:365）。
- 映射到引擎的唯一通道：`cirq-ir::TranAnalysis { step, stop, start, uic, tmax }`（`cirq-ir-0.5.0/src/lib.rs:1695-1703`）→ `thevenin-0.5.0/src/mna_ir.rs:581,626-630`（`t_step: tran.step, t_max: tran.tmax, …`）→ `TranRunParams`。
- **引擎没有输出采样**（本轮独立复核，与 design-freeze §1 一致）：
  - `h_max = t_max.unwrap_or(min(h_print, t_stop/50))` — `thevenin-0.5.0/src/transient.rs:798-799`；
  - `min_break = tstep * 5e-5` — `transient.rs:308`（断点去重窗 `:321-353`）；
  - `h_min = h_print * 1e-9`、初始 `h = max(h_max/400, h_min)` — `transient.rs:1407-1408`；
  - 非电抗/无 LTE 分支步长上限 `min(h_max, h_print)` — `transient.rs:1689-1694`；
  - **PULSE 边沿下限**：`tr.unwrap_or(tstep).max(tstep)`、`tf` 同 — `thevenin-0.5.0/src/waveform.rs:37-38`；breakpoints 用同样的 clamp — `waveform.rs:271-272`；`period = per.unwrap_or(tstop).max(tr+pw+tf).max(tstep)` — `waveform.rs:135`；
  - **输出录制 = 每条被接受的内步**（`record_point` 同一调用里 push 时间 + 所有节点/支路，`transient.rs:2388-2419`；录制条件 `t >= t_start`，`:2272-2285`；t=0 首点单独录，`:1336-1383`）⇒ 想要"按 output_interval 抽样"引擎做不到。
  - 本项目 IR 只有 `Pulse/Sin/Pwl`（`crates/circuit-core/src/ir.rs:210-233`），适配器一律显式传 `tr/tf/td/pw/per`（`thevenin.rs:662-680`）⇒ 实际生效的只有 `.max(tstep)` clamp（`.unwrap_or(tstep)` 分支不可达）。
- `output_interval` 的另一半语义（默认 `span/1000`）**同样会污染波形**：`rc_filter.cdsl`（未写 `output_interval`）实际得到 `step = 500 µs/1000 = 500 ns`，把声明 `rise: 1.ns` 拉宽成 500 ns（`examples/rc_filter.cdsl:8-24, 51`；`docs/review-evidence/backend-contract.md:108-110`）。

### 3.2 结果如何变成 `Signal`：时间轴来源、是否共享

- `convert_plot`（`thevenin.rs:457-532`）先 `build_axis`（:467）再物化信号：
  - `build_axis` 的 TRAN 分支（:1214-1218）：从 plot 里找 **名为 `"time"` 的向量** ⇒ `Axis::Time(d)`，**整个数据集只有这一条时间轴**；
  - 探头物化 `materialise_probe`（:916-1034）：每个 probe 从**自己的向量**拷数据成 `Signal`（`make_signal`，:869-885）；无 probe 时暴露 plot 全部可翻译向量（:472-482）；
  - 差分电压 `v(a,b)`（:938-995）与电阻电流推导（:1044-1191）都是**逐元素**对两条向量做减法/相除 ⇒ 隐含前提是"所有向量与 `time` 同索引对齐"（来自 `record_point` 的成对 push）；长度不一致时 `:962-973` / `:1103-1116` 直接报 `Code::Backend`。
  - **结论：是的，一个 tran 数据集里所有 probe 共享同一条时间轴**，`Dataset::validate` 再强制 `signal.len() == axis.len()`（`crates/circuit-results/src/dataset.rs:509-540`），不等长直接 `Code::Value` 报错（不截断、不补齐）。
- 数据集由 `Dataset::new`（`thevenin.rs:513-527` → `dataset.rs:483-503`）构造，`BackendInfo` 目前只写 `adapter` 版本（`thevenin.rs:306-307`）——`max_step`/`output_interval` 等求解设置**目前完全没有进元数据**。

### 3.3 `SimulationBackend` trait 的公共接口

`crates/circuit-backend/src/backend.rs:64-84`：

```rust
65: pub trait SimulationBackend {
66:     fn capabilities(&self) -> BackendCapabilities;
73:     fn validate(&self, circuit: &Circuit, plan: &AnalysisPlan) -> Result<(), Diagnostics>;
79:     fn run(&mut self, circuit: &Circuit, plan: &AnalysisPlan)
83:         -> Result<SimulationResults, Diagnostics>;
84: }
```

`SimulationResults { pub datasets: Vec<Dataset> }`（`backend.rs:46-50`，唯一构造点 `thevenin.rs:325`）。返回类型是**扁平的 Dataset 列表**，没有"原始/输出"两套结果的通道。实现者唯一：`TheveninBackend`（`thevenin.rs:119`）；调用方：`execute.rs:94`、`sweep.rs:255`、`cli/check.rs:58`（validate）、4 个集成测试文件。

- `TheveninBackend::validate`（`:124-299`）**目前没有任何 tran 参数检查**（grep `Tran|tran|max_step|output_interval` 在 124-299 区间 0 命中）：不支持的器件、模型、探针电流、DC 扫描目标在这里失败；`output_interval <= 0` 即使从 IR 层进来也不报错。`run` 只调一次 `validate`（:304）。
- 若要新增"transient 输出采样能力检查"，`validate` 的签名够用（有 `circuit` 与 `plan`），诊断 span 可用 `TranSpec.span`（`plan.rs:164`，由 `elaborate.rs:2460` 设为 `call.span`）——与 DC 分支用 `spec.sweep.span` 的既有做法一致（`thevenin.rs:793, 802`）。

### 3.4 后端"能不能在返回前重采样"？

**技术上可以，但不该在这里做。**

- 可以：`convert_plot` 拿到完整 `SimPlot`（含 `time` 向量与全部信号向量，`thevenin.rs:463-467`），在 `Dataset::new`（:513）之前把 `plot.vecs` 或 `Signal` 重采样即可，改动局域。
- 不该：
  1. 测量在**下游**才算（`execute.rs:120` → `measure.rs`），后端重采样会直接把"用户看到的网格"变成"测量的网格"，违背"粗输出采样不得进入积分测量"（README/计划要求见 `docs/next-iteration-plan.md:63`）；
  2. `SimulationResults` 是扁平列表，无法在一等公民层面同时交付"原始 + 输出"两套；
  3. 后端现有测试**把求解器网格当契约钉住**：`adapter.rs:353-362` 断言 tran 轴非均匀、`transient_reference_regression.rs:742-781` 断言点数随 `max_step` 严格递增（516/5025/50115）、`:684-726` 专门复现"声明 1 ps 被 500 ns clamp"。在适配器里重采样会让这些测试失去意义或直接失败。
- **引擎能力结论**：resample 只能由本项目在求解后自己做（线性插值），引擎侧的 `step` 永远不是输出间隔。

---

## 4. 会话与结果：执行、保存、测量、导出、REPL vs 文件（问题 4）

### 4.1 唯一执行路径

`crates/circuit-session/src/execute.rs:79-127`（`pub fn execute<B: SimulationBackend>`）：

- 重新 elaborate（:84-89）→ 判参数扫描（:91）→ **普通路径直接 `backend.run(&elaborated.circuit, &elaborated.plan)`（:94）**，结果 `run.datasets` 叠加 override 元数据（:97-104）；
- 空结果 → `Code::Backend`（:109-114）；
- warnings 收集（:116-119）；
- **`let measures = evaluate_measures(&elaborated.plan, &datasets);`（:120）** —— 测量在这一层、针对**后端原样返回的 datasets** 计算；
- 返回 `RunOutcome { datasets, measures, warnings }`（:25-30、:122-126）。

两个入口**共用**这条路径：

| 入口 | 位置 |
|---|---|
| 文件模式 `cdsl run` | `crates/circuit-cli/src/run.rs:76-94`（`circuit_session::execute`，:87） |
| REPL `:run` | `crates/circuit-session/src/session.rs:484-564`（`run_command`）→ `:567-594`（`Session::run`，:594 调 `execute`） |

差异只在**周边**：文件模式先跑 `check::front_end`（`run.rs:24` → `check.rs:25-76`，含 `backend.validate`，:55-62）与写盘格式由 CLI 决定（`run.rs:97-101`）；REPL 没有独立 `:check` 命令（`session.rs` grep `check` 0 命中），编译走 `session.rs:306` `circuit_dsl::compile`，后端校验靠 `execute → backend.run → validate`（`thevenin.rs:304`）；REPL 写盘固定 `Format::Both`（`session.rs:647-653`）。⇒ **语义一致，展示/写盘参数不同。**

### 4.2 结果保存与导出

- `write_datasets(out, format, datasets, approve)`：`execute.rs:425-463`；逐数据集写 `{experiment}.{analysis}.csv/json`（:440-450），内容来自 `circuit_results::to_csv` / `to_json`（:443-448）。
- 调用点：`cli/run.rs:103-117`（带 `guard_output` 拒绝覆盖输入文件）、`session.rs:649`（REPL `--out`，approve 恒 Ok）。
- 导出实现：`crates/circuit-results/src/export.rs:59-109`（CSV：轴列 + 每信号列）、`:156-203`（JSON：`axis{type,unit,values}` + signals + backend 元数据）；重导出在 `crates/circuit-results/src/lib.rs:59-65`。
- **没有 `TranResult` 类型**（全库 grep `TranResult` 0 命中）：tran 结果就是 `Dataset { axis: Axis::Time(..), signals, backend, .. }`（`dataset.rs:462-475`）。题面中的 "`TranResult`" 在本代码库不存在，对应物是 `Dataset`。

### 4.3 `measure` 的时间积分实现（关键）

`crates/circuit-results/src/measure.rs`：

- 定义（:1-27）：`avg = ∫x dt / ∫dt`、`rms = sqrt(∫x² dt / ∫dt)`；`:12-17` 明确"tran 轴一般非均匀 ⇒ 梯形积分而不是样本均值"。
- 入口：`measure_signal`（:118-125）→ `measure`（:106-114）→ `reduce`（:131-142）→ `integral`（:182-256）。
- **轴就是 `dataset.axis`**（:189 `let Axis::Time(times) = axis else { Code::Type }`），样本是信号向量（:202 `value.magnitudes()`），长度不一致报 `Code::Value`（:203-212），<2 点报错（:213-221），梯形累加在 `:226-233`：
  ```rust
  226: for i in 0..times.len() - 1 {
  227:     let (t0, t1) = (times[i], times[i + 1]);
  228:     let (x0, x1) = (samples[i], samples[i + 1]);
  231:     area += 0.5 * (x0 + x1) * dt;
  232:     area_sq += 0.5 * (x0 * x0 + x1 * x1) * dt;
  }
  ```
- **因此 avg/rms 用的轴 = 后端返回的 `Axis::Time` = 引擎 `time` 向量**（`thevenin.rs:1214-1218`）。`max/min` 是纯样本极值、与轴无关（:150-179）。

---

## 5. 关键风险：重采样不能污染测量（问题 5）

### 5.1 今天的事实

- `evaluate_measures`（`execute.rs:336-367`）对每个 measure 选择**最富分析**的 dataset（tran 优先，:336-338、:370-377），然后 `measure_signal(kind, …, d)`（:355）在**该 dataset 自己的轴上**积分。
- 所以：**只要重采样后的 `Dataset` 变成 `RunOutcome.datasets` 的元素，`avg/rms/max/min` 立刻跟着改**——`avg/rms` 会变成"重采样网格上的梯形积分"，`max/min` 会变成"重采样点上的极值"（可能漏峰）。

### 5.2 数学边界（给实现者的判据）

- 线性插值 + 输出网格严格落在 `[t0, t_last]` 内 + **首末点原值保留**时，梯形法对分段线性函数是精确的 ⇒ `∫x dt`、`∫x² dt`（以及 `∫dt`）与原网格**数学相等**（仅浮点舍入差）。这是"允许在输出数据上做积分"的充分条件。
- 破坏条件（必须避免，且是审查重点）：输出网格在首/末点被裁剪（`∫dt` 变化 ⇒ avg/rms 漂移）、使用高阶/样条插值（不再是原分段线性函数）、对非有限样本外推、`start_s > 0` 时把网格强行对齐到 `start_s` 而原始首点在 `start_s` 之后。
- 但 `max/min` **无论怎么插值都可能变**（极值是样本泛函）⇒ 结论：**测量必须固定在原始网格上，不能在重采样数据上算**。

### 5.3 存放位置建议（与 design-freeze §2.3 一致）

- `RunOutcome.datasets` **保持原始求解数据不变**（唯一构造点 `execute.rs:122`，`measures` 紧跟其后由 `:120` 计算）——测得的 avg/rms/max/min 与 `output_interval` 完全无关。
- 新增 `RunOutcome.output_datasets: Vec<Dataset>`（重采样后）专供**展示/导出**；`write_datasets` 与两个入口的展示读它。
- 元数据区分：`BackendInfo.settings`（`dataset.rs:429-451`）写 `tran.solve_points` / `tran.output_grid` / `tran.output_interval` / `tran.solver_step` 等（冻结清单见 `design-freeze.md:82-84`），使"原始 vs 重采样"在 JSON 里可判别。
- 补充风险：**REPL/CLI 的摘要目前读 `outcome.datasets`**（`session.rs:607-632` 打印 `d.signals.first().len()` 作点数；`run.rs:154-156` 同样调 `outcome.summaries()`），若不改这两处，修复后用户界面仍会显示"2015 time points"，与 CSV 实际行数（≈21）矛盾。这是验收时最容易漏的一致性点。

---

## 6. 推荐实现层次与接口改动清单（问题 6）

### 6.1 推荐层次（与 Lead 冻结设计一致，我独立复核为最优）

| 层 | 文件 | 改什么 | 为什么在这一层 |
|---|---|---|---|
| L1 前端校验 | `crates/circuit-dsl/src/elaborate.rs:2446-2452` | 仿 :2433-2444 加 `Code::Value` 正值校验（`.at(arg.value.span)`，`return None`） | 需要源位置级诊断；`check` 与 `run`、REPL 都经此 |
| L2 计划类型 | `crates/circuit-core/src/plan.rs:148-165` | **保持 `output_interval: Option<f64>` 不变**；只改 :158-159 注释为"求解后重采样" | 避免打断 5 个 `TranSpec` 字面量（§2.2），语义拆分靠后端映射而非类型 |
| L3 后端边界 | `crates/circuit-backend/src/thevenin.rs` | ① `validate`（:124-299）加 tran 能力校验（有限正 `output_interval`、`h_print` 可解析性），用 `spec.span`；② `map_analysis`（:758-783）把 `step` 从 `output_interval` 解耦，改成 `h_print` 单一函数（`validate` 与 `map_analysis` 共用），规则见 `design-freeze.md:37-57`；③ `caps().notes`（:110-112）文案更新 | 这是唯一 IR→引擎的映射点；trait 签名不必改 |
| L4 重采样实现 | **新** `crates/circuit-results/src/resample.rs` + `lib.rs:59-65` 重导出 | `Dataset`（`Axis::Time`）→ 新 `Dataset`：等间隔网格 + 相邻样本线性插值 + 首末点原值 + `max_result_values` 上限（`dataset.rs:562-575` 已有校验） | 中立层、无后端依赖、可脱离求解器做单元测试；`circuit-results` 本就依赖 `circuit-core`（Limits/Diagnostic） |
| L5 会话接线 | `crates/circuit-session/src/execute.rs:25-30, 116-127, 425-463` | `RunOutcome` 增 `output_datasets`；在 `measures`（:120）**之后**重采样；`write_datasets` 改为写输出数据；`BackendInfo.settings` 记录设置 | `execute` 是文件模式与 REPL 的唯一共享点（§4.1），测量的 raw 语义在此被固定 |
| L6 展示/CLI | `crates/circuit-cli/src/run.rs:103-117, 140-160`；`crates/circuit-session/src/session.rs:607-653` | 写盘与摘要改读 `output_datasets`（或至少同时显示 raw/output 点数） | 否则界面与 CSV 不一致（§5.3） |

**不推荐**：把重采样放进 `thevenin.rs`（§3.4：会破坏 raw 契约与 3 组后端测试）；放进 `circuit-core`（core 不应有结果层类型）；新增 crate（无必要）。

### 6.2 受影响公共 API（精确清单）

| API | 位置 | 现状调用点 | 改动影响 |
|---|---|---|---|
| `TranSpec` 字段集 | `plan.rs:154-165` | 5 个字面量（§2.2）；1 个读取点 `thevenin.rs:771-781` | 保持字段不变 ⇒ **0 破坏**；若加字段/换类型 ⇒ 6 处同改 |
| `RunOutcome` | `execute.rs:25-30`；重导出 `session/src/lib.rs:18` | 构造 `execute.rs:122`；字段读 `session.rs:599,613,649`、`cli/run.rs:106,146` | 加字段（`Default`/`..` 不适用，`RunOutcome` 无构造器外的消费方会编译报错处= 只有上述读点，均可控） |
| `write_datasets` | `execute.rs:425-463`；重导出 `session/src/lib.rs:18`、`session.rs:713` | `cli/run.rs:103`、`session.rs:649` | 改签名（或新增 `write_output_datasets`）→ 2 个调用点 + 1 个 re-export 需同步；集成测试 `crates/circuit-cli/tests/{e2e,repl}.rs` 若直接调它也要改（当前只经 CLI 进程，未直接调） |
| `circuit_results` 新模块 | `circuit-results/src/lib.rs:54-65` | 无现有调用者 | 纯新增；`pub use resample::{…}` |
| `TheveninBackend::validate` | `thevenin.rs:124`（trait `backend.rs:73`） | `thevenin.rs:304`、`cli/check.rs:58` | 新增校验分支，签名不变 |
| `BackendCapabilities.notes` | `thevenin.rs:105-114` | `cdsl capabilities`（README.md:178 引用同一文本） | 文案变更会牵动 README/QA 文本，非测试断言 |

### 6.3 现有测试调用点（实现时必须同步处理）

| 测试 | 位置 | 为什么受影响 |
|---|---|---|
| `rc_transient_matches_analytic` | `crates/circuit-backend/tests/adapter.rs:254-366` | 显式 `output_interval = τ/200`（:304）且声明 rise=1 ps（:271）；`step` 映射改变后 `T_eff` 不再是 `max(rise, τ/200)`，:353-362 的"非均匀轴"断言在"重采样不进后端"的前提下仍成立，但期望边沿/噪点需按新 `h_print` 重算 |
| `declared_rise_below_output_interval_is_clamped_to_the_output_step` | `transient_reference_regression.rs:684-726` | **钉的正是被修复的旧映射**（`T_eff = max(rise, output_interval)`，:687）：解耦后 clamp 不再由 `output_interval` 触发，必须重写为"新映射下声明边沿不被展宽"的断言 |
| `max_step_reaches_the_engine_and_every_setting_meets_the_criteria` | `transient_reference_regression.rs:742-781` | 注释 :744-745 "cap 是被测的 max_step 而不是 output_interval" 在新语义下失效；点数仍应随 `max_step` 严格递增（重采样在后端之后 ⇒ 后端测试不受影响），余量需实测 |
| `effective_edge` helper + 文件头 | `transient_reference_regression.rs:21-42, 351-356` | 文档化的 `T_eff = max(declared rise, output_interval)` 前提作废，须改成基于新 `h_print` 的表达 |
| `phase_regression.rs:668-690, 786-800` | `output_interval = stop/1000`、`stop/500` | `h_print` 改为 `min(span/1000, waveform_bound)` 后点数会变（sin 无 tstep 依赖 ⇒ 走默认 `span/1000`，与旧值相同 ⇒ 预计不变，但需实测确认 `t.len() > 500` 仍过） |
| `elaborate.rs::…tran 断言` | `crates/circuit-dsl/tests/elaborate.rs:242-250` | `:247` 断言 `output_interval == None` 仍成立；需**新增**零/负数拒绝用例（`Code::Value`、span 落在参数上） |
| 会话级会话测试 | `crates/circuit-session/tests/session.rs`（grep `output_interval` 0 命中） | 不受直接语法影响；需**新增** "改 `output_interval` 不改变 `measures`" 的回归 |
| CLI e2e | `crates/circuit-cli/tests/e2e.rs:275-360`（读 `response.tran1.csv`、断言非均匀轴）、`repl.rs:231-233` | 用 `examples/rc_filter.cdsl`（无 `output_interval`）⇒ 输出网格=原始网格，**预计不变**；但 `rc_filter` 的 `h_print` 会因新规则变为 `min(500ns, 1ns) = 1ns`（求解波形由 500ns 斜坡恢复成 1ns 斜坡），文件里那段"实际是 500 ns 斜坡"的注释（`examples/rc_filter.cdsl:7-24`）与 README.md:231 都变成**过时描述**，必须同步改，否则文档与新行为矛盾 |

### 6.4 我在冻结设计之外补充的风险点（建议纳入验收）

1. **`h_print` 会同时改变"非电抗电路"的点数**：`transient.rs:1694` 用 `min(h_max, h_print)` 限制无 LTE 电路的步长。`MAX_PRINT_STEPS = 1e6`（`design-freeze.md:49`）是第一道闸，但 `Limits::max_result_values = 5e7`（`crates/circuit-core/src/limits.rs:38`）才是 results 层实际闸门（`dataset.rs:562-575`）。两者都要给出**点名 tran 参数**的诊断，否则用户会先看到 results 层的 `E_LIMIT` 而不知道是 `rise` 太小。
2. **既有示例不会触发新能力上限**（我按新规则核算）：`rc_filter` span=500 µs、`rise=1 ns` ⇒ 5e5 步 (<1e6)；`rlc.cdsl:33-44` span=600 µs、`rise=10 ns` ⇒ 6e4；`diode_rectifier.cdsl:19,26` 只有 `sin`（无 tstep 依赖）⇒ 走 `span/1000 = 3 µs`。⇒ 兼容，可放心加闸。
3. **`start > 0` 时输出网格起点**：引擎只在 `t >= t_start` 时录制（`transient.rs:2272`），原始首点不保证等于 `start_s`。冻结规则"起点=原始首点 `t0`"（`design-freeze.md:65`）与引擎事实一致，但**必须写进用户文档**，否则"固定起点"会被读成"等于 `start:`"。
4. **`period` 也进 `waveform_bound` 是对的**（`waveform.rs:135` 的 `period.max(tr+pw+tf).max(tstep)`），但注意 `PULSE` 的 `per` 适配器恒传 `Some`（`thevenin.rs:679`）⇒ `period` 一定参与 `min`，文档要说明"很短的 period 会收紧 `h_print`"。
5. **诊断一致性**：前端用 `Code::Value`（模板 :2437），后端 `validate` 若也要报同一错误，建议同样给 `Code::Value` 并用 `spec.span`（`plan.rs:164`），避免同一个非法值在两层出现两种错误码。

---

## 7. 结论

**PASS** —— 六个问题均以源码事实回答，调用链闭合：

- DSL：`elaborate.rs:2395-2462`，`max_step` 正值模板在 :2429-2444，`output_interval` 仅缺 `>0` 校验（:2446-2452）；非有限值已由 `num_arg→check_finite`（:1565, :1585-1591）拒绝。
- 计划：`plan.rs:154-165`；5 个构造点、1 个运行时读取点（清单见 §2）。
- 后端：`thevenin.rs:758-783` 是唯一映射点；`step` 同时被引擎当作 `h_max` 回退、`h_min/min_break` 基准、非电抗步长上限与 **PULSE/EXP 边沿下限**（`thevenin-0.5.0` 行号见 §3.1）；引擎**无输出抽样**，输出=每条接受内步；一个数据集**共享一条 `time` 轴**；`SimulationBackend` 签名见 `backend.rs:65-84`。
- 会话/结果：文件模式与 REPL 共用 `circuit_session::execute`（`run.rs:87`、`session.rs:594`）；测量在 `execute.rs:120` 对 raw `Dataset` 的轴做梯形积分（`measure.rs:182-256`）。
- 风险：重采样一旦进入 `RunOutcome.datasets`，`avg/rms/max/min` 全部随 `output_interval` 变化；推荐 raw 与 output 双数据集，测量固定在 raw。
- 层次：`circuit-results/src/resample.rs`（新模块）+ `circuit-session::execute` 接线，后端只做 `step` 解耦与 `validate`，与 Lead 的 `docs/review-evidence/round2/design-freeze.md` 一致；我补充的 5 条风险（§6.4）建议纳入验收。

### 未验证项与限制

1. **未运行任何构建/测试/CLI**：本任务要求只读取证，我未跑 `cargo test`、未跑 `cdsl run`（会写 `target/debug`、与并行代理争锁）。P1/P2 的运行时表现引自已冻结证据 `docs/review-evidence/round2/repro-baseline.md`（真实 CLI，exit 0，含缺失边沿/回退判别探针）与 `docs/next-iteration-plan.md:26-43`，非我复现。
2. **未验证修复后行为**：重采样点数、`h_print` 新规则下各测试的期望值、REPL/CSV 一致性都需实现后实测（主代理负责全量测试）。
3. **行号漂移**：`thevenin.rs`/`elaborate.rs` 正被其他代理修改（工作树已 dirty）；本报告以取证时刻哈希为准，符号名是稳定锚点。
4. **未验证 `--format json` 的重采样落盘**、`_probe`（独立工程，grep `output_interval` 0 命中）不受影响这一推断未实测。
5. **未覆盖 AC/DC/OP 路径**：重采样只应作用于 `Axis::Time`；`stitch`（`execute.rs:223-326`）产出 `Axis::Parameter`，参数扫描路径与重采样的交互未实测。
