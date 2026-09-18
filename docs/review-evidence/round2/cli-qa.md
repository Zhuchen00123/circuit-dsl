# W5 — 真实 CLI/REPL QA（修复后，任务 A 验收）

- 任务：共享任务 `task-8`（owner: `w5-cli-qa`）
- 日期：2026-09-18 21:00–21:25 (+08:00)
- 被测对象：工作树二进制 `target\debug\cdsl.exe`，**两轮**验证：
  - **pass-1**（rev1，mtime 20:56:24，SHA256 `551B8838E2AAAFD7D2E96FF95A24B33F05D1EECFFEFAFCA9C2A2936B43E6B6F6`）：60 条命令；
  - **pass-2 / rev2**（收到 lead 冻结通知后重新 `cargo build --bin cdsl`，mtime 21:03:30，SHA256 `8B0B40859424A68B813F683A1E004CC0C1103C1CDFF257F2816459ACC7E876EF`）：27 条命令复跑，**全部产物/诊断/REPL 转录与 pass-1 逐字节相同**（详见 §1.2）。
- 结论：**PASS（任务 A 的产品路径验收成立，rev2 复验通过）**，并附 **2 项 NEEDS_FIX（均为文档/示例注释陈旧，非产品行为缺陷）** 与 1 项口径澄清（粗网格插值在细边沿处的必然表现）。
- 全程**未修改任何源码 / 测试 / 文档 / `Cargo.toml` / `Cargo.lock`**；未 `git commit/push/checkout/reset`；只写入 `target\round2-evidence\w5\**` 与本报告；未写入 `target\round2-evidence\repro\`（仅只读引用）与 `target\round2-evidence\lead\`。

---

## 1. 证据来源与可复核锚点

```
$ git rev-parse HEAD
cb5d8a212f66922181580900a05fb3d42abe32f2
```

| 对象 | SHA256 | mtime |
|---|---|---|
| `target\debug\cdsl.exe`（rev2，最终验证对象） | `8B0B40859424A68B813F683A1E004CC0C1103C1CDFF257F2816459ACC7E876EF` | 2026-09-18 21:03:30 |
| `target\debug\cdsl.exe`（rev1，pass-1） | `551B8838E2AAAFD7D2E96FF95A24B33F05D1EECFFEFAFCA9C2A2936B43E6B6F6` | 2026-09-18 20:56:24 |
| `crates\circuit-backend\src\thevenin.rs`（rev2 权威） | `FCF3BD88E8BF24CE9BF3ADCCD64C805FAA56169AEC9FBB2E2158A5386FB3AFAC` | 2026-09-18 21:02:39 |
| `crates\circuit-backend\src\thevenin.rs`（rev1，pass-1） | `2D28F01E8FD7754DDEA8A5DDFAD202F61166E0CA68E21A7A83E7CDD6B24B0C87` | 2026-09-18 20:44:25 |
| `crates\circuit-dsl\src\elaborate.rs` | `61CA8FE8B45787EE7E9FA36A2E910BA0946D3187DDBF6FD88159EE6C1DAC2DCE` | 2026-09-18 20:44:17 |
| `crates\circuit-results\src\resample.rs` | `A726934F15B445BDF38C574427544F918984DD566377664F70873B5C7E10E363` | 2026-09-18 20:53:12 |
| `crates\circuit-session\src\execute.rs` | `9A0BD10393C419FF03870653ABA3D69357A77B50425E13DC97614A8425621F00` | 2026-09-18 20:43:40 |
| `crates\circuit-cli\src\run.rs` | `4B4D32A45536717872643AE82DBEA008AC43460F5261A2E9203FBC7ADDC78BD5` | 2026-09-18 20:43:40 |

rev2 源哈希与 `target\round2-logs\freeze-manifest.txt` 中 `FCF3BD88...  crates/circuit-backend/src/thevenin.rs` 及该文件末行 `rev2: re-frozen 21:11 (thevenin.rs comment-only update; other hashes unchanged)` 逐位一致；本轮只读取该 manifest，未修改。

工具链：`cargo 1.98.1 (797e8a9bc 2026-08-05)`、`rustc 1.98.1 (48ae229cea 2026-09-01)`。

**二进制新鲜度（真实构建确认，非推断）**：

```
pass-1（rev1）: $ cargo build --bin cdsl   → exit 0，Finished `dev` in 0.15s（cargo 判定 up-to-date，二进制未被重链接，mtime 保持 20:56:24）
pass-2（rev2）: $ cargo build --bin cdsl   → exit 0，Finished `dev` in 0.22s（thevenin.rs 于 21:02:39 变更后重新编译链接，二进制 mtime → 21:03:30，哈希 → 8B0B4085…）
末次确认    : $ cargo build --bin cdsl   → exit 0，Finished `dev` in 0.20s（up-to-date，mtime 仍 21:03:30，哈希仍 8B0B4085…）
```

- 每次构建均**未出现** `Blocking waiting for file lock on build directory`（未遇并发构建锁等待）；
- 唯一构建告警为既存 `#[warn(linker_messages)]`（`circuit-cli` bin，1 warning），与本轮修复无关；
- pass-1 的 60 条命令全部作用于 rev1 二进制（其 mtime 在 pass-1 期间恒为 20:56:24，可复核），pass-2 的 27 条命令全部作用于 rev2 二进制（mtime 21:03:30 且在 pass-2 后经 up-to-date 构建确认）。

### 1.1 前置确认（真实 `--help`，非猜测）

`cdsl` 子命令为 `check / run / capabilities / repl / help`；`run` 参数为 `[--experiment E] [--out DIR] [--format csv|json|both] [--verbose] <FILE>`（`--format` 默认 `both`）；`repl [--verbose] [FILE]` 从 stdin 读脚本。

### 1.2 rev2 复验（lead 冻结通知后）——行为不变性证明

lead 通知：`thevenin.rs` 仅注释改动、行为不变，权威哈希 `FCF3BD88…`，并要求重跑关键用例 + 记录二进制哈希。执行结果：

