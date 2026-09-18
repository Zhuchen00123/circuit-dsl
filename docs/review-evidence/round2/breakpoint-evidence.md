# W3 任务 B 证据报告：运行中的源断点（断点重启 BE、max_step、容差）

- 仓库 `F:\codexprojects\dsl000`，Windows / pwsh，thevenin 0.5.0（vendored）
- 任务：共享任务 `task-7`（W3 任务 B 断点精度回归与内核实验）
- 执行者：`w3-breakpoint`
- 结论：**PASS**（产品路径必过配置满足 §17；内核固有行为已定位并最小复现；容差通道结论已量化）
- 本轮未改动任何产品 `src/`、其他测试文件、`Cargo.toml`/`Cargo.lock`、`_probe/src/main.rs`、`_probe/src/bin/robustness.rs`、`examples/`；无 `#[ignore]`；未放宽 §17；未删除任何超限样本；未 commit。

---

## 1. 写入文件与哈希（rev3：已过 `cargo fmt` 门禁）

| 文件 | 行数 | SHA256（rev3） | 说明 |
|---|---|---|---|
| `crates/circuit-backend/tests/source_breakpoint_regression.rs`（新） | 1126 | `2F1BDAF499973BE7515863345E8220CBCC56F00BEE2B257B11C3ADCCE0DC009B` | 产品路径回归（本文件唯一写入者） |
| `_probe/src/bin/breakpoint_study.rs`（新） | 1485 | `1197516B7C9E9E69F8FF4DB0BA926726091B1EAD9F572960EBCBBD7DE37BD485` | 直接驱动 thevenin 的内核/容差实验 |
| `_probe/src/bin/tran_contract.rs`（新） | 763 | `9596B2485A9A66564A184354B1600A4A2FC5254091741B6C3102079799AE0363` | 底层 `tstep` 钳位钉住（承接被 W1 重写的旧断言） |
| `docs/review-evidence/round2/breakpoint-evidence.md`（新） | — | — | 本报告 |
| `target/round2-evidence/w3/*.txt` | — | — | 原始 stdout/stderr 证据（`target/` 不入版本管理） |

rev2（未格式化）哈希 → rev3（格式化后）哈希的对应关系、格式化门禁的真实命令/退出码，以及“行为逐位不变”的证据见文末 **§13 rev3 格式化返修**。

`git status --porcelain` 中属于本轮的新增仅上述 3 个源文件；`_probe/Cargo.lock` 哈希在实验前后一致（`B00E4B6CA83AD439…`），产品 `Cargo.lock` 未触碰。

原始证据文件（本次运行的真实输出）：
- `target/round2-evidence/w3/product_test.stdout.txt` / `.stderr.txt`（产品测试）
- `target/round2-evidence/w3/product_test_full_dump.stdout.txt`（`CDSL_BP_DUMP_ALL=1`，含全部 253 条超限样本）
- `target/round2-evidence/w3/breakpoint_study.stdout.txt` / `.stderr.txt`
- `target/round2-evidence/w3/tran_contract.stdout.txt` / `.stderr.txt`

---

## 2. 真实命令与退出码

| # | 命令 | 退出码 | 结果 |
|---|---|---|---|
| C1 | `cargo test -p circuit-backend --test source_breakpoint_regression` | **0** | 6 passed / 0 failed / 0 ignored（0.37 s） |
| C2 | `cargo test -p circuit-backend --test source_breakpoint_regression -- --nocapture --test-threads=1` | **0** | 同上，含全部打印 |
| C3 | `$env:CDSL_BP_DUMP_ALL=1; cargo test -p circuit-backend --test source_breakpoint_regression -- --nocapture --test-threads=1` | **0** | 输出 542 行，含 253 条超限样本明细（τ/50 第 6–258 行，τ/200 第 259–261 行） |
| C4 | `cargo run --manifest-path _probe/Cargo.toml --bin breakpoint_study` | **0** | 14/14 contract checks PASS；2 条 §17 NOT-MET 行保留（不翻转退出码，见 §5 策略） |
| C5 | `cargo run --manifest-path _probe/Cargo.toml --bin tran_contract` | **0** | 12/12 contract pins PASS |

两个 probe bin 的退出码语义（写死并在 stdout 声明）：
- `0`：全部实验跑通，且全部**契约检查**（`[PASS]`/`[FAIL]`，即“机制类”断言：重启步长公式、BE 阶、钳位公式、容差不可动 h1 等）通过；§17 的 `[NOT-MET]` 行原样打印并汇总，**不**翻转退出码（内核实验里 NOT-MET 是待保留的发现；§17 验收在 C1/C2 的产品测试里）。
- `1`：任一契约检查失败，或实验 panic（`catch_unwind` 把 panic 映射为 1，而不是默认 101）。

