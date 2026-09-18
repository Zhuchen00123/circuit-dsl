# W4 独立数值复核（Task A + Task B）— rev3 口径

- 任务：共享任务 `task-9`（owner: `r4-repro-qa`）
- 日期：2026-09-18 21:0x–21:2x (+08:00)
- 结论：**PASS**（数值结论全部独立复算一致；§17 未放宽；两不变式成立；证伪尝试未击穿；rev3 权威清单已发布且逐行核对 **17/17 一致**，见 §1.5）
- 模式：只读。除本报告与 `target/round2-evidence/w4/` 外未写任何文件；未改产品代码/测试/`Cargo.toml`；未 commit/push。
- 所有数值均由**我自己写的代码**（`target/round2-evidence/w4/` 下的 Rust 探针 + Python 参考）产生，未转述他人结论。

---

## 1. 审查对象、版本与哈希

### 1.1 版本锚点

```
HEAD = cb5d8a212f66922181580900a05fb3d42abe32f2
```

被复核二进制（cargo 报告 up-to-date，即与当前源一致）：

| 对象 | SHA256 | mtime |
|---|---|---|
| `target/debug/cdsl.exe` | `5B688194F32D9020FA591647A41324C047C6882CB1933D9481F2B76248A3024B` | 21:10:34 |
| `…/w4/rustprobe/target/debug/w4probe.exe`（我的探针） | `9E7ACFAB32CE3057BC1520A865C4465D4C7023081EF89CA15A9427DAB5621BD3` | 21:1x |

工具链：`cargo 1.98.1 (797e8a9bc…)`、`rustc 1.98.1 (48ae229cea…)`、Python 3（numpy 2.4.4 / scipy 1.18.1 / mpmath 1.3.0）。

### 1.2 我自己重新计算的哈希（不是照抄清单）

`target/round2-logs/freeze-manifest.txt` 当前在盘上仍是 **rev2**（mtime 21:03:04，文件 SHA256 `6131298999B0667B44F8EF2F4EA8AE9239B3C2AB6F182C52FA12BD03DB4AE59A`；注意 task-9 描述里引用的 rev1 清单 SHA256 是 `A2FB8D93…`，长度 2175 B；rev2 为 2256 B、`thevenin.rs` 已变为 `FCF3BD88…`）。

我在 21:07 对 rev2 清单做逐文件比对：**19/19 全匹配**。
我在 21:12 再次比对：**16/19 匹配，3 个不匹配** —— 这些文件在 **21:10:07–21:10:18** 被改动，晚于 rev2 冻结（21:03:04）：

| 文件 | rev2 清单值 | 我实测值（rev3 口径） | mtime |
|---|---|---|---|
| `crates/circuit-backend/src/thevenin.rs` | `FCF3BD88E8BF24CE…6FB3AFAC` | `33E309BEEC901CB13B70711459D438898D2AC9649F5BF2B8A50E2126E85AEF76` | 21:10:18 |
| `crates/circuit-dsl/src/elaborate.rs` | `61CA8FE8B45787EE…B24B0C87` | `F0F170922B5CC7A958DEE9140A81F99E1F1068F0D8D7650AA04E33BDC0ED065C` | 21:10:13 |
| `crates/circuit-results/src/resample.rs` | `A726934F15B445BD…7E10E363` | `29405F1B8C89FAC62ECFC64C358C8B237AABA9C9FF6C0559BC1FCE07D4434B1D` | 21:10:07 |

`HEAD` 未变；`git status --short` 中的 `M/??` 集合与 rev2 时一致（3 个文件都已是 `M`/`??` 状态的既有条目，未新增路径）。

**结论：rev2 清单在 21:10 起不再是当前代码的描述，任何 rev2 之后的结论不能以“清单未变”为前提。** 我按 lead 的 rev3 通知把复核重跑到当前源（rev3），并对每个产物做了前后对照（见 §1.4）。

### 1.3 rev3 变更的独立核实（lead 通知的 5 项，我逐项验证）

| # | lead 声明 | 我的独立核实 | 结果 |
|---|---|---|---|
| 1 | `thevenin.rs` E_LIMIT 触发面/归因修正 | 读 `thevenin.rs:983-1021`：新增 `effective_step_for`（`:1034`），判据变为 `steps > MAX_PRINT_STEPS && steps_without_waveform <= MAX_PRINT_STEPS`（`:998`）；上下文改为 `declared waveform timing` / `solver step` / `effective step`（`:1011-1013`）。实测 F9/F10（§7）与通知一致 | ✅ 行为确实改变，方向正确 |
| 2 | capability note 补充 output_interval 说明 | `thevenin.rs:112` 出现「`output_interval:` is honoured by resampling the solver's trace after the run」 | ✅ 文字存在（非数值） |
| 3 | `resample.rs` `interior_count` 门禁改写 | `resample.rs:89` 现为 `if !span.is_finite() \|\| span <= 0.0 \|\| self.interval_s <= 0.0` | ✅ 语义保持（NaN/inf 归零，与旧 `!(span > 0.0)` 等价），我的 12 项网格契约全部仍 PASS |
| 4 | 文档章节号 `§6`→`§5.3` | `resample.rs:18` 现为 `docs/language.md §5.3` | ✅ |
| 5 | w3 三个文件仅 rustfmt | 我实测哈希：`source_breakpoint_regression.rs` rev2 `F8C9F390…` → 现 `2F1BDAF4…`；`breakpoint_study.rs` `70F2EE42…` → `1197516B…`；`tran_contract.rs` `CB6890BC…` → `9596B248…`（mtime 均 21:11:45）。由于**文本行号会随格式化漂移**，本报告的 `source_breakpoint_regression.rs` 行号全部取自格式化后的当前文件 | ✅ 行号已重取；语义未变（测试 6/6 通过） |

