# Round 4 phase-B QA 独立验收（task-12）

Owner：`qa-worker`。写集：新增 `crates/circuit-dsl/tests/r4b_param_graph.rs`、
`crates/circuit-session/tests/r4b_session_dag.rs`、`crates/circuit-cli/tests/r4b_topo_sweep.rs`、
`crates/circuit-cli/tests/r4b_regressions.rs`；仅改写 `crates/circuit-dsl/tests/elaborate.rs` 中点名的两条
旧规则测试；本文件。没有修改任何生产代码（`crates/*/src/**`），没有动 phase-A 已交付的 `r4_*.rs`。

依据：`docs/round3-review-and-round4-plan.md` §5.3、`docs/review-evidence/round4/design-contract.md` §4。

## 1. 结论

| 门禁 | 结果 |
|---|---|
| phase-B 5 个测试目标 | **PASS**，92 passed / 0 failed（`target/round4/qa-b/phase-b-final.log`，`OVERALL_EXIT=0`；T11 完成后已复跑，见 §6） |
| 先 RED 后 GREEN | **PASS**：T11 落地前 5 个目标共 **20 failed**（`phase-b-red.log`）；落地后 0 failed |
| phase-A 回归（scoped 重跑） | **PASS**，42 passed / 0 failed（`target/round4/qa-b/phase-a-scoped.log`） |
| 新增/改写目标的 clippy（`-D warnings`） | **PASS**，3 组 exit 0（`clippy-scoped.log`） |
| 新增/改写文件 rustfmt `--check` | **PASS**，5 个文件 exit 0 |
| 示例 `examples/parameter_sweep.cdsl` | **PASS**，check exit 0、run exit 0、8 点数值与手算一致 |

## 2. 命令与观察结果

最终证据全部用 `target/round4/qa-b/cargo2.ps1` 执行（该封装把整条命令作为一个字符串传给 cargo，
`-p` 不会被 PowerShell 吃掉；用 `--no-run` 验证过只列出目标包的 6 个 target）。

### 2.1 RED 基线（T11 落地前，`phase-b-red.log`）

```text
cargo test -p circuit-dsl     --test r4b_param_graph   -> FAILED. 4 passed; 9 failed
cargo test -p circuit-dsl     --test elaborate         -> FAILED. 65 passed; 2 failed
cargo test -p circuit-session --test r4b_session_dag   -> FAILED. 0 passed; 3 failed
cargo test -p circuit-cli     --test r4b_topo_sweep    -> FAILED. 2 passed; 4 failed
cargo test -p circuit-cli     --test r4b_regressions   -> FAILED. 1 passed; 2 failed
OVERALL_EXIT=1
```

RED 的机制与契约一一对应：

- 前向引用当时是 `error[E_NAME]: `a` is not declared`（声明顺序决定可见性）；
- 自引用/多节点环当时也是 `E_NAME`，没有 `E_PARAM_CYCLE`；
- `cdsl check` 对拓扑扫描 exit 0，只有运行期拓扑比较才拦得住（诊断只有 1 个 span，没有解释路径）；
- 覆盖参数时默认表达式仍被求值（`param :r, default: nope` + 实验覆盖仍报 E_NAME）；
- 走 DAG 的参数化瞬态/导出用例因前向引用而无法运行。

### 2.2 GREEN（`phase-b-final.log`）

```text
cargo test -p circuit-dsl     --test r4b_param_graph   -> ok. 13 passed; 0 failed
cargo test -p circuit-dsl     --test elaborate         -> ok. 67 passed; 0 failed
cargo test -p circuit-session --test r4b_session_dag   -> ok.  3 passed; 0 failed
cargo test -p circuit-cli     --test r4b_topo_sweep    -> ok.  6 passed; 0 failed
cargo test -p circuit-cli     --test r4b_regressions   -> ok.  3 passed; 0 failed
OVERALL_EXIT=0
```

### 2.3 静态检查