---

## 3. 参考解自检（产品测试 `reference_matches_independent_checks`）

固定问题：`R=1kΩ`、`C=100nF`（`τ=100 µs`），PULSE `delay=100 µs`、`rise=fall=1 µs`、`width=5 µs`、`period=20 µs`，`stop=300 µs`（10 个脉冲、40 个断点）。产品适配层给出的 `.tran` print step = `min(span/1000, min(rise,fall,period)) = min(300 ns, 1 µs) = 300 ns` ⇒ 声明边沿**不被**引擎钳位（`tr_used = 1 µs`）。

分段解析解用**无消去形式** `y = v0 + m·τ·S(x) + (y_start − v0)·e^{−x}`（`S(x)=x+expm1(−x)`，`x<1e-2` 走级数），三条独立路线互相验证：

| 自检 | 数值 | 判据 |
|---|---|---|
| ODE 残差 `max|τ·y′ + y − v_in|`（解析导数） | **5.773e-15 V** | < 1e-12 |
| Green 函数/卷积路径 vs 分段递推（独立代数路线） | **2.687e-14 V** | < 1e-12 |
| RK4（h=10 ns，30001 步，独立 PULSE 求值器驱动） | **6.706e-14 V** | < 1e-12 |
| 分段输入 `v_in` vs 引擎波形副本 | **6.450e-14 V** | < 1e-12 |
| 断点两侧分支连续性 `|y(t0−1e-18) − y(t0)|` | **9.978e-15 V** | < 1e-12 |
| 级数/expm1 形式互校 `|m·τ·S(x) − (m·u−m·τ+m·τ·e^{−x})|` | 1.892e-14 V（probe） | 参考 |

即参考解精度在 **1e-14 V 量级**，远优于所要求的 1e-12 量级。

---

## 4. 产品路径数值表（C1/C2/C3）

列：`max_step` / 返回点数 / `max|err|` / 发生时间 / 超限点数 / 判定。判定列中的 `MET` 行是**逐点断言**（`cargo test` 会失败），`LIMITATION` 行是保留的限制/诊断行（见 §6）。

| max_step | h_max | 点数 | max\|err\| [V] | 发生时间 [s] | 超限点数 | 判定 |
|---|---|---|---|---|---|---|
| `τ/1000 = 1.0e-7`（必过配置） | 1.0e-7 | 3129 | 4.999167e-7 | 1.000100e-4 | **0** | MET（逐点断言） |
| `τ/500 = 2.0e-7` | 2.0e-7 | 1629 | 1.999333e-6 | 1.000200e-4 | **0** | MET（逐点断言） |
| `τ/200 = 5.0e-7` | 5.0e-7 | 729 | 1.248959e-5 | 1.000500e-4 | **3** | LIMITATION（保留） |
| `τ/50 = 2.0e-6` | 2.0e-6 | 309 | 7.331775e-4 | 2.802000e-4 | **250** | LIMITATION（保留） |

其它实测事实：
- 时间轴 40 个断点**全部精确落在返回轴上**：`max|t_sample − t_breakpoint| = 0.000e0 s`（第 2 节测试，含 `delay`、`delay+rise`、`delay+rise+width`、`delay+rise+width+fall` 及第 2 周期同名四点）。
- 适配层元数据断言：`tran.solver_step = 3e-7 s`、`tran.waveform_bound = 1e-6 s`，且 `solver_step ≤ waveform_bound` ⇒ 声明 1 µs 边沿不被拉宽——这是 `tran_contract`（C5）钉住的引擎事实在产品层的对应断言。
- `output_interval` 不进入求解器：`output_interval = 1 µs` 与 `20 ns` 两次运行的原始网格**逐点相同**（3129 点、轴与值完全一致）。这同时覆盖了 Task A 的语义变更。

### 4.1 必过配置的依据（测试中断言，不是散文）

断点重启被强制 Backward-Euler（`transient.rs:1433 / :1464-1468`），重启步长 `h1 = min(step_h, h·0.1).max(h_min) ≤ 0.1·h_max`（`:1443`）。静息斜坡起点处的 BE 局部误差为

```
err_restart = (V0/T) · h1² / (2τ)          ⇒ 要求 h_max ≤ 10·sqrt(2·atol·τ·T/V0)
```

本激励 `V0/T = 1e6 V/s`、`τ = 100 µs`、`atol = 1e-5 V`：

```
bound = 10·sqrt(2·1e-5·1e-4·1e-6/1) = 4.472136e-7 s = τ/223.6
```

