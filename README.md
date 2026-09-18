# circuit-dsl（`cdsl`）

## 项目简介

circuit-dsl 是一门描述模拟电路的小型领域语言，配一套纯 Rust 的实现：`.cdsl` 源文件声明电路、器件、参数与实验，命令行工具 `cdsl` 负责检查源文件、执行 op / dc / ac / tran 分析、求测量值并导出 CSV / JSON 结果。`cdsl repl` 提供交互式会话：表达式即时求值、会话变量、逐条定义电路与实验并直接运行。它面向可复现的小规模模拟电路验证：电路由结构化 IR 直接交给仿真后端构造，不生成 SPICE / Cirq 源文本，展开期的错误以带源码位置的诊断给出。工作区测试实测 `cargo test --workspace` 共 **389 个测试全部通过**（0 失败），`cargo clippy --workspace --all-targets -- -D warnings` 0 条警告，`cargo fmt --all -- --check` 无差异。

## 快速开始

```bash
# 构建 release 二进制
cargo build --release

# Windows 上的实际产物
target/release/cdsl.exe --version

# 查看后端能力
target/release/cdsl.exe capabilities

# 检查源文件（完整前端 + 后端能力校验，不仿真）
target/release/cdsl.exe check examples/rc_filter.cdsl

# 运行实验，结果写入 results/
target/release/cdsl.exe run examples/rc_filter.cdsl --experiment response --out results --format csv
```

也可以不显式构建，直接用 cargo 跑：

```bash
cargo run -p circuit-cli -- run examples/voltage_divider.cdsl --experiment divider
```

要求 Rust stable（workspace 声明 `rust-version = "1.85"`，实测环境 rustc 1.98.1 / `stable-x86_64-pc-windows-msvc`）。后端为纯 Rust，构建不需要 C 工具链。Windows 上 release 产物路径为 `target/release/cdsl.exe`。

## 最小示例

`examples/voltage_divider.cdsl` 全文：

```ruby
# Resistive divider.
#
# Running this with
#
#     cdsl run examples/voltage_divider.cdsl --experiment divider
#
# must give v(out) = 2.000 V and i(r1) = +1.000 mA.
#
# The sign is worth reading carefully: v1 sits between `in` and `gnd`, so the
# current through it in the p -> n direction is negative (-1 mA), because it
# delivers power. The current through r1, which absorbs it, is positive.

circuit :divider do
  node :in, :out

  # `p` and `n` fix the positive current direction: r1's current is measured
  # from `in` to `out`.
  voltage_source :v1, p: :in, n: :gnd, dc: 5.V
  resistor :r1, p: :in, n: :out, value: 1.kohm
  resistor :r2, p: :out, n: :gnd, value: 1.5.kohm
end

experiment :divider, circuit: :divider do
  op
  save v(:in), v(:out), i(:r1), i(:v1)
end
```

运行：

```bash
cargo run -p circuit-cli -- run examples/voltage_divider.cdsl --experiment divider --out results --format csv
```

实际输出（stdout）：

```text
experiment `divider` on circuit `divider` (backend thevenin 0.5.0)
  op1: scalar; signals: v(in), v(out), i(r1), i(v1)
  wrote results\divider.op1.csv
```

生成的 `results/divider.op1.csv`：

```csv
v(in),v(out),i(r1),i(v1)
5,3,0.002,-0.002
```

CSV 数值使用 SI 基本单位（V、A）；带轴的分析（dc / ac / tran）第一列是轴，OP 没有轴列。`i(v1) = -0.002 A` 为负是因为 `v1` 的 `p → n` 方向是 `in → gnd`，电压源输出功率。

> 说明：该示例文件头部注释写的 `v(out) = 2.000 V`、`i(r1) = +1.000 mA` 与实际电路参数不符（5 V 加在 1 kΩ + 1.5 kΩ 上应为 3 V / 2 mA）；以上面的实测输出为准。

## CLI 用法

```text
cdsl check <FILE> [--json] [--verbose]
cdsl run   <FILE> [--experiment <NAME>] [--out <DIR>] [--format csv|json|both] [--verbose]
cdsl repl  [FILE]
cdsl capabilities [--verbose]
cdsl --version
```

