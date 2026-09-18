# W6 独立代码审核（Task A+B 冻结候选 rev2）

- 任务：`task-10`（W6 独立代码审核）／审核者：`r3-test-inventory`
- 仓库 `F:\codexprojects\dsl000`，HEAD `cb5d8a2`（未 commit / 未 push）
- 模式：**只读**。本报告只写 `docs/review-evidence/round2/code-review.md` 与 `target/round2-evidence/w6/**`；
  未改任何代码/测试/文档/`Cargo.toml`/`Cargo.lock`
- 审核对象：**rev2 冻结候选**（`target/round2-logs/freeze-manifest.txt`，mtime 21:03:04，19 个文件）

## 0. 冻结状态与哈希核对（含冻结外变更）

| 时刻 | 事件 | 结果 |
|---|---|---|
| 21:00:47 | 按 rev1 清单逐项 `Get-FileHash -Algorithm SHA256` | **19/19 一致**（`target/round2-evidence/w6/freeze-check.txt` 为首版记录） |
| 21:03:22 | Lead 通知 rev2（仅 `thevenin.rs` 注释改动）后重新核对 | **19/19 一致**；`thevenin.rs` = `FCF3BD88E8BF24CE9BF3ADCCD64C805FAA56169AEC9FBB2E2158A5386FB3AFAC` |

**冻结外变更如实记录（Lead 已通知并为 rev2 权威）**：`crates/circuit-backend/src/thevenin.rs`
`2D28F01E…` → `FCF3BD88…`。我做了独立比对：把 rev1 时刻保存的 `git diff` 转储
（`target/round2-evidence/w6/diff-thevenin.txt`）与 rev2 转储
（`diff-thevenin-rev2.txt`）逐行比对，**唯一差异是 map_analysis TRAN 分支的 3 行注释**
（旧："Its only effect on this adapter is the metadata below." → 新：指向 `tran_settings` /
`convert_plot`，并多 1 行；两个 hunk 的偏移随之 +1）。**行为面无差异，rev1 期间已完成的
cargo test / 产品路径复现结论对 rev2 继续有效**（rev2 后我又重跑了一次 cargo test，见 §1）。

其余全部审核引用以下哈希（均为 rev2 清单值）：`plan.rs 0D089AC3`、`elaborate.rs 61CA8FE8`、
`resample.rs A726934F`、`results/lib.rs 91FD0437`、`dataset.rs FEC7992E`、`execute.rs 9A0BD103`、
`session.rs 428CB2AA`、`cli/run.rs 4B4D32A4`、`transient_reference_regression.rs 889E7122`、
`output_interval_regression.rs 17481F51`、`source_breakpoint_regression.rs F8C9F390`、
`tran_output_interval.rs 120197BE`、`tran_option_validation.rs 3D7EF591`、
`breakpoint_study.rs 70F2EE42`、`tran_contract.rs CB6890BC`、`parser.rs E62AE862`、
`ir.rs 267C3A2D`、`limits.rs DB02EC4D`。
**不在清单内的用户可见文件**：`README.md`（21:02 已同步新契约）、`examples/rc_filter.cdsl`
（20:05，**未同步**，见 W6-3）、`docs/testing.md`（20:23）、`docs/backend-evaluation.md`（21:04）、
`_probe/src/main.rs`、`_probe/src/bin/robustness.rs`。

## 1. 实际读 / 跑的内容（命令与退出码）

| # | 命令 | 退出码 | 关键结果 |
|---|---|---|---|
| 1 | `git status --short` / `git diff` / `Get-FileHash` 逐项核对清单 | 0 | 19/19 一致；工作区 19 改 + 20 未跟踪 |
| 2 | `cargo test -p circuit-results -p circuit-session -p circuit-dsl -p circuit-backend --offline` | **0** | 15 个测试目标全绿，含 backend lib 15、adapter 21、output_interval_regression **5**、phase_regression 6、source_breakpoint_regression **6**、transient_reference_regression 4、dsl lib 84、elaborate 67、phase_syntax 6、reference_path 8、tran_option_validation **15**、results lib 83（`resample` 单测 **12**）、session lib 9、session.rs 25、tran_output_interval **5**；合计 **360**（+4 个 doc-test 目标 1 个测试）。日志：`target/round2-evidence/w6/cargo-rev2.txt` |
| 3 | `cargo clippy -p circuit-core -p circuit-dsl --all-targets --offline -- -D warnings` | **0** | 这两个 crate 干净 |
| 4 | `cargo clippy -p circuit-core -p circuit-dsl -p circuit-backend -p circuit-results -p circuit-session -p circuit-cli --all-targets --offline -- -D warnings` | **101** | **W6-1**：`resample.rs:87` `clippy::neg_cmp_op_on_partial_ord`，`circuit-results` 编译中断，其余依赖 crate 的 clippy 状态未知 |
| 5 | `cargo build -p circuit-cli --offline` | 0 | `Finished`；1 条 `linker_messages` 警告（MSVC 链接器 stdout，非代码问题，与 `docs/next-iteration-plan.md:16` 记录一致） |
| 6 | `target\debug\cdsl.exe run examples\rc_filter.cdsl --experiment response --out …\example-run --format csv` | **0** | tran1 **1019** 点；首个 `v(vin) ≥ 0.999999` 在 **t = 1 ns**；`max|v(vout)-(1-exp(-t/τ))| = 4.9218437e-6 V`；`vfinal = 0.9932620899316276 V` |
| 7 | `… run …\p1_repro.cdsl --experiment interval_1ns` / `interval_100ns` | 0 / 0 | 输出行数 **2001 / 21**；50 ns 处 `v(vin) = 1 V`（与 `output_interval` 无关）；`output_interval` 只改输出网格 |
| 8 | `cdsl check …\p2_illegal.cdsl`（`output_interval: -1.ns`） | **1** | `E_VALUE` 定位到参数 span，消息含 `output_interval`；**P2 已修** |
| 9 | `cdsl check …\budget-maxstep.cdsl`（纯 DC 源 + RC，`stop:1.s, max_step:1.ns`） | **1** | **W6-2**：`E_LIMIT` 归因"声明的源时序过细"，而该电路**没有任何波形源** |
| 10 | `cdsl check …\budget-maxstep-ok.cdsl`（同上，`max_step:1.us`，`span/step = 1e6`） | 0 | 确认边界是 `steps > 1e6`（等于 1e6 放行） |
| 11 | 源码级核对引擎：`~/.cargo/registry/src/*/thevenin-0.5.0/src/waveform.rs` grep `tstep` | 0 | `tstep` 只出现在 `eval_pulse`（`:37-38`、`:135`）与 `eval_exp`（`:75-78`、`:311-312`）；SIN/PWL 不受 `tstep` 影响 ⇒ `tran_step_for` 只统计 PULSE 是**正确的** |

