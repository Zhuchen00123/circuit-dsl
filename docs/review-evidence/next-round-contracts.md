# 下一轮契约差异清单（A14 · 只读分析）

- 角色：a14-contract-gap（只读契约差异分析，低优先级）
- 仓库：`F:\codexprojects\dsl000`；HEAD = `cb5d8a212f66922181580900a05fb3d42abe32f2`
- 快照时间：2026-09-18 19:42（本报告所有行号、哈希、mtime 都是这一刻的取值）
- 结论：**NEEDS_FIX**（差异确实存在；其中 README 分压示例一项在本轮被其他成员修复，见 §3）
- 本报告是唯一写入文件；除它以外本轮未创建/修改/删除仓库内任何文件，未 commit/push。
- 未执行 `cargo test` / `cargo fmt` / `cargo clippy`（只读约束；workspace 级命令按 team-board §3 只由 lead 执行）。本报告不把「未重跑」当成「已验证」。

---

## 1. 审查范围与版本

### 1.1 实际读取的文件（哈希为 SHA-256 前 16 位）

| 文件 | 哈希 | mtime |
|---|---|---|
| README.md | `D3669EE5285F7313` | 09-18 19:38:56（本轮被其他成员改写，见 §3） |
| RUST_CIRCUIT_DSL_PROMPT.md | `EE5D9248DCA4EC6B` | 09-18 17:19:10 |
| docs/language.md | `A0E0B32E1F7F9055` | 09-18 16:57:18 |
| docs/testing.md | `60C9EE4C14B9B487` | 09-18 17:02:02 |
| docs/architecture.md | `8344A0C50DAA5A44` | 09-18 16:59:50 |
| docs/prompt-review.md | `D65B50C2203D48FE` | 09-18 17:19:14 |
| docs/backend-evaluation.md | `0D275B340A6FF6A4` | 09-18 14:53:28 |
| examples/voltage_divider.cdsl | `005B65C54086630E` | 09-18 16:50:44 |
| examples/{rc_filter,rlc,diode_rectifier,parameter_sweep,ladder,two_stage}.cdsl | （全文读过，未逐个取哈希） | — |
| crates/circuit-dsl/src/elaborate.rs | `6E76B5F900676753` | 09-18 16:48:55 |
| crates/circuit-dsl/src/eval.rs | `958E19B0A9643C74` | 09-18 16:48:55 |
| crates/circuit-dsl/src/parser.rs | `E62AE862D9C8A5E7` | 09-18 16:52:34 |
| crates/circuit-results/src/expr.rs | `5351AA015E9EB2F4` | 09-18 14:35:11 |
| crates/circuit-core/src/plan.rs | `E562162A41A6B0F4` | 09-18 15:38:45 |
| crates/circuit-core/src/ir.rs | `267C3A2D88E286D1` | 09-18 15:12:45 |
| crates/circuit-session/src/execute.rs | `1E906B9E5A668799` | 09-18 16:48:55 |
| crates/circuit-cli/src/{main.rs,run.rs} | `6C1B1680156E9C91` / `440245E71E7344EA` | 16:44 / 16:48 |
| crates/circuit-backend/src/{thevenin.rs,sweep.rs} | `6337B9EB2A9917F5` / `38A3D6FAB39D73EE` | 15:38 / 15:15 |
| crates/circuit-backend/tests/adapter.rs | `6AE48B621274BA82` | 09-18 15:38:45 |
| crates/circuit-cli/tests/e2e.rs | `1A491B4499C8A9B4` | 09-18 15:49:51 |

执行体（runner）：
- `target/debug/cdsl.exe` = `904C9328CEFC07D4`，mtime 09-18 17:02:15。**全部 .rs 中最新的一个是 `crates/circuit-cli/src/repl.rs`（17:02:11）**，因此该 debug 二进制晚于当前源码，是本报告的权威执行体。
- `target/release/cdsl.exe` = `40CC4F9773697F88`，mtime 09-18 15:38:57，**早于** parser.rs(16:52)、tests/elaborate.rs(17:02)、repl.rs(17:02) 等源码，属陈旧产物。两个二进制在分压示例上输出一致（§3.2），但 README 快速开始段推荐的 release 产物是否等于当前源码，本报告**未验证**。

