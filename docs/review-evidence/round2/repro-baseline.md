# R4 — P1 复现基线验证（修复前，真实 CLI）

- 任务：共享任务 `task-4`（owner: `r4-repro-qa`）
- 日期：2026-09-18 20:33–20:35 (+08:00)
- 结论：**PASS（复现成立）** —— P1-a、P1-b 均在本工作树 + 本地构建的 `cdsl.exe` 上以真实 CLI 复现，且额外确认 `output_interval: 0.s` 走同一静默回退分支。
- 本轮**未做任何修复**，未修改任何源码/测试/文档（仅本报告与本任务输出目录）。

---

## 1. 代码版本与证据来源（可复核锚点）

```
$ git rev-parse HEAD
cb5d8a212f66922181580900a05fb3d42abe32f2
```

`git status --short` 摘要（本次取证时刻，工作树**已带未提交改动**，其中 `crates/circuit-backend/src/thevenin.rs`、`crates/circuit-dsl/src/elaborate.rs`、`crates/circuit-core/src/connectivity.rs` 等为他人/前轮改动，本次未触碰）：

```
 M README.md
 M RUST_CIRCUIT_DSL_PROMPT.md
 M _probe/src/bin/robustness.rs
 M _probe/src/main.rs
 M crates/circuit-backend/src/thevenin.rs
 M crates/circuit-cli/tests/e2e.rs
 M crates/circuit-core/src/connectivity.rs
 M crates/circuit-dsl/src/elaborate.rs
 M docs/architecture.md
 M docs/backend-evaluation.md
 M docs/language.md
 M docs/testing.md
 M examples/rc_filter.cdsl
?? AGENT_TEAM_EXECUTION_PROMPT.md
?? agent-team-switch.md
?? crates/circuit-backend/tests/phase_regression.rs
?? crates/circuit-backend/tests/transient_reference_regression.rs
?? crates/circuit-dsl/tests/phase_syntax_regression.rs
?? crates/circuit-dsl/tests/reference_path_regression.rs
?? docs/next-iteration-plan.md
?? docs/prompt-review.md
?? docs/review-evidence/
```

被验证的二进制与源文件指纹（取证时刻实测）：

| 对象 | SHA256 | mtime |
|---|---|---|
| `target\debug\cdsl.exe` | `10A3B72A22D60EDA6BBC6DFB71741E9407E30902F860BF18F30553DF6513BE16` | 2026-09-18 19:54:43 |
| `crates\circuit-backend\src\thevenin.rs` | `CCB30627A0A5872FBE497D212ED9774F7DEAF8AFB6231DF3944C40081466848E` | 2026-09-18 19:51:37 |
| `crates\circuit-dsl\src\elaborate.rs` | `F95ECB7793F69C6D97995D5BA6D34B37D0E7914BAB9444B5A3AC4D7500B3C794` | 2026-09-18 19:51:37 |
| `crates\circuit-core\src\plan.rs` | `E562162A41A6B0F42954C225BABB793C5BFB821137A3783BE40E6044F39609D8` | 2026-09-18 15:38:45 |

工具链：`cargo 1.98.1 (797e8a9bc 2026-08-05)`、`rustc 1.98.1 (48ae229cea 2026-09-01)`。

二进制新鲜度用真实构建确认（**未出现 build dir 文件锁等待**）：

```
$ cargo build --bin cdsl
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.17s     → exit 0
```
（0.17s 且 up-to-date ⇒ 19:54:43 的 `cdsl.exe` 即当前 `thevenin.rs`/`elaborate.rs` 的产物；同时说明 cargo 确认其与源一致，无需重链接。）

**P1-a 源码位（只读引用，未修改）** —— `crates/circuit-backend/src/thevenin.rs:771-782`：

```rust
let span = spec.stop_s - spec.start_s;
let step = spec
    .output_interval
    .filter(|s| *s > 0.0)
    .unwrap_or_else(|| span / 1000.0);
CqAnalysis::Tran(CqTran { step, stop: spec.stop_s, start: spec.start_s, uic: spec.uic, tmax: spec.max_step })
```
即 `output_interval` 被直接送进引擎的 `Tran.step`（tstep），而引擎把 tstep 当作 PULSE `tr`/`tf` 的下限；`filter(|s| *s > 0.0)` 让 `-1.ns` 与 `0.s` 都落到 `span/1000.0` 静默回退。下文所有行为观测与该源码位一致。

---

## 2. 复现输入（全部为本轮新建，位于 `target/round2-evidence/repro/`）

