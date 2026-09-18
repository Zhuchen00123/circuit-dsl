# 架构

> 本文描述**当前实现**的结构。每一条结论都能在 `crates/` 里找到对应代码；
> 未实现的能力在 §9 和 `docs/language.md` §10 中单独列出，不在本文声称。

## 0. 工作区

根 `Cargo.toml` 声明 6 个成员 crate：

```
crates/circuit-core       语义数据结构、量纲与数值显示（无第三方仿真依赖）
crates/circuit-dsl        lexer / parser / 展开 / 共享求值器 / 参数依赖图 / 完备性判定
crates/circuit-backend    后端契约 + Thevenin 适配 + 参数扫描驱动
crates/circuit-results    结果数据集 / 表达式 / 测量 / 导出
crates/circuit-session    会话状态与命令 + 实验执行（文件模式与 REPL 共用）
crates/circuit-cli        cdsl 命令行
```

`_probe` 是 Phase-0 的后端评估工具，被根 `Cargo.toml` 用 `exclude` 排除在产物之外，
作为可复现证据保留（对应 `docs/backend-evaluation.md`）。它是独立 workspace，
不参与本项目的构建与测试。

实际依赖（`crates/*/Cargo.toml`）：

| crate | 工作区内依赖 | 第三方 |
|---|---|---|
| `circuit-core` | — | `thiserror` |
| `circuit-dsl` | `circuit-core` | `thiserror` |
| `circuit-results` | `circuit-core` | `thiserror`, `serde`, `serde_json` |
| `circuit-backend` | `circuit-core`, `circuit-results` | `cirq-ir`, `thevenin`, `thevenin-types`, `thiserror` |
| `circuit-session` | `circuit-core`, `circuit-dsl`, `circuit-backend`, `circuit-results` | — |
| `circuit-cli` | `circuit-core`, `circuit-dsl`, `circuit-backend`, `circuit-results`, `circuit-session` | `clap`, `serde_json`, `thiserror`, `rustyline` |

## 1. 分层结构

```
.cdsl 源文本
  │  circuit-dsl::lexer::lex
  ▼
Vec<token::Token>
  │  circuit-dsl::parser::parse
  ▼
ast::Program                        名字未解析、量纲未检查
  │  circuit-dsl::elaborate::{compile, elaborate_experiment}
  │  参数 DAG · 名字解析 · 量纲检查 · 参数求值 · 子电路/端口展开 · for/if 展开
  ▼
(circuit_core::ir::Circuit, circuit_core::plan::AnalysisPlan)
  │  计划层已含结果表达式 IR 与分析绑定（ExprIr/ProbeRef/AnalysisBinding）与每个分析的
  │  save 探针 + 隐式探针依赖（plan.rs）
  │  circuit-backend::thevenin::TheveninBackend
  │  内存中的结构映射（不生成任何源文本）；按名选 plot、探针子集化（save + 隐式依赖）、
  │  电阻电流推导；数据集名 = 分析标识 {kind}{ordinal}
  ▼
cirq_ir::Circuit（每个分析任务单独构造，只含一个 Analysis）
  │  thevenin::circuit::{simulate_op, simulate_dc, simulate_ac, simulate_tran}
  ▼
SimResult / SimPlot / SimVector
  │  轴构造 + 单位/复数类型转换（thevenin_types::Complex → circuit_results::Complex）
  ▼
circuit_results::Dataset（axis + signals + 单位 + backend 元数据）
  │  expr::from_ir 降级 + expr::eval：在原始网格上求值 derive，把派生列追加进数据集
  │  measure::reduce：在原始网格上归约，Measured 记录值取自哪个分析
  │  输出视图：导出信号 + 派生列；有 output_interval: 时重采样（resample）
  │  to_csv / to_json
  ▼
cdsl run 的 stdout 摘要与结果文件
```

参数扫描是旁路：`cdsl run` 检出 DC 参数扫描后走
`circuit_backend::sweep::run_parameter_sweep`，每个扫描点重新展开一次，再进入上面
`Circuit → 后端` 那一段，最后由 `circuit_session::execute` 的 `stitch` 拼成单个 Dataset
（轴是扫描值）。详见 §7。

**依赖方向**：`core <- results <- backend <- session <- cli`，另有 `core <- dsl <- session`。

- `backend <- session`：会话（以及文件模式的 `run`）通过 `circuit_session::execute`
  驱动后端。执行流程放在这一层而不是 CLI 里，是因为参数扫描要按点重新展开设计，
  那需要 `circuit-dsl` 的 `Program`——而 `circuit-backend` 不依赖 DSL。
- `session` **不依赖任何终端库**：`rustyline` 只在 `circuit-cli` 里出现，所以会话
  逻辑可以直接在测试里驱动（`crates/circuit-session/tests/session.rs`），
  也便于以后接编辑器。

- `core <- results`：`Dataset`/`Signal` 用 `Dimension` 标注单位，用 `Diagnostic` 报错，
  用 `Limits` 约束结果规模（`crates/circuit-results/src/lib.rs`）。
- `core <- results` 在本轮多了一条**表达式的**边界：结果表达式的**计划层 IR**
  （`ExprIr` / `ProbeRef` / `AnalysisBinding` / `DeriveRequest` / `MeasureRequest`）在
  `circuit-core::plan`，**运行期 AST 与求值器**在 `circuit-results::expr`；两者之间只有
  `expr::from_ir` 这一次显式降级，所以 `circuit-core` 仍然不依赖 `circuit-results`，
  而分析计划可以脱离求值器被打印、检查与单测（`plan.rs` 的 `ExprIr` 方法与
  `crates/circuit-cli/src/check.rs` 的 JSON）。
- `results <- backend`：适配层把引擎自己的 `SimPlot`/`SimVector`/`Complex` 转换成中立的
  `Dataset`，这个转换点就是两条边存在的理由（`crates/circuit-backend/src/thevenin.rs`）。
- `backend <- cli`：只有 CLI 需要"真的跑仿真"；`cdsl check` 走到后端只做能力校验
  （`crates/circuit-cli/src/check.rs` 调用 `backend.validate`），不执行任何分析。
- `core <- dsl`：前端只依赖 IR 与诊断类型，因此它能独立于任何求解器编译、测试。

**为什么 `circuit-core` 不能依赖后端**：`core` 是前端与 IR 共享的词汇表。一旦它依赖
Thevenin，IR 就会按某一个引擎的类型塑形，第二个后端无法接入，错误格式也会被第三方
crate 左右；`Circuit` 的字段会混进 solver 句柄，`check --json` 的输出就不再稳定。
`crates/circuit-core/Cargo.toml` 的依赖只有 `thiserror`。