| 子命令 | 说明 |
|---|---|
| `check <file>` | 跑完整前端（词法 → 语法 → 展开）并做后端能力校验，不执行任何分析。`--json` 打印展开后的电路与实验（含节点、器件、端子、探针）。 |
| `run <file>` | 选择实验、执行分析、计算测量并写结果文件。`--experiment` 在文件只定义一个实验时可省略，否则必填。 |
| `repl [file]` | 交互式会话：表达式即时求值（`1.kohm * 100.nF` → `100 us`），`r = 1.kohm` 定义会话变量，可以逐条定义电路 / 子电路 / 实验并 `:run`，还支持 `:load`、`:list`、`:reset`、`:help`。给出文件则先载入。完整说明见 [`docs/repl.md`](docs/repl.md)。 |
| `capabilities` | 打印后端名称与版本、支持的分析、器件、扫描能力与备注。`--verbose` 追加 `Limits`（实测：max devices / nodes / loop iterations = 100000，max subcircuit depth = 16，max sweep points = 1000000，max result values = 50000000）。 |

`run` 的默认值：`--out results`、`--format both`。结果文件按 `<experiment>.<analysis>` 命名，例如 `divider.op1.csv`、`response.ac1.csv`、`response.tran1.csv`；参数扫描为 `sweep.dc_param_r.csv`（轴名 `parameter`，单位为 SI 欧姆）。若输出目录不存在会自动创建；命令拒绝把结果写到输入源文件上。

一段 REPL 会话（实际输出；多行输入用 `....> ` 续行提示）：

```text
$ cdsl repl examples/rc_filter.cdsl
loaded `examples/rc_filter.cdsl`: defined circuit `rc_filter`, experiment `response`
cdsl> r = 1.kohm
r = 1 kohm
cdsl> c = 100.nF
c = 100 nF
cdsl> tau = r * c
tau = 100 us
cdsl> :run response
experiment `response` (backend thevenin 0.5.0)
  op1: scalar; signals: v(vin), v(vout), i(input), i(r1)
  ac1: 121 frequency points; signals: v(vin), v(vout), i(input), i(r1)
  tran1: 1015 time points; signals: v(vin), v(vout), i(input), i(r1)
  ...
  measure vfinal = 993.245 mV
  measure vavg = 800.851 mV
  measure vrms = 837.972 mV
```

几条真实发生的错误（会话里先 `:load examples/voltage_divider.cdsl`）：

```text
cdsl> :run devider                 # 实验名拼错：列出真实存在的实验
error[E_NAME]: no experiment named `devider` in this session
   = defined: divider
cdsl> :run divider r1=3.kohm       # 覆盖一个并不存在的参数：明确拒绝，不静默忽略
error[E_NAME]: circuit `divider` has no parameter `r1`
  --> examples/voltage_divider.cdsl:24:31
   |
24 | experiment :divider, circuit: :divider do
   |                               ^^^^^^^^
   = declared parameters: <none>
   = an override that names nothing would silently leave every value at its default
cdsl> load x.cdsl                  # 命令少写冒号
error[E_SYNTAX]: `load` is a command; write `:load`
  --> <repl:5>:1:1
   |
1 | load x.cdsl
  | ^^^^
   = commands are not part of the language, so they always start with `:`
```

注意 `r1=3.kohm` 在提示符下**不是**覆盖，而是一个名为 `r1` 的会话变量——覆盖只在
`:run` 的参数位置出现，两者不会混淆。

`--version` 实际输出：

```text
cdsl 0.1.0
```

`capabilities` 实际输出：

```text
backend: thevenin 0.5.0
analyses: op, dc, ac, tran
devices: resistor, capacitor, inductor, voltage_source, current_source, diode
source-value DC sweep: yes
parameter DC sweep: yes (one elaboration per point, via the CLI)
note: DC sweeps of a source value are executed natively.
note: DC sweeps of a parameter are executed by the sweep driver in this crate, one elaboration per point.
note: Transient output time points are solver-chosen and non-uniform; `max_step` bounds the internal step, not the output interval.
note: Verified on Windows MSVC only.
```

**退出码契约**：`0` 成功；`1` 用户错误（文件读取失败、词法 / 语法 / 名称 / 量纲 / 展开错误、后端不支持的能力）；`2` 内部错误（如后端未产出任何结果）。诊断写 stderr，数据与摘要写 stdout。

## 已支持的功能

### 语言特性

