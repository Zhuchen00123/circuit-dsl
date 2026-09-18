# 第 2 轮实施与验收记录（Lead）

日期：本轮。仓库：`F:\codexprojects\dsl000`，git HEAD `cb5d8a2`（工作区未提交；按用户要求不 commit / 不 push / 不发布）。
权威版本：**rev4 冻结清单** `target/round2-logs/freeze-manifest.txt`（16 个文件哈希 + `target\debug\cdsl.exe` 哈希）。
本轮范围：`docs/next-iteration-plan.md` **任务 A + 任务 B**。任务 C（结果表达式）与 D（参数 DAG）按计划留待下一轮，本轮未动。

**版本链**：rev1（20:59 首个候选）→ rev2（`thevenin.rs` 注释-only，已通知全部审核者）→ rev3（clippy 门禁修复 + `E_LIMIT` 归因修复 + capability note + 章节号 + w3 三文件 rustfmt）→ **rev4**（代码审核 R3 建议补的步数预算归因回归：`output_interval_regression.rs` 5 → 6 个测试；产品代码与 rev3 逐字节相同）。每一步都有哈希、真实命令与退出码、以及不变性证据。

## 0. 基线（开工实测，不沿用上一轮结论）

| 项 | 实测 | 命令 |
|---|---|---|
| 工作区测试 | **413 passed / 0 failed / exit 0** | `cargo test --workspace`（`target/round2-logs/baseline-workspace-test.txt`） |
| 独立 probe | 6/6 PASS，exit 0；robustness 13/13，exit 0 | `cargo run --manifest-path _probe/Cargo.toml --bin probe|robustness` |
| 未提交成果 | 全部保留（无 reset / clean / checkout / commit） | `git status --short` |

修复前两个 P1 由 `r4-repro-qa` 用真实 CLI 独立复现（`docs/review-evidence/round2/repro-baseline.md`）：
- **P1-a**：`stop=2us, max_step=1ns, rise=10ns` 不变，仅把 `output_interval` 由 1 ns 改为 100 ns ⇒ 约 50 ns 处 `v(vin)` 由 `1` 变 **`0.5002375000000003`**；两份 CSV 均 2015 点 ⇒ 当时的 `output_interval` 对输出点数毫无影响，唯一效果是污染 PULSE 边沿。
- **P1-b**：`output_interval: -1.ns`（以及 `0.s`）`check`/`run` 均 **exit 0、零诊断**，静默回退 `span/1000`；回退判别探针证明回退值 20 ns 确实到达引擎并把声明 `rise=10ns` 拉宽到 20 ns。

## 1. 任务 A：瞬态参数契约

### 1.1 根因（Lead 直读 vendored 引擎源码，`docs/review-evidence/round2/kernel-contract.md` 逐行复核）

引擎把 `.tran` 的 `step`（`h_print`）用在四处：`h_max = t_max.unwrap_or(min(tstep, tstop/50))`（`transient.rs:799`）、`h_min = tstep*1e-9`（`:1407`）、无-LTE 分支的步长上限 `min(h_max, h_print)`（`:1694`）、以及 **PULSE `tr`/`tf` 的下限 clamp `tr.unwrap_or(tstep).max(tstep)`**（`waveform.rs:37-38`，breakpoint 表同样使用被 clamp 的值 `waveform.rs:271-278`）。旧适配层把 `output_interval` 直接当 `step`，于是「输出采样请求」改写了「用户声明的激励波形」与断点位置。

### 1.2 修复后的契约

| 概念 | 载体 | 规则 |
|---|---|---|
| 输出采样 | `TranSpec.output_interval` | 显式值必须有限且 > 0（前端 `E_VALUE`，**不回退默认**）；**不进入求解器**，求解后独立重采样 |
| 积分步长上界 | `TranSpec.max_step` | 显式值必须有限且 > 0；映射到引擎 `tmax` |
| 引擎 print step | 适配器内部量 `h_print` | `min(span/1000, min(所有源声明的 rise/fall/period))`，单一入口 `tran_step_for`/`print_step_for`，**与 `output_interval` 无关** |
| 能力错误 | `validate()`（`check` 与 `run` 都经过） | 声明 `rise/fall/period` 为 0 或非有限 → `E_UNSUPPORTED`；声明边沿过细、超 1e6 步预算且**归因于波形** → `E_LIMIT` |
| 输出网格 | `circuit-results::resample` | 首点=原始首点；内部点 `首点+k*iv` 且严格小于原始末点；**末点恒保留**；线性插值；禁止越界外推；超 `Limits::max_result_values` → `E_LIMIT`（不截断）；省略 = 原始求解网格 |
| 测量 | `measure` | `avg/rms/max/min` 一律在**原始求解网格**上计算；`RunOutcome.datasets` 是原始数据，`RunOutcome.output_datasets` 是展示/导出视图 |
| 元数据 | 结果 backend settings | `tran.solver_step`、`tran.solve_points`、`tran.waveform_bound`、`tran.max_step`、`tran.output_interval`（后端层）+ `tran.output_grid=resampled-linear`、`tran.output_points`（重采样层） |