### 1.4 变更影响：我的数值结论是否受影响？

我对**每个产物**都做了 rev2→rev3 前后对照：

| 产物 | rev3 与 rev2 对照 |
|---|---|
| `w4probe taskb/inv/grid` 三份 stdout | **逐字节相同**（`Compare-Object` 无差异） |
| `taskb_*.csv`（6 份原始轨迹） | 重新生成后 `refcheck.py` 输出**逐行相同**（除我本轮修正的 DOP853 路线数值，见 §3.4） |
| 证伪用例 CSV（F1 raw/mid/coarse、F2、F4、F5 tran1/tran2） | **SHA256 全部相同**（7/7） |
| 单元/集成测试 | 全部 exit 0（§2） |

⇒ **rev2→rev3 的 3 项代码改动（E_LIMIT 归因、resample 守卫写法、章节号）不改变我复核的任何数值**；唯一行为差异在 E_LIMIT 的触发面，我已用 F9/F10 单独取证（§7）。

> 说明：task-9 描述引用的 rev1 哈希（`2D28F01E…`）在我开工前已被 lead 的 rev2 通知取代；rev2 又被 21:10 的改动取代。本报告以**我实测的 rev3 文件内容**为口径，并给出上表供 lead 生成 rev3 清单时核对。

### 1.5 rev3 终检（权威清单逐行核对）

Lead 已发布 rev3 权威清单：`target/round2-logs/freeze-manifest.txt`，**mtime 21:15:03**、1991 B、文件 SHA256 `BFB78B0972F0ED277F9B9B403FFD646C5ECD9FB0303C204588A6D05A2D6B0615`。清单为 **16 行代码/证据文件 + 1 行 `target\debug\cdsl.exe` 哈希**（二进制哈希写在头部注释行内，不是独立哈希行）；相较 rev2，rev3 去掉了 3 行本轮未改动的参照文件（`crates/circuit-dsl/src/parser.rs`、`crates/circuit-core/src/ir.rs`、`crates/circuit-core/src/limits.rs`）。

**我逐行重算的结果（我自己算哈希，不沿用清单结论）：**

| 项 | 数量 | 结果 |
|---|---|---|
| 代码/证据文件哈希行 | 16 | **16/16 一致**，0 不一致、0 缺失 |
| 二进制 `target\debug\cdsl.exe` | 1 | **一致**：清单 `5B688194F32D9020FA591647A41324C047C6882CB1933D9481F2B76248A3024B` = 我实测（逐位比对） |
| 合计 | **17** | **17/17 一致** |

逐文件结果（清单值 = 我实测值；下表由脚本从清单原文生成：首 8 位 … 末 8 位）：

| 文件 | 状态 |
|---|---|
| `crates/circuit-core/src/plan.rs` `0D089AC3…540CBCEF` | OK |
| `crates/circuit-dsl/src/elaborate.rs` `F0F17092…C0ED065C` | OK |
| `crates/circuit-backend/src/thevenin.rs` `33E309BE…E85AEF76` | OK |
| `crates/circuit-results/src/resample.rs` `29405F1B…D4434B1D` | OK |
| `crates/circuit-results/src/lib.rs` `91FD0437…EEDB8EF6` | OK |
| `crates/circuit-results/src/dataset.rs` `FEC7992E…8EFA427E` | OK |
| `crates/circuit-session/src/execute.rs` `9A0BD103…25621F00` | OK |
| `crates/circuit-session/src/session.rs` `428CB2AA…44EA3899` | OK |
| `crates/circuit-cli/src/run.rs` `4B4D32A4…DDC78BD5` | OK |
| `crates/circuit-backend/tests/transient_reference_regression.rs` `889E7122…1B1B572C` | OK |
| `crates/circuit-backend/tests/output_interval_regression.rs` `17481F51…2A884334` | OK |
| `crates/circuit-backend/tests/source_breakpoint_regression.rs` `2F1BDAF4…E0DC009B` | OK |
| `crates/circuit-session/tests/tran_output_interval.rs` `120197BE…AD7EC382` | OK |
| `crates/circuit-dsl/tests/tran_option_validation.rs` `3D7EF591…13C91AA9` | OK |
| `_probe/src/bin/breakpoint_study.rs` `1197516B…E37BD485` | OK |
| `_probe/src/bin/tran_contract.rs` `9596B248…99AE0363` | OK |

**冻结后（21:15:03 之后）的状态确认**（cargo 报告 up-to-date ⇒ 源文件内容与我的二进制/数值同源）：

```
cargo build --bin cdsl                      -> Finished in 0.14s（up-to-date）；cdsl.exe = 5B688194…48A3024B
cargo build --offline（w4probe）            -> Finished in 0.16s（up-to-date）
w4probe taskb / inv / grid                  -> exit 0/0/0，三份 stdout 与已验证运行逐字节相同
cargo test -p circuit-backend --test source_breakpoint_regression -> exit 0，6 passed
python refcheck.py                          -> exit 0，误差表与已验证表逐行相同
```

**确认结论：rev3 的三个代码改动（① `resample.rs::interior_count` 的 clippy 门禁改写 ② `thevenin.rs` 的步数预算 `E_LIMIT` 触发面/归因修复 ③ capability note 与 `§6`→`§5.3` 章节号）不改动本报告的任何数值。** 逐字节对照见 §1.4；其中 ② 是唯一的行为变更，方向已由 §7 的 F9（W6-2 反例：纯 DC 源 + `stop: 1.s, max_step: 1.ns` → `check` exit 0）与 F10（`rise: 1.ps` + `stop: 1.s` → exit 1，上下文 `declared waveform timing: 1e-12 s`）独立取证；①③ 为零语义/纯文字改动，我的 12 项网格契约与全部用例均无变化。

