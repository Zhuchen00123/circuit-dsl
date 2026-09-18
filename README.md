# circuit-dsl（`cdsl`）

## 项目简介

circuit-dsl 是一门描述模拟电路的小型领域语言，配一套纯 Rust 的实现：`.cdsl` 源文件声明电路、器件、参数与实验，命令行工具 `cdsl` 负责检查源文件、执行 op / dc / ac / tran 分析、求测量值并导出 CSV / JSON 结果。`cdsl repl` 提供交互式会话：表达式即时求值、会话变量、逐条定义电路与实验并直接运行。它面向可复现的小规模模拟电路验证：电路由结构化 IR 直接交给仿真后端构造，不生成 SPICE / Cirq 源文本，展开期的错误以带源码位置的诊断给出。工作区测试实测 `cargo test --workspace` 共 **660 个测试全部通过**（0 失败；第 3 轮为 554，第 4 轮阶段 A 冻结快照 618、阶段 B 新增 42），`cargo clippy --workspace --all-targets -- -D warnings` 0 条警告，`cargo fmt --all -- --check` 无差异。这些是**实测值**：第 3 轮的逐场景数值与退出码见 [`docs/review-evidence/round3/acceptance.md`](docs/review-evidence/round3/acceptance.md)，第 4 轮阶段 A 见 [`docs/review-evidence/round4/acceptance.md`](docs/review-evidence/round4/acceptance.md)、阶段 B 见 [`docs/review-evidence/round4/qa-acceptance-phase-b.md`](docs/review-evidence/round4/qa-acceptance-phase-b.md)（静态门禁在阶段 A 冻结快照上实测 exit 0，第 4 轮最终复跑由 lead 统一执行）；README 不再维护会漂移的历史计数。

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
# must give v(out) = 3.000 V and i(r1) = +2.000 mA.
#
# The sign is worth reading carefully: v1 sits between `in` and `gnd`, so the
# 2 mA it drives flows out of its `p` terminal into the divider, and the current
# measured in the p -> n direction is therefore -2 mA: the source delivers
# power. The current through r1, which absorbs it, is +2 mA.

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

> 上面引用的注释与 `examples/voltage_divider.cdsl` 当前内容逐字一致。实测复现（退出码 0）：
> `cargo run -q -p circuit-cli -- run examples/voltage_divider.cdsl --experiment divider --out target/lead-qa/divider --format both`。
> 示例注释承诺的 `v(out) = 3.000 V`、`i(r1) = +2.000 mA` 与实测输出一致；此处曾引用该示例注释的旧版本（写 2 V / 1 mA）并据此声称示例有误，那处说明已删除。

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
  tran1: 1019 time points; signals: v(vin), v(vout), i(input), i(r1)
  ...
  measure vfinal = 0.9932620899316276 V (tran1)
  measure vavg = 0.8013465976386364 V (tran1)
  measure vrms = 0.8382662216736428 V (tran1)
```

第 3 轮起，测量行由结果层统一渲染（`Measured::render_with_analysis`）：文件模式与 REPL 打印**同一条**
文本，`(tran1)` 是实际取值的分析标识。此前的 REPL 会把测量值换算成工程单位（`993.262 mV`），
与 `run` 命令的输出不一致；现在两者一致，单位与数值仍与 CSV/JSON 中完全相同。

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
note: Transient output time points are solver-chosen and non-uniform; `max_step` bounds the internal step, not the output interval. A declared `output_interval:` is honoured by resampling the solver's trace after the run, so it changes neither the solve nor a declared source edge.
note: Verified on Windows MSVC only.
```

**退出码契约**：`0` 成功；`1` 用户错误（文件读取失败、词法 / 语法 / 名称 / 量纲 / 展开错误、浮空节点、后端不支持的能力）。诊断写 stderr，数据与摘要写 stdout。

> 实测更正：`crates/circuit-cli/src/main.rs` 里定义了 `EXIT_INTERNAL = 2`，但**全仓库没有任何返回它的路径**（`main` 只返回 `EXIT_OK` 或 `EXIT_USER_ERROR`；7 条失败路径实测全为 1，见 `docs/review-evidence/cli-qa.md`）。因此当前版本**不会**产生退出码 2；此处曾把它写成「内部错误」，已更正。

