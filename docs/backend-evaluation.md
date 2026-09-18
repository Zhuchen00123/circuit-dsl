# 后端选型评估（阶段 0）

> 状态：**已完成**，结论已用于实现。
> 评估对象：Thevenin 0.5.0（主选）。
> 环境：Windows 10.0.26200 x64，rustc/cargo 1.98.1，`stable-x86_64-pc-windows-msvc`。
> 评估日期：2026-09-18。

## 1. 结论

**采用 Thevenin 0.5.0 作为唯一后端。**

它满足项目对后端的全部硬性要求：纯 Rust、可从 Rust 数据结构直接构造电路、
OP/DC/AC/TRAN 四种分析齐备且数值正确、失败可识别。因此**不需要**进入自研内核路径
（brief §10），也不需要 ngspice 外部进程后备方案。

许可证 BSD-3-Clause，允许本项目使用。

## 2. 候选与实测动作

| 候选 | 处置 | 理由 |
|---|---|---|
| **Thevenin 0.5.0** | **采用** | 见下文全部实测证据 |
| Spice21 | 未评估 | 主选已通过全部准入用例，无需后备 |
| ngspice（外部进程） | 未采用 | 主选满足目标；引入 C 内核会劣化部署与错误定位 |
| faer | 未采用 | 仅自研内核需要；本项目不自研 |
| diffsol | 未采用 | 通用 ODE 库，不等同电路仿真器 |

按 brief 要求，未评估项**不声称**其能力。

## 3. crate 版本与依赖

实测 `cargo search thevenin` 返回的家族（全部 0.5.0，版本互相匹配）：

| crate | 作用 |
|---|---|
| `thevenin` | 主入口。`thevenin::circuit` 是公开仿真层 |
| `thevenin-types` | 结果类型（`SimResult` / `SimPlot` / `SimVector` / `Complex`）与网表类型 |
| `cirq-ir` | 规范 IR：`Circuit`，仿真的规范输入 |
| `thevenin-cirq` | Circuit 形状 API 的再导出 + SPICE 源便利函数 |
| `thevenin-xspice` | XSPICE 码模型（本项目未使用） |

实际使用的依赖（`crates/circuit-backend/Cargo.toml`）：

```toml
cirq-ir      = "0.5.0"
thevenin     = "0.5.0"
```

`thevenin-types` 经 `thevenin` 传递引入。**未**依赖 `thevenin-cirq`
（它是给已有 `thevenin-cirq` 调用方的兼容层，本项目直接用 `thevenin::circuit`）。

## 4. 关键问题逐条回答（brief §4.1）

### 4.1 能否在 Windows 环境编译运行？

**能。** 实测：冷编译 50.35 秒，`Finished dev profile`，无警告失败。
`x86_64-pc-windows-msvc`，无需 C 工具链（纯 Rust，无 `-sys` 依赖）。

### 4.2 能否从 Rust 数据结构构造电路，而不必先生成 Cirq 源码？

**能，这是本次评估最重要的结论。**

`cirq_ir::Circuit` 的字段全部 `pub`，且 `Net` / `Element` / `Connection` /
`Value` / `SourceSpec` / 各 `Analysis` 结构体均可直接构造：

```rust
pub struct Circuit {
    pub name: String,
    pub nets: Vec<Net>,
    pub elements: Vec<Element>,
    pub models: Vec<Model>,
    pub analyses: Vec<Analysis>,
    pub params: Vec<ResolvedParam>,
    // ... 其余字段
}
pub struct Element {
    pub id: Id, pub name: String, pub kind: ElementKind,
    pub connections: Vec<Connection>,
    pub params: Vec<(String, Value)>,
    pub model: Option<Id>,
    pub source_spec: Option<SourceSpec>,
}
```

因此 `circuit-backend` 可以直接做 `我们的 IR → cirq_ir::Circuit` 的结构映射，
不需要生成源码文本、不需要调用解析器。这消除了"生成-再解析"这一整类错误。

> 注：`ElementKind`、`Value`、`Analysis`、`Waveform` 都标了 `#[non_exhaustive]`。
> 适配层对这些做转换时必须带兜底分支，并在遇到未知变体时报错而非静默丢弃。

### 4.3 OP / AC / TRAN / DC 的调用接口

`thevenin::circuit` 提供：

```rust
pub fn simulate_op  (circuit: &Circuit) -> Result<SimResult, CircuitSimError>;
pub fn simulate_dc  (circuit: &Circuit) -> Result<SimResult, CircuitSimError>;
pub fn simulate_tran(circuit: &Circuit) -> Result<SimResult, CircuitSimError>;
pub fn simulate_ac  (circuit: &Circuit) -> Result<SimResult, CircuitSimError>;
pub fn simulate     (circuit: &Circuit) -> Result<SimResult, CircuitSimError>; // 跑完 analyses 里声明的全部
```

