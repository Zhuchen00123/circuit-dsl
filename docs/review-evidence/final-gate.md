# A15 最终门禁复核（final-gate）

复核者：a15-final-gate（严格只读；本文件是本代理唯一写入）
时间：2026-09-18 20:19–20:40 +08:00
仓库：F:\codexprojects\dsl000 ；git HEAD `cb5d8a212f66922181580900a05fb3d42abe32f2`
基线：`docs/review-evidence/freeze-manifest.md`（30 个文件哈希 + 最终门禁声称）
只读声明：未修改/删除仓库内任何文件（仅本报告）；未 commit、未 push；命令只写 `target/` 与仓库外 `%TEMP%\a15\`。
下面每一条都区分【我实测】与【仅读到】。

---

## 0. 结论速览

| 核对项 | 结果 |
|---|---|
| 30 项冻结哈希 | **28/30 一致**；2 项漂移：`README.md`、`docs/review-evidence/implementation-summary.md`（**全部 13 个代码/测试文件一致**，见 §1.1） |
| 工作区文件集合 vs 清单 | 一致（清单内 30 项 + 4 项开工前用户文件 + 原始日志/输出目录 + manifest 自身，逐项归类见 §1.2；无多余源码/临时文件） |
| 六条门禁 | **6/6 exit 0**；`cargo test --workspace` = **413 passed / 0 failed / 0 ignored**（20 条 `test result:` 行，非零 15 条，求和 413）【我实测】 |
| 五套证据一致性 | 无互相矛盾的结论；2 处"看似冲突"已核实为口径差异（407 vs 413、1015 vs 1016），见 §3.2 |
| a12 四条发现 | F1 当时成立（现已重冻结，但**残留 2 个哈希漂移**）；F2 已修正；F3 已修正；**F4 经我独立核实为不成立**（见 §3.3） |
| a11-F2（源断点未覆盖） | 已如实写入 `docs/testing.md` §7（标注"**这是未通过项，不是通过项**"）与 `docs/backend-evaluation.md`，未放宽阈值 |
| cli-qa 两条 LOW | 已写入 `README.md:184`、`README.md:233`、`docs/testing.md:282-284/335` |
| 两个 P1 目标 | 均已修正且**可复现**（§3.6 数字逐项由我重跑核对） |
| 未验证项遗漏 | 2 项（见 §4） |

**最终判定：PASS（产品、测试、数值证据与门禁全部成立）；附 1 项 P2 收尾动作**——`freeze-manifest.md` 中 `README.md` 与 `implementation-summary.md` 的哈希在其自身生成之后又被写入，需由 lead 更新这两个哈希（或回退文件）后，清单才与工作区一一绑定。**无 BLOCKED 项。**

---

## 1. 审查范围与版本

### 1.1 冻结哈希核对【我实测】

对清单中 30 个文件逐个 `Get-FileHash -Algorithm SHA256` 比对：

- **一致 28 项**（含全部 13 个代码/测试文件：`connectivity.rs`、`elaborate.rs`、`thevenin.rs`、`e2e.rs`、`_probe/main.rs`、`robustness.rs`、`_probe/Cargo.toml`、`reference_path_regression.rs`、`phase_syntax_regression.rs`、`transient_reference_regression.rs`、`phase_regression.rs`、`adapter.rs`、`tests/elaborate.rs`）。
- **不一致 2 项**：

| 文件 | 清单哈希 | 实测哈希 | 文件 mtime vs 清单 mtime |
|---|---|---|---|
| `README.md` | `298A3047A01044BC…` | `C832D08EF2B08C81…` | README **20:19:01** > manifest **20:18:35** |
| `docs/review-evidence/implementation-summary.md` | `3F5D4C74FCFB1F4C…` | `BDD29D4AC297D902…` | summary **20:18:47** > manifest **20:18:35** |

**README 漂移的精确 delta 已重建**（我找到 a12 的仓库外快照 `%TEMP%\a12\repo\README.md`，其哈希 **正是清单值 `298A3047…`**，mtime 19:59:04）：

```text
git diff --no-index --stat %TEMP%\a12\repo\README.md README.md
  -> 1 file changed, 3 insertions(+), 2 deletions(-)
