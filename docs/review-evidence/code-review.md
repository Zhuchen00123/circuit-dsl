# Wave-2 代码与 diff 审核（a12-code-review，只读）

日期：本轮 wave 2；审查者：a12-code-review；模式：严格只读（本文件是本代理唯一写入）。

## 0. 结论

**NEEDS_FIX（仅 P2 流程 + P3 文档；代码与测试本身 PASS）**

- **代码/测试层面：PASS**。本轮 10 个被修改文件 + 3 个新增测试文件全部落在被指派的写集内，**无越界**；未发现被削弱的断言（对比 HEAD 逐处验证，`_probe` 的 a–e 检查被**加强**而非削弱）；未发现被掩盖的失败或静默跳过；三条反事实破坏实验全部让对应测试以 **exit 101 失败**（有鉴别力）。
- **P2（流程/可审计性）**：冻结（20:00:19 本地）之后、且在我审查期间，仍有 3 个**未列入 freeze-manifest** 的追踪文件被写入（20:05–20:07）：`examples/rc_filter.cdsl`、`docs/backend-evaluation.md`、`docs/testing.md`。manifest 声明「任何写入都会使本清单失效」，但它的哈希集只覆盖 16 个文件，审查者仅凭 manifest 无法发现这三处漂移。
- **P3（文档不实）**：`docs/architecture.md:322` 的括号理由「DSL 无相位语法，转换只在 IR 直构路径被走到」对 SIN 波形不成立——DSL 支持 `sin(..., phase:)`（`crates/circuit-dsl/src/elaborate.rs:1700-1742`，:1731 做 `to_radians()`；`docs/language.md:201` 有文档）。只有 **AC 源相位**没有语法（`elaborate.rs:816-822` 硬编码 0）。由此还暴露一个真实覆盖缺口：DSL `sin(phase:)` 的 deg→rad 转换在任何测试里都没走到。
- **P4（自述不精确，不影响结论）**：freeze-manifest 的计数措辞「18 个 test binary + 1 个 doc-test」与我实测的 19 个 test target 口径不同（总数 407 一致）——该措辞已在 **20:11:02 的 manifest 重写**中改为「14 个 test binary + 5 个 doc-test 目标，共 19 条 `test result:` 行」，故 F3 视为已更正（但该重写本身未更新冻结时间戳，见 F1）；`examples/rc_filter.cdsl:11` 的 `thevenin.rs:772-775` 行号指向不准（实际 762-773）。
- 16/16 冻结哈希在审查期间三次校验一致（开始 / 20:08 / 20:11:37）；我未修改仓库任何追踪文件，未 commit/push。

## 1. 审查范围与版本

- 仓库：F:/codexprojects/dsl000，`git rev-parse HEAD` = **cb5d8a212f66922181580900a05fb3d42abe32f2**。
- 冻结清单：`docs/review-evidence/freeze-manifest.md`（冻结时间 2026-09-18T12:00:19.824Z，本地 20:00:19），16 个文件哈希。
- **冻结校验**：我在审查开始（约 20:0x）与结束（20:08）各跑一次 `Get-FileHash` 逐项比对 → **checked=16 mismatches=0**（两次）。
- **实测改动集合**（`git status --porcelain`，审查结束时）：
  `M README.md / M RUST_CIRCUIT_DSL_PROMPT.md / M _probe/src/bin/robustness.rs / M _probe/src/main.rs / M crates/circuit-backend/src/thevenin.rs / M crates/circuit-cli/tests/e2e.rs / M crates/circuit-core/src/connectivity.rs / M crates/circuit-dsl/src/elaborate.rs / M docs/architecture.md / M docs/backend-evaluation.md / M docs/language.md / M docs/testing.md / M examples/rc_filter.cdsl`；
  新增（??）：`crates/circuit-backend/tests/phase_regression.rs / crates/circuit-backend/tests/transient_reference_regression.rs / crates/circuit-dsl/tests/reference_path_regression.rs`；另有本轮之前就存在的 `?? AGENT_TEAM_EXECUTION_PROMPT.md / ?? agent-team-switch.md / ?? docs/prompt-review.md / ?? docs/review-evidence/`。