> 注：`clippy` 运行与 Lead 的 rev2 注释改动无冲突（`resample.rs` 哈希未变）；`cargo test` 在 rev2 后重跑，
> 退出码 0。第 2 项的分目标计数取自同一次运行的完整日志。

## 2. 发现清单（按严重度）

### W6-1 【HIGH · 门禁回归】`clippy -D warnings` 在冻结候选上失败

```
error: the use of negated comparison operators on partially ordered types produces code that is
       hard to read and refactor, please consider using the `partial_cmp` method instead, …
  --> crates\circuit-results\src\resample.rs:87:12
87 |     if !(span > 0.0) || self.interval_s <= 0.0 {
   = note: `-D clippy::neg-cmp-op-on-partial-ord` implied by `-D warnings`
error: could not compile `circuit-results` (lib) due to 1 previous error
```

- 复现：命令 4（日志 `target/round2-evidence/w6/cargo-rev2.txt` 尾部）。
- 影响：① 上一轮门禁含 `cargo clippy --workspace --all-targets -- -D warnings → exit 0`
  （`docs/review-evidence/freeze-manifest.md` §2），本轮候选不满足；② 由于 `circuit-results`
  编译中断，`circuit-backend`/`circuit-session`/`circuit-cli` 的 clippy 结果**未知**，修完后必须重跑全量；
  ③ `cargo test` 不受影响（已 exit 0），所以 Lead 的「test + rustfmt」复核未覆盖它。
- 修复方向（由实现方决定，我不改代码）：`if span <= 0.0 || span.is_nan() || self.interval_s <= 0.0`
  或对该行加 `#[allow(clippy::neg_cmp_op_on_partial_ord)]` 并说明 NaN 语义。
  **注意保持 NaN 语义**：原写法 `!(span > 0.0)` 正是为了把 NaN 归入"无内部点"，改成 `span <= 0.0`
  会漏掉 NaN（`interior_count` 之后还有 `!(span > 0.0)` 的同类判断，见 `resample.rs:87` 与 `:67`）。

### W6-2 【MEDIUM · 触发面与诊断不准确】步数预算 `E_LIMIT` 的归因错误

- 位置：`crates/circuit-backend/src/thevenin.rs:981-1015`（`lte_controls` / `effective_step` / `steps`
  / `Code::Limit` 分支）与 `:856-872` 注释；文档 `docs/language.md:356`。
- 事实：`effective_step` 在电路含 C/L 时取 `h_max`，即用户 `max_step`（缺省时 `min(h_print, stop/50)`），
  **与"声明的源时序"无关**。实测反例（命令 9）：电路只有 `dc: 1.V` 的电压源 + R + C（**无任何
  `pulse/sin/pwl`**），`tran stop: 1.s, max_step: 1.ns` →

  ```
  error[E_LIMIT]: the declared source timings are too fine for this simulation window:
                  honouring them would need about 1000000000 solver steps, over the limit of 1000000
    --> budget-maxstep.cdsl:13:3
    = waveform bound: 0.001 s        ← 实为 span/1000 默认值，不是"声明的"边沿
    = effective step: 0.000000001 s  ← 实为 max_step
    = shorten `stop:`, set a `max_step:` …   ← max_step 已经设了
  ```
- 控制组（命令 10）：同一电路 `max_step: 1.us`（`span/effective_step = 1e6`，不满足 `> 1e6`）→ exit 0。
- 影响：a) 触发面比 `docs/language.md:356`（"声明的边沿相对仿真窗口过细"）与
  `thevenin.rs:855-870` 的描述更宽：**纯 `max_step` 配置也会被这条"源时序"错误拒绝**（这是本轮新增的
  硬拒绝，旧行为会尝试求解）；b) 诊断的 message/context/note 三处归因与建议均指向错误的旋钮；
  c) `steps` 用 LTE 分支的 `h_max` 当作精确步数，是下界（LTE 可取更小步），"about N solver steps" 只应
  当作预算估计。
- 建议（不阻塞，除非 Lead 认为触发面需收窄）：把 message 改为"该 `tran` 配置需要的求解步数超过预算"，
  并在 message/context 里区分 `waveform bound` 与 `max_step` 两个来源；`docs/language.md:356` 同步。

### W6-3 【MEDIUM · 用户可见示例与代码行为矛盾】`examples/rc_filter.cdsl` 未随修复更新

- 位置：`examples/rc_filter.cdsl:7-28`（mtime 20:05，早于修复冻结；**不在 rev2 清单内**）。
- 注释仍写："适配层交给引擎的 step 是 `stop/1000 = 500 ns`（`thevenin.rs:772-775`）"、
  "声明的 `rise: 1.ns` 被展宽为 500 ns"、"实测 1015 样本、`max |v(vout) - (1-exp)| = 2.491963e-3 V`、
  与匹配 500 ns 斜坡差 `6.278341e-7 V`"。
- 我对**冻结候选**实测（命令 6）：**1019** 点、首个 `v(vin) ≥ 0.999999` 在 **1 ns**（声明值被兑现）、
  `max |v(vout)-(1-exp(-t/τ))| = 4.9218437e-6 V`、`vfinal = 0.9932620899316276 V`；
  `cdsl capabilities` 的 step 现在是 `min(span/1000, min(rise,fall,period)) = 1 ns`，
  `thevenin.rs:772-775` 已不再是该映射。⇒ 该注释段整体**不成立**。