`max_step = τ/1000 = 1.0e-7 s` 比该界低 **4.47 倍**（测试打印 `margin 4.47x` 并断言 `max_step < bound`）；`τ/500` 亦在界内并同样逐点断言。**该界含 `V0/T` 与 `τ` 两个因子，只对本激励成立**；换边沿速率或时间常数必须重算，不能把“τ/1000”当成通用保证（文件头与测试注释均已写明）。

### 4.2 必过配置下的 BE 重启实测（产品路径）

首个断点（`t = delay = 1e-4 s`）处：`h_before = 7.225e-8 s`，`h1 = 1.0e-8 s = min(2·h_before, h_max)·0.1`（相对差 5.2e-13）；该步末：

| 量 | 值 |
|---|---|
| 引擎 `v(out)` | 9.99900010e-7 V |
| Backward-Euler 单步闭式 | 9.99900010e-7 V（`|engine − BE| = 5.175e-19 V`） |
| 梯形单步闭式 | 4.99975001e-7 V（`|engine − TRAP| = 4.999e-7 V`） |
| 解析解 | 4.99983334e-7 V |
| 实测误差 | 4.999167e-7 V |
| 预测 `(V0/T)·h1²/(2τ)` | 5.000000e-7 V（比值 **0.9998**） |

---

## 5. 失败（超限）配置清单——原样保留

### 5.1 `τ/200 = 5.0e-7 s`（729 点，3 点超限）

```
t=1.000500000e-4 s: |err|=1.248959e-5 V > allowance=1.001250e-5 V (expected=0.000012498 V, actual=0.000024988 V, ratio=1.247x)
t=1.001500000e-4 s: |err|=1.246879e-5 V > allowance=1.011244e-5 V (expected=0.000112444 V, actual=0.000124913 V, ratio=1.233x)
t=1.003500000e-4 s: |err|=1.237744e-5 V > allowance=1.061179e-5 V (expected=0.000611786 V, actual=0.000624163 V, ratio=1.166x)
```

（与上一轮 `3/1217` 同量级、同位置特征：全部落在**首个上升沿起步后的前几步**。）

### 5.2 `τ/50 = 2.0e-6 s`（309 点，250 点超限）

全部 250 条见 `target/round2-evidence/w3/product_test_full_dump.stdout.txt` 第 **6–258** 行（`CDSL_BP_DUMP_ALL=1` + `--test-threads=1` 运行，输出顺序确定；τ/200 的 3 条在其后第 259–261 行，共 253 条）。首 3 条：

```
t=1.002000000e-4 s: |err|=1.993349e-4 V > allowance=1.019987e-5 V (expected=0.000199867 V, actual=0.000399202 V, ratio=19.543x)
t=1.006000000e-4 s: |err|=1.980090e-4 V > allowance=1.179641e-5 V (expected=0.001796405 V, actual=0.001994414 V, ratio=16.786x)
t=1.010000000e-4 s: |err|=1.966905e-4 V > allowance=1.498337e-5 V (expected=0.004983375 V, actual=0.005180065 V, ratio=13.127x)
```

末条（`t=3.000000000e-4 s`，比值 2.190x）。此配置 `h_max = 2 µs > rise = 1 µs`，上升沿本身被欠解析，误差覆盖整个窗口——因此它既含 BE 重启误差也含粗网格积分误差，二者在报告里不混为一谈。

保留方式：`max_step_trend_table_marks_passing_and_limitation_rows`（表内 `LIMITATION` 标签 + `[LIMITATION-DIAGNOSTIC]` 打印）与独立测试 `limitation_coarse_max_step_forced_backward_euler_restart`（characterization pin：断言 `h1 = min(2·h_before,h_max)·0.1`、`err/((V0/T)h1²/(2τ)) ∈ [0.5,2]`、`violations > 0`）。若未来内核修复导致第三条断言失败，注释明确要求“更新该 pin 与报告，不得删除样本”。

---

## 6. 是否内核固有行为：结论与最小复现

**结论：是。** 直接驱动 thevenin 0.5.0（不经产品适配层）的 `breakpoint_study`（C4，14/14 契约检查通过，exit 0）给出：

1. **重启步长公式成立（12/12）**：12 个断点（前 3 个脉冲 × 4）的首个接受步 `h1` 全部等于 `min(2·h_before, h_max)·0.1`（相对差 ≤ 1e-9），其中 `h_before` 是落在断点上的那一步（被断点距离钳位）。
2. **阶为 Backward-Euler（12/12）**：同一批 12 步的返回值与 BE 单步闭式之差最大 **1.110e-16 V**，而与梯形单步闭式之差为 4.5e-9…5.0e-7 V；`|engine − BE| < |engine − TRAP|` 在 12/12 行成立。
3. **误差定律**：首个静息斜坡起点实测误差 4.999167e-7 V vs 预测 5.000000e-7 V（比值 0.9998）。
4. **容差不可动 h1**：见 §7.2。