**rev3 终检判定：PASS（17/17 一致）。** 不一致文件：无，故无需点名任何新哈希。§10 的未验证项照旧保留（本次终检不扩大验证面）。

---

## 2. 实际跑的命令与退出码

| # | 命令 | 退出码 | 结果 |
|---|---|---|---|
| C1 | `cargo build --bin cdsl` | 0 | up-to-date，`cdsl.exe` = `5B688194…` |
| C2 | `cargo test -p circuit-backend --test source_breakpoint_regression` | **0** | 6 passed / 0 failed / 0 ignored |
| C3 | 同上 `-- --nocapture --test-threads=1` | 0 | 含全部打印（下表数值来源） |
| C4 | `cargo test -p circuit-backend --test transient_reference_regression -- --nocapture` | **0** | 4 passed |
| C5 | `cargo test -p circuit-backend --test output_interval_regression -- --nocapture` | **0** | 5 passed |
| C6 | `cargo test -p circuit-session --test tran_output_interval` | **0** | 5 passed（本次出现一次 `Blocking waiting for file lock on build directory`，按约定重试后成功，非失败） |
| C7 | `cargo test -p circuit-dsl --test tran_option_validation` | **0** | 15 passed |
| C8 | `cargo test -p circuit-results` | **0** | 83 passed + 1 doctest |
| C9 | `cargo test -p circuit-results --test resample` | 101 | **我的命令错**：`no test target named 'resample'`（该文件的测试是 `src/resample.rs` 内联单测，C8 已覆盖） |
| C10 | `cargo run --manifest-path _probe/Cargo.toml --bin breakpoint_study` | **0** | 14/14 契约检查通过；2 行 `[NOT-MET]` 原样保留 |
| C11 | `cargo run --manifest-path _probe/Cargo.toml --bin tran_contract` | **0** | 12/12 契约钉通过（1 项 `SKIPPED`，见 §4.3） |
| C12 | `rustprobe taskb/inv/grid`（我的探针，`cargo build --offline`） | 0/0/0 | 见 §4/§5/§6 |
| C13 | `python refcheck.py` / `refcheck_dop853.py` / `f1_interp.py` | 0 | 见 §3/§5 |
| C14 | 证伪 CLI 用例 F1–F10（`cdsl check` / `cdsl run`） | 见 §7 | — |

原始输出留档：`target/round2-evidence/w4/run-*.txt`、`probe-*.stdout.txt`、`refcheck*.out.txt`、`falsify/rev3-*`。

---

## 3. 发现 1：独立重推分段解析参考解

我没有使用被测方的 `Reference` 实现，而是从**引擎源码**（`thevenin-0.5.0/src/waveform.rs:23-44,108-156`，我直接读的 vendored 源）重建输入模型，再用**独立的积分表示**求解：

```
y(t) = (1/τ) ∫_0^t e^{-(t-s)/τ} · v_in(s) ds
```

逐线性分段闭式（我自己的推导，未用分段递推）：

```
u1 = t-b, u2 = t-a
∫ = (v0 + m(t-a))(e^{-u1/τ} - e^{-u2/τ}) - m·τ·[(u1/τ+1)e^{-u1/τ} - (u2/τ+1)e^{-u2/τ}]
```

主表用 **mpmath 50 位十进制**求值；密集自检用同一公式的 float64 版本（刻意**不用**递推，以与被测方实现无共享路径）。

### 3.1 我的参考解自检（4 条互相独立的路线）

| 自检路线（我的方法） | 我的实测值 | 判据 | 被测方报告值（对照） |
|---|---|---|---|
| ODE 残差 `max|τ·y' + y - v_in|`（解析导数，30 万点 + 断点加密） | **2.220e-16 V**（@t=1.010040e-4） | < 1e-12 | 5.773e-15 V |
| float64 积分路线 vs 50 位 mpmath | **1.065e-13 V**（@t=2.684e-4） | 参考解自身精度 | —（他们用另一条代数路线 2.687e-14 V） |
| RK4 固定步 h=10 ns（30001 步，独立输入求值器） | **1.696e-13 V** | < 1e-12 | 6.706e-14 V |
| scipy DOP853，**每个断点重启**（42 段，2634 nfev，rtol 1e-12/atol 1e-14，max_step=(b-a)/4） | **8.016e-14 V** | < 1e-12 | —（他们的第四路线是 Green 函数 2.687e-14 V） |
| 理想分段 `v_in` vs 引擎求值器复本（300 万 + 4002 点） | **6.450e-14 V** | 输入模型一致 | 6.450e-14 V |

四条路线互差都在 **1e-13 V 量级**，比 §17 的 `atol = 1e-5 V` 小 8 个数量级 ⇒ **足以作为判据参考**；与被测方参考解的量级一致（他们的 6.7e-14 V）。

### 3.2 我修正的一次自检失败（如实记录）

我第一次写的 DOP853 自检把 `solve_ivp` 布在整段 `[0, 300 µs]` 且不设步长上限，得到 **0.0579 V** 的“巨大偏差”。根因是**我的架设有错**：`t < delay` 时 RHS 恒为 0，误差控制器接受任意大的一步，求解器直接跨过 1 µs 斜坡。改为**逐断点重启 + 步长上限**后为 **8.016e-14 V**（§3.1）。该 0.0579 V **是被测方的假失败**，不是我发现的缺陷；留档 `refcheck.out.txt`（修复前）与 `refcheck.rev3.out.txt`（修复后）。

### 3.3 ODE/判据定义（逐字对照）