- 顶层定义 `circuit`、`subcircuit`（必须声明 `ports`）、`experiment`，出现顺序无关。
- 层次实例 `instance :x, of: :sub, ports: { ... }, params: { ... }`；实例内部节点互相隔离，层次路径用 `.` 连接（如 `stage1.internal`）。
- `param`：`default:` 可省略；可由实例或实验覆盖；覆盖顺序为默认值 → 实例/实验 → 扫描点；影响条件、循环次数或连线的参数是拓扑参数，不允许扫描。
- `for`（数组元素与含两端的整数区间）、`if / elsif / else`；循环变量可参与表达式并用于生成器件名（动态名必须唯一）。
- 量纲字面量（`1.kohm`、`100.nF`、`1.us`、`10.Hz`、`1.V` 等），`+ - * /` 与比较会做量纲检查；内置常量 `pi`、`e`；数值函数 `abs` / `sqrt` / `min` / `max`；波形函数 `pulse` / `sin` / `pwl`。
- 符号（`:name`）与字符串（`"text"`）是不同类型；支持数组、字典、布尔与以 `#` 开始的行注释；换行终止语句，括号内、逗号后、行尾二元运算符后可续行。
- 普通 R/L/C 必须是严格正值，零值或负值报 `E_VALUE`，不会被替换成很小的正数。

### 器件

- 线性：`resistor`、`capacitor`、`inductor`、`voltage_source`、`current_source`。
- 非线性：`diode` + `model :name, type: :diode, is: <电流>, n: <数>`。
- 源的 `dc:`、`ac:`（幅度）与 `waveform:` 相互独立；`p` / `n` 端子同时定义电流正方向 `p → n`。
- 节点必须显式 `node` 声明（`:gnd` 除外），器件端子引用未声明节点报 `E_NAME`。

### 分析

- `op`：工作点。
- `dc source: :dev, from:, to:, step:`：源值扫描；`dc param: :r, from:, to:, step:`：单参数扫描（由 CLI 逐点重新展开并拼接）。
- `ac from:, to:`，配 `points_per_decade:` 或 `points:`（二选一）；相位内部以弧度存储，输入输出用度。
- `tran start:, stop:, max_step:`（`start` 可省略，默认 0）。
- 一个实验可声明多个分析，结果按分析 ID 分开导出，互不覆盖。

### 结果与导出

- 探针：`save v(:n)`、`v(:a, :b)`（= Va − Vb）、`i(:dev)`（正方向 `p → n`）；测量：`measure :vmax, max: v(:out)`，归约可选 `max` / `min` / `avg` / `rms`。
- `avg` / `rms` 在**非均匀求解器时间轴**上按时间积分，而不是样本算术平均。实测 `examples/rc_filter.cdsl`：`vavg = 0.8008509 V`、`vrms = 0.8379724 V`。
- CSV：第一列为轴；复数列拆成 `_re` / `_im`（实测 `response.ac1.csv` 表头 `frequency,v(vin)_re,v(vin)_im,...`）；时间轴非均匀（同一实验 1015 个时间点）。
- JSON：保留单位、轴类型与后端元数据；非有限值导出为 `null`（CSV 为空字段）并给出警告。
- 本仓库实测数值：分压器 OP 精确给出 `v(out)=3 V`、`i(r1)=+2 mA`、`i(v1)=-2 mA`；参数扫描逐点等于 `3·1.5k/(r+1.5k)`（实测 r=500 Ω → 2.25 V、1 kΩ → 1.8 V、2 kΩ → 1.285714… V）；`examples/diode_rectifier.cdsl` 的 `op` 给出 `v(vout)=0.692872 V`，其 DC 扫描在 5 V 点给出 0.692869 V（对照 Shockley 方程二分法独立解 0.692868 V），扫描中压降只随电流对数变化（1 V 时 0.629424 V，5 V 时 0.692869 V）。
- 后端准入用例实测（`docs/backend-evaluation.md`）：RC 瞬态对齐 `v(t)=1-e^{-t/τ}`，最差偏差 1.95e-3 V；RC 交流对齐 `H=1/(1+jωRC)`，最差偏差 5.55e-17。

## 已知限制