```text
cargo clippy -p circuit-dsl     --test r4b_param_graph --test elaborate -- -D warnings        -> exit 0
cargo clippy -p circuit-session --test r4b_session_dag -- -D warnings                        -> exit 0
cargo clippy -p circuit-cli     --test r4b_topo_sweep --test r4b_regressions -- -D warnings  -> exit 0
rustfmt --edition 2024 --check  （4 个新文件 + elaborate.rs）                                 -> exit 0
```

## 3. 红线矩阵（plan §5.3）

### 3.1 前向引用、多层链、菱形图（含手算数值）

| 场景 | 独立数值依据 | 观察 | 判定 |
|---|---|---|---|
| 前向引用 | `b = 2 * a`，a = 1 kohm → 2000 ohm | `r4b_param_graph::a_forward_reference_resolves_to_the_declared_value` 通过；`elaborate.rs::a_forward_parameter_reference_is_resolved` 通过 | PASS |
| 三层链 | c = a + b，a = 3、b = 2a = 6 → 9 kohm | `a_multi_level_chain_resolves_in_dependency_order` | PASS |
| 菱形图 | base = 5，left = 2base = 10，right = 3base = 15，top = 25 kohm；声明顺序反转后仍为 25 kohm | `a_diamond_evaluates_once_and_independently_of_declaration_order` | PASS |
| 长链（20 层，全部前向） | p_k = 2·p_(k-1)、p_0 = 1 ohm → p_20 = 2^20 = 1048576 ohm | `a_long_forward_chain_is_fully_resolved` | PASS |
| 重复运行确定性 | 声明顺序不同的同一菱形图给出同一数值 | 同上（同一测试内对照） | PASS |

### 3.2 未知名称 vs 自环 vs 多节点环（诊断与 span）

| 场景 | 观察 | 判定 |
|---|---|---|
| 自环 `param :a, default: a` | `E_PARAM_CYCLE`，且**不再**出现 `[E_NAME]`；定位到声明行 `test.cdsl:2` | PASS |
| 两节点环 a -> b -> a | 码为 `E_PARAM_CYCLE`、无 `[E_NAME]`、消息含闭合路径（`a -> b -> a` 或 `b -> a -> b`）、定位两条声明（`:2`、`:3`） | PASS |
| 三节点环 a -> c -> b -> a | 三个名字都出现，三条声明都被定位（`:2`、`:3`、`:4`） | PASS |
| 未声明名称 `default: nope` | 仍 `E_NAME` + "not declared"，且不含 `E_PARAM_CYCLE` | PASS |
| `param :a` 无默认值且无覆盖 | 仍 `E_NAME`，不是环 | PASS |

### 3.3 嵌套实例同名参数互不污染 + 覆盖后重算

| 场景 | 独立数值依据 | 观察 | 判定 |
|---|---|---|---|
| 两个 leaf 实例的 `r` | s1.r 由父 `rbase` 经 `params:` 连线 = 1 kohm；s2.r 自带 3 kohm | `same_named_parameters_of_nested_instances_do_not_pollute_each_other`：r0 = 1000、s1.rl = 1000、s2.rl = 3000 | PASS |
| 覆盖 `rbase = 2 kohm` 后重算 | r0 = 2000、s1.rl = 2000（连线跟着走）、s2.rl 仍 3000 | `an_override_recomputes_every_dependent_parameter_across_scopes` | PASS |

### 3.4 REPL：改基参数后更新、失败不污染

| 场景 | 独立数值依据 | 观察 | 判定 |
|---|---|---|---|
| `r -> r_doubled = 2r -> r_eff = 2·r_doubled = 4r`，v(out) = 6·r_eff/(1k + r_eff) | r=1k → 4.8 V；r=2k → 6·8/9 = 5.33333 V；r=3k → 6·12/13 = 5.53846 V；回到默认 → 4.8 V | `an_override_recomputes_every_dependent_parameter`：四次 `:run` 文本全部命中 | PASS |
| 失败覆盖（`r=0.ohm`，零值电阻在展开期就是 `E_VALUE`） | 失败前后同一 `:run e` 的文本逐字符相同；之后的成功覆盖仍得 5.33333 V | `a_failed_override_does_not_pollute_the_next_run` | PASS |
| 会话拒绝拓扑扫描 | 定义期（`:load` → compile）即 `E_TOPO_PARAM`，含中间参数 width 与路径 `n -> width`；会话里没有留下实验，`:run` 报 `E_NAME`；对照：合法数值扫描仍能 load 且 run 出 3 个扫描点 | `the_session_refuses_a_topology_sweep_before_it_can_run` | PASS |

