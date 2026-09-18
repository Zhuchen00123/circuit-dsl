# 架构

> 本文描述**当前实现**的结构。每一条结论都能在 `crates/` 里找到对应代码；
> 未实现的能力在 §9 和 `docs/language.md` §10 中单独列出，不在本文声称。

## 0. 工作区

根 `Cargo.toml` 声明 5 个成员 crate：

```
crates/circuit-core       语义数据结构（无第三方仿真依赖）
crates/circuit-dsl        lexer / parser / 展开
crates/circuit-backend    后端契约 + Thevenin 适配 + 参数扫描驱动
crates/circuit-results    结果数据集 / 表达式 / 测量 / 导出
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
| `circuit-cli` | `circuit-core`, `circuit-dsl`, `circuit-backend`, `circuit-results` | `clap`, `serde_json`, `thiserror` |

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
  │  名字解析 · 量纲检查 · 参数求值 · 子电路/端口展开 · for/if 展开
  ▼
(circuit_core::ir::Circuit, circuit_core::plan::AnalysisPlan)
  │  circuit-backend::thevenin::TheveninBackend
  │  内存中的结构映射（不生成任何源文本）；按名选 plot、探针子集化、电阻电流推导
  ▼
cirq_ir::Circuit（每个分析任务单独构造，只含一个 Analysis）
  │  thevenin::circuit::{simulate_op, simulate_dc, simulate_ac, simulate_tran}
  ▼
SimResult / SimPlot / SimVector
  │  轴构造 + 单位/复数类型转换（thevenin_types::Complex → circuit_results::Complex）
  ▼
circuit_results::Dataset（axis + signals + 单位 + backend 元数据）
  │  measure / to_csv / to_json
  ▼
cdsl run 的 stdout 摘要与结果文件
```

参数扫描是旁路：`cdsl run` 检出 DC 参数扫描后走
`circuit_backend::sweep::run_parameter_sweep`，每个扫描点重新展开一次，再进入上面
`Circuit → 后端` 那一段，最后由 `circuit-cli/src/run.rs::stitch` 拼成单个 Dataset
（轴是扫描值）。详见 §7。

**依赖方向**：`core <- results <- backend <- cli`，另有 `core <- dsl <- cli`。

- `core <- results`：`Dataset`/`Signal` 用 `Dimension` 标注单位，用 `Diagnostic` 报错，
  用 `Limits` 约束结果规模（`crates/circuit-results/src/lib.rs`）。
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
  `plan`（`AnalysisPlan` / `AnalysisTask` / `Probe` / `Sweep` / 各分析规格）、
  `limits`（`Limits`）。
- **不拥有**：词法/语法（`dsl`）、结果语义（`results`）、求解（`backend`）、
  CLI 的输出与退出码策略（`cli`）。
- **公开入口**：上列类型；`Circuit::new` 是唯一带校验的构造器，
  `Circuit::{node_id, device_id, model_id, node_name, summary}` 等查询。

### `circuit-dsl`

- **拥有**：`lexer`（含量纲字面量的词法）、`parser`（递归下降，语句关键字按文本分派）、
  `ast`（语法树，名字是字符串、每个表达式带 span）、`elaborate`
  （名字解析、量纲检查、参数求值、层次展开、循环与条件展开、分析计划构造）。
- **不拥有**：任何仿真、文件访问或网络访问。整个 crate 是纯函数式的
  （`crates/circuit-dsl/src/lib.rs`）；DSL 不参与仿真过程，展开完成后拓扑固定。
- **公开入口**：`lex`、`parse`、`compile`（整个文件的全部 circuit 与 experiment）、
  `elaborate_experiment`（单实验 + 覆盖值，参数扫描每点调用它）、`Elaborated`、
  `Compiled`、`Program`。`circuit_parameters` 已定义但当前没有任何调用方。

### `circuit-backend`

- **拥有**：`backend`（`SimulationBackend` trait、`BackendCapabilities`、
  `classify_backend_failure` / `backend_failure`）、`thevenin`（唯一的实现，
  结构映射 + 结果物化）、`sweep`（参数扫描驱动 + 拓扑不变性检查）。