- 文件 mtime（本地）：README 19:59:04；RUST_CIRCUIT_DSL_PROMPT 17:19:10（**本轮之前的用户改动**）；robustness 19:49:54；_probe/main 19:58:55；thevenin.rs 19:51:37；e2e.rs 19:45:22；connectivity.rs 19:51:37；elaborate.rs 19:51:37；architecture.md 19:45:36；language.md 19:45:36；三个新测试 19:50–19:53；**后加三个：rc_filter.cdsl 20:05:11、backend-evaluation.md 20:06:51、testing.md 20:07:43**。

## 2. 实际执行的命令与真实输出摘要

| # | 命令（均在仓库内只读运行，除注明外） | 真实结果 |
|---|---|---|
| 1 | `git status --porcelain` / `git diff --stat HEAD` / `git diff HEAD -U0 -- _probe/src/main.rs` | 10 改 + 3 新增；各文件逐处 diff 见 §3–§5 |
| 2 | 16 项 `Get-FileHash -Algorithm SHA256` 与 manifest 比对（两次） | checked=16 mismatches=0（开始、结束各一次） |
| 3 | `cargo fmt --all -- --check` | `FMT_ALL_EXIT=0` |
| 4 | `cargo fmt --manifest-path _probe/Cargo.toml -- --check` | `FMT_PROBE_EXIT=0` |
| 5 | `cargo clippy --workspace --all-targets -- -D warnings`（仓库内，命中缓存） | `CLIPPY_EXIT=0` |
| 6 | 同上命令在**仓库外副本**（独立 target 目录，clippy 全新检查） | `COPY_CLIPPY_EXIT=0`，无 warning/error 行 |
| 7 | 仓库外副本 `cargo test --offline --workspace` | `WORKSPACE_TEST_EXIT=0`，`TOTAL passed=407 failed=0 ignored=0` |
| 8 | 仓库外副本 `cargo run --offline --manifest-path _probe/Cargo.toml --bin probe` | `PROBE_EXIT=0`，6/6 case PASS，`RESULT: ALL ACCEPTANCE CASES PASSED` |
| 9 | 仓库外副本 `... --bin robustness` | `ROBUSTNESS_EXIT=0`，`sub-case summary: 13/13 passed`，`RESULT: ALL SUB-CASES PASSED (exit 0)` |
| 10 | 仓库外副本 3 次反事实破坏 + 对应测试（§6） | 三次均 `EXIT=101`，失败点与设计一致 |
| 11 | 仓库外副本 CLI：`cargo run -q -p circuit-cli -- run examples/rc_filter.cdsl --out <tmp> --format csv` + 逐点比对 | `CLI_EXIT=0`；1015 样本；`max|v-ideal_step| = 2.491963e-3 V`；`max|v-matched 500ns ramp| = 6.278341e-7 V`（与 examples 注释的 3 个数字逐位一致） |

副本路径：`C:\Users\15185\AppData\Local\Temp\a12\repo`（robocopy 拷贝，排除 target/.git）；主工作区未被写入任何追踪文件。

## 3. 改动清单与写集归属（是否越界）