**重要实测细节**：`simulate_tran` 会在瞬态数据**前面插入一个工作点 plot**。
返回的 `plots` 形如 `[op1, tran1]`，**不是**只有瞬态。
适配层必须按 plot 名（`op`/`tran`/`ac`/`dc` 前缀）选取，不能取 `plots[0]`。
本项目的适配层因此统一用"按名字前缀选取"的策略。

实测 plot 命名：`op1`、`tran1`、`ac1`、`dc1`。

### 4.4 节点、电流、频率、时间及复数结果的表达

- 节点电压向量名：`v(<net name>)`，如 `v(mid)`、`v(out)`。
- 支路电流向量名：`<element>#branch`，如 `v1#branch`。
- 频率轴：`frequency`（实数）。时间轴：`time`（实数）。
- DC 扫描轴：`v-sweep`（另有 `@v1[dc]` 形式）。
- 复数结果：`thevenin_types::VectorData::Complex(Vec<thevenin_types::Complex>)`，
  `Complex { pub re: f64, pub im: f64 }`，附 `magnitude()` / `phase_rad()` / `phase_deg()`。

`thevenin_types::Complex` **不是** `num_complex::Complex<f64>`，两者不能直接互换。

### 4.5 源、模型、初值、容差和分析选项如何传递

- 源：`SourceSpec { dc: Option<f64>, ac: Option<AcSpec>, waveform: Option<Waveform> }`，
  `AcSpec { mag, phase }`（phase 单位为**度**）。
  `Waveform` 支持 `Pulse` / `Sin` / `Exp` / `Pwl` / `Sffm` / `Am`。
- 模型：`Circuit.models: Vec<Model>`，`Element.model: Option<Id>` 指向它。
- 初值：`Circuit.initial_conditions`（`.ic`）与 `Circuit.nodeset`（`.nodeset`）。
- 容差/选项：`Circuit.options: Vec<(String, Value)>`（如 `RELTOL`、`ABSTOL`、`GMIN`），
  引擎侧唯一解析点是 `thevenin-0.5.0/src/mna_ir.rs:107-133`（键名 `to_uppercase()`，`Value::String` 被静默忽略）。
  **但适配层把它写空**：`crates/circuit-backend/src/thevenin.rs:437` 恒为 `options: Vec::new()`，
  因此产品路径没有容差通道，永远跑引擎默认值
  （`thevenin-0.5.0/src/newton.rs:166-220`：`reltol=1e-3`、`abstol=1e-12`、`vntol=1e-6`、
  `chgtol=1e-14`、`trtol=7.0`，积分方法默认 Trapezoidal）。
- 温度：`Circuit.temps: Vec<f64>`（摄氏度）。

**填上 `options` 会不会改变结果？分两种情形（本轮 `_probe` 实跑 exit 0）：**

- **`h_max` 被 `tmax` 钉死时：不可观察。** 组 A（`tmax=Some(tau/1000)=100 ns`、`tr=1 µs`）把
  空 options、`RELTOL=1e-12`、`ABSTOL=1e-15` 三种配置逐字段对比：都是 6025 点，
  `n 6025/6025 identical=true`、对齐前缀 `max |dv(out)| = 0.000e0 V`、无时间轴分歧。
  ⇒ 该配置下容差设置**不能作为实验变量**；任何"调紧容差所以更准"的说法在这里都是假阳性。
- **`h_max` 自由时（`tmax=None` ⇒ `h_max = min(step, stop/50) = 1 µs`）：单因素仍不可观察，只有交互作用可观察。**
  组 B（`step=1 µs`、`tr=20 µs`）：空 options / `RELTOL=1e-12` / `ABSTOL=1e-15` / `TRTOL=0.7`
  四种配置都是 635 点且逐字段相同；只有 `RELTOL+ABSTOL` **同时**改才变成 757 点（`n 635/757 identical=false`，
  对齐前缀 110 个采样逐位相同，首个时间轴分歧在 index 110）。
  ⇒ 该效应只能报告为 `RELTOL` 与 `ABSTOL` 的**交互作用**，不能归因到其中任何一个单独设置。
- 传递链（源码）：`options` → `nr_options_from_circuit`（`mna_ir.rs:107`）→ `TranRunParams.nr_opts`
  （`mna_ir.rs:625-632`、`transient.rs:662-672`）→ LTE 步长估计（`transient.rs:1621-1634`、`1664-1677`）。
  纯线性电路不进 Newton 收敛判据，所以 `reltol/abstol/vntol` 只能通过 LTE 起作用。

### 4.6 失败是否可识别？

**可以，但覆盖范围有限，而且失败信息不定位。**

