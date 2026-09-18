# CLI 质量保证报告（独立 QA，a13-cli-qa）

本报告的全部结论来自 **真实命令行**：直接执行 `target/release/cdsl.exe`，不使用 `cargo run`，不接受任何人的「通过」转述。
每个用例的原始 stdout / stderr / 结果文件保存在 `docs/review-evidence/cli-qa-output/` 下，报告中只引用关键行。

## 1. 审查范围与版本

| 项 | 值 |
|---|---|
| 仓库 | F:\codexprojects\dsl000 |
| `git rev-parse HEAD` | `cb5d8a212f66922181580900a05fb3d42abe32f2`（工作区不干净，快照见 `cli-qa-output/_git-status-at-qa.txt`） |
| 被测二进制 | `target/release/cdsl.exe` |
| 二进制 `Get-FileHash -Algorithm SHA256` | `A830BADA6DABFB9E03D531CE60B6E188F84FB45B88FC0384573F3321FBBF2DD0`（QA 开始与结束时一致） |
| 构建 | `cargo build --release` → `exit 0`（`_build-release.log`）；QA 结束时复跑仍为 `Finished release profile in 0.19s`（无重编译），确认二进制对应当前源码 |
| 运行环境 | Windows + PowerShell；用例经 `cmd /c` 调用真实二进制，逐条记录 `$LASTEXITCODE` |

范围：`--version` / `capabilities[--verbose]` / `check` / `run` 的成功与失败路径；REPL 仅做非 TTY 探针。
方法：每个用例把 stdout、stderr 重定向到独立文件后单独取退出码；成功路径的数值从写出的 CSV/JSON **读回核对**，tran 的 `measure` 行按 `docs/language.md` 定义的算法（max 取 |x| 最大；avg = ∫x dt / ∫dt；rms = sqrt(∫x² dt / ∫dt)，梯形法）**独立重算**后再比对。

## 2. 用例结果总表

| # | 命令（相对仓库根，二进制省略为 cdsl.exe） | 退出码 | 结论 |
|---|---|---|---|
| M1 | `cdsl.exe --version` | 0 | 输出 `cdsl 0.1.0`（与 README:165 一致） |
| M2 | `cdsl.exe capabilities` | 0 | 10 行，与 README:171-179 逐字一致 |
| M3 | `cdsl.exe capabilities --verbose` | 0 | 追加 `limits:` 6 行（max devices/nodes/loop iterations 100000、depth 16、sweep points 1000000、result values 50000000），与 README:110 一致 |
| S1 | `run examples/voltage_divider.cdsl --experiment divider --out .../divider --format both` | 0 | CSV + JSON 均写出；数值见 §3 |
| S2 | `run examples/rc_filter.cdsl --experiment response --out .../rc_filter --format csv` | 0 | op1 + ac1(121 点) + tran1(1015 点) + 3 条 measure |
| S3 | `run examples/diode_rectifier.cdsl --experiment forward_drop --out .../diode --format csv` | 0 | 非线性 OP + dc1 扫描 11 点 |
| S4 | `run examples/diode_rectifier.cdsl --experiment rectified --out .../diode-tran --format csv` | 0 | 非线性 tran 1008 点 + 2 条 measure |
| S5 | `check examples/voltage_divider.cdsl` / `--verbose` | 0 / 0 | stdout 相同；`--verbose` 仅在 stderr 多一行 `checked ... successfully` |
| S6 | `run examples/diode_rectifier.cdsl --out ... --format csv`（缺 `--experiment`） | 1 | `E_ARGUMENT`：defines 2 experiments; choose one with --experiment（列出 rectified, forward_drop） |
| P1 | `run .../inputs/open_output.cdsl --experiment open_op --out .../open-output/results --format csv` | **0** | **合法开路输出被接受**；CSV `v(a),v(b),i(v1)` = `1,1,0`，stderr 为空 |
| F1 | `check .../inputs/does_not_exist.cdsl` | 1 | `error[E_IO]: cannot read ... (os error 2)` |
| F2 | `check .../inputs/syntax_error.cdsl` | 1 | `error[E_SYNTAX]` + 位置 + 脱字符 |
| F3 | `check .../inputs/dimension_error.cdsl` | 1 | `error[E_DIMENSION]: r1.value needs ohm, found s` + expected/received |
| F4 | `check .../inputs/floating_capacitor.cdsl` | 1 | `error[E_NAME]` + no DC path to ground + 阻断器件 + 源码位置 |
| F5 | `check .../inputs/island.cdsl` | 1 | 两条 `error[E_NAME]`，分别指向 a 与 b |
| F6 | `run examples/voltage_divider.cdsl ... --out .../fail-write/blocker/sub`（blocker 是普通文件） | 1 | `error[E_IO]: cannot create ... (os error 183)` |
| F7 | `run examples/voltage_divider.cdsl ... --out examples/voltage_divider.cdsl` | 1 | `error[E_IO]: cannot create 'examples\voltage_divider.cdsl' (os error 183)`；源文件未被修改 |