| 文件 | 预期 owner | 实际改动内容 | 越界？ |
|---|---|---|---|
| README.md | lead | 文档：示例数值更正、退出码契约更正（`EXIT_INTERNAL` 从未返回）、PULSE 沿钳制、无容差通道 | 否 |
| docs/architecture.md | lead | 文档两行（AC 相位测试、浮空节点判据） | 否 |
| docs/language.md | lead | 文档：浮空节点段落更新 | 否 |
| crates/circuit-backend/src/thevenin.rs | lead（注释修正） | **仅注释**：finding 3 段（17-24 附近）与 Tran 映射注释（757-775 附近），无代码改动 | 否 |
| crates/circuit-core/src/connectivity.rs | lead（注释修正） | **仅模块头注释**（1-24 行），`conducts_dc` 等代码未动 | 否 |
| crates/circuit-dsl/src/elaborate.rs | lead（注释修正） | **仅约 10 行注释**（浮空检查处），逻辑未动 | 否 |
| crates/circuit-cli/tests/e2e.rs | lead（注释修正） | **仅测试上方 doc 注释**，断言体未动（diff 只有该注释 hunk） | 否 |
| _probe/src/main.rs | a06 | Case 2 重写为 13 个小节 + 工具函数；`main` 增加逐 case PASS/FAIL 行 | 否 |
| _probe/src/bin/robustness.rs | a05 | 结构性重写：`Report` + exit(1) + 13 个子用例（a–g） | 否 |
| crates/circuit-dsl/tests/reference_path_regression.rs | a07 | 新增 8 个测试（lex→parse→compile 全链路） | 否 |
| crates/circuit-backend/tests/transient_reference_regression.rs | a08 | 新增 4 个测试（有限斜坡参考解） | 否 |
| crates/circuit-backend/tests/phase_regression.rs | a09 | 新增 6 个测试（非零相位） | 否 |
| examples/rc_filter.cdsl | （无人在派工中声明） | 20:05:11 在本轮冻结后追加了 19 行**注释**（瞬态参考 caveat） | **后加，见 F1** |
| docs/backend-evaluation.md | （无人在派工中声明） | 20:06:51 重写 +141 行（含 §4.6 更正） | **后加，见 F1** |
| docs/testing.md | （无人在派工中声明） | 20:07:43 更新 +/-62 行（407 计数明细） | **后加，见 F1** |
| RUST_CIRCUIT_DSL_PROMPT.md | 用户（本轮之前） | 87 行，mtime 17:19:10（早于本轮全部写入）；wave-1 baseline 已记录为开工前的用户改动 | 否（非本轮） |

- **多余文件/临时文件/误建目录：无**。`Get-ChildItem -Recurse -Include *.orig,*.bak,*.rej,*.tmp,*~`（排除 target）无命中；无新增未预期目录。
- `_probe/Cargo.toml` 哈希与 manifest 一致（未改）。

## 4. 既有断言是否被削弱（逐文件证据）

### 4.1 `_probe/src/main.rs`（a06 声称 Case 1/3/4/5/6 未变）

我把 HEAD 与工作树中相应函数体**逐字节比对**（按函数边界切片后 SHA256 前 16 位）：

| 函数 | HEAD 行区间 | 当前行区间 | 体哈希（HEAD / CUR） | 结论 |
|---|---|---|---|---|
| `case1_divider_op` | 193-239 | 231-277 | `65BF68BEEE00769B` / 同 | **字节相同** |
| `case3_rc_ac` | 346-433 | 1466-1553 | `EB618CED857013F7` / 同 | **字节相同** |
| `case6_dc_sweep` | 622-702 | 1749-1829 | `84DC33A2D1A44CE3` / 同 | **字节相同** |
| `case4_rlc_ac` | 439-531 | 1559-1653 | — | 仅 1 处 hunk：`println!` 被 rustfmt 折行，语义相同 |
| `case5_diode_op` | 536-616 | 1659-1743 | — | 仅 1 处 hunk：`println!` 被 rustfmt 折行，语义相同 |

`git diff HEAD -U0 -- _probe/src/main.rs` 的全部 hunk 头显示：改动集中在 123/154/173（工具函数与 `main`）、242-338（Case 2 区域），以及 Case 4/5 的两处纯格式化；**Case 3 与 Case 6 区间没有任何 hunk**。
另：`main` 现在逐 case 打印 `[PASS]/[FAIL]` 并把 6 个返回值 `&=&` 起来（`_probe/src/main.rs:176-225`），任一 FAIL 仍 `std::process::exit(1)`。**无削弱**。

### 4.2 `_probe/src/bin/robustness.rs`（a05）