### 1.2 实际执行的命令（全部只读；结果文件只写系统 TEMP，未写仓库）

| # | 命令 | 结果摘要 |
|---|---|---|
| E1 | `target/release/cdsl.exe --version` | `cdsl 0.1.0`，exit 0 |
| E2 | `target/release/cdsl.exe run examples/voltage_divider.cdsl --experiment divider --out %TEMP%\a14_contract_gap_out --format both` | exit 0；stdout 3 行；写 `divider.op1.csv`、`divider.op1.json` |
| E3 | `target/debug/cdsl.exe run examples/voltage_divider.cdsl --experiment divider --out %TEMP%\a14_contract_gap_out_dbg --format csv` | exit 0；CSV 与 E2 相同 |
| E4 | `target/debug/cdsl.exe capabilities` | exit 0，输出与 README:167-177 引用块逐字一致 |
| E5 | `target/debug/cdsl.exe check examples/voltage_divider.cdsl` | exit 0；`circuit divider: 3 nodes (2 signal), 3 devices`；1 个 op 任务 |
| E6 | TEMP 中 8 个探针用例上的 `cdsl check` / `cdsl run`（见 §1.3） | case1/2/3/4/7/BOM exit 1；case5 check exit 0 而 run exit 1；case6 exit 0 |
| E7 | `git rev-parse HEAD` / `git status --porcelain` / `git diff -- README.md` | HEAD=`cb5d8a2…`；工作区 ` M README.md`、` M RUST_CIRCUIT_DSL_PROMPT.md`、4 个未跟踪项 |
| E8 | `git show HEAD:README.md \| Select-String "2.000 V"` | 命中第 45 行（HEAD 版本确为过时文本） |
| E9 | `git show HEAD:examples/voltage_divider.cdsl \| Select-String "3.000 V"` | 命中第 7 行 |
| E10 | `git show HEAD:crates/circuit-backend/tests/adapter.rs \| Select-String ac_phase_is_converted_from_radians_to_degrees` | 命中第 643 行（该测试属于已提交内容） |
| E11 | 对 `crates/**/*.rs` 统计 `^\s*#\[test\]` 与 `#[ignore]` | 合计 `#[test]` = 388，`#[ignore]` = 0；+1 doc-test = 389，与 lead 基线（team-board §0、docs/prompt-review.md:16）在**数量上**自洽（**未重跑 cargo test**） |

### 1.3 只读探针用例（写在 `%TEMP%\a14_probe_cases\`，不属仓库文件）

| 用例 | 内容要点 |
|---|---|
| case1_expr_measure.cdsl | `measure :gain, max: v(:out) / v(:in)` |
| case2_forward_ref.cdsl | `param :b, default: 2 * a` 写在 `param :a` 之前 |
| case3_self_ref.cdsl | `param :a, default: a + 1` |
| case4_ac_phase_arg.cdsl | `voltage_source :input, …, ac: 1.V, phase: 90` |
| case5_topo_param_sweep.cdsl | `param :taps` 控制 `for`/`node`/器件名，`dc param: :taps, from: 2, to: 4, step: 1` |
| case6_chain_ok.cdsl | 对照组：`a` 先声明、`b = 2 * a` 后声明（顺序依赖可工作） |
| case7_unknown_measure.cdsl | `measure :g, gain: v(:out)` |
| caseBOM.cdsl | case6 内容 + UTF-8 BOM（首字节 `239,187,191`） |

---

## 2. 能力差异表（目标契约 vs 当前实现）

> 契约出处均在 `RUST_CIRCUIT_DSL_PROMPT.md`（下表简写 P:行）。