未覆盖、未删除 `target/next-round-review/` 的任何文件（该目录 mtime 在 20:24–20:25 被**他人**写入；我全程未写入该目录，可复核见 §8）。

### 2.1 `target/round2-evidence/repro/pulse-fine.cdsl` （406 bytes）

```cdsl
circuit :rc do
  node :vin, :out
  voltage_source :v1, p: :vin, n: :gnd, dc: 0.V, waveform: pulse(low: 0.V, high: 1.V, delay: 0.s, rise: 10.ns, fall: 10.ns, width: 10.us, period: 20.us)
  resistor :r1, p: :vin, n: :out, value: 1.kohm
  capacitor :c1, p: :out, n: :gnd, value: 100.nF
end
experiment :fine, circuit: :rc do
  tran stop: 2.us, max_step: 1.ns, output_interval: 1.ns
  save v(:vin), v(:out)
end
```

### 2.2 `target/round2-evidence/repro/pulse-coarse.cdsl` （410 bytes）

与 2.1 仅实验名与 `output_interval` 不同：

```cdsl
experiment :coarse, circuit: :rc do
  tran stop: 2.us, max_step: 1.ns, output_interval: 100.ns
  save v(:vin), v(:out)
end
```

### 2.3 `target/round2-evidence/repro/negative-interval.cdsl` （406 bytes）

```cdsl
experiment :neg, circuit: :rc do
  tran stop: 2.us, max_step: 1.ns, output_interval: -1.ns
  save v(:vin), v(:out)
end
```

### 2.4 `target/round2-evidence/repro/zero-interval.cdsl` （405 bytes，额外验证项）

```cdsl
experiment :zero, circuit: :rc do
  tran stop: 2.us, max_step: 1.ns, output_interval: 0.s
  save v(:vin), v(:out)
end
```

### 2.5 回退判别探针（P1-b 的“回退值确实到达引擎”证据）

`target/round2-evidence/repro/fallback-probe-neg.cdsl`（414 bytes）：

```cdsl
circuit :rc do
  node :vin, :out
  voltage_source :v1, p: :vin, n: :gnd, dc: 0.V, waveform: pulse(low: 0.V, high: 1.V, delay: 0.s, rise: 10.ns, fall: 10.ns, width: 10.us, period: 20.us)
  resistor :r1, p: :vin, n: :out, value: 1.kohm
  capacitor :c1, p: :out, n: :gnd, value: 100.nF
end
experiment :negprobe, circuit: :rc do
  tran stop: 20.us, max_step: 100.ns, output_interval: -1.ns
  save v(:vin), v(:out)
end
```

`target/round2-evidence/repro/fallback-probe-control.cdsl`（414 bytes）：同上，仅 `output_interval: 1.ns`、实验名 `:ctrlprobe`。

> 设计意图：`stop=20.us` ⇒ 静默回退值 `span/1000 = 20 ns`，此时回退值 **大于** 声明 `rise=10.ns`，PULSE 边沿被拉宽到 20ns ⇒ v(vin) 波形**可被直接观测**。若把 `stop` 取默认 2us（回退值 2ns < 10ns），回退与 1ns 在波形上无法区分——这正是标准电路上看不出 P1-b 的原因。

---

## 3. 真实命令与退出码清单

所有命令工作目录 `F:\codexprojects\dsl000`，二进制 `.\target\debug\cdsl.exe`（子命令与参数由 `--help` 实测确认，非猜测）。