**为什么 `circuit-results` 也不能**：`Dataset` 是"仿真"与"后处理"之间的中立契约。
如果它依赖后端，表达式求值、测量与 CSV/JSON 导出都会被迫拖入求解器，单元测试必须
起一个引擎才能跑，导出格式也被绑死在某一家引擎的结果类型上。因此复数类型是本项目
自己的 `circuit_results::Complex`（`crates/circuit-results/src/dataset.rs`），
转换发生在适配层。

## 2. 各 crate 的职责

### `circuit-core`

- **拥有**：`span`（字节偏移、source map、诊断渲染）、`diagnostic`（`Diagnostic` /
  `Diagnostics` / 稳定的 `Code`）、`units`（`Dimension`、`Quantity`、单位后缀解析）、
  `id`（`NodeId` / `DeviceId` / `ModelId` / `AnalysisId` / `CircuitId`，`GROUND`）、
  `ir`（`Circuit` / `Device` / `Node` / `Model` / `SourceSpec` / `Waveform`）、
  `plan`（`AnalysisPlan` / `AnalysisTask` / `Probe` / `Sweep` / 各分析规格，以及结果表达式的
  计划层 IR：`ExprIr` / `ProbeRef` / `AnalysisBinding` / `DeriveRequest` / `MeasureRequest`；
  `AnalysisTask` 同时带 `probes`（用户 `save` 的）与 `implicit_probes`（表达式依赖））、
  `limits`（`Limits`）。
- **不拥有**：词法/语法（`dsl`）、结果语义（`results`）、求解（`backend`）、
  CLI 的输出与退出码策略（`cli`）。
- **公开入口**：上列类型；`Circuit::new` 是唯一带校验的构造器，
  `Circuit::{node_id, device_id, model_id, node_name, summary}` 等查询。

### `circuit-dsl`

- **拥有**：`lexer`（含量纲字面量的词法）、`parser`（递归下降，语句关键字按文本分派；
  `parse` 是文件入口，`parse_input` 是 REPL 的单条输入入口；表达式按
  `MAX_EXPR_DEPTH` 做嵌套与形态双护栏）、`ast`（语法树，名字是
  字符串、每个表达式带 span）、`eval`（表达式求值器，**唯一**一套值语义，见下）、
  `complete`（多行输入的三态判定）、`param_graph`（参数依赖图：一个 body 的节点/边、
  确定性拓扑序、环报告、拓扑使用点的反向闭包）、`elaborate`
  （名字解析、量纲检查、参数 DAG 求值、层次展开、循环与条件展开、分析计划构造、
  结果表达式降级成 `ExprIr`、分析绑定解析、隐式探针依赖收集、结果重名检查与
  check 期拓扑参数扫描拒绝）。
- **不拥有**：任何仿真、文件访问或网络访问。整个 crate 是纯函数式的
  （`crates/circuit-dsl/src/lib.rs`）；DSL 不参与仿真过程，展开完成后拓扑固定。
- **公开入口**：`lex`、`parse`、`parse_input`、`assess`、`compile`（整个文件的全部
  circuit 与 experiment）、`elaborate_experiment`（单实验 + 覆盖值，参数扫描每点调用它）、
  `Elaborated`、`Compiled`、`Program`。`circuit_parameters` 已定义但当前没有任何调用方。

**结果表达式在展开期定型**：`derive` 与 `measure` 先按源码顺序收集（`RawDerive` /
`RawMeasure`），等实验的**全部**分析任务登记完之后才解析 `analysis: :ac2` 这类标识——
这正是"一个分析可以省略绑定、多个分析必须写明"得以实现的原因；同一步还完成静态量纲检查
（`ExprIr::static_dimension_error`）、`avg`/`rms` 需要时间轴、AC 上 `max`/`min` 需要
`abs(...)`、以及 `derive` 与 `measure` 共用命名空间的重名检查（`claim_result_name`；
`save` 的信号名带探针形式，与派生名不可能相撞，见 `docs/language.md` §7.3）。
- **共享求值器（`src/eval.rs`）**：`eval::eval(&Expr, &dyn Variables)` 是文件模式与
  REPL 共用的表达式语义——量纲传播、比较、短路、内置函数、拼接规则都在这里。
  名字查找走 `Variables` trait：展开器传自己的参数作用域（并借此汇报"参数在此声明"
  的次要标签），会话传它的变量表。`elaborate.rs` 保留一层同名薄包装，调用点读起来
  与从前一致，但实现只有一份。这是本轮刻意的结构调整：交互式前端若自带一套求值器，
  迟早会在"这个程序是什么意思"上跟仿真器分道扬镳。

### `circuit-backend`

- **拥有**：`backend`（`SimulationBackend` trait、`BackendCapabilities`、
  `classify_backend_failure` / `backend_failure`）、`thevenin`（唯一的实现，
  结构映射 + 结果物化）、`sweep`（参数扫描驱动 + 拓扑不变性检查）。
  `convert_plot` 物化的是 `task.read_probes()`（`save` 探针 + 表达式依赖），
  导出集合仍由 `task.probes` 决定；数据集的名字与 `analysis` 字段是
  `{kind}{ordinal}`，也就是 `docs/language.md` §7.4 的分析标识。
- **不拥有**：结果数据模型。`Dataset`、`Signal`、`Axis` 都在 `circuit-results`；
  适配层只负责把引擎输出搬进这些类型。
- **公开入口**：`TheveninBackend::{new, with_limits, capabilities, validate, run}`、
  `BACKEND_VERSION`（`"0.5.0"`）、`run_parameter_sweep`、`sweep_coordinates`、
  `SweepOutcome`、`SweepError`。
  `branch_current_to_p_to_n` 与 `node_index` 是公开的自由函数，当前没有任何调用方；
  见 §5 的说明。

### `circuit-results`

- **拥有**：`dataset`（`Dataset` / `Axis` / `Signal` / `Data` / `Complex` /
  `BackendInfo` / `normalize_signal_name`）、`expr`（结果表达式 AST、求值器与
  `from_ir` 降级，作用在整个采样向量上；`EvalSite`/`eval_at` 把 `derive`/`measure`
  名字带进诊断，`is_constant`/`eval_constant` 供 check 期常量判定；每个运算节点校验
  自己产出的样本有限性，`sqrt` 定义域、精确零分母、`gain_db` 零幅值是确定性错误，
  乘除走受检量纲，求值前还有一道深度护栏）、
  `measure`（`max` / `min` / `avg` / `rms`，`Measured` 记录值、单位与来源分析标识）、
  `export`（CSV/JSON、`SCHEMA = "circuit-dsl.result/1"`、非有限值警告）、
  `format_number`。
- **不拥有**：后端类型（`thevenin_types` 只在 `circuit-backend` 出现）、
  仿真调度、CLI 输出。
- **公开入口**：上列类型与 `measure` / `measure_signal` / `reduce` / `to_csv` /
  `to_json` / `to_json_value` / `non_finite_diagnostics` / `eval` / `expr::from_ir` /
  `Measured::{render, render_with_analysis, with_analysis}`。

### `circuit-cli`