```

两处改动均为**正向更正**：
1. 第 5 行：测试数 `389 → 413（本轮新增 24 个）`；
2. 第 218 行：删掉旧句"RC 瞬态对齐 `v(t)=1-e^{-t/τ}`，最差偏差 1.95e-3 V"，改为"有限斜坡 + 匹配分段解析解；基线 `tmax=τ/1000` 下 **6025 点逐点**满足判据、最大误差 **4.999167e-7 V**；旧说法是参考模型错"。
   即：漂移内容本身正确，问题只在**清单哈希没有随之更新**。
- `implementation-summary.md` 的 delta **无法重建**（a12 快照中该文件尚不存在，仓库内无更早副本）。但其**当前内容我逐条核对了关键数字**（§3.6），与我的实测一致，未发现不实陈述。

### 1.2 工作区文件集合核对【我实测】

`git status --porcelain`：# 我实测结果

```text
13 个 tracked 修改：README.md / RUST_CIRCUIT_DSL_PROMPT.md / _probe/src/bin/robustness.rs / _probe/src/main.rs /
  crates/circuit-backend/src/thevenin.rs / crates/circuit-cli/tests/e2e.rs / crates/circuit-core/src/connectivity.rs /
  crates/circuit-dsl/src/elaborate.rs / docs/architecture.md / docs/backend-evaluation.md / docs/language.md /
  docs/testing.md / examples/rc_filter.cdsl
未跟踪：AGENT_TEAM_EXECUTION_PROMPT.md / agent-team-switch.md / docs/prompt-review.md /
  crates/{circuit-backend,circuit-dsl}/tests/{phase_regression,transient_reference_regression,reference_path_regression,phase_syntax_regression}.rs /
  docs/review-evidence/