## 已支持的功能

### 语言特性

- 顶层定义 `circuit`、`subcircuit`（必须声明 `ports`）、`experiment`，出现顺序无关。
- 层次实例 `instance :x, of: :sub, ports: { ... }, params: { ... }`；实例内部节点互相隔离，层次路径用 `.` 连接（如 `stage1.internal`）。
- `param`：`default:` 可省略；**同一 body 内前向引用合法**（一个 body 的声明先整体收集，再按依赖顺序求值，书写顺序只用来打破平局），自引用与多节点环报 `E_PARAM_CYCLE`（带闭合路径与每个参与声明的位置），未知名字仍是 `E_NAME`。覆盖顺序为默认值 → 实例 `params:` → 实验 `param:` → 扫描点 → REPL `:run name=expr`，被覆盖的参数不重新求值它的默认值。影响条件、循环次数或生成名称的参数是拓扑参数：扫描它在 `check` / `run` / `:load` 阶段就报 `E_TOPO_PARAM`（带"被扫描参数 → 中间参数 → 使用点"的解释路径），只出现在数值位置的扫描仍然可用。
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
- `tran start:, stop:, max_step:, output_interval:`（`start` 可省略，默认 0；`output_interval` 可省略）。`max_step` 约束**积分步长**，`output_interval` 只控制**输出采样**：它不是求解器参数，而是求解结束后对求解器自己的时间轴做独立重采样（首点与末点保留，内部点等间隔，线性插值，不越界外推；超结果规模报 `E_LIMIT`）。因此改 `output_interval` 不会改变已声明的激励波形，也不会改变 `avg` / `rms` 等测量——测量一律用**原始求解网格**。省略 `output_interval` 时导出求解器自己的时间点。
- 一个实验可声明多个分析，结果按分析 ID 分开导出，互不覆盖。

### 结果与导出