- **拥有**：`cdsl` 二进制、四个子命令（`check`、`run`、`repl`、`capabilities`）、
  退出码（0 成功 / 1 用户错误 / 2 内部错误）、"诊断走 stderr、数据与摘要走 stdout"、
  结果文件命名与"拒绝写回输入文件"（`guard_output`）、终端层（提示符、行编辑、
  历史、补全，`repl.rs`）。`check` 还会用与运行期同一个求值器算出**常量**结果表达式
  （`check_constant_expressions`），所以 `sqrt(-1)` 这类非法常量在 check 阶段就是
  exit 1。`main` 把每条子命令都放到 **64 MiB 栈**的工作线程上（`WORK_STACK_BYTES`），
  与 `MAX_EXPR_DEPTH` 配对：被接受的表达式深度在 debug 构建里也不会耗尽栈。
- **不拥有**：语言语义、仿真细节、结果格式定义、会话状态与执行流程（都在
  `circuit-session`）。`run.rs` 现在只负责文件 I/O、写出与摘要打印。
- **产物入口**：`cdsl` 可执行文件（`check` / `run` / `repl` / `capabilities`）。
  crate 内部的 `check::front_end` 是 `check` 与 `run` 共用的前端入口
  （读文件 → lex → parse → compile → 后端能力校验），并把解析好的 `Program`
  一并返回，于是扫描路径不必二次读盘解析。

### `circuit-session`

- **拥有**：会话的定义集合与会话变量、定义替换的原子性规则、`:load` / `:run` /
  `:list` / `:reset` / `:help` 的行为、REPL 的值显示格式（`format.rs`）、
  以及实验执行 `execute`（单次 / 参数扫描 / 测量求值 / 结果写出）。
  `execute` 的顺序是：后端求解 → 在**原始**网格上追加派生信号（`attach_derived`）→
  求值测量 → 组装输出视图 → 必要时重采样；参数扫描里绑定到非扫描分析的语句在**求解前**
  被拒（`check_sweep_bindings`，`E_UNSUPPORTED`）。
- **不拥有**：终端交互（在 `circuit-cli/src/repl.rs`），语言语义（在 `circuit-dsl`）。
- **为什么不放进 CLI**：这样会话行为可以在没有 TTY 的情况下测试，且文件模式与
  REPL 共用同一条执行路径——两套实现迟早会对同一个实验给出不同答案。

## 3. Circuit IR

`circuit_core::ir` 是**语义** IR：名字已解析、参数已求值、层次已展平，但对后端一无所知。

- `Node { id, name, local_name, kind, span }`：`name` 是展平后的唯一名（`stage1.internal`），
  `local_name` 是它在自己 body 里的写法（`internal`）。`NodeKind::Ground` 的节点永远是
  `NodeId(0)`、名为 `gnd`；`Circuit::node_id("0")` 与 `"gnd"` 都解析到地。
- `Device { id, kind, local_name, name, terminals, params, model, source, def_span,
  instance_path }`：`terminals` 顺序固定。两端器件是 `[("p", …), ("n", …)]`，
  二极管是 `[("anode", …), ("cathode", …)]`；端子名常量集中在 `ir::terminal`，
  防止前后端各写一套字符串。
- `Model`：首期只有 `ModelKind::Diode`（`type: :diode`，参数 `is`、`n` 等）。
- `SourceSpec { dc, ac, waveform }`：三者相互独立，可以同时存在（`rc_filter` 例子就是
  `dc: 0.V` + `ac: 1.V` + `pulse(...)`）。`Waveform` 有 `Pulse` / `Sin` / `Pwl` 三种。
- **方向约定 `p -> n`**：`Device::pos()` / `neg()` 返回正/负端（二极管回退到
  anode/cathode），器件电流的正方向就是 `p -> n`，`docs/language.md` §4.1 与
  `Probe::DeviceCurrent` 都用这一条。适配层直接沿用引擎的 `pos -> neg` 方向，
  不做符号翻转（`thevenin.rs::branch_current_to_p_to_n`，当前无调用方）。

**量纲是运行期值，不是幽灵类型参数。** brief 的草图是 `Quantity<Dimension>`；本实现用
`Quantity { value: f64, dimension: Dimension }`（`crates/circuit-core/src/units.rs`）。
理由：（1）展开器本质是一个动态求值器，`Value::Num(Quantity)` 要和
`Bool` / `Sym` / `Str` / `Array` / `Dict` 放在同一个 `Value` 枚举里，幽灵类型会强迫求值器
对每种算术组合单态化，并让 `Value` 变成泛型；（2）量纲检查的错误路径需要**同时打印**
expected 与 received（`E_DIMENSION` 的 `= expected: ohm` / `= received: s`），
运行期值天然带着这两个信息，而类型级方案要在类型擦除后再还原；
（3）brief 要求观察到的行为是"量纲不匹配必须报错并给出两侧量纲"，这一点两种方案等价，
`units.rs` 的测试（`addition_requires_matching_dimensions`、
`dimensionless_is_required_explicitly`）与 `docs/language.md` §8 的示例覆盖的是同一个行为。
`Dimension` 只有 (volt, amp, second) 三个指数——刻意不建模长度、温度等，因为支持的
器件不需要它们（`units.rs` 模块注释）。指数的类型是 `i8`（`MIN_EXPONENT = -128`、
`MAX_EXPONENT = 127`），`Dimension`/`Quantity` 的 `*` `/` 只提供
`checked_mul` / `checked_div`（`Quantity` 不再实现 `Mul`/`Div` 运算符），
调用点必须把 `None` 变成 `E_DIMENSION`：这就是 128 个电压因子在 debug 不 panic、
release 不回绕的原因（R4-02）。

**IR 里没有矩阵槽位，也没有后端句柄。** 三个可见的好处：（1）后端替换不改前端——
适配层在 `cirq_ir::Circuit` 构造时重新分配 id（`nets` 用同一套稠密索引，元素 id 直接映射），
IR 自己不知道 Thevenin 存在；（2）诊断定位靠 `SourceSpan`，不是求解器内部索引，
`InstanceStep` 还保留"写在子电路里、实例化在这里"的调用链；（3）`check --json` 的输出
（`crates/circuit-cli/src/check.rs::circuit_json`）是手写的，不引入 serde 依赖，
IR 的字段就是对外契约。

已知缺口：`Device.instance_path` 里的 `InstanceStep.of`（子电路名）目前恒为空串，
构造时只填了 `instance`；`instance_display` 不使用它，所以诊断不受影响，
但这个字段现在没有信息。

## 4. 展开（elaboration）

入口是 `compile`（整个文件）和 `elaborate_experiment`（单实验，可带覆盖值）。
两条贯穿全模块的规则写在 `elaborate.rs` 头部：不猜（未声明的名字是错误，绝不会
隐式建节点；量纲不符是错误，绝不静默转换）、错误累积（一次运行报告尽量多的独立问题）。
但 `run_body` 在一个 body 内遇到第一个结构性错误就停止，避免用已经坏掉的 scope 继续
产生噪声。