- 判据：`|actual - expected| <= atol + rtol*|expected|`，`atol = 1e-5 V`、`rtol = 1e-3`（来源见 §4）。
- 违规计数用 `err > allow`（严格大于），与「≤ 通过」一致。
- 参考解按引擎**实际执行**的边沿构造：`tr_used = max(declared, tstep)`，本激励 `tstep = h_print = 300 ns < rise = 1 µs` ⇒ `tr_used = 1 µs`（声明值未被夹取），我在报告中同样按 `tr_used = 1 µs` 构造。

---

## 4. 发现 2：Task B 误差表独立复算（我的数 vs 被测方的数）

### 4.1 独立取数路径

我**没有**复用被测方的测试代码，而是：
1. 自己写 `w4probe`（`target/round2-evidence/w4/rustprobe/src/main.rs`），只调 `circuit_core::ir` + `circuit_backend::TheveninBackend` 公共 API，自建同一物理激励（`R=1 kΩ, C=100 nF, τ=100 µs`；PULSE `delay=100 µs, rise=fall=1 µs, width=5 µs, period=20 µs`；`stop=300 µs`），把 6 组 `max_step` 的**原始求解轨迹**导出成 CSV（`taskb_*.csv`，Rust `{:?}` 最短往返格式）；
2. 用我自己写的 50 位参考解（§3）在**每一个返回点**上重算 `|err|`、allowance、违规数与 worst ratio（`refcheck.py`，输出 `refcheck.json`）。

### 4.2 逐格对照表（我的数 / 被测方的数）

被测方数值取自：`cargo test … -- --nocapture`（C3）打印，与 `docs/review-evidence/round2/breakpoint-evidence.md` §4（`:70-74`）、`docs/backend-evaluation.md:342-347`、`docs/testing.md:211` 一致。

| `max_step` | 点数（我 / 他们） | `max|err|`（我 / 他们） | 发生时间（我 / 他们） | 违规点数（我 / 他们） | worst ratio（我 / 他们） | 判定 |
|---|---|---|---|---|---|---|
| `τ/50 = 2e-6` | **309 / 309** | **7.331775e-4 / 7.331775e-4** | **2.802000e-4 / 2.802e-4** | **250 / 250** | **19.543 / 19.543** | LIMITATION（一致） |
| `τ/200 = 5e-7` | **729 / 729** | **1.248959e-5 / 1.248959e-5** | **1.000500e-4 / 1.0005e-4** | **3 / 3** | **1.247 / 1.247** | LIMITATION（一致） |
| `τ/500 = 2e-7` | **1629 / 1629** | **1.999333e-6 / 1.999333e-6** | **1.000200e-4 / 1.0002e-4** | **0 / 0** | **0.200 / 0.200** | MET（一致） |
| `τ/1000 = 1e-7` | **3129 / 3129** | **4.999167e-7 / 4.999167e-7** | **1.000100e-4 / 1.0001e-4** | **0 / 0** | **0.050 / 0.050** | MET（一致） |
| `None`（适配层默认） | **1156 / 1156** | **1.657138e-5 / 1.6571e-5** | 2.800284e-4 / （报告未给） | **0 / 0** | 0.068 /（未给） | 一致 |
| `τ/5000 = 2e-8` | **15129 / 15129** | **1.999933e-8 / 1.9999e-8** | 1.000020e-4 /（未给） | **0 / 0** | 0.002 /（未给） | 一致 |

**六行全对**（点数与 `max|err|` 按被测方打印精度逐位相同；后两行是 `breakpoint-evidence.md` §7.1（`:171-176`）里的行，我也一并独立复算）。

### 4.3 超限样本逐条对照（“没有删除失败样本”的独立验证）

被测方 `[LIMITATION-DIAGNOSTIC]` / `[LIMITATION]` 打印与我的 offenders 列表**逐字段相同**：

```
(我 / 他们一致)
τ/50  : t=1.002000e-4 |err|=1.993349e-4 > allowance=1.019987e-5 (expected=0.000199866733, actual=0.000399201597, ratio=19.543x)
τ/50  : t=1.006000e-4 |err|=1.980090e-4 > allowance=1.179641e-5 (expected=0.00179640539, actual=0.00199441436, ratio=16.786x)
τ/50  : t=1.010000e-4 |err|=1.966905e-4 > allowance=1.498337e-5 (expected=0.00498337492, actual=0.00518006541, ratio=13.127x)
τ/200 : t=1.000500e-4 |err|=1.248959e-5 > allowance=1.001250e-5 (expected=1.24979169e-05, actual=2.49875062e-05, ratio=1.247x)
τ/200 : t=1.001500e-4 |err|=1.246879e-5 > allowance=1.011244e-5 (expected=1.12443771e-04, actual=1.24912556e-04, ratio=1.233x)
τ/200 : t=1.003500e-4 |err|=1.237744e-5 > allowance=1.061179e-5 (expected=6.11786041e-04, actual=6.24163480e-04, ratio=1.166x)
```

- 我的违规总数 **250 + 3 = 253**，与被测方 `CDSL_BP_DUMP_ALL=1` 落盘的 253 条明细（`breakpoint-evidence.md:127`、`:254`）**一致** ⇒ “未删除超限样本”有独立证据。
- 我没有发现任何**不一致**：**0 处点名**。

### 4.4 `_probe` 侧（cargo run，C10/C11，exit 0）

我复跑了两个 bin：`breakpoint_study` **14/14 PASS**、`tran_contract` **12/12 PASS**；两行 `[NOT-MET]`（τ/50 250/309、τ/200 3/729）原样打印、**不翻转退出码**，与其文档声明的退出码语义一致。`tran_contract` 的 D 用例下降沿标记 `SKIPPED`（下降沿在窗外），是**显式标注的跳过**，不是静默丢点。

> 这两个 bin 是被测方自己的代码，我只用作旁证；§4.2 的主表完全来自我自己的探针 + 我的参考解。

---

## 5. 发现 3：§17 阈值未放宽（逐字核对）