```

归类结论：
- 清单内 30 项覆盖了 12 个 tracked 修改 + 4 个新增测试文件 + 11 个证据文档 + `_probe` 两项 + 示例与文档；
- **清单外**只有 4 类，均非本轮产物：① `RUST_CIRCUIT_DSL_PROMPT.md`（用户改动，mtime 17:19，早于本轮）；② `AGENT_TEAM_EXECUTION_PROMPT.md`/`agent-team-switch.md`/`docs/prompt-review.md`（开工前已有，`baseline.md` §0 已登记）；③ `docs/review-evidence/` 下的原始日志（`raw-*.txt` 7 个）与 `cli-qa-output/`（a13 的原始证据）；④ `freeze-manifest.md` 自身（**清单没有包含自己**，是清单的一个结构性盲点，但 `git status` 已显示该目录，可人工发现）。
- 未发现 `*.orig/*.bak/*.rej/*.tmp` 之类临时文件。

---

## 2. 门禁独立复跑【我实测，全部看自己的 exit code】

| # | 命令（在仓库根，串行执行） | 我的 exit code | 我的实测输出 | 清单声称 | 一致 |
|---|---|---|---|---|---|
| 1 | `cargo test --workspace` | **0** | **413 passed / 0 failed / 0 ignored**；20 条 `test result:` 行（15 条非零：15+21+6+4+9+18+11+58+84+67+6+8+71+9+25+1=413），无 `FAILED`/`failures:` | 413 / exit 0 | ✅ |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | **0** | 输出中 `^(warning\|error)` 行数 = **0** | exit 0 | ✅ |
| 3 | `cargo fmt --all -- --check` | **0** | 输出文件 **0 字节** | exit 0 | ✅ |
| 4 | `cargo run --manifest-path _probe/Cargo.toml --bin probe` | **0** | 6 个 case 全 `[PASS]`；`RESULT: ALL ACCEPTANCE CASES PASSED` | exit 0（6/6） | ✅ |
| 5 | `cargo run --manifest-path _probe/Cargo.toml --bin robustness` | **0** | `--- sub-case summary: 13/13 passed ---`；`RESULT: ALL SUB-CASES PASSED (exit 0)` | exit 0（13/13） | ✅ |
| 6 | `cargo fmt --manifest-path _probe/Cargo.toml -- --check` | **0** | 输出 **0 字节** | exit 0 | ✅ |
| 7 | （附加）`cargo test --offline -p circuit-backend --test transient_reference_regression -- --nocapture` | **0** | 4 passed；`rc_ramp` 5025 点 7.990897e-8 / 0 超限；`rc_clamped` 1016 点 6.278341e-7 / 0，声明 1 ps 参考 2.491958e-3 / 259；`[516, 5025, 50115]` | — | ✅ |

清单 §3 对 `test result:` 行数的解释（"20 条 = 14 个测试二进制 + 5 个 doc-test 目标 + 1 条额外行；非零计数共 15 行，求和 413"）与我的实测**逐字吻合**。

---

## 3. 五套证据交叉核对

### 3.1 九份报告的结论与状态

| 报告 | 结论 | 与我的实测是否一致 |
|---|---|---|
| `implementation-summary.md`（lead） | 两个 P1 的修正 + 门禁 + 5 条已知限制 | 一致（关键数字见 §3.6） |
| `numerical-review.md`（a11） | PASS；2 项低严重性（F1 a03 阈值文本、F2 delay=0 覆盖缺口） | 一致（我上轮独立复算过；本轮复跑门禁仍然成立） |
| `code-review.md`（a12） | NEEDS_FIX（F1 P2 / F2 P3；F3 已改；F4 P4） | F1 成立（残留漂移见 §1.1）、F2/F3 已处理、**F4 不成立**（§3.3） |
| `cli-qa.md`（a13） | PASS + 2 条 LOW NEEDS_FIX | 一致（两条 LOW 已进 README/testing.md） |
| `floating-audit.md`（A02） | PASS / NEEDS_FIX（旧 gmin 结论 11 处） | 一致：本轮 `robustness` 实跑 a2 显示 `v(b)=1` 精确、a3 显示 GMIN 1e-12/1e-3 同解 |
| `backend-contract.md`（a04） | 线性 singular / 非线性 gmin 二分类 + C1–C5 实测 | 【仅读到】我未复算 C5，但其结论与 a11/A02 的线性实测不冲突 |
| `repo-forensics.md`（a01） | 389 基线独立复核成立 | 【仅读到】本轮基线已到 413，属增量 |
| `next-round-contracts.md`（a14） | 下一轮契约清单 | 【仅读到】与"未验证项"（§4）方向一致 |
| `baseline.md`（lead） | 开工基线与两个 P1 记录 | 一致 |

### 3.2 矛盾/已被后续修正的结论扫描

| # | 表面冲突 | 独立判定 |
|---|---|---|
| C1 | a12 报 **407 passed**，清单/README/testing.md 报 **413** | **不是矛盾**：a12 的副本测于 `phase_syntax_regression.rs`（6 个测试）加入之前；407+6=413。我本轮实测 413，且 docs/testing.md 的 389+24 分解与实测逐条吻合 |
| C2 | `docs/testing.md:185` 写 **1016 点 / 2.491958e-3 V / 259/1016**；`docs/backend-evaluation.md:237` 写 **1015 点 / 2.491963e-3 V / 259/1015** | **不是矛盾，是两套口径**：前者=产品路径测试（`stop = t_eff + 5τ` → 1016 点），后者=`_probe` 旧配置（`stop = 5τ` → 1015 点）。我两条都复现了（§2 第 7 行 + probe 输出）。**建议**：在两句旁各标一次"（产品路径 stop=t_eff+5τ）"/"（_probe stop=5τ）"，避免读者误判 |
| C3 | a12 的 **F4** 说 `examples/rc_filter.cdsl:11` 的行号引用漂移 | **不成立**，见 §3.3 |
| C4 | a11 的 **F1**（a03 §5.3 阈值文本） | **已被 a03 修正**：`rc-reference-math.md:203-206` 加入 a11 独立复核注释，`:213-219` 明确"本行原先误抄了 direct 列的门槛… 订正（F1）"。该文件哈希与最终清单一致 |
| C5 | `README.md` 与 `implementation-summary.md` 的哈希 | **真漂移**（§1.1），但内容是正向更正，且不在代码/测试范围内 |
| C6 | `implementation-summary.md:75` 写原始输出在 `raw-workspace-{test,clippy,fmt}.txt`，清单 §2 写 `raw-final-workspace-*.txt` | 两套文件都存在；**同一句指向了旧日志**，建议改为 `raw-final-*`（INFO） |

### 3.3 a12 的 F1–F4 逐条独立核实【我实测】

- **F1（P2 冻结漂移）——当时成立，现残留同类问题。** a12 的要点（F0 16 项清单外的 `examples/rc_filter.cdsl`、`docs/backend-evaluation.md`、`docs/testing.md` 在冻结后被写入）与事实相符：最终清单已把这 3 个文件 + `phase_syntax_regression.rs` 等纳入（30 项）。但**最终清单自身仍有 2 项漂移**（§1.1）。严重性：P2（审计绑定），不影响门禁结果（我的门禁跑在**当前树**上）。
- **F2（P3 `docs/architecture.md:322` 相位表述）——已修正，我核实新表述正确。** 当前 `architecture.md:322` 写"…**AC 源相位**：…本轮另加 `tests/phase_regression.rs`…**DSL 层的 `sin(..., phase:)` 度→弧度换算此前…没有测试**，本轮补 `phase_syntax_regression.rs`。**AC 源的相位目前没有 DSL 语法**（`elaborate.rs` 构造 `AcSpec { phase_rad: 0.0 }`）"。我核对了源码：`crates/circuit-dsl/src/elaborate.rs:821` = `phase_rad: 0.0,`；`:1707` 允许 `phase` 参数、`:1728-1731` `deg.value.to_radians()`。即 a12 指出的"DSL 有 `sin(phase:)` 语法、只有 AC 源没有"**属实**，且 `phase_syntax_regression.rs` 确实存在（清单哈希一致，6 个测试，我实测通过）。**F2 已闭环。**
- **F3（P4 计数口径）——已修正。** 最终清单 §3 的"20 条 = 14 个 test binary + 5 个 doc-test 目标 + 1 条额外行；非零 15 行；求和 413"与我的实测完全一致。
- **F4（P4 行号漂移）——【独立判定：不成立/假】。** a12 称"映射实际在 `thevenin.rs:762-773`，772 行是 `tmax: spec.max_step,`"。我逐行读了冻结文件（哈希 `CCB30627A0A5872F…`，与清单一致，本轮未被改动）：
  `thevenin.rs:772-775` = `let step = spec / .output_interval / .filter(|s| *s > 0.0) / .unwrap_or_else(|| span / 1000.0);` —— **正是** `examples/rc_filter.cdsl:11` 引用的 `output_interval` 缺省映射；`tmax: spec.max_step` 在 **:781**。因此**该引用准确，a12 的 F4 基于错误的行号**（762-770 是注释块，781 才是 tmax）。**结论：F4 应作废**，无需修改 `examples/rc_filter.cdsl`；若要在最终汇总中保留，应改写为"F4 经复核不成立"。

### 3.4 a11-F2（产品回归未覆盖运行中源断点）是否如实写入、是否被掩盖

**如实写入，未放宽。** 证据（【我实测】+【读到】混合，均已定位）：
- `docs/testing.md:324-333`：以"**产品路径回归未覆盖「运行中源断点」（F2，已知精度限制）**"成段列出，写明 `delay` 固定 0、改成 `delay=100 µs` 后"由 0 点超限变成 **10 点超限、最大误差 4.991676e-5 V**"，并明确"**这是未通过项，不是通过项**：本轮不放宽 §17 阈值、不隐藏数字"。
- `docs/testing.md:226`、`docs/backend-evaluation.md:273` 记录 `τ/200` 的 `3/1217` 超限；probe 输出里我实测到 `NOT-MET(未达标,保留)` 两行与 `3/1217 … max |err| = 1.2490e-5 V`。
- 判据本身未动：`_probe/src/main.rs` 的 `TRAN_ATOL=1e-5/TRAN_RTOL=1e-3` 与清单哈希一致。

### 3.5 cli-qa 两条 LOW 是否已反映

| 发现 | 文档位置 | 是否如实 |
|---|---|---|
| F-1 退出码 2 无返回点 | `README.md:184`（"定义了 `EXIT_INTERNAL = 2`，但全仓库没有任何返回它的路径…不会产生退出码 2"）、`docs/testing.md:282-284`、`:335` | ✅ |
| F-2 `--out` 指向普通文件时报"无法创建目录"而非 `guard_output` 文案 | `README.md:233`、`docs/testing.md`（§6 限制） | ✅（README 交叉引用写作"cli-qa.md 的 **F6**"，而 cli-qa 里该场景的表行是 **F7**/发现编号 **F-2**；属引用标号不精确，INFO） |