- **仅在 Windows MSVC 上验证**（rustc 1.98.1 / `stable-x86_64-pc-windows-msvc`）。Linux 目标（`x86_64-unknown-linux-musl` 存在编译目标）在本项目中未验证，不做跨平台声明。
- **电流探针只覆盖部分器件**：电压源与电感的支路电流由引擎直接给出；电阻的电流按欧姆定律推导，并用引擎给出的源电流交叉验证；电容、二极管、独立电流源的电流**明确拒绝**（报 `E_UNSUPPORTED`），不做近似。
- **参数扫描只支持单个参数**（`dc param:`），不支持多参数联合扫描。
- **参数只能引用其之前声明的参数**（顺序敏感），因此参数依赖不可能是环；扫描参数时每个点会检查拓扑不变，拓扑参数不允许扫描。
- **`max_step` 约束的是求解器内部步长，不是输出间隔**：返回的时间轴由求解器决定且通常非均匀。
- 悬空节点（无直流参考通路）由**前端**检查，不由后端报告：引擎的 gmin 处理会让这类节点取到看似正常的有限值并照常返回结果（实测见 `docs/backend-evaluation.md` §4.6），因此 `circuit-core::connectivity` 在展开结束时做直流参考通路可达性检查——判据是能否经**直流导通**器件到达地，电容与独立电流源不算——命中时报 `E_NAME`。同一个 body 内没有任何器件连接的节点也会被报出。
- 首期只对外开放 R / L / C / 独立电压源 / 独立电流源 / 二极管，其余器件即便后端声明支持也未在本项目验证。

## 尚未实现 / 未验证

以下**不是**本版本的能力，不得视为已支持：

- MOSFET、BJT、受控源、行为源、开关、互感。
- SPICE 网表导入 / 导出。
- 噪声、灵敏度、Monte Carlo、优化、参数拟合。
- 多参数联合扫描（仅单参数）。
- `while` 循环、递归、用户自定义函数、`include` 与外部模型文件。
- 复数的完整结果表达式（如任意传递函数表达式）。
- `uic`（跳过工作点）——后端路径未验证，未开放。
- 跨平台构建（仅 Windows MSVC 实测）。

## 后端依赖

仿真后端采用 **Thevenin 0.5.0**（BSD-3-Clause，纯 Rust，无 `-sys` 依赖，构建不需要 C 工具链）。`crates/circuit-backend/Cargo.toml` 直接依赖 `cirq-ir = "0.5.0"` 与 `thevenin = "0.5.0"`；电路以 Rust 值直接构造为 `cirq_ir::Circuit`，**不生成任何 SPICE / Cirq 源文本**，因此不存在"生成再解析"这一类错误。

选型结论、逐项实测证据（构造方式、四种分析接口、失败可识别性、并发隔离）与适配层必须处理的约束，全部记录在 `docs/backend-evaluation.md`。上面"已支持的功能"中的 RC 瞬态 / RC 交流最差偏差与二极管对照值取自该文件；其余数值为示例的直接运行结果，可用 `run` 命令复现。

## 文档索引

- [`docs/language.md`](docs/language.md) — 语言规范：词法、表达式、电路 / 实验结构、探针与测量、诊断格式，以及"尚未实现"清单。
- [`docs/architecture.md`](docs/architecture.md) — 架构说明：前端 / IR / 后端 / 结果各层的职责与数据流。
- [`docs/backend-evaluation.md`](docs/backend-evaluation.md) — 后端选型评估：准入用例、实测数据与适配层约束。
- [`docs/testing.md`](docs/testing.md) — 测试说明：测试层次、覆盖范围与运行方式。
- [`docs/repl.md`](docs/repl.md) — REPL 与语法审查：调用与语句的边界、会话作用域、多行输入的三态判定，以及哪些交互行为有测试、哪些没有。

## 项目结构

```
crates/circuit-core      语义 IR、单位与量纲、诊断码、展开 / 结果上限（Limits）
crates/circuit-dsl       词法器、语法分析器、展开器（子电路、循环 / 条件、参数覆盖、探针解析）
crates/circuit-backend   SimulationBackend 抽象、Thevenin 0.5.0 适配层、参数扫描驱动
crates/circuit-results   数据集与轴、测量求值（max/min/avg/rms）、CSV / JSON 导出
crates/circuit-session   会话状态与命令、实验执行（文件模式与 REPL 共用）
crates/circuit-cli       cdsl 二进制：check / run / repl / capabilities
_probe                   阶段 0 后端评估实验，独立于 workspace（exclude），作为可复现证据保留
examples                 七个示例：voltage_divider、rc_filter、rlc、diode_rectifier、parameter_sweep、two_stage、ladder
docs                     语言规范、架构、后端评估、测试说明
```
