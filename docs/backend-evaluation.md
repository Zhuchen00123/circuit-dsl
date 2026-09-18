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
- 容差/选项：`Circuit.options: Vec<(String, Value)>`（如 `RELTOL`、`ABSTOL`、`GMIN`）。
- 温度：`Circuit.temps: Vec<f64>`（摄氏度）。

### 4.6 失败是否可识别？

**可以，但覆盖范围有限。**

| 故障 | 实测行为 |
|---|---|
| 冲突的理想电压源（奇异矩阵） | `Err("... matrix is singular, cannot solve")` ✅ |
| 器件端子引用不存在的 net id | `Err("... terminal `neg` references unknown net id")` ✅ |
| 悬空节点（无直流通路） | **返回 `Ok`，不报错** ⚠️ |

实测：`v1 - r1 - b`（b 无对地通路）时 `v(b) = 1` 而**不是**错误。
后端用 gmin/漏电把节点拉住了。

**因此悬空节点检测必须由本项目前端完成**，且必须基于"是否存在直流参考通路"
判断，不能只看图上有没有连线——这与 brief §9 的要求一致
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
| 2 | RC 瞬态阶跃 | `v(t)=1-e^{-t/τ}`，τ=100 µs | 最差 `|diff|=1.95e-3`（0.2%） ✅ |
| 3 | RC 交流幅相 | `H=1/(1+jωRC)` | 最差 `|diff|=5.55e-17`（机器精度） ✅ |
| 4 | RLC 交流 | `H=1/(1-ω²LC+jωRC)` | 最差 `|diff|=2.31e-15` ✅ |
| 5 | 二极管非线性 OP | 二分法独立求解（`Vt=0.02586419`） | `v(out)=0.692872` vs `0.692868`，`|diff|=3.28e-6` ✅ |
| 6 | DC 扫描 | `v(mid)=0.5·Vsweep` | 6 点全部 `|diff|=0` ✅ |

用例 2 的残差来自脉冲有限上升沿（1 ps）与输出采样对齐，
在默认 `RELTOL=1e-3` 下属于预期量级，**不是**后端错误。
用例 2 同样验证了"内部步长 ≠ 输出采样"：1015 个输出点，
时间轴实测 `[0, 5e-4] s`。

可复现命令：

```bash
cd _probe
cargo run --bin probe        # 6 个准入用例
cargo run --bin robustness   # 失败/隔离/子集/孤岛检查
```

## 6. 适配层必须处理的实测约束

这些是从实测中直接得出的、会写进 `circuit-backend` 的约束：

1. **按 plot 名选取结果**，不能取 `plots[0]`（`simulate_tran` 会前置 OP plot）。
2. **`save` 不被单分析入口遵守**：实测设置 `circuit.save = ["v(mid)"]` 后
   `simulate_op` 仍返回 `v(mid)`、`v(a)`、`v1#branch` 三个向量。
   探针子集化必须由适配层自己做。
3. **悬空节点后端不报错**，必须由前端做直流参考通路检查。
4. `Complex` 是 `thevenin_types` 自己的类型，不能与 `num_complex` 混用。
5. `AcSpec.phase` 单位是**度**（实测 `thevenin/src/mna_ir.rs` 中
   `ac.phase * PI / 180.0`），而本项目内部相位以**弧度**存储，转换在适配层完成。
6. `#[non_exhaustive]` 枚举必须写兜底报错分支。
7. 零值/负值 R/L/C：本项目前端按 brief §6 拒绝，不依赖后端行为。

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
- `uic`（跳过工作点）未验证，首期不开放。
- 未评估噪声、灵敏度、PZ、TF、Fourier/FFT 分析。
- 未做大规模电路（数万节点）的性能与内存测试。
- 后端自身是第三方 RELEASE 声明其与 ngspice 兼容；本项目只对本文件
  列出的用例负责，不转发其未验证的能力声明。

## 8. 对项目架构的影响

Thevenin 可以直接由 Rust 结构体驱动，因此：

- 不需要 SPICE 网表生成器（不引入文本往返错误）。
- 后端边界就是一个纯函数式的 trait（`SimulationBackend`），
  首期只有一个实现，但接口按 brief §7.2 保留能力声明与 ID 映射。
- 由于不需要"生成源码"，`circuit-core` 的 IR 可以保持**面向语义**
  而不是面向文本，这对错误定位（brief §9）更有利。

后续若要接入第二个后端（如 SPICE 网表导出、ngspice 进程），
只需新增一个 `SimulationBackend` 实现，不需要改动 IR 与前端。