### 3.6 关键数字抽查：文档 vs 我的实测

| 数字 | 文档位置 | 文档值 | 我的实测来源 | 一致 |
|---|---|---|---|---|
| 工作区测试总数 | `docs/testing.md:95/129`、README:5 | 413 | `cargo test --workspace` → 413 | ✅ |
| 基线返回点数 / 最大误差 / 超限 | `testing.md:163`、`backend-evaluation.md:209/234`、README:218 | 6025 / 4.999167e-7 V / 0 | probe 输出：`n=6025`、`max \|err\| = 4.999167e-7 V`、`0 / 6025` | ✅ |
| 基线反事实（理想阶跃） | `backend-evaluation.md:235` | 4.966368e-3 V / 1789-of-6025 | probe：`over-limit points = 1789 / 6025`、`4.966368e-3` | ✅ |
| 旧配置匹配参考 | `testing.md?/implementation-summary:31` | 1015 点 / 6.278341e-7 V / 0 | probe：`0 / 1015`、`6.278341e-7` | ✅ |
| 旧配置理想阶跃 | `backend-evaluation.md:237`、implementation-summary:32 | 1015 / 2.491963e-3 V / 259 | probe：`2.491963e-3 V`、`259 / 1015` | ✅ |
| `τ/200` 超限 | `testing.md:226`、`backend-evaluation.md:245/248`、implementation-summary:35 | 1217 点 / 1.2490e-5 V / **3** | probe：`tau/200 500.0 1217 1.2490e-5 … 3 NOT-MET(未达标,保留)` | ✅ |
| 产品路径逐点 | `testing.md:184`、implementation-summary:44 | 5025 / 7.990897e-8 / 0；夹取测试 1016 / 6.278341e-7 / 0 与 2.491958e-3 / 259 | 我复跑 `--nocapture`：逐字一致；`[516, 5025, 50115]` | ✅ |
| `robustness` 子用例 | 清单 §0 / implementation-summary:14 | 13/13 | 我实跑：`13/13 passed`、exit 0 | ✅ |
| 浮空正例 | `implementation-summary:12`、`floating-audit` | `v(b)=v(a)=1 V`、`i(v1)=0`、GMIN 1e-12/1e-3 同解 | 我实跑 robustness a2/a3：`1.000000000000e0`、`0.000000000000e0`、`same solution (observed, to 1e-12)` | ✅ |