### 1.3 验收结果（真实 CLI，rev3 二进制 `5B688194…`）

| 判据 | 结果 |
|---|---|
| 10 ns 边沿反例修复 | coarse（`output_interval=100ns`）**21 点**、`t=100ns` 处 `v(vin)=1`；fine **2001 点**；两者末点 `v(out)` **逐位相同** `0.01975231511815381`（修复前 coarse 为 `0.019311063940867238`） |
| 改 `output_interval` 不改物理解 | 后端层三跑（None/1ns/100ns）原始时间轴与全部样本 **逐位相同**（`crates/circuit-backend/tests/output_interval_regression.rs`）；把修复前 fine 的全部 2015 个解算点与修复后对照，`v(out)` 最大偏差 **1.28e-7 V** ≪ §17 atol |
| 非法值显式拒绝 | `-1.ns` / `0.s` / inf / NaN → `check` 与 `run` 均 **exit 1**、`E_VALUE`、指向实参 span、**stdout 0 字节、零结果文件**（15 个 DSL 层用例，`crates/circuit-dsl/tests/tran_option_validation.rs`） |
| 重采样契约 | 首末点保留、内部等间隔、不越界（实测 337 ns 网格 → 7 点，末间隔缩短为 315 ns 且末点值不变）、复数分量分别插值、单点/退化轴原样、超限 `E_LIMIT`（`crates/circuit-session/tests/tran_output_interval.rs` + `resample` 12 个单测） |
| 测量不被粗采样改变 | fine/coarse 下 `vout_max/min/avg/rms` 与 `vin_avg` **逐位相同**（`avg=0.009884244068709601`、`rms=0.011418146458759418`）；对照：直接对输出视图积分会得到不同的 `vin_avg=0.975` |
| 文件模式与 REPL 一致 | 同一实验的 CSV/JSON **逐字节相同**（`fc /b` 无差异），REPL 打印点数 = CSV 数据行数（21/2001） |
| 既有示例回归 | `examples/` 全部 `check` 7/7、`run` 9/9 exit 0；`rc_filter.cdsl` 的 `rise: 1.ns` 现在被兑现（1019 点，末点与 `1-exp(-t/τ)` 差 4.921844e-6 V，修复前约 2.5e-3 V） |
| 底层行为仍被钉住 | `_probe/src/bin/tran_contract.rs` 12/12：显式 `step=100ns` + 声明 `tr=10ns` ⇒ 实测边沿 **1.0e-7 s**；`step≤tr` ⇒ 边沿=声明值 |

## 2. 任务 B：运行中源断点精度

产品路径回归 `crates/circuit-backend/tests/source_breakpoint_regression.rs`（6 测试）改用**非零 delay、有限 rise/fall、≥2 周期**的 PULSE，对照独立分段解析解，全有效区间逐点按 §17（atol 1e-5 V / rtol 1e-3）判定：

| max_step | 点数 | max abs err | 超限点 | 判定 |
|---|---|---|---|---|
| τ/1000 = 1e-7 | 3129 | 4.999167e-7 V | 0 | **MET（必过）** |
| τ/500 = 2e-7 | 1629 | 1.999333e-6 V | 0 | **MET（必过）** |
| τ/200 = 5e-7 | 729 | 1.248959e-5 V | 3 | 限制（原样保留） |
| τ/50 = 2e-6 | 309 | 7.331775e-4 V | 250 | 限制（原样保留） |