- **不拥有**：结果数据模型。`Dataset`、`Signal`、`Axis` 都在 `circuit-results`；
  适配层只负责把引擎输出搬进这些类型。
- **公开入口**：`TheveninBackend::{new, with_limits, capabilities, validate, run}`、
  `BACKEND_VERSION`（`"0.5.0"`）、`run_parameter_sweep`、`sweep_coordinates`、
  `SweepOutcome`、`SweepError`。
  `branch_current_to_p_to_n` 与 `node_index` 是公开的自由函数，当前没有任何调用方；
  见 §5 的说明。

### `circuit-results`

- **拥有**：`dataset`（`Dataset` / `Axis` / `Signal` / `Data` / `Complex` /
  `BackendInfo` / `normalize_signal_name`）、`expr`（结果表达式 AST 与求值器，
  作用在整个采样向量上）、`measure`（`max` / `min` / `avg` / `rms`）、
  `export`（CSV/JSON、`SCHEMA = "circuit-dsl.result/1"`、非有限值警告）、
  `format_number`。
- **不拥有**：后端类型（`thevenin_types` 只在 `circuit-backend` 出现）、
  仿真调度、CLI 输出。
- **公开入口**：上列类型与 `measure` / `measure_signal` / `reduce` / `to_csv` /
  `to_json` / `to_json_value` / `non_finite_diagnostics` / `eval`。

### `circuit-cli`

- **拥有**：`cdsl` 二进制、三个子命令（`check`、`run`、`capabilities`）、
  退出码（0 成功 / 1 用户错误 / 2 内部错误）、"诊断走 stderr、数据与摘要走 stdout"、
  结果文件命名与"拒绝写回输入文件"（`guard_output`）、`run` 的测量取值顺序。
- **不拥有**：语言语义、仿真细节、结果格式定义。
- **产物入口**：`cdsl` 可执行文件（`check` / `run` / `capabilities`）。
  crate 内部的 `check::front_end` 是 `check` 与 `run` 共用的前端入口
  （读文件 → lex → parse → compile → 后端能力校验）。

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
器件不需要它们（`units.rs` 模块注释）。

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

### 4.1 参数与覆盖顺序

- `param` 只能在 body 内声明，**按出现顺序求值**，因此只能引用**之前**声明的参数。
  这使依赖环在结构上不可能出现：`eval` 只读 `scope.vars`，而 `vars` 里只会有已声明的
  参数和预装的覆盖值。自引用与前置引用都会在读取时报 `E_NAME`
  （`crates/circuit-dsl/tests/elaborate.rs` 的 `a_self_referential_parameter_is_rejected`、
  `a_forward_parameter_reference_is_rejected`）。
  `Code::ParamCycle`（`E_PARAM_CYCLE`）在枚举里保留，但**当前没有任何代码会构造它**——
  这正是"环不可能出现"的结果，`docs/language.md` §4.2 仍写着会报 `E_PARAM_CYCLE`，
  与实现不一致。
- 覆盖顺序是 **默认值 → 实例或实验覆盖 → 扫描点覆盖**，实现方式是
  `Scope::vars` 在 body 执行**之前**就被覆盖链填好；`param` 语句里的 `default:` 只有在
  `vars` 里还没有该名字时才写入（`stmt_param`）。因此 `param :r, default: 1.kohm`
  配合实例覆盖 `r: 2.kohm` 得到 2 kΩ，且默认值仍会被记入 `scope.defaults`，
  用于检查覆盖值的量纲。
- 实例覆盖写在**外层 scope** 里求值，所以 `params: { r: rstage }` 可以引用父电路的参数；
  子电路自己的默认值作为兜底折入 `eval_scope`，且每个被接受的覆盖值立即加入
  `eval_scope`，于是后面的条目可以引用前面的（`stmt_instance` 中的注释与代码）。
