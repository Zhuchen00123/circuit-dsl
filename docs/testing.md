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
`crates/circuit-dsl/tests/elaborate.rs`（67 个）用**真实前端**跑源码字符串
（lex → parse → compile，没有一个测试手搭 AST），因此语法或展开器改动无法蒙混过关；
`crates/circuit-backend/tests/adapter.rs`（21 个）在 Rust 里**直接构造本项目的 IR**，
把它交给真实的 Thevenin 引擎跑，再与闭式解对比。前者是"语言 → IR"的契约，
后者是"IR → 引擎"的契约。本轮（任务 A/B）**新增 4 个集成测试目标并重写 1 个**：
`crates/circuit-backend/tests/output_interval_regression.rs`（6 个：`output_interval` 不改变原始求解网格、
声明 10 ns 边沿被交付、能力错误不再静默展宽、步数预算的**归因**只落在声明波形上）、`crates/circuit-session/tests/tran_output_interval.rs`
（5 个：会话层 raw/output 双视图、测量必须用原始网格、规模上限）、
`crates/circuit-dsl/tests/tran_option_validation.rs`（15 个：`max_step`/`output_interval` 的 0/负/非有限
在 DSL 层被拒绝）、`crates/circuit-backend/tests/source_breakpoint_regression.rs`（6 个：非零 `delay` 的
运行中断点逐点回归 + 限制行保留），并重写
`crates/circuit-backend/tests/transient_reference_regression.rs`（4 个：声明 1 ps 边沿不再被 print step
展宽，旧 `T_eff = max(rise, output_interval)` 参考改为可失败的判别对照）。上一轮新增的
`crates/circuit-dsl/tests/reference_path_regression.rs`（8 个）与
`crates/circuit-backend/tests/phase_regression.rs`（6 个）继续保留。

**第 4 轮（阶段 A + 阶段 B）**新增 10 个测试目标与一次契约改写。阶段 A 的
`circuit-results/tests/r4_expr_policy.rs`（16）、`circuit-session/tests/r4_export_diagnostics.rs`（7）、
`circuit-cli/tests/r4_repro_cli.rs`（8）、`r4_cli_repl_parity.rs`（3）、`r4_output_integrity.rs`（3）、
`r4_regressions.rs`（5）覆盖 R4-01 / R4-02 / R4-03 的四个复现输入、raw Dataset 导出警告、
CLI 与 REPL 同一表达式的数值/错误一致，以及"失败不落盘"；阶段 B 的
`circuit-dsl/src/param_graph.rs`（17 个单元测试）、`circuit-dsl/tests/r4b_param_graph.rs`（13）、
`circuit-cli/tests/r4b_topo_sweep.rs`（6）、`r4b_regressions.rs`（3）、
`circuit-session/tests/r4b_session_dag.rs`（3）覆盖参数 DAG（前向引用、多层链、菱形图、环与
span、未知名字）、同名实例参数互不污染、覆盖后重算、REPL 事务性、check 期拓扑扫描拒绝与
普通数值扫描正控制。`crates/circuit-dsl/tests/elaborate.rs` 只改了两条旧规则测试并改名：
`a_forward_parameter_reference_is_rejected` → `a_forward_parameter_reference_is_resolved`、
`a_self_referential_parameter_is_rejected` → `a_self_referential_parameter_is_a_cycle`，
其余 65 条未动（该文件仍是 67 个测试）。

**CLI 端到端测试** `crates/circuit-cli/tests/e2e.rs`（18 个）用
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

**本仓库当前状态（第 4 轮）**：`cargo test --workspace` → **660 passed / 0 failed / exit 0**
（第 3 轮基线 554）。构成：阶段 A 冻结快照 **618**（`docs/review-evidence/round4/acceptance.md`），
阶段 B 新增 **42** 个测试——`crates/circuit-dsl/src/param_graph.rs` 的 17 个单元测试、
`crates/circuit-dsl/tests/r4b_param_graph.rs` 13、`circuit-cli/tests/r4b_topo_sweep.rs` 6、
`circuit-cli/tests/r4b_regressions.rs` 3、`circuit-session/tests/r4b_session_dag.rs` 3
（`docs/review-evidence/round4/qa-acceptance-phase-b.md` §2）——外加
`crates/circuit-dsl/tests/elaborate.rs` 里两条按契约 §4.4 **改写并改名**的旧规则测试
（数量不变，该文件仍是 67 个）。618 + 42 = 660。
`cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo fmt --all -- --check` 在阶段 A
冻结快照上实测 **exit 0**（`docs/review-evidence/round4/acceptance.md` §Gates）；第 4 轮最终复跑
由 lead 在 task-15 执行。**测试计数只在这一处与 `docs/review-evidence/round4/` 里维护**；
下面的 rev5/rev4 段落是历史记录，保留不改。

（rev4 历史，仅供对照）门禁三条全部实跑：`cargo test --workspace` → **457 passed / 0 failed /
exit 0**；`cargo clippy --workspace --all-targets -- -D warnings` → **exit 0**；
`cargo fmt --all -- --check` → **exit 0**。日志 `target/round2-logs/rev4-workspace-test.txt`（rev4）、
`final-workspace-test.txt`（rev3）、`final-clippy.txt`、`final-fmt.txt`，冻结清单 `target/round2-logs/freeze-manifest.txt`（rev4）。
（历史：rev2 时 clippy 曾被 `circuit-results/src/resample.rs` 的 `neg_cmp_op_on_partial_ord`
挡成 exit 101，rev3 已修。本文件作者本轮也独立复跑过 `cargo test --workspace`，同样得到
457 passed / 0 failed / exit 0（rev3 时为 456，rev4 增加 1 个归因回归）。`_probe` 侧另有
`cargo fmt --manifest-path _probe/Cargo.toml -- --check` → exit 0。）