### 最小复现（`breakpoint_study` 第 2 节，配置 `step=300 ns`、`tmax=τ/1000`）

| t [s] | h [s] | v(in) [V] | v(out) [V] | 解析 [V] | \|err\| [V] | allowance [V] |
|---|---|---|---|---|---|---|
| 9.982775000000159e-5 | 1.0e-7 | 0.0 | 0.0 | 0.0 | 0 | 1.0e-5 |
| 1.000000000000000e-4 | 7.225e-8 | 0.0 | 0.0 | 0.0 | 0 | 1.0e-5 |
| **1.000100000000000e-4** | **1.0e-8** | 0.01 | **9.999000e-7** | **5.000000e-7** | **4.999e-7** | 1.0e-5 |
| 1.000300000000000e-4 | 2.0e-8 | 0.03 | 4.999300e-6 | 4.5e-6 | 4.998e-7 | 1.0e-5 |
| 1.000700000000000e-4 | 4.0e-8 | 0.07 | 2.4993302e-5 | 2.4494e-5 | 4.990e-7 | 1.002e-5 |
| 1.001500000000000e-4 | 8.0e-8 | 0.15 | 1.12938129e-4 | 1.12444e-4 | 4.944e-7 | 1.011e-5 |

即：**引擎在断点起步的那一步返回了 BE 的结果（约等于精确值的 2 倍），随后梯形以约 5e-7 V 的固定偏置继续**——这就是历史“源断点后首个接受步被强制 BE”的机制，且与 `h_max` 成平方关系，与用户可见的 `§17` 允许量（不随 `h_max` 变化）无关。

---

## 7. `max_step` 扫描与容差实验（`breakpoint_study`，exit 0）

### 7.1 max_step 扫描（`.tran step = 300 ns` 固定；`tmax` 是唯一变量）

| tmax | h_max [s] | 点数 | h1(首个断点) [s] | 预测 h1 [s] | max\|err\| [V] | 位置 u/τ | 预测 BE 误差 [V] | 超限 |
|---|---|---|---|---|---|---|---|---|
| τ/50 | 2.0e-6 | 309 | 2.0e-7 | 2.0e-7 | 7.3318e-4 | 1.8020 | 2.0e-4 | **250** |
| τ/200 | 5.0e-7 | 729 | 5.0e-8 | 5.0e-8 | 1.2490e-5 | 0.0005 | 1.25e-5 | **3** |
| τ/500 | 2.0e-7 | 1629 | 2.0e-8 | 2.0e-8 | 1.9993e-6 | 0.0002 | 2.0e-6 | 0 |
| τ/1000 | 1.0e-7 | 3129 | 1.0e-8 | 1.0e-8 | 4.9992e-7 | 0.0001 | 5.0e-7 | 0 |
| τ/5000 | 2.0e-8 | 15129 | 2.0e-9 | 2.0e-9 | 1.9999e-8 | ~0 | 2.0e-8 | 0 |
| `None`（适配层默认） | 3.0e-7 | 1156 | 3.35e-9 | 3.35e-9 | 1.6571e-5 | 1.8003 | 5.6e-8 | 0 |

- 点数随 `h_max` 减小**单调不减**（309 → 729 → 1156 → 1629 → 3129 → 15129，已按 `h_max` 降序重排；测试内断言）。
- `max|err|` **不**单调（`None` 行 1.6571e-5 V 大于 τ/500 的 1.9993e-6 V），因为峰值位置随网格移动且 `§17` 允许量随 `|expected|` 增大；报告与测试都明确写“看超限数而不是 max|err|”，不做虚假单调性断言。
- 由 `h1 ≤ 0.1·h_max` 推出的界 `h_max ≤ 4.4721e-7 s (= τ/223.6)` **干净地**把超限与不超限分开（测试断言 `above → violations>0`、`at-or-below → violations==0`）：τ/200 (5e-7) 在界之上 ⇒ 3 点超限；τ/500 (2e-7) 在界之下 ⇒ 0。
- 注意 `h1` 并非总是 `0.1·h_max`：落在断点上的那一步被断点距离钳位，`h1 = min(2·h_before, h_max)·0.1`。例如 `tmax=None` 时 `h_before = 16.75 ns`（100 µs 的整除残差），故 `h1 = 3.35 ns` 而不是 `0.1·h_max = 30 ns`。表内两列同时打印，避免误读。

### 7.2 reltol / abstol / trtol（单因子与交互）