- 40/40 断点精确落在返回时间轴上（偏差 0）；`tran.solver_step=3e-7 ≤ tran.waveform_bound=1e-6`（声明 1 µs 边沿未被拉宽）。
- 机制由 `_probe/src/bin/breakpoint_study.rs`（14/14）独立证实：断点后首个接受步 `h1 = min(2·h_before, h_max)·0.1`，12/12 更接近 Backward-Euler 闭式（worst `|engine−BE|` = 1.110e-16 V）而非梯形闭式；首断点误差 4.999167e-7 V vs 预测 `(V0/T)h1²/(2τ)=5.000000e-7 V`（比值 0.9998）。
- **属于第三方内核固有行为**（`transient.rs:1433` 用步起点判定断点 ⇒ 断点后首步强制 BE；`:1443` 重启步规则），本轮**未改内核**，给出最小复现（step=300 ns、tmax=τ/1000、`t=1.0001e-4 s` 引擎 9.999e-7 V vs 精确 5.0e-7 V）与三种最小适配方案评估。
- **容差**：`RELTOL`/`ABSTOL`/`TRTOL` 单因子在两组配置下均不可观测；`RELTOL+ABSTOL` 交互可改变轨迹，但 **`h1` 与 §17 结论不变** ⇒ 容差通道修不了该缺陷（产品路径仍无容差通道，属既有限制）。
- **精度范围**：可达标界 `h_max ≤ 10·sqrt(2·atol·τ·T/V0)`（本例 = τ/223.6，把 MET/NOT-MET 干净分开）。该界**只对本激励推导**，报告与文档均声明不得外推为任意电路的保证。

## 3. 门禁（Lead 实跑，真实退出码）

```text
cargo test --workspace                                   exit 0   457 passed / 0 failed（24 个测试目标；413 → 457，+44）
cargo clippy --workspace --all-targets -- -D warnings    exit 0
cargo fmt --all -- --check                               exit 0
cargo fmt --manifest-path _probe/Cargo.toml -- --check   exit 0
cargo run --manifest-path _probe/Cargo.toml --bin probe            exit 0（6/6 Case PASS）
cargo run --manifest-path _probe/Cargo.toml --bin robustness       exit 0（13/13 子用例 PASS）
cargo run --manifest-path _probe/Cargo.toml --bin breakpoint_study exit 0（14/14 checks；2 条 §17 NOT-MET 原样保留）
cargo run --manifest-path _probe/Cargo.toml --bin tran_contract    exit 0（12/12 pins）
```

原始日志：`target/round2-logs/rev4-workspace-test.txt`（rev4 测试）、`final-{workspace-test,clippy,fmt}.txt`（rev3）。
**限制**：仅 Windows MSVC / debug profile；未做 release、非 Windows、性能或内存测量；`EXIT_INTERNAL=2` 不可达（源码静态判定，未注入内部错误实证）；`max_step` 是用户显式请求，项目**没有运行期步数上限**（`stop: 1.s + max_step: 1.ns` ≈ 1e9 步会长时间运行，`check` 不拒绝——既有限制，未实测运行时长）。

## 4. 独立审核与 QA（各自对应 rev2/rev3 版本）

| 角色 | 范围 | 结论 |
|---|---|---|
| r1-kernel-recon | 引擎契约（只读） | PASS：`tstep` 的全部使用点、断点机制、输出录制规则均带文件:行号 |
| r2-product-path → 文档 | 产品路径 → 5 个文档/示例 | 完成：language.md 新增 §5.3 输出网格契约，backend-evaluation §5.1 映射与限制，testing §2/3 分布、architecture §5、rc_filter 注释；并按 R4 复核意见补齐窗口参数、E_LIMIT 归因限定与运行期资源边界 |
| r3-test-inventory → 代码审核 | rev2 → rev3 聚焦复核 | 首轮 NEEDS_FIX：W6-1（clippy 门禁 exit 101）必修 + W6-2（E_LIMIT 归因错误）等；**rev3 聚焦复核 PASS：两项均已关闭**（含强制删除 fingerprint 重跑 clippy = exit 0、6 例归因矩阵），并追加一条 LOW 建议（归因语义缺自动化断言）→ **rev4 已补测试并做判别力对照**（把守卫改回"只看步数"→ 新测试 FAIL exit 101，恢复后通过） |
| r4-repro-qa → 数值复核 | 独立复算参考解/误差表/§17 | **PASS**：自建 4 条独立参考路线互检 ≤1.7e-13 V；6 档配置的误差表与点数**零差异**；253 条超限样本逐字段一致；§17 逐字核对未放宽；两不变式成立；10 例证伪未击穿；rev4 清单 **17/17** 一致 |
| w5-cli-qa | 真实 CLI/REPL（87 条命令） | PASS（rev2 复验与 rev1 **逐字节相同**）；2 项 NEEDS_FIX（示例注释 N1、capability note N2）均已在 rev3 关闭 |
| w1/w2/w3 | 回归实现 | PASS：后端 6 测试 / DSL 15 测试 / 会话 5 测试 / 断点 6 测试；三者均做过判别力对照（反转关键断言会变红）；w3 另用 stdout 哈希证明 rustfmt 不改变行为 |