### 5.1 §17 原文（`RUST_CIRCUIT_DSL_PROMPT.md:620`，逐字）

> 误差断言采用 `abs(actual - expected) <= atol + rtol * abs(expected)`。线性 OP 可先采用电压 atol = 1e-9 V、电流 atol = 1e-12 A、rtol = 1e-6；AC 电压先采用 atol = 1e-8 V、rtol = 1e-4；**TRAN 电压先采用 atol = 1e-5 V、rtol = 1e-3**。以上是初始验收阈值，必须记录求解配置；失败先诊断，任何阈值调整都要给出依据，不能只为通过而放宽。

### 5.2 代码中的实际阈值（rev3 当前行号）

| 文件:行 | 常量 | 值 | 与 §17 |
|---|---|---|---|
| `crates/circuit-backend/tests/source_breakpoint_regression.rs:99` | `ATOL_V` | `1e-5` | ✅ 完全相同 |
| `crates/circuit-backend/tests/source_breakpoint_regression.rs:101` | `RTOL` | `1e-3` | ✅ |
| `crates/circuit-backend/tests/source_breakpoint_regression.rs:596-598` | `allowance()` | `ATOL_V + RTOL * expected.abs()` | ✅ 判据形式一致 |
| `crates/circuit-backend/tests/transient_reference_regression.rs:114,116` | `ATOL_V` / `RTOL` | `1e-5` / `1e-3` | ✅ |
| `crates/circuit-backend/tests/transient_reference_regression.rs:379-381` | `allowance()` | 同上 | ✅ |
| `_probe/src/bin/breakpoint_study.rs:81-82` | `ATOL` / `RTOL` | `1e-5` / `1e-3` | ✅ |
| `crates/circuit-backend/tests/phase_regression.rs:64-65` | `AC_ATOL` / `AC_RTOL` | `1e-8` / `1e-4` | ✅ 对应 §17 的 AC 行 |

**没有任何文件出现比 §17 更大的 TRAN 阈值**（我在 `crates/` 全量 grep `ATOL|RTOL|atol|rtol|tolerance|allowance`，命中 72 处，逐一读过相关行；其余为 OP/AC 常量与文档）。

### 5.3 没有跳过点 / 没有删除失败样本

| 检查项 | 我实测的证据 |
|---|---|
| `#[ignore]` / `#[cfg(ignore)]` | 全仓库 `*.rs` grep：**0 处**（唯二命中是注释里声明“未使用 ignore”） |
| 判据是否逐点遍历全部返回点 | `source_breakpoint_regression.rs:636-669` `fit_against` 用 `t.iter().zip(got.iter())` 遍历**全部**点，无 `continue`/过滤；`assert_within_criteria`（`:674-687`）对 MET 行断言 `violations == 0` |
| 超限样本是否被截断 | `violation_lines`（`:702-725`）**计数与打印分离**：`n` 统计全部违规，只截断*打印条数*；`CDSL_BP_DUMP_ALL=1` 打印全部（`offender_limit` `:693-700`）。我按此复算得 253 条，与落盘一致（§4.3） |
| 粗配置行是否被合并/删除 | `LIMITATION_MAX_STEPS`（`:773`）与 `MET_MAX_STEPS`（`:771`）分离；`max_step_trend_table_…`（`:1036`）对粗行只打标签，对 MET 行**逐点断言**（`:1074`）；独立 `limitation_coarse_max_step_forced_backward_euler_restart`（`:1106`）另有 3 条 characterization 断言（`:1132,:1138,:1143`），其中 `:1143` 要求 `violations > 0` —— 即**若限制消失，测试会失败并强制更新证据**，而不是静默通过 |
| 是否“挑点比较” | `transient_reference_regression.rs:584-586` 的一处**稀疏子采样**只出现在**参考解自身**的 RK4 校验里（注释说明是 RK4 截断误差可见性），并断言 `checked > 50`（`:595`）；**产品判据**（`:661` `assert_within_criteria`）仍逐点。`source_breakpoint_regression.rs` 的 RK4 自检是逐点（`:579-597`） |

**结论：§17 阈值未放宽；无跳过点；无删除失败样本。** 被测方文档在这点上的自我声明（`breakpoint-evidence.md:250,254`；`backend-evaluation.md:349-354`）与我的实测一致。

---

## 6. 发现 4：Task A 两个不变式（我的独立路径）

### 6.1 不变式 (a)：`output_interval` 不进入求解器

**路径 1（session 产品路径，我的 `w4probe inv`）**：解析我自己的 `.cdsl`（同一 circuit，三个 experiment 只差 `output_interval: 1.us / 20.ns / 省略`），调 `circuit_session::execute` 取 `RunOutcome.datasets`（原始）与 `output_datasets`（输出视图）：

```
[inv] RAW point counts: ivA=2015 ivB=2015 ivNone=2015
[inv] RAW grids bitwise equal (1us vs 20ns): axis=true signals=true
[inv] RAW grid equals the no-output_interval run: axis=true signals=true
[inv] OUT point counts: ivA=3 ivB=101 (raw=2015)
[inv] OUT first/last: ivA=(0.0,2e-6) ivB=(0.0,2e-6) raw=(0.0,2e-6)
[inv] OUT(1us) == resample_time(RAW, 1us): times=true values=true
[inv] OUT(1us) inside raw range (no extrapolation): true
[inv] OUT(1us) interior spacing deviation from 1us: 0e0 s
[inv] measures on raw grid:
[inv]   vmax = 0.01975231511815381 (ivB identical: true)
[inv]   vavg = 0.009884244068709601 (ivB identical: true)
[inv]   vrms = 0.011418146458759418 (ivB identical: true)
```

