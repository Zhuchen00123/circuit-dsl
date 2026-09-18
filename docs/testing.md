# 测试

> 本文记录**实际存在**的测试、实测数字与容差来源。所有数字都是在本文写作时运行
> 下列命令得到的，不是估计值。未验证的部分在 §7 单独列出。

## 1. 测试策略

三层，各自回答不同的问题：

**单元测试（crate 内部 `#[cfg(test)]`）** 验证单个组件的局部性质：
`units` 的量纲与单位后缀、`lexer` 的续行与量纲字面量、`parser` 的优先级与错误恢复、
`plan` 的 AC 点数约定、`dataset` 的形状规则、`measure` 的积分、`expr` 的求值、
`export` 的 CSV/JSON 细节、`sweep` 的坐标生成与拓扑比较、
`backend` 的失败分类。它们不启动仿真器。

**生产者/消费者契约测试** 验证两个组件真正接上：
`crates/circuit-dsl/tests/elaborate.rs`（51 个）用**真实前端**跑源码字符串
（lex → parse → compile，没有一个测试手搭 AST），因此语法或展开器改动无法蒙混过关；
`crates/circuit-backend/tests/adapter.rs`（16 个）在 Rust 里**直接构造本项目的 IR**，
把它交给真实的 Thevenin 引擎跑，再与闭式解对比。前者是"语言 → IR"的契约，
后者是"IR → 引擎"的契约。

**CLI 端到端测试** `crates/circuit-cli/tests/e2e.rs`（14 个）用
`CARGO_BIN_EXE_cdsl` 启动真正的二进制，跑 `examples/` 里的真实文件，读回它写出的
CSV/JSON 并解析数值。它同时验证退出码、stderr 诊断与 stdout 摘要。
一次运行写不出结果、写出错误数值、或把错误当成功，都必须在这里失败。

**测试断言的是数值，不是形状。** 具体例子：

- 分压器：断言 `v(in)=5 V`、`v(out)=3 V`、`i(r1)=+2 mA`、`i(v1)=-2 mA`
  （各 1e-9），并额外断言 KCL：`i(r1)+i(v1)` 的绝对值 < 1e-12。
  **符号本身是被断言的一部分**——电阻吸收功率所以 `p -> n` 为正，源释放所以为负；
  一个符号写反的实现会在这里失败，而不是靠"张量形状对上了"蒙混过去
  （`crates/circuit-cli/tests/e2e.rs::run_divider_produces_the_correct_voltages_and_currents`）。
- RC 瞬态：不只检查"有没有结果"，而是在 0.5/1/2/3 个时间常数处与闭式解
  `1 - exp(-t/τ)` 逐点比较，容差 1e-2（`run_rc_filter_matches_the_analytic_response`）；
  适配层测试在同一电路上按 0.25/0.5/1/2/3 τ 比较，并检查最后一个采样点落在 `5τ`（1e-9）。
- 参数扫描：每个点的 `v(out)` 与公式 `3 V * 1.5k / (r + 1.5k)` 比较，容差 1e-9，
  并断言点数恰为 8（`run_parameter_sweep_is_exact`）。
- 测量：`avg` 在一个**非均匀**轴上等于 2.0，并显式断言它**不等于**样本算术平均
  1.75（差值 > 0.2）；`rms` 等于 `sqrt(5.75) = 2.3979157616563596`，
  且与样本 RMS `sqrt(5.25)` 的差 > 0.05（`crates/circuit-results/src/measure.rs`）。
  只按"形状"写测试（例如只检查返回了一个数）会同时放过这两个错误实现。
- 二极管：与二分法独立求解 Shockley 方程的结果比较（容差 5e-3），
  再叠加物理范围检查 0.3 V < Vd < 0.8 V。

少数测试确实只看形状，但那是因为被验证的契约就是形状：`check --json` 的字段结构
（`check_json_describes_the_circuit`）、CSV 的列名与 RFC 4180 引号、JSON 的
`schema`/单位字段。数值能力一律用上面的方式验证。

## 2. 如何运行

本环境的 `cargo` 不在默认 PATH 上，先加：