**两个 P1 目标的可复现性结论**：P1-a（浮空证据）与 P1-b（RC 瞬态归因）的关键数字全部由我在本轮**亲自重跑**核对通过，不依赖任何转述。

---

## 4. 未验证项清单与覆盖检查

各报告提出的"未验证"条目，与 `README.md` / `docs/*` 的"未实现/未验证"段落对照：

| 来源 | 未验证项 | 文档覆盖 | 判定 |
|---|---|---|---|
| a11 §12 | 全量 workspace 门禁（当时未跑） | 本报告 §2 已补跑 | 已闭环 |
| a11 §12 / floating-audit | 非线性电路浮空行为 | `testing.md:313-314` 明确"仍未覆盖：含非线性器件时前端判定与后端 gmin 行为之间的差异…"，+ `backend-contract C5` | ✅ |
| a11 §12 | AC/TRAN 下的浮空路径 | `testing.md:314` | ✅ |
| a11-F2 | 运行中源断点 | `testing.md:324-333`（"未通过项"） | ✅ |
| a11 §12 | `tmax` 仅 3 档、`delay` 仅 2 点 | `testing.md:324-333` 给出机制与最小复现 | ✅（细度限制） |
| a12 §9 | 快照为移动目标；未通读 main.rs 2.1–2.8；未复核其他证据文档 | 非产品未验证项 | — |
| a13 §7 | REPL 真实 TTY 交互 | `docs/repl.md:299-312` "真实终端下的整段会话 **未验证**" | ✅ |
| a13 §7 | 退出码 2 不可达 | `README:184`、`testing.md:335` | ✅ |
| a13 §7 | 非 Windows 平台 | `testing.md:296-298`、README "尚未实现/未验证" | ✅ |
| a13 §7 | `run --format json` 单独模式 | **未在 testing.md §7 / README 中列出** | ⚠ **遗漏（LOW）** |
| a13 §7 | `check --json` 未执行 | `testing.md:47` 说明其字段结构由测试覆盖 | ✅（部分） |
| a13 §7 | `forward_drop.dc1.csv` 未逐点核对 | 未单列 | ⚠ **遗漏（INFO）** |
| a13 §7 | 并发/长时/性能 | `testing.md:334` "规模性能：没有数万节点级别的性能/内存测试" | ✅ |
| implementation-summary §5 | 产品无容差通道、AC 源无相位语法、`uic` 未暴露、退出码 2、NOT-MET 不影响退出码 | README:232、testing.md:304-307/315-317/335、repl.md | ✅ |
| cli-qa/README | `--out` 指向普通文件 | README:233 | ✅ |