- 探针：`save v(:n)`、`v(:a, :b)`（= Va − Vb）、`i(:dev)`（正方向 `p → n`）；测量：`measure :vmax, max: v(:out)`，归约可选 `max` / `min` / `avg` / `rms`。
- **结果表达式（第 3 轮新增）**：`derive :name, expr: <表达式>` 导出命名派生信号，`measure` 的目标可以是表达式，例如 `measure :avg_power, avg: v(:vin, :vout) * i(:r1)`。表达式支持探针、无量纲常量、括号、一元 `+`/`-`、`+ - * /` 与 `abs` / `sqrt` / `min` / `max` / `gain_db`。表达式用到的探针**自动读取**，不要求写 `save`；隐式探针只参与求值，不进入导出列。多分析实验用 `analysis: :ac1`（`{kind}{序号}`，如 `ac1`、`tran2`）显式绑定；只有一个分析时可省略；不写且无法唯一确定时报 `E_AMBIGUOUS`，绝不按「哪个跑成功」猜。不带 `analysis:` 的旧式直接探针测量保持原有 TRAN → AC → DC → OP 选择顺序。派生信号与测量都在**原始求解网格**上求值，之后才重采样，所以 `output_interval` 不改变测量值，也不会先把信号插值再算非线性表达式。实测 RC 截止频率点：`gain = 0.5 − 0.5j`、`|gain| = 0.7071067811865472`、`gain_db = −3.010299956639815`；`avg: v(:vin,:vout) * i(:r1)` 与独立 `v²/R` 梯形积分相对偏差 1.3e-16。
- `avg` / `rms` 在**非均匀求解器时间轴**上按时间积分，而不是样本算术平均。实测 `examples/rc_filter.cdsl`：`vavg = 0.8013466 V`、`vrms = 0.8382662 V`（`vfinal = 0.9932621 V`）。
- CSV：第一列为轴；复数列拆成 `_re` / `_im`（实测 `response.ac1.csv` 表头 `frequency,v(vin)_re,v(vin)_im,...`）；时间轴非均匀（同一实验 1019 个时间点，逐点由求解器决定）。
- **表达式错误策略（第 4 轮）**：结果表达式的**每个运算节点**都校验自己产出的样本——实数 `is_finite()`、复数实部与虚部都有限；`sqrt` 的负样本、分母精确为 0、`gain_db` 零幅值都是错误，**没有 epsilon、没有饱和、没有跳过样本**，所以 `min(sqrt(-1), 2)` 在 `sqrt` 处失败、不会被 `min` 掩盖，`1e308 * 1e308` 也不会让 `inf` 继续参与计算。**常量**表达式（不读信号）在 `cdsl check` 阶段就被同一个求值器拒绝（`sqrt(-1)`、`1e308*1e308`、`x/0`、`gain_db(0, x)`）；读信号的表达式只做静态量纲检查，在 `cdsl run` 报错。诊断带 `analysis`/`kind`/`signal`/`sample`/`index`，标量分析会说明没有轴坐标。
- **量纲指数溢出是诊断**：量纲指数是 `i8`（`-128..=127`），`*` `/` 走受检算术，超出范围报 `E_DIMENSION`；128 个 `v(:vin)` 因子在 debug 与 release 都是 exit 1，不 panic、不回绕。
- **表达式深度上限 256**（`circuit_core::limits::MAX_EXPR_DEPTH`）：更深的表达式报 `E_LIMIT`；`cdsl` 的每条子命令都在 64 MiB 栈的线程上运行，所以被接受的深度在 debug 构建里也能处理（FINDING-1）。
- JSON：保留单位、轴类型与后端元数据；非有限值导出为 `null`（CSV 为空字段）。渲染期警告（每个非有限样本一条）由 `cdsl run` 与 REPL 打印；`<file>.json` 里的 `diagnostics` 数组是**数据集自己的**来源诊断，与这批渲染警告互补、不是同一批，同一个数据集写 CSV+JSON 只报一次。
- 本仓库实测数值：分压器 OP 精确给出 `v(out)=3 V`、`i(r1)=+2 mA`、`i(v1)=-2 mA`；参数扫描逐点等于 `3·1.5k/(r+1.5k)`（实测 r=500 Ω → 2.25 V、1 kΩ → 1.8 V、2 kΩ → 1.285714… V）；`examples/diode_rectifier.cdsl` 的 `op` 给出 `v(vout)=0.692872 V`，其 DC 扫描在 5 V 点给出 0.692869 V（对照 Shockley 方程二分法独立解 0.692868 V），扫描中压降只随电流对数变化（1 V 时 0.629424 V，5 V 时 0.692869 V）。
- 后端准入用例实测（`docs/backend-evaluation.md`）：RC 交流对齐 `H=1/(1+jωRC)`，最差偏差 5.55e-17。
- RC 瞬态用**有延迟的有限斜坡**与匹配的分段解析解对照（见 `docs/backend-evaluation.md` §5）：基线配置 `tmax=τ/1000` 下 6025 个返回点**逐点**满足 TRAN 判据（`atol=1e-5 V`、`rtol=1e-3`），最大误差 **4.999167e-7 V**。两处历史结论都已被本轮证据取代：(a) 旧文档的「对齐 `1-e^{-t/τ}`，最差偏差 1.95e-3 V」是**参考模型错**——当时的适配层把 `.tran` 的 step 取成输出间隔，使引擎把声明的 1 ps 上升沿夹紧到 500 ns，用理想阶跃当参考必然得到 `C·e^{-t/τ}`（`C≈-2.504e-3 V`）的残差；(b) 适配层现在按声明边沿选 step，声明值会被兑现（`rise=10.ns`、`output_interval` 1 ns→100 ns 时原始求解网格逐位相同，约 50 ns 处 `v(vin)=1 V`）。
- **输出间隔不改变物理解**（本轮回归）：同一电路 `rise=fall=10.ns`、`max_step=1.ns`、`stop=2.us`，`output_interval` 取 1 ns 与 100 ns 时原始求解时间轴与全部样本**逐位相同**；`output_interval` 只改变输出网格（100 ns → 21 点、首末点等于原始首末点）与 `avg` / `rms` / `max` / `min` **无关**。修复前同一对照在约 50 ns 处给出 `v(vin)=0.5002375 V`（`docs/review-evidence/round2/repro-baseline.md`），修复后为 `1 V`。
- `examples/rc_filter.cdsl` 的 `tran`（`stop: 500.us, max_step: 500.ns`，未给 `output_interval`）实测 1019 个输出点，末点 `t=500 us` 时 `v(vout)=0.9932620899316276 V`，与 `1-exp(-t/τ)` 的 `0.993262053000915 V` 相差 **3.7e-8 V**——声明 `rise: 1.ns` 现在被真正兑现（修复前该文件是 500 ns 斜坡，同一点误差约 2.5e-3 V）。