| # | 能力项 | 目标契约出处 | 当前实现事实（文件:行） | 差异性质 | 下一轮建议动作与验收标准 |
|---|---|---|---|---|---|
| 1 | 参数依赖 DAG（前向引用、依赖链、拓扑序求值） | P:204「参数依赖可构成 DAG；循环依赖必须报错并显示依赖链」；P:303「展开层保留参数定义、依赖图」 | 参数在**声明顺序**上立即求值并写入扁平 `Scope.vars: HashMap`（`crates/circuit-dsl/src/elaborate.rs:191-219`、`:573-645`，尤其 `:621-631`）；查找失败即 `E_NAME`（`crates/circuit-dsl/src/eval.rs:137`）；前向引用被测试钉死为错误（`crates/circuit-dsl/tests/elaborate.rs:434-445`）；`E_PARAM_CYCLE` 只有错误码、无生产者（`crates/circuit-core/src/diagnostic.rs:59,92`；全仓 grep 仅此两处）；文档自述（`docs/language.md:255`、`README.md:225`） | **部分实现**（求值与覆盖链已实现；依赖图/拓扑序/环诊断缺失） | 收集全部 `param` 默认值表达式 → 建依赖图 → 拓扑序求值；环报 `E_PARAM_CYCLE` 并渲染依赖链。本报告实测：case2 → `error[E_NAME]: `a` is not declared`（exit 1）；case6（顺序链）→ exit 0；case3 → `E_NAME` 带 secondary span、**无**依赖链。验收：`cargo test -p circuit-dsl`（新增：后向声明可求值 + 环报 `E_PARAM_CYCLE` 且消息含链） |
| 2 | 拓扑参数沿依赖图传播 + 扫描前静态拒绝 | P:218-220「参数若影响条件、循环次数或连线即拓扑参数；判定必须沿参数依赖图传播；首期扫描拓扑参数时明确拒绝」 | `circuit-dsl` 内**不存在**静态标记（grep `topolog` 于该 crate 仅命中注释 `crates/circuit-dsl/src/elaborate.rs:136`）；`E_TOPO_PARAM` 只由后端扫描驱动在**第二个差异点**事后产生（`crates/circuit-backend/src/sweep.rs:227-249`，尤其 `:236`）；每点重新展开（`crates/circuit-session/src/execute.rs:180-190`、`crates/circuit-backend/src/sweep.rs:213-221`） | **部分实现**（运行期后验检查；非契约要求的“标记/传播”，且 `check` 期不可见） | 实测：case5 `check` **exit 0**（接受拓扑参数扫描），`run` **exit 1** → `error[E_TOPO_PARAM]: sweeping this parameter changes the circuit topology at 3`（note: node `mid3` appears only at this point）。建议：在展开期按依赖图标记拓扑参数，参数扫描命中即拒；验收：`cdsl check` 对 case5 形式 exit 1 + `cargo test -p circuit-dsl -p circuit-cli` |
| 3 | 结果表达式 DSL / CLI 入口（`v(:out)/v(:in)`、增益 dB、RMS） | P:250「结果表达式：`v(:out)`、电压比值、RMS 等」；P:354「增益 dB 仅接受无量纲比值，`20*log10(abs(ratio))`」；P:385「结果表达式在分析上下文内验证」；P:642「可进入…结果表达式接通」 | 求值器已存在且完整：`crates/circuit-results/src/expr.rs:1-262`（AST 含 `GainDb`、`Abs`、`Sqrt`、`Min/Max`、运算与复数提升），`eval` 在 `:269` 起；单元测试 23 个。**但用户路径未接**：`measure` 目标在语法上解析为任意表达式（`crates/circuit-dsl/src/parser.rs:1362`），展开期却只接受 `v()/i()`（`crates/circuit-dsl/src/elaborate.rs:2137-2149`）；`AnalysisPlan.measures` 的目标类型是 `Probe` 而非表达式（`crates/circuit-core/src/plan.rs:236-246`）；会话/CLI 只按探针名归约（`crates/circuit-session/src/execute.rs:21,336-355`）；`crates/circuit-cli/src/run.rs:10-15` 未 import 任何 expr 类型；文档自述（`docs/testing.md:277-279`、`docs/architecture.md:373-377`） | **部分实现**（库能力存在，DSL/CLI 入口缺失） | 实测：case1 → `error[E_TYPE]: a probe must be `v(:node)` or `i(:device)``（exit 1）；case7 → `error[E_UNSUPPORTED]: unknown measurement `gain`` + `available: max, min, avg, rms`（exit 1）。建议：AST→`circuit_results::Expr` 翻译 + 分析上下文校验（OP 无时间 RMS、dB 要求同量纲）+ CLI e2e 数值断言。验收：`cargo test --workspace` 且新增 e2e 断言 `v(:out)/v(:in)` 与 dB 数值 |
| 4 | 非零相位：DSL 语法与数值覆盖 | P:354-355「相位单位、主值范围以及是否展开相位必须明确」；P:604（基准用例相位 0）；team-board D3（AC 与 SIN 两条路径都要覆盖） | IR 有 `AcSpec.phase_rad` / `Waveform::Sin.phase_rad`（`crates/circuit-core/src/ir.rs:199-207,222-232`）；DSL 的 `ac:` 只收幅度、相位**硬编码 0.0**（`crates/circuit-dsl/src/elaborate.rs:812-819`，`:817`）；`sin(phase:)` 接受度并转弧度（`:1696-1739`，`:1724-1730`）；适配层统一 `to_degrees()`（`crates/circuit-backend/src/thevenin.rs:648-654` AC、`:682-690` sin）；**已存在非零 AC 相位的数值测试** `crates/circuit-backend/tests/adapter.rs:635-705`（`FRAC_PI_2`，断言 `re≈0`、`im≈0.5`，1e-9），且该测试在 HEAD 中已提交（E10 → 第 643 行）；sin 相位**无任何测试**；导出侧无相位列/展开相位选项（`crates/circuit-results/src/export.rs` 无 `phase` 命中，公开面见 `:59,156,175,338`），而 `crates/circuit-core/src/plan.rs:112-114` 注释声称“除非 exporter 显式要求展开”——**该选项不存在** | **部分实现 + 文档过时** | 实测：case4 → `error[E_ARGUMENT]: `voltage_source` has no argument `phase``（`= accepted: p, n, dc, ac, waveform`，exit 1）。文档过时两处：`docs/testing.md:268-270` 与 `docs/architecture.md:322` 仍写“没有数值测试 / 都产生 0 相位”，与 HEAD 中 adapter.rs:642 冲突。建议（本轮边界禁止新增 DSL 相位语法）：补 `sin(phase:)` 的 DSL→IR→引擎实虚部数值测试 + 按实况改上述两处文档 + 修正 `plan.rs:112-114` 注释或实现导出展开选项（择一，需 lead 决策）。验收：`cargo test -p circuit-dsl -p circuit-backend` |
| 5 | `save` 实验级语义（对所有分析生效；只接受在所有目标分析中均有效的探针） | P:379「experiment 顶层的 save 对其中所有分析生效，首期只接受在所有目标分析中均有效的探针；分析级 save 语法后续另行定义」 | 实验级语义**已实现**：单条 `save` 校验（`crates/circuit-dsl/src/elaborate.rs:1955-1967`，第二条 `save` 报 `E_DUPLICATE`）、解析后复制到全部任务（`:2059-2087`，重复探针 `E_DUPLICATE`）；`validate` 按 span 去重“每个探针只查一次”（`crates/circuit-backend/src/thevenin.rs:152-268`：器件/模型/分析/探针电流可用性，`:219-268` 明确拒绝电容/二极管/电流源电流）；**没有**“探针必须在该实验的每个分析里都有效”的逐分析校验；分析级 `save` 语言无语法 | **已实现（主干）+ 未验证（逐分析有效性）** | 建议：造一个“只在部分分析有效”的探针用例（如 `i(:r1)` 配 `op+ac+tran`，以及电容电流 `i(:c1)`），确认是 `validate` 期 `E_UNSUPPORTED` 还是运行期 `E_BACKEND`，据此补校验或写入文档。验收：`cargo test -p circuit-backend --test adapter` + 新用例断言错误码与“未产生结果” |
| 6 | README 分压示例与 `examples/voltage_divider.cdsl`、真实执行一致 | P:497「文档与实现一致，没有把 TODO 或模拟数据标为已实现」；P:544；P:459「README 和实际 CLI 同步」 | **HEAD 版不一致**：`git show HEAD:README.md` 第 45 行仍为 `# must give v(out) = 2.000 V and i(r1) = +1.000 mA.`，第 90 行 note 据此宣称“示例文件头部注释写的是 2 V / 1 mA、与实际参数不符”；而 `git show HEAD:examples/voltage_divider.cdsl` 第 7 行为 `3.000 V / +2.000 mA`（工作区同）。**工作区 README 已在 19:38:56 被其他成员改写**（team-board 中 README 属 lead），现 39-65 行与示例文件 1-27 行逐字一致（含末行 `end`），note 已改为“逐字一致 + 实测复现”，旧说法删除。真实 CLI 输出 3 V / +2 mA 与两者都一致 | **文档过时（HEAD）；工作区已修复** | 建议：把“README 引用块 == examples/voltage_divider.cdsl”钉成自动回归（读文件比较）。验收：`cargo test -p circuit-cli --test e2e` 新增用例通过 |
| 7 | （附加）UTF-8 BOM 源文件 | P:156「UTF-8 源文件」；P:497 | 词法器把 BOM 当非法字符；未在任何文档声明该限制（README「已知限制」无此条） | **未实现 / 未声明** | 实测：caseBOM（首字节 `239,187,191`）→ `error[E_SYNTAX]: unexpected character`，exit 1。建议：决定“跳过 BOM”或“写入已知限制 + 明确诊断文案”。验收：`cdsl check` 该文件 exit 0，或文档明示 + 测试断言该诊断 |
| 8 | （附加）文档与实现不同步（测试计数、接线描述、计划注释） | P:497；P:518 | `docs/testing.md:17` 称 `tests/elaborate.rs` “51 个”、`:19` 称 adapter.rs “16 个”、`:23` 称 e2e.rs “14 个”，而同文件 §3 表写 67/21/18，**实际静态 `#[test]` 计数为 67/21/18**（E11）；`docs/architecture.md:376` 称“`run.rs` 只 import 了 `measure_signal`”，实际 `crates/circuit-cli/src/run.rs:10-15` 只 import `{Format, RunRequest, write_datasets}`，`measure_signal` 在 `crates/circuit-session/src/execute.rs:21`；`docs/architecture.md:322` 见第 4 行；`crates/circuit-core/src/plan.rs:112-114` 见第 4 行 | **文档过时** | `docs/testing.md` 已被 team-board 分给 A10（依赖“实现冻结”），**避免重复劳动**；`docs/architecture.md` 本轮无人认领。验收：文档数字与 `#[test]` 计数一致；`cargo test` 计数复核由 lead 执行 |