- **`cargo fmt --all -- --check`（rev4 实测通过）**：退出码 0，无输出。这是格式检查命令，
  不修改文件；要改格式需要单独跑一次 `cargo fmt --all`。
- **`cargo clippy --workspace --all-targets -- -D warnings`（rev4 实测通过）**：退出码 0，
  0 条警告。此前 `result_large_err` 等告警的根因是 `Diagnostic` 约 144 字节；
  `circuit-results` 在 `src/lib.rs` 里用 `#![allow(clippy::result_large_err)]`
  明确接受这个大小（理由是 boxing 会把 `Box<Diagnostic>` 推进公开 API，而错误
  路径本来就在分配内存），其余告警已逐条修掉，现在加 `-D warnings` 也能过。

## 3. 测试分布

`cargo test --workspace` 本轮实测输出（完整日志 `target/round2-logs/rev4-workspace-test.txt`；rev3 门禁日志
`target/round2-logs/final-workspace-test.txt` 为 456 passed，rev4 增加 1 个归因回归后为 **457 passed / 0 failed / exit 0**）：

```
test result: ok. 15 passed; 0 failed; 0 ignored; ...   （circuit-backend lib）
test result: ok. 21 passed; 0 failed; 0 ignored; ...   （circuit-backend tests/adapter.rs）
test result: ok.  6 passed; 0 failed; 0 ignored; ...   （circuit-backend tests/output_interval_regression.rs）
test result: ok.  6 passed; 0 failed; 0 ignored; ...   （circuit-backend tests/phase_regression.rs）
test result: ok.  6 passed; 0 failed; 0 ignored; ...   （circuit-backend tests/source_breakpoint_regression.rs）
test result: ok.  4 passed; 0 failed; 0 ignored; ...   （circuit-backend tests/transient_reference_regression.rs）
test result: ok.  9 passed; 0 failed; 0 ignored; ...   （circuit-cli bin）
test result: ok. 18 passed; 0 failed; 0 ignored; ...   （circuit-cli tests/e2e.rs）
test result: ok. 11 passed; 0 failed; 0 ignored; ...   （circuit-cli tests/repl.rs）
test result: ok. 58 passed; 0 failed; 0 ignored; ...   （circuit-core lib）
test result: ok. 84 passed; 0 failed; 0 ignored; ...   （circuit-dsl lib）
test result: ok. 67 passed; 0 failed; 0 ignored; ...   （circuit-dsl tests/elaborate.rs）
test result: ok.  6 passed; 0 failed; 0 ignored; ...   （circuit-dsl tests/phase_syntax_regression.rs）
test result: ok.  8 passed; 0 failed; 0 ignored; ...   （circuit-dsl tests/reference_path_regression.rs）
test result: ok. 15 passed; 0 failed; 0 ignored; ...   （circuit-dsl tests/tran_option_validation.rs）
test result: ok. 83 passed; 0 failed; 0 ignored; ...   （circuit-results lib）
test result: ok.  9 passed; 0 failed; 0 ignored; ...   （circuit-session lib）
test result: ok. 25 passed; 0 failed; 0 ignored; ...   （circuit-session tests/session.rs）
test result: ok.  5 passed; 0 failed; 0 ignored; ...   （circuit-session tests/tran_output_interval.rs）
test result: ok.  1 passed; ...                       （circuit-results 的 doc-test；其余 4 个目标为 0）
```

**（rev4 历史上为 457；第 3 轮实测清单见 `docs/review-evidence/round3/acceptance.md`；第 4 轮经本文件开头与 `docs/review-evidence/round4/` 更新为 660 个测试全部通过、退出码 0。）**

**合计 457 个测试，全部通过，`cargo test --workspace` 退出码 0。**
（15+21+6+6+6+4+9+18+11+58+84+67+6+8+15+83+9+25+5+1 = 457；24 条 `test result:` 行 = **19 个测试
二进制 + 5 个 doc-test 目标**，其中只有 `circuit-results` 的 doc-test 含 1 个测试。）
与上一轮基线 413 的差：新增 4 个集成测试目标（6 + 15 + 5 + 6 = 32）与 `circuit-results` 的
`resample` 单元测试 12 个（71 → 83），共 +44 ⇒ 457（其中 1 个是代码审核 R3 建议补的步数预算归因断言）。作为历史对照，再上一轮基线是 389
（当时没有 6 + 4 + 8 + 6 = 24 这四个目标）；旧基线时 `cargo nextest --workspace` 报 388
（nextest 不跑 doc-test），本轮**未复跑 nextest**。