| 故障 | 实测行为 |
|---|---|
| 冲突的理想电压源（奇异矩阵） | `Err("... matrix is singular, cannot solve")` ✅ |
| 器件端子引用不存在的 net id | `Err("... terminal `neg` references unknown net id")` ✅ |
| **真无参考的线性网络**（整个连通块不接地，或节点到地的每条路都被电容/电流源阻断） | `Err("... matrix is singular, cannot solve")`，**错误文本不指向任何节点** ⚠️ |
| **合法开路输出**（节点经 R/L/V/二极管可达地，支路电流为 0） | `Ok`，值为确定解：实测 `v(a)=v(b)=1 V`（精确）、`i(v1)=0 A`，且 `GMIN` 从 1e-12 改到 1e-3 结果不变 ✅ |
| **含非线性器件的无参考网络**（同一孤立岛 + 一个二极管，触发 Newton） | `Ok`，且值**随 `GMIN` 变化**（gmin stepping 兜底）⚠️ |

实测出处：

- 线性反例（本仓库 `_probe`，本轮实跑 `robustness` exit 0）：`[f]`（仅 `r1(a,b)`、无端子接地）与
  `[g]`（节点只经电容连接）都返回逐字相同的
  `Err(simulation failed: failed to solve MNA system: matrix is singular, cannot solve)`；
  `docs/review-evidence/floating-audit.md` §5 用四种线性形态（`nets{1,2}`、`nets{0,1}`、
  含电压源的孤立岛、只经电容连接的节点）复现同一结论，其中 `GMIN=1e-3` 也救不回来。
- 合法开路正例（同一次 `robustness` 实跑）：`[a2]` 给出 `v(a)=v(b)=1.00000000000000000e0 V`（17 位精确）、
  `i(v1)=0.00000000000000000e0 A`；`[a3]` 实测 `GMIN=1e-12` 与 `GMIN=1e-3` 结果相同。
- 非线性/孤立岛（A04 的仓库外实验，见 `docs/review-evidence/backend-contract.md` §3.D 表 C5）：
  同一孤立岛加一个二极管后返回 `Ok`，`v(a)` 随 `GMIN` 变化——默认 `GMIN` 下
  `v(a)=5.000000e-1 V = I/(2·gmin)`，`GMIN=1e-6` 时 `5.002499e-7 V`，`GMIN=1e-3` 时 `6.666667e-10 V`。
  **我本轮未复跑**这条（`_probe` 的两个二进制都不含该用例），引用的是 A04 的实验记录。

**机制（引擎源码）**：线性电路的 OP 直接解线性系统且 `diag_gmin` 被强制为 0
（`thevenin-0.5.0/src/simulate.rs:69-104`）——这条路径不加对角 gmin，真无参考网络只会奇异失败；
对角 gmin 只存在于 Newton 路径（`src/newton.rs:361-364`），gmin stepping 是 NR 失败后的兜底
（`src/newton.rs:420-533`），只有含非线性器件的电路才会走到。

**因此前端必须自己做直流参考通路检查**：理由不是"后端不报错"，而是后端的 singular 错误
**不指向任何节点**、也**无法区分"合法开路输出"与"真无参考"**；非线性路径还会用 gmin 给出一个
看似合理的有限值。检查规则见 `crates/circuit-core/src/connectivity.rs`、调用点见
`crates/circuit-dsl/src/elaborate.rs`（报 `E_NAME`，定位到节点与阻断器件）——这与 brief §9 的要求一致
（电容通路不等于直流参考通路）。

### 4.7 全局状态、线程安全、实例隔离

实测：8 个线程并发跑不同输入的仿真，结果全部正确，无交叉污染。
后端没有可观测的全局可变状态。适配层可以按需并行执行参数扫描。

### 4.8 crate 版本匹配与接口稳定性

家族内所有 crate 均为 0.5.0，`cargo build` 无版本冲突。
`ElementKind` / `Value` / `Analysis` / `Waveform` 标注 `#[non_exhaustive]`，
意味着 1.x 内可能新增变体——适配层的转换必须写兜底分支并显式报错。
`Circuit` 等核心结构体字段为 `pub` 且无 `#[non_exhaustive]`，
但**添加字段是破坏性变更**，升级时需重新检查。

## 5. 准入用例结果（brief §4.3）

全部用例的电路都是**用 Rust 代码直接构造 `cirq_ir::Circuit`**，
没有生成任何 Cirq/SPICE 源文本。

| # | 用例 | 对照基准 | 实测结果 |
|---|---|---|---|
| 1 | 分压器 OP | `Vmid = Vin·R2/(R1+R2)` | `v(mid)=0.66666667`，`|diff|=0` ✅ |
| 2 | RC 瞬态（有限斜坡） | 分段匹配解析解：`T_eff = max(声明 rise, .tran step)`、稳定形式 `x + expm1(-x)`，对每个采样点套 §17 判据（atol 1e-5 V / rtol 1e-3）。（该 `_probe` 配置 `rise = 1 µs ≥ step`，新旧步长规则同值；任务 A 后产品路径按声明边沿建模，见 §5.1.1） | 基线 6025 点，`max err = 4.999167e-7 V`，超限 0/6025 ✅（明细见下） |
| 3 | RC 交流幅相 | `H=1/(1+jωRC)` | 最差 `|diff|=5.55e-17`（机器精度） ✅ |
| 4 | RLC 交流 | `H=1/(1-ω²LC+jωRC)` | 最差 `|diff|=2.31e-15` ✅ |
| 5 | 二极管非线性 OP | 二分法独立求解（`Vt=0.02586419`） | `v(out)=0.692872` vs `0.692868`，`|diff|=3.28e-6` ✅ |
| 6 | DC 扫描 | `v(mid)=0.5·Vsweep` | 6 点全部 `|diff|=0` ✅ |