```bash
export PATH="$PATH:/c/Users/15185/.cargo/bin"
cd /f/codexprojects/dsl000
```

全部命令：

```bash
# 整个工作区的全部测试（5 个成员 crate；_probe 被 exclude，不会被构建）
cargo test --workspace

# 只要每个测试二进制的汇总行
cargo test --workspace 2>&1 | grep "^test result"

# 静态检查（不产生修改）
cargo clippy --workspace --all-targets

# 格式检查（只报告，不写文件）
cargo fmt --all -- --check
```

单个 crate 或单个测试目标：

```bash
cargo test -p circuit-core
cargo test -p circuit-dsl
cargo test -p circuit-results
cargo test -p circuit-backend
cargo test -p circuit-cli

# 集成测试文件
cargo test -p circuit-dsl     --test elaborate
cargo test -p circuit-backend --test adapter
cargo test -p circuit-cli     --test e2e

# 更窄的过滤：测试名里包含给定子串的都跑
cargo test -p circuit-results --lib measure::tests
cargo test -p circuit-cli     --test e2e run_divider
```

本仓库的实际状态：三个命令都干净。

- **`cargo fmt --all -- --check` 通过**：退出码 0，无输出。这是格式检查命令，
  不修改文件；要改格式需要单独跑一次 `cargo fmt --all`。
- **`cargo clippy --workspace --all-targets -- -D warnings` 通过**：退出码 0，
  0 条警告。此前 `result_large_err` 等告警的根因是 `Diagnostic` 约 144 字节；
  `circuit-results` 在 `src/lib.rs` 里用 `#![allow(clippy::result_large_err)]`
  明确接受这个大小（理由是 boxing 会把 `Box<Diagnostic>` 推进公开 API，而错误
  路径本来就在分配内存），其余告警已逐条修掉，现在加 `-D warnings` 也能过。

## 3. 测试分布

`cargo test --workspace 2>&1 | grep "^test result"` 的实测输出：

```
test result: ok. 15 passed; 0 failed;  ...   （circuit-backend lib）
test result: ok. 21 passed; 0 failed;  ...   （circuit-backend tests/adapter.rs）
test result: ok.  9 passed; 0 failed;  ...   （circuit-cli bin）
test result: ok. 18 passed; 0 failed;  ...   （circuit-cli tests/e2e.rs）
test result: ok. 11 passed; 0 failed;  ...   （circuit-cli tests/repl.rs）
test result: ok. 58 passed; 0 failed;  ...   （circuit-core lib）
test result: ok. 84 passed; 0 failed;  ...   （circuit-dsl lib）
test result: ok. 67 passed; 0 failed;  ...   （circuit-dsl tests/elaborate.rs）
test result: ok. 71 passed; 0 failed;  ...   （circuit-results lib）
test result: ok.  9 passed; 0 failed;  ...   （circuit-session lib）
test result: ok. 25 passed; 0 failed;  ...   （circuit-session tests/session.rs）
test result: ok.  0 passed; ...              （circuit-backend / circuit-core / circuit-dsl 的 doc-tests）
test result: ok.  1 passed; ...              （circuit-results 的 doc-test）
```

**合计 389 个测试，全部通过。**（15+21+9+18+11+58+84+67+71+9+25+1 = 389；三个 0 的
doc-test 目标不计。）`cargo nextest run --workspace` 报 388：nextest 不跑 doc-test，
差值就是那一个 doc-test。