- `docs/review-evidence/round2/cli-qa.md:485-487` 已独立记录同一问题（判定"不成立"），但文件未改。
  Lead 已说明由文档代理稍后更新：**此项按"已知未修项"记录，不改代码**。
- 同文件 `:26-28`（`max_step` 是内部步长、轴非均匀、`avg/rms` 需积分）**仍然正确**。

### W6-4 【LOW】退化轴时 `output_interval` 被静默忽略

- 位置：`crates/circuit-results/src/resample.rs:152-159`（`Axis::Time` 且 `from_axis` 返回 `None` →
  `Ok(dataset.clone())`），测试 `:425 a_degenerate_axis_is_returned_unchanged` 把这个沉默路径钉住。
- 事实：原始瞬态点少于 2 个（或 `last <= first`）时，用户显式写的 `output_interval:` 被丢弃：
  无诊断、无 `tran.output_grid` / `tran.output_points` 元数据（对比正常路径会写入，
  `resample.rs:207-211`），调用方无法从结果里看出请求未被兑现。`docs/language.md:399` 记载了行为
  但没有可观测信号。触发条件苛刻（单点瞬态），影响低。

### W6-5 【LOW · 文档引用错误】`docs/language.md §6` 应为 §5.3

- `crates/circuit-results/src/resample.rs:18`：`//! # The grid contract (docs/language.md §6)`；
  `crates/circuit-dsl/src/elaborate.rs:2451`：`// implemented by resampling … (docs/language.md §6)`。
- 实际：网格契约在 `docs/language.md:385-399` 的 **§5.3 输出采样与输出网格**；`:416` 的 **§6 是
  "CLI 与结果"**。纯引用漂移，改一个字符即可。

### W6-6 【LOW · 能力 note 过时】`cdsl capabilities` 的输出轴描述不完整

- 位置：`crates/circuit-backend/src/thevenin.rs:110-112`
  （"Transient output time points are solver-chosen and non-uniform; `max_step` bounds the internal
  step, not the output interval."）。
- 修复后：声明了 `output_interval` 时，**交付/导出的**时间点恰恰是均匀网格（本轮实测 21 点 / 2001 点）。
  该 note 未提这一点，用户会得出"给了 `output_interval` 也得不到均匀输出"的错误结论；它是
  `cdsl capabilities` 的用户可见文本（README:178 引用同一段）。`cli-qa.md:499-501` 已独立记录；
  对照 `plan.rs:150-166` 的文档注释**已经**写对，即"代码文档已更新、capability note 未更新"。

### W6-7 【LOW · 级联诊断】非法 `output_interval` 会多报一条 `E_ARGUMENT`

- 实测（命令 8）：`cdsl check p2_illegal.cdsl` 输出
  ① `error[E_VALUE]: output_interval: must be a finite number greater than zero`（span 落在参数上，准确）
  ② `error[E_ARGUMENT]: experiment `illegal` declares no analysis`。
  ②是 `tran_spec` 返回 `None` 的直接后果（与既有 `max_step` 校验路径行为一致，非本轮引入的新缺陷），
  但对用户是误导性的第二错误。`tran_option_validation.rs:685-694` 正是利用 ②的存在来证明"没有回退默认值"
  （结构性断言的依据），因此**不建议删除 ②**，可考虑只在存在其它错误时抑制。

### W6-8 【INFO · 审核面陈旧】`docs/review-evidence/round2/test-inventory.md` 是修复前快照

- 该文件（`task-3` 的产物，20:39）仍以**修复前**的测试指纹为准，例如
  `test-inventory.md:44` 与 `:287` 引用 `transient_reference_regression.rs:684
  declared_rise_below_output_interval_is_clamped_to_the_output_step`——该测试已被按新契约重写为
  `:725 declared_rise_below_output_interval_is_not_widened`（`transient_reference_regression.rs:693-829`）。
- 文件自身声明了"以冻结版 + 正在写入的 diff 为准、修复后需重核"，但读者仍可能被旧行号误导。
  建议在文件头加一行"修复前快照，已被 `code-review.md` 取代"，或由 Lead 在汇总里说明。

## 3. 逐项结论（对应任务的 7 条要求）

### 3.1 语义分离是否成立 —— **成立（PASS）**

- 全仓 grep `output_interval`（190 处命中，逐条归类）：进入求解器的读者**只有**
  `thevenin.rs:1039` 的元数据写入（`tran.output_interval`）；`map_analysis` 的 TRAN 分支
  （`thevenin.rs:808`）只用 `print_step_for(circuit, spec)`，**不再读 `spec.output_interval`**；
  旧写法 `.filter(|s| *s > 0.0).unwrap_or_else(|| span / 1000.0)` 已从全仓消失
  （grep `filter(|` 无此命中）。
- `h_print` 计算入口**唯一**：`thevenin.rs:909 fn tran_step_for`（含 `:949 default_print`、
  `:950 h_print`）；三个调用点共享同一结果——`:808`（映射）、`:220`（validate 能力检查）、
  `:1024 tran_settings`（元数据），不会出现"校验值 ≠ 实际交给引擎的值"。
- 其余保留的默认/回退都是**有意的且在文档里**：`default_print = span/1000`（`:949`）、
  `h_max` 缺省 `min(h_print, stop/50)`（`:984-986`，逐字镜像引擎 `transient.rs:799`）、
  非 PULSE 波形不参与 bound（`:915-920`，我已到引擎源码核对：`waveform.rs` 的 `tstep` 只出现在
  `eval_pulse`/`eval_exp`，而产品 IR 只有 `Pulse/Sin/Pwl`，`circuit-core/src/ir.rs:211-240`）。
- 端到端判别（命令 6–7）：同一电路仅改 `output_interval` → 原始解 50 ns 处 `v(vin)=1 V` 不变；
  P1 旧反例 0.5002375 V 不再出现；`p1_repro.cdsl` 输出 2001 / 21 行只差采样密度。
  最强断言见 §4。

### 3.2 能力错误路径 —— **覆盖完整（PASS），但归因文案见 W6-2**