- HEAD 版本**没有任何断言**：a–e 五个检查全部只有 `println!`，`main`（HEAD:91-99）顺序调用 5 个函数后直接结束，**恒定 exit 0**。
- 现版本：`Report` 收集每子用例布尔结论（`robustness.rs:96-123`），`main` 在任一失败时打印失败清单并 `exit(1)`（`:288-300`）。a/b/c/d/e 全部保留并新增 f/g 两个负例；实测 13/13 子用例。
- 判据未被放宽：`TOL_V_EXACT=1e-12`、`TOL_I_ZERO=1e-12`、`TOL_AC_MAG=1e-9`、`TOL_AC_PHASE_RAD=1e-9`（`:77-83`）。
- 旧 `base("float")` 用例已被重写为 a2「合法开路输出」（v(a)=v(b)=1 V、i(v1)=0，1e-12），并新增 a3 记录 GMIN 实验；**这是把一条错误证据改成正确证据，不是放宽**。

### 4.3 `crates/circuit-cli/tests/e2e.rs`

`git diff HEAD -- crates/circuit-cli/tests/e2e.rs` 只有一处 hunk，位于 `check_rejects_a_node_with_no_dc_path_to_ground` 上方的 `///` 文档注释（3 行改 4 行）；`#[test]` 与断言体零改动。**仅注释**。

### 4.4 冻结测试文件未被触碰

`crates/circuit-backend/tests/adapter.rs`、`crates/circuit-dsl/tests/elaborate.rs`、`crates/circuit-core/src/plan.rs`、`crates/circuit-core/src/connectivity.rs`（代码部分）哈希与 manifest 一致 → 既有阈值（如 adapter 的 1e-2、connectivity 单测）不可能被改大。**无阈值放宽**。

## 5. 测试诚实性扫描（全仓库）

| 模式 | crates/ | _probe/src | 判定 |
|---|---|---|---|
| `#[ignore]` | 0 | 0 | 无命中 |
| `todo!(` / `unimplemented!(` | 0 | 0 | 无命中 |
| `unreachable!()` | 4（`circuit-backend/src/thevenin.rs:876`、`circuit-dsl/src/eval.rs:284`、`parser.rs:3035,3053`） | 0 | 均为 src 中的穷尽匹配守卫，非占位/跳过 |
| `assert!(true)` | 0 | 0 | 无命中 |
| 被注释掉的断言 `// assert` | 0 | 0 | 无命中 |
| `#[should_panic]` | 0 | 0 | 无命中 |
| skip 相关 | 2 处 `.skip(...)`（`circuit-cli/tests/e2e.rs:507`、`circuit-core/src/span.rs:247`），均为迭代器切片 | — | 与测试跳过无关 |

另对新测试文件单独扫描：`#[allow`、`is_err()/is_ok()` 式静默、`return;` 早退、`unwrap_or` 掩盖，**均无命中**；信号读取一律 `unwrap_or_else(|| panic!(...))`（`phase_regression.rs:275`、`transient_reference_regression.rs:207`），缺信号会立刻炸而非空过。

## 6. 新测试是否自证（期望值来源 / 循环论证 / 静默跳过）

### 6.1 `crates/circuit-dsl/tests/reference_path_regression.rs`（8 个测试）

- 走**真实前端**：`lex` → `parse` → `compile`（`:69-98`），没有任何内部 helper 参与判定；“接受/拒绝”由 `accepted()/rejected()` 直接翻转，期望值是诊断文本 + 错误码 + IR 拓扑，不来自被测实现。
- **双向对照**：`a_legal_open_output_is_accepted`（接受）与 `an_isolated_resistor_network_is_rejected`（拒绝）、`the_same_devices_with_the_capacitor_to_ground_are_accepted`（接受）与 `a_node_coupled_only_through_a_capacitor_is_rejected`（拒绝）、`an_inductor_is_a_dc_reference_path`/`a_diode_is_a_dc_reference_path`（接受）与 `a_current_source_alone_does_not_reference_a_node`（拒绝）——同一器件集的正反两侧都在，单侧“更严/更松”的规则无法全过。
- 断言强度：`assert_name_errors` 同时钉住**错误条数**与**每条都是 `E_NAME`**（`:148-165`）；`assert_does_not_mention` 用于排除“两条消息混用”（`:132-138`）。失败信息带整段渲染文本。
- 无静默跳过：所有分支都 `panic!` 或 `assert!`；无 `#[ignore]`。