### 4.1 参数 DAG 与覆盖顺序

- `param` 只能在 body 内声明。一个 body 的声明先由**预扫描**整体收集，交给
  `param_graph`（`crates/circuit-dsl/src/param_graph.rs`）建成依赖图：节点是
  **一个 body 实例里的一个参数**（`ScopePath` + 名字，`top`、`top.stage1`…），
  边是"这个默认值的有效定义读了那个参数"。求值按**确定性拓扑序**：依赖先算，平局按
  声明顺序打破，所以同一个输入永远展开成同一个结果。
- **前向引用合法**：`param :b, default: 2 * a` 写在 `param :a` 之前也能解析
  （`docs/language.md` §4.2 有可运行例子）。自引用与多节点环报 `E_PARAM_CYCLE`，
  消息含闭合路径（`a -> a`、`a -> b -> a`），主标签落在闭合的引用处，每个参与声明的
  `param` 行各带一个次标签；未知名字仍是 `E_NAME`，图只负责把两者分开
  （`param_graph::BodyGraph::unknown_reads`）。
- **边只在一个 body 内**：子电路默认值看不到父作用域；唯一的跨作用域通道是实例的
  `params: { .. }`——值在父作用域求值，并成为实例内同名参数的有效定义
  （`DesignGraph::bind_from_parent`）。因此同名参数在不同实例里是两个节点，
  扫描 `top.r` 不会牵连 `top.stage1.r`，除非 `params:` 真的把它们连起来。
- 覆盖顺序是 **默认值 → 实例 `params:` → 实验 `param:` → 扫描点 →
  REPL `:run name=expr`**（后者胜出），实现方式不变：`Scope::vars` 在 body 执行**之前**
  就被覆盖链填好，`param` 语句里的 `default:` 只有在 `vars` 里还没有该名字时才写入
  （`stmt_param`）。被覆盖的参数**不产生边、也不求值它的默认值**——有效定义先选出，
  再建图。默认值仍会被记入 `scope.defaults`，用于检查覆盖值的量纲：`param :r, default:
  1.kohm` 配合实例覆盖 `r: 2.kohm` 得到 2 kΩ。`cdsl check` 仍然按每个顶层电路自己的
  默认值单独展开一遍（`compile()` 用空覆盖链），所以默认值本身写错、只被实验覆盖的电路
  仍会在 `check` 阶段报 `E_NAME`（契约 §4.3 记录的边界）。
- 实例覆盖写在**外层 scope** 里求值，所以 `params: { r: rstage }` 可以引用父电路的参数；
  子电路自己的默认值作为兜底折入 `eval_scope`，且每个被接受的覆盖值立即加入
  `eval_scope`，于是后面的条目可以引用前面的（`stmt_instance` 中的注释与代码）。
- 实验级 `param :x, value: …` 只在"更早的实验级覆盖"构成的 scope 里求值，
  看不到电路参数（`experiment_overrides`）。`elaborate_experiment` 收到的扫描覆盖值
  按名字替换实验自身的覆盖值——"后者胜出"，这就是覆盖链最后一环。
- **拓扑使用点与 check 期拒绝**：`if` 条件、`for` 的迭代源、生成名称与端子都会向
  `DesignGraph` 记一个使用点（`note_use`）；从使用点沿依赖图**反向闭包**得到拓扑参数
  集合。实验声明 `dc param: :n` 时，`check_swept_parameter` 在展开期就报
  `E_TOPO_PARAM`，诊断带 `被扫描参数 -> 中间参数 -> 使用点` 路径与每一步的 span
  （`topology_diagnostic`）。实验计划是在 `compile()` 里构造的，所以 `cdsl check`、
  `cdsl run` 与 REPL 的 `:load` 都在任何求解之前拒绝它；`circuit-backend/src/sweep.rs`
  的**逐点**拓扑比较原样保留，是第二道防线而不是替代品。只到达数值位置
  （`value:`、`dc:`、`ac:`、`waveform:`、模型参数）的参数不受影响，普通数值扫描照常可用。

### 4.2 子电路、层次与端口

- 实例必须绑定**全部**端口：缺端口、多端口、端口绑两次都是 `E_PORT`；
  绑定值不是节点符号是 `E_TYPE`。
- 递归实例化报 `E_RECURSION` 并打印调用链；嵌套深度超过 `Limits::max_depth` 报 `E_LIMIT`。
- 实例内部节点用 `Bodies::qualify` 加前缀：`stage1` + `internal` → `stage1.internal`；
  两个实例的同名内部节点因此是不同 `NodeId`。
- **端口不是新节点**：`resolve_node` 先查 `ctx.ports`，命中时直接返回**调用方的
  `NodeId`**。所以子电路端口只是"外层节点在实例内的局部名字"，不会创建
  `stage1.output` 这样的节点——测试断言 `circuit.node_id("stage1.output").is_none()`
  （`subcircuit_instances_are_isolated_and_parameters_override`）。
- 层次名里的 `.` 不是词法记号，而是名字拼接的结果；引擎按字符串键处理，实测无冲突
  （`docs/backend-evaluation.md` §6.2）。

### 4.3 循环与条件

- `for k in [2, 3, 4]` 遍历数组；`for k in 1..3` 遍历整数区间且**含两端**。
  区间为空（`hi < lo`）是合法的空展开。循环变量是**无量纲数值**
  （`Quantity::scalar`），循环结束后恢复它遮蔽的旧值。
- 循环**不会**创造新语法：每轮把 body 再执行一遍，所以器件要区分开就必须自己算出名字。
  `+` 在任一侧是文本时做字符串拼接（`eval_binary` 顶部的特例），于是 `("r" + k)`
  这类名字表达式可用；生成的名字经过 `check_identifier` 校验成合法标识符
  （与 `docs/language.md` §1.2 的 `ident` 同形），非法时报 `E_VALUE`（"not a usable name"）。
  唯一性不靠生成器保证，而由普通的重名检查兜底：`device_spans` / `node_index`
  最后看到的就是解析出来的名字，重名报 `E_DUPLICATE`
  （`duplicate_names_from_a_loop_are_reported`）。
- `if` / `elsif` / `else` 只执行第一个条件为真的分支；条件必须是 `Bool`
  （比较运算产生 `Bool`，`&&` / `||` / `!` 也只在布尔上工作），否则 `E_TYPE`。
  **未被选中的分支完全不展开**，所以条件可以决定拓扑——这正是扫描这类参数必须被拒绝的
  原因（§7）。
- 规模由 `Limits` 兜底：单个 `for` 的迭代数、总步数、器件数、节点数、嵌套深度、pwl 点数、
  扫描点数、结果值个数，超限一律 `E_LIMIT`，绝不静默截断。