**冻结纪律**：rev1（20:59）→ rev2（21:03，`thevenin.rs` 注释-only，已通知全部审核者并说明 diff）→ rev3（21:15，clippy 门禁修复 + E_LIMIT 归因修复 + capability note + 章节号 + w3 三个文件 rustfmt）→ rev4（代码审核建议的归因测试；产品代码不变）。每次冻结外变更都有哈希、真实命令/退出码与不变性证据（w3 用格式化前后 stdout 哈希比对：probe 输出完全一致，产品测试输出仅差 cargo 墙钟行；r4 用探针 stdout/CSV 哈希证明 rev3 不改数值）。

## 5. 已知限制与未验证（如实保留，不做过度声称）

1. **断点重启精度**：`max_step` 超过上面那个界时产品路径仍会超 §17（τ/200 为 1.25×，属临界）；单 RC 的经验界不外推到多极点/电感/二极管/极端 T/τ。
2. **重采样边界组合未验证**：`start_s ≠ 0`（Lead 实测：`start: 1.us` + `output_interval: 5.us` → 11 点，首点=首个 ≥ start 的求解点）、`uic=true`、同一实验多个 `tran`（Lead 实测：tran1=21 点 / tran2=2015 点，各自正确）、参数扫描（`Axis::Parameter` 不走重采样）。
3. **重采样规模限制只有 `run` 能报**：`check` 是静态检查，无法预知求解器实际点数 ⇒ 同一文件可能 `check` exit 0 而 `run` exit 1（`E_LIMIT`）。
4. **单点/退化时间轴**：不做重采样、原样返回，且不写 `output_grid` 元数据（不可插值时的唯一合理行为，已在 docs/language.md §5.3 记录）。
5. **非法 `output_interval` 会伴随一条级联 `E_ARGUMENT`**（`experiment … declares no analysis`），因为该分析任务被丢弃；不影响退出码，测试依赖它做结构性断言。
6. **产品路径无容差通道**（`RELTOL/ABSTOL/VNTOL/GMIN` 恒为引擎默认）；DSL 无相应语法。
7. `i()`/`w()` 探针的重采样与测量、`--verbose`、release profile、非 Windows 平台、REPL 行编辑（需真实 TTY）未验证。
8. 旧证据文件（`docs/review-evidence/*.md` 第 1 轮、`target/next-round-review/`）**未被覆盖**；`docs/review-evidence/round2/test-inventory.md` 是修复前快照，其引用的 `transient_reference_regression.rs:684` 已被 `:725 …is_not_widened` 取代（见 `code-review.md` W6-8）。

## 6. 本轮交付文件

**产品代码**：`crates/circuit-core/src/plan.rs`、`crates/circuit-dsl/src/elaborate.rs`、`crates/circuit-backend/src/thevenin.rs`、`crates/circuit-results/src/{resample.rs(新),lib.rs,dataset.rs}`、`crates/circuit-session/src/{execute.rs,session.rs}`、`crates/circuit-cli/src/run.rs`
**回归**：`crates/circuit-backend/tests/{output_interval_regression.rs(新),source_breakpoint_regression.rs(新),transient_reference_regression.rs}`、`crates/circuit-session/tests/tran_output_interval.rs(新)`、`crates/circuit-dsl/tests/tran_option_validation.rs(新)`、`_probe/src/bin/{breakpoint_study.rs(新),tran_contract.rs(新)}`
**文档**：`README.md`、`docs/language.md`、`docs/backend-evaluation.md`、`docs/testing.md`、`docs/architecture.md`、`examples/rc_filter.cdsl`
**证据**：`docs/review-evidence/round2/`（design-freeze、team-board、repro-baseline、kernel-contract、product-path、breakpoint-evidence、cli-qa、code-review、numerical-review、test-inventory、implementation-summary、final-gate）

未 commit / 未 push / 未发布，工作区改动完整保留。