- 实验级 `param :x, value: …` 只在"更早的实验级覆盖"构成的 scope 里求值，
  看不到电路参数（`experiment_overrides`）。`elaborate_experiment` 收到的扫描覆盖值
  按名字替换实验自身的覆盖值——"后者胜出"，这就是覆盖链最后一环。

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
`measure` 解析成 `MeasureRequest`。一个 `save` 作用于该实验的**所有**分析任务
（`task.probes = probes.clone()`），重复保存同一探针报 `E_DUPLICATE`。
没有任何分析语句的实验报 `E_ARGUMENT`。

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
避开多分析入口对 plot 排序与命名的依赖。`tran` 的 `step` 取
`output_interval`（缺省为 `span/1000`）、`tmax` 取 `max_step`——两者不能混用，
否则 `max_step` 会变成对输出网格的承诺（`docs/language.md` §5.1）。

引擎的以下行为由适配层补偿。这些不是猜测，而是 Phase-0 实测（`docs/backend-evaluation.md`
§4.3、§4.6、§6）与 `crates/circuit-backend/src/thevenin.rs` 顶部注释记录的约束：

| 引擎行为 | 适配层的补偿 | 为什么必须补偿 | 测试位置 |
|---|---|---|---|
| `simulate_tran` 返回 `[op1, tran1]`，瞬态数据不在 `plots[0]` | `select_plot` 按小写名称前缀（`op`/`dc`/`ac`/`tran`）选 plot | 取 `plots[0]` 会静默返回工作点 | `tests/adapter.rs::rc_transient_matches_analytic`（若选错，轴不是时间轴，测试会以 "expected a time axis" 失败） |
| 单分析入口**忽略** `circuit.save` | `build_circuit` 把 `save` 留空；`convert_plot` 只按 `task.probes` 物化信号 | 否则一次只要一个探针的运行会返回引擎的全部内部向量 | `tests/adapter.rs::only_requested_probes_are_returned`（断言信号名恰为 `["v(mid)"]`） |
| `AcSpec.phase` 单位是**度**，本项目内部存弧度 | `map_ac` 做 `to_degrees()`；`sin` 波形的 `phi` 同样转换 | 混用会让每个相量整体旋转 | **没有数值测试**：测试与 DSL 的 `ac:` 都只产生 0 相位（`elaborate.rs` 构造 `AcSpec { phase_rad: 0.0 }`），转换代码只在 0 上走过路径 |
| `thevenin_types::Complex` 不是 `num_complex` | 在 `complex_of` / `make_signal` 边界转换成本项目自己的 `circuit_results::Complex` | `circuit-results` 不能依赖任何后端类型 | 间接覆盖：`rc_ac_matches_analytic`、`rlc_ac_matches_analytic`、`differential_probe_subtracts_complex_signals` 都在复数域比对解析解 |
| 只有自带支路未知量的器件才有 `#branch` 电流（电压源、电感） | 电压源/电感直接读取；**电阻**用 `i = (v(p) - v(n)) / R` 推导；**电容、二极管、独立电流源**在 `validate` 阶段报 `E_UNSUPPORTED` 并说明原因 | brief 禁止伪造不可得的电流；推导只对线性电阻做，且必须被独立验证 | 推导：`divider_op_and_current_direction`（直流 KCL，`i(r1) = -i(v1)`，1e-12）、`derived_resistor_current_agrees_with_source_current_in_ac`（交流复数，相对误差 1e-9）；拒绝：`capacitor_current_is_refused_with_a_reason` |
| 悬空节点**不报错**（gmin 把无直流通路的节点拉住） | **由前端补偿**：`circuit-core` 的 `connectivity` 模块做直流参考通路可达性检查，`circuit-dsl` 的 `finish_circuit` 在展开结束时调用并报 `E_NAME` | 引擎会返回貌似正常的有限值，所以只能在前端发现 | `circuit-core::connectivity` 单元测试（8 个）；`circuit-cli/tests/e2e.rs` 的 `check_rejects_a_node_with_no_dc_path_to_ground` 与 `check_accepts_an_ac_coupled_stage_with_a_bias_resistor` |

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
- `backend: BackendInfo`：引擎名、版本、设置项，随 JSON 一起落盘，便于复现。
- `diagnostics: Vec<Diagnostic>`：属于这个结果的警告（后端警告、导出警告）。
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