**遗漏项（需补文档）**：① `run --format json` 单独模式未验证未列入 `docs/testing.md` §7；② `forward_drop` 的 DC 扫描 11 点未逐点核对物理值（更接近"未测"而非"已知限制"，可选）。
另外一条**README 层面的完整性建议**：源断点重启精度限制目前只在 `docs/testing.md` §7 与 `docs/backend-evaluation.md`，README 的"已知限制"未列（README 已指向 docs，属可接受，但若希望 README 自洽建议补一行）。

---

## 5. 发现与严重性

| ID | 严重性 | 位置 | 内容 | 建议（谁改什么） |
|---|---|---|---|---|
| G1 | **P2（审计绑定）** | `docs/review-evidence/freeze-manifest.md` 第 34/50 行 | `README.md` 与 `implementation-summary.md` 的哈希在清单生成后（+12s / +26s）又被写入，与实际不符（28/30）。README 的 delta 已重建（389→413、旧瞬态句→已验证的 6025/4.999167e-7），内容是**正向更正**；`implementation-summary.md` 的 delta 无法重建（无更早快照） | **lead**：更新这两个哈希（或回退），并在清单里注明"README/summary 于 20:18:47/20:19:01 有收尾更新"。**不需要重跑门禁**（漂移含 0 个代码/测试文件；我的门禁跑在当前树上） |
| G2 | P3（报告不实） | `docs/review-evidence/code-review.md:178-179`（F4） | F4 依赖错误的行号（称 772 是 `tmax`；实际 772-775 是 `let step = spec.output_interval…`，`tmax` 在 781）→ **F4 不成立** | **lead**：在最终汇总里把 F4 标为"经复核不成立"，不要据它改 `examples/rc_filter.cdsl` |
| G3 | P4（文档口径） | `docs/testing.md:185` vs `docs/backend-evaluation.md:237` | 1016/2.491958e-3（产品路径）与 1015/2.491963e-3（`_probe`）并列出现，未标注 stop 差异 | **A10/lead**：各加 6 字口径标注 |
| G4 | P4（引用） | `implementation-summary.md:75` | 指向 `raw-workspace-*.txt`，与最终清单的 `raw-final-workspace-*.txt` 不一致 | lead 改一行 |
| G5 | P4（引用） | `README.md:233` | 引用"cli-qa.md 的 F6"，cli-qa 中对应表行是 F7 / 发现 F-2 | lead 改引用标号 |
| G6 | LOW（遗漏） | `docs/testing.md` §7 | `run --format json` 单独模式（a13 §7 第 4 条）未列入 | A10/lead 补一行 |
| G7 | INFO | `freeze-manifest.md` | 清单不含自身（自引用盲点），也不含 `raw-*.txt` / `cli-qa-output/` | 保持现状即可，但清单可加一句"另含原始日志目录，不参与哈希" |

**无高严重性发现**：未发现被削弱的断言、被放宽的阈值、被掩盖的失败、代码/测试漂移或临时文件。

---

## 6. 最终判定