### 4.4 分析计划

`elaborate_experiment` 把 `op` / `dc` / `ac` / `tran` 变成 `AnalysisTask`，
`save` 解析成 `NamedProbe`（`Probe::NodeVoltage` / `DifferentialVoltage` / `DeviceCurrent`，
节点与器件在展开期就解析成 `NodeId` / `DeviceId`，后端永远看不到用户写的名字），
`measure` 与 `derive` 分别解析成 `MeasureRequest` / `DeriveRequest`。一个 `save` 作用于该实验的
**所有**分析任务（`task.probes = probes.clone()`），重复保存同一探针报 `E_DUPLICATE`。
没有任何分析语句的实验报 `E_ARGUMENT`。

本轮的顺序在这一层变得重要：`derive` 与 `measure` 先按源码顺序收集，**任务列表全部建好之后**
才解析绑定（`analysis: :ac2` 只有在第二个 `ac` 已登记时才有意义），随后把表达式依赖写进对应任务的
`implicit_probes`。绑定规则见 `docs/language.md` §7.4：显式 `analysis:` 必须命中；
只有一个分析时可以省略；多个分析省略报 `E_AMBIGUOUS`；只有「裸探针且没写 `analysis:`」的
`measure` 保留 legacy 的 tran > ac > dc > op 选取顺序（`AnalysisBinding::LegacyPreferred`）。

## 5. 后端适配

适配层在**内存里**把 `circuit_core::ir::Circuit` 映射成 `cirq_ir::Circuit`
（`thevenin.rs::build_circuit`），不生成 SPICE/Cirq 源文本。这是 Phase-0 评估的直接结果：
`cirq_ir::Circuit` 及其子结构字段全为 `pub`，可以直接构造，于是"生成文本再解析"
这一整类往返错误被消除（`docs/backend-evaluation.md` §4.2）。

映射要点：

| 本项目的 IR | Thevenin 的 IR |
|---|---|
| `Node.id`（稠密 u32） | `CqNet.id`（同一套索引）；`Ground` 节点名写成 `gnd` |
| `DeviceKind::Resistor/Capacitor/Inductor` | 同名 `ElementKind`；值放 `params: [("value", Real)]` |
| `VoltageSource` / `CurrentSource` | 同名；值放 `source_spec`（dc / ac / waveform） |
| `Diode` + `Model` | `ElementKind::Diode` + `Element.model` 指向 `CqModel` |
| 端子 `p` / `n` | 引擎的 `pos` / `neg`（二极管保持 `anode` / `cathode`） |
| `AnalysisTask`（每个任务） | 单独构造一个只含该分析的 `CqCircuit`，再调对应的单分析入口 |

每次 `run` 对每个任务构造一个新的 Thevenin 电路并只放一个 `Analysis`，
避开多分析入口对 plot 排序与命名的依赖。瞬态的三个概念在适配层分开：

- `max_step` → 引擎 `tmax`（`h_max`），只约束**积分步长**；
- `output_interval` → **不传给引擎**：求解结束后由 `circuit-results::resample` 把原始时间轴重采样
  到等间隔输出网格（契约见 `docs/language.md` §5.3），它只改变展示与导出的采样点；
- 引擎 print step（`Tran.step`）由 `thevenin.rs::tran_step_for` 算出：
  `h_print = min(span/1000, 所有源声明的 rise/fall/period 最小值)`。这保证引擎对 PULSE 边沿的
  `.max(tstep)` 夹取（`waveform.rs:37-38`）不会落在声明值上；声明边沿无法执行时在 `validate`
  阶段报能力错误（`E_UNSUPPORTED` / `E_LIMIT`），**绝不静默展宽边沿**。

`circuit-session` 的 `RunOutcome` 有**两份**瞬态数据：`datasets` 是原始求解网格（测量来源），
`output_datasets` 是重采样后的展示/导出视图（无 `output_interval` 时两者相同）。
`avg`/`rms`/`max`/`min` 只在原始网格上计算，改输出采样不影响测量。

引擎的以下行为由适配层补偿。这些不是猜测，而是 Phase-0 实测（`docs/backend-evaluation.md`
§4.3、§4.6、§6）与 `crates/circuit-backend/src/thevenin.rs` 顶部注释记录的约束：

| 引擎行为 | 适配层的补偿 | 为什么必须补偿 | 测试位置 |
|---|---|---|---|
| `simulate_tran` 返回 `[op1, tran1]`，瞬态数据不在 `plots[0]` | `select_plot` 按小写名称前缀（`op`/`dc`/`ac`/`tran`）选 plot | 取 `plots[0]` 会静默返回工作点 | `tests/adapter.rs::rc_transient_matches_analytic`（若选错，轴不是时间轴，测试会以 "expected a time axis" 失败） |
| 单分析入口**忽略** `circuit.save` | `build_circuit` 把 `save` 留空；`convert_plot` 只按 `task.read_probes()`（`save` + 表达式依赖）物化信号，导出集合仍由 `task.probes` 决定 | 否则一次只要一个探针的运行会返回引擎的全部内部向量 | `tests/adapter.rs::only_requested_probes_are_returned`（断言信号名恰为 `["v(mid)"]`） |
| 两处相位单位相反：`AcSpec.phase` 与 `sin` 的 `phi` 在引擎里都是**度**，本项目内部都存弧度 | `map_source` 对两者都做 `to_degrees()`；反方向由 `elaborate` 的 `sin(..., phase:)` 分支做 `to_radians()` | 混用会让每个相量整体旋转 | **AC 源相位**：`tests/adapter.rs::ac_phase_is_converted_from_radians_to_degrees`（`phase_rad = π/2`，断言 `v(out) ≈ +0.5j`，实部 < 1e-9），本轮另加 `tests/phase_regression.rs` 覆盖多个非零角度、负相位与实/虚部。**`sin` 波形的 `phi`**：`tests/phase_regression.rs`（IR 直构）覆盖适配层换算；**DSL 层的 `sin(..., phase:)` 度→弧度换算此前在 `elaborate.rs` 与 `e2e.rs` 里都没有任何测试**（`grep -c phase crates/circuit-dsl/tests/elaborate.rs` = 0），本轮补 `crates/circuit-dsl/tests/phase_syntax_regression.rs`。**AC 源的相位目前没有 DSL 语法**（`elaborate.rs` 构造 `AcSpec { phase_rad: 0.0 }`），所以 AC 非零相位只能在 IR 层验证 |
| `thevenin_types::Complex` 不是 `num_complex` | 在 `complex_of` / `make_signal` 边界转换成本项目自己的 `circuit_results::Complex` | `circuit-results` 不能依赖任何后端类型 | 间接覆盖：`rc_ac_matches_analytic`、`rlc_ac_matches_analytic`、`differential_probe_subtracts_complex_signals` 都在复数域比对解析解 |
| 只有自带支路未知量的器件才有 `#branch` 电流（电压源、电感） | 电压源/电感直接读取；**电阻**用 `i = (v(p) - v(n)) / R` 推导；**电容、二极管、独立电流源**在 `validate` 阶段报 `E_UNSUPPORTED` 并说明原因 | brief 禁止伪造不可得的电流；推导只对线性电阻做，且必须被独立验证 | 推导：`divider_op_and_current_direction`（直流 KCL，`i(r1) = -i(v1)`，1e-12）、`derived_resistor_current_agrees_with_source_current_in_ac`（交流复数，相对误差 1e-9）；拒绝：`capacitor_current_is_refused_with_a_reason` |
| 真无参考的线性网络：引擎报 `matrix is singular, cannot solve`，但**不指向节点、也无法区分合法开路输出** | **由前端补偿**：`circuit-core` 的 `connectivity` 模块做直流参考通路可达性检查，`circuit-dsl` 的 `finish_circuit` 在展开结束时调用并报 `E_NAME`（含命中节点与阻断器件） | 单靠后端错误信息无法告诉用户哪个节点缺直流参考，也无法把「合法开路输出」与「真正无参考」分开 | `circuit-core::connectivity` 单元测试（9 个）；`circuit-dsl/tests/reference_path_regression.rs`；`circuit-cli/tests/e2e.rs` 的 `check_rejects_a_node_with_no_dc_path_to_ground` 与 `check_accepts_an_ac_coupled_stage_with_a_bias_resistor` |
| PULSE 的 `tr`/`tf` 仍被夹到 `.tran` 步长（`thevenin-0.5.0/src/waveform.rs:37-38`） | 适配层自己取 `h_print = min(span/1000, min 声明 rise/fall/period)`（`thevenin.rs::tran_step_for`）；无法在步数预算内执行时报 `E_UNSUPPORTED` / `E_LIMIT` | 否则声明 `rise: 1.ns` 会被静默展宽成 print step（历史缺陷：适配层把 `output_interval` 送进了 `Tran.step`） | `tests/transient_reference_regression.rs::declared_rise_below_output_interval_is_not_widened`（含旧契约判别对照）、`tests/output_interval_regression.rs`、`_probe` 的 `tran_contract`（内核钳位事实） |