| crate | 测试数 | 明细 | 覆盖内容 |
|---|---|---|---|
| `circuit-core` | 58 | `units` 13、`format` 11、`ir` 9、`connectivity` 9、`span` 6、`plan` 4、`diagnostic` 3、`limits` 2、`id` 1 | 单位后缀与量纲运算、**工程计数法显示与"显示出来的单位一定能读回来"**、IR 构造与索引不变量、直流参考路径可达性、源码位置/行列/caret、AC 点数约定、诊断渲染、限制默认值 |
| `circuit-dsl` | 84（lib）+ 67（`tests/elaborate.rs`）= 151 | lib：`parser` 36、`lexer` 25、`complete` 12、`ast` 7、`token` 4 | 词法（续行、量纲字面量、注释、错误恢复、`...`/插值/`=` 的处置）、语法（优先级、块、错误、层次路径、**REPL 输入的四态与针对性提示**）、**多行输入的三态判定**、展开（参数、层次、循环、条件、分析、诊断、限制、**参数位置与覆盖校验**） |
| `circuit-results` | 71（lib）+ 1（doc-test）= 72 | `expr` 23、`measure` 16、`dataset` 14、`export` 14、lib 4 | 表达式求值与量纲、积分/极值、数据形状校验、CSV/JSON 与非有限值策略、端到端 doc 示例 |
| `circuit-backend` | 15（lib）+ 21（`tests/adapter.rs`）= 36 | lib：`sweep` 10、`backend` 5 | 扫描坐标与拓扑比较、失败分类；适配层与真实引擎的数值对比、推导电流（含接地端与 AC）、分析命名、诊断去重 |
| `circuit-session` | 9（lib）+ 25（`tests/session.rs`）= 34 | lib：`execute` 6、`format` 3 | 执行器（单点计划转换、扫描检测、测量优先级）、值显示；**会话行为**：变量、续行、定义替换与回滚、命名空间、作用域边界、覆盖、命令 |
| `circuit-cli` | 9（bin）+ 18（`tests/e2e.rs`）+ 11（`tests/repl.rs`）= 38 | bin：`repl` 6、`run` 3 | CLI 定义与防覆盖输入、端到端命令与数值；**真实进程的管道会话**（定义→运行→改参→再运行、`:load`、冲突、`:run --out`、退出码） |

测试分层，以及为什么这样分：

- `circuit-dsl` 与 `circuit-session` 的测试直接驱动库，不做 I/O，因此毫秒级。
- `circuit-backend/tests/adapter.rs` 真的跑仿真引擎（数值对照，约 0.1 秒）。
- `circuit-cli/tests/*.rs` 真的起进程：`e2e.rs` 跑文件模式，`repl.rs` 用管道驱动
  交互会话——**会话逻辑不依赖终端库**，所以它在没有 TTY 的环境里也能端到端验证。
- 终端按键（Ctrl+C/Ctrl+D/历史/补全）需要真实 TTY，无法自动化；本轮把交互路径里
  可测的部分拆成了函数并加了测试，边界写清楚在 `docs/repl.md` §8。

`cargo test --workspace` 实测约 16 秒（端到端测试占大头，它们要起进程、跑仿真）；
`cargo nextest run --workspace` 约 16 秒。

## 4. 数值验证

### 4.1 Phase-0 准入用例（`docs/backend-evaluation.md` §5，`_probe` 复现）

| # | 用例 | 独立对照 | 实测 |
|---|---|---|---|
| 1 | 分压器 OP | `Vmid = Vin·R2/(R1+R2)` | `v(mid)=0.66666667`，`|diff|=0` |
| 2 | RC 瞬态阶跃 | `v(t)=1-e^{-t/τ}`，τ=100 µs | 最差 `|diff|=1.95e-3`（0.2%），1015 个输出点，时间轴 `[0, 5e-4] s` |
| 3 | RC 交流幅相 | `H=1/(1+jωRC)` | 最差 `|diff|=5.55e-17`（机器精度） |
| 4 | RLC 交流 | `H=1/(1-ω²LC+jωRC)` | 最差 `|diff|=2.31e-15` |
| 5 | 二极管非线性 OP | 二分法独立求解 Shockley 方程（`Vt=0.02586419`） | `v(out)=0.692872` vs `0.692868`，`|diff|=3.28e-6` |
| 6 | DC 扫描 | `v(mid)=0.5·Vsweep` | 6 点全部 `|diff|=0` |

用例 2 的残差来自脉冲的有限上升沿与输出采样对齐，在默认 `RELTOL=1e-3` 下属预期量级，
**不是**后端错误（`docs/backend-evaluation.md` §5）。

### 4.2 测试套件里的数值验证