- **原始网格与数值按位相同**（`axis=true signals=true`），且与**完全不给** `output_interval` 的运行也相同 ⇒ 物理不变。
- **测量不变**：`max/avg/rms` 三个测量在 1 µs 与 20 ns 两种输出间隔下**逐位相同** ⇒ 它们来自原始网格（与 `execute.rs:139`「先测量、后重采样」一致）。
- 输出视图等于我对原始数据集独立调用 `resample_time` 的结果（times/values 均 `true`）⇒ 产品路径与重采样层无额外加工。

**路径 2（CLI + 我的独立插值器，F1）**：在**不同电路**（τ=1 µs 快速 RC、50 µs 窗口、10 ns `max_step`）上跑 raw / 100 ns / 5 µs 三个视图，用我自己的线性插值器比对：

```
view      points    max|err| vs analytic      at t [s]  violations
raw         5015            2.494371e-06  1.214647e-06           0
mid          501            1.215657e-05  1.000000e-07           0
coarse        11            1.859904e-07  5.000000e-06           0
interpolation identity: mid  max |view - lerp(raw)| = 0.000e+00 V
                        coarse max |view - lerp(raw)| = 0.000e+00 V
first/last 三个视图完全相同（0 … 4.9999999999999996e-05）
```

⇒ **换一组完全不同的参数，结论不变**：输出视图是原始轨迹的**精确线性插值**，不改变物理解。（这也是“两不变式”在默认配置之外的独立复核。）

**路径 3（被测方回归的对照，仅作旁证）**：`output_interval_regression.rs:476` 的 `a_declared_ten_nanosecond_edge_is_delivered_on_the_raw_grid` 正是我 task-4 的基线电路；其打印 `[edge/1ns]` 与 `[edge/100ns]` 的 `v(vin)` **均为 `1.0`**，且 `:491-499` 断言必须等于 `1.0` 且必须**远离** `OLD_COARSE_VALUE_AT_50NS = 0.5002375000000003`（`:97`）。这与我 task-4 测得的缺陷值完全对应 ⇒ 修复在原始网格上确实生效。

### 6.2 不变式 (b)：输出网格契约（我的 12 项检查，全 PASS）

`w4probe grid`，对**手工构造**的 `Dataset` 直接调 `circuit_results::resample_time`：

```
[grid] PASS first-interior-last :: [0.0, 0.25, 0.5, 0.75, 1.0]
[grid] PASS interval>window -> 2 points :: [0.0, 1.0]
[grid] PASS interval==window -> no duplicate :: [0.0, 1.0]
[grid] PASS grid point on last sample not duplicated :: [0.0, 0.5, 1.0]
[grid] PASS linear interpolation on non-uniform axis :: max |v-t| = 0e0
[grid] PASS no extrapolation :: [0.0, 0.3, 0.6, 0.8999999999999999, 1.0]
[grid] PASS interval 0/-/NaN/inf rejected :: E_VALUE expected for all five
[grid] PASS single-point axis unchanged :: [0.0]
[grid] PASS non-time axis unchanged :: [1.0, 10.0]
[grid] PASS oversized grid -> E_LIMIT :: Some([Limit])
[grid] PASS complex signal component-wise :: re=1, im=2 at the midpoint
[grid] PASS 1e15 interior points -> error (not truncation) :: Some([Limit])
[grid] ALL = true
```

与文档契约（`docs/language.md:385-399` §5.3）逐条对应：首点复制、内部点严格小于末点、末点恒保留、线性插值、禁外推、`E_LIMIT` 不截断、省略时=原始网格、退化轴原样。

两点实现观察（非缺陷）：
- 内部点用 **`first_s + k as f64 * interval_s`**（`resample.rs:125`）单次乘法而非累加，所以没有累加漂移；上面 `0.8999999999999999` 正是这一形式的可见证据（`3*0.3` 的最近 f64）。
- “末段可短于 interval”由 `:127` 保证，且 `interval == window`、网格点落在末样本上都不会重复（我各测一例）。

---

## 7. 发现 5：证伪尝试（10 个用例，全部实测）

| # | 用例（若实现有 bug 就会暴露） | 预期 | 实测（rev3） |
|---|---|---|---|
| **F1** | coarse 输出 vs 解析解（见 §6.1 路径 2） | 视图=插值、物理不变 | ✅ 视图与我的 lerp **0.000e+00 V** 差；0 违规 |
| **F2** | `output_interval ≥ 窗口`（`10.us` vs `stop: 2.us`） | 只有首末 2 点，不外推 | ✅ run exit 0，`tran1: 2 time points`，CSV = `0` 与 `0.000002` |
| **F3** | `interval` 极小（`1.ns` over `stop: 1.s`） | `check` 通过、`run` 报 `E_LIMIT` 且不截断 | ✅ check **exit 0**；run **exit 1** `E_LIMIT: … needs 1000000001 points (1000000001 values), over the limit of 50000000` |
| **F4** | `start > 0`（`start: 1.us, stop: 2.us, output_interval: 400.ns`） | 网格起点=**原始首点**（不是 `start`），末点=原始末点 | ✅ 4 点：`1.0004999999999934e-6`（引擎原始首点）、`1.4004999999999934e-6`、`1.8004999999999936e-6`、`2.0e-6`；内部间隔精确 400 ns |
| **F5** | 同一实验**两个 tran 任务**、不同 interval | 各自拿到自己的 interval（序号映射） | ✅ `tran1: 401 time points`（5 ns）与 `tran2: 5 time points`（500 ns），两个 CSV |
| **F6** | 声明 `rise: 0.s` | `E_UNSUPPORTED`（不静默替换） | ✅ check **exit 1**：`the backend cannot honour 'rise: 0' on source 'v1' …` |
| **F7** | 声明边沿过细超预算（`rise: 1.ns` + `stop: 1.s`，无 `max_step`） | `E_LIMIT`，归因到**声明波形** | ✅ check **exit 1**：`… would need about 1000000000 solver steps, over the limit of 1000000` + `declared waveform timing: 0.000000001 s` |
| **F8** | `output_interval: 0.s` 与 `-1.ns` | `E_VALUE`（**task-4 的 P1-b 修复**） | ✅ check **exit 1**：`'output_interval:' must be a finite number greater than zero`（+ `experiment 'zeroiv' declares no analysis`） |
| **F9** | **W6-2 反例**：**纯 DC 源**（无波形）+ RC + `stop: 1.s, max_step: 1.ns` | rev3 修复后应 **exit 0**（旧实现误报） | ✅ check **exit 0** —— 我独立复现了 lead 的修复方向 |
| **F10** | `rise: 1.ps` + `stop: 1.s`，无 `max_step` | 仍应被拒，且归因/上下文正确 | ✅ check **exit 1**：`about 1000000000000 solver steps` + `declared waveform timing: 0.000000000001 s`（另见 `solver step` / `effective step` 上下文，`thevenin.rs:1011-1013`） |