### 3.5 拓扑扫描：直接/间接被 check 拒绝，普通数值扫描可用

| 场景 | 观察 | 判定 |
|---|---|---|
| 直接：`for k in 1..n` | `cdsl check` exit 1，`[E_TOPO_PARAM]`，>= 2 个 `-->` 位置，且定位 `direct.cdsl:2`（声明）与 `direct.cdsl:5`（for 使用点）；消息说明"只改数值的扫描仍允许" | PASS |
| 间接：n -> width -> for | exit 1，定位 `:2`、`:3`、`:6`，消息含中间参数 `width` | PASS |
| `if flag > 0` 条件 | exit 1，定位 `:2` 与 `:6` | PASS |
| `cdsl run` 同一程序 | exit 1、无 panic、stdout 无 `wrote`、输出目录为空（拒绝发生在任何求解与写盘之前） | PASS |
| 普通数值扫描 | check exit 0、run exit 0；3 点 v(out) = 3·rf/(1k+rf) = 1.5 / 2.0 / 2.25 V | PASS |
| 同名实例参数不误报 | 顶层 `n` 只用于数值，实例内部自己的 `n` 驱动其 for 而不被连线 → check exit 0、run exit 0；R(stage) = 1k‖1k = 500 ohm 固定，v(out) = 500/(n·1000+500) = 1/3、0.2、1/7 | PASS |

### 3.6 扫描点数值与独立手算一致

- `target/round4/qa-b/example-sweep/`：`examples/parameter_sweep.cdsl`，check exit 0、run exit 0；
  8 行 `v(out)` 与手算 `3·1500/(r+1500)` 逐点一致（r=500 → 2.25，r=1000 → 1.8，r=1500 → 1.5，r=4000 → 0.818181…）。
- `r4b_topo_sweep::a_plain_value_sweep_is_still_accepted_and_exact` 与
  `a_same_named_instance_parameter_is_not_implicated` 覆盖另外两组扫描点（上述公式，误差 < 1e-9）。

### 3.7 第 2/3 轮回归

| 场景 | 独立数值依据 | 观察 | 判定 |
|---|---|---|---|
| 瞬态网格 + 原始网格测量 | R=1k、C=100nF → tau = 100 us；`output_interval: 250.us` 网格为 0、250us、…+2 ms 末点；v(t)=1-exp(-t/tau)（插值误差界 ~1.25e-7 V，取 1e-3 V）；avg = 1-(1/20)(1-e^-20) = 0.95 | `a_parameterized_transient_keeps_its_grid_and_raw_measures`：网格点与末点精确、曲线吻合、有无 `output_interval` 的 `measure vavg` 行逐字符相同且含 0.95 | PASS |
| 分析绑定 | 实验把 c 覆盖为 10 nF → fc = 1/(2π·1k·10n) = 15915.494309189535 Hz，x = wRC = 1 → |H| = 1/√2；op 下电容隔直 → v(vout) = 1 V | `an_analysis_binding_still_picks_the_bound_analysis`：dc_gain = 1、ac_mag 每点 = 1/√(1+x²)、角点 x=1 | PASS |
| 导出集合 | r_lo = 3·r_hi = 1500 ohm，v(out) = 3.75 V，ratio = 5/3.75 = 4/3 | `the_export_set_still_follows_save_with_parameterized_signals`：表头恰好 2 列（v(out)、ratio），无 v(in) 列 | PASS |
| 参数化参数链不改变物理 | c -> tau_c = 2c -> c_eff = tau_c/2 = c | 同上瞬态用例（曲线只有 C=100 nF 才吻合） | PASS |
| phase-A 全套 | — | `target/round4/qa-b/phase-a-scoped.log`：42 passed / 0 failed | PASS |
| 运行期拓扑防线仍在 | — | 会话包内 `sweep::tests` 的 4 个拓扑用例仍通过（`run-session-crate.log`） | PASS |