---

## 3. README 分压示例是否不一致？—— 明确回答

### 3.1 调用形式（先读 CLI 源码）

- 子命令与参数：`crates/circuit-cli/src/main.rs:59-72` —— `cdsl run <FILE> [--experiment <NAME>] [--out <DIR>（默认 results）] [--format csv|json|both]`；主派发在 `:108-113`。
- 实验选择与写出：`crates/circuit-cli/src/run.rs:29-70`（`--experiment` 必须命中，否则 `E_NAME` + 列出真实实验名）、`:96-127`（写结果，且 `guard_output` 拒绝覆盖输入文件，见 `main.rs:142-156`）。
- 因此示例文件第 5 行注释 `cdsl run examples/voltage_divider.cdsl --experiment divider` 是合法调用形式（省略 `--out` 时写 `./results`）。

### 3.2 真实执行（E2 完整输出）

命令（工作目录 `F:\codexprojects\dsl000`，输出目录在系统 TEMP，避免写仓库）：

```powershell
target\release\cdsl.exe run examples/voltage_divider.cdsl --experiment divider --out $env:TEMP\a14_contract_gap_out --format both
```

stdout（无 stderr，exit 0）：

```text
experiment `divider` on circuit `divider` (backend thevenin 0.5.0)
  op1: scalar; signals: v(in), v(out), i(r1), i(v1)
  wrote C:\Users\15185\AppData\Local\Temp\a14_contract_gap_out\divider.op1.csv
  wrote C:\Users\15185\AppData\Local\Temp\a14_contract_gap_out\divider.op1.json
```