| # | 命令 | 退出码 |
|---|---|---|
| 1 | `.\target\debug\cdsl.exe --help` | 0 |
| 2 | `.\target\debug\cdsl.exe check --help` | 0 |
| 3 | `.\target\debug\cdsl.exe run --help` | 0 |
| 4 | `cargo build --bin cdsl` | 0 |
| 5 | `.\target\debug\cdsl.exe check target\round2-evidence\repro\pulse-fine.cdsl` | 0 |
| 6 | `.\target\debug\cdsl.exe check target\round2-evidence\repro\pulse-coarse.cdsl` | 0 |
| 7 | `.\target\debug\cdsl.exe check target\round2-evidence\repro\negative-interval.cdsl` | **0** |
| 8 | `.\target\debug\cdsl.exe check target\round2-evidence\repro\zero-interval.cdsl` | **0** |
| 9 | `.\target\debug\cdsl.exe check --json target\round2-evidence\repro\negative-interval.cdsl` | 0 |
| 10 | `.\target\debug\cdsl.exe run --experiment fine --out target\round2-evidence\repro\out-fine --format csv target\round2-evidence\repro\pulse-fine.cdsl` | 0 |
| 11 | `.\target\debug\cdsl.exe run --experiment coarse --out target\round2-evidence\repro\out-coarse --format csv target\round2-evidence\repro\pulse-coarse.cdsl` | 0 |
| 12 | `.\target\debug\cdsl.exe run --experiment neg --out target\round2-evidence\repro\out-neg --format csv target\round2-evidence\repro\negative-interval.cdsl` | **0** |
| 13 | `.\target\debug\cdsl.exe run --experiment zero --out target\round2-evidence\repro\out-zero --format csv target\round2-evidence\repro\zero-interval.cdsl` | **0** |
| 14 | `.\target\debug\cdsl.exe run --verbose --experiment neg --out target\round2-evidence\repro\out-neg-verbose --format csv target\round2-evidence\repro\negative-interval.cdsl` | 0 |
| 15 | `.\target\debug\cdsl.exe run --verbose --experiment fine --out target\round2-evidence\repro\out-fine-verbose --format csv target\round2-evidence\repro\pulse-fine.cdsl` | 0 |
| 16 | `.\target\debug\cdsl.exe run --experiment negprobe --out target\round2-evidence\repro\out-probe-neg --format csv target\round2-evidence\repro\fallback-probe-neg.cdsl` | 0 |
| 17 | `.\target\debug\cdsl.exe run --experiment ctrlprobe --out target\round2-evidence\repro\out-probe-control --format csv target\round2-evidence\repro\fallback-probe-control.cdsl` | 0 |
| 18 | `.\target\debug\cdsl.exe run --experiment neg ... target\round2-evidence\repro\neg-interval.cdsl`（**文件名笔误**，见 §3.1） | 1 |

### 3.1 一条真实失败命令（如实记录，非复现结论）

第 18 条是我方脚本的文件名笔误（写 `neg-interval.cdsl`，实际文件名为 `negative-interval.cdsl`），真实输出：

```
error[E_IO]: cannot read `target\round2-evidence\repro\neg-interval.cdsl`: 系统找不到指定的文件。 (os error 2)   → exit 1
```

退出码 1 只反映“文件不存在”，**不作为任何 P1 证据**；正确的第 12 条已重跑并 exit 0。记录于此仅为可复核性完整。

### 3.2 stdout / stderr 分离取证（“无诊断”的硬证据）

用 `cmd /c "<cmd> > <stdout> 2> <stderr>"` 分离两路，产物在 `target/round2-evidence/repro/logs/`：

| 命令 | exit | stdout 字节 | stderr 字节 |
|---|---|---|---|
| `check negative-interval.cdsl` | 0 | 360 | **0** |
| `check zero-interval.cdsl` | 0 | 357 | **0** |
| `check pulse-fine.cdsl` | 0 | 354 | **0** |
| `check pulse-coarse.cdsl` | 0 | 358 | **0** |
| `run --experiment neg ...` | 0 | 169 | **0** |
| `run --experiment zero ...` | 0 | 172 | **0** |

`check` 的完整 stdout（第 7 条，逐字节 360 bytes，**无任何 warning/error 行**）：

```
target\round2-evidence\repro\negative-interval.cdsl: 1 circuit(s), 1 experiment(s)
  circuit rc: 3 nodes (2 signal), 3 devices, 0 models
  voltage_source v1               p=vin n=gnd
  resistor   r1               p=vin n=out
  capacitor  c1               p=out n=gnd
  experiment `neg` on `rc`: 1 analysis task(s), 0 measure(s)
    - tran  save v(vin), v(out)
```

`run --experiment neg` 的完整 stdout：

```
experiment `neg` on circuit `rc` (backend thevenin 0.5.0)
  tran1: 2015 time points; signals: v(vin), v(out)
  wrote target\round2-evidence\repro\out-neg\neg.tran1.csv
```

`check --json`（第 9 条）exit 0，其 JSON 中 `analyses` 只序列化 `{"kind":"tran","probes":[...]}`，**时序 spec（`output_interval`/`max_step`/`stop`）完全不出现**，即非法值连 JSON 通路也不可见。

---

## 4. 数值表一：fine(1ns) vs coarse(100ns) —— P1-a 复现

`~50ns` 取该 CSV 中最接近 50 ns 的采样点，并给出其精确时间。数值**直接引自 CSV 原文**（下表同时给出原始科学与十进制字面量），非四舍五入叙述。