**Case 2 明细（`_probe` 口径；该 bin 记录的是内核层事实，未随任务 A 改动）。** 旧做法用 1 ps 上升沿 + 理想阶跃解
`1-e^{-t/τ}` 在 5 个采样点对照，最差 1.95e-3 V，并归因于「有限上升沿 + 采样对齐」。
**这个归因错了两次**：

1. **1 ps 从来没有进入求解器。** `thevenin-0.5.0/src/waveform.rs:37` 把 PULSE 的 `tr`/`tf`
   夹到 `.tran` 步长（`tr.unwrap_or(tstep).max(tstep)`）；旧配置 `step = tmax = τ/200 = 500 ns`
   ⇒ 实际激励是 **500 ns 斜坡**，不是 1 ps。对宽度 T 的斜坡，RC 响应与理想阶跃之差为
   `C(T)·e^{-t/τ}`，`C(T) = 1 - (tau/T)(e^{T/tau}-1) ≈ -T/(2τ)`；`T = 500 ns` 时
   `|C| = 2.504e-3 V`，恰与旧实测 1.95e-3 V（在 0.25τ 采样点）一致。
2. **「采样对齐」的贡献恒为 0。** 旧代码用**实际返回时间** `t[idx]` 求参考值
   （`_probe/src/main.rs:322`），两者在时间上本来就是对齐的；`1 ps/τ = 1e-8` 的量级
   也解释不了 1e-3。所以 1.95e-3 是**参考模型错**（用了理想阶跃），**不是**求解器缺陷。

**本轮的新做法**：声明 `td = 100 µs`、`T = 1 µs` 的有限斜坡（`tr >= step`，保证夹取不改变 T），
参考值取分段匹配解析解（`u ≤ T`：`(V0/rho)(x + expm1(-x))`；`u > T`：`V0 + (y(T)-V0)e^{-(u-T)/τ}`），
对**每一个返回采样点**套用判据 `|a-e| ≤ atol + rtol·|e|`（`atol = 1e-5 V`、`rtol = 1e-3`）。实测
（**`_probe` Case 2 口径：单次脉冲、`td = 100 µs`、`T = 1 µs`、`stop = 601 µs = td + T + 5τ`**，
所以基线是 6025 点）：

| 配置 | 点数 | max err | 超限 |
|---|---|---|---|
| 基线 `tmax = tau/1000 = 100 ns`（推荐） | 6025 | 4.999167e-7 V | 0 / 6025 |
| 同一输出 vs **理想阶跃**参考（反事实，不参与 PASS/FAIL） | 6025 | 4.966368e-3 V | 1789 / 6025 |
| 旧配置复现（**`_probe` 口径**：`tr = 1 ps`、`step = tmax = 500 ns`、`stop = 5τ`）vs 匹配 500 ns 斜坡 | 1015 | 6.278341e-7 V | 0 / 1015 |
| 同上 vs **理想阶跃**参考 | 1015 | 2.491963e-3 V | 259 / 1015 |
| 其中 0.25τ 采样点（t = 25.232 µs） | — | 1.945643e-3 V | 解析预测 `|C(500 ns)|·e^{-0.25} = 1.950251e-3 V` |

`max_step` 三档（只改 `tmax`，`tr = 1 µs`、`step = 500 ns`、`stop = 601 µs` 固定；与上表同一
`_probe` 配置与窗口）：

| `tmax` | 点数 | 匹配参考 max err | 超限 | 结论 |
|---|---|---|---|---|
| `tau/50` = 2 µs | 316 | 1.9933e-4 V | **15** | **未达标，保留不放宽** |
| `tau/200` = 500 ns | 1217 | 1.2490e-5 V | **3** | **未达标，保留不放宽** |
| `tau/1000` = 100 ns | 6025 | 4.9992e-7 V | 0 | 达标（推荐配置） |

**`3/1217` 超限是本轮的真实发现，没有被删除、也没有放宽阈值。** 可复现机制：斜坡起点
（源断点）之后求解器强制用 Backward Euler 重启，首个被接受的步长 `h1 = tmax/10`
（`thevenin/src/transient.rs:1442-1444` 在断点处把步长压到 0.1×），该步局部误差
`≈ (V0/T)·h1²/(2τ)`；`tmax = τ/200` 时 `h1 = 5e-8 s` ⇒ 1.25e-5 V，**略高于** `TRAN atol = 1e-5 V`；
`tmax = τ/50` 时 `h1 = 2e-7 s` ⇒ 2.0e-4 V，超限更多。把 `tmax` 收到 `τ/1000` 即 0/6025 全过 ——
这是「参考判据在步长重启处与实现精度不相容」，不是求解器缺陷。