| 对象 | 对照 | 实测容差 |
|---|---|---|
| 分压器 OP（IR 直构） | `v(mid)=2/3 V`、`i(r1)=1/3000 A`、`i(v1)=-1/3000 A` | 1e-12，且断言两个符号与 KCL 和 < 1e-12 |
| 分压器（CLI，5 V / 1k / 1.5k） | `v(out)=3 V`、`i(r1)=+2 mA`、`i(v1)=-2 mA` | 1e-9，KCL < 1e-12 |
| RC 瞬态（IR 直构，τ=100 µs） | `1-exp(-t/τ)`，在 0.25/0.5/1/2/3 τ 处 | 1e-2；另断言末点 = 5τ（1e-9）、点数 > 100、轴非均匀 |
| RC 瞬态（CLI，`examples/rc_filter.cdsl`） | 同上，在 0.5/1/2/3 τ 处 | 1e-2；轴非均匀 |
| RC 交流 | `H=1/(1+jωRC)`，在 fc/10、fc、10fc 三点 | 复数误差 < 1e-9；另验 10fc 处 ≈ −20 dB（±0.5） |
| RC 交流（CLI） | 同上，取最接近 fc 的频点 | < 1e-9 |
| RLC 交流 | `H=1/(1-ω²LC+jωRC)`，在 f0/5、f0、5f0 三点 | < 1e-9；谐振处 `|H|>1` |
| 二极管 OP | 二分法解 Shockley 方程 | < 5e-3，且 0.3 V < Vd < 0.8 V |
| 二极管 DC 扫描（CLI） | 10 倍电流的压降变化量 | 压降都在 0.3–0.8 V，变化量 0.01–0.15 V |
| DC 源扫描 | `v(mid)=0.5·Vsweep`，6 点 | 1e-12 |
| 参数扫描（CLI） | `v(out)=3·1.5k/(r+1.5k)`，8 点 | 1e-9 |
| 会话显式覆盖（session） | 5 V 分压：默认 `r1=1k` → `v(out)=2.5 V`；`r1=3k` → `1.25 V` | 1e-9，且两次运行不互相影响 |
| 会话运行真实进程（CLI `tests/repl.rs`） | 同一段脚本里的两次数值必须不同（2.5 V → 1.25 V） | 1e-9 |
| 示例载入并运行（CLI `tests/repl.rs`） | `voltage_divider.cdsl`：`v(out)=3 V`、`i(r1)=2 mA` | 字符串断言，取自示例注释的承诺值 |
| 循环建梯形网络（CLI，`examples/ladder.cdsl`） | 节点方程解：`v(midk)=1.5/2^{k-1}`、`i(r0)=1.5 mA`、`i(rs1)=0.75 mA` | 1e-9 |
| 层次探针（IR，`v(:stage1.internal)`、`i(:stage1.r1)`） | 解析到实例内部的节点/器件 id，与非限定叶名（唯一时）等价 | 全等，`name` 为 `v(stage1.internal)` |
| 层次探针 AC（CLI，`two_stage.cdsl --experiment inside`，1 MHz） | 独立复数节点分析（6 节点、含两只电容与负载电容） | 实部/虚部逐位一致（< 1e-15） |
| `avg`（非均匀轴 x=t，t={0,1,2,4}） | `∫x dt/∫dt = 8/4 = 2` | 1e-12，且与样本均值 1.75 的差 > 0.2 |
| `rms`（同一轴） | `sqrt(23/4) = sqrt(5.75) = 2.3979157616563596` | 1e-12，且与样本 RMS `sqrt(5.25)` 的差 > 0.05 |
| 电阻电流推导 | 串联回路中 `i(r1) = -i(v1)`（引擎自己给出的源电流） | 直流 1e-12；交流复数相对误差 1e-9 |
| 差分探针 `v(a,b)` | `v(a) - v(b)`，实数与复数两种 | 1e-12，并断言结果确实带虚部 |
| 测量输出（CLI，RC 阶跃） | 阶跃峰值与 5τ 上的积分平均 | stdout 里 `measure vfinal = 0.9…`、`measure vavg = 0.80…`（工作点会给出 0，因此这个断言同时防住"读错分析"） |

## 5. 容差怎么定的

容差是**逐用例**定的，每条都能说出它挡住什么、放过什么：