### 6.2 `crates/circuit-backend/tests/phase_regression.rs`（6 个测试）

- 期望值全部**解析推导**：`Cx::polar(mag, phi)`（`:108-110`）与 `1/(1+jwRC)`；判据是 brief §17 目标（`AC_ATOL=1e-8, AC_RTOL=1e-4`）**外加**更紧的 `LINEAR_STRICT=1e-9`（`:63-90`）。
- **反向哨兵**：`convention_guards...`/`ac_phase_is_converted_from_radians_to_degrees` 除断言正确值外，还断言三种错误实现（符号取反、实虚交换、漏 `to_degrees`）距离 >1e-3，并要求“若漏转换则差 6.7e-2”这类可量化分离。
- 与实现不共用代码：测试自己写复平面运算（`Cx` 的 `mul/div/phase_rad`，`:102-138`），不调用 `circuit-results` 的同类实现。

### 6.3 `crates/circuit-backend/tests/transient_reference_regression.rs`（4 个测试）

- 参考解 `ramp_reference_at` 是独立数学（`:307-349`，含 `expm1` 稳定化与小 `x` 级数），并由**自己的测试**做独立校验（`reference_solution_matches_independent_checks`，`:489+`）：y(0)=0、两分支在 `u=T` 连续、解析导数满足 ODE、**定步长 RK4 数值积分**比对、小 `T` 极限的 `-(V0*T/(2*tau))` 律、以及表头 allowance 表复现。RK4 与闭式解不是同一段代码 → 不循环。
- 被测值来自产品路径（`TheveninBackend` → `Dataset`），参考解来自手推公式，两者无共享 helper。
- **逐点断言**：`assert_within_criteria` 要求 `violations == 0`（`:436-450`）；`fit_against` 先断言时间轴与信号**长度相等**（`:399`）并逐点断言有限（`:411`）→ 长度不匹配或 NaN 都会失败，不会静默。
- **轴契约**在判据之前执行（`assert_axis_contract`，`:455-473`）：>100 点、`t[0]==0`、严格递增、覆盖 `T_eff+5*tau`；空数组/单点无法“通过”。
- `max_step` 测试把“设置是否真的到达引擎”写成**断言**（点数严格递增，`:768-774`），而不是注释。

### 6.4 反事实实验（仓库外副本，破坏只在副本）

| # | 破坏点（副本内） | 运行命令 | exit | 真实失败信息（摘要） |
|---|---|---|---|---|
| B1 | `thevenin.rs:658` `phase: a.phase_rad.to_degrees()` → `phase: a.phase_rad` | `cargo test --offline -p circuit-backend --test phase_regression` | **101** | `test result: FAILED. 2 passed; 4 failed`；`ac_phase_is_converted_from_radians_to_degrees`：`v(out).re: got 4.99979121996500131e-1, expected 4.33012701892219298e-1, |err| = 6.697e-2 > tol 4.331e-5`；两个 SIN 测试仍 PASS（它们测 `phi`，符合设计） |
| B2 | `connectivity.rs:38` `DeviceKind::Capacitor | DeviceKind::CurrentSource => false` → `true` | `cargo test --offline -p circuit-dsl --test reference_path_regression` | **101** | `FAILED. 6 passed; 2 failed`；`a_node_coupled_only_through_a_capacitor_is_rejected` 与 `a_current_source_alone_does_not_reference_a_node` 均 `panicked: expected the front end to reject this circuit, but compilation succeeded`（reference_path_regression.rs:116） |
| B3 | `thevenin.rs:772` `tmax: spec.max_step` → `tmax: None` | `cargo test --offline -p circuit-backend --test transient_reference_regression` | **101** | `FAILED. 3 passed; 1 failed`；`max_step did not change the returned grid: point counts [516, 516, 516] are not strictly increasing, so this run cannot show that max_step reached the engine` |