补充说明：

- `validate` 在 `run` 之前执行：不支持的器件/模型、不支持的探针电流、源值扫描目标不是源、
  未知探针目标都在做任何求解前失败（`SimulationBackend` 的契约要求
  "validate 通过后 run 不得再以 unsupported 失败"）。
- 失败分类是保守的：只有信息里明确出现 `singular` / `converge` / `unsupported` 才给
  特定错误码，其余一律 `E_BACKEND` 并保留引擎原文（`classify_backend_failure`）。

## 6. 结果与测量

`Dataset`（`crates/circuit-results/src/dataset.rs`）是后端无关的结果模型：

- `experiment` / `analysis` / `kind`：分析名由适配层写成 `op1`、`tran1`、`ac1`、`dc1`
  （`format!("{kind}1")`）。
- `axis`：`Axis::None`（工作点，**不是**长度为 0 的轴）、`Time`、`Frequency`、`Parameter`。
  前两者带 `unit()`；`Parameter` 的单位不可从数值恢复，因此返回 `None`，
  JSON 里也不声称。
- `signals: Vec<Signal>`：每个信号有名字、`Dimension` 单位、`Data::Real | Data::Complex`。
  实数与复数保持区分，避免让 OP/DC/TRAN 的消费者处理恒为零的虚部。
- `backend: BackendInfo`：引擎名、版本、设置项，随 JSON 一起落盘，便于复现。瞬态结果带有
  `tran.solver_step` / `tran.solve_points` / `tran.waveform_bound` / `tran.max_step`（后端层），
  以及重采样后才出现的 `tran.output_grid = resampled-linear` / `tran.output_points`（结果层），
  因此"原始求解数据"与"输出采样"在元数据里可区分。
- 输出采样：`resample`（`crates/circuit-results/src/resample.rs`）把瞬态结果重采样到
  `output_interval` 指定的等间隔网格（起点/末点保留、线性插值、不越界外推、超
  `Limits::max_result_values` 报 `E_LIMIT`）。会话层同时返回原始与重采样两份数据，
  见 §5 的瞬态说明。
- `diagnostics: Vec<Diagnostic>`：**数据集自己的**警告与来源信息（计划层、后端附加）。
  渲染期的非有限值警告不写进这里，而是由 `write_datasets` 作为 `Written::warnings`
  返回并打印（见下），JSON 文件里保留的仍然只是这个数组。
- `Dataset::validate` 强制形状规则：每个信号的样本数必须等于轴长（或有轴时为 1）、
  信号名（大小写与空白不敏感地归一后）不得重复、标量值总数不得超过
  `Limits::max_result_values`（复数样本计 2）。校验失败是错误，**绝不截断或补零**。
  信号查找同样按归一化名字进行，所以 `v(out)`、`V(OUT)`、`v ( out )` 是同一个信号。

导出（`export.rs`）：

- CSV：第一列是轴（工作点无轴列），随后每个信号一列；复信号拆成 `<name>_re` / `<name>_im`；
  含逗号的信号名（如 `v(a,b)`）按 RFC 4180 加引号——不这样做右侧所有列都会错位。
- JSON：`schema = "circuit-dsl.result/1"`，保留 experiment/analysis/kind、轴类型与单位、
  每个信号的名称/单位/类型、后端元数据与诊断。
- **非有限值策略**：CSV 写成空字段，JSON 写成 `null`（`serde_json` 表示不了 NaN/inf），
  两种情况都为受影响的信号（以及轴）产生一条 `warning[E_VALUE]`，说明有几个非有限样本、
  索引在哪、以及"导出为空字段/`null`"。数值因此是"缺失且可见"，而不是静默变成一个
  貌似合理的数字。复数的两个分量独立处理：一个分量为 inf 时另一个仍然保留。
- **两批诊断是互补的，不是同一批**（契约 §1.4 修正后的说法）：JSON 文件里的
  `diagnostics` 数组是**数据集自己的**来源诊断（计划层与后端附加，`to_json_value`）；
  而 `Written::warnings` 是**渲染期**的 `non_finite_diagnostics`——每个非有限样本一条，
  文件本身只把它表达为空字段或 `null`。两者互不拷贝，返回的警告按
  `render_plain()` 文本**按数据集去重**，所以 CSV+JSON 两种格式只报一次。
  `cdsl run` 与 REPL 先显示 `RunOutcome::warnings`（数据集级）再显示
  `Written::warnings`（渲染级），看到的才是完整画面。会话层的 `write_datasets` 就按这个
  关系接线：用 `to_*_with_diagnostics` 渲染、按数据集去重、再写文件，并返回
  `Written { paths, warnings }`；CLI 与 REPL 共用 `warning_lines()` 打印（R4-03）。

