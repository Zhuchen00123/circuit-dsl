# R3 测试与文档清点（只读取证）

- 任务：共享任务 `task-3` / 角色 `r3-test-inventory`（只读）
- 仓库：`F:\codexprojects\dsl000`，分支 `main`，HEAD `cb5d8a2`（不 commit / 不 push）
- 报告写入：`docs/review-evidence/round2/test-inventory.md`（本文件）
- 临时产物：`target/round2-recon/r3/`（本次新建；`target/` 不入版本管理）
- 取证时刻：2026-09-18 20:35–20:45（+08:00）
- 关键前提：**P1/P2 修复正在并行写入中**。`crates/circuit-core/src/plan.rs`（20:37:44）、
  `crates/circuit-dsl/src/elaborate.rs`（20:37:34）、`crates/circuit-backend/src/thevenin.rs`（20:37:50）
  在我取证期间被改动（`mtime` 由 19:51:37 变为 20:37）。本报告的"现有测试行为"以
  **上一轮冻结版（`docs/review-evidence/freeze-manifest.md`）+ 冻结后的工作区 diff** 为准，
  对正在编辑的文件只给符号名锚点，行号会漂移。

## 0. 冻结哈希核对（本次实跑）

命令（解析 `freeze-manifest.md` §1 表 + 逐文件 `Get-FileHash -Algorithm SHA256`）：

```powershell
# 结果写入 target/round2-recon/r3/freeze-hash-check.txt
verified_ok=30 drift=2 missing=0
DRIFT  crates/circuit-dsl/src/elaborate.rs   want=F95ECB7793F69C6D got=1B667B5D6A3525B2
DRIFT  crates/circuit-backend/src/thevenin.rs want=CCB30627A0A5872F got=0862402437484255
```

- 32 项清单中 **30 项仍与冻结值一致**（含 `_probe/src/main.rs` = `4E58AAE5…`、
  `_probe/src/bin/robustness.rs` = `BD52BC83…`，与我在 20:35 独立计算一致）。
- 2 项漂移就是本轮 P1/P2 正在修改的两个生产文件（另加清单外新改的 `plan.rs`）。
  ⇒ 上一轮冻结已失效，本轮必须在**全部写入停止后**重新冻结（见 §6 的"必须避免"）。
- 冻结清单只记录 SHA-256 值、未记录生成命令；本次核对命令如上。

## 1. 测试清单（瞬态 / max_step / output_interval / PULSE / delay / rise / fall / 断点 / 精度阈值）

口径说明：
- "**声明验收**" = 参考解/阈值按用户（DSL 或 IR）**声明的波形参数**建模；
- "**钉后端行为**" = 断言的成立前提是"引擎当前会把声明边沿夹到 `.tran` step"这类实现事实，
  而不是用户声明的语义。这类断言在 P1 修复后必须重写或重新定位。

### 1.1 `crates/circuit-backend/tests/transient_reference_regression.rs`（4 个测试，全部与 P1/P2 相关）

| 文件:行号 | 测试函数 | 断言了什么（阈值与比较对象） | 判定 | P1/P2 修复后的风险 |
|---|---|---|---|---|
| `:490` | `reference_solution_matches_independent_checks` | 参考解自检：分支连续 `<1e-14`；ODE 残差 `τy'+y-v_in < 1e-12`；独立 RK4（h=τ/20000）`<1e-9`；tiny-edge 标度律比值 `|ratio-1|<1e-5`；文档允许误差表 `(y-expected) ≤ 1e-6·|expected|`、allowance `≤ 1e-4·allow` | 独立参考（不驱动引擎） | 无（与本轮修复正交） |
| `:627` | `rc_finite_ramp_matches_reference_on_every_point` | `rise=1e-6`、`output_interval=τ/1000=100 ns`、`max_step=100 ns`、`stop=T_eff+5τ`；对**每个返回点** `\|a-e\| ≤ 1e-5 + 1e-3·\|e\|`；轴契约（`t[0]==0`、严格递增、`len>100`、末点 `≥T_eff+5τ`）；`len>4000`；区间内存在 `0<t<rise` 的点；末值 `V0-v < 1e-2`（窗口检查，非判据） | **声明验收**（`rise ≥ interval`，夹取不生效） | 中：`effective_edge()`（`:354`）与文件头 `:19-30` 把 `T_eff=max(rise, output_interval)` 写死为旧映射；修复后 `h_print` 来源改变，注释/helper 必须同步，断言本身应仍成立 |
| `:684` | `declared_rise_below_output_interval_is_clamped_to_the_output_step` | ① `assert_eq!(t_eff, output_interval)`；② 有效边沿参考 0 超限；③ **要求**声明 1 ps 参考 `violations>0` 且 `max_error > atol+rtol·V0 = 1.01e-3 V`（实测 2.491958e-3 V / 259 点） | **钉后端行为（钉住 P1 bug 本身）** | **最高**：P1 修复后 `t_eff≠output_interval`、声明边沿不再违规 → 该测试必失败；必须改写为"显式设定引擎 step 的底层行为证据"或移入 `_probe`，不能留作产品语义验收 |
| `:742` | `max_step_reaches_the_engine_and_every_setting_meets_the_criteria` | 三档 `max_step ∈ {τ/100, τ/1000, τ/10000}` 只改 `max_step`；点数**严格递增**（516/5025/50115）且 `counts[2] > 5·counts[0]`；每档逐点 §17；轴契约 | **声明验收 + 通道断言** | 低-中：依赖 `h = min(..., h_max, h_print)` 现状；若 `output_interval` 改为"求解后重采样"，点数与 `max_step` 的单调关系仍需成立（重采样会削弱该关系，需复核） |