| 复验项 | 结果 |
|---|---|
| 源哈希核对 | `thevenin.rs` 实测 = `FCF3BD88E8BF24CE9BF3ADCCD64C805FAA56169AEC9FBB2E2158A5386FB3AFAC`，与通知/manifest **一致** |
| 重新构建 | `cargo build --bin cdsl` exit **0**，0.22s，**二进制确实被重链接**（哈希 `551B8838…` → `8B0B4085…`，mtime 20:56:24 → 21:03:30，长度仍 33361920 字节） |
| rev2 命令数 / 退出码 | **27 条：22 条 exit 0、5 条 exit 1**，与 pass-1 对应命令**逐条一致** |
| 产物哈希（8 组） | fine CSV/JSON、coarse CSV/JSON、rc_filter tran CSV / ac CSV / tran JSON、nonaligned CSV —— **全部 pass-1 ≡ rev2（逐字节）** |
| examples 全量回归（rev2） | `check` 7/7 exit 0；`run` 9/9 exit 0；10 个回归产物与 pass-1 **逐字节相同**（`compared=10 mismatches=0 missing=0`） |
| 诊断文本 | 5 条失败命令 + 2 条 REPL 会话的 **stdout 与 stderr 均逐字节相同**（`SequenceEqual=True`，共 7 组） |
| REPL 产物 | coarse/fine 的 REPL CSV+JSON 与 pass-1 记录的哈希**逐位相同** |
| 失败时零产物 | rev2 的 `FAIL-*` 目录同样**不存在** |

⇒ 由真实 CLI 独立证实"注释改动 / 行为不变"：**rev2 与 rev1 在产品可观测面上逐字节等价**，§3–§11 的全部结论对 rev2 同样成立（下文未逐条重述的数值即 pass-1 实测值与 rev2 实测值相同）。

---

## 2. 命令与退出码总矩阵（87 条，全部真实执行）

记录文件：`target\round2-evidence\w5\logs\matrix.tsv`；每条命令的 stdout/stderr 原文在 `target\round2-evidence\w5\logs\<name>.{stdout,stderr}.txt`（经 `cmd /c "<cmd> > out.txt 2> err.txt"` 分离写出，字节数见矩阵）。rev2 复跑的命令以 `rev2-` 前缀单独命名，不覆盖 pass-1 日志。

**汇总：87 条命令 = 67 条 exit 0 + 20 条 exit 1；退出码取值集合 = {0, 1}，无任何其他退出码。**
（pass-1 于 rev1 二进制上 60 条 = 45×0 + 15×1；pass-2 于 rev2 二进制上 27 条 = 22×0 + 5×1。）

| 组 | 命令数 | exit 0 | exit 1 | 说明 |
|---|---|---|---|---|
| `--help` / `--version` 探测 | 6 | 6 | 0 | 参数面确认 |
| examples 成功路径（check/run csv/json/both） | 11 | 11 | 0 | rc_filter / rlc / voltage_divider |
| Task A 验收（fine/coarse check+run csv+json） | 6 | 6 | 0 | 2001 / 21 点 |
| 失败路径 | 13 | 0 | 13 | E_VALUE×2 类、E_UNSUPPORTED、E_LIMIT×2 层、E_NAME、E_IO、E_ARGUMENT |
| REPL | 2 | 2 | 0 | CSV 与文件模式逐字节相同 |
| examples 全量 `check` 回归 | 7 | 7 | 0 | 7/7 |
| examples 全量 `run` 回归（9 个实验） | 10 | 9 | 1 | 第 10 条为「多实验未指定 --experiment」契约用例 |
| 附带：capabilities / 非对齐网格 / 重跑 json | 5 | 4 | 1 | 见 §7、§9 |
| **rev2 复验（§1.2）** | **27** | **22** | **5** | 关键用例 + 全量 examples 回归，产物与 pass-1 逐字节相同 |

完整清单（`name  exit  stdout/stderr 字节`）：

```
help-root 0 674/0            help-check 0 487/0        help-run 0 534/0
help-repl 0 421/0            help-capabilities 0 192/0  version 0 11/0
check-rc-filter 0 471/0      check-rlc 0 737/0          check-vdiv 0 363/0
run-rc-csv 0 571/0           run-rc-json 0 577/0       run-rc-both 0 768/0
run-vdiv-csv 0 182/0         run-vdiv-json 0 184/0     run-vdiv-both 0 248/0
run-rlc-csv 0 208/0          run-rlc-ring-csv 0 264/0  run-rlc-ring-json 0 270/0
check-pulse-fine 0 354/0     check-pulse-coarse 0 358/0
run-fine 0 168/0             run-coarse 0 172/0
run-fine-json 0 174/0        run-coarse-json 0 178/0
fail-check-neg 1 0/584       fail-check-zero 1 0/575   fail-check-neg-json 1 0/584
fail-run-neg 1 0/584         fail-run-zero 1 0/575
fail-check-zerorise 1 0/562  fail-run-zerorise 1 0/562
fail-check-limit 1 0/503     fail-run-limit 1 0/503
fail-run-badexp 1 0/110      fail-run-nofile 1 0/104   fail-check-nofile 1 0/104
fail-run-neg-into-existing-out 1 0/584
regr-run-multi-noexp 1 0/137
run-resample-limit 1 0/276
repl-coarse 0 337/0          repl-fine-filearg 0 487/0
regr-check-<7 examples> 0 全绿（stdout 361–911 / stderr 0）
regr-run-<9 experiments> 0 全绿（stdout 210–362 / stderr 0）
run-nonaligned 0 183/0       check-resample-limit 0 363/0
capabilities 0 553/0
```

（全部 60 条 pass-1 记录的逐条字节数见 `logs\matrix.tsv`；pass-2 的 27 条 `rev2-*` 记录同表，退出码与上表对应条目逐条一致。）

---

## 3. Task A 验收（核心）：fine / coarse 对照

输入为修复前基线使用的**同一批文件**（只读引用，未复制、未修改）：
`target\round2-evidence\repro\pulse-fine.cdsl`（`output_interval: 1.ns`）、`pulse-coarse.cdsl`（`output_interval: 100.ns`）；
两者电路完全相同：`rise=10.ns, fall=10.ns, period=20.us, width=10.us`、`max_step: 1.ns`、`stop: 2.us`。

### 3.1 命令与真实退出码