## 4. 两条旧规则测试的改写

`crates/circuit-dsl/tests/elaborate.rs` 只动了点名的两条（其余 65 条未改动，仍全绿）：

| 旧 | 新 | 断言 |
|---|---|---|
| `a_self_referential_parameter_is_rejected`（E_NAME） | `a_self_referential_parameter_is_a_cycle` | `E_PARAM_CYCLE`，且不含 `[E_NAME]` |
| `a_forward_parameter_reference_is_rejected`（E_NAME） | `a_forward_parameter_reference_is_resolved` | `b = 2·1 kohm = 2000 ohm`，量纲 RESISTANCE |

改名原因：旧名字断言的行为已按契约 §4.4 反转，保留 "is_rejected" 会自相矛盾。

**task-13（docs worker）需要同步**：

- `docs/testing.md:323` 的 "未声明的参数名" 行仍引用旧测试名 `a_forward_parameter_reference_is_rejected`；
- `docs/architecture.md:263-264` 同样引用两条旧测试名；
- `crates/circuit-dsl/tests/elaborate.rs:400-403` 的文档注释仍写着 "按声明顺序求值、自引用是未声明名称" ——
  该注释属于第三条测试 `parameters_are_resolved_in_declaration_order`，按 lead 的指令（该文件只改点名的两条）
  我没有改动，请 lead 决定是否由后续任务清理。

## 5. 证据文件

| 文件 | 内容 |
|---|---|
| `target/round4/qa-b/phase-b-red.log` | T11 落地前的 RED（20 failed） |
| `target/round4/qa-b/phase-b-final.log` | T11 落地后的 5 目标最终结果（92 passed / 0 failed） |
| `target/round4/qa-b/phase-a-scoped.log` | phase-A 42 项 scoped 重跑 |
| `target/round4/qa-b/clippy-scoped.log` | 三组 scoped clippy `-D warnings` |
| `target/round4/qa-b/run-session-crate.log` | `cargo test -p circuit-session`（见 §7 命令纪律说明） |
| `target/round4/qa-b/example-sweep/` | `examples/parameter_sweep.cdsl` 的 8 点 CSV |
| `target/round4/qa-b/*.ps1` | 复现脚本（`cargo2.ps1` 为保 `-p` 的封装） |

## 6. 验证基线（SHA256 前 16 位）

T11 已 `completed`，下面指纹是 T11 完成后的内容；§2.2/§2.3/§3.7 的全部命令都在该版本上复跑过
（phase-B 92 passed / 0 failed，phase-A 42 passed / 0 failed，clippy 三组 exit 0）。若之后有人再改
`crates/circuit-dsl/src/**` 或 `crates/circuit-cli/src/**`，需按 §2.2 重新执行。

| 文件 | SHA256-16 |
|---|---|
| crates/circuit-dsl/src/param_graph.rs | AF450D6C6E40431D |
| crates/circuit-dsl/src/elaborate.rs | BC5EC3ED5B95C021 |
| crates/circuit-dsl/src/lib.rs | B40967F8EA2780B2 |
| crates/circuit-dsl/tests/elaborate.rs | 83CD87986143E413 |
| crates/circuit-dsl/tests/r4b_param_graph.rs | BEBB63BF2776EF7A |
| crates/circuit-session/tests/r4b_session_dag.rs | A163338EAD8D066D |
| crates/circuit-cli/tests/r4b_topo_sweep.rs | BD1C6E32C345E237 |
| crates/circuit-cli/tests/r4b_regressions.rs | 2FAF6F0C4E3608FD |

## 7. 遗留风险与备注