`avg` / `rms` **按时间积分**，不是样本算术平均：`avg = ∫x dt / ∫dt`、
`rms = sqrt(∫x² dt / ∫dt)`，用梯形法在**非均匀**时间轴上积分（`measure.rs`）。
原因是求解器返回的时间轴由它自己选步长，通常非均匀（`docs/language.md` §5.1），
对样本取算术平均会得到一个用户没有要求、也没有物理意义的数。`avg` / `rms` 要求
`Axis::Time`，对 OP、DC、AC 报 `E_TYPE`；复数信号取模长后积分；时间轴不前进
（总宽为 0）或点数不足 2 时报 `E_VALUE`，而不是除以零。
`max` / `min` 是样本极值，不需要轴，实信号不取绝对值（负的极值是合法的，
需要模长请用 `abs`）。

`expr.rs` 既提供作用在采样向量上的结果表达式求值器（`v(a)`、`v(a,b)`、`i(r1)`、
`abs`、`sqrt`、`min`、`max`、`20*log10(abs(a/b))` 形式的增益），也提供 `from_ir`：
把 `circuit-core::plan::ExprIr` 降级成这个运行期 AST。实数/复数混合时提升为复数。
**本轮已接通**：DSL 的 `derive` 与表达式形式的 `measure` 都走这条路径——前端降级成 `ExprIr`，
会话层（`execute.rs::attach_derived` / `evaluate_measures_with`）在**原始**数据集上调用
`circuit_results::expr::eval` 与 `measure::reduce`。求值失败不再"换下一个分析再试"，
而是整次运行失败，并把分析标识与表达式的规范形式（`ExprIr::render`，不是逐字源文本）放进诊断 context。
`docs/language.md` §7.1 描述的 `measure :name, max: v(:out)` 形式仍然有效，并保留它自己的
legacy 选取顺序（见下一段）。

**R4-01 的不变量**（同一文件，全部实现在一个求值器里）：每个运算节点产出样本后立刻校验
有限性——实数 `x.is_finite()`、复数要求两个分量都有限，失败就是命名该运算与子表达式的
`E_VALUE`；因此 `min(sqrt(-1), 2)` 在 `sqrt` 处失败、`1e308*1e308` 不会让 `inf` 继续
参与计算。`sqrt` 负实数是错误、分母精确为 0 是错误、`gain_db` 零幅值是错误，**没有
epsilon、没有饱和、没有跳过样本、没有 `catch_unwind`**；输入信号本身是 NaN/±inf 也由
读取它的运算拒绝。诊断带 `analysis`/`kind`/`signal`/`sample`/`index` 上下文，具名站点
（`derive`/`measure`）还会把名字写进消息并以名字替换 `signal`；标量分析说明没有轴坐标。
`check` 对**常量**表达式（`is_constant`）调用同一个求值器，所以同一批错误不用等到运行
（见 §2 的 `circuit-cli`）。求值前还有一道显式深度护栏：深度超过
`circuit_core::limits::MAX_EXPR_DEPTH`（256）报 `E_LIMIT`，且不渲染那个深层子表达式。

CLI 侧还有一个取值规则：一个实验可以跑多个分析，`measure` 会从**能支持该归约的最丰富
分析**里取数，顺序是 tran → ac → dc → op（`crates/circuit-session/src/execute.rs::measurement_rank`）。
这条规则只适用于 legacy 形式——目标是裸探针、且没写 `analysis:` 的 `measure`；
写了 `analysis:` 或目标是表达式时绑定是确定的，选中分析上的求值失败就是错误，不会跳到下一个。
没有这个顺序，RC 阶跃上的 `max: v(:out)` 会静默读到工作点的 0 V 而不是瞬态峰值。

## 7. 参数扫描

参数扫描**不能**是一次后端调用：引擎的 DC 分析扫的是**源的值**，而 `param` 影响的是
器件参数，改一个参数意味着器件值变了，必须重新展开整个设计。
（适配层也确实不把参数传给引擎：`build_circuit` 把 `params` 留空。）

因此：

1. `cdsl run` 的执行器 `circuit-session::execute::parameter_sweep_of` 检出"唯一的 DC 任务且目标是参数"，
   生成坐标（`sweep_coordinates`，端点规则：`stop` 只在落在步长整数倍上时包含；
   方向与步长符号必须一致；步长为 0 报 `E_SWEEP`）。在**任何求解之前**，
   `check_sweep_bindings` 拒绝绑到非扫描分析的 `derive`/`measure`：整个扫描只产出一个
   拼接数据集，所以这是 `E_UNSUPPORTED`，而不是静默丢弃（`docs/language.md` §7.4）。
2. 每个坐标调用一次 `circuit_dsl::elaborate_experiment`，覆盖值形如
   `[(name, Quantity::new(value, sweep.dimension))]`——每次都是**全新展开**，
   所以 `save`、拓扑、参数求值全部按该点的值重新计算。
   展开用的是 `check::front_end` 一并返回的 `Program`（`crates/circuit-cli/src/check.rs` 的
   `FrontEnd::program`），所以扫描路径不再二次读盘解析。
3. 单点运行把该 DC 任务改成 `AnalysisKind::Op`（`as_single_point_plan`），
   因为一个扫描点就是一个工作点。
4. `run_parameter_sweep` 在每个点展开后做**拓扑不变性检查**：
   `sweep.rs` 里的私有类型 `Topology` 提取并比较两组东西——
   节点集合 `Vec<(name, is_ground)>`，以及器件集合
   `Vec<(name, kind.name(), 排序后的端子 [(terminal, node_id)])>`。
   端子被排序，所以声明顺序的变化不会被误判为拓扑变化；不同点之间只要节点集合、
   器件名、器件种类或连线有差异，就报 `E_TOPO_PARAM`，并说明"参考拓扑来自哪个点、
   差异是什么"。这把"拓扑参数"从一个需要用户相信的标签变成了被真正检查的性质
   （`docs/language.md` §4.2 的规则）。
   这条检查现在是**第二道**防线：拓扑参数扫描已经在 `compile()` 的展开期被
   `E_TOPO_PARAM` 拒绝（§4.1），所以 `cdsl check` 不必求解就能报出来，
   而运行期逐点比较仍然保留，防住任何静态分析没覆盖到的变化。
5. 任一点失败立即停止并报告（不继续产生部分结果）。
6. `stitch` 把各点结果拼成一个 `Dataset`：轴是 `Axis::Parameter`，
   分析名是 `dc_param_<参数名>`（于是结果是 `sweep.dc_param_r.csv`），
   并检查每个点报告的信号列表与第一个点完全一致——否则报 `E_BACKEND`
   （"参数不得改变存在哪些信号"）；复数结果拒绝拼接（`E_UNSUPPORTED`）。

## 8. 错误诊断