| 实验 | `output_interval` | 返回点数（CLI 报告 / CSV 数据行） | 最近 50ns 采样点精确时间（CSV 原文） | 与 50ns 偏差 | **v(vin)** | **v(out)** |
|---|---|---|---|---|---|---|
| `fine` | `1.ns` | 2015 / 2015 | `0.00000004950000000000003` s（文件第 65 行，= 49.5 ns） | −0.500 ns | **1** | **0.0004449005776724958** |
| `coarse` | `100.ns` | 2015 / 2015 | `0.00000005002375000000004` s（文件第 63 行，= 50.02375 ns） | +0.02375 ns | **0.5002375000000003** | **0.00012509791368610083** |

结论（实测，非推断）：**仅把 `output_interval` 从 1ns 改成 100ns（`rise=10.ns`、`max_step=1.ns`、`stop=2.us` 完全不变），约 50 ns 处 `v(vin)` 由 `1` V 变为 `0.5002375000000003` V**，同时 `v(out)` 由 `4.449005776724958e-4` V 变为 `1.2509791368610083e-4` V。P1-a 复现成立。

### 4.1 边沿被拉宽的直接证据（同一份 CSV）

| 实验 | 首次 `v(vin) >= 0.999999` 的时刻（CSV 原文行） | 由斜率反推的 rise |
|---|---|---|
| `fine` | `0.00000001,1,0.00004999832583715257`（= 10 ns） | 10 ns（与声明 `rise: 10.ns` 一致） |
| `coarse` | `0.00000010000000000000001,1,0.0004998333667460576`（= 100 ns） | 100 ns（= `output_interval`，被 tstep 下限静默替换） |

`coarse` 在上升沿附近完全线性：`v(vin) = t / 100ns`（CSV 原文 `t=0.00000009902374999999989 → 0.9902374999999988`、`t=0.00000010000000000000001 → 1`、`t=0.0000001001 → 1`；另见 `t=0.00000004502375 → 0.4502375`、`t=0.00000005002375000000004 → 0.5002375000000003`、`t=0.00000005402375 → 0.5402375`），斜率 `0.01 V/ns` ⇒ rise = 100 ns。`fine` 在 10–55ns 区间 `v(vin)` 恒为 `1`（如 `0.0000000101 → 1`、`0.0000000545 → 1`）。`max v(vin)` 两者均为 `1`；末点时间均为 `2000 ns`。

### 4.2 `output_interval` 对返回点数**完全无影响**（重要副产物）

`fine`=2015 点，`coarse`=2015 点（`neg`、`zero` 同样 2015 点）。即当前产品路径下 `output_interval` **根本不是输出采样间隔**：它既没有做输出抽样（coarse 若真按 100ns 采样本应为 2us/100ns+1 ≈ 21 点，实测 2015 点），也对输出时间轴无任何作用，其唯一可观测效果就是污染 PULSE 边沿。实测内部时间轴由 `max_step`/`h_max = 1.ns` 上限约束、并在 PULSE 断点附近自适应细化（前段实测为 `0.025 / 0.075 / 0.175 / 0.375 / 0.775 / 1.575 / 3.175 / 6.375 / 12.775 ns…`，上升沿后稳定在 1ns 间距，`45.5/46.5/…/54.5 ns`），2000ns 跨度共录得 2015 点；§5 判别探针（`stop=20.us, max_step=100.ns`）因步长上限更大而只有 225/228 点。

### 4.3 数据自洽性校验（证明数值真实而非拼接）

RC 时间常数 `τ = 1 kohm × 100 nF = 100 us`，在 `t << τ` 时 `v(out) ≈ (1/τ)·∫v(vin)dt`：

- `fine`：面积 = `½·10ns·1V + 39.5ns·1V = 44.5 ns·V` ⇒ 预测 `4.45e-4` V，实测（CSV 原文）`0.0004449005776724958` V（偏差 ≈0.2%）。
- `coarse`：`v(vin)=t/100ns` ⇒ 面积 = `½·(50.02375)²/100 ns·V = 12.511875 ns·V` ⇒ 预测 `1.2511875e-4` V，实测（CSV 原文）`0.00012509791368610083` V（偏差 ≈0.017%）。

两条独立积分校验均与实测一致 ⇒ 两份 CSV 分别对应“rise=10ns”与“rise=100ns”的真实瞬态解。

---

## 5. `negative`（`output_interval: -1.ns`）结论 —— P1-b 复现