| crate | 测试数 | 明细 | 覆盖内容 |
|---|---|---|---|
| `circuit-core` | 58 | `units` 13、`format` 11、`ir` 9、`connectivity` 9、`span` 6、`plan` 4、`diagnostic` 3、`limits` 2、`id` 1 | 单位后缀与量纲运算、**工程计数法显示与"显示出来的单位一定能读回来"**、IR 构造与索引不变量、直流参考路径可达性、源码位置/行列/caret、AC 点数约定、诊断渲染、限制默认值 |
| `circuit-dsl` | 84（lib）+ 67（`tests/elaborate.rs`）+ 8（`tests/reference_path_regression.rs`）+ 6（`tests/phase_syntax_regression.rs`）+ 15（`tests/tran_option_validation.rs`）= 180 | lib：`parser` 36、`lexer` 25、`complete` 12、`ast` 7、`token` 4 | 词法（续行、量纲字面量、注释、错误恢复、`...`/插值/`=` 的处置）、语法（优先级、块、错误、层次路径、**REPL 输入的四态与针对性提示**）、**多行输入的三态判定**、展开（参数、层次、循环、条件、分析、诊断、限制、**参数位置与覆盖校验**）、**瞬态选项 0/负/非有限全部拒绝且不回退默认值** |
| `circuit-results` | 83（lib，含 `resample` 12）+ 1（doc-test）= 84 | `expr` 23、`measure` 16、`dataset` 14、`export` 14、`resample` 12、lib 4 | 表达式求值与量纲、积分/极值、数据形状校验、CSV/JSON 与非有限值策略、**输出网格契约（首末点/等间隔/不越界外推/复数/规模上限/非时间轴原样）**、端到端 doc 示例 |
| `circuit-backend` | 15（lib）+ 21（`tests/adapter.rs`）+ 6（`tests/output_interval_regression.rs`）+ 6（`tests/phase_regression.rs`）+ 6（`tests/source_breakpoint_regression.rs`）+ 4（`tests/transient_reference_regression.rs`）= 58 | lib：`sweep` 10、`backend` 5 | 扫描坐标与拓扑比较、失败分类；适配层与真实引擎的数值对比、推导电流（含接地端与 AC）、分析命名、诊断去重；**`output_interval` 不进求解器、声明边沿被交付、断点逐点回归、步数预算归因** |
| `circuit-session` | 9（lib）+ 25（`tests/session.rs`）+ 5（`tests/tran_output_interval.rs`）= 39 | lib：`execute` 6、`format` 3 | 执行器（单点计划转换、扫描检测、测量优先级）、值显示；**会话行为**：变量、续行、定义替换与回滚、命名空间、作用域边界、覆盖、命令；**raw/output 双视图与测量不变性** |
| `circuit-cli` | 9（bin）+ 18（`tests/e2e.rs`）+ 11（`tests/repl.rs`）= 38 | bin：`repl` 6、`run` 3 | CLI 定义与防覆盖输入、端到端命令与数值；**真实进程的管道会话**（定义→运行→改参→再运行、`:load`、冲突、`:run --out`、退出码） |

测试分层，以及为什么这样分：

- `circuit-dsl` 与 `circuit-session` 的测试直接驱动库，不做 I/O，因此毫秒级。
- `circuit-backend/tests/adapter.rs` 真的跑仿真引擎（数值对照，约 0.1 秒）。
- `circuit-cli/tests/*.rs` 真的起进程：`e2e.rs` 跑文件模式，`repl.rs` 用管道驱动
  交互会话——**会话逻辑不依赖终端库**，所以它在没有 TTY 的环境里也能端到端验证。
- 终端按键（Ctrl+C/Ctrl+D/历史/补全）需要真实 TTY，无法自动化；本轮把交互路径里
  可测的部分拆成了函数并加了测试，边界写清楚在 `docs/repl.md` §8。

`cargo test --workspace` 本轮实测 **14.5 秒**（`Measure-Command`，已编译后的增量运行；端到端测试占大头，
它们要起进程、跑仿真）。`cargo nextest run --workspace` 上一轮约 16 秒，本轮**未复跑**。

## 4. 数值验证

### 4.1 Phase-0 准入用例（`docs/backend-evaluation.md` §5，`_probe` 复现）

| # | 用例 | 独立对照 | 实测 |
|---|---|---|---|
| 1 | 分压器 OP | `Vmid = Vin·R2/(R1+R2)` | `v(mid)=0.66666667`，`|diff|=0` |
| 2 | RC 瞬态（有限斜坡） | 分段匹配解析解：`T_eff = max(声明 rise, .tran step)`、稳定形式 `x + expm1(-x)`，对每个采样点套 §17 判据（atol 1e-5 V / rtol 1e-3）。（该 `_probe` 配置 `rise = 1 µs ≥ step`，新旧步长规则同值；产品路径现在按声明边沿建模，见 §5.1.1） | 基线 6025 点，最大误差 4.999167e-7 V，超限 0/6025；反事实（同一输出 vs 理想阶跃参考）4.966368e-3 V，超限 1789/6025 |
| 3 | RC 交流幅相 | `H=1/(1+jωRC)` | 最差 `|diff|=5.55e-17`（机器精度） |
| 4 | RLC 交流 | `H=1/(1-ω²LC+jωRC)` | 最差 `|diff|=2.31e-15` |
| 5 | 二极管非线性 OP | 二分法独立求解 Shockley 方程（`Vt=0.02586419`） | `v(out)=0.692872` vs `0.692868`，`|diff|=3.28e-6` |
| 6 | DC 扫描 | `v(mid)=0.5·Vsweep` | 6 点全部 `|diff|=0` |