`divider.op1.csv` 全文：

```csv
v(in),v(out),i(r1),i(v1)
5,3,0.002,-0.002
```

`divider.op1.json` 的 signals（其余为 schema/axis/backend 元数据）：

```json
[{"name":"v(in)","unit":"V","values":[5.0]},
 {"name":"v(out)","unit":"V","values":[3.0]},
 {"name":"i(r1)","unit":"A","values":[0.002]},
 {"name":"i(v1)","unit":"A","values":[-0.002]}]
```

E3 用 `target/debug/cdsl.exe` 重跑，CSV 完全相同；E4 `capabilities` 与 README:167-177 逐字一致；E5 `check` exit 0。

### 3.3 结论（分版本回答，避免含糊）

1. **对 HEAD（`cb5d8a2`）而言：不一致，README 过时。**
   - `git show HEAD:README.md` 第 45 行 = `# must give v(out) = 2.000 V and i(r1) = +1.000 mA.`；第 90 行 note 进一步把该 2 V/1 mA 当成示例文件的实际内容并称其“与实际电路参数不符”。
   - `git show HEAD:examples/voltage_divider.cdsl` 第 7 行 = `# must give v(out) = 3.000 V and i(r1) = +2.000 mA.`；第 10-12 行解释的是 2 mA / -2 mA。
   - 真实执行（§3.2）= `v(out)=3 V`、`i(r1)=+0.002 A`、`i(v1)=-0.002 A`。
   - 即：README 引用的“示例文件内容”与示例文件本身、与真实输出三者不一致；README 自己的 CSV 块（`:84-86`）反而是对的。