| 组 | 配置 | 点数 | h1(首断点) [s] | max\|err\| [V] | 超限 | 与空 options 逐字段相同 |
|---|---|---|---|---|---|---|
| P（`tmax=τ/1000` 钉住 h_max） | 空 | 3129 | 1.0e-8 | 4.9992e-7 | 0 | baseline |
| P | `RELTOL=1e-12` | 3129 | 1.0e-8 | 4.9992e-7 | 0 | 是 |
| P | `ABSTOL=1e-15` | 3129 | 1.0e-8 | 4.9992e-7 | 0 | 是 |
| P | `TRTOL=0.7` | 3129 | 1.0e-8 | 4.9992e-7 | 0 | 是 |
| P | `RELTOL=1e-12 + ABSTOL=1e-15` | 3168 | 1.0e-8 | 9.4120e-7 | 0 | **否**（首个轴分歧 index 1010） |
| F（`tmax=None`，h_max=300 ns） | 空 | 1156 | 3.35e-9 | 1.6571e-5 | 0 | baseline |
| F | `RELTOL=1e-12` / `ABSTOL=1e-15` / `TRTOL=0.7` | 1156 | 3.35e-9 | 1.6571e-5 | 0 | 是（三个单因子一致） |
| F | `RELTOL+ABSTOL` | 1273 | 3.35e-9 | 7.6682e-6 | 0 | **否**（首个轴分歧 index 346） |

结论（已如实报告，未夸大）：
- **单因子不可观测**：`RELTOL`、`ABSTOL`、`TRTOL` 单独变化在两组的轨迹都与空 options **逐字段相同**（点的值、轴全部 `==`）。
- **交互可观测但无用**：`RELTOL+ABSTOL` 同时收紧确实改变了轨迹（P 组 3129→3168，F 组 1156→1273），但**重启步长 `h1` 在 5 种容差设置下完全不变**（P 组 1.0e-8 s；F 组 3.35e-9 s），`§17` 结论也不变。也就是说：容差通道**不是**该缺陷的实验变量——重启步由 `min(2·h_before, h_max)·0.1` 决定，`h_max` 被 `tmax`/`h_print` 钉住时 LTE 无法把它压下来。
- 产品路径更彻底：`build_circuit` 传空 `options`，`RELTOL/ABSTOL/TRTOL` 根本不可达（本报告不声称产品路径设置了任何容差）。

### 7.3 唯一有效的杠杆：`h_print`（`tmax=None` 时 `h_max = min(h_print, stop/50)`）

| h_print [s] | h_max [s] | 点数 | h1(首断点) [s] | max\|err\| [V] | 超限 |
|---|---|---|---|---|---|
| 3.0e-7（Task A 适配层默认） | 3.0e-7 | 1156 | 3.35e-9 | 1.6571e-5 | 0 |
| 1.0e-7 | 1.0e-7 | 3129 | 1.0e-8 | 4.9992e-7 | 0 |
| 3.0e-8 | 3.0e-8 | 10156 | 3.35e-10 | 1.6684e-7 | 0 |
| 1.0e-8 | 1.0e-8 | 30129 | 1.0e-9 | 4.9999e-9 | 0 |

---

## 8. `tran_contract`：底层钳位事实（C5，12/12 PASS，exit 0）

被测事实：`waveform.rs:37-38` `tr_used = tr.unwrap_or(tstep).max(tstep)`，`tstep = TranAnalysis::step`；边沿宽度由**返回时间轴上的 `v(in)`** 用最小二乘拟合（边沿内样本）反推，不假设网格。

| 用例 | `.tran step` | 声明 tr/tf | `tmax` | 边沿内样本 | 反推上升宽度 [s] | 反推下降宽度 [s] | 拟合 rms 残差 [V] | 判定 |
|---|---|---|---|---|---|---|---|---|
| A | 100 ns | 10 ns | None | 4 (+4) | **1.000000000e-7** | **1.000000000e-7** | 8.695e-16 / 3.149e-15 | 被钳位（= step） |
| A2 | 100 ns | 10 ns | `1 ns` | 102 (+102) | **1.000000000e-7** | **1.000000000e-7** | 1.563e-15 / 2.432e-15 | 与 A 相对差 1.323e-16（`tmax` 不参与钳位） |
| B | 1 ns | 10 ns | None | 12 (+12) | **1.000000000e-8** | **1.000000000e-8** | 3.934e-15 / 8.820e-15 | 声明值保留（`step ≤ tr`） |
| C | 10 ns | 1 µs | None | 102 (+102) | **1.000000000e-6** | **1.000000000e-6** | 2.950e-16 / 2.948e-16 | 声明值保留（对照） |
| D | 500 ns | 1 ps | `τ/1000` | 15 | **5.000000000e-7** | —（下降沿在窗外，明确标注 SKIPPED） | **4.059e-17** | 历史配置复现：1 ps 被执行为 500 ns |