用例 2 的旧归因（「有限上升沿 + 采样对齐」）本轮已被实测推翻：引擎把 PULSE 的 `tr`/`tf` 夹到
`.tran` 步长（`thevenin-0.5.0/src/waveform.rs:37`），旧配置里真正生效的是 **500 ns 斜坡**，
1.95e-3 V 是「参考模型用了理想阶跃」造成的确定性差异（`|C(500 ns)| = 2.504e-3 V`），
**不是**后端错误、也不是采样对齐——参考值用实际返回时间求值，对齐误差恒为 0
（`docs/review-evidence/rc-reference-math.md` §7）。机理、`max_step` 三档与容差分组实验
见 `docs/backend-evaluation.md` §5。

任务 A 之后产品路径的映射已改（`thevenin.rs::tran_step_for`：print step 只由声明边沿与窗口决定，
`output_interval` 不再进入求解器），所以**声明边沿不再被展宽**；上表用例 2 的 `_probe` 配置
（`tr = 1 µs > step = 500 ns`）本来就不触发夹取，数字不变。产品侧的新实测见 §4.2 与
`docs/backend-evaluation.md` §5.1.1。

### 4.2 测试套件里的数值验证

| 对象 | 对照 | 实测容差 |
|---|---|---|
| 分压器 OP（IR 直构） | `v(mid)=2/3 V`、`i(r1)=1/3000 A`、`i(v1)=-1/3000 A` | 1e-12，且断言两个符号与 KCL 和 < 1e-12 |
| 分压器（CLI，5 V / 1k / 1.5k） | `v(out)=3 V`、`i(r1)=+2 mA`、`i(v1)=-2 mA` | 1e-9，KCL < 1e-12 |
| RC 瞬态（IR 直构，τ=100 µs） | `1-exp(-t/τ)`，在 0.25/0.5/1/2/3 τ 处 | 1e-2；另断言末点 = 5τ（1e-9）、点数 > 100、轴非均匀 |
| RC 瞬态（CLI，`examples/rc_filter.cdsl`，`rise = fall = 1.ns`） | `1-exp(-t/τ)`，在 0.5/1/2/3 τ 处；另与**匹配的 1 ns 斜坡解析解**逐点对照 | 1e-2；输出网格 = 求解网格（未给 `output_interval`），1019 点、末点 500 µs；`v(vin)` 在 t = 1 ns 到 1 V；vs 理想阶跃 max 4.921844e-6 V（= 1 ns 斜坡的 `\|C(1 ns)\| = 5.000007e-6 V` 量级），vs 匹配斜坡参考 max 7.916906e-7 V |
| 有限斜坡逐点（重写 `tests/transient_reference_regression.rs`） | `stop = 5.01e-4 s`、`step` 由新映射给出、`tmax = tau/1000 = 100 ns`、`T_eff = 声明 1 µs` 的分段解析解，逐点套 §17 判据 | `rc_ramp`：5025 点，最大误差 7.990897e-8 V，超限 0；`max_step` 三档（1e-6 / 1e-7 / 1e-9 s）→ 516 / 5025 / 50115 点，最大误差 2.494e-6 / 7.991e-8 / 8.672e-10 V，三档全部 0 超限 |
| 声明边沿不再被展宽（同一个文件，`declared_rise_below_output_interval_is_not_widened`） | `output_interval = τ/200 = 500 ns`（声明的 1 ps 的 5e5 倍）：按**声明的 1 ps** 与按**旧契约的 `T_eff = 500 ns`** 两种匹配参考各跑一遍 | 声明边沿参考：1024 点，最大误差 7.644154e-7 V，0 超限；旧契约参考：257/1024 超限、最大误差 2.483260e-3 V（判别对照——若适配层退回旧映射，这条会红） |
| 输出采样不改变求解（`tests/output_interval_regression.rs`） | 同一电路（rise = fall = 10 ns、`max_step = 1 ns`、`stop = 2 us`）跑三遍：无 / `output_interval: 1.ns` / `100.ns` | 三份**原始**网格都是 2015 点、设置逐项相同（`tran.solver_step = 2 ns`、`waveform_bound = 10 ns`、`solve_points = 2015`）；距 50 ns 最近的样本三份都是 `v(vin) = 1.0 V`（旧契约下 coarse 为 0.5002375000000003 V）；`max_step` 三档（4e-9 / 1e-9 / 2.5e-10 s）→ 516 / 2015 / 8015 点 |
| 会话层 raw/output 双视图（`circuit-session/tests/tran_output_interval.rs`） | 同一实验 `output_interval: 1.ns`、`100.ns`、缺省三遍，经真实前端 | raw 2015 点；输出视图 2001 / 21 / 2015 点，首末点与 raw 相同；`max/min/avg/rms` 两种间隔**逐位相同**（如 `avg = 0.009884244068709601`，`bits-equal = true`）；`output_interval: 1.ps` 在 `Limits::for_tests()` 下报 `E_LIMIT`（需要 2000001 点 / 4000002 值 > 10000），raw 数据集仍完整 |
| 运行中断点逐点回归（`tests/source_breakpoint_regression.rs`，非零 `delay`） | τ = 100 µs、`delay = 100 µs`、`rise = fall = 1 µs`、10 个脉冲 40 个断点、分段解析解（`expm1` 形式）逐点套 §17 | `max_step = τ/1000`：3129 点 max 4.999167e-7 V **0 超限**；`τ/500`：1629 点 max 1.999333e-6 V **0 超限**；`τ/200`：729 点 max 1.248959e-5 V **3 超限**（限制行保留）；`τ/50`：309 点 max 7.331775e-4 V **250 超限**（限制行保留）；40/40 断点精确落在返回轴上（偏差 0.0 s） |
| 非零相位（新增 `tests/phase_regression.rs`，6 个测试） | 适配层两条相位通道逐分量对照：AC 源相位与 `sin` 的 `phi`，判据 `atol = 1e-8`、`rtol = 1e-4`（线性电路另加 1e-9 的额外界） | RC 在 fc、源相位 +0.523599 rad：`v(out)` 实部 6.83012701892219298e-1、虚部 -1.83012701892219382e-1，分量误差 1.144392e-16（该点目标 7.072068e-5）；`sin` 相位全区间最大误差 5.551115e-17，若把 φ 当弧度则 t=0 从 2.5e-1 V 变成 4.56919769858802390e-3 V |
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
- **瞬态的两套容差：旧的 1e-2 与新增的逐点 §17 判据（`atol = 1e-5 V`、`rtol = 1e-3`）。**
  `adapter.rs` 与 `e2e.rs` 的 RC 对照用理想阶跃解 `1 - exp(-t/τ)`，而引擎实际收到的是宽度
  `T = max(声明 rise, print step)` 的**有限斜坡**（`thevenin-0.5.0/src/waveform.rs:37`）。
  两者之差是**确定性、可解析**的：`C(T)·e^{-t/τ}`，`C(T) = 1 - (tau/T)(e^{T/tau}-1) ≈ -T/(2τ)`。
  任务 A 之后 print step 只由声明边沿与窗口决定，`examples/rc_filter.cdsl` 的 `T` 就是声明的
  1 ns ⇒ `|C| = 5.000007e-6 V`，实测对理想阶跃最差差 **4.921844e-6 V**，对匹配的 1 ns 斜坡参考
  只差 7.916906e-7 V（§4.2）。旧值 2.491963e-3 V 对应 `T = 500 ns`（修复前
  `output_interval`/`span/1000` 被映射进 `Tran.step` 的行为），已随修复消失。
  1e-2 仍然保留：它本来就是**容纳参考模型与实际边沿的差距**（不是随机噪声），只是这个差距现在
  小了约 500 倍。**「采样对齐」不是原因**：参考值用实际返回时间求值（`_probe/src/main.rs:322`、
  `adapter.rs:344`、`e2e.rs:325`），对齐误差恒为 0。
  `transient_reference_regression.rs` 用匹配 `T_eff` 的解析解 + 逐点 §17 判据钉住这条差距
  （`T_eff = 声明的 1 µs`：5025 点、最大误差 7.990897e-8 V、0 超限），并保留一条**判别对照**：
  按旧契约 `T_eff = max(rise, output_interval) = 500 ns` 建模时 257/1024 点超限、最大误差
  2.483260e-3 V —— 适配层若退回旧映射，这条会红。
  粗 `max_step` 下**运行中断点**的超限点（产品回归 τ/200 → 3/729、τ/50 → 250/309）是保留的
  限制项，未放宽阈值（`docs/backend-evaluation.md` §5.1.2、
  `docs/review-evidence/round2/breakpoint-evidence.md` §5）。