- 三次破坏后副本文件已从冻结的主仓库还原，引用哈希比对 `COPY-RESTORED ... matches_frozen=True`（thevenin.rs / connectivity.rs）。
- 结论：三组新测试对“相位转换丢失”“电容被当成直流通路”“max_step 通道断开”三类回归都有**实测鉴别力**（非纸面声明）。

## 7. 注释与引擎源码一致性（lead 的三处 gmin/浮空注释 + Tran 注释）

| 注释位置 | 声明 | 引擎源码核对 | 判定 |
|---|---|---|---|
| `crates/circuit-core/src/connectivity.rs:5-11` | 线性无参考网络由直接解失败，错误不点名节点（`simulate.rs:77-83`） | `thevenin-0.5.0/src/simulate.rs:77-83` 正是 `if !mna.has_nonlinear() { ... mna.system.solve() ... }` 分支起点与首个错误返回 | 一致 |
| 同上 | 对角 gmin 只在 Newton 路径（`newton.rs:361-364`），OP 的 `diag_gmin` 被强制 0（`simulate.rs:99-102`） | `newton.rs:361-364` = `for i in 0..num_nodes { system.matrix.add(i, i, attempt.diag_gmin); }`；`simulate.rs:99-102` = `let opts = NrOptions { diag_gmin: 0.0, ..*base_opts };` | 一致 |
| 同上 | 非线性路径可能返回 `Ok` 且值依赖 gmin | 独立实测（`docs/review-evidence/backend-contract.md` C5，本次复核副本可复现）：默认 `v(a)=5.000000e-1 = I/(2·gmin)`、`GMIN=1e-6 → 5.002499e-7`、`GMIN=1e-3 → 6.666667e-10` | 一致 |
| `crates/circuit-backend/src/thevenin.rs:17-24` | 旧文字“gmin 让无参考节点保持有限”对线性电路是错的；错误不能区分合法开路输出 | 与 `thevenin.rs` 自身实现（`validate` 不含连通性检查）及上述实测一致 | 一致 |
| `crates/circuit-dsl/src/elaborate.rs:479-490` | 同上 + 前端把两种行为都转成点名节点的 `E_NAME` | `elaborate.rs:491-520` 的 `floating_nodes` 循环确实产出点名诊断 | 一致 |
| `crates/circuit-backend/src/thevenin.rs:757-775` | `tmax` 是内部步长上界；引擎无输出抽样；`step` 只用于 tmax 缺省时的 h_max 与 PULSE tr/tf 下限 | `transient.rs:799`（h_max）、`transient.rs:2271-2285`（每接受步记录）、`waveform.rs:37/271-272`（tr/tf 钳制） | 一致 |

抽查 README 新增声明：`EXIT_INTERNAL = 2` 定义于 `crates/circuit-cli/src/main.rs:26`，全仓库 grep 仅命中该定义行 → “没有任何返回它的路径”**属实**；`examples/rc_filter.cdsl` 注释的三个数字经产品路径逐点比对**逐位复现**（1015 样本、2.491963e-3 V、6.278341e-7 V）。

## 8. 发现清单（按严重性）

**F1 — P2｜冻结后仍有清单外追踪文件被写入（流程/可审计性）**
- 位置：`docs/review-evidence/freeze-manifest.md`（清单）+ `examples/rc_filter.cdsl:7-24`、`docs/backend-evaluation.md`、`docs/testing.md`。
- 证据：manifest 冻结时间 2026-09-18T12:00:19.824Z（本地 20:00:19）；我在审查开始时（约 20:0x）的 `git status --porcelain` **不含**这三个文件；审查过程中再次 `git status` 出现 `M docs/backend-evaluation.md`（mtime 20:06:51）、`M docs/testing.md`（20:07:43）、`M examples/rc_filter.cdsl`（20:05:11）；**20:11:37 复查发现仍在继续写入**：`docs/backend-evaluation.md` 20:09:27、`docs/testing.md` 20:10:11（同时 16/16 冻结哈希仍未变，checked=16 mismatches=0）。三者都**不在** manifest 的 16 个哈希内。另外，**`docs/review-evidence/freeze-manifest.md` 自身也在 20:11:02 被重写**（16 项哈希与“冻结时间: 2026-09-18T12:00:19.824Z”均未变，只有门禁计数措辞从「18 个 test binary + 1 个 doc-test」改成「14 个 test binary + 5 个 doc-test 目标」）——即被审查的“冻结基线”文件本身也在审查期间被编辑，且没有任何机制钉住它。
- 影响：manifest 自述“审查者必须基于以上哈希判定”“任何写入都会使本清单失效”，但清单外文件的漂移无法由 manifest 发现；本轮“冻结候选”在审查期间仍是移动目标。我抽查了这三处内容（见 §7 抽查），未发现削弱断言或与源码矛盾；`examples/rc_filter.cdsl` 追加的 19 行注释数值经独立复现**正确**。
- 建议：Lead 在收尾前二选一——(a) 把这三处（以及本轮其它未列入清单的 `docs/review-evidence/*`）纳入并**重新冻结**，重跑门禁；(b) 回退这三处到 HEAD。任选其一后本报告的哈希结论仍需在新清单下复核。