**容差实验（分组口径，`_probe` 实测）**：

- **组 A（`h_max` 被 `tmax` 钉死）**：空 options / 单改 `RELTOL=1e-12` / 单改 `ABSTOL=1e-15`
  三种配置逐字段完全相同（各 6025 点，`max |dv(out)| = 0.000e0 V`）⇒ 该配置下设置**不可观察**，
  **不能作为实验变量**。
- **组 B（`tmax = None`，`h_max = min(step, stop/50) = 1 µs` 自由）**：单改 `RELTOL`、单改 `ABSTOL`、
  单改 `TRTOL` 都与空 options 逐字段相同（各 635 点）；只有 `RELTOL + ABSTOL` **同时**改才改变
  时间轴（757 点，首个分歧在 index 110）⇒ 只能报告**交互作用**，不能归因到单一设置。

用例 2 同样验证了「内部步长 ≠ 输出采样」：基线返回 6025 个采样点、时间轴 `[0, 6.01e-4] s`，
`dt_min = 2.5e-10 s`（= `h_max/400`）、`dt_max = 1e-7 s`（= `tmax`），且严格递增、无重复
（`duplicates = 0`、`non-increasing = 0`）。

**已知精度限制（a11 复核发现，F2；这一条不是通过项）**：上面三档 `max_step` 都在「源断点发生在
t = 0」下测得。把同一电路的脉冲 `delay` 改成 100 µs（`step = tmax = 1 µs`）后，a11 在裸引擎上实测
由 0 点超限变成 **10 点超限、最大误差 4.991676e-5 V**（首个超限点 t = 1.0010000e-4 s，
err/allow = 4.96）。机制与 `tau/50`、`tau/200` 两档相同：源断点之后第一个被接受的步被强制回退成
Backward-Euler，`h1 = tmax/10`，局部误差 `≈ (V0/T)·h1²/(2τ)`（`tmax = τ/100` 时 `h1 = 100 ns`
⇒ 约 5e-5 V，与实测吻合）。**下一轮工作**：把 `max_step` 收细，或对断点重启步做专门的误差控制；
本轮**不放宽 §17 阈值、不删除任何数字**。（出处：`docs/review-evidence/numerical-review.md` §7 F2。）

**读 `_probe` 退出码的口径（F3）**：`_probe` 的 Case 2 PASS 判据是「`max_step` 三档都跑通 + 推荐档
`tmax = τ/1000` 合格」；`tau/50` 与 `tau/200` 两行 NOT-MET 只被逐字打印并保留，**不影响进程退出码**
（probe exit 0）。因此「`probe` exit 0」**不等于**「所有配置都满足 §17」——判定某一档是否达标，
必须看它自己那一行的 over-limit 计数（`docs/review-evidence/numerical-review.md` §8）。

可复现命令：

```powershell
cargo run --manifest-path _probe/Cargo.toml --bin probe        # 6 个准入用例
cargo run --manifest-path _probe/Cargo.toml --bin robustness   # 13/13 失败/隔离/子集/孤岛检查
```

（实测：`probe` exit 0、Case 1–6 全 PASS；`robustness` exit 0、sub-case summary 13/13。
两个 bin 都从仓库根目录运行，`_probe` 有独立 target。）

### 5.1 瞬态步长契约与运行中断点的实测限制（任务 A + 任务 B）

#### 5.1.1 新的适配层步长映射（任务 A）

原来的产品路径把 `output_interval` 直接送进引擎的 `Tran.step`（tstep），而引擎把 tstep 当作
PULSE `tr`/`tf` 的下限（`thevenin-0.5.0/src/waveform.rs:37-38`），于是改变输出间隔会改写用户
声明的激励波形。现在三个概念彻底分离：

| 概念 | 载体 | 传给引擎 |
|---|---|---|
| 积分步长上界 | `TranSpec.max_step` | `CqTran.tmax`（引擎 `h_max`） |
| 输出采样 | `TranSpec.output_interval` | **不传**；求解后用 `circuit-results::resample` 重采样到等间隔网格 |
| 引擎 print step | 适配层内部量 `h_print = min(span/1000, 所有源声明的 rise/fall/period 最小值)` | `CqTran.step` |

- 含电容/电感时引擎走 LTE 分支，步长只受 `h_max` 约束；无电抗时该分支的步长上限是
  `min(h_max, h_print)`（`thevenin-0.5.0/src/transient.rs:1694`），因此 `h_print` 的取值必须
  同时考虑这两种情形（否则会误拒 `rise = 1 ps + max_step = τ/200` 这类合法配置）。