- **读 `_probe` 结果的口径（F3，提示级）**：`_probe` 的退出码**不反映** `max_step` 实验里
  两档 NOT-MET（`τ/50`、`τ/200`）——Case 2 的 PASS 判据是「3 档都跑通 + 推荐档
  `tmax = τ/1000` 合格」，NOT-MET 两行只被逐字打印并保留，进程仍 exit 0
  （`docs/review-evidence/numerical-review.md` §8）。所以「`probe` exit 0」**不能**读成
  「所有配置都满足 §17」；判定某一档是否达标必须看它自己那一行的 over-limit 计数。
  任务 B 新增的两个内核 bin 采用同一口径：`breakpoint_study`（14/14 契约检查、exit 0，§17
  的 `[NOT-MET]` 行保留）与 `tran_contract`（12/12 契约钉、exit 0）——**契约检查失败或 panic
  才会 exit 1**。
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
同时失效；一个全局的严格阈值（1e-9）又会让瞬态因为（参考未建模的）边沿差异而"正确地失败"。
所以阈值跟着物理与求解器走，而不是跟着方便走。本轮把瞬态从「宽松绝对值」推进到
「匹配参考 + 逐点 §17 判据」后，剩下的差距只有两处**可定位、可复现**的来源，且都已进入产品回归
并以限制项原样保留：运行中断点的 Backward-Euler 重启步（τ/200 → 3/729、τ/50 → 250/309 超限，
§4.2 与 `docs/backend-evaluation.md` §5.1.2），以及产品路径没有容差通道
（`RELTOL`/`ABSTOL`/`TRTOL` 不可达）。两者都不是"阈值太紧"，也没有靠放宽阈值掩盖。

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
| `tran` 的 `max_step` / `output_interval` 为 0、负或非有限 | `E_VALUE`，span 指向该实参，**不回退默认值** | `tran_option_validation.rs::output_interval_zero_is_rejected`、`output_interval_negative_is_rejected`、`output_interval_nan_is_rejected`、`output_interval_infinity_is_rejected`、`max_step_must_be_finite_and_greater_than_zero`；CLI 实测 `-1.ns`/`0.s` 在 `check` 与 `run` 均 exit 1、无结果文件（`docs/review-evidence/round2/cli-qa.md` §5 F1–F5） |
| 声明 PULSE 的 `rise`/`fall`/`period` 为 0 或非有限 | `E_UNSUPPORTED`，点名源与参数，不静默展宽边沿 | `output_interval_regression.rs::an_edge_that_cannot_be_honoured_is_refused_not_widened`；CLI 实测 `rise: 0.s` → exit 1（`cli-qa.md` §5 F6/F7） |
| 声明边沿相对窗口过细（**由声明波形导致的**步数预算超限） | `E_LIMIT`，消息含所需步数与归因上下文（`declared waveform timing` / `solver step` / `effective step`）；**用户自己的 `max_step` 造成的超预算不报错** | `output_interval_regression.rs::the_step_budget_blames_the_waveform_not_the_users_max_step`（消息与归因）+ `::an_edge_that_cannot_be_honoured_is_refused_not_widened`（`E_LIMIT` 码）；CLI 实测 `rise: 1.ps` + `stop: 1.s`（无 `max_step`）→ exit 1，消息 `the declared source rise/fall/period is too fine for this simulation window: honouring it would need about 1000000000000 solver steps, over the limit of 1000000`（`cli-qa.md` §5 F8/F9 是同一路径的旧文案实例） |
| 纯 `max_step` 造成的步数超预算（没有声明边沿，或没有波形约束时同样超预算） | **不报错**：`max_step` 是用户显式请求，不由波形契约拒绝（但也没有运行期步数上限，见 §7） | 本文件作者 CLI 实测（rev3）：纯 DC 源 + RC + `tran stop: 1.s, max_step: 1.ns` → `cdsl check` **exit 0**（修复前曾被误拒，代码审核 W6-2 反例） |
| 输出重采样规模超过 `Limits::max_result_values` | `E_LIMIT`（**重采样层**；`check` 是静态检查，只有 `run` 能报） | `tran_output_interval.rs::an_oversized_output_grid_is_rejected_and_truncates_nothing`、`resample.rs::an_oversized_output_is_rejected_not_truncated` |
| 未声明的参数名 | `E_NAME`（前向引用在同一 body 内合法；环才报 `E_PARAM_CYCLE`） | `an_undeclared_parameter_is_an_error`、`a_forward_parameter_reference_is_resolved`（前向引用按依赖序求值）、`r4b_param_graph.rs` 的未知名字用例 |
| 参数自引用 / 多节点环 | `E_PARAM_CYCLE` + 闭合路径 + 每个参与声明的位置（不再是 `E_NAME`） | `a_self_referential_parameter_is_a_cycle`、`r4b_param_graph.rs` 的两节点与三节点环 |
| 结果表达式非法值：`sqrt(-1)`、`min(sqrt(-1),2)`、`1e308*1e308`、非有限复数分量 | 检查（常量表达式）或运行期 `E_VALUE`，带分析、样本坐标与 index | `r4_expr_policy.rs`（16）、`r4_repro_cli.rs`（真实 CLI，debug+release）、`r4_cli_repl_parity.rs` |
| 量纲指数溢出（128 个 `v(:vin)` 因子、`V^-129`） | `E_DIMENSION`；debug 不 panic、release 不回绕 | `r4_repro_cli.rs`、`r4_expr_policy.rs`、reviewer 的 `dimbound/neg131` |
| 表达式深度超过 `MAX_EXPR_DEPTH`（256） | `E_LIMIT`（不 abort、无栈溢出） | `parser.rs` 的深度护栏测试、`expr.rs` 的 `depth()` 边界测试、reviewer 的 `nest300`/`const512` |
| 扫描拓扑参数（`for` 迭代源、`if` 条件、生成名称、实例 `params:` 绑定） | `E_TOPO_PARAM` + 解释路径，check / run / `:load` 一致 | `r4b_topo_sweep.rs`（6）、`r4b_session_dag.rs`、`examples/parameter_sweep.cdsl` 正控制 |
| raw Dataset 含 NaN/±inf 的 CSV/JSON 导出 | 空字段 / `null` + 每个非有限信号一条警告，CLI 与 REPL 打印 | `r4_export_diagnostics.rs`（7）、reviewer 的手工 Dataset 探针 |
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