| 项目 | 实测结果 |
|---|---|
| `cdsl check` 退出码 | **0** |
| `cdsl check` 诊断输出 | **无诊断**（stderr = 0 字节；stdout 仅摘要，全文见 §3.2） |
| `cdsl check --json` 退出码 | 0（时序 spec 不进 JSON） |
| `cdsl run` 退出码 | **0** |
| `cdsl run` stderr | **0 字节（无警告、无诊断）** |
| 返回点数 | **2015**（与 `fine` 相同） |
| 输出 CSV SHA256 | `348973A51D487072DFDCD93460796B3ACC44A17039959919C65309F7FDEFCA96` —— **与 `fine` 逐字节相同** |

**默认回退的证据（关键）**：`fine` 与 `neg` 的产物逐字节相同，是因为在默认 2us 电路上两者用的是同一个 `tr` 下限（`neg` 回退 `span/1000 = 2ns`，`fine` = `1ns`，二者都 < `rise=10ns` ⇒ 均为 `max(10ns,·)=10ns`），**单看标准电路无法区分“回退”与“被忽略”**。因此增加回退判别探针（§2.5，`stop=20.us ⇒ 回退值 = 20 ns`）：

| 探针 | `output_interval` | 点数 | 首次 `v(vin)>=0.999999` 的 CSV 原文行 | 由斜率反推 rise | 结论 |
|---|---|---|---|---|---|
| `fallback-probe-neg` | `-1.ns` | 225 | `0.000000019999999999999997,1,0.00009999320796566952`（= 20 ns） | **20 ns**（`0.000000012775000000000002,0.6387500000000002,…` ⇒ 斜率 `0.6375/12.775 = 0.05 V/ns`） | 回退值 `span/1000 = 20ns` **确实到达引擎**，把声明 `rise=10.ns` 静默拉宽到 20ns |
| `fallback-probe-control` | `1.ns` | 228 | `0.00000001,1,0.00004999857494754641`（= 10 ns） | 10 ns（`0.000000006375000000000001,0.6375000000000001,…` ⇒ 斜率 `0.6375/6.375 = 0.1 V/ns`） | 对照：电路本身无 20ns 边沿，`20ns` 只能来自回退值 |

⇒ **P1-b 复现成立**：`-1.ns` 通过 `cdsl check`（exit 0、零诊断）并在 `cdsl run` 中静默回退为 `stop/1000`，且该回退值直接参与波形合成（污染 PULSE `tr`/`tf` 下限），没有任何 warning 提示用户其声明被丢弃。

---

## 6. 额外验证：`output_interval: 0.s`

| 项目 | 实测结果 |
|---|---|
| `cdsl check zero-interval.cdsl` | 退出码 **0**，stderr **0 字节**（无诊断） |
| `cdsl run --experiment zero` | 退出码 **0**，stderr 0 字节，**2015** 点 |
| 输出 CSV SHA256 | `348973A51D487072DFDCD93460796B3ACC44A17039959919C65309F7FDEFCA96` —— 与 `fine`、`neg` **三者逐字节相同** |

⇒ `0.s` **也被接受**，且与 `-1.ns` 落入同一分支（`filter(|s| *s > 0.0)` 为假 ⇒ `span/1000`），属与 P1-b 同一缺陷类，修复时应一并覆盖（`0` 应报错或明确文档化为“使用默认”）。

---

## 7. 完整产物清单与哈希（复跑可校验）

输入（`target/round2-evidence/repro/`）：`pulse-fine.cdsl`、`pulse-coarse.cdsl`、`negative-interval.cdsl`、`zero-interval.cdsl`、`fallback-probe-neg.cdsl`、`fallback-probe-control.cdsl`、`logs/*.stdout.txt`、`logs/*.stderr.txt`。

输出 CSV 的 SHA256：

| 文件 | SHA256 | 点数 |
|---|---|---|
| `out-fine\fine.tran1.csv` | `348973A51D487072DFDCD93460796B3ACC44A17039959919C65309F7FDEFCA96` | 2015（行数 2016 含表头；50ns 最近点在第 65 行） |
| `out-coarse\coarse.tran1.csv` | `FE133BDC297A2D68CEB1B91ADE50A784905A92FFD19B4ECD62F87CBF6546FAC3` | 2015 |
| `out-neg\neg.tran1.csv` | `348973A51D487072DFDCD93460796B3ACC44A17039959919C65309F7FDEFCA96` | 2015 |
| `out-zero\zero.tran1.csv` | `348973A51D487072DFDCD93460796B3ACC44A17039959919C65309F7FDEFCA96` | 2015 |
| `out-probe-neg\negprobe.tran1.csv` | `14D47F613F87A6C9BB18D5368C3DE2CD3F0FFDC232CAA21D55862F20FD945A8B` | 225 |
| `out-probe-control\ctrlprobe.tran1.csv` | `7C0CD8AF1724CD5D224351F19AC26102FB386610C006105224E1592FDE111596` | 228 |