## 已知限制

- **仅在 Windows MSVC 上验证**（rustc 1.98.1 / `stable-x86_64-pc-windows-msvc`）。Linux 目标（`x86_64-unknown-linux-musl` 存在编译目标）在本项目中未验证，不做跨平台声明。
- **电流探针只覆盖部分器件**：电压源与电感的支路电流由引擎直接给出；电阻的电流按欧姆定律推导，并用引擎给出的源电流交叉验证；电容、二极管、独立电流源的电流**明确拒绝**（报 `E_UNSUPPORTED`），不做近似。
- **参数扫描只支持单个参数**（`dc param:`），不支持多参数联合扫描。
- **参数依赖是 DAG，不是声明顺序**：同一 body 内前向引用合法，声明先整体收集再按依赖顺序求值；环报 `E_PARAM_CYCLE`（带闭合路径与每个参与声明的位置），不是 `E_NAME`。扫描参数时每个点仍会检查拓扑不变，而拓扑参数更早在 `check` / `run` / `:load` 阶段就被 `E_TOPO_PARAM` 拒绝（带解释路径）；只改元件数值的扫描照常可用。
- **表达式深度上限 256**：超过 `MAX_EXPR_DEPTH` 报 `E_LIMIT`；`cdsl` 在 64 MiB 栈线程上运行每个子命令，这条上限同时保护解析器与结果表达式求值器，不会以栈溢出终止进程。**边界**：在更小栈上直接嵌入本项目的 crate 时，需要调用方自己提供同样的栈（`docs/review-evidence/round4/findings.md` FINDING-1 的边界）。
- **REPL 定义期不预求值常量表达式**：`cdsl check` 会拒绝 `sqrt(-1)` 这类常量表达式，而 REPL 在定义实验时只做静态检查，`:run` 时才给出同一条 `E_VALUE` 诊断（两处最终文本一致）。
- **`max_step` 约束的是求解器内部步长，不是输出间隔**：返回的时间轴由求解器决定且通常非均匀。要固定输出间隔请用 `tran output_interval:`——它在求解后重采样，不参与求解。
- 悬空节点（无直流参考通路）由**前端**检查：引擎对**真无参考**的线性网络会以 `matrix is singular, cannot solve` 失败，但该错误不指向任何节点，也无法区分「合法开路输出」与「真正无参考」（实测见 `docs/review-evidence/floating-audit.md`；旧文档把它写成「gmin 把节点拉住并返回 Ok」，已废弃）。因此 `circuit-core::connectivity` 在展开结束时做直流参考通路可达性检查——判据是能否经**直流导通**器件到达地，电容与独立电流源不算——命中时报 `E_NAME`。同一个 body 内没有任何器件连接的节点也会被报出。
- **PULSE 的 `rise` / `fall` / `period` 不会被分析选项展宽**：引擎把 PULSE 的上升/下降时间夹紧到 `.tran` 的 print step（`tr.unwrap_or(tstep).max(tstep)`），而适配层现在把该 step 选为 `min(span/1000, 电路中声明的最小 rise/fall/period)`，且**完全不受 `output_interval` 影响**，所以已声明的边沿会按声明值执行（`examples/rc_filter.cdsl` 的 `rise: 1.ns` 不再是 500 ns 斜坡）。若声明的边沿是 0 或非有限（引擎没有理想零宽边沿），或在给定窗口内执行需要超过 1e6 个求解步，`cdsl check` / `cdsl run` 会给出明确的能力诊断（`E_UNSUPPORTED` / `E_LIMIT`）而不是静默展宽。实测与源码依据见 `docs/review-evidence/round2/`。
- **运行中的源断点附近精度受求解器重启步限制**：引擎在源波形断点（PULSE 的 delay、上升结束、下降开始、周期边界等）之后的第一个接受步强制 Backward-Euler，并把步长缩到 `step_h.min(h*0.1)`；由此产生的局部误差约为 `(V0/T)·h1²/(2τ)`（`T` 为声明边沿宽度、`τ` 为电路时间常数、`h1` 为该重启步）。τ=100 µs、T=1 µs 的 RC 实测：`max_step=τ/1000` → 0/3129 点超 §17（最大误差 4.999167e-7 V）、`τ/500` → 0/1629、`τ/200` → 3/729 超限（最大 1.248959e-5 V）、`τ/50` → 250/309 超限（最大 7.331775e-4 V）。超限配置原样保留为限制，不删除样本、不放宽阈值；上述界只针对该激励推导，不构成任意电路的精度保证。详见 `docs/review-evidence/round2/breakpoint-evidence.md`。
- **求解器容差没有产品通道**：适配层 `build_circuit` 把 Thevenin 的 `options` 传成空，因此 `RELTOL` / `ABSTOL` / `VNTOL` / `GMIN` 恒为引擎默认（`reltol = 1e-3`、`abstol = 1e-12`），DSL 也没有对应语法。容差只影响 `max_step` 未钉住步长时的 LTE 步长控制；本轮在 `_probe`（直接构造 `cirq_ir::Circuit`）里做过受控实验：`RELTOL` / `ABSTOL` / `TRTOL` 单因子不可观测，`RELTOL+ABSTOL` 交互会改变轨迹但**不改变断点重启步与 §17 结论**。适配层新增容差通道属于后续工作。
- **`--out` 指向一个已存在的普通文件时**：报的是「无法创建目录」而不是 `guard_output` 的防覆盖诊断，退出码仍为 1，输入源文件不会被改写（`docs/review-evidence/cli-qa.md` 的 F6）。
- 首期只对外开放 R / L / C / 独立电压源 / 独立电流源 / 二极管，其余器件即便后端声明支持也未在本项目验证。
- **复数没有隐式大小顺序**：`max` / `min` 不能直接归约复数表达式（AC 的 `v(...)` 是复数），必须先写 `abs(...)` / `gain_db(...)`；直接归约在 check 阶段（显式绑定 AC 分析）或运行阶段报 `E_TYPE`，不会静默取模。
- **参数扫描实验只能绑定被扫描的那个分析**：`dc param:` 逐点重跑并拼接出唯一数据集（文件名 `dc_param_<参数>`），把派生信号或测量绑定到同实验的其它分析会在求解前被明确拒绝（`E_UNSUPPORTED`），不会静默丢弃。