### 1.2 `crates/circuit-backend/tests/adapter.rs`（21 个测试，瞬态相关 1 个）

| 文件:行号 | 测试函数 | 断言了什么 | 判定 | 风险 |
|---|---|---|---|---|
| `:254` | `rc_transient_matches_analytic` | `delay=0`、`rise=fall=1e-12`；`stop=5τ`、`max_step=output_interval=τ/200=500 ns`；在 0.25/0.5/1/2/3τ 的最近采样点对**理想阶跃** `1-e^{-t/τ}` 断言 `< 1e-2`；`len>100`、`t[0]==0`、末点 `=5τ±1e-9`；**断言时间轴非均匀**（`:355-362`，相邻步差 `>1e-12`）；`i(v1)` 长度一致 | **半钉**：宽阈值（1e-2）恰好容纳 500 ns 斜坡的 2.49e-3 V 残差；注释只讲 `plots[0]` | **高（不同原因）**：若 P1 修复把 `output_interval` 实现为**均匀重采样**，`:355-362` 的"非均匀轴"断言会失败；该测试显式设置了 `output_interval` |

### 1.3 `crates/circuit-backend/tests/phase_regression.rs`（6 个测试，涉及 tran 2 个）

| 文件:行号 | 测试函数 | 断言了什么 | 判定 | 风险 |
|---|---|---|---|---|
| `:617` | `sin_waveform_with_nonzero_phase_matches_engine_formula` | SIN `phi=60°`、`td=200 µs`；Tran `stop=1 ms`、`max_step=stop/2000`、`output_interval=stop/1000`；OP=0；首采样=0；**逐点** `\|v(out)-0.5·v_in(t)\| ≤ 1e-9`；`t≤td` 与 `t>td` 两分支都被执行 | **声明验收**（SIN 无 tr 夹取） | 低。**重要**：这是现有**唯一** `delay≠0` 的产品路径 tran 用例，但它是 SIN、无状态电路（精确到 1e-9），**不能**替代 PULSE 断点回归 |
| `:755` | `sin_waveform_phase_is_degrees_not_radians` | `td=0`、30° 相位首采样 `0.5·sin(30°)=0.25`（误当弧度会得 0.004569） | 声明验收 | 无 |

（同文件 4 个 AC 相位测试不涉及 tran；6 个测试计数与上一轮文档一致。）

### 1.4 `crates/circuit-cli/tests/e2e.rs`（18 个测试，瞬态相关 2 个）

| 文件:行号 | 测试函数 | 断言了什么 | 判定 | 风险 |
|---|---|---|---|---|
| `:275` | `run_rc_filter_matches_the_analytic_response` | 真进程 `cdsl run examples/rc_filter.cdsl --experiment response`；读 `response.tran1.csv`；`tran stop:500 µs, max_step:500 ns`（**无** `output_interval`），源 `rise:1 ns`；4 个采样点 vs 理想阶跃 `< 1e-2`；`len>100`；**非均匀轴**（`:336-343`）；`measure vfinal=0.9…`、`measure vavg=0.80…`；AC 复数值 `< 1e-9` | **半钉**：靠 1e-2 容纳被夹宽后的 2.49e-3 V 残差 | 中：断言本身应仍通过（示例无 `output_interval`，官方网格仍非均匀），但示例注释（`examples/rc_filter.cdsl:7-28`）与 `docs/testing.md` §5 的"参考模型差异"叙述必须同步 |
| `:612` | `json_output_parses` | `response.tran1.json` 的 `analysis=="tran1"`、`kind=="tran"`（`:626-629`） | 命名/契约 | 无 |

### 1.5 `crates/circuit-cli/tests/repl.rs`（11 个测试，瞬态相关 1 个）

| 文件:行号 | 测试函数 | 断言了什么 | 判定 | 风险 |
|---|---|---|---|---|
| `:220` | `a_run_can_write_its_results` | REPL `:run response --out <dir>` 退出码 0、stdout 含 "has"、`response.tran1.csv` 存在 | 文件产出 | 无 |

### 1.6 `crates/circuit-dsl/tests/elaborate.rs`（67 个测试，瞬态/PULSE 相关 4 个）

| 文件:行号 | 测试函数 | 断言了什么 | 判定 | 风险 |
|---|---|---|---|---|
| `:156` | `rc_filter_example_elaborates` | pulse 参数装配（`width=5e-6±1e-18`、`period=1e-5±1e-18`）；`tran stop=30 us`；`max_step` 为正；**`assert_eq!(t.output_interval, None)`（`:247`，注释"`max_step` 不得变成 output interval"）** | 声明验收（语言层契约锚点） | 中：`output_interval==None` 断言本身应保留；但若修复给 `TranSpec.output_interval` 引入"缺省即派生"的语义，该断言语义要重新定义（`docs/testing.md:362-363` 正引用它） |
| `:890` | `tran_validates_its_window` | `stop ≤ start` → `E_SWEEP` | 错误路径 | 低（修复后仍应成立） |
| `:904` | `a_pulse_that_cannot_fit_its_period_is_reported` | `delay+width+rise/fall` 放不进 `period` → `E_VALUE`，消息含 "period" | 错误路径 | 无 |
| `:999` | `measurements_resolve_their_target` | `tran stop:10 ms, max_step:10 us` 可展开；measure 分类 | 语言层 | 无 |