- **线性电路（OP/DC/AC）用 1e-12 ~ 1e-9**。线性方程组直接求解，误差只有浮点舍入；
  Phase-0 实测 RC 交流的残差是 5.55e-17、RLC 是 2.31e-15。把阈值定在 1e-9 既能容纳
  浮点累积与单位换算，又能挡住任何"公式写错"——写错的结果偏差是量级级别的，不是 1e-9。
  比较复数时用欧氏距离 `|H_got - H_want|`，避免只比幅值放过相位错误。
- **瞬态用 1e-2**，原因有三：求解器的默认 `RELTOL=1e-3`；激励脉冲有**有限上升沿**
  （例子里 1 ns，Phase-0 探针里 1 ps），而闭式解假设理想阶跃；测试取的是**最接近目标
  时刻的采样点**，本身可能偏离目标时间最多半个步长。τ=100 µs、步长 500 ns 时，
  单单时间对齐就能贡献 1e-3 量级的电压差。收紧到 1e-4 会让正确实现随机失败，
  放松到 1e-1 则连 τ 写成两倍都可能通过。`docs/backend-evaluation.md` 记录了
  这类残差的来源，测试直接引用同一解释。
- **单位换算不假设位相等**。`100.nF` 是 `100 * 1e-9` 的二进制浮点结果，与字面量
  `1e-7` 不保证 bit 相同（`units.rs` 的测试注释明确写了这一点）。因此
  `100.nF` 这类值用绝对 1e-20、`5.us` 用 1e-18 之类的紧公差做近似相等，
  而不是 `assert_eq!`。
- **非线性（二极管）用 5e-3**：Newton 迭代 + 指数模型 + 温度电压 `Vt` 的取值，
  机器精度不可达；Phase-0 实测偏差是 3.28e-6，5e-3 留了两个数量级余量，
  同时仍能挡住"忘了乘 n 的 Vt"或"串了电阻"这类量级错误。再加一条 0.3–0.8 V
  的物理范围断言，防住"数值自洽但物理荒谬"的收敛。
- **端到端里打印值的断言用前缀**（`measure vfinal = 0.9`、`measure vavg = 0.80`），
  因为断言对象是 CLI 的文本输出，格式由 `format_number` 决定；数值本身另有上面的
  逐点比较兜底。

一个全局的宽松阈值（比如到处都是 1e-2）会让分压器、DC 扫描、AC 幅相的所有精度验证
同时失效；一个全局的严格阈值（1e-9）又会让瞬态因为上升沿而"正确地失败"。
所以阈值跟着物理与求解器走，而不是跟着方便走。

## 6. 错误路径测试

失败路径与成功路径同等对待，每个错误码都有触发它的测试：