独立复跑一致性（同参数再跑到 `out-neg2`/`out-zero2`/`out-fine-verbose`）：三者与首次产物 **逐字节相同** ⇒ 结果确定、可复现。

CSV 表头与首行（上述四份 2us 实验全部相同）：`time,v(vin),v(out)` / `0,0,0`。

---

## 8. 与上一轮证据的交叉核对（不覆盖旧证据）

`target/next-round-review/` 全程**未被我写入**（其 mtime 20:24–20:25 为他人写入）。把我独立重跑的产物与其中的历史 CSV 做哈希对比：

| 文件对 | 历史 SHA256 | 本轮 SHA256 | 是否相同 |
|---|---|---|---|
| `next-round-review\fine\fine.tran1.csv` ↔ `round2-evidence\repro\out-fine\fine.tran1.csv` | `348973A5…CA96` | `348973A5…CA96` | **相同** |
| `next-round-review\coarse\coarse.tran1.csv` ↔ `round2-evidence\repro\out-coarse\coarse.tran1.csv` | `FE133BDC…FAC3` | `FE133BDC…FAC3` | **相同** |

⇒ 本轮基线与上一轮证据一致，P1-a 在这段时间窗内未被修复、也未漂移。（上一轮 `next-round-review` 中存在 `negative-interval.cdsl` 但**没有对应输出目录**，即 P1-b 此前只有 `check` 层面证据，无 run 产物；本轮补齐了 run 产物与回退判别探针。）

---

## 9. 未验证项（明确边界）

1. **修复后行为未验证**：本轮是修复前基线，未做任何源码改动；修复后的期望值（例如 `coarse` 在真正重采样后应约 21 点、`v(vin)` 边沿应保持 10ns）**尚未实测**，仅建议作为修复验收标准，不作为结论。
2. 只验证了 `v()` 电压探针；`i()`、`w()`、`.meas`/`measure` 在本轮输入中未使用，未验证。
3. 只验证了文件模式 `cdsl run`；**REPL 模式未验证**。
4. 只验证 `--format csv`，`--format json` 的 run 产物未验证。
5. 未验证粗输出采样是否影响 `measure avg/rms`（属 R2/product-path 范围）。
6. 未验证其他非法值形态（`NaN`、`inf`、极小正数、非数值字面量）的接受/拒绝行为——仅验证了 `-1.ns` 与 `0.s`。
7. 未验证 release profile、其他后端（仅 thevenin 0.5.0）与并发 build 锁场景（本次构建未遇到 "Blocking waiting for file lock"）。
8. `cargo test --workspace`（主代理报 413 passed）不属本任务，未复跑。

---

## 10. 结论

**PASS（复现成立）**

- **P1-a 复现成立**：仅改 `output_interval` 1ns→100ns，约 50ns 处 `v(vin)` 由 `1` V 变为 `0.5002375000000003` V、`v(out)` 由 `0.0004449005776724958` V 变为 `0.00012509791368610083` V，返回点数两者均 2015；根因可见 `thevenin.rs:771-782`（`output_interval` → `Tran.step`），PULSE 边沿被拉宽到 100ns（首次 `v(vin)=1` 在 `100 ns`；`fine` 为 `10 ns`）。
- **P1-b 复现成立**：`output_interval: -1.ns` 下 `cdsl check` exit **0** 且**零诊断**，`cdsl run` exit **0**，产物与 1ns 逐字节相同；回退判别探针证明回退值 `span/1000 = 20 ns` 真实到达引擎并把声明 `rise=10.ns` 静默拉宽到 20ns。
- **额外**：`output_interval: 0.s` 同样被接受（check/run 均 exit 0、零诊断、产物相同），属同一缺陷类，修复时需一并处理。
- **副产物（影响修复验收口径）**：当前 `output_interval` 对返回点数**无任何影响**（1ns/100ns/-1ns/0s 全部 2015 点），即它现在既不是输出采样间隔，也不是采样开关，只污染边沿。