一条**我方输入错误**（如实记录）：F5 首版把两个 `save` 语句写在同一个 experiment 里，被正确拒绝：`error[E_DUPLICATE]: an experiment may have only one 'save' statement`（exit 1）。改为一条 `save` + 两个 `tran` 后通过。**不作任何结论**。

---

## 8. 发现 6：过度声称评估

### 8.1 未发现实质过度声称

| 检查对象 | 结论 |
|---|---|
| `h_max ≤ 10·sqrt(2·atol·τ·T/V0)` 是否被当作通用保证 | **否**。测试文件 `source_breakpoint_regression.rs:47-52` 明写“**This is a bound for this stimulus**… It is not a general 'tau/1000' guarantee for arbitrary circuits”；`docs/backend-evaluation.md:349-351`「只对该激励推导…不要把它写成'任何电路用 τ/1000 都安全'」；`docs/language.md:531-534`、`docs/testing.md:371`、`breakpoint-evidence.md:96,265` 同样限定。我读到的四处口径一致 |
| 限制行是否被包装成通过 | **否**。`LIMITATION` 标签 + 独立 characterization 测试 + “不得删除样本”的显式注记（`breakpoint-evidence.md:254,259`，`source_breakpoint_regression.rs:1074-1081,1143`）。`backend-evaluation.md:259` 还把它列为“需要 Lead 注意的遗留风险” |
| `max|err|` 单调性 | **未被虚假断言**。`breakpoint-evidence.md:179` 明确写出 `max|err|` **不**单调（`None` 行 1.6571e-5 > τ/500 的 1.9993e-6），并要求“看超限数而不是 max|err|”。我的独立复算正好证实这一点（我的 msNone=1.657138e-5 > τ/500=1.999333e-6，而两者违规数都是 0） |
| `output_interval` 视图是否被声称满足 §17 | **否**。`docs/language.md:385-399` 只声明网格与三条可依赖性质，**没有**说重采样视图逐点满足 §17。这是正确的克制：我的 F1 实测到 `mid`（100 ns）视图插值误差达 **1.215657e-5 V**（原始视图 2.494e-6 V），单点已**超过 `atol = 1e-5 V`**（该点 allowance = 1.05e-4 V，故未违规）。⇒ **粗输出间隔会引入它自己的插值误差，§17 只覆盖求解网格**；建议在 §5.3 或 §17 加一句显式说明，避免读者把视图误差算到求解器头上 |

### 8.2 三处**需要澄清**（非数值缺陷，建议 lead 处置）

1. **`transient_reference_regression.rs` 的 `effective_edge()` 仍是旧映射语义**：`:375-376` 定义为 `declared_rise.max(output_interval)`，即 Task A 之前的 `T_eff` 规则。它在 `:652`、`:860` 两处**通过路径**仍被调用；由于两处 `output_interval ≤ rise` 且有 `:653` 的 `assert_eq!(t_eff, rise)` 守卫，结论**不受影响**（我复跑了该文件 4/4 通过，`[rc_ramp] T_eff=1e-6 s`）。但 helper 名与文件头 `:19-30,:41` 的叙述会误导读者以为 `output_interval` 仍参与引擎步长。`docs/review-evidence/round2/test-inventory.md:43` 已识别该风险；建议 rev3 后改名/加注（纯注释改动）。
2. **`docs/backend-evaluation.md` 内同一标签两个点数**：§5.1（`:234,246`）写「基线 `tmax = τ/1000` → **6025** 点」，§5.1.2（`:344`）写「`τ/1000` → **3129** 点」。两者**都对**，因为窗口不同（前者 `[0, 6.01e-4] s`，后者 `stop = 300 µs`），但同一个标签「τ/1000 基线」并排出现容易被读成矛盾。建议在 §5.1 标题补窗口参数。
3. **`docs/language.md:417` 的“引擎步数预算的 `E_LIMIT` 在 check 阶段即报”** 在 rev3 后需要限定：归因判据（`thevenin.rs:998`）意味着**纯 DC 源 + 极小 `max_step`** 这类“用户自选的步数预算”不再在 check 报错（F9 exit 0，符合设计意图）。该行紧接的 `:419` 已提醒“不要读成 check 能捕获全部运行时限制”，但读者可能仍把“步数预算”读成全覆盖。建议改为“**由声明波形导致的**步数预算 `E_LIMIT` 在 check 即报”。

### 8.3 另一项遗留风险（非本轮引入，供 lead 记录）

F9 的设计选择是“用户显式 `max_step` 造成的步数预算不属于本契约”（`thevenin.rs:987-994`）。后果：`stop: 1.s, max_step: 1.ns`（约 1e9 步）**通过 check**，我没有运行它（会极慢）。是否存在运行期保护未验证。这是**设计取舍**而非我发现的缺陷，但值得在文档里与 `Locals`/资源边界一起说明。