| 场景 | 期望 | 测试 |
|---|---|---|
| 量纲不符（`10.ms` 当电阻） | `E_DIMENSION`，同时给出 expected/received 与位置 | `elaborate.rs::a_wrong_dimension_is_reported_with_both_dimensions`；CLI 端到端还断言 caret、`bad.cdsl:3`、退出码 1、且"没有模拟任何东西" |
| 使用未声明节点 | `E_NAME`，不隐式建节点，提示如何声明 | `an_undeclared_node_is_an_error_not_a_new_node` |
| 探针指向未知节点/器件 | `E_NAME`，note 里列出可用的节点/器件 | `a_probe_on_an_unknown_node_is_reported_with_the_available_ones`、`a_probe_on_an_unknown_device_is_reported` |
| 器件重名 | `E_DUPLICATE` 并给出两处位置 | `a_duplicate_device_names_both_locations` |
| 循环生成重名 | `E_DUPLICATE` | `duplicate_names_from_a_loop_are_reported` |
| 重复 `save` 同一探针 | `E_DUPLICATE` | `saving_the_same_probe_twice_is_reported` |
| 端口缺失/多余 | `E_PORT`，列出已声明端口 | `a_missing_port_binding_is_reported`、`an_unknown_port_binding_is_reported` |
| 未知参数覆盖 | `E_NAME`，列出已声明参数 | `an_unknown_parameter_override_is_reported` |
| 覆盖值量纲错误 | `E_DIMENSION` | `an_override_with_the_wrong_dimension_is_reported` |
| 递归实例化 | `E_RECURSION` + 调用链 | `recursion_is_reported_with_the_call_chain` |
| R/L/C 为零或负 | `E_VALUE`，**不做钳位替换** | `non_positive_passives_are_rejected_rather_than_clamped`（`0.ohm` 与 `-1.kohm` 两个值） |
| 未声明的参数名 | `E_NAME`（参数按声明顺序求值，前向引用就是未声明） | `an_undeclared_parameter_is_an_error`、`a_forward_parameter_reference_is_rejected` |
| 器件数/循环数超限 | `E_LIMIT` | `the_device_limit_is_enforced`（用 `Limits::for_tests()`）、`the_loop_limit_is_enforced` |
| 扫描参数有问题（步长为 0、方向相反、缺 step） | `E_SWEEP` | `sweep.rs::a_zero_step_is_rejected`、`a_backwards_step_is_rejected`、`a_missing_step_is_rejected_with_advice` |
| 冲突理想源（奇异矩阵） | `E_SINGULAR`，且保留引擎原文 "singular" | `adapter.rs::a_singular_circuit_is_reported_not_swallowed` |
| 参数扫描直接交给单点执行器 | `E_UNSUPPORTED`，说明要走扫描驱动 | `a_parameter_sweep_is_refused_by_the_single_point_executor` |
| 电容电流 | `E_UNSUPPORTED`，理由里出现 "differentiated"，不做近似 | `capacitor_current_is_refused_with_a_reason` |
| 零电阻到达后端 | 报错（`zero` 或 `singular`），绝不能返回无穷大电流 | `zero_resistance_does_not_produce_an_infinite_current` |
| OP 上做 `avg`/`rms` | `E_TYPE`，note 指向 OP 与 `max/min` | `measure.rs::avg_on_an_operating_point_is_a_type_error`、`rms_on_a_dc_sweep_is_a_type_error` |
| 结果形状不符（信号长度 ≠ 轴长、OP 不是 1 个样本、信号重名） | 拒绝并报出全部问题，不补零不截断 | `dataset.rs::signal_length_must_match_the_axis`、`operating_point_requires_exactly_one_sample` 等 |
| CLI 文件不存在 | `E_IO` + 退出码 1 | `e2e.rs::a_missing_file_is_a_user_error` |
| CLI 指定不存在的实验 | 退出码 1 + 列出真实实验名 | `running_an_unknown_experiment_lists_the_real_ones` |
| 输出路径指向输入文件 | 拒绝写入（`guard_output`） | `main.rs::guard_rejects_writing_over_the_input` |

**退出码覆盖情况**：0 与 1 都有端到端断言；**退出码 2（内部错误）定义了但没有测试触发**，
`run.rs` 里只有"datasets 为空"一条路径会返回它。

**"失败不得伪装成空成功"** 在测试里的对应物：`a_singular_circuit_is_reported_not_swallowed`
（必须 `Err`，而不是空结果）、`convert_plot` 对缺失探针报 `E_BACKEND`
（`only_requested_probes_are_returned` 的反面）、以及 `Dataset::validate`
对超限拒绝的单元测试。

## 7. 未验证的部分

明确列出，**不声称**：

- **平台**：只在 Windows MSVC（`stable-x86_64-pc-windows-msvc`，rustc/cargo 1.98.1）
  上构建并运行过。Linux/macOS 在本项目**没有**构建或测试记录
  （`docs/backend-evaluation.md` §7，`cdsl capabilities` 的输出里也带这条 note）。
- **AC/正弦相位换算未验证**：`map_ac` 的 radians→degrees 与 `sin` 波形的 `phi`
  转换没有数值测试——所有测试与 DSL 的 `ac:` 都产生 0 相位。语言目前也没有给
  AC 源设置相位的语法。
- **悬空节点检查已实现**：`circuit-core::connectivity` 做直流参考通路可达性检查，由 `circuit-dsl` 在展开结束时调用。`circuit-core` 里有对应单元测试，`circuit-cli/tests/e2e.rs` 里有两个端到端用例：拒绝只有电容通路的节点，接受带偏置电阻的交流耦合节点。
- **`uic` 未验证、也未暴露**：`TranSpec.uic` 恒为 `false`（`elaborate.rs`），
  语言里没有对应语法（`docs/language.md` §10）。