- `validate` 覆盖 `check` 与 `run`：`crates/circuit-cli/src/check.rs:58` 调用
  `backend.validate`；`thevenin.rs:325` 在 `run` 入口再次调用 ⇒ 两条产品命令都拦。
- 绕过 DSL 的 IR 直构也被拦：`output_interval_regression.rs:665-774` 直接用 `be().run(...)`
  （不经 validate 的显式调用）验证"声明零边沿 → `E_UNSUPPORTED`"与"1 ps 边沿 + 10 us 窗 → `E_LIMIT`"，
  因为 `map_analysis → print_step_for → tran_step_for` 自身返回 `Err`（`:909-1015`）。
- `max_step` 非有限/非正：`elaborate.rs:2433-2444`（DSL）+ `thevenin.rs:207-222`（IR/后端）。
- 零边沿（PULSE `rise/fall/period` 为 0 或非有限）：`thevenin.rs:923-945`（`for (name, q) in
  [("rise", rise), ("fall", fall), ("period", period)]`，消息在 `:929`）→ `E_UNSUPPORTED`，
  消息点名 `{name}` 与源名；`docs/language.md:355` 与之一致。
- 超预算：`thevenin.rs:994-1015` → `E_LIMIT`，`steps` 用 `!is_finite() || > MAX_PRINT_STEPS`，
  **NaN/inf 会被安全地归入拒绝**（不会漏到求解器）。
- IR 直构的**非法 `output_interval`**（负/零/NaN）不经 DSL：后端不读它（设计如此），
  到会话层 `output_view → resample_time` 会返回 `E_VALUE`（`resample.rs:142-150`，单测 `:486-493` 覆盖
  0/-1/NaN/inf）。**未验证**的是"IR 直构 + 非法值"穿过 `execute` 的端到端用例（现有负值用例都走 DSL 层）。

### 3.3 重采样正确性 —— **正确（PASS）**

| 检查点 | 结论 | 证据 |
|---|---|---|
| 端点 | 首点=原始首点、末点=原始末点（位相等） | `resample.rs:66-74, 118-129`；单测 `:300-312`、session `:344-350` |
| 等间隔 | 内部点 `t0+k·iv`，末段可短于 `iv`；严格递增 | 同上；session `:351-360` |
| 不越界外推 | `interpolate` 对 `t <= t0` / `t >= tN` 取端点值 | `resample.rs:239-247`；单测 `:327-334` |
| 单点/退化轴 | 原样返回（见 W6-4 的静默问题） | `resample.rs:67-69, 155-159` |
| 复数信号 | 实/虚部分量线性插值 | `resample.rs:189-200`；单测 `:444-469` |
| 规模限制 | 输出值总数（点数 × 信号数，`max(1)`）超 `Limits::max_result_values` → `E_LIMIT`，不截断 | `resample.rs:161-182`；单测 `:397-406`；session `:574-626`（`toofine` 1 ps → 2e6 点被拒，粗档仍 21 点通过） |
| 元数据自洽 | 保留后端设置 + `tran.output_grid=resampled-linear` + `tran.output_points`；不重复写 `tran.output_interval` | `resample.rs:205-211`；单测 `:472-483` |
| `point_count` 与实际网格一致 | **同一函数**（`interior_count`）同时用于计数与建网格；`points()` 容量即 `point_count()` | `resample.rs:85-129`；单测 `:372-394`、`:337-369` |
| O(n²)/超大分配 | 插值用单向游标（`interpolate` 的 `lo` 只随升序输出前进）⇒ O(n+m)，注释自称 O(n log n) 是保守表述；内存上界 = `max_result_values`（默认 5e7 值 ⇒ 单信号约 4e8 B 量级，与原始数据集同量级，非无界） | `resample.rs:233-267`；`limits.rs:38` |
| 理论边角（不构成缺陷） | `point_count()` 的 `interior + 1 + u64::from(...)` 若 `interior_count()` 返回 `u64::MAX-1/MAX-2` 会溢出，但 `== u64::MAX` 有显式守卫（`:107-111`），而 `MAX-1` 在 f64 估计下不可达（`estimate` 在 2^64 邻域是 2048 的倍数）。**仅记录，不建议改** | 分析 + `:85-111` |

### 3.4 测量语义与 `datasets`/`output_datasets` 混用 —— **无混用（PASS）**

- 测量在重采样**之前**、在 `datasets`（raw）上计算：`execute.rs:136-139`（`evaluate_measures(&plan, &datasets)`，
  随后才 `output_view`）。session 测试 `tran_output_interval.rs:446-518` 用**位相等**证明 fine/coarse 的
  `avg/rms/max/min` 完全一致，并给出反证控制：在 21 点输出视图上直接算 `avg: v(:vin)` 与 raw 值相差 >1 mV，
  且 raw 值等于解析的 `1 - (0.5·10ns)/2us = 0.9975 V`。
- 全部 `RunOutcome` 使用点（grep 全仓）：
  | 位置 | 用途 | 视图 |
  |---|---|---|
  | `circuit-cli/src/run.rs:105-120` | 写盘 | `output_datasets` ✅ |
  | `circuit-cli/src/run.rs:157`（`print_summary` → `summaries()`） | 打印点数 | `output_datasets`（`execute.rs:49-53`）✅ |
  | `circuit-cli/src/run.rs:149-153` | backend 名/版本 | `datasets[0]` ✅（两视图的 `BackendInfo` 名/版本相同，resample 只追加 settings） |
  | `circuit-cli/src/run.rs:163` | 打印 measure | `measures`（raw 计算）✅ |
  | `circuit-session/src/session.rs:599` | backend 名/版本 | `datasets[0]` ✅ 同上 |
  | `circuit-session/src/session.rs:607` | 摘要 | `summaries()` → output ✅ |
  | `circuit-session/src/session.rs:615-632` | REPL 逐信号展示 | `output_datasets` ✅ |
  | `circuit-session/src/session.rs:651` | REPL `--out` 写盘 | `output_datasets` ✅ |
  | 其它 `datasets` 命中 | 参数扫描 `stitch`/`sweep_experiment`、后端自身 `SimulationResults.datasets` | 与视图无关 ✅ |