```
$ .\target\debug\cdsl.exe check target\round2-evidence\repro\pulse-fine.cdsl      → exit 0  (stdout 354 / stderr 0)
$ .\target\debug\cdsl.exe check target\round2-evidence\repro\pulse-coarse.cdsl    → exit 0  (stdout 358 / stderr 0)
$ .\target\debug\cdsl.exe run --experiment fine   --out ...\w5\out\fine   --format csv <fine>   → exit 0
$ .\target\debug\cdsl.exe run --experiment coarse --out ...\w5\out\coarse --format csv <coarse> → exit 0
```

`run` stdout 原文：

```
experiment `fine` on circuit `rc` (backend thevenin 0.5.0)
  tran1: 2001 time points; signals: v(vin), v(out)
  wrote target\round2-evidence\w5\out\fine\fine.tran1.csv

experiment `coarse` on circuit `rc` (backend thevenin 0.5.0)
  tran1: 21 time points; signals: v(vin), v(out)
  wrote target\round2-evidence\w5\out\coarse\coarse.tran1.csv
```

### 3.2 验收项逐条判定

| # | 验收项 | 期望 | 实测 | 判定 |
|---|---|---|---|---|
| A1 | fine 输出视图点数 | 2001（1 ns 均匀网格 0..2us） | **2001**（CLI 报告、CSV 数据行、JSON `axis.values` 三者一致） | **PASS** |
| A2 | coarse 输出视图点数 | 21（100 ns 网格 0..2us） | **21**（同上三者一致） | **PASS** |
| A3 | coarse 在 `t=100ns` 处 `v(vin)` | 1 | **1**（CSV 精确行 `0.00000010000000000000001,1,0.000949548456079576`；JSON `signals[0].values[1] == 1.0`） | **PASS** |
| A4 | 末点 `v(out)` fine vs coarse | 逐位相同 | **逐位相同**：两者末行均为 `0.000002,1,0.01975231511815381` | **PASS** |
| A5 | 首点保留 | 原始首点 | 两者首行均为 `0,0,0`（t 字面量 `0`） | **PASS** |
| A6 | 声明 `rise=10.ns` 未被展宽 | 10 ns | fine：`v(vin)=1` 首次出现在 `0.00000001`（=10 ns），斜坡段 `0.000000001→0.1`、`0.000000009000000000000001→0.9` ⇒ 斜率 0.1 V/ns ⇒ **rise=10 ns** | **PASS** |
| A7 | `output_interval` 不再进入引擎 `Tran.step` | 两实验解算器设置相同 | fine 与 coarse 的 `tran.solver_step` **相同**（`0.0000000019999999999999997`）、`tran.solve_points` **相同**（`2015`） | **PASS** |
| A8 | 重采样网格 = 首点 / 首点+k·interval / 原始末点 | — | coarse 21 点 = `0` + `k·100ns`(k=1..19) + `2000ns`；非对齐用例（337 ns）见 §7.2 | **PASS** |

`coarse` 全部 21 行（原文）：

```
time,v(vin),v(out)
0,0,0
0.00000010000000000000001,1,0.000949548456079576
0.00000020000000000000002,1,0.0019480995488728298
...
0.0000019000000000000002,1,0.018771577133740856
0.000002,1,0.01975231511815381
```

---

## 4. 修复前 vs 修复后：数值与哈希对照（关键表）

修复前基线产物**未被覆盖、未被修改**（哈希与 `repro-baseline.md` §7 逐位一致，见 §4.4），本轮结论以它为对照。

### 4.1 总览

| 量 | 修复前 fine (1ns) | 修复前 coarse (100ns) | 修复后 fine | 修复后 coarse |
|---|---|---|---|---|
| CSV SHA256 | `348973A51D487072DFDCD93460796B3ACC44A17039959919C65309F7FDEFCA96` | `FE133BDC297A2D68CEB1B91ADE50A784905A92FFD19B4ECD62F87CBF6546FAC3` | `1CDD663A9920DA0C255E5CD209BB581D6BE85C3ECC2EF1C89E705C877797364E` | `F1017F6372A3A10FEAD6D77BF2DD43E97FD9A916410F9459EB64B20D93DE487B` |
| CSV 数据行 | 2015 | 2015 | **2001** | **21** |
| 返回时间轴 | 解算器自适应网格 | 解算器自适应网格 | 均匀 1 ns | 均匀 100 ns |
| 首行 | `0,0,0` | `0,0,0` | `0,0,0` | `0,0,0` |
| **末行** | `0.000002,1,0.01975231511815381` | `0.000002,1,`**`0.019311063940867238`** | `0.000002,1,0.01975231511815381` | `0.000002,1,0.01975231511815381` |
| `t=100ns` 处 `v(out)` | 无该时刻行；99.49999999999989→0.0009445532038218552 / 100.49999999999989→0.0009545437083372945，**线性插值 = 0.000949548456079576** | `0.0004998333667460576` | **`0.000949548456079576`** | **`0.000949548456079576`** |
| 首次 `v(vin)≥0.999999` | 10 ns | 100 ns | 10 ns | 100 ns |
| 由 CSV 斜率反推 rise | 10 ns | **100 ns**（被静默展宽） | **10 ns** | 网格粗于边沿，不可分辨（真值由 `tran.waveform_bound=10ns` 记录） |

### 4.2 题面点名的关键差异（同物理时刻对照）

修复前 coarse 在 `t = 50.02375000000000004 ns` 的**真实 CSV 行**：

```
0.00000005002375000000004,0.5002375000000003,0.00012509791368610083
```

| 时刻 | 量 | 修复前 coarse（CSV 原文） | 修复后真值（同 t，1 ns 输出网格线性插值） | 差异 |
|---|---|---|---|---|
| `t=50.02375 ns` | `v(vin)` | **0.5002375000000003** | **1** | +0.4997625 V |
| `t=50.02375 ns` | `v(out)` | **0.00012509791368610083** | **0.000450135720143042** | ×3.60 |
| `t=100 ns` | `v(vin)` | 1 | 1 | 0 |
| `t=100 ns` | `v(out)` | **0.0004998333667460576** | **0.000949548456079576** | +0.000449715 V |

即：修复前 coarse 的**解算波形本身是错的**——声明 `rise: 10.ns` 被 `output_interval: 100.ns` 经引擎 `Tran.step` 静默展宽为 100 ns，因此 `v(vin)` 沿 100 ns 斜坡线性上升（`0.5002375 = 50.02375/100`）、`v(out)` 相应偏小约 2.7 倍。修复后同一时刻 `v(vin)=1`、`v(out)` 提升到正确量级。