- 诊断类型是 `circuit_core::diagnostic::Diagnostic`：`severity`、稳定的 `code`、
  `message`、一个主标签（span + 可选文字）、若干次标签、`notes`、`context`
  （渲染成 `= key: value`）。统一渲染格式：

  ```text
  error[E_DIMENSION]: resistor.value requires resistance, found time
    --> examples/filter.cdsl:8:48
     |
   8 | resistor :r1, p: :vin, n: :out, value: 10.ms
     |                                         ^^^^^
     = expected: ohm
     = received: s
  ```

  `SourceMap` 把字节偏移换算成 1-based 行列，caret 按 Unicode 标量数列对齐；
  `SourceSpan::synthetic()`（循环生成的实体、构造出来的 IR）不会伪装成一个位置，
  诊断改用"命名所在定义"的方式说明。
- `Code` 的 `E_*` 字符串是 CLI 契约的一部分，注释明确写了"发布后不得改名"：
  `E_SYNTAX`、`E_NAME`、`E_DUPLICATE`、`E_AMBIGUOUS`、`E_DIMENSION`、`E_VALUE`、`E_ARGUMENT`、
  `E_TYPE`、`E_PARAM_CYCLE`、`E_RECURSION`、`E_PORT`、`E_TOPO_PARAM`、`E_LIMIT`、
  `E_UNSUPPORTED`、`E_BACKEND`、`E_CONVERGE`、`E_SINGULAR`、`E_SWEEP`、`E_IO`。
  其中 `E_PARAM_CYCLE` 由参数依赖图的环报告产生（§4.1），`E_TOPO_PARAM` 既来自
  展开期的拓扑参数扫描拒绝（§4.1）也来自后端逐点拓扑比较（§7），`E_LIMIT` 还覆盖
  表达式深度上限（`MAX_EXPR_DEPTH`），`E_CONVERGE` / `E_SINGULAR` 只在后端失败
  分类里产生，`E_AMBIGUOUS` 由结果表达式的绑定解析产生（多分析实验里没写
  `analysis:`，`docs/language.md` §7.4）。
- **失败不会被伪装成一次空成功**，具体在三处兜底：
  `SimulationBackend::run` 的契约要求"报告失败，而不是返回空但成功的结果"；
  `convert_plot` 遇到"后端没有报告这个探针"或"没有该分析对应的 plot"就报错，
  而不是返回只少了几个信号的 Dataset；`cdsl run` 在
  `datasets.is_empty()` 时报 `E_BACKEND` 并以退出码 2 结束。
  `Dataset::new` 对形状错误与超限也是拒绝而不是裁剪；`measure` 找不到信号时报 `E_NAME`
  并在 note 里列出可用信号。

## 9. 关键设计取舍

| 决策 | 被否决的替代方案 | 原因 |
|---|---|---|
| 语义 IR + 内存结构映射到引擎 | 生成 SPICE/Cirq 源文本再解析 | Phase-0 实测 `cirq_ir::Circuit` 可直接构造，去掉"生成-解析"这一整类往返错误（`docs/backend-evaluation.md` §4.2、§8） |
| `Quantity` 携带运行期 `Dimension` | brief 草图的 `Quantity<Dimension>` 幽灵类型 | 展开器是动态求值器，类型级方案要单态化每种算术组合；可观察行为（报错时同时给出 expected/received）相同（`units.rs`） |
| 保留 `SimulationBackend` trait，但只有一个实现 | CLI 直接调 Thevenin | trait 是让 `circuit-core`/`circuit-results` 不依赖第三方类型的接缝；能力声明让"不支持"在 `validate` 阶段显式失败 |
| 参数扫描每点重新展开 + `Topology` 检查 | 指望引擎的 `.dc` 扫参数，或不做检查 | 引擎只能扫源值；拓扑检查把"拓扑参数不可扫描"从文档约束变成强制约束 |
| 电阻电流用欧姆定律推导并交叉验证；电容/二极管/电流源明确拒绝 | 统一用差分近似，或全部拒绝 | 前者会伪造数据（brief 禁止），后者会让常见探针不可用；推导只对精确成立的线性电阻做 |
| 错误累积（`Diagnostics`）而不是首错即停 | 只报第一个错误 | `cdsl check` 的价值在于一次列全；但单个 body 内仍在首个结构性错误后停止，避免噪声 |
| `Limits` 显式可配、超限报 `E_LIMIT` | 隐藏常量或按上限截断 | 有限循环 + 无条件展开必须防止无界分配；截断会让结果看起来完整 |
| `Dataset::validate` 拒绝超限结果 | 截断到上限 | 结果文件必须要么完整要么报错 |
| `circuit-core` 不派生 serde，`check --json` 手写 | 给 IR 加 serde 派生 | 让 IR 不绑定一种序列化选择（`check.rs` 注释） |
| `uic` 不暴露 | 直接透传 `TranSpec.uic` | 后端路径未经 Phase-0 验证，未验证的能力不开放（`plan.rs` 注释、`docs/language.md` §10） |
| 结果表达式的计划层 IR 在 `circuit-core`，降级与求值在 `circuit-results` | 把求值器放进 `core`，或让前端把 AST 直接交给后端 | `core` 必须与求解器和结果层无关；`expr::from_ir` 是两条边唯一的接触点，计划因此可被 `check` 打印、检查与单测 |
| 派生信号在原始网格上求值，重采样放在最后 | 先重采样再求值 | `v*v` 这类非线性表达式在插值点上的值会被改变；测量与派生必须看到求解器真正给出的样本（`docs/language.md` §7.6） |
| 参数依赖图：预扫描 + 确定性拓扑序，环报 `E_PARAM_CYCLE` | 继续按声明顺序求值、前向引用报 `E_NAME` | 前向引用是自然写法；依赖顺序由数据决定而不是书写位置，平局按声明顺序打破保持可复现；环被定位到参与声明而不是伪装成"未声明"（`param_graph.rs`） |
| 每个运算节点校验自身样本的有限性 | 只在表达式末尾检查，或让 NaN/inf 传播 | 中间值非法时 `min`/`max` 之类的组合会把它掩盖成一个看似合法的结果（R4-01）；逐节点校验才能把错误指到真正产生它的运算 |
| 量纲指数溢出报 `E_DIMENSION`（受检算术） | debug panic / release 回绕，或把指数加宽 | 用户可达的长乘积链必须有一个诊断；加宽指数只是把边界推远，仍会有一个不可达的深度（R4-02、`units.rs`） |
| 隐式探针只读不导出（写了 `save` 时） | 把表达式用到的信号自动加进导出列 | 导出集合是用户写下的契约；自动加列会让结果文件出现没写过的列，`check` 把它单列为 `reads ... (expression inputs, not exported)` |
| 每个用例单独的容差 | 全局一个宽松阈值 | 线性解接近机器精度，瞬态受脉冲上升沿与求解器 `RELTOL` 限制；见 `docs/testing.md` §5 |