- 实测一致性：CLI 汇总 "tran1: 1019 time points" 与 CSV 数据行 1019 一致；P1 100 ns 档汇总 21 点 / CSV 21 行。
- **能力边界（`cli-qa.md:297` 已记录，我确认）**：`check` 不重采样，所以"输出网格超预算"的 `E_LIMIT`
  只有 `run` 能报；`check` 能报的是 DSL 层 `E_VALUE` 与后端能力 `E_UNSUPPORTED`/`E_LIMIT`。这是分层契约，不是缺陷。

### 3.5 失败会真实变红吗 —— 抽查结论

**最强（鉴别力最高）**
1. `crates/circuit-backend/tests/output_interval_regression.rs:410 output_interval_does_not_change_the_solved_trace`
   —— 三个只差 `output_interval` 的运行做**逐位**比较（`assert_trace_bits_equal`，`:320-362`，
   用 `to_bits` 避免 `-0.0 == 0.0` 掩盖符号变化）；若有人把映射改回去，粗档的 `v(vin)` 会变成
   0.5002375（历史实测值，写在同一文件 `:467-471`），比较立刻变红。不可同源自证：比较对象是**另一次运行**。
2. `transient_reference_regression.rs:725 declared_rise_below_output_interval_is_not_widened`
   —— 三条读数：① 首个 ≥1 ps 的采样点 `v(vin)` 必须 `== 1.0`（起点步上界 `h_max/400`，无参考解也能判）；
   ② **旧契约参考** `T_eff = max(1 ps, 500 ns)` 必须违规 >100 点且最大误差 > 1.01e-3 V（保证旧契约下必红）；
   ③ 声明 1 ps 边沿下逐点满足 §17。这直接反驳了上一轮被替换掉的断言方向，是"不能靠旧语义通过"的设计。
3. `crates/circuit-session/tests/tran_output_interval.rs:446 measurements_are_computed_on_the_raw_grid`
   —— 位相等 + 反证控制 + 解析值 0.9975 V 三重约束，恒真不可能。

**最弱（仍有效但覆盖薄）**
1. `crates/circuit-results/src/resample.rs:425 a_degenerate_axis_is_returned_unchanged`
   —— 只断言"退化轴原样返回"，把 §W6-4 的**静默忽略**路径钉成合法行为；没有任何"必须给出
   诊断/元数据"的断言，所以"用户请求被悄悄丢弃"这类回归它抓不到。
2. `crates/circuit-results/src/resample.rs:337 the_interval_becomes_the_output_point_count`
   —— 点数用 `(21..=22)` 区间断言（浮点末位理由充分），但对"网格是否真按 `iv` 生成"只有
   `≤ iv + 1e-21` 的上界；若末段被换成更粗的间隔（例如内部点计数整体偏小 1 个数量级，
   只要不超过 22），它仍会通过。绝对点数由 session 层 `:343`（`== 21`）与 `:420`（`== 2001`）补上，
   所以整体覆盖不弱，单点看这条最松。
3. （参考）`source_breakpoint_regression.rs:1021` 的 `assert_eq!(limitation_rows, 2)` 与
   `:1102` 的 `fit.violations > 0` 是**特征化钉子**（"限制仍在"），不是验收断言；文件与 README
   都明确标注为 LIMITATION，且失败信息要求"更新而不是删除"。符合上一轮"不得隐藏失败样本"的约束。

**恒真/自证排查**：未发现空集合通过（每条循环断言前都有 `len > 100 / > 3000` 等形状守卫）；
未发现"阈值远大于真实误差"（§17 判据逐点断言，实测 max|err| 4.999167e-7 V vs allowance 1e-5 量级，
余量约 20x 而非数量级虚设）；未发现比较对象与实现同源（参考解有独立自检：
RK4、ODE 残差、BE 闭式、解析式三路互证，`source_breakpoint_regression.rs:787-870`）。

### 3.6 范围与回归风险 —— **未超范围（PASS）**

- 改动面：9 个产品文件（`plan.rs` 文档、`elaborate.rs` 校验+注释、`thevenin.rs` 映射/能力/元数据、
  `resample.rs` 新模块、`results/{lib,dataset}.rs` 导出与访问器、`session/{execute,session}.rs` 双视图接线、
  `cli/run.rs` 写盘视图）+ 6 个新测试目标 + 2 个 `_probe` bin。**没有**换后端、没有新增器件/语法、
  没有 GUI/绘图/LSP，符合 `docs/next-iteration-plan.md:51-86` 的任务 A/B 边界。
- 公共 API 变化（均为**增量**）：
  | 变化 | 调用方影响 |
  |---|---|
  | `RunOutcome.output_datasets: Vec<Dataset>`（`execute.rs:37-39`） | 唯一构造点 `execute.rs:141`；仓内所有读者已同步（§3.4）。**外部**用结构体字面量构造 `RunOutcome` 的代码会编译失败（仓内无此用法）；纯读者不受影响 |
  | `BackendInfo::setting(&self, key) -> Option<&str>`（`dataset.rs:454`，doc 注释 `:452-453`） | 纯新增访问器，无破坏 |
  | `circuit_results::resample` 模块 + `OutputGrid`/`resample_time` 导出（`lib.rs:60, 69`） | 纯新增 |
  | `write_datasets` 签名不变，语义变为"写传入的视图"（`run.rs:105-120`、`session.rs:651`） | 2 个调用点均已传 `output_datasets` |
  | `TranSpec` 字段集未变，仅文档（`plan.rs:147-175`） | 0 破坏（上一轮 5 个构造点无需改） |
- `examples/rc_filter.cdsl`：注释与行为矛盾（W6-3），**必须由文档代理更新**；程序本体（电路/实验语句）仍正确，
  `e2e.rs` 用例凭 1e-2 容差继续通过。
- 调试残留：`grep TODO|FIXME|XXX|HACK|#[ignore]|dbg!|unimplemented!|todo!` 在产品代码 **0 命中**
  （仅 `source_breakpoint_regression.rs:60,1100` 的文档注释提到"没有使用 `#[ignore]`"）。
  未发现被注释掉的断言；`clippy` 唯一问题是 W6-1。