**退出码覆盖情况**：0 与 1 都有端到端断言；`crates/circuit-cli/src/main.rs:26` 定义了
`EXIT_INTERNAL: u8 = 2`，但**全仓库没有任何返回点**（`grep EXIT_INTERNAL` 只有这一处定义，
`check`/`run`/`repl` 的失败分支全部返回 `EXIT_USER_ERROR = 1`）。独立 QA
（`docs/review-evidence/round2/cli-qa.md` §5、§6）本轮实测 **13 条失败路径**
（`E_VALUE`、`E_UNSUPPORTED`、两层 `E_LIMIT`、`E_NAME`、`E_IO`、`E_ARGUMENT`）退出码**全是 1**、
stdout 0 字节、零结果文件，从未观察到 2；任何依赖「2 = 内部错误」的外部脚本都走不到该分支
（该不可达性只有源码静态检索证据，未通过注入内部错误实证）。

**"失败不得伪装成空成功"** 在测试里的对应物：`a_singular_circuit_is_reported_not_swallowed`
（必须 `Err`，而不是空结果）、`convert_plot` 对缺失探针报 `E_BACKEND`
（`only_requested_probes_are_returned` 的反面）、以及 `Dataset::validate`
对超限拒绝的单元测试。

## 7. 未验证的部分