两条独立解析积分校验（`τ = R·C = 100 us`，`v(out) ≈ (1/τ)∫v(vin)dt`）：

| 用例 | 面积积分预测 | CSV 实测 | 一致性 |
|---|---|---|---|
| 修复后 fine/coarse @100ns（10 ns 斜坡） | `(½·10 + 90) ns·V / 100us` = `9.5e-4` | `0.000949548456079576` | ✓（0.05%，差值来自 RC 自身充电修正） |
| 修复前 coarse @100ns（100 ns 斜坡） | `(½·100) ns·V / 100us` = `5.0e-4` | `0.0004998333667460576` | ✓（即修复前确为 100 ns 斜坡） |

### 4.3 口径澄清（必须记录的"不一致"）

修复后 **coarse 输出视图**在 `t = 50.02375 ns` 处线性插值得到的仍是 `0.5002375`：

```
post-fix coarse interp @50.02375ns v(vin): 0.5002375
```

**这不是缺陷残留**：coarse 的输出网格就是 100 ns（用户显式声明），在 `0..100 ns` 之间线性插值一条 10 ns 边沿必然得到 `t/100ns`。它是"粗网格 + 线性插值"的必然表现，也正是本轮验收把 `v(vin)=1` 的判据落在 `t=100ns`（网格点）而非 `t≈50ns` 的原因。**记录于此以免被误读为两个不同的结论。**

### 4.4 求解解未被修复扰动的量化证据（独立于上面各项）

- **末点锚定**：修复后 fine 末点 `0.01975231511815381` 与**修复前 fine 末点逐位相同**（字面量完全相同，见 §4.1）；修复后 coarse 末点亦与之逐位相同。
- **三点交会**：`t=100ns` 处 `v(out)` = 修复前 fine 网格线性插值 = 修复后 fine 精确网格点 = 修复后 coarse 精确网格点 = `0.000949548456079576`（三方逐位一致）。
- **全网格扫描**：把修复前 fine 的**全部 2015 个解算点**作为比较时刻，在修复后 fine 的 1 ns 输出网格上线性插值后逐点比较：

| 比较量 | 最大绝对偏差 | 发生时刻 |
|---|---|---|
| `v(out)` | **1.27897362595743e-07 V** | `t = 5.1175e-10 s`（10 ns 斜坡内部，曲率最大处） |
| `v(vin)` | **2.77555756156289e-17 V** | `t = 2.02375e-09 s` |
| `v(out)` @49.5 ns | 2.49888777747488e-11 V | — |

  最大偏差 `1.28e-7 V` **远小于 brief §17 的 `atol = 1e-5 V`**（且其中还含 1 ns 网格插值本身的误差）⇒ 修复后的输出视图在 §17 容差内重现了修复前的求解解；修复只改变了**输出采样**，没有改变**解**。
- **求解网格点数锚定**：`tran.solve_points = 2015`（修复后，fine 与 coarse 相同）恰等于修复前 fine/coarse CSV 的数据行数 `2015`（修复前 CSV 即原始解算网格，无重采样）⇒ 解算网格规模与修复前一致。

### 4.5 基线证据完整性（证明对照物未被污染）

```
target\round2-evidence\repro\out-fine\fine.tran1.csv      348973A51D487072DFDCD93460796B3ACC44A17039959919C65309F7FDEFCA96
target\round2-evidence\repro\out-coarse\coarse.tran1.csv  FE133BDC297A2D68CEB1B91ADE50A784905A92FFD19B4ECD62F87CBF6546FAC3
target\round2-evidence\repro\out-neg\neg.tran1.csv        348973A51D487072DFDCD93460796B3ACC44A17039959919C65309F7FDEFCA96
target\round2-evidence\repro\out-zero\zero.tran1.csv      348973A51D487072DFDCD93460796B3ACC44A17039959919C65309F7FDEFCA96
```

与 `repro-baseline.md` §7 表中哈希逐位相同 ⇒ 修复前产物未被覆盖，且 `out-neg`/`out-zero` 与 `out-fine` 逐字节相同这一修复前特征仍然成立（本轮未触碰该目录）。

---

## 5. 失败路径（四类 + 两层 E_LIMIT）

全部用 `cmd /c "<cmd> > out.txt 2> err.txt"` 分离取证。**共同特征：`stdout = 0 字节`，全部诊断在 `stderr`，退出码 1，且 `--out` 目录根本未被创建（无任何结果文件，含无部分写入）。** 其中 F2/F4/F6/F8/F13 已在 rev2 二进制上复跑（`rev2-fail-*`），**退出码、stdout、stderr 字节数与内容全部逐字节相同**。

| # | 输入 | 命令 | exit | stdout | stderr | 诊断码 | span 指向 |
|---|---|---|---|---|---|---|---|
| F1 | `repro\negative-interval.cdsl`（`output_interval: -1.ns`） | `check` | **1** | 0 B | 584 B | `E_VALUE` | `:8:53`（`-1.ns` 实参，caret 4 字符） |
| F2 | 同上 | `run --experiment neg --format csv` | **1** | 0 B | 584 B | `E_VALUE` | 同上 |
| F3 | `repro\zero-interval.cdsl`（`output_interval: 0.s`） | `check` | **1** | 0 B | 575 B | `E_VALUE` | `:8:53`（caret 3 字符） |
| F4 | 同上 | `run --experiment zero --format csv` | **1** | 0 B | 575 B | `E_VALUE` | 同上 |
| F5 | F1 输入 | `check --json` | **1** | 0 B | 584 B | `E_VALUE` | `--json` 亦不吞掉诊断 |
| F6 | `w5\cases\zero-rise.cdsl`（`rise: 0.s`） | `check` | **1** | 0 B | 562 B | `E_UNSUPPORTED` | `:3:18`（源 `v1`） |
| F7 | 同上 | `run --experiment zerorise` | **1** | 0 B | 562 B | `E_UNSUPPORTED` | 同上 |
| F8 | `w5\cases\micro-rise-long-stop.cdsl`（`rise: 1.ps` + `stop: 1.s`，无 `max_step`） | `check` | **1** | 0 B | 503 B | `E_LIMIT` | `:8:3`（`tran stop: 1.s`） |
| F9 | 同上 | `run --experiment tinyedge` | **1** | 0 B | 503 B | `E_LIMIT` | 同上 |
| F10 | 合法文件 + `--experiment nonexistent` | `run` | **1** | 0 B | 110 B | `E_NAME` | — |
| F11 | `examples\does_not_exist.cdsl` | `run` | **1** | 0 B | 104 B | `E_IO` | — |
| F12 | 同上 | `check` | **1** | 0 B | 104 B | `E_IO` | — |
| F13 | `w5\cases\resample-limit.cdsl`（重采样规模） | `run --experiment bigout` | **1** | 0 B | 276 B | `E_LIMIT`（重采样层） | — |