2. **对当前工作区而言：已一致（但在本报告分析期间被其他成员改动）。**
   - `git status` 显示 ` M README.md`；`git diff -- README.md` 显示 45 行改为 `3.000 V / +2.000 mA`、原 note 被替换；新哈希 `D3669EE5285F7313`，mtime `2026-09-18 19:38:56`。
   - 我用行级比较复核：README 39-64 行与示例文件 1-26 行**逐字相同**，第 65 行 `end` 对应示例第 27 行，闭合围栏在第 66 行 —— 引用块完整且等于文件。
   - 该修复与本报告独立实测（§3.2）一致：3 V / +2 mA / -2 mA。
3. 因此，**README 分压示例确实是真实存在过的文档缺陷（HEAD 版），但它已在本轮被 lead 修复**；下一轮该做的是“把它钉成回归测试”，而不是再改一遍文案。

---

## 4. 下一轮任务清单（每项一行：写集 / 依赖 / 验收命令）

> 写集为仓库相对路径；依赖列只列**必须在前完成**的项。第 2 项与 team-board 已分配的文件重叠，已注明避免重复劳动。

1. **README 示例块回归** —— 写集：`crates/circuit-cli/tests/e2e.rs`；依赖：无；验收：`cargo test -p circuit-cli --test e2e`（新用例读 `README.md` 的最小示例围栏与 `examples/voltage_divider.cdsl` 逐行比较，并断言 run 输出 `5,3,0.002,-0.002`）。
2. **docs/testing.md 计数与相位条目校正** —— 写集：`docs/testing.md`（**本轮已归 A10，等其释放**）；依赖：无；验收：文档中 67/21/18 与实际 `#[test]` 计数一致；§7 相位条目按 `adapter.rs:642` 实况改写；`cargo test -p circuit-backend --test adapter` 绿。
3. **docs/architecture.md 与 plan.rs 注释校正** —— 写集：`docs/architecture.md`、`crates/circuit-core/src/plan.rs`（仅注释）；依赖：无；验收：`:322` 相位行、`:373-377` expr 行与实现一致；`plan.rs:112-114` 的“exporter 可展开相位”改为事实描述（或删）；`cargo test -p circuit-core` 绿。
4. **参数依赖 DAG 求值** —— 写集：`crates/circuit-dsl/src/elaborate.rs`、`crates/circuit-dsl/src/eval.rs`（如需作用域接口）、`crates/circuit-dsl/tests/elaborate.rs`、`docs/language.md`（§4.2/§10）；依赖：无；验收：`cargo test -p circuit-dsl`，新用例覆盖“后声明参数可被前向引用求值”“默认值引用实例覆盖值”“覆盖链顺序不变”。
5. **循环依赖诊断 E_PARAM_CYCLE + 依赖链** —— 写集：同第 4 项（同批修改同一文件集）、`crates/circuit-core/src/diagnostic.rs`（如需）；依赖：4；验收：`cargo test -p circuit-dsl`，新用例断言错误码与消息中的完整依赖链（如 `a -> b -> a`）。
6. **拓扑参数静态标记与 check 期拒绝** —— 写集：`crates/circuit-dsl/src/elaborate.rs`、`crates/circuit-dsl/tests/elaborate.rs`、`crates/circuit-cli/tests/e2e.rs`、`docs/language.md`（§4.2/§5.1）；依赖：4（依赖图）；验收：`cdsl check` 对 case5 形式（`dc param:` 扫拓扑参数）exit 1 + `E_TOPO_PARAM`，`cargo test -p circuit-dsl -p circuit-cli` 绿，且 `sweep.rs` 既有点间拓扑比较测试不回归。
7. **结果表达式接通 DSL/CLI** —— 写集：`crates/circuit-dsl/src/ast.rs`、`parser.rs`、`elaborate.rs`、`crates/circuit-core/src/plan.rs`（measure 目标改表达式/保留探针双形态）、`crates/circuit-session/src/execute.rs`、`crates/circuit-results/src/{expr.rs,measure.rs}`、`crates/circuit-dsl/tests/elaborate.rs`、`crates/circuit-cli/tests/e2e.rs`、`docs/language.md`（§7）、`README.md`（“尚未实现”清单）；依赖：无；验收：`cargo test --workspace`；新 e2e 断言 `measure :g, max: v(:out) / v(:in)` 数值与 `20*log10(abs(...))` dB 数值；OP 上 `avg/rms`、dB 量纲错误的负例。
8. **非零相位数值覆盖（不改语法）** —— 写集：`crates/circuit-dsl/tests/elaborate.rs`（`sin(phase:)` 的 IR 断言）与/或新建 `crates/circuit-backend/tests/phase_regression.rs`（**本轮 A09 已占用该文件名，下一轮先确认归属**）；依赖：无；验收：`cargo test -p circuit-dsl -p circuit-backend`；断言非零 `sin` 相位的 IR 弧度值与引擎侧 `phi` 度值，并在真实瞬态输出上核对相位（实/虚或过零点），而非只看幅度。
9. **save 逐分析有效性校验** —— 写集：`crates/circuit-backend/src/thevenin.rs`、`crates/circuit-backend/tests/adapter.rs`、`docs/language.md`（§5.2）；依赖：无；验收：`cargo test -p circuit-backend --test adapter`；新用例断言“探针在某目标分析不可得”在 `validate` 期以确定错误码失败且不产生结果。
10. **UTF-8 BOM 处置** —— 写集：`crates/circuit-dsl/src/lexer.rs`、`crates/circuit-dsl/tests/elaborate.rs`、`docs/language.md`（§1）、`README.md`（已知限制，若选择拒绝）；依赖：无；验收：`cdsl check` 带 BOM 的 `.cdsl` exit 0（接受方案）或文档明示 + 词法测试断言明确诊断（拒绝方案）。