明确列出，**不声称**：

- **平台**：只在 Windows MSVC（`stable-x86_64-pc-windows-msvc`，rustc/cargo 1.98.1）
  上构建并运行过。Linux/macOS 在本项目**没有**构建或测试记录
  （`docs/backend-evaluation.md` §7，`cdsl capabilities` 的输出里也带这条 note）。
- **相位换算：适配层已覆盖，产品端到端仍未覆盖。** 本轮新增
  `crates/circuit-backend/tests/phase_regression.rs`（6 个测试），直接构造项目 IR 驱动适配层的
  两条相位通道：AC 源相位（±30°/45°/60°/90° 等，逐个分量核对实部与虚部，判据
  `atol = 1e-8`、`rtol = 1e-4`，线性电路另加 1e-9 的额外界）与 `sin(phase:)`
  （t=0 采样精确等于 `v0 + va·sin φ`，全区间最大误差 5.551115e-17；反假设「φ 当弧度」
  给出 4.56919769858802390e-3 V，差两个数量级）。**仍未覆盖**：DSL 没有 `ac phase:` 语法
  （`elaborate.rs:819-822` 写死 `phase_rad: 0.0`），所以非零 AC 相位没有「源文件 → CLI」的
  端到端路径；`sin(phase:)` 虽有语法（`elaborate.rs:1728-1734`，度→弧度），但相位测试只覆盖
  电阻分压这类无电抗电路，含 C/L 的状态电路与端到端路径都没有用例。
- **悬空节点检查：前端规则已实现且有测试，后端行为本轮已定界，但仍有角落未覆盖。** 已覆盖：
  `circuit-core::connectivity` 的通达性规则（9 个单元测试）、`circuit-cli/tests/e2e.rs` 的 2 个端到端
  用例（拒绝只经电容耦合的节点、接受带偏置电阻的交流耦合节点）、本轮新增
  `crates/circuit-dsl/tests/reference_path_regression.rs`（8 个，经真实前端 lex→parse→compile：
  合法开路接受 / 孤立电阻网拒绝 / 电感与二极管算直流通路 / 电流源不算 / 已声明未连接的节点单独报）。
  **仍未覆盖**：含非线性器件时前端判定与后端 gmin 行为之间的差异（后端只在 A04 的仓库外实验里
  测过孤立岛 + 二极管，见 `docs/backend-evaluation.md` §4.6），以及 AC/TRAN 分析下的同类路径。
- **`uic` 产品路径未暴露**：`TranSpec.uic` 恒为 `false`（`elaborate.rs:2494`），语言里没有
  对应语法（`docs/language.md` §10）。引擎语义已由 A04 取证（`uic=true` 跳过 OP、初值全 0，见
  `docs/review-evidence/backend-contract.md` §3.C），**产品端到端未验证**。
- **未暴露的分析与器件**：噪声、灵敏度、PZ、TF、Fourier/FFT、Monte Carlo、
  多参数联合扫描；MOSFET、BJT、受控源、行为源、开关、互感、`include`/模型文件。
  后端可能声明支持其中一部分，本项目既不测试也不声称（`docs/backend-evaluation.md` §2、§7）。
- **结果表达式的运行期边界**（第 3 轮起已接通 CLI 与 REPL；早前"未接通 CLI"的说法已废弃）：
  `derive` 与表达式形式的 `measure` 都走 `circuit-results::expr`，但表达式仍不支持
  比较/布尔/条件、数组、字典、带量纲字面量、自定义函数以及 `derive` 之间的引用
  （`docs/language.md` §7.8）；深度超过 `MAX_EXPR_DEPTH`（256）报 `E_LIMIT`。
  **未验证**：交互式 TTY 会话、磁盘写失败路径，以及"手工构造的超深 `Expr` 直接调用
  公开的 `expr::is_constant` / `expr::from_ir`"——这两个 API 仍是递归实现，CLI 路径
  由解析器的深度护栏先挡住（`docs/review-evidence/round4/acceptance.md` 的
  "Deviations and limits recorded"）。
- **运行中源断点的精度只有单激励的界（限制项，不是通过项）。**
  本轮新增 `crates/circuit-backend/tests/source_breakpoint_regression.rs`（6 个测试）用真实产品路径
  覆盖非零 `delay`：τ = 100 µs、`delay = 100 µs`、`rise = fall = 1 µs`、10 个脉冲 40 个断点，
  逐点套 §17。必过配置 `max_step = τ/1000`（3129 点，max 4.999167e-7 V）与 `τ/500`（1629 点，
  0 超限）；**`τ/200`（3/729 超限，max 1.248959e-5 V）与 `τ/50`（250/309 超限，max 7.331775e-4 V）
  作为限制行原样保留，不放宽阈值、不删除样本**。可达标的经验界
  `h_max ≤ 10·sqrt(2·atol·τ·T/V0)`（本例 4.472136e-7 s = τ/223.6）**只对该激励推导**；多极点网络、
  不同 `T/τ`、电感/二极管电路**未验证**，产品路径也没有对过粗 `max_step` 的校验。
  （出处：`docs/review-evidence/round2/breakpoint-evidence.md` §4–§6、§9。）