## 3. 成功路径真实数值（读回文件核对）

**S1 voltage_divider（`divider.op1.csv` 原样 2 行）**

```
v(in),v(out),i(r1),i(v1)
5,3,0.002,-0.002
```

任务书要求 v(in)=5、v(out)=3、i(r1)=0.002、i(v1)=-0.002 —— **四项精确相符**。
`divider.op1.json` 同时确认单位 V/V/A/A 与 `backend: thevenin 0.5.0`、`adapter 0.1.0`，无 diagnostics。

**S2 rc_filter**

- `response.op1.csv`：`v(vin),v(vout),i(input),i(r1)` = `0,0,0,0`（示例输入 dc 为 0 V，OP 全零，与源文件一致）。
- `response.ac1.csv`：121 行数据（stdout 声明 121 points），频率 10 Hz → 9.9999999999999 MHz，与 `ac from: 10.Hz, to: 10.MHz, points_per_decade: 20`（6 个十倍程 → 121 点）相符。
- `response.tran1.csv`：1015 行数据（stdout 声明 1015 time points），时间 0 → 500 µs（`tran stop: 500.us`），时间轴严格单调递增。

| measure | stdout 实测 | 从 CSV 独立重算 | 相对差 |
|---|---|---|---|
| vfinal (max) | 0.9932452475172647 V | 0.993245247517265 | ~2e-16 |
| vavg (梯形积分) | 0.8008509267600744 V | 0.800850926760074 | ~5e-16 |
| vrms (梯形积分) | 0.8379723543524203 V | 0.83797235435242 | ~4e-16 |

**S3 diode forward_drop（非线性 OP）**：`v(vin)=5`，`v(vout)=0.6928715252252958`，`i(r1)=0.004307128474774704`；与 `examples/diode_rectifier.cdsl` 注释中的 0.692872 V 相符；另产出 `forward_drop.dc1.csv`（11 点扫描）。

**S4 diode rectified（非线性 tran）**：1008 点；`vpeak = 4.3071284013527 V`（独立重算相同）、`vavg = 1.2684005202349173 V`（独立重算 1.26840052023492，相对差 ~3e-15）。

**P1 合法开路输出（本轮纠错的关键正例）**
```
$ cdsl.exe run docs\review-evidence\cli-qa-output\inputs\open_output.cdsl --experiment open_op --out ... --format csv
experiment 'open_op' on circuit 'open' (backend thevenin 0.5.0)
  op1: scalar; signals: v(a), v(b), i(v1)
  wrote ...\open_op.op1.csv
exit=0    stderr 为空
CSV: v(a),v(b),i(v1) / 1,1,0
```
即：节点 b 只由 r1 与 a 相连、不接地，仍被正确接受，v(b)=1 V、i(v1)=0 A —— 与「真正无参考网络被拒绝」（F5）形成对照，两条路径都用真实 CLI 验证。

## 4. 失败路径诊断摘要（关键行）

- **F1 E_IO**：`error[E_IO]: cannot read '...does_not_exist.cdsl': 系统找不到指定的文件。 (os error 2)`
- **F2 E_SYNTAX**：`error[E_SYNTAX]: unexpected identifier 'n'; expected the end of the statement` + `--> ...syntax_error.cdsl:4:23` + 脱字符指向 `n`
- **F3 E_DIMENSION**：`error[E_DIMENSION]: 'r1.value' needs ohm, found s` + `= expected: ohm` / `= received: s` + 位置 5:40
- **F4 浮空（只经电容）**：
  `error[E_NAME]: node 'out' has no DC path to ground, so its operating point is undefined`
  `--> ...floating_capacitor.cdsl:4:13`（指向 `node :in, :out` 的 `out`，带脱字符）
  `= a capacitor or current source does not provide a DC reference path; ...`
  `= attached but not conducting at DC: c1`
  → 任务书要求的四要素（E_NAME、no DC path to ground、阻断器件名、源码位置）**全部满足**，退出码 1。
- **F5 孤立电阻网络**：两条 E_NAME，分别 `node 'a' ...`（3:8）与 `node 'b' ...`（3:12），都带源码位置与脱字符，退出码 1。
- **F6/F7 I/O**：均为 `error[E_IO]` + `(os error 183)`，退出码 1。

## 5. 发现（严重性 + 文件位置 + 最小复现）

### F-1（LOW，文档与实现不符）README 声明的退出码 2 无任何实现
- 位置：`README.md:182`（退出码契约）；`crates/circuit-cli/src/main.rs:26`（`EXIT_INTERNAL: u8 = 2`）。
- 证据：全仓库 grep `EXIT_INTERNAL` 只有该定义一处，**零调用点**；`check/run/repl` 的失败分支全部返回 `EXIT_USER_ERROR(1)`（`check.rs:35,42,49,64`；`run.rs:47,57,67,91,125`；`repl.rs:125,145,204`）。
- 实测：本报告 7 条失败路径（含 I/O 失败）退出码**全是 1**，未观察到 2。
- 最小复现：`target\release\cdsl.exe run examples\voltage_divider.cdsl --experiment divider --out docs\review-evidence\cli-qa-output\fail-write\blocker\sub --format csv` → `exit 1`（README 归类为「内部错误」的 I/O 失败也是 1）。
- 影响：只影响文档承诺，不影响成功/失败判定；若外部脚本按 README 依赖「2=内部错误」分支，将永远走不到。