D 的旁证：同一输出对理想阶跃模型的偏差达 **2.491750e-3 V**，与解析 `|C(T_eff)| = 2.504172e-3 V`（`T_eff = max(1 ps, 500 ns)`）吻合（5% 内）——即历史 1.95e-3 V 量级残差确实来自“声明边沿被执行成 500 ns 斜坡”，与旧产品测试
`transient_reference_regression.rs::declared_rise_below_output_interval_is_clamped_to_the_output_step` 的结论一致。该产品测试文件与 `_probe/src/main.rs`、`_probe/src/bin/robustness.rs` **均未被本轮修改**；该断言以独立工程形式在此承接。

补充：产品路径现在**无法**触发该钳位——适配层取 `h_print = min(span/1000, min(rise,fall,period)) ≤ rise/fall`，因此 `max(tr, tstep) = tr`。本 bin 记录的是引擎规则本身，不是当前适配层映射。

---

## 9. 最小适配评估（不改内核源码）

现状（实测）：用户显式给 `max_step` 时，只要 `max_step > 10·sqrt(2·atol·τ·T/V0)`（本激励 τ/223.6 ≈ 447 ns），断点重启步就会把 §17 顶穿；`max_step ≤ τ/500` 则干净通过。`tmax` 缺省时适配层自己算出的 `h_max = min(h_print, stop/50) = span/1000`（本激励 300 ns）已经安全，并额外因“断点距离残差”效应把 `h1` 压到 3.35 ns。

按代价从低到高给出三个方案（本轮**均未实施**，因禁止改产品 src/ 与内核）：

1. **零代码：文档化边界（推荐先做）**。在 `docs/backend-evaluation.md`/`docs/language.md` 写明“运行中源断点的重启步 = `min(2·h_before, h_max)·0.1`，静息斜坡起点误差 ≈ `(V0/T)·h1²/(2τ)`；建议 `max_step ≤ 10·sqrt(2·atol·τ·T/V0)`，或直接省略 `max_step` 让适配层取 `span/1000`”。依据：本报告 §4/§7 的实测分离（τ/200 超限、τ/500 通过、界 4.4721e-7 s）。
2. **适配层校验（最小代码改动，无需内核）**：在 `thevenin.rs::tran_step_for`/`validate` 里，用已声明的边沿速率 `V0/T`（可由 PULSE `high-low`/`rise` 算）与电路里的最小 `τ`（可由 RC 网络或用 `span/1000` 保守替代）检查用户 `max_step`，超出界时给出**能力错误**（`E_LIMIT`/`E_VALUE`）并在 note 里给出建议值——与 Task A “宁可拒绝也不静默改波形”的既定哲学一致。代价：需要从 IR 估计 `V0/T` 与 `τ`，对一般电路只能保守（例如用 `min(rise, fall)` 与反射最慢极点），会误拒一部分合法输入。
3. **上游补丁（真正的修复，需改 vendored/上游源码，本轮禁止）**：`transient.rs:1433` 的 `is_at_breakpoint` 只看步起点，对“斜率折点”（PULSE 的 `td`、`td+tr`、`td+tr+pw`、`td+tr+pw+tf` 中值连续的四类）与“数值跳变”一视同仁地强制 BE + 0.1h。候选改法：
   - (a) 仅对**值不连续**的断点强制 BE；对斜率折点保留原阶（但需处理 BE→Trap 的历史一致性）；
   - (b) 保留 BE 但把重启步长改为由容差推导的上界（例如同时受 `sqrt(2·atol·... )` 类的量约束），而不是固定的 `h*0.1`；
   - (c) 在断点后做一次“重起步”：BE 一步后立即用二阶公式重算同一时间点（ngspice 的 `dctran` 用 2–5 个 BE 步过渡，本引擎的 `force_be = trap_h <= 1.05*step_h` 只做到“不过早升级”）。
   量化收益（若采纳 b/a）：本激励 τ/200 与 τ/50 的超限点可消失；代价是断点附近需要更多步（`τ/50` 行若把重启步压到 ≈ 40 ns，点数会显著上升）。**本轮只在报告中评估，未改一行内核代码。**

---

## 10. PASS / NEEDS_FIX / BLOCKED