- **未暴露的分析与器件**：噪声、灵敏度、PZ、TF、Fourier/FFT、Monte Carlo、
  多参数联合扫描；MOSFET、BJT、受控源、行为源、开关、互感、`include`/模型文件。
  后端可能声明支持其中一部分，本项目既不测试也不声称（`docs/backend-evaluation.md` §2、§7）。
- **结果表达式语言未接通 CLI**：`circuit-results::expr` 有求值器与单元测试，
  但 `cdsl` 的 `measure` 只支持 `v(...)`/`i(...)` 探针形式，没有从源文件写
  任意结果表达式的路径。
- **规模性能**：没有数万节点级别的性能/内存测试。
- **退出码 2** 无测试（§6）。

## 8. 回归测试

套件里有明确针对"曾经踩过或极易踩到"的行为的回归钉子：

- **`simulate_tran` 会先返回工作点 plot**（Phase-0 发现 1）。`select_plot` 必须按名字
  前缀选。`adapter.rs::rc_transient_matches_analytic` 是这颗钉子：一旦退回取 `plots[0]`，
  返回的轴不是时间轴，测试以 "expected a time axis" 失败；`rc_ac_matches_analytic`
  同时钉住 AC 的 61 点与频率轴。
- **引擎忽略 `circuit.save`**（Phase-0 发现 2）。`only_requested_probes_are_returned`
  断言一次性请求只返回 `["v(mid)"]`；否则适配层会退回"返回引擎全部内部向量"。
- **测量不得静默读工作点而不是瞬态**。`run.rs::transient_is_preferred_for_measurements`
  钉住 tran < ac < dc < op 的排序；端到端
  `run_rc_filter_matches_the_analytic_response` 钉住输出里的 `measure vfinal = 0.9…`
  （若读成工作点会是 0）。
- **电阻电流推导必须与引擎自己给出的电流一致**：直流用
  `divider_op_and_current_direction`（`i(r1) = -i(v1)`，1e-12），交流用
  `derived_resistor_current_agrees_with_source_current_in_ac`（复数，相对 1e-9）。
  这是允许"推导"而不是"读取"的唯一依据。
- **不可得的电流不得伪造**：`capacitor_current_is_refused_with_a_reason` 钉住
  `E_UNSUPPORTED` 与理由文本（"differentiated"）。
- **求解器时间轴非均匀**：端到端与适配层都显式断言存在相邻步长不等的点。
  若将来有人把 `output_interval` 当成内部步长、或把 `max_step` 当成输出间隔，
  这条断言与 `elaborate.rs` 里"`max_step` 不得变成 output interval"的断言会一起失败。
- **AC 点数约定**：`plan.rs::decade_point_count_matches_ngspice_convention`（10 点/十倍频程、
  2 个十倍频程 → 21 点）与适配层实测的 61 点。
- **测量必须积分而非平均**：§1 里 `avg`/`rms` 与样本均值/样本 RMS 的差值断言。
- **参数扫描的拓扑不变性**：`sweep.rs` 的 `changing_only_a_value_keeps_the_topology`、
  `adding_a_device_is_a_topology_change`、`rewiring_is_a_topology_change`、
  `changing_a_device_kind_is_a_topology_change` 钉住 `Topology` 比什么；
  `elaborate.rs::re_elaborating_with_an_override_changes_the_value_not_the_topology`
  钉住 `elaborate_experiment` 的覆盖只改值不改结构。
- **非有限值策略**：`export.rs` 的 `csv_writes_non_finite_values_as_empty_fields`、
  `json_serialises_nan_and_infinity_as_null_and_round_trips`、
  `json_is_valid_even_when_every_sample_is_non_finite`，以及"每个受影响信号一条警告"。
  NaN 不允许变成一个看起来正常的数字。
- **形状规则**：信号长度必须等于轴长、OP 恰好 1 个样本、信号名归一化后不重复——
  这些是防止"结果错位但看起来合法"的钉子。