- 能力错误（`cdsl check` 即报，退出码 1）：
  - 声明源的 `rise`/`fall`/`period` 为 0 或非有限 → `E_UNSUPPORTED`，消息点名源与参数。
  - **由声明波形导致的**步数预算超限 → `E_LIMIT`。rev3 的归因判据（修自代码审核 W6-2）：只有
    "存在已声明的 `rise`/`fall`/`period`"**且**"没有波形约束时同一 1e6 预算不会被突破"才报错；
    消息为 `the declared source rise/fall/period is too fine for this simulation window:
    honouring it would need about N solver steps, over the limit of 1000000`，contexts 为
    `declared waveform timing` / `solver step` / `effective step`（如有 `max_step` 再加一项）。
    实测（rev3 CLI）：`rise: 1.ps` + `stop: 1.s`（无 `max_step`）→ **exit 1**，归因正确；
    纯直流源 + RC + `tran stop: 1.s, max_step: 1.ns` → **exit 0**（不再被误拒：那 1e9 步来自
    用户自己的 `max_step`）。注意后一种配置**没有运行期步数保护**——`check` 通过不代表运行很快，
    `Limits::max_result_values` 只在结果生成后生效（`docs/language.md` §5.3）。
  - `max_step` / `output_interval` 为 0、负数或非有限 → 前端 `E_VALUE`，**不回退默认值**。
- `output_interval` 只改变输出的采样点：输出网格契约（首点、`首点+k·interval`、末点恒保留、
  线性插值、不越界外推、超 `Limits::max_result_values` 报 `E_LIMIT`）见
  `docs/language.md` §5.3；`avg`/`rms`/`max`/`min` 保持在**原始求解网格**上计算。

**产品示例的实测（本轮，`examples/rc_filter.cdsl`：`rise = fall = 1.ns`、`max_step: 500.ns`、
未给 `output_interval`）**：

```
cdsl run examples/rc_filter.cdsl --experiment response --out <dir> --format csv   → exit 0
```

| 量 | 实测 |
|---|---|
| 输出点数 / 末点 | 1019（= 原始求解网格，无重采样）/ t = 500 µs |
| `v(vin)` 达到 1 V 的时刻 | t = 1 ns（声明 `rise: 1.ns` 被兑现；旧行为要到 500 ns） |
| 元数据 | `tran.solver_step = 1e-9`、`tran.waveform_bound = 1e-9`、`tran.solve_points = 1019`、`tran.max_step = 5e-7` |
| vs 理想阶跃 `1-exp(-t/τ)` | max \|err\| = **4.921844e-6 V**（t = 1 ns） |
| vs 匹配的 1 ns 斜坡解析解 | max \|err\| = **7.916906e-7 V** |

对理想阶跃的 4.92e-6 V 是**斜坡的物理差异**而不是求解误差：`C(T) = 1-(τ/T)(e^{T/τ}-1)`，
`T = 1 ns` 时 `|C| = 5.000007e-6 V`；同一批点对匹配斜坡参考只差 7.9e-7 V。旧的
2.491963e-3 V 对应「`rise: 1.ns` 被 print step 展宽成 500 ns 斜坡」，该行为已随任务 A 消失。

#### 5.1.2 运行中源断点的重启误差（任务 B，限制项）

引擎在源断点之后的首个被接受步强制 Backward-Euler（`thevenin-0.5.0/src/transient.rs:1433,
1464-1468`），重启步 `h1 = min(2·h_before, h_max)·0.1`（`:1443`），静息斜坡起点的局部误差
`≈ (V0/T)·h1²/(2τ)`。产品路径实测（`crates/circuit-backend/tests/source_breakpoint_regression.rs`；
τ = 100 µs、`V0/T = 1e6 V/s`、`delay = 100 µs`、`rise = fall = 1 µs`、`stop = 300 µs`、
§17 判据 atol 1e-5 V / rtol 1e-3）：

| `max_step` | 点数 | max \|err\| | 超限点数 | 判定 |
|---|---|---|---|---|
| `τ/1000` = 1.0e-7 s（必过配置） | 3129 | 4.999167e-7 V | **0** | 逐点断言 |
| `τ/500` = 2.0e-7 s | 1629 | 1.999333e-6 V | **0** | 逐点断言 |
| `τ/200` = 5.0e-7 s | 729 | 1.248959e-5 V | **3** | LIMITATION（保留） |
| `τ/50` = 2.0e-6 s | 309 | 7.331775e-4 V | **250** | LIMITATION（保留） |

> **不要与本文件 §5 的 6025 点并排比较**：§5 是 `_probe` Case 2 的**单次脉冲**配置
> （`td = 100 µs`、`stop = 601 µs = td + T + 5τ`，窗口内只覆盖第 1 个上升沿），本节是产品回归的
> **10 个脉冲**配置（`stop = 300 µs`、40 个断点）。窗口与激励都不同，3129 点与 6025 点各自
> 对应自己的配置；唯一可比的是"同一配置内 `max_step` 的趋势"。

- 可达标的经验界 `h_max ≤ 10·sqrt(2·atol·τ·T/V0)`（本激励 4.472136e-7 s = τ/223.6）**只对该
  激励推导**：换边沿速率、时间常数、拓扑或判据必须重算，**不是通用保证**。
  不要把它写成“任何电路用 τ/1000 都安全”。