### F-2（LOW，诊断文案）拒绝覆盖输入源的护栏在该路径上不触发
- 位置：`crates/circuit-cli/src/main.rs:142-156`（`guard_output`，文案 `refusing to write results over the input file`）。
- 实测：`--out` 指向输入源文件时，先做目录创建即失败，报 `error[E_IO]: cannot create 'examples\voltage_divider.cdsl' (os error 183)`，护栏文案不出现。
- 安全：源文件**未被修改**（QA 前后 `git status --porcelain` 快照一致，`examples/voltage_divider.cdsl` 未出现在变更列表中）。
- 最小复现：命令 F7（退出码 1）。
- 影响：安全性无问题，只是用户看到的提示与设计意图不完全对应。

### F-3（INFO，非产品缺陷）E_IO 文本在 PowerShell 控制台显示为乱码
- 现象：`Get-Content` 默认按 ANSI 解码显示为 `绯荤粺鎵句笉鍒版寚瀹氱殑鏂囦欢`。
- 核实：`[System.IO.File]::ReadAllBytes` 首字节为 `65 72 72 6f 72 5b 45 5f 49 4f 5d`（`error[E_IO]`），用 `Get-Content -Encoding utf8` 显示正常 —— 输出是合法 UTF-8，属控制台解码假象，不需修改产品。

### F-4（INFO）`check --verbose` 的可观察差异
- `check.rs:67-69`：成功时仅在 **stderr** 追加 `checked '<file>' successfully`；stdout 与不带 `--verbose` 逐字相同。`capabilities --verbose` 才会在 stdout 追加 `limits` 段。与 README 的表述不冲突，记录以备后续文档细化。

## 6. 结论

**PASS**（成功路径与失败路径均符合任务书与 README 的主要声明），附带两条 LOW 级 NEEDS_FIX：

- 必须成功的正例 P1（合法开路输出）在真实 CLI 上 **exit 0 且 CSV = `1,1,0`**；真正的无参考网络 F5 **exit 1 且同时指向 a 与 b** —— 本轮纠错的数值/语义区别在命令行上成立。
- 示例数值与文档承诺一致（含独立重算的 measure），未发现数值错误。
- NEEDS_FIX-1 = F-1（退出码 2 的契约无实现，建议 README 收窄措辞或补实现）。
- NEEDS_FIX-2 = F-2（护栏文案不可达，建议在创建目录前先判定输出路径是否等于输入）。
- 无 BLOCKED 项。

## 7. 未验证项与限制

1. **REPL 真交互（需要 TTY）**：仅做了非 TTY 探针 `echo :quit | cdsl.exe repl` → `exit 0`，stdout 只有 `cdsl> `；提示符编辑、历史、补全、`:load/:run/:reset/:help` 等交互行为**未验证**。
2. **退出码 2**：未能触发（源码中无返回点，见 F-1），因此「2 = 内部错误」未被实测证实也未被证伪，只是**不可达**。
3. **非 Windows 平台**：`capabilities` 自述 `Verified on Windows MSVC only`；本次 QA 仅在 Windows 上执行。
4. **`run --format json` 单独模式**：divider 用 `both` 覆盖了 json 产出，未单独验证 `--format json` 只写 json、不写 csv。
5. **`check --json`** 未执行（不在任务清单内）。
6. **`forward_drop.dc1.csv`（11 点扫描）** 已产出但未逐点核对物理值；`rectified` 的 measure 已独立重算。
7. **并发、长时稳定性、性能**未测；其他代理可能在 QA 期间继续修改源码——本报告对应 `git rev-parse HEAD` + 二进制哈希 + `_git-status-at-qa.txt` / `_git-status-final.txt`（两者一致）。

## 8. 证据文件索引（`docs/review-evidence/cli-qa-output/`）

- `_build-release.log`、`_git-status-at-qa.txt`、`_git-status-final.txt`
- `meta/`：version、capabilities、capabilities-verbose 的 stdout/stderr
- `divider/`、`rc_filter/`、`diode/`、`diode-tran/`：stdout/stderr + 结果 CSV/JSON
- `check-divider/`：check 与 check --verbose 的 stdout/stderr
- `open-output/`：合法开路输出的 stdout/stderr + `results/open_op.op1.csv`
- `fail-missing-file/`、`fail-syntax/`、`fail-dimension/`、`fail-floating/`、`fail-island/`、`fail-write/`：失败路径的 stdout/stderr
- `inputs/`：五个 QA 输入（syntax_error / dimension_error / floating_capacitor / open_output / island .cdsl）
- `repl-nontty/`：REPL 管道探针的 stdout/stderr