- `_probe/src/bin/tran_contract.rs`（801 行，case A/A2/B/C/D，exit 0/1 语义在 `:784-800`）与
  `breakpoint_study.rs`（1471 行）在**独立工程**里，不进 workspace；`cargo test --workspace` 不构建它们。
  `tran_contract.rs` 只覆盖 PULSE，但 §3.1 的引擎源码核对表明 SIN/PWL 不需要该 bound，故不是缺口。

### 3.7 文档与代码一致性抽查 —— **基本一致，两处小漂移**

| 引用 | 核对结果 |
|---|---|
| `docs/language.md:341-357, 385-414`（三概念、print step 规则、网格契约、三条性质、元数据键名） | 与 `thevenin.rs:905-1015`、`resample.rs:18-41, 205-211` **逐条一致**；`:356` 的触发面描述见 W6-2 |
| `README.md:222-235`（新契约、1019 点、`4.9218e-6`、断点表 3129/1629/729/309 与 0/0/3/250 超限） | 与我的实测（命令 6）和 `breakpoint-evidence.md:72-75` 一致 ✅ |
| `docs/backend-evaluation.md:302, 338, 393, 462`（输出采样不传、断点回归、`check`/`run` 分层） | 与代码一致 ✅ |
| `docs/review-evidence/round2/`：`cli-qa.md:461-533`（1019 行、4.921844e-6、元数据键、`check` 不重采样） | 我用独立构建的 `cdsl.exe` 复算，**数字全部复现** ✅ |
| `cli-qa.md:384` → `resample.rs:113-129`（网格构造） | 行号精确 ✅（`points()` 在 `:118-129`） |
| `breakpoint-evidence.md:15` → `source_breakpoint_regression.rs` 1122 行 / `F8C9F390` | 现文件 **1191 行**、哈希 `F8C9F390` ✅（哈希一致，行数在报告后又有追加，属正常漂移） |
| `cli-qa.md:499-501`（capability note 过时） | 独立确认为真（W6-6） |
| `cli-qa.md:485-487`（示例注释不成立） | 独立确认为真（W6-3） |
| `kernel-contract.md:474, 598` 引用 `thevenin.rs:771-775` | 该文件显式标注"我读取时刻的快照，该文件可能正被修改" ✅ 诚实披露；当前行号已漂移 |
| `product-path.md`（设计快照，`execute.rs:120` 等行号） | 属实现前分析，文件自身定位为快照；其中 `:301` 预言的"摘要读 `datasets` 会与 CSV 矛盾"已被实现修正（§3.4） |
| `test-inventory.md`（`task-3` 预修复快照） | **已陈旧**，见 W6-8 |

## 4. 结论

**NEEDS_FIX**（1 项必修门禁 + 1 项建议修文案 + 1 项文档收尾；功能语义本身经独立复现**成立**）

- **必修（W6-1）**：`crates/circuit-results/src/resample.rs:87` 触发 `clippy::neg_cmp_op_on_partial_ord`，
  `cargo clippy --workspace --all-targets -- -D warnings` exit 101；修后必须重跑**全量** clippy
  （依赖 crate 的检查结果目前未知）。这不影响 `cargo test`（exit 0）与产品行为。
- **建议修（W6-2）**：步数预算 `E_LIMIT` 的触发面/文案与"声明源时序"不符（已给纯 DC 源反例与控制组）；
  至少在诊断与 `docs/language.md:356` 里区分 `max_step` 与 waveform bound 两个来源。
- **文档收尾（W6-3、W6-5、W6-6、W6-7、W6-8）**：`examples/rc_filter.cdsl:7-28` 必改（用户可见且与实测矛盾）；
  `resample.rs:18`/`elaborate.rs:2451` 的 §6→§5.3；`thevenin.rs:110-112` capability note；
  级联 `E_ARGUMENT`；`test-inventory.md` 标注为已被本报告取代。这些都不在 rev2 清单内，
  不影响冻结哈希，`check`/`run`/`cargo test` 全部 exit 0。
- **功能层面 PASS 的依据**：语义分离（grep + 单入口 + 位相等实测）、能力错误路径（`check`/`run`/IR 直构三路）、
  重采样契约（端点/等间隔/不外推/复数/限额/`point_count` 一致）、测量 raw 语义与 CLI/REPL 双视图一致，
  均由本报告 §3.1–§3.4 的源码行号 + 实跑数据支持。

## 5. 未验证项（本报告不声称）

1. **`cargo test --workspace` 全量**未跑：只跑了 4 个 crate（360 tests + 1 doc-test，exit 0）。
   按测试目标计数推算全量应约 **456**（413 + 43 新增：5+6+5+15+12），**未实测**；
   `circuit-core`（58）与 `circuit-cli`（38，含真进程 e2e/repl）本轮未跑；`cargo fmt --check` 未跑
   （Lead 已自跑并报告 exit 0）。
2. **clippy 全貌未知**：`circuit-backend`/`circuit-session`/`circuit-cli` 因 `circuit-results` 编译中断
   未被 clippy 检查；修 W6-1 后可能暴露更多告警。
3. **`_probe` 两个新 bin 未编译/未运行**（会写 `_probe/target`，超出本任务写集）；
   `breakpoint_study.rs` / `tran_contract.rs` 的 exit 码语义只按源码阅读（`:784-800`）确认。
4. **未验证 SIN/PWL 的端到端边沿语义**：我只核对到"引擎 `waveform.rs` 的 `tstep` 只作用于 PULSE/EXP"，
   `_probe/tran_contract.rs` 也只覆盖 PULSE；产品 IR 无 EXP，故风险低但未实测。
5. **未验证 IR 直构 + 非法 `output_interval` 穿过 `execute` 的端到端用例**（DSL 层与 `resample` 单测已覆盖两侧）。
6. **未验证** 退化轴（单点瞬态）在生产路径上是否可达；W6-4 的触发条件仅从代码推断。
7. **未复现** `_probe`/`breakpoint_study` 报告的引擎级数字（`h1 = min(2*h_before, h_max)*0.1`、
   BE/TRAP 比值等）；这些由 `breakpoint-evidence.md` 与 `source_breakpoint_regression.rs:1102-1155`
   自行断言，我只核对了产品层可观察的部分（测试 exit 0、README 数字与我的实测一致）。