## 尚未实现 / 未验证

以下**不是**本版本的能力，不得视为已支持：

- MOSFET、BJT、受控源、行为源、开关、互感。
- SPICE 网表导入 / 导出。
- 噪声、灵敏度、Monte Carlo、优化、参数拟合。
- 多参数联合扫描（仅单参数）。
- `while` 循环、递归、用户自定义函数、`include` 与外部模型文件。
- 派生信号之间的引用：`derive` 只能引用探针与常量，不能引用另一个 `derive`，也不能跨分析取数据（首期实现边界，见 `docs/language.md` §7）。
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
crates/circuit-dsl       词法器、语法分析器、展开器（参数依赖图、子电路、循环 / 条件、参数覆盖、探针解析）
crates/circuit-backend   SimulationBackend 抽象、Thevenin 0.5.0 适配层、参数扫描驱动
crates/circuit-results   数据集与轴、测量求值（max/min/avg/rms）、CSV / JSON 导出
crates/circuit-session   会话状态与命令、实验执行（文件模式与 REPL 共用）
crates/circuit-cli       cdsl 二进制：check / run / repl / capabilities
_probe                   阶段 0 后端评估实验，独立于 workspace（exclude），作为可复现证据保留
examples                 七个示例：voltage_divider、rc_filter、rlc、diode_rectifier、parameter_sweep、two_stage、ladder
docs                     语言规范、架构、后端评估、测试说明
```