- **PASS**（可复核）
  - 产品路径：`cargo test -p circuit-backend --test source_breakpoint_regression` → exit 0，6/6；必过配置（`max_step = τ/1000`，且 `τ/500` 亦验证）在 3129/1629 个返回点上逐点满足 §17（`atol=1e-5 V, rtol=1e-3`，未放宽）。
  - 必过依据已在测试内断言（`max_step < 10·sqrt(2·atol·τ·T/V0)`、h1 公式、BE 误差定律、适配层 `tran.solver_step ≤ tran.waveform_bound`）。
  - 参考解三条独立路线互检 ≤ 6.7e-14 V（要求 ≤ 1e-12）。
  - 断点落在返回轴上（40/40，偏差 0.0 s）。
  - 超限配置（τ/50、τ/200）以独立测试 + `LIMITATION` 标签保留，全部 253 条样本落盘。
  - 内核侧：14/14 契约检查 + 12/12 重启步公式/BE 阶；结论“内核固有行为”有行号级机制 + 数值双证据。
  - 容差通道结论：单因子不可观测、交互可观测但不改变 `h1`/§17 结论——均给出数值。
- **NEEDS_FIX**：无（本轮范围内）。
- **BLOCKED**：无。
- 需要 Lead 注意的**遗留风险**（非本轮可解）：`max_step` 过大时产品路径仍会静默产生超 §17 的结果（τ/200 仅超 1.25x，属“临界”；τ/50 严重超限）。是否在适配层加校验取决于任务边界（本轮禁止改产品 src/）。

---

## 11. 未验证项（明确不做过度声称）

1. **单 RC 之外的一般性**：所有必过依据都绑定本激励（`V0/T = 1e6 V/s`、`τ = 100 µs`、`T/τ = 0.01`）。多极点网络、不同 `T/τ`、电感/二极管电路未验证；“τ/1000 安全”不能外推。
2. **`_probe` 的容差实验不覆盖产品路径**：产品适配层无容差通道，故 §7.2 只说明“引擎的容差旋钮对该缺陷无用”，不构成产品行为声明。
3. **仅 BE 分支的阶判定**：`tran_contract`/`breakpoint_study` 的 BE/TRAP 单步闭式对比是针对线性 RC + 分段线性输入的精确刻画；非线性电路（二极管、BJT）不受此证据覆盖。
4. **未测 `uic=true`、`start_s ≠ 0`、`max_step` 缺省 + `output_interval` 极小**等组合；`output_interval` 只验证了后端原始网格不变（重采样层由 Task A 的测试覆盖，本文件不重复）。
5. **`h_min` 与 `min_break = tstep*5e-5` 的边界**（例如 `period` 接近 300 ns 的极窄脉冲、`delay` 与 `period` 使断点间距 < `min_break`）未做实验；本轮断点间距最小 1 µs ≫ 1.5e-11 s。
6. **未在上游 thevenin 仓库提交 issue/PR**（本轮只做方案评估，未改内核）。
7. **未做多线程/并发下的复现**（测试默认并行执行已通过，但未做压力验证）。

---

## 12. 机制行号索引（vendored `thevenin-0.5.0`，供复核）

| 行号 | 内容 |
|---|---|
| `src/transient.rs:308` | `min_break = tstep * 5e-5` |
| `src/transient.rs:318-326` | `next_after`：跳过 `<= t + min_break` 的断点 |
| `src/transient.rs:350-357` | `is_at_breakpoint(t)`：用**步起点**判断 |
| `src/transient.rs:799` | `h_max = tmax.unwrap_or(min(tstep, tstop/50))` |
| `src/transient.rs:1407-1408` | `h_min = tstep*1e-9`；`h` 初值 `h_max/400` |
| `src/transient.rs:1425-1444` | `step_h = h.min(h_max)`；夹到断点距离；断点处 `step_h.min(h*0.1).max(h_min)` |
| `src/transient.rs:1464-1468` | `is_first_step \|\| at_breakpoint \|\| force_be` ⇒ BackwardEuler |
| `src/transient.rs:1620-1655` | 梯形分支的 LTE 接受/拒绝与 `h` 更新（含 `min(h_max)`） |
| `src/transient.rs:1656-1681` | BE 分支的阶升级检查 `force_be = trap_h <= 1.05*step_h` |
| `src/transient.rs:2271-2285, 2400` | 每条被接受内步都被记录（无输出抽样） |
| `src/waveform.rs:37-38` | `tr_used/tf_used = ...unwrap_or(tstep).max(tstep)` |
| `src/waveform.rs:116-156` | PULSE 求值（线性斜坡 + 周期折叠） |
| `src/waveform.rs:257-302` | 断点表 `td + k·per + {0, tr, tr+pw, tr+pw+tf}` |
| `crates/circuit-backend/src/thevenin.rs::tran_step_for` | `h_print = min(span/1000, min(rise, fall, period))`；`output_interval` 不进求解器 |

---

## 13. rev3 格式化返修（格式门禁）

### 11.1 触发与修复动作

Lead 报告：`cargo fmt --all -- --check` 与 `cargo fmt --manifest-path _probe/Cargo.toml -- --check` 均 **exit 1**，原因是本轮 3 个新文件未格式化（产品文件 5 处 diff、probe 多处、`tran_contract` 2 处）。全部 diff 均为**纯换行/缩进**（长 `use`、长 `println!`、长元组臂、闭包签名缩进），无一处改动表达式或字面量。