- 超限配置以 `LIMITATION` 标签与独立 characterization 测试原样保留：不放宽 §17 阈值、
  不删除超限样本（253 条明细见 `target/round2-evidence/w3/`，报告
  `docs/review-evidence/round2/breakpoint-evidence.md` §4–§5）。
- 内核侧独立复现（不经适配层）：

```powershell
cargo run --manifest-path _probe/Cargo.toml --bin breakpoint_study   # 14/14 契约检查，exit 0
cargo run --manifest-path _probe/Cargo.toml --bin tran_contract      # 12/12 契约钉，exit 0
```

  `breakpoint_study` 钉住 `h1 = min(2·h_before, h_max)·0.1` 与 BE 阶（12/12）；`tran_contract`
  钉住 `tr_used = max(declared tr, tstep)`（`step = 100 ns` + 声明 10 ns → 实测斜坡 100 ns；
  `step ≤ tr` 时斜坡等于声明值）。两个 bin 的退出码语义：任一契约检查失败或 panic → 1；
  §17 的 `[NOT-MET]` 行原样打印并汇总，**不翻转退出码**（判某档是否达标必须看该行自己的
  over-limit 计数）。
- 容差通道（内核层，`breakpoint_study`）：`RELTOL` / `ABSTOL` / `TRTOL` 单因子变化**不可观测**；
  `RELTOL+ABSTOL` 同时收紧会改变轨迹，但重启步 `h1` 与 §17 结论**不变**。产品路径
  `build_circuit` 传空 `options`，所以**产品端到端没有容差通道**。

## 6. 适配层必须处理的实测约束

这些是从实测中直接得出的、会写进 `circuit-backend` 的约束：

1. **按 plot 名选取结果**，不能取 `plots[0]`（`simulate_tran` 会前置 OP plot）。
2. **`save` 不被单分析入口遵守**：实测设置 `circuit.save = ["v(mid)"]` 后
   `simulate_op` 仍返回 `v(mid)`、`v(a)`、`v1#branch` 三个向量。
   探针子集化必须由适配层自己做。
3. **真无参考的线性网络会以 singular 失败，但错误不指向节点**，也无法区分「合法开路输出」；
   含非线性器件的电路则可能返回 gmin 支撑的 `Ok`。直流参考通路检查因此仍由前端做
   （报定位到节点的 `E_NAME`，机制与实测见 §4.6）。
4. `Complex` 是 `thevenin_types` 自己的类型，不能与 `num_complex` 混用。
5. `AcSpec.phase` 单位是**度**（实测 `thevenin/src/mna_ir.rs` 中
   `ac.phase * PI / 180.0`），而本项目内部相位以**弧度**存储，转换在适配层完成。
6. `#[non_exhaustive]` 枚举必须写兜底报错分支。
7. 零值/负值 R/L/C：本项目前端按 brief §6 拒绝，不依赖后端行为。
8. **PULSE 的 `tr`/`tf` 仍会被引擎夹到 print step**（`tr.unwrap_or(tstep).max(tstep)`，
   `thevenin-0.5.0/src/waveform.rs:37`），所以适配层必须自己保证
   `h_print ≤ min(声明 rise/fall/period)`：现在的取值是
   `h_print = min(span/1000, 该最小值)`（`thevenin.rs::tran_step_for`），产品路径因此不会触发该夹取。
   **历史行为已移除**：旧映射把 `output_interval` 送进 `Tran.step`，导致
   `examples/rc_filter.cdsl`（`rise: 1.ns`、未给 `output_interval` ⇒ step = 500 ns）实际跑的是
   500 ns 斜坡；现在 `output_interval` 不进入求解器，改由 `circuit-results::resample` 在求解后
   重采样。参考值仍按**实际执行的** `T_eff = max(T_声明, h_print)` 建模——修复后该值就是声明值
   本身（产品实测见 §5.1.1，内核钳位事实见 `_probe` 的 `tran_contract`）。

### 6.1 支路电流只有部分器件可得（实测）

用 `_probe/src/bin/currents.rs` 实测：把 V、R、L、C 串成一个回路，
三种分析下引擎返回的向量分别是

```text
[OP]   v(c) v(b) v(a) v1#branch l1#branch
[AC]   frequency v(a) v(b) v(c) v1#branch l1#branch
[TRAN] time v(c) v(b) v(a) v1#branch l1#branch
```

结论：

| 器件 | 引擎是否给出支路电流 |
|---|---|
| 电压源 | **是**（`<name>#branch`） |
| 电感 | **是**（`<name>#branch`） |
| 电阻 | **否** |
| 电容 | **否** |
| 独立电流源 | **否** |
| 二极管 | **否** |

只有自带支路未知量的器件才有 `#branch` 向量。

因此本项目对 `i(:dev)` 的处理是：