关键 stderr 原文（UTF-8；已核对为合法 UTF-8，非终端编码假象）：

```
F1/F2 (E_VALUE, output_interval: -1.ns)
error[E_VALUE]: `output_interval:` must be a finite number greater than zero
  --> target\round2-evidence\repro\negative-interval.cdsl:8:53
   |
8 |   tran stop: 2.us, max_step: 1.ns, output_interval: -1.ns
  |                                                     ^^^^^
   = it sets the sampling of the returned waveform; omit it to keep the solver's own time points
error[E_ARGUMENT]: experiment `neg` declares no analysis
  --> target\round2-evidence\repro\negative-interval.cdsl:7:12
   = add `op`, `dc`, `ac` or `tran`

F3/F4 (E_VALUE, output_interval: 0.s) —— 同型，caret 为 `^^^`

F6/F7 (E_UNSUPPORTED, rise: 0.s)
error[E_UNSUPPORTED]: the backend cannot honour `rise: 0` on source `v1`: the engine substitutes its own print step for a zero or non-finite timing, which would silently change the declared waveform
  --> target\round2-evidence\w5\cases\zero-rise.cdsl:3:18
   = source: v1
   = give the pulse a finite, positive rise, fall and period; the engine has no ideal (zero-width) edge

F8/F9 (E_LIMIT, 引擎步数预算)
error[E_LIMIT]: the declared source timings are too fine for this simulation window: honouring them would need about 1000000000000 solver steps, over the limit of 1000000
  --> target\round2-evidence\w5\cases\micro-rise-long-stop.cdsl:8:3
8 |   tran stop: 1.s
  |   ^^^^^^^^^^^^^^
   = waveform bound: 0.000000000001 s
   = effective step: 0.000000000001 s
   = shorten `stop:`, set a `max_step:`, use a longer `rise:`/`fall:`/`period:`, or split the run; the edge is never widened silently to fit

F10 (E_NAME)
error[E_NAME]: no experiment named `nonexistent` in `examples\voltage_divider.cdsl`
   = experiments: divider

F11/F12 (E_IO)
error[E_IO]: cannot read `examples\does_not_exist.cdsl`: 系统找不到指定的文件。 (os error 2)

F13 (E_LIMIT, 重采样层 —— 与 F8 不同层)
error[E_LIMIT]: an output interval of 0.000000001 s over this trace needs 100000001 points (200000002 values), over the limit of 50000000
   = interval: 0.000000001 s
   = no values were truncated: use a longer `output_interval:`, a shorter `stop:`, or raise the result limit
```

补充与观察：

1. **`E_VALUE` 不回退默认值（硬证据）**：`-1.ns` 与 `0.s` 在修复前均 exit 0 且静默回退（基线 §5/§6），修复后同一批文件 exit 1 且 `stdout` 为空 —— 即"不回退"不只是库层断言，而是真实 CLI 的退出码行为。
2. **失败时零产物**：13 条失败命令全部未创建 `--out` 目录（`Get-ChildItem -Directory -match 'FAIL'` 结果为空）。另加一条更严的检查：把失败运行的 `--out` 指向**已存在且已有 1 个产物文件**的目录（`w5\out\coarse`，`--format both`），实测 `files before=1 after=1`，exit 1 ⇒ 无部分写入、无覆盖。
3. **两级 `E_LIMIT` 可区分**：F8 是引擎步数预算（`max_print_steps = 1000000`），F13 是重采样结果规模预算（`max_result_values = 50000000`），诊断文本与 context 键不同，用户可据此区分"改 `stop`/`max_step`"与"改 `output_interval`"。
4. **`check` 与 `run` 的边界（如实记录）**：`check w5\cases\resample-limit.cdsl` **exit 0**，而其 `run` exit 1（E_LIMIT）。`check` 是静态的 parse/elaborate（不求解、不重采样），故**重采样规模的 `E_LIMIT` 只有 `run` 能报**；而 DSL 层的 `E_VALUE`/`E_UNSUPPORTED`/引擎步数 `E_LIMIT` 在 `check` 即报。这是既有的分层契约，不是本轮缺陷，但**不应**被写成"check 能捕获全部运行时限制"。
5. **一个根因两条诊断**：F1–F5 在 `E_VALUE` 之后还报 `E_ARGUMENT: experiment 声明没有 analysis`（tran 在校验期被拒后该实验无分析任务）。属级联提示而非独立缺陷，如实记录。

---

## 6. 退出码契约

| 码 | 常量 | 声明位置 | 实测 |
|---|---|---|---|
| 0 | `EXIT_OK` | `crates\circuit-cli\src\main.rs:24` | 45 条命令（成功、`--help`、`:quit` 正常退出） |
| 1 | `EXIT_USER_ERROR` | `crates\circuit-cli\src\main.rs:25` | 15 条命令（语法/取值/能力/规模/名称/IO 全部归入 1） |
| 2 | `EXIT_INTERNAL` | `crates\circuit-cli\src\main.rs:26` | **不可达（既有限制，如实记录）** |

`EXIT_INTERNAL` 不可达的判据：对 `crates/**/*.rs` 全文检索 `EXIT_INTERNAL`，**命中数 = 1，即其定义本身**（无任何构造点 / `return Err(EXIT_INTERNAL)` / 比较点）。因此经设计路径无法产出退出码 2；若发生 panic，Rust 运行时会给出 101（或平台异常码）而非 2。**本轮 87 条命令（pass-1 60 条 + rev2 27 条）实测退出码集合 = {0, 1}，无 2。** 本项属"无法通过 CLI 主动触发验证"，仅由源码静态判定。

---

## 7. 元数据观测