### 1.7 `_probe/src/main.rs`（Case 2，独立工程，非产品路径）

| 位置 | 内容 | 判定 |
|---|---|---|
| `_probe/src/main.rs:646` `case2_rc_tran_body`（`main` 在 `:176`，Case 2 打印在 `:627`） | `td=100 µs`、`T=1 µs`、`step=500 ns`、`tmax∈{τ/50, τ/200, τ/1000}`；§17 判据常量 `TRAN_ATOL=1e-5 V`、`TRAN_RTOL=1e-3`（`:331-332`）；`t_eff = max(t_rise, step)` 契约打印（`:679-693`）；step 扫描（`:1000` 附近）、tmax 扫描（`:1091-1229`）、容差组 A/B（`:1240-1444`）、NOT-MET 汇总（`:1446-1457`） | **引擎行为证据**，直连 `thevenin`，**不能**作为产品语义（声明波形）验收 |
| `:740-781`（`crates/circuit-backend/tests/transient_reference_regression.rs` 的注释也引用） | 断点重启步 `h1=tmax/10`、局部误差 `≈(V0/T)·h1²/(2τ)` 的实测与估计 | P2 已知限制的机制证据 |
| `:1117-1186` | tmax 三档结果：`τ/50`→316 点/1.9933e-4 V/**15 超限**；`τ/200`→1217 点/1.2490e-5 V/**3 超限**；`τ/1000`→6025 点/4.9992e-7 V/0 超限（**NOT-MET 行逐字保留**） | 实测（我本次复跑一致） |

### 1.8 `_probe/src/bin/robustness.rs`（13 个子用例，**无瞬态**）

| 位置 | 内容 | 判定 |
|---|---|---|
| `:271-277`（`main`） | 7 组检查 a–g 共 13 个子用例，全部为 OP/AC（singular、合法开路、GMIN、8 线程隔离、畸形 net id、`save` 子集、DC 路径、无地块、只经电容）；`TOL_V_EXACT=1e-12`、`TOL_I_ZERO=1e-12`、`TOL_AC_MAG=1e-9`、`TOL_AC_PHASE_RAD=1e-9`（`:77-83`） | 与 P1/P2 **无关**；本轮不改 |

### 1.9 覆盖缺口（新增回归前必须知道"哪里是空的"）

| # | 缺口 | 证据 |
|---|---|---|
| G1 | **DSL 层 `tran ... output_interval:` 零测试**：全部 crates 中 `output_interval` 只出现在 `elaborate.rs:247`（断言为 `None`）与两个后端测试直构 IR（`adapter.rs:304`、`transient_reference_regression.rs:278`、`phase_regression.rs:672/790`） | `grep output_interval crates` |
| G2 | **非法值零覆盖**：`0` / 负 / `NaN` / `inf` 的 `output_interval` 无任何测试；`elaborate.rs::tran_spec` 旧版只校验量纲（我实测负值 `check`/`run` 均 exit 0，见 §5） | `grep`, 我的 CLI 复现 |
| G3 | **PULSE `delay ≠ 0` 的产品路径 tran 回归零覆盖**：`transient_reference_regression.rs` 的电路固定 `delay = Quantity::seconds(0.0)`（`:235`） | 上一轮 `numerical-review.md` F2 已记录，本轮仍成立 |
| G4 | `fall`（下降沿）与 `period` 从未在 tran 回归中生效：回归电路用 `width=10 s, period=20 s` 把两者推出窗口（`:240-241`） | 读文件 |
| G5 | 无"改变 `stop` 不改变激励波形"的测试（P1 验收项之一） | 无对应断言 |
| G6 | 无重采样网格契约测试（均匀性、端点、最大点数、`None` 保留原网格）——该功能本轮才引入 | 无对应代码/测试 |
| G7 | `crates/circuit-core/tests/`、`crates/circuit-results/tests/` 目录**不存在**；若重采样落在 `circuit-results`，新测试文件所在目录需先建（或放 `#[cfg(test)]` 单测） | 目录清单 |

### 1.10 测试计数（供文档 §3 更新）

| 测试目标 | 数量 | 本次核对方式 |
|---|---|---|
| `crates/circuit-backend/tests/adapter.rs` | 21 | grep `#[test]` 计数，与 `docs/testing.md:111` 一致 |
| `crates/circuit-backend/tests/phase_regression.rs` | 6 | 同上（`:112`） |
| `crates/circuit-backend/tests/transient_reference_regression.rs` | 4 | 同上（`:113`） |
| `crates/circuit-cli/tests/e2e.rs` / `repl.rs` | 18 / 11 | 同上（`:115-116`） |
| `crates/circuit-dsl/tests/elaborate.rs` | 67 | 同上（`:119`） |
| `crates/circuit-dsl/tests/reference_path_regression.rs` / `phase_syntax_regression.rs` | 8 / 6 | 同上（`:120-121`） |
| `crates/circuit-session/tests/session.rs` | 25 | 同上（`:124`），**无 tran 用例**（只 op/dc/ac） |
| `cargo test --workspace` 基线 | 413 passed / 0 failed / exit 0 | 上一轮冻结门禁 + Lead 本轮实测（我未复跑，原因见 §9 未验证项 1） |

## 2. `_probe` 结构与运行方式

| 项 | 事实 | 证据 |
|---|---|---|
| 是否 workspace 成员 | **否**：根 `Cargo.toml:14` `exclude = ["_probe"]`；`_probe/Cargo.toml` 无 `[workspace]`、无 path 依赖 | `Cargo.toml:1-14`、`_probe/Cargo.toml` |
| 依赖来源 | 全部来自 crates.io：`cirq-ir 0.5.0`、`thevenin 0.5.0`、`thevenin-types 0.5.0`、`num-complex 0.4.6`；`_probe/Cargo.lock` **已入库**（`git ls-files _probe`） | `_probe/Cargo.toml:6-10` |
| bin 清单 | **3 个**（自动发现，无 `[[bin]]`）：`probe` ← `src/main.rs`（1829 行）、`robustness` ← `src/bin/robustness.rs`（841 行）、`currents` ← `src/bin/currents.rs`（145 行，遗留探索工具，无 PASS/FAIL 框架，`.expect()` 失败即 panic/101） | `_probe/src` 清单、`git ls-files _probe` |
| 退出码语义 | `probe`：6 个 Case 各自 PASS/FAIL，任一 FAIL → `std::process::exit(1)`（`:176-225`，`:223`）。`robustness`：13 子用例全部记录，任一 FAIL → `exit(1)`（`:288-298`）。`currents`：无退出码管理 | 源码行号 |
| 输出目录约定 | **无文件输出**：两个 bin 只写 stdout（全文件 grep 无 `File::create`/`fs::write`/CSV 写出）；上一轮把输出重定向到 `target/lead-qa/probe/*.txt`（`target/` 不入库） | `grep`；`freeze-manifest.md:64-65` |
| 运行命令 | `cargo run --manifest-path _probe/Cargo.toml --bin probe`；`cargo run --manifest-path _probe/Cargo.toml --bin robustness`（`docs/backend-evaluation.md:281-287` 给的是 `cd _probe && cargo run --bin probe`） | 文档 + 根 Cargo.toml exclude |
| 编译隔离 | `_probe/target` 是独立 target 目录（根 `.gitignore` 有 `_probe/target`；`_probe/.gitignore` 只有 `/target`），与 workspace 编译互不加锁，但两个 bin 共享 `_probe/target`（Cargo 自带锁，等待不是失败） | `.gitignore`、`team-board.md:78` |

**"判据 / NOT-MET"语义（读 `_probe` 结果的口径，必须写进新报告）：**
- `probe` 的 Case 2 PASS 判据 = **"3 档 tmax 都跑通 + 推荐档 `tmax=τ/1000` 合格"**
  （`rows.len()==3 && recommended_ok`，`main.rs:1138-1141, 1222-1229`）。
- `τ/50`、`τ/200` 两档超限只被逐字打印为 **`NOT-MET(未达标,保留)`**（`:1174-1179`）并在
  `:1446-1457` 汇总，**不影响退出码** ⇒ `probe exit 0 ≠ 所有配置满足 §17`。
- 只有当某档**运行失败**（`:1181-1184`）或容差组任一行越界（`:1435-1444`）才会 FAIL。
- `docs/backend-evaluation.md:276-279`、`docs/testing.md:228-232` 已如实记载该口径。

**本次实测（运行冻结后的既有 exe，不触发编译、不写任何工程文件）：**

```powershell
& .\_probe\target\debug\probe.exe      *> target\round2-recon\r3\probe-run.txt       # exit 0, 163 行, 4.3 s
& .\_probe\target\debug\robustness.exe *> target\round2-recon\r3\robustness-run.txt  # exit 0, 121 行, 13/13
```

- `probe` 末尾 `RESULT: ALL ACCEPTANCE CASES PASSED`；NOT-MET 两行与 `freeze-manifest.md` 记录一致
  （`τ/50`: 316 点 / 1.9933e-4 V / 15 超限；`τ/200`: 1217 点 / 1.2490e-5 V / 3 超限）。
- `robustness` 末尾 `RESULT: ALL SUB-CASES PASSED (exit 0)`、`--- sub-case summary: 13/13 passed ---`。
- 说明：直接运行既有 exe 只写 stdout，未重新编译、未改动 `_probe/target` 内容（无写入）。

## 3. 需要同步的文档章节清单

行号 = 本次核对的当前文件行号（`docs/testing.md`/`backend-evaluation.md` 在 20:23 最后一次写入之后未再变）。
**凡是引用 `thevenin.rs` / `elaborate.rs` / `plan.rs` 行号的地方，修复落地后必须按符号名重核**（上一轮 F4 误报的教训）。

| 文件 | 章节 / 行号 | 现状 | 本轮需要做什么 |
|---|---|---|---|
| `docs/language.md` | §5.1 分析语句 `:318-341`；TRAN 段 `:340-341`；示例 `:326-327` | **完全没有 `output_interval` 语法**（正文只写 `max_step` 是最大内部步长）；`tran` 示例也不带该参数 | 补 `tran ... output_interval:` 的语法、语义（输出采样、非求解器参数）、缺省（None=保留求解器网格）与非法值诊断（`E_VALUE`） |
| `docs/language.md` | §9 数值与安全边界 `:440-447` | 只写 R/L/C 严格正值 | 可在此补"显式非法瞬态选项必须报错、不得回退默认"的一般原则 |
| `docs/language.md` | §10 尚未实现/未验证 `:449-463` | 未提重采样 | 若重采样尚未实现（或仅部分实现），必须显式列为未实现/限制 |
| `docs/backend-evaluation.md` | §5 准入用例结果 `:201-287`；Case 2 明细 `:215-238`；max_step 表 `:240-246`；断点机制 `:248-253`；容差组 `:255-266`；F2 限制 `:268-274`；F3 退出码口径 `:276-279`；可复现命令 `:281-287` | 全部按 `T_eff=max(声明 rise, .tran step)` 与 `output_interval→step` 旧映射写成 | Case 2 参考建模、step/tmax 口径、可复现命令、NOT-MET 数字需按新语义重写；F2（断点）需更新为"产品回归已覆盖/仍未覆盖"的实际状态 |
| `docs/backend-evaluation.md` | §6 适配层约束 `:289-309`，第 8 条 `:305-309` | "PULSE 的 tr/tf 被夹到 .tran 步长"并据此解释 `examples/rc_filter.cdsl` 的 500 ns 斜坡 | 改为对新 `print_step_for`/`waveform_bound` 映射的描述（`step` 不再取 `output_interval`） |
| `docs/backend-evaluation.md` | §7 未验证/限制 `:351-378`（`:367-378` 为本轮新增项） | 无 output_interval/重采样条目 | 增补：重采样契约（网格、端点、上限）、断点精度适用范围、非法值拒绝路径 |
| `docs/testing.md` | §3 测试分布 `:105-155`（汇总行 `:107-131`，明细表 `:135-142`，`:132` 历史对照 389→413） | 计到 413（= 24 个新测试） | 更新新增测试目标/数量与总计数；新文件必须进表 |
| `docs/testing.md` | §4.1 `:158-174`（用例 2 行 `:163`）、§4.2 表 `:176-204` | 判据写 `T_eff = max(声明 rise, .tran step)` | 按新语义改参考模型说明；补新回归的行 |
| `docs/testing.md` | §5 容差怎么定 `:206-249`（`:214-227` 瞬态两套容差、`:228-232` probe 退出码口径、`:245-249` 剩余差距） | 旧夹取叙述 + "断点未被覆盖" | 同步新容差/新覆盖；保留"probe exit 0 ≠ 全达标"口径 |
| `docs/testing.md` | §7 未验证的部分 `:292-339`，断点条目 `:324-333` | "产品路径回归未覆盖运行中源断点（F2）" | 视本轮结果改写为已覆盖/仍受限；`uic` 条目 `:315-317` 与 `elaborate.rs:2459` 行号需重核 |
| `docs/testing.md` | §8 回归测试 `:341-377`，尤其 `:361-363` | 断言"若有人把 `output_interval` 当成内部步长…会失败" | 该表述在 P1 修复后语义反转，必须改写为"新契约 + 新钉子" |
| `docs/architecture.md` | §5 后端适配 `:292-334`，尤其 `:311-313` | 明写"`tran` 的 `step` 取 `output_interval`（缺省 `span/1000`）、`tmax` 取 `max_step`" | **必改**：改为新映射；`:318-325` 表的"测试位置"列补新测试名 |
| `README.md` | §已知限制 `:223-234`：`:229`、`:231`、`:232` | `:231` 明写"PULSE 沿被夹到 `output_interval`（缺省 `span/1000`）⇒ 示例是 500 ns 斜坡，要更陡必须收紧 `output_interval`" | `:231` 必改（新契约 + 新限制）；`:221`（瞬态数值结论）需按新数字更新；`:229`、`:232` 复核后保留 |
| `RUST_CIRCUIT_DSL_PROMPT.md` | §17 `:580-646`：§17.1 `:584-591`、§17.2 `:593-620`（C 用例 `:610-620`，判据句 `:620`）、§17.3 `:622-627`、§17.4 `:629+` | 只读；§17.2 C 已要求"明确的有限上升时间 Tr"并按实际返回时间取解析解，判据 `atol=1e-5 V / rtol=1e-3`，且写明"任何阈值调整都要给出依据" | **不改**。新回归必须能引用 §17.2 C 作为判据出处；不得放宽该判据 |

## 4. 构建配置事实

| 项 | 事实 | 证据 |
|---|---|---|
| `autotests = false` | **6 个 crate 全部没有** | 逐个读 `crates/*/Cargo.toml` |
| 显式 `[[test]]` 列表 | **无**（`circuit-cli/Cargo.toml` 只有 `[[bin]] name="cdsl"`） | 同上 |
| ⇒ 新增集成测试 | `crates/<crate>/tests/<name>.rs` 会被自动发现为目标；**无需**改 `Cargo.toml` | 结论（与 `team-board.md:55` 一致） |
| `crates/*/tests/` 现有文件名（防重名） | `circuit-backend`: `adapter.rs`、`phase_regression.rs`、`transient_reference_regression.rs`；`circuit-cli`: `e2e.rs`、`repl.rs`；`circuit-dsl`: `elaborate.rs`、`phase_syntax_regression.rs`、`reference_path_regression.rs`；`circuit-session`: `session.rs`；`circuit-core`、`circuit-results`: **无 tests 目录** | `glob crates/*/tests/*.rs` |
| workspace 是否 exclude `_probe` | 是（`Cargo.toml:14`），members 6 个（`:3-10`） | `Cargo.toml` |
| `.gitignore` | `/target`、`_probe/target`、`*.rs.bk`、`*.pdb`；`_probe/.gitignore` = `/target` | `.gitignore` |
| `.gitattributes` | `* text=auto eol=lf`、`*.png binary`、`*.pdf binary`；**无 LFS** ⇒ 新文件必须 LF（`cargo fmt --check` 与 diff 才能跨平台一致） | `.gitattributes` |
| target 产物与版本管理 | `target/`、`_probe/target/` 均被忽略 ⇒ 我的临时产物 `target/round2-recon/r3/` 不会污染工作区；`_probe/target/debug/{probe,robustness,currents}.exe` 已存在（19:58/19:50/14:51） | `git status`、目录清单 |

## 5. P1 / P2 现场复现（产品路径，只读，产物在 `target/round2-recon/r3/`）

用既有 `target/release/cdsl.exe`（生成于 19:54:57，晚于 round-2 全部生产改动）与我新建的两个临时 `.cdsl`：

```powershell
& target\release\cdsl.exe run target\round2-recon\r3\p1_repro.cdsl --experiment interval_1ns   --out target\round2-recon\r3\out-interval_1ns   --format csv   # exit 0
& target\release\cdsl.exe run target\round2-recon\r3\p1_repro.cdsl --experiment interval_100ns --out target\round2-recon\r3\out-interval_100ns --format csv   # exit 0
& target\release\cdsl.exe check target\round2-recon\r3\p2_illegal.cdsl   # exit 0
& target\release\cdsl.exe run  target\round2-recon\r3\p2_illegal.cdsl --experiment illegal --out target\round2-recon\r3\out-p2 --format csv   # exit 0
```

| 复现 | 输入 | 最近 50 ns 采样 | 结论 |
|---|---|---|---|
| P1 | 同一电路/`stop=2 µs`/`max_step=1 ns`/`rise=10 ns`，仅 `output_interval` 不同 | `1 ns`：`t=4.95e-8`，`v(vin)=1.0`、`v(vout)=4.449e-4`；`100 ns`：`t=5.0024e-8`，`v(vin)=0.5002375`、`v(vout)=1.251e-4` | 声明的 10 ns 上升沿在两处都已完成，却因 `output_interval` 被静默展宽；两次都 2015 点、exit 0 |
| P2 | `output_interval: -1.ns` | `check` 与 `run` 都 **exit 0**，无诊断；`run` 返回 2015 点（等价于回退默认） | 非法值被静默替换（`elaborate.rs::tran_spec` 旧版只校验量纲；`thevenin.rs` 旧版 `.filter(\|s\| *s > 0.0)` 兜底） |

两处都会与 §1.1 表中 `:684` 的测试、以及 `docs/testing.md:361-363` 的表述冲突——这正是本轮修复的靶子。

## 6. 上一轮证据冻结做法：可沿用 / 必须避免

**`freeze-manifest.md`（79 行）要点**：git HEAD `cb5d8a2`、工作区未提交；
**三版冻结历史**（F0 16 项 → 第二版 30 项 → 第三版 32 项），并披露每次漂移；
32 个文件的 SHA-256 表 + 6 条门禁实跑记录（`cargo test/clippy/fmt`、`probe`、`robustness`、`_probe fmt`，全 exit 0）；
原始日志 `docs/review-evidence/raw-final-workspace-{test,clippy,fmt}.txt` 与 `target/lead-qa/probe/*.txt`；
自引用盲点：清单不含自身哈希、不含 `raw-*.txt` 与 `cli-qa-output/`。
清单**只记哈希值、未记生成命令**（本轮我以 `Get-FileHash -Algorithm SHA256` 逐项复核）。

**`team-board.md`（131 行）**：文件所有权表（同文件单一写入者）+ 开工用户改动清单 + 范围禁令
（禁止放宽阈值/删除断言/`#[ignore]`/回滚他人改动）+ 资源调度（全量测试只由 Lead 跑、
`_probe` 首次 build 串行化）+ 决策 D1–D8（D2 产品无容差通道、D6 PULSE 夹取、D7 前端 `E_NAME`）
+ §6 实测 Team 上限 8 人 ⇒ 复用代理而非新增。

**`final-gate.md`（247 行）**：6/6 门禁 exit 0；13 个代码/测试文件与冻结清单一致；
确认两处"看似冲突"是口径差（407 vs 413 是加入时间差；1015 vs 1016 是 `_probe` 与产品路径的 `stop` 口径差）；
**F4（称 `examples/rc_filter.cdsl:11` 行号漂移）复核为误报（不成立）**，不得据它改示例；
末尾给出 F4 判定所依据的逐行读取记录。

**`final-summary.md`（142 行）**：P1-a/P1-b 两项旧证据修正的实际数字；"生产逻辑一行未改、只改注释与文档"；
24 个新测试分 4 个唯一目标；独立数值复核/代码审核/最终门禁三条结论；未验证项清单；
下一轮任务 1–5（第 4 条正是"断点重启步误差控制 + 运行中源断点产品回归"）。

**本轮应沿用**：
1. 单一写入者 + 交付物哈希冻结，且**所有写入停止后再冻结**（顺序：实现 → 验证 → 文档 → 冻结 → 门禁 → 再冻结）。
2. 新测试"唯一文件 + 唯一文件名"，并做**仓库外反事实实验**（改坏一处 → 对应测试必须 exit 101/1）证明鉴别力。
3. NOT-MET/失败样本逐字保留、不放宽阈值、不 `#[ignore]`、不删既有断言。
4. 引用行号时同时给符号名锚点（引擎 `thevenin-0.5.0/src/waveform.rs:37` 之类外部行号除外）。
5. 门禁原始日志落盘（`target/...`/*.txt），报告引用命令 + 退出码 + 关键计数。

**本轮必须避免**：
1. 上一轮 F1（P2）：先冻结后写入导致清单漂移 —— 本轮 `thevenin.rs`/`elaborate.rs`/`plan.rs` **已在冻结后被改**，
   旧清单已失效，必须重新冻结并显式披露。
2. 上一轮 G1：清单生成后 README 又被写 —— 冻结前停止一切写入。
3. 把 `probe exit 0` 读成"所有配置达标"（F3 口径）。
4. 只凭行号断言就下结论（F4 误报）；以及把"引擎行为证据"当"产品语义验收"（`:684` 这个测试就是例子）。
5. 用旧的 `_probe` 数字（6025/4.999167e-7/τ/50=15 超限…）去描述本轮产品路径结果——两者是不同层、不同 `stop`。

## 7. `git status` / `git diff` 现状（谁改了什么）

`git status --short`（20:45 复核）：**14 个已跟踪文件被修改 + 9 个未跟踪项**（`git status --porcelain` 共 23 行）。

| 类别 | 文件 | 归属 | 本轮提示 |
|---|---|---|---|
| **用户成果（开工前已有，必须保留）** | `RUST_CIRCUIT_DSL_PROMPT.md`（M，17:19）、`AGENT_TEAM_EXECUTION_PROMPT.md`（??，17:24）、`agent-team-switch.md`（??，19:19）、`docs/prompt-review.md`（??，17:19） | 用户 | **不得改**（`RUST_CIRCUIT_DSL_PROMPT.md` 本轮明确只读） |
| 用户本轮计划 | `docs/next-iteration-plan.md`（??，20:26） | Lead（本轮） | P1/P2/断点/任务 A–D 的权威依据 |
| 上一轮（round 2）改动 | `README.md`、`_probe/src/main.rs`、`_probe/src/bin/robustness.rs`、`crates/circuit-cli/tests/e2e.rs`、`crates/circuit-core/src/connectivity.rs`、`docs/architecture.md`、`docs/backend-evaluation.md`、`docs/language.md`、`docs/testing.md`、`examples/rc_filter.cdsl`（10 个）+ 4 个新测试文件 + `docs/review-evidence/` | round-2 代理/Lead | 再次修改需谨慎：其中 4 个测试文件与 6 个文档已进冻结清单，改动即失效需重冻 |
| **上一轮 + 本轮（正在改）** | `crates/circuit-backend/src/thevenin.rs`、`crates/circuit-dsl/src/elaborate.rs` | round-2 改注释；**本轮 P1/P2 修复中**（20:37） | 只读，不得覆盖 |
| **本轮新增改动** | `crates/circuit-core/src/plan.rs`（M，+23 行，20:37:44） | 本轮 P1 修复 | 同上 |
| 未跟踪证据 | `docs/review-evidence/`（含 `round2/`）、`crates/*/tests/{phase_regression,transient_reference_regression,phase_syntax_regression,reference_path_regression}.rs` | 上一轮 | 不 commit |

`git diff --stat`（20:38）：14 files changed, 2362 insertions(+), 306 deletions(-)；
其中 `thevenin.rs` +50、`elaborate.rs` +66、`plan.rs` +23 属**本轮在写状态**。
修复的当前方向（读 diff，供参考，**非我的验收结论**）：`plan.rs` 把三个概念分开（`max_step`=积分步长、
`output_interval`=求解后重采样、源波形=声明值）；`thevenin.rs` 用 `print_step_for(circuit, spec)?` 取代
`output_interval→step`，并新增 `waveform_bound(circuit)` 元数据；`elaborate.rs` 为 `output_interval` 与
`stop/start` 增加有限正数校验。
**注意：`print_step_for`/`waveform_bound` 目前只有调用点、没有定义，`circuit-results::resample` 也不存在
（`grep resample\|print_step_for\|waveform_bound crates` 仅 7 处命中，全在注释/调用），
⇒ 该树**当前不可编译**，任何 `cargo test` 结论都必须等修复落盘后重新采集。**

## 8. 命名冲突与写集建议（供 Lead 排任务）

| 项 | 事实 / 建议 |
|---|---|
| 冲突文件名（禁用） | `transient_reference_regression.rs`、`phase_regression.rs`、`adapter.rs`（backend）；`e2e.rs`、`repl.rs`（cli）；`elaborate.rs`、`phase_syntax_regression.rs`、`reference_path_regression.rs`（dsl）；`session.rs`（session） |
| 建议新文件名 | `crates/circuit-backend/tests/output_interval_regression.rs`（或 `tran_output_interval_regression.rs`）——与现有 3 个目标均不重名；若重采样落在 `circuit-results`，因该 crate 无 `tests/` 目录，可新建 `crates/circuit-results/tests/resample.rs` |
| 测试函数名 | 同一文件内不得重名（编译错）；跨文件重名可编译但 `cargo test <name>` 过滤会含糊，建议每个新函数带唯一前缀，如 `output_interval_*` |
| 改 `transient_reference_regression.rs` 的代价 | 改 `:684` 会使该文件哈希（`E147AF12…`）失效 ⇒ 必须在 round-3 报告里重新冻结并说明"改写理由 + 原断言被移到何处"；`docs/testing.md:113/184-185`、`recommended` 等的引用需同步 |
| `_probe` bin 名 | `probe` / `robustness` / `currents` 已占用；不要新增同名 bin。P1/P2 属产品语义，**不应**只用 `_probe` 证据（`_probe` 直连 `thevenin`，不经 `circuit-dsl`） |
| `autotests` 风险 | 无（见 §4）；新增 `.rs` 无需改任何 `Cargo.toml`，因此**不会触碰 `Cargo.toml`/`Cargo.lock`**（符合只读约束） |

## 9. 结论

**PASS** —— 清点完成，6 项取证全部完成：① 测试面（§1，含逐条断言、阈值、比较对象与"钉行为 vs 声明验收"判定）；
② `_probe` 结构与运行方式（§2，含实跑退出码 0 / 0 与 NOT-MET 语义）；③ 文档同步章节清单（§3）；
④ 构建配置事实（§4）；⑤ 上一轮冻结做法（§6）；⑥ git 现状（§7）。

**同时给出高风险提示（不改变 PASS 结论，属本轮必须处理的已知冲突）**：
1. `crates/circuit-backend/tests/transient_reference_regression.rs:684`
   `declared_rise_below_output_interval_is_clamped_to_the_output_step` 钉住的是 P1 bug 本身，
   P1 落地后**必然失败**，必须改写并说明原断言的去向。
2. `crates/circuit-backend/tests/adapter.rs:355-362` 的"时间轴非均匀"断言，与
   "`output_interval` = 求解后均匀重采样"的新契约可能互斥，需在实现时确认取舍。
3. `examples/rc_filter.cdsl:7-28`、`docs/testing.md:214-227, 361-363`、`docs/architecture.md:311-313`、
   `docs/backend-evaluation.md:219-238, 305-309`、`README.md:221, 231` 全部以旧映射为前提，必须同步。
4. 冻结已失效（2 项漂移 + 1 项清单外改动，见 §0），round-3 必须重新冻结。

### 未验证项（本报告不声称）

1. **未复跑 `cargo test --workspace`**（413 passed / exit 0 引自 Lead 基线），也未跑 clippy/fmt。
   原因：取证时 `thevenin.rs`/`elaborate.rs`/`plan.rs` 正在被并行写入，`print_step_for`/`waveform_bound`/
   `circuit-results::resample` 尚不存在（§7），当前树不可编译，跑出的结果不代表任何稳定版本。
   建议：修复落盘、写入停止后，由 Lead 跑全量门禁并重新冻结。
2. **未运行** `cargo run --manifest-path _probe/Cargo.toml --bin probe|robustness`（会写 `_probe/target`，
   超出本任务允许写集）；实际跑的是**冻结后既有 exe**（§2），语义一致但不是 `cargo run` 路径本身。
3. **未验证修复后的行为**：P1/P2 修复尚未落盘，其测试影响面（§1 各行"风险"列）是**基于当前 diff 的预测**，
   不是实测。
4. **未逐项复现 `docs/review-evidence/*` 中的历史数字**（只核对冻结哈希，§0）；上一轮数值已由
   `numerical-review.md` 独立复核，本轮不重复。
5. **未检查** `_probe/src/bin/currents.rs` 的退出码语义（仅读源码推断：无 PASS/FAIL 框架，`.expect` 失败即 101）。
6. **未验证** 新测试文件所在目录对 `cargo fmt --check` 的影响（新文件须 LF；`.gitattributes` 已强制 `eol=lf`）。

### 本次实际执行的命令与退出码

| 命令 | 退出码 | 产物 |
|---|---|---|
| `git status --short` / `git diff --stat` / `git ls-files _probe` | 0 | §7 |
| `Get-FileHash` 逐项核对 `freeze-manifest.md`（32 项） | 0 | `target/round2-recon/r3/freeze-hash-check.txt`（30 ok / 2 drift） |
| `& _probe\target\debug\probe.exe *> …\probe-run.txt` | **0** | `target/round2-recon/r3/probe-run.txt`（163 行） |
| `& _probe\target\debug\robustness.exe *> …\robustness-run.txt` | **0** | `target/round2-recon/r3/robustness-run.txt`（121 行） |
| `target\release\cdsl.exe run … --experiment interval_1ns` | **0** | `target/round2-recon/r3/out-interval_1ns/` |
| `target\release\cdsl.exe run … --experiment interval_100ns` | **0** | `target/round2-recon/r3/out-interval_100ns/` |
| `target\release\cdsl.exe check …\p2_illegal.cdsl` | **0** | P2 证据 |
| `target\release\cdsl.exe run …\p2_illegal.cdsl --experiment illegal` | **0** | `target/round2-recon/r3/out-p2/` |
| `git diff -U3 -- plan.rs / thevenin.rs / elaborate.rs`（只读） | 0 | §7 在写状态 |

（未 commit、未 push、未改任何产品代码/测试/文档/`Cargo.toml`/`Cargo.lock`。）