- **直接读取**：电压源、电感。
- **经过验证的推导**：电阻用欧姆定律 `i = (v(p) - v(n)) / R`。
  这是精确推导，且在 `tests/adapter.rs` 中用**引擎自己给出的源电流**做了
  交叉验证：串联回路中 `i(r1) = -i(v1)`，直流与交流（复数）两种情形都验过。
- **明确拒绝**：电容、二极管、独立电流源。报 `E_UNSUPPORTED` 并说明原因
  （电容电流需要微分，本项目不做）。

按 brief §8.5「无法直接获取的器件电流不得伪造」，这里没有对不可得电流做近似。

### 6.2 负极接地与节点命名

实测：节点名为 `gnd` 或 `0` 都被当作参考地（引擎内部把 `gnd` 重写为 `0`）。
本项目统一用 `gnd`，层次节点名可以含 `.`（如 `stage1.out`），
引擎按字符串键处理，实测无冲突。

## 7. 未验证 / 限制

按 brief 要求，明确标注未验证项，不声称通过：

- **仅 Windows MSVC 实测。** Linux（`x86_64-unknown-linux-musl` 目标存在）
  在本项目中**未验证**。
- 仅验证了 R / L / C / 独立电压源 / 二极管。MOSFET、BJT、受控源等
  虽由后端声明支持，但本项目**未验证**，首期也不暴露。
- `uic`（跳过工作点）**产品路径未暴露**：`crates/circuit-dsl/src/elaborate.rs:2494` 恒为
  `uic: false`，语言没有对应语法；引擎语义已由 A04 取证（`uic=true` 跳过 OP、初值全 0，见
  `docs/review-evidence/backend-contract.md` §3.C），**产品端到端未验证**。
- 未评估噪声、灵敏度、PZ、TF、Fourier/FFT 分析。
- 未做大规模电路（数万节点）的性能与内存测试。
- 后端自身是第三方 RELEASE 声明其与 ngspice 兼容；本项目只对本文件
  列出的用例负责，不转发其未验证的能力声明。

本轮新增/更新的未验证项：

- **产品路径没有容差通道**：`crates/circuit-backend/src/thevenin.rs:458` 恒空 `options`，
  「改 reltol/abstol 会改变产品输出」只在引擎层 / `_probe` 层验证过，**产品端到端未验证**（见 §4.5）。
- **运行中源断点的精度界只对一种激励验证过**：`h_max ≤ 10·sqrt(2·atol·τ·T/V0)` 由单个 RC
  （τ = 100 µs、`V0/T = 1e6 V/s`）推出；多极点网络、不同 `T/τ`、电感/二极管电路**未验证**（§5.1.2）。
  产品路径也没有对过粗 `max_step` 的校验，只是把超限行作为限制保留。
- **重采样的边界组合未穷尽**：已覆盖首末点、等间隔、不越界外推、单点/退化轴、复数信号、
  规模上限与非时间轴原样返回（`circuit-results/src/resample.rs` 的单元测试）；**未验证**
  `start_s ≠ 0`、`uic = true`、多个 `tran` 任务同实验共存、以及参数扫描路径（产出
  `Axis::Parameter`，不经重采样）的组合。
- **AC 相位只覆盖适配层**：本轮新增的相位回归
  （`crates/circuit-backend/tests/phase_regression.rs`，6 个测试）直接构造项目 IR，覆盖 ±30°/60°/90°/45°
  等角度并分别核对实部与虚部；但 DSL **没有** `ac phase:` 语法（`elaborate.rs:819-822` 写死
  `phase_rad: 0.0`），所以**产品端到端（源文件 → CLI）的非零 AC 相位仍未验证**。
- **正弦相位只覆盖无状态电路**：`sin(phase:)` 的度→弧度换算存在于 DSL 侧（`elaborate.rs:1728-1734`），
  但相位回归只覆盖电阻分压这类无电抗电路，**含 C/L 的状态电路未验证**，该路径也没有端到端用例。
- **容差只能报交互作用**：组 B 中单因素（RELTOL / ABSTOL / TRTOL 单独）不可观察，本轮**没有**
  把可观察的差异归因到其中任何一个（见 §5）；内核层的 `breakpoint_study` 进一步显示
  `RELTOL+ABSTOL` 交互会改变轨迹但**不改变重启步 `h1`**（§5.1.2）。

## 8. 对项目架构的影响

Thevenin 可以直接由 Rust 结构体驱动，因此：

- 不需要 SPICE 网表生成器（不引入文本往返错误）。
- 后端边界就是一个纯函数式的 trait（`SimulationBackend`），
  首期只有一个实现，但接口按 brief §7.2 保留能力声明与 ID 映射。
- 由于不需要"生成源码"，`circuit-core` 的 IR 可以保持**面向语义**
  而不是面向文本，这对错误定位（brief §9）更有利。

后续若要接入第二个后端（如 SPICE 网表导出、ngspice 进程），
只需新增一个 `SimulationBackend` 实现，不需要改动 IR 与前端。