### 7.1 fine / coarse（声明了 `output_interval`）

`coarse.tran1.json` 的 `backend.settings` 原文：

```
adapter              = 0.1.0
tran.solver_step     = 0.0000000019999999999999997     (= 2 ns，min(span/1000, min 声明 rise/fall/period))
tran.solve_points    = 2015
tran.waveform_bound  = 0.00000001                      (= 10 ns，声明的 rise/fall)
tran.max_step        = 0.000000001                     (= 1 ns)
tran.output_interval = 0.00000010000000000000001       (= 100 ns)
tran.output_grid     = resampled-linear
tran.output_points   = 21
```

| 键 | fine | coarse | 判定 |
|---|---|---|---|
| `tran.solver_step` | `0.0000000019999999999999997` | `0.0000000019999999999999997` | **相同** ⇒ `output_interval` 未进入引擎 step |
| `tran.solve_points` | `2015` | `2015` | **相同**，且 = 修复前解算网格行数 2015 |
| `tran.waveform_bound` | `0.00000001` | `0.00000001` | 声明边沿被如实记录 |
| `tran.max_step` | `0.000000001` | `0.000000001` | 用户声明原样透传 |
| `tran.output_interval` | `0.000000001` | `0.00000010000000000000001` | 用户声明原样保留 |
| `tran.output_grid` | `resampled-linear` | `resampled-linear` | 重采样层可见 |
| `tran.output_points` | `2001` | `21` | 与 CSV 数据行、`axis.values` 长度三者一致 |

### 7.2 未声明 `output_interval`（应保留求解网格）

`examples\rc_filter.cdsl` → `run --experiment response --format json|csv`，exit 0：

```
adapter             = 0.1.0
tran.solver_step    = 0.000000001          (= 1 ns)
tran.solve_points   = 1019
tran.waveform_bound = 0.000000001          (= 1 ns，声明的 rise/fall)
tran.max_step       = 0.0000005000000000000001  (= 500 ns)
```

- **`tran.output_grid` / `tran.output_interval` / `tran.output_points` 三键不存在** ⇒ 重采样层未被触发；
- CSV 数据行 `1019` = `tran.solve_points` = JSON `axis.values` 长度 `1019` ⇒ **输出视图 = 求解网格，保留成立**；
- 数值自洽：`solver_step = min(500us/1000 = 500ns, min(1ns, 1ns, 10ms) = 1ns) = 1ns` ✓ 与 `waveform_bound = 1ns` 一致。

同型验证 `examples\rlc.cdsl` → `ringing`（`rise=fall=10.ns, period=200.us, stop=600.us, max_step=200ns`）：

```
tran.solver_step = 0.00000001 (=10 ns = min(600ns, 10ns))   tran.solve_points = 3054
tran.waveform_bound = 0.00000001                            tran.max_step = 0.00000020000000000000002
axis_count = 3054 = CSV 数据行 3054    (无 output_* 三键)
```

**纯 op 实验**（`voltage_divider.cdsl` → `divider`）的 `backend.settings` 只有 `adapter = 0.1.0`，无任何 `tran.*` 键；`axis.type = none`；数值 `v(in)=5.0, v(out)=3.0, i(r1)=0.002, i(v1)=-0.002`，与示例文件头部文档中"必须给出 v(out)=3.000 V、i(r1)=+2.000 mA"一致。

### 7.3 附加：非对齐网格（`output_interval: 337.ns`，额外验证项）

`w5\cases\nonaligned-interval.cdsl`（同样 `stop: 2.us, max_step: 1.ns`）→ exit 0，`tran1: 7 time points`：

```
time,v(vin),v(out)
0,0,0
0.000000337,1,0.003314494458273682
0.000000674,1,0.00666767133546527
0.0000010110000000000001,1,0.010009567026056312
0.000001348,1,0.013340219483657653
0.000001685,1,0.016659666534192392
0.000002,1,0.01975231511815381
```

- 网格 = `0`、`k·337ns`（k=1..5，`6·337ns=2022ns > 2000ns` 故不生成）+ **原始末点 `2000ns`**；末间隔 `315ns` 短于请求的 `337ns`；
- 末点 `v(out) = 0.01975231511815381` 与 fine/coarse **逐位相同** ⇒ 原始末点恒保留、**不越界外推**（若外推，`6·337ns` 或 `2000ns` 之外必产生额外点或错值）；
- 与源码 `crates\circuit-results\src\resample.rs:113-129`（首点 → `first_s + k*interval` 的"严格小于末点"的内部点 → `last_s`）一致。

---

## 8. REPL 与文件模式一致性

REPL 用 stdin 喂脚本（与 `crates/circuit-cli/tests/repl.rs` 相同做法），真实命令：

```
$ .\target\debug\cdsl.exe repl < target\round2-evidence\w5\repl\script-coarse.txt                      → exit 0 (stdout 337 / stderr 0)
$ .\target\debug\cdsl.exe repl target\round2-evidence\repro\pulse-fine.cdsl < ...\script-fine.txt       → exit 0 (stdout 487 / stderr 0)
```

`script-coarse.txt` = `:load target\round2-evidence\repro\pulse-coarse.cdsl` + `:run coarse --out target\round2-evidence\w5\repl\coarse` + `:quit`。

REPL stdout 原文：

```
cdsl> defined circuit `rc`, experiment `coarse`
cdsl> experiment `coarse` (backend thevenin 0.5.0)
  tran1: 21 time points; signals: v(vin), v(out)
  tran1 has 21 points; pass `--out <dir>` to write them
  wrote target\round2-evidence\w5\repl\coarse\coarse.tran1.csv
  wrote target\round2-evidence\w5\repl\coarse\coarse.tran1.json
cdsl>
```

| 检查 | 实测 | 判定 |
|---|---|---|
| REPL 退出码 | `0`（`:quit`；stderr 0 字节） | **PASS** |
| REPL 打印点数 vs CSV 行数（coarse） | `tran1: 21 time points` = `tran1 has 21 points` = CSV 数据行 **21** = 文件总行 **22**（含表头） | **PASS** |
| REPL 打印点数 vs CSV 行数（fine） | `2001` = `2001` = 数据行 **2001** = 总行 **2002** | **PASS** |
| REPL CSV vs 文件模式 CSV（coarse） | SHA `F1017F6372A3A10F...` 双方**相同**；`fc /b` 输出 `FC: no differences encountered`，`fc_exit=0` | **PASS（逐字节）** |
| REPL CSV vs 文件模式 CSV（fine） | SHA `1CDD663A9920DA0C...` 双方**相同** | **PASS（逐字节）** |
| REPL JSON vs 文件模式 JSON | coarse `CBF6E24D90679A14...`、fine `9486237510957E51...` 双方各自相同 | **PASS（逐字节）** |