8. **未做**性能/内存测量；§3.3 的规模结论基于 `Limits::max_result_values = 5e7`（`limits.rs:38`）的静态上界。
9. 审核后若再有写入，需重新核对清单（本报告以 21:03:22 的 19/19 一致为准）。

## 6. 本次写入的文件

- `docs/review-evidence/round2/code-review.md`（本报告）
- `target/round2-evidence/w6/`：`freeze-check.txt`（rev1 哈希核对）、`diff-small.txt`、`diff-thevenin.txt`、
  `diff-thevenin-rev2.txt`（冻结外变更比对）、`cargo-rev2.txt`（cargo test + clippy 日志）、
  `clippy-core-dsl.txt`、`budget-maxstep.cdsl` / `budget-maxstep-ok.cdsl`（W6-2 反例与控制组）、
  `example-run/`、`p1-1ns/`、`p1-100ns/`（产品路径复现 CSV）
- **未** commit / push / checkout / reset；**未**修改任何产品代码、测试、文档或 manifest。

## 7. rev3 复核（返修后聚焦复核，2026-09-18 21:15–21:22）

返修请求：关闭 W6-1（clippy 门禁）与 W6-2（步数预算归因），并检查 rev3 是否引入新问题。
**rev3 清单**：`target/round2-logs/freeze-manifest.txt`（21:15:03，**16 个文件 + `target/debug/cdsl.exe` 二进制哈希**）。

### 7.1 哈希

- 21:15:35 与 21:17:56 两次逐项核对：**16/16 文件一致，0 drift**；二进制 `target\debug\cdsl.exe` = `5B688194…` **与清单一致**。
- 相对 rev2 的变化与 Lead 通知一致：`thevenin.rs` `FCF3BD88…`→`33E309BE…`、`resample.rs` `A726934F…`→`29405F1B…`、
  `elaborate.rs` `F0F17092…`；w3 三个文件仅 rustfmt（`source_breakpoint_regression.rs 2F1BDAF4…`、
  `breakpoint_study.rs 1197516B…`、`tran_contract.rs 9596B248…`）。
- **rev2 清单里有、rev3 清单里没有**的 5 个文件我另行核对，仍与 rev2 值一致：
  `parser.rs E62AE862`、`ir.rs 267C3A2D`、`limits.rs DB02EC4D`、`transient_reference_regression.rs 889E7122`、
  `tran_output_interval.rs 120197BE` ⇒ 只是清单覆盖面收窄（rev3 补了二进制哈希），不是漏检。
- 我用 rev2/rev3 两次 `git diff` 转储（`diff-thevenin-rev2.txt` / `diff-thevenin-rev3.txt`）做 `Compare-Object`，
  确认 `thevenin.rs` 的 rev2→rev3 增量**只有**：capability note 补句（W6-6）、E_LIMIT 归因判定 + 抽出
  `effective_step_for`、消息/context 改写。除此之外无隐藏改动。

### 7.2 门禁（我自己跑的真实退出码）

| 命令（rev3 树） | 退出码 | 证据 |
|---|---|---|
| `cargo test --workspace --offline` | **0** | 24 条 `test result:`，求和 **456 passed / 0 failed**；`rev3-gate.txt` |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | **0** | 同上 |
| 上一条**强制重检**（先删 `target/debug/.fingerprint/circuit-*`，迫使 6 个 workspace crate 重跑 clippy） | **0** | `Checking circuit-*` 6 次、0 error、`rev3-clippy-forced.txt` |
| `cargo fmt --all -- --check` | **0** | `rev3-gate.txt` |
| `cargo fmt --manifest-path _probe/Cargo.toml -- --check` | **0** | 同上 |

### 7.3 W6-1 —— **已关闭**

- 代码：`resample.rs:86-91` 现为
  `if !span.is_finite() || span <= 0.0 || self.interval_s <= 0.0 { return 0; }`。
  **NaN 语义保持**：`!span.is_finite()` 覆盖 NaN（旧 `!(span > 0.0)` 对 NaN 同样返回 0），
  NaN 仍归零 ⇒ `point_count` 不变、不会误判成"巨大网格"。
- 门禁：全量 clippy exit 0，且**删除 fingerprint 强制重检**后 exit 0 ⇒ 不是陈旧缓存导致的假绿。
- 附带（新发现 R4，INFO）：对 **±inf 的 span**，行为与 rev2 不同——rev2 走 `estimate = inf → u64::MAX` 得到
  `E_LIMIT`，rev3 走 `!span.is_finite() → 0` 得到 **2 点网格且不报错**。
  `Dataset::validate`（`dataset.rs:518-587`）不检查轴样本是否有限，所以原理上可达；实际只可能来自
  求解器返回非有限时间点（不应发生）。无测试覆盖，建议按需加一条非有限轴用例，**不构成阻塞**。

### 7.4 W6-2 —— **已关闭**

- 代码：`thevenin.rs:983`（`effective_step`）、`:995-998`（归因判定）、`:1000-1021`（消息/contexts）、
  `:1034-1047`（`fn effective_step_for`，LTE 判定集中一处，映射与归因共用同一函数，不会互相漂移）。
- **逻辑正确性（静态）**：`h_print = min(bound, default_print) ≤ default_print`，
  故 `effective_step(h_print) ≤ effective_step(default_print)` ⇒ `steps ≥ steps_without_waveform`；
  条件 `steps > 1e6 && steps_without_waveform ≤ 1e6` 恰好表示"**去掉波形约束就负担得起**"，
  即超预算确实由声明波形引起 ⇒ **不会误拒**。`!steps.is_finite()` 保留为独立兜底（NaN/inf 一律拒绝）。