**F2 — P3｜`docs/architecture.md:322` 的事实性错误 + 由此掩盖的覆盖缺口**
- 原文（本轮 lead 改写）：……**正弦波形的 `phi` 在 2026-09 前没有数值测试**（**DSL 无相位语法**，转换只在 IR 直构路径被走到）。
- 反例：DSL **有** SIN 相位语法 —— `crates/circuit-dsl/src/elaborate.rs:1700-1742`（`sin` 的 allowed 含 `phase`；`:1728-1734` 取数并 `deg.value.to_radians()`），`docs/language.md:201` 亦列为公开语法。真正没有语法的是 **AC 源相位**：`elaborate.rs:816-822` 固定 `phase_rad: 0.0`。
- 覆盖缺口（同一处声明掩盖）：`sin(phase:)` 的 deg→rad 转换在任何测试层级都没有走到——`grep phase crates/circuit-dsl/tests/elaborate.rs` = 0 命中，`crates/circuit-cli/tests/e2e.rs` = 0 命中，而新 `phase_regression.rs` 直构 IR 绕过 DSL（其文件头 5-7 行的“语言没有 AC 源相位语法”表述本身是**正确**的）。
- 建议：把该括号改成“AC 源相位无语法（`elaborate.rs` 固定 0），SIN 的 `phase:` 有语法但本项目的测试只走 IR 直构路径”，并补一条最小端到端断言（例如 `elaborate.rs` 或 e2e 里 `sin(..., phase: 30)` → `Waveform::Sin.phase_rad == PI/6`）。

**F3 — P4｜freeze-manifest 计数口径不精确**
**状态：审查期间已被更正（20:11:02），保留此条以便说明审查时点的差异。**
- 我最初读到的版本（冻结版）第 28 行写：「18 个 test binary + 1 个 doc-test」。实测 `cargo test --workspace`（副本）输出 19 行 `test result:` = 14 个测试二进制 + 5 个 doc-test 目标（其中 `circuit_results` 的 doc-test 含 1 个测试；总数 407 一致）。现版本已改为「14 个 test binary + 5 个 doc-test 目标，共 19 条 `test result:` 行」，与实测一致。

**F4 — P4｜冻结外文件的引擎行号引用漂移**
- `examples/rc_filter.cdsl:11` 把“`output_interval` 缺省 → `span/1000`”的适配层映射记作 `thevenin.rs:772-775`；实际该映射在 `crates/circuit-backend/src/thevenin.rs:762-773`（772 行是 `tmax: spec.max_step,`）。同一轮的两个新测试已在文件头声明“行号会漂移，符号名才是锚”，此处建议同样处理。