- **重采样边界组合未穷尽**：已覆盖首末点、等间隔、不越界外推、单点/退化轴、复数信号、规模上限、
  非时间轴原样返回（`circuit-results/src/resample.rs` 单元测试）与 raw/output 双视图（会话测试）；
  **未验证** `start_s ≠ 0`、`uic = true`、同一实验多个 `tran` 任务、参数扫描路径
  （产出 `Axis::Parameter`，不经重采样）的组合。
- **`run --format json` 单格式**：W5 QA 已对 `pulse-fine` / `pulse-coarse` 单独跑
  `--format json` 并核对元数据（`docs/review-evidence/round2/cli-qa.md` §7/§8）；但 `e2e.rs`
  没有只写 JSON 的数值断言，也没有对 `examples/` 三例单独跑 `--format json`。
- **没有运行期步数上限（既有限制）**：`max_step` 是用户显式请求，`check` 不会因为
  "极小 `max_step` + 长窗口" 而拒绝（纯 DC 源 + RC + `tran stop: 1.s, max_step: 1.ns` 实测
  `check` exit 0，约 1e9 个求解步）。`Limits::max_result_values = 5e7` 只在结果生成后生效，
  不是求解过程的保护。**该组合的实际运行时长未实测**；文档只声明"可能非常长"，不声称会 OOM 或
  超时退出（修复前同样如此，不是本轮引入）。
- **未在 release profile / 其他平台验证**：CLI 与 `_probe` 的所有本轮实测都是 debug 产物、
  Windows MSVC。（`cargo clippy` / `cargo fmt` 已在 rev3 门禁中复跑，exit 0，见 §2。）
- **`examples/diode_rectifier.cdsl` 的 DC 扫描逐点物理值未核对**：QA 只核对了端点与对数趋势
  （`docs/review-evidence/round2/cli-qa.md` §9.3），11 个扫描点没有逐点对照 Shockley 方程。
- **`rlc.cdsl` / `diode_rectifier.cdsl` 没有修复前的 CSV 对照**：修复前基线只录制了
  `pulse-fine` / `pulse-coarse` 两条，因此这两例只能报告修复后实测值，不声称差异幅度
  （`cli-qa.md` §11.8）。
- **规模性能**：没有数万节点级别的性能/内存测试。
- **退出码 2** 既没有返回点、也没有测试（§6）。

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
- **求解器时间轴非均匀**：端到端与适配层都显式断言存在相邻步长不等的点。这是**未给
  `output_interval` 时**的输出契约；给了 `output_interval` 后输出是等间隔网格，但**原始求解网格
  仍然非均匀**（`thevenin.rs` 不把 `output_interval` 传给引擎）。
- **`output_interval` 不得进入求解器**（任务 A 的新钉子）：
  `output_interval_regression.rs::output_interval_does_not_change_the_solved_trace` 让同一实验跑
  无 / 1 ns / 100 ns 三遍，断言原始网格都是 2015 点且 `tran.solver_step` / `solve_points` /
  `waveform_bound` / `max_step` 逐项相同，只有 `tran.output_interval` 元数据不同；
  `solver_metadata_describes_the_solve_not_the_output_request` 与
  `a_declared_ten_nanosecond_edge_is_delivered_on_the_raw_grid`（距 50 ns 最近的样本三份都是
  `v(vin) = 1.0 V`；旧契约下 coarse 为 0.5002375000000003 V）各自钉住一角；
  `circuit-session/tests/tran_output_interval.rs::measurements_are_computed_on_the_raw_grid` 钉住
  `avg`/`rms`/`max`/`min` 在两种间隔下**逐位相同**。
- **声明边沿不得被 print step 展宽**：
  `transient_reference_regression.rs::declared_rise_below_output_interval_is_not_widened` 用**旧契约
  参考**（`T_eff = max(rise, output_interval) = 500 ns`）作判别对照：该参考下 257/1024 点超限、
  最大误差 2.483260e-3 V，而按声明的 1 ps 参考 0 超限（max 7.644154e-7 V）。适配层若退回
  "`output_interval` → `Tran.step`"，这条会红。
- **能力错误不得静默回退**：`tran_option_validation.rs`（15 个）钉住 `max_step`/`output_interval`
  的 0/负/NaN/inf 在 DSL 层报 `E_VALUE` 且不产生计划；
  `output_interval_regression.rs::an_edge_that_cannot_be_honoured_is_refused_not_widened` 钉住
  声明边沿不可执行时的 `E_UNSUPPORTED` 与步数预算的 `E_LIMIT`（含消息中的步数）；
  `output_interval_regression.rs::the_step_budget_blames_the_waveform_not_the_users_max_step`
  钉住**归因**：纯 DC 源 + `max_step: 1.ns` + `stop: 1.s` 必须**不**报错（那是用户自己的预算），
  而 `rise: 1.ps` + `stop: 1.s` 必须报 `E_LIMIT` 且消息含 `declared source rise/fall/period` 与
  `declared waveform timing` 上下文（把守卫改回"只看步数"会让这条测试变红，已做判别力对照）。
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