---

## 9. PASS / NEEDS_FIX / BLOCKED

**PASS**（数值层面，rev3 口径）

1. 参考解独立重推：4 条路线互检 ≤ 1.7e-13 V，比 §17 的 `atol` 小 8 个数量级；与被测方参考解同量级（§3）。
2. Task B 误差表 **6/6 行零差异**（点数、`max|err|`、发生时间、违规数、worst ratio），253 条超限样本逐条一致（§4）。
3. §17 阈值**逐字一致**（`atol = 1e-5 V, rtol = 1e-3`），**无**更大阈值、**无** `#[ignore]`、**无** 跳过点、**无** 删除失败样本（§5）。
4. Task A 两不变式在**我自己的** session 路径与**另一组电路参数**上成立；输出网格契约 12/12 PASS（§6）。
5. 10 个证伪用例全部符合契约；含 lead 的 W6-2 反例（F9 exit 0）与归因改进（F10）（§7）。
6. 未发现实质过度声称；限制行与不单调性均被如实记录（§8.1）。

**NEEDS_FIX**：无（数值结论层面）。

**BLOCKED**：无。

**必须由 lead 处置的一项流程问题（不阻塞数值结论）——已于 21:15:03 闭环**：
- **rev2 清单在 21:10 起失效**：`thevenin.rs`、`elaborate.rs`、`resample.rs` 三个文件在 rev2 冻结后 7 分钟被改动；另有 w3 的 3 个文件在 21:11:45 被 rustfmt。我按**当前 rev3 内容**完成全部复核，并逐项验证 rev2→rev3 **不改变本报告任何数值**（§1.4）。Lead 已于 **21:15:03 发布 rev3 权威清单**，我逐行重算 **17/17 一致（16 文件 + 1 二进制）**，见 §1.5 ⇒ **该项流程问题已闭环，rev3 终检 PASS**。

---

## 10. 未验证项（明确边界）

1. **`cargo test --workspace` 全量未复跑**（只跑了 5 个指定 target + `circuit-results` 全 crate，共 **118** 个测试 + 1 doctest，全绿）；`cargo clippy --workspace --all-targets -- -D warnings` 未复跑（lead 报 exit 0，我未独立验证）。
2. **F9 配置未实跑**（`stop: 1.s, max_step: 1.ns` ≈ 1e9 步，只跑了 `check`）：运行期是否有资源保护、要多久，**未验证**。
3. **通用性未验证**：所有误差结论绑定单一激励（`V0/T = 1e6 V/s`、`τ = 100 µs`、`T/τ = 0.01`）。多极点网络、不同 `T/τ`、电感/二极管/非线性电路**未验证**；`h_max ≤ 10·sqrt(2·atol·τ·T/V0)` 未在其他激励上检验。
4. **容差通道未独立验证**：`RELTOL/ABSTOL/TRTOL` 的“单因子不可观测、交互可观测”结论来自被测方 `breakpoint_study`（我复跑 exit 0、14/14），我**没有**独立写代码复算这三个容差实验。产品路径无容差通道（`options: Vec::new()`）我未逐行复核。
5. **`Data::Complex` 重采样的产品级路径未验证**：我只在单元层用自建 `Dataset` 验证了分量插值（`grid` 第 11 项）；`cdsl run` 不导出复数瞬态，故端到端未走通。
6. **REPL 路径未验证**（只验证了文件模式 CLI 与 session API）。
7. **`uic: true`、`pwl`/`sin` 波形、`start > 0` 与 `output_interval` 的组合**只测了 F4 一例；`sin`/`pwl` 的 `tran_step_for` 分支（`thevenin.rs:919-923` 明确 `continue`）未做数值验证。
8. **`_probe` 的 `probe` / `robustness` 两个 bin 未复跑**（我只跑了 `breakpoint_study` 与 `tran_contract`）。
9. ~~rev3 的最终清单未做比对~~ → **已完成**（§1.5，17/17 一致）。仍未验证的是**该清单之后是否还会有新的代码改动**：本报告的数值只对 rev3 冻结态有效；若再次解冻需重新做 §1.5 的核对与 §1.4 的产物对照。
10. **`docs/` 中我未逐字通读全文**：§8 的过度声称检查基于对含相关数值的章节与全仓库 grep（`tau/1000`、`4.4721`、`10·sqrt`、`223.6` 等 50 处命中）的逐一阅读，不是全文审校。

---

## 11. 我的产物清单（可复跑）

| 路径 | 内容 |
|---|---|
| `target/round2-evidence/w4/rustprobe/` | 我写的探针 crate（独立调用公共 API）；`cargo build --offline` |
| `target/round2-evidence/w4/refcheck.py` | 50 位 mpmath 参考解 + ODE 残差 + RK4 + DOP853 + 误差表复算 |
| `target/round2-evidence/w4/refcheck_dop853.py` | 逐断点重启的 DOP853 交叉验证 |
| `target/round2-evidence/w4/f1_interp.py` | F1 独立插值/解析对照 |
| `target/round2-evidence/w4/taskb_*.csv` | 6 组 `max_step` 的原始求解轨迹（我导出） |
| `target/round2-evidence/w4/probe-*.stdout.txt` | 三支探针的真实输出（含 rev2/rev3 前后对照） |
| `target/round2-evidence/w4/falsify/` | F1–F10 的 `.cdsl`、CSV、stdout/stderr（rev3 重跑） |
| `target/round2-evidence/w4/run-*.txt` | 各 `cargo test` 原始输出 |
| `target/round2-evidence/w4/refcheck.json` | 机器可读的复核结果 |