1. **T11 已完成**：GREEN 在 T11 `completed` 后的版本上复跑（§6 指纹）；T11 完成前后
   `elaborate.rs` 有过一次改动，复跑结果一致，无需进一步处置。
2. **命令纪律偏差（如实披露；结论不受影响）**：phase-A 与 phase-B 早期使用的封装
   `target/round4/qa/cargo.ps1` 声明为 `param([string]$Out, [string[]]$CargoArgs)`，PowerShell 绑定会把
   `-p` 和它的值两个 token 一起吞掉：`target/round4/qa-b/arg-echo.ps1` 实测
   `arg-echo.ps1 x.log test -p circuit-session --no-run` 只收到 `test` 与 `--no-run`。
   **因此实际执行的命令与书面命令不一致，逐条说明：**

   | 书面命令（phase-A/B 早期） | 实际执行 | 影响 |
   |---|---|---|
   | `cargo test -p <crate> --test <name>`（phase-A 全部、phase-B RED） | `cargo test --test <name>` = 全工作区、只跑该名字的 target | 该名字只存在于目标 crate，日志里也只有一条 `Running tests\<name>.rs` + 一条 `test result`，**执行的测试集合与预期相同** |
   | `cargo check -p circuit-results`（phase-A 一次编译探针） | `cargo check`（工作区检查） | 只用于判断 expr.rs 当时能否编译，结论不受影响 |
   | `cargo test -p circuit-session`（phase-B 一次，无 `--test`） | `cargo test` = **整个工作区测试套件**（32 个 target，exit 0，全部通过，日志 `target/round4/qa-b/run-session-crate.log`） | 这条确实超出"只跑局部命令"的纪律，如实记录；它是只读测试运行，未改动任何文件，也未与其它 worker 的写集冲突 |
   | `cargo build --release -p circuit-cli`（phase-A 一次） | `cargo build --release`（工作区 release 构建） | 随后 `cargo test --release -p circuit-cli --test r4_repro_cli` 同样只跑该 target（日志只有一条 `Running tests\r4_repro_cli.rs`），R4-02 release 门禁结论不受影响 |
   | `cargo clippy -p <crate> --test <name> -- -D warnings`（phase-A/B 早期） | `cargo clippy --test <name> -- -D warnings` | `--test <name>` 仍把 lint 目标限定为该名字的测试 target（依赖包只是被编译）；无论是否顺带检查了别的 target，三组都 exit 0，属"不少于预期"，不构成漏检 |

   **后续如何验证作用域**：改用 `target/round4/qa-b/cargo2.ps1`（把整条 cargo 命令作为**单个字符串**
   传入再拆词，`-p` 不经过 PowerShell 参数绑定），并用
   `cargo2.ps1 … 'test -p circuit-session --no-run'` 的 Executable 列表确认只剩 circuit-session 的 6 个
   target（`target/round4/qa-b/arg-probe3.log`）；同法确认 `-p circuit-results --test r4_expr_policy` 只跑 1 个
   target（`arg-probe.log`）。

   **对结论的影响**：§2–§6 的最终数字全部来自 cargo2.ps1 的 scoped 运行（每个日志都只有目标 target 的
   `Running`/`test result` 行），phase-A 42 项与 phase-B 92 项均已在该封装下重跑；被放大的那次工作区
   全套运行是唯一一次"超范围"，它 exit 0 全绿，但没有被当作任何 PASS/FAIL 或数值结论的依据。
3. **会话拒绝点**：拓扑扫描在 `:load`/`:define`（即 `compile`）即被拒绝，这是契约 §4.6 的规定；
   运行期防线没有被移除，仍由 `circuit-backend` 的拓扑比较测试覆盖。
4. `dc param:` 扫描的 CSV 坐标列名是 `parameter`（不是参数名），本文件与测试按此断言。
5. 未运行 `cargo fmt --all`、`cargo clippy --workspace`、`cargo test --workspace`（除 §7.2 中
   误由封装放大的那一次）；只对写集内文件做了 `rustfmt --check`/有界格式化。