⇒ **文件模式与 REPL 写出完全相同的输出视图**（time 列与全部数值列逐字节一致，无需近似比较）。

---

## 9. examples 全量回归

### 9.1 `check`（7/7）

| 示例 | exit | stdout 字节 | stderr 字节 |
|---|---|---|---|
| `diode_rectifier.cdsl` | 0 | 729 | 0 |
| `ladder.cdsl` | 0 | 850 | 0 |
| `parameter_sweep.cdsl` | 0 | 361 | 0 |
| `rc_filter.cdsl` | 0 | 471 | 0 |
| `rlc.cdsl` | 0 | 737 | 0 |
| `two_stage.cdsl` | 0 | 911 | 0 |
| `voltage_divider.cdsl` | 0 | 363 | 0 |

**7/7 全绿，无任何既有示例变成失败。**

### 9.2 `run`（9 个实验，9/9 exit 0）

| 示例 / 实验 | exit | 产物与关键数值 |
|---|---|---|
| `diode_rectifier` / `rectified` | 0 | `tran1: 1008 time points`；`vpeak = 4.3071284013527 V`、`vavg = 1.2684005202349173 V` |
| `diode_rectifier` / `forward_drop` | 0 | 正常写出 |
| `ladder` / `tap` | 0 | 正常写出 |
| `ladder` / `two_rungs` | 0 | `overrides: taps=2`；`op1: scalar` |
| `parameter_sweep` / `sweep` | 0 | `dc_param_r: 8 sweep points` |
| `rlc` / `frequency_response` | 0 | `ac1: 121 frequency points` |
| `rlc` / `ringing` | 0 | `tran1: 3054 time points`；`peak = 3.7284289988626247 V`、`trough = -3.85849415262593 V` |
| `two_stage` / `response` | 0 | 正常写出 |
| `two_stage` / `inside` | 0 | 正常写出 |
| （契约用例）`rlc.cdsl` 多实验未指定 `--experiment` | 1 | `error[E_ARGUMENT]: 定义了 2 个实验；choose one with --experiment`（stdout 0 B） |

### 9.3 受本轮修复影响的示例（如实记录的行为变化）

修复改变了"未声明 `output_interval` 时的引擎 print step"：由 `span/1000` 变为 `min(span/1000, min 声明 rise/fall/period)`。因此**声明了细边沿的既有示例的瞬态波形按修复意图发生了变化**：

| 示例 | 声明边沿 | 修复前 print step | 修复后 print step | 实测影响 |
|---|---|---|---|---|
| `rc_filter.cdsl` | `rise=fall=1.ns`, `period=10.ms` | 500 ns | **1 ns** | 边沿由 500 ns 恢复为声明的 1 ns；`tran1` 行数 1015 → **1019**；`max abs(v(vout) - (1-exp(-t/tau)))` 由示例注释记载的 `2.491963e-3 V` 降至 **4.921844e-6 V**（实测，见下方） |
| `rlc.cdsl` | `rise=fall=10.ns`, `period=200.us` | 600 ns | **10 ns** | 边沿由 600 ns 恢复为声明值；`tran1` 3054 点；`peak/trough` 实测如上（无修复前 CSV 可对照） |
| `diode_rectifier.cdsl` | 无 PULSE 边沿声明 | 3 us | 3 us | 不变 |
| 其余（op/dc/ac 或无 tran） | — | — | — | 不变 |

`rc_filter` 的实测细节（本轮跑出的 CSV `response.tran1.csv`，SHA `76F1944151348EE253F5D568DBF63D2D455D95BF5EA4E9E390440B905E3A8025`）：

```
first data rows: 0,0,0 / 0.00000000012500000000000003,0.12500000000000003,... / 0.000000001,1,...
first v(vin)>=0.999999 at t = 0.000000001 (= 1 ns)      ← 声明的 rise=1.ns 被兑现
CSV 数据行 = 1019 = tran.solve_points = axis.values 长度
max |v(vout) - (1 - exp(-t/tau))| = 4.921844e-06 V at t = 1.0e-9 s   （compare 示例注释所载 2.491963e-3 V）
```

---

## 10. 发现的不一致（NEEDS_FIX，均为文档/注释，未修改源码）

### N1（中）`examples\rc_filter.cdsl:7-24` 的注释块已与当前二进制矛盾

该 19 行注释块（`git diff` 显示为本工作树**新增**、在修复前那轮写下）陈述的是**修复前**事实，三处关键数字现已不成立：

| 注释中的断言 | 修复后实测 | 判定 |
|---|---|---|
| "the step the adapter hands to the engine is `stop/1000 = 500 ns` (thevenin.rs:772-775)" | 实际 `tran.solver_step = 1 ns`；`thevenin.rs:772-775` 不再是该处代码 | **不成立** |
| "the declared `rise: 1.ns` below is widened to 500 ns ... therefore a 500 ns ramp" | `v(vin)=1` 首次出现在 **1 ns**（CSV 原文），声明的 1 ns 被兑现 | **不成立** |
| "Measured on the CSV this run writes ... `max |v(vout) - (1 - exp(-t/tau))| = 2.491963e-3 V` ... the same **1015** samples" | 实测 **4.921844e-06 V**、**1019** 行 | **不成立** |

该文件被 README/示例回归直接引用，且 `crates/circuit-backend/tests/transient_reference_regression.rs` 的注释与之互相引用（"this round's regression builds the matched finite-ramp reference instead"）。**建议**：由拥有 `examples/` 写权限的成员把该注释块改写为"声明边沿现在被兑现（1 ns），实测上与理想阶跃的最大偏差 4.92e-6 V / 1019 点"，或明确标注为"修复前历史观测"。**本任务为只读 QA，未修改。**

### N2（低）`cdsl capabilities` 的用户可见 note 已不完整