**非缺陷观察（列出以免被误读）**
- O1：`_probe/src/main.rs` Case 2 的总判据只要求**推荐配置** `tmax=tau/1000` 满足 §17；被扫过的其它 `max_step` 行以 `NOT-MET` 原文保留（`main.rs:1142-1179, 1446-1457`）。这不是掩盖：任一行**运行失败**（`:1181-1184` → `rows.len() != 3` → `tmax_ok=false`，`:1222-1229`）或容差组任一行越界（`:1372-1373`）都会 FAIL。但读者不可把“Case 2 PASS”读成“所有被扫设置都达标”。
- O2：`_probe/src/bin/robustness.rs:588` 的 `has_mid && !honoured` 故意钉住“引擎不尊重 `save`”的现状；将来引擎若开始尊重 save，该子用例会**按设计失败**（注释已说明）。

## 9. 未验证项与限制

1. 审查对象是 **20:00–20:11 的移动快照**：16 个冻结文件在整个审查期间哈希未变（三次校验），但清单外的 `docs/backend-evaluation.md`、`docs/testing.md`、`examples/rc_filter.cdsl` 与 `freeze-manifest.md` 本身都在审查期间被再次写入；我对这些只按 20:11 的内容抽查（rc_filter.cdsl 数值已独立复现）。若之后再有写入，本报告的哈希结论与内容结论均需重做。
2. `_probe/src/main.rs` 约 1829 行：我通读了工具函数、`main`、2.9–2.13 关键段与 NOT-MET 记账（960-1059、1130-1229、1236-1260、1350-1459）以及全部小节标题与判据行；**没有逐行通读 2.1–2.8 的约 600 行**。`robustness.rs` 读了 88-172、258-819（check_a/e/f/g 全文、check_b/c/d 全文）。
3. 未复核 `docs/review-evidence/` 其它证据文档（`cli-qa.md`、`floating-audit.md`、`backend-contract.md`、`rc-reference-math.md`、`next-round-contracts.md` 等）的内部细节，只在与冻结声明交叉处抽查（README 的退出码声明、rc_filter 的三个数字）。
4. 反事实实验只在**仓库外副本**执行；主仓库我运行过 `cargo fmt --check`/`cargo clippy`（仅写 `target/`，未追踪、非 manifest 范围），追踪文件哈希在审查开始与结束两次均为 16/16 一致。我**没有**在主仓库跑 `cargo test --workspace`（避免与其他写者争锁）；407/0/0 是我在副本里独立跑出来的。
5. 门禁的“13/13 子用例”依赖 `_probe` 自身的判定逻辑；我读了它的判定代码并复核 exit 码语义（任一失败 exit 1），未对 13 个子用例逐条重算数学。

## 10. 结论与复现

**结论：NEEDS_FIX**（F1 P2 冻结漂移必须先解决；F2 P3 一句话文档修正 + 建议补一条端到端断言；F3/F4 P4 可选）。
除此之外：**无越界改动、无被削弱的断言、无被掩盖的失败、无静默跳过、无临时/备份文件**；静态检查与全量测试在独立副本中复现通过（`cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` = 407 passed, 0 failed, 0 ignored, exit 0；`probe` exit 0；`robustness` exit 0, 13/13）。

复现要点：

 ``
 # 1) 冻结校验（仓库内只读）
 $rows = ...; foreach (...) { (Get-FileHash $p -Algorithm SHA256).Hash -eq $want }   # → checked=16 mismatches=0
 # 2) 门禁
 cargo fmt --all -- --check ; cargo fmt --manifest-path _probe/Cargo.toml -- --check
 cargo clippy --workspace --all-targets -- -D warnings
 # 3) 仓库外副本（robocopy /E /XD target .git → %TEMP%\a12\repo）
 cargo test --offline --workspace            # TOTAL passed=407 failed=0 ignored=0
 cargo run --offline --manifest-path _probe/Cargo.toml --bin probe       # exit 0
 cargo run --offline --manifest-path _probe/Cargo.toml --bin robustness  # exit 0, 13/13
 # 4) 反事实（副本内改一行 → 跑对应测试 → exit 101；见 §6.4 三条）
 # 5) 产品路径数值复现（副本内）
 cargo run -q -p circuit-cli -- run examples/rc_filter.cdsl --out <tmp> --format csv
 ``

（本报告仅写入 `docs/review-evidence/code-review.md`；未 commit/push；仓库外临时目录 `%TEMP%\a12` 可自由删除。）