---

## 5. 未验证项与限制（不得当作结论）

1. **未重跑 cargo test**：389 passed / 0 failed 是 lead 的基线（team-board §0、docs/prompt-review.md:16），我只做了静态 `#[test]` 计数（388 + 1 doc-test）作为数量旁证；本轮未独立复核任何测试结果。
2. **未运行 cargo fmt/clippy**，未验证格式与 lint 现状。
3. **release 二进制可能陈旧**：`target/release/cdsl.exe` mtime 15:38:57 早于 16:44-17:02 的多个源码文件；我用 debug 二进制（17:02:15，晚于所有 .rs）作为权威执行体。README 快速开始段推荐的 release 路径与当前源码是否等价，**未验证**。
4. **`_probe` 与两个 P1 证据修正未做**：浮空节点用例（`_probe/src/bin/robustness.rs`）与 RC 瞬态误差归因（`_probe/src/main.rs`）按 team-board 属 A02/A03/A05/A06 的写域，本报告未读、未跑 `_probe`，也未验证 `docs/backend-evaluation.md` §4.6/§5 的内容。
5. **非零相位数值正确性未独立验证**：我只确认 `adapter.rs:642` 测试存在且属于 HEAD；未运行它，也未核对 `sin(phase:)` 链路（无测试可参照）。
6. **save 逐分析有效性未构造反例**：无法断言当前实现是“正确拒绝”还是“运行期才失败”，表中标记为未验证。
7. **文档存在并发修改**：README.md 在 19:38:56 被改写；`docs/testing.md` 被 team-board 分给 A10 但此刻仍是旧内容；所有文档结论仅对 §1.1 的哈希负责。
8. **未验证 `check --json` 输出**、未验证 REPL 路径、未验证 AC 相位导出/展开选项（“不存在”的证明只到 `grep phase` 于 `export.rs` 无命中 + 公开函数清单）。
9. **未评估参数 DAG 改动的兼容性影响**：改成拓扑序求值会改变“前向引用报 `E_NAME`”这一被测试钉死的行为（`tests/elaborate.rs:434-445`）与 `docs/language.md:255`、`README.md:225` 的表述，需 lead 明确决策“实现 DAG”还是“修改契约”。

---

## 6. 结论

**NEEDS_FIX。**

- 目标契约与当前实现存在**真实差异**：参数 DAG/依赖链（P:204、P:303）、拓扑参数传播与静态拒绝（P:218-220）、结果表达式 DSL/CLI 入口（P:250、P:354、P:642）、非零相位的 DSL 语法与 sin 路径覆盖（P:354-355）。
- **文档过时**集中在 `docs/testing.md`（计数、相位条目）、`docs/architecture.md:322,376`、`crates/circuit-core/src/plan.rs:112-114`；README 分压示例属 HEAD 版过时、工作区已被 lead 修复（§3）。
- 已实现主干但需补强验证的是 `save` 实验级语义与 `capabilities/check/run` 主路径（E2-E5 均实测通过）。
- 无阻塞项：本报告的每一条差异都能定位到具体文件行，并附可复现命令；未验证项已在 §5 单列，不冒充结论。