`avg` / `rms` **按时间积分**，不是样本算术平均：`avg = ∫x dt / ∫dt`、
`rms = sqrt(∫x² dt / ∫dt)`，用梯形法在**非均匀**时间轴上积分（`measure.rs`）。
原因是求解器返回的时间轴由它自己选步长，通常非均匀（`docs/language.md` §5.1），
对样本取算术平均会得到一个用户没有要求、也没有物理意义的数。`avg` / `rms` 要求
`Axis::Time`，对 OP、DC、AC 报 `E_TYPE`；复数信号取模长后积分；时间轴不前进
（总宽为 0）或点数不足 2 时报 `E_VALUE`，而不是除以零。
`max` / `min` 是样本极值，不需要轴，实信号不取绝对值（负的极值是合法的，
需要模长请用 `abs`）。

`expr.rs` 提供作用在采样向量上的结果表达式求值器（`v(a)`、`v(a,b)`、`i(r1)`、
`abs`、`sqrt`、`min`、`max`、`20*log10(abs(a/b))` 形式的增益），实数/复数混合时提升为复数。
**注意**：这个求值器目前是库能力，CLI 的 `measure` 只按探针名取信号后直接归约
（`run.rs` 只 import 了 `measure_signal`），DSL 不能写任意结果表达式。
`docs/language.md` §7 描述的 `measure :name, max: v(:out)` 形式是当前唯一接通的形式。

CLI 侧还有一个取值规则：一个实验可以跑多个分析，`measure` 会从**能支持该归约的最丰富
分析**里取数，顺序是 tran → ac → dc → op（`run.rs::measurement_rank`）。没有这个顺序，
RC 阶跃上的 `max: v(:out)` 会静默读到工作点的 0 V 而不是瞬态峰值。

## 7. 参数扫描

参数扫描**不能**是一次后端调用：引擎的 DC 分析扫的是**源的值**，而 `param` 影响的是
器件参数，改一个参数意味着器件值变了，必须重新展开整个设计。
（适配层也确实不把参数传给引擎：`build_circuit` 把 `params` 留空。）

因此：

1. `cdsl run` 在 `run.rs::parameter_sweep_of` 检出"唯一的 DC 任务且目标是参数"，
   生成坐标（`sweep_coordinates`，端点规则：`stop` 只在落在步长整数倍上时包含；
   方向与步长符号必须一致；步长为 0 报 `E_SWEEP`）。
2. 每个坐标调用一次 `circuit_dsl::elaborate_experiment`，覆盖值形如
   `[(name, Quantity::new(value, sweep.dimension))]`——每次都是**全新展开**，
   所以 `save`、拓扑、参数求值全部按该点的值重新计算。
   为此 `run.rs` 重新 lex/parse 了一次源文件：`check::FrontEnd` 只保留 `Compiled`，
   而 `elaborate_experiment` 需要 `Program`。
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
  `E_SYNTAX`、`E_NAME`、`E_DUPLICATE`、`E_DIMENSION`、`E_VALUE`、`E_ARGUMENT`、
  `E_TYPE`、`E_PARAM_CYCLE`、`E_RECURSION`、`E_PORT`、`E_TOPO_PARAM`、`E_LIMIT`、
  `E_UNSUPPORTED`、`E_BACKEND`、`E_CONVERGE`、`E_SINGULAR`、`E_SWEEP`、`E_IO`。
  其中 `E_PARAM_CYCLE` 当前不可达（§4.1），`E_CONVERGE` / `E_SINGULAR` 只在
  后端失败分类里产生。
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
| 每个用例单独的容差 | 全局一个宽松阈值 | 线性解接近机器精度，瞬态受脉冲上升沿与求解器 `RELTOL` 限制；见 `docs/testing.md` §5 |