只对**本轮的 3 个文件**执行（未使用 `cargo fmt -p circuit-backend`，以免波及他人文件）：

```
rustfmt --edition 2024 crates/circuit-backend/tests/source_breakpoint_regression.rs   # exit 0
rustfmt --edition 2024 _probe/src/bin/breakpoint_study.rs                              # exit 0
rustfmt --edition 2024 _probe/src/bin/tran_contract.rs                                 # exit 0
```

### 11.2 哈希对照（旧 = rev2 未格式化，新 = rev3 交付件）

| 文件 | rev2 SHA256 | rev3 SHA256 | 行数 1122→1126 / 1412→1485 / 760→763 |
|---|---|---|---|
| `crates/circuit-backend/tests/source_breakpoint_regression.rs` | `F8C9F390199112B0559898C2E94F61A6376D3BBE0A3301E1FD412918425BBA06` | `2F1BDAF499973BE7515863345E8220CBCC56F00BEE2B257B11C3ADCCE0DC009B` | 1122 → **1126** |
| `_probe/src/bin/breakpoint_study.rs` | `70F2EE42D765941544BB0700C0ED20ADD51AD4897964ADA7BE6C5C07A1606A6E` | `1197516B7C9E9E69F8FF4DB0BA926726091B1EAD9F572960EBCBBD7DE37BD485` | 1412 → **1485** |
| `_probe/src/bin/tran_contract.rs` | `CB6890BC5C2EED701BC7F7BC7A7C1FB242C64E3EE70FEE818AC32B2380A93053` | `9596B2485A9A66564A184354B1600A4A2FC5254091741B6C3102079799AE0363` | 760 → **763** |

### 11.3 门禁命令与真实退出码（rev3）

| 命令 | 退出码 |
|---|---|
| `rustfmt --edition 2024 --check <三个文件>`（逐个） | **0 / 0 / 0** |
| `cargo fmt --all -- --check` | **0** |
| `cargo fmt --manifest-path _probe/Cargo.toml -- --check` | **0** |
| `cargo test -p circuit-backend --test source_breakpoint_regression` | **0**（6 passed / 0 failed / 0 ignored） |
| `cargo test ... -- --nocapture --test-threads=1` | **0** |
| `$env:CDSL_BP_DUMP_ALL=1; cargo test ... -- --nocapture --test-threads=1` | **0**（542 行） |
| `cargo run --manifest-path _probe/Cargo.toml --bin breakpoint_study` | **0**（14/14 contract checks；2 条 NOT-MET 保留） |
| `cargo run --manifest-path _probe/Cargo.toml --bin tran_contract` | **0**（12/12 pins） |
| `cargo clippy -p circuit-backend --test source_breakpoint_regression` | 无任何指向本文件的告警 |

### 11.4 行为不变的证据（逐位比对）

格式化前的 stdout 已留存为 `*.prefmt.txt`，格式化后重跑并做**字节级**比较（`target/round2-evidence/w3/`）：

| 输出 | 格式化前 SHA256(前16) | 格式化后 SHA256(前16) | 结论 |
|---|---|---|---|
| `breakpoint_study.stdout.txt` | `01CCAED6B048A9CB` | `01CCAED6B048A9CB` | **完全相同**（全文逐字节一致） |
| `tran_contract.stdout.txt` | `AB201523A71E6F63` | `AB201523A71E6F63` | **完全相同**（全文逐字节一致） |
| `product_test_full_dump.stdout.txt` | `E97A1D833F51EBBA` | `D65AA951E3EABE6D` | 仅差 cargo 测试框架的墙钟行 `test result: … finished in 1.30s`（1.22s）；剔除该行后 **544/544 行完全一致**（差异 0 行） |

因此格式化**不改变任何数值**，§4–§8 的全部数值表（3129/1629/729/309 点、`max|err|` 4.999167e-7 / 1.999333e-6 / 1.248959e-5 / 7.331775e-4 V、超限 0/0/3/250；probe 的 14/14 与 NOT-MET 250+3；`tran_contract` 的 12/12 与 1.000000000e-7 / 1.0e-8 / 1.0e-6 / 5.000000000e-7 s）在 rev3 逐位复现，无需修订。

### 11.5 约束遵守
- 只改了本轮 3 个源文件 + 本报告；未动他人文件（未用 `cargo fmt -p …`/`--all` 的写模式，只用 `--check`）。
- 未 `git commit/push/checkout/reset`；`_probe/Cargo.lock` 与产品 `Cargo.lock` 未变。