`crates\circuit-backend\src\thevenin.rs:110-112` 的 note 原文：

```
Transient output time points are solver-chosen and non-uniform; `max_step` bounds the internal step, not the output interval.
```

修复后，当声明了 `output_interval` 时瞬态输出时间点**恰恰是均匀的**（本轮实测 coarse = 均匀 100 ns 网格、fine = 均匀 1 ns 网格，`tran.output_grid = resampled-linear`）。该 note 未提及 `output_interval`，读者会据此得出"声明 `output_interval` 也不会得到均匀输出"的错误结论。

对照：`crates\circuit-core\src\plan.rs:150-161` 的文档注释**已经**正确描述了新契约（"`output_interval` requests *output sampling* ... the returned trace is resampled afterwards"），即**代码文档已更新、用户可见的 capability note 未更新**。**建议**：为该 note 补一句"…unless `output_interval` is declared, in which case the returned trace is resampled onto that uniform grid (layer: `circuit-results`)"。本任务未修改。

### N3（口径，非缺陷）粗网格插值与细边沿的组合

见 §4.3。coarse 在 `t≈50 ns` 的插值显示值仍是 `0.5002375`。这是"用户显式要求 100 ns 输出网格 + 线性插值"的必然结果，**不是**修复未生效；但若不写明，极易被读成"修复后仍是 0.5"。已如实记录。

---

## 11. 未验证项（明确边界）

1. **退出码 2（`EXIT_INTERNAL`）无法由 CLI 主动触发**：87 条命令（pass-1 60 + rev2 27）实测退出码集合 = {0,1}；其不可达性仅由源码静态检索（定义 1 处、无构造点）判定，**未**通过注入内部错误来实证。按题面要求如实记录为既有限制。
2. **未在 release profile 下验证**（仅 `cargo build --bin cdsl` 的 debug 产物）。
3. **未复跑 `cargo test --workspace`**：属 W1/W2/W3 的任务范围；本任务只做真实 CLI/REPL 黑盒验证。
4. **未验证 `i()` / `w()` 探针的重采样与测量**：本轮输入只用了 `v()`；`measure` 在同径 CLI 输入中未声明（W1 的 `circuit-session` 测试覆盖测量口径，本任务未重复）。
5. **未验证 `NaN` / `inf` / 极小正数 `output_interval` 的字面量形态**：本轮覆盖 `-1.ns` 与 `0.s` 两个题面指定形态；非有限值的可达性由 W2 的 DSL 层测试覆盖。
6. **未验证 `--verbose` 对诊断/元数据的额外影响**（本轮未使用 `--verbose`）。
7. **未验证其他后端**（仅 thevenin 0.5.0）与 REPL 的行编辑/历史/补全（需真实 TTY）。
8. **`rlc.cdsl` 与 `diode_rectifier.cdsl` 的瞬态无修复前 CSV 对照**：`target\round2-evidence\repro\` 只含 pulse-fine/coarse 两条基线，故这两例只给修复后实测值（§9.3），不声称数值差异幅度。
9. `run --format both` 仅对 `rc_filter` 与 `voltage_divider` 验证；fine/coarse 分别以 `csv` 与 `json` 单格式验证（产物内容已各自核对）。

---

## 12. 结论

**PASS** —— 任务 A 的修复在真实 CLI/REPL 上成立，且已对 lead 冻结的 **rev2** 复验通过：

- **rev2 复验**：`thevenin.rs` 注释-only 改动后重新构建（exit 0，二进制 `551B8838…` → `8B0B4085…`），27 条关键命令 + examples 全量回归复跑，**8 组产物哈希、10 个回归产物、5 条失败诊断（stdout+stderr）、2 条 REPL 转录全部与 pass-1 逐字节相同** ⇒ 注释改动未产生任何行为差异，下文全部结论对 rev2 成立。

- **成功路径**：examples 的 `check` 7/7、`run` 9/9 exit 0；`--format csv|json|both` 均按声明写出正确文件集。
- **Task A 验收**：coarse = **21 点**、fine = **2001 点**；coarse 在 `t=100ns` 处 `v(vin)=1`；fine/coarse 末点 `v(out)` **逐位相同**（`0.01975231511815381`，且与修复前 fine 末点逐位相同）；声明 `rise=10.ns` 未被展宽。
- **与修复前对照**：修复前 coarse 在同一物理时刻 `t=50.02375 ns` 报 `v(vin)=0.5002375000000003`、`v(out)=0.00012509791368610083`；修复后同刻真值为 `1` 与 `0.000450135720143042`；`t=100ns` 处 `v(out)` 由 `0.0004998333667460576` 修正为 `0.000949548456079576`。修复前 fine 与 coarse 的末点互不相同（`0.01975231511815381` vs `0.019311063940867238`），修复后两者逐位相同。
- **解未被扰动**：全部 2015 个修复前解算点比对，`v(out)` 最大偏差 `1.28e-7 V` ≪ §17 `atol=1e-5 V`。
- **失败路径**：`E_VALUE` / `E_UNSUPPORTED` / `E_LIMIT`（引擎步数层 + 重采样层各一）/ `E_NAME` / `E_IO` 五类全部 exit 1、`stdout` 0 字节、诊断只在 `stderr`、**零结果文件（含无部分写入）**；`output_interval` 的 `-1.ns` 与 `0.s` 在 `check` 与 `run` 上均硬失败，不回退默认值。
- **REPL 与文件模式**：CSV/JSON **逐字节相同**（`fc /b` 无差异），REPL 打印点数 = CSV 数据行数（21/2001）。
- **元数据**：`tran.solver_step` / `solve_points` / `waveform_bound` / `max_step` / `output_interval` / `output_grid=resampled-linear` / `output_points` 全部如实记录，且 `solve_points` 在 fine/coarse 间相同（2015）、无 `output_interval` 时 `output_*` 三键不出现且输出行数 = `solve_points`。
- **退出码契约**：成功 0、用户错误 1；2 不可达（既有限制，已如实记录）。
- **待办（不阻塞验收）**：N1（`examples/rc_filter.cdsl` 注释陈旧，含 3 处错误数字）、N2（`capabilities` note 未提及 `output_interval` 会产生均匀网格）——两项均为文档/注释，需由拥有相应写权限的成员处理；N3 为必须随结论一起陈述的口径澄清。