- **实测矩阵**（全部 `check`，不求解；`rev3-w6-2-matrix.txt`，6/6 与预期一致）：

  | 用例 | 配置 | exit | 判定 |
  |---|---|---|---|
  | A0 | 纯 DC 源 + RC，`stop:1s, max_step:1ns`（W6-2 原反例） | **0** | 旧 rev2 为 exit 1 且归因错误 ⇒ 误拒已消除 |
  | A1 | 同 A0，`max_step:1us`（控制组） | 0 | 一致 |
  | B | PULSE `rise/fall=1ps`，`stop:1s`，无 `max_step` | **1** | `E_LIMIT`，contexts = `declared waveform timing: 1e-12 s` / `solver step: 1e-12 s` / `effective step: 1e-12 s`，消息 "the declared source rise/fall/period is too fine…"，note 不再建议"set a max_step" |
  | C | 同 B，`max_step:1us`（steps = 1e6，未超） | 0 | 边界 `> 1e6` 正确 |
  | D | 同 B，`max_step:100ns`（steps = 1e7，但无波形约束同样 1e7） | 0 | **漏拒检查**：正确不把 max_step 的账算到波形上 |
  | E | 纯电阻（无 C/L）+ `rise=1ps` + `stop:10us` | **1** | 与 `output_interval_regression.rs:665-774` 的产品断言一致 |

- **残留在案（R2，INFO/LOW，非缺陷）**：只要设了 `max_step` 且电路含 C/L，`steps == steps_without_waveform`，
  该守卫永不触发 ⇒ `PULSE rise=1ps + stop:1s + max_step=1ns` 这类"1e9 步"的配置 `check` 通过（旧 rev2 会拒）。
  这是代码注释里写明的取舍（"the user's own request"），与 round-2 之前的行为一致；但用户没有运行期
  步数上限的提示，建议在 `docs/language.md §7`（现已在 `:519` 附近说明 E_LIMIT 归因）保留一句"纯 `max_step`
  超预算不报错、也不会被截断"的说明。`docs/testing.md:311` 已如实记录该行为。

### 7.5 其余 W6 项状态

| 项 | 状态 | 依据 |
|---|---|---|
| W6-3 示例注释 | **已关闭**（由文档代理完成） | `examples/rc_filter.cdsl:7-31` 已改写为新契约；我复核其数字：1019 点、`v(vin)` 在 1 ns 到 1 V、对理想阶跃 4.921844e-6 V —— 与我 rev2/rev3 实测一致；`C(1ns) = -5.000007e-6 V` 算式自洽 |
| W6-5 §引用 | **已关闭** | `resample.rs:18` 与 `elaborate.rs:2451` 均为 `§5.3` |
| W6-6 capability note | **已关闭** | `thevenin.rs:110-112` 补了重采样句；`README.md:178` 的 `capabilities` 快照已同步该新句；`e2e.rs::capabilities_reports_the_backend` 只断言三个子串，不受影响 |
| W6-4 退化轴静默 | **未变** | 仍为 LOW（记录在 §2，不阻塞） |
| W6-7 级联 E_ARGUMENT | **未变** | LOW（有意保留，`tran_option_validation.rs:685-694` 依赖它） |
| W6-8 round2 证据陈旧 | **未变** | INFO（`docs/review-evidence/round2/*.md` 由 Lead 决定是否加 superseded 注记） |

### 7.6 rev3 新发现

- **R3【LOW · 测试覆盖缺口，建议补】** W6-2 的归因语义**没有任何自动化断言**：
  `grep 'declared waveform timing|steps_without_waveform|too fine for this simulation' crates` 只命中
  `thevenin.rs` 源码，未命中任何测试；唯一的 E_LIMIT 产品测试
  （`output_interval_regression.rs:665 an_edge_that_cannot_be_honoured_is_refused_not_widened`）
  只断言 `Code::Limit`（`:771-774`），**在 rev2 的错误归因下同样会通过**（该用例两种实现都拒绝），
  因此它不具备判别力。`docs/testing.md:310` 把"消息含归因上下文"归到该测试名下，
  与测试实际断言**不符**（文档高估了覆盖）。
  建议补两条最小断言（可直接进 `output_interval_regression.rs` 或 `tran_option_validation.rs`）：
  ① 纯 DC + RC + `stop:1s, max_step:1ns` → `run/check` **不报** E_LIMIT（反向）；
  ② 声明 `rise:1ps` + `stop:1s`（无 `max_step`）→ E_LIMIT 且消息含 `declared waveform timing`（正向）。
- **R4【INFO】** 见 §7.3 末段（inf span 行为变化，无测试）。
- **R5【INFO】** 清单覆盖面：rev3 不再钉 `parser.rs`/`ir.rs`/`limits.rs`（已另行核对未变），新增二进制哈希；
  `_probe/src/main.rs`、`robustness.rs` 仍未进清单。
- **无**其他新问题：`effective_step_for` 为私有函数，本轮无公共 API 变化；`write_datasets`/`RunOutcome`/
  `BackendInfo` 均未再动（`dataset.rs`/`execute.rs`/`run.rs` 哈希与 rev2 相同）；工作区测试/门禁全绿。

### 7.7 rev3 复核结论

**PASS** —— W6-1 与 W6-2 **均已关闭**（有强制重检的 clippy exit 0、6/6 归因矩阵与静态逻辑证明），
W6-3/W6-5/W6-6 亦随之关闭；rev3 **未引入新的功能性缺陷**。
唯一建议项是 **R3（补 2 条归因回归断言 + 修正 `docs/testing.md:310` 的覆盖表述）**，
属测试强度提升，不阻塞冻结；R2/R4/R5 为在案观察。

### 7.8 rev3 未验证项

1. `_probe` 四个 bin 我未运行（会写 `_probe/target`，超出写集）；Lead 报告全部 exit 0，未独立复核。
2. `cargo test --workspace` 我跑了 exit 0/456，但未用 `nextest`、未跑 release profile。
3. w3 三个文件的"仅 rustfmt、行为不变"由 Lead/w3 的 stdout 逐字节比对保证；我通过
   `cargo test --workspace` 全绿 + 哈希与 rev3 一致间接确认，未做逐字节 diff。
4. 非 Windows 平台、性能/内存、R2 的"1e9 步运行是否可用"均未验证。
5. 本复核以 21:17:56 的 16/16 + 二进制一致为准；此后的写入（README/docs 仍在更新）
   不影响已钉的 16 个文件哈希。