- **门禁：PASS**（6/6 exit 0；413 passed / 0 failed / 0 ignored，与清单声称一致，我亲自复跑）。
- **五套证据指向同一版本：基本成立**——13 个代码/测试文件全部与最终清单一致，所有数值证据的关键数字我都能在同一棵树上复现（§3.6）；唯一不完全绑定的是 G1 的两个文档文件哈希。
- **两个 P1 目标：已修正且可复现**（P1-a 浮空正例 `v(b)=1` 精确 + 真反例 singular；P1-b 有限斜坡 6025 点 4.999167e-7 V / 0 超限 + 旧配置 259/1015 反例）。
- **未处理阻断项：无。** a12 的 F1/F2/F3 已闭环；F4 不成立；a11 的两条发现一条已被 a03 修订、另一条（源断点）已作为"未通过项"如实写入文档。
- **最终结论：PASS**（产品与证据成立），附 **1 项 P2 收尾动作 G1**（更新 `freeze-manifest.md` 中两个文档哈希）与 4 项 P3/P4/INFO 文档润色。若把"清单必须与工作区哈希一一绑定"作为强制验收条件，则仅 G1 为 NEEDS_FIX，其余全部通过。

---

## 7. 本报告的未验证项与限制

1. **门禁在"当前树"执行**，而清单的两个文档哈希已过期（G1）；因此"门禁对应当前树"成立，"门禁对应当前清单"对 28/30 文件成立。若 lead 更新清单后文件再变动，本报告结论需重算。
2. **`implementation-summary.md` 的漂移 delta 无法重建**（无更早副本）；我只能确认其**当前内容**与实测一致，不能排除 20:18:35–20:18:47 之间存在我未读到的中间版本。
3. **未复算** `backend-contract.md` 的 C1–C5（含非线性 gmin 实验）、`repo-forensics.md` 的 389 基线明细、`next-round-contracts.md` 的契约条目——只读了结论并与我的实测交叉比对（标注【仅读到】）。
4. **未复跑** a13 的 CLI 用例（`target/release/cdsl.exe` 未被我从零重建）；README/testing.md 中引用的 cli-qa 结果我按"读到的"处理，只交叉核对了退出码 2 与 `--out` 两条 LOW 的文档落点。
5. 集合核对依据 `git status --porcelain` 与目录列举；`target/` 下的构建产物（含 lead 的 QA 日志）未纳入哈希范围（清单也未纳入）。
6. 未验证 `docs/review-evidence/` 中 7 个 `raw-*.txt` 日志内容与我的实测逐字一致（我信任自己重跑的结果，未逐字 diff 日志）。

---

## 8. 复现命令（本报告全部结论）

```powershell
# 1) 哈希核对（列出不一致项）
cd F:\codexprojects\dsl000
$rows = Select-String docs/review-evidence/freeze-manifest.md -Pattern '^\| `([^`]+)` \| ([0-9A-F]{64}) \|' |
  ForEach-Object { $m = $_.Matches[0]; [pscustomobject]@{ p = $m.Groups[1].Value; h = $m.Groups[2].Value } }
foreach ($r in $rows) { $a = (Get-FileHash -Algorithm SHA256 $r.p).Hash; if ($a -ne $r.h) { "MISMATCH $($r.p) manifest=$($r.h.Substring(0,16)) actual=$($a.Substring(0,16))" } }

# 2) 六条门禁（逐条看 exit code）
cargo test --workspace ; "EXIT=$LASTEXITCODE"
cargo clippy --workspace --all-targets -- -D warnings ; "EXIT=$LASTEXITCODE"
cargo fmt --all -- --check ; "EXIT=$LASTEXITCODE"
cargo run --manifest-path _probe/Cargo.toml --bin probe ; "EXIT=$LASTEXITCODE"
cargo run --manifest-path _probe/Cargo.toml --bin robustness ; "EXIT=$LASTEXITCODE"
cargo fmt --manifest-path _probe/Cargo.toml -- --check ; "EXIT=$LASTEXITCODE"

# 3) README 漂移 delta（a12 快照哈希 = 清单值 298A3047…）
git diff --no-index --stat "$env:TEMP\a12\repo\README.md" README.md

# 4) F4 判定：772-775 是 step 映射，781 才是 tmax
(Get-Content crates/circuit-backend/src/thevenin.rs)[771..780]
```