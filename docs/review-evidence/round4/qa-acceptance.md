# Round 4 QA 独立验收（task-5）

Owner：`qa-worker`。写集：仅新增 `crates/circuit-cli/tests/r4_*.rs`、
`crates/circuit-session/tests/r4_*.rs`、`crates/circuit-results/tests/r4_*.rs`，以及本文件。
没有修改任何生产代码、已有测试文件或 `Cargo.toml`；没有 `git commit/push/reset/checkout/restore`，
没有运行 `cargo fmt`。

依据：`docs/round3-review-and-round4-plan.md` §4 验收矩阵、`ROUND4_AGENT_TEAM_PROMPT.md`、
`docs/review-evidence/round4/design-contract.md` §1.3–§1.5。

## 1. 结论

| 门禁 | 结果 |
|---|---|
| 6 个新增测试目标（debug） | **PASS**，42 passed / 0 failed（日志 `target/round4/qa/qa-final-debug.log`，`OVERALL_EXIT=0`） |
| R4-02 定向 release 门禁 | **PASS**，`r4_repro_cli` release 8 passed / 0 failed（`target/round4/qa/release-repro-test.log`） |
| 四个复现输入走真实 CLI（debug） | **PASS**，全部 exit 1 + 结构化诊断，不产生输出文件（`target/round4/qa/postfix-cli.md`） |
| 四个复现输入走真实 CLI（release） | **PASS**，全部 exit 1，不产生输出文件（`target/round4/qa/release-cli.md`） |
| 新增测试目标的 clippy | **PASS**，`cargo clippy -p … --test r4_* -- -D warnings` 三组全部 exit 0（`target/round4/qa/qa-clippy.log`） |
| 新增测试文件格式 | 6 个新文件 `rustfmt --edition 2024 --check` exit 0（只对这 6 个写集内新文件做了有界格式化；未运行 `cargo fmt`，未触碰任何他人文件） |
| 修复前 RED 证据 | 已保留：修复前二进制 `target/round4/qa/prefix-cdsl.exe` 的输出在 `target/round4/qa/red-prefix-cli.md` |
| 遗留缺陷 | 1 项，非本轮范围、已由 lead 裁决（§6） |

没有使用 epsilon、饱和、跳过样本或 `catch_unwind`；所有容差都在测试注释里给出了推导。

## 2. 交付的测试文件

| 文件 | 测试数 | 覆盖 |
|---|---|---|
| `crates/circuit-results/tests/r4_expr_policy.rs` | 16 | 求值器值域策略：sqrt(-1)、min/max 掩盖、1e308*1e308、溢出除法、非有限实数/复数样本、128 因子量纲溢出、量纲边界精确性、合法算术不被误杀、`EvalSite` 具名站点、`is_constant`/`eval_constant`、check 期与运行期诊断一致 |
| `crates/circuit-session/tests/r4_export_diagnostics.rs` | 7 | 手工构造 NaN/±inf Dataset 的 CSV 空单元格 / JSON null 渲染、warning 非空、两种格式诊断集合一致、`write_datasets` 返回 `Written{paths,warnings}` 且按 dataset 去重、拒绝回调不落盘 |
| `crates/circuit-cli/tests/r4_repro_cli.rs` | 8 | 四个复现输入的真实 CLI 验收（退出码、诊断类别/键、无成功测量、无输出文件）、debug+release 双跑、合法长乘积正控制、合法表达式正控制 |
| `crates/circuit-cli/tests/r4_cli_repl_parity.rs` | 3 | CLI 与 REPL（`circuit_session::Session` 与真实 `cdsl repl` 二进制）同一表达式的数值、CSV 字节、错误类别与文本、上下文键一致 |
| `crates/circuit-cli/tests/r4_output_integrity.rs` | 3 | 同一实验"先有效 derive 后非法 derive"（运行期失败 + check 期失败）不产生新文件、旧文件不被删除/覆盖；成功运行对照组 |
| `crates/circuit-cli/tests/r4_regressions.rs` | 5 | 旧功能回归：RC 截止频率/增益、电阻功率、多分析绑定、隐式探针、`output_interval` 重采样 |

运行方式（只跑局部命令，release 构建一次）：

```powershell
cargo test -p circuit-results --test r4_expr_policy
cargo test -p circuit-session --test r4_export_diagnostics
cargo test -p circuit-cli    --test r4_repro_cli
cargo test -p circuit-cli    --test r4_cli_repl_parity
cargo test -p circuit-cli    --test r4_output_integrity
cargo test -p circuit-cli    --test r4_regressions
cargo build --release -p circuit-cli
cargo test --release -p circuit-cli --test r4_repro_cli
```

合并日志（一次脚本顺序执行上面 6 条 debug 命令）：
`target/round4/qa/qa-final-debug.log`；脚本 `target/round4/qa/qa-final-debug.ps1`。
实测输出：

```text
exit=0 :: -p circuit-results --test r4_expr_policy          :: 16 passed; 0 failed
exit=0 :: -p circuit-session --test r4_export_diagnostics   ::  7 passed; 0 failed
exit=0 :: -p circuit-cli     --test r4_repro_cli            ::  8 passed; 0 failed
exit=0 :: -p circuit-cli     --test r4_cli_repl_parity      ::  3 passed; 0 failed
exit=0 :: -p circuit-cli     --test r4_output_integrity     ::  3 passed; 0 failed
exit=0 :: -p circuit-cli     --test r4_regressions          ::  5 passed; 0 failed
OVERALL_EXIT=0
```

release 门禁（`target/release/deps/r4_repro_cli-*.exe`）：

```text
running 8 tests ... test result: ok. 8 passed; 0 failed; 0 ignored
```

## 3. 红线矩阵

每行给出：命令 → 观察结果 → 判定。

### 红线 1：四个复现输入走真实 CLI（`std::process::Command` + `env!("CARGO_BIN_EXE_cdsl")`）

输入文件由测试生成在 `target/round4/qa/r4_*/…`（`r4-01-sqrt.cdsl`、`r4-01-mul.cdsl`、
`r4-02-64.cdsl`、`r4-02-128.cdsl`；128 因子表达式由 128 个 `v(:vin)` 用 ` * ` 连接生成）。

| 场景 | 命令（`cdsl` = 真实二进制） | 观察结果 | 判定 |
|---|---|---|---|
| `sqrt(-1)` 常量 | `cdsl check …\r4-01-sqrt.cdsl` | exit **1**，`error[E_VALUE]`，含 `sqrt`、`= signal:`、`= index: 0`、`= derive: invalid`、`= measure: masked` | PASS |
| `sqrt(-1)` 运行 | `cdsl run … --experiment sqrt_repro --out <dir> --format csv` | exit **1**（非 101、无 `panicked`），`E_VALUE` + `= analysis: op1`/`= kind: op`/`= signal: invalid`/`= sample:`/`= index: 0`/`= expression: sqrt(-1)`；stdout 无 `measure masked`、无 `wrote`；输出目录为空 | PASS |
| `1e308 * 1e308` 常量 | `cdsl check …\r4-01-mul.cdsl` | exit **1**，`E_VALUE`，含 `*`、`= measure: masked` | PASS |
| `1e308 * 1e308` 运行（`min` 嵌套） | `cdsl run … --experiment mul_repro …` | exit **1**，`E_VALUE`，`= expression: (1e308 * 1e308)`；无 `measure masked`；无输出文件 | PASS |
| 128 因子量纲溢出 | `cdsl check …\r4-02-128.cdsl` | exit **1**，`E_DIMENSION`，**无** `E_LIMIT`，有 `-->` 位置与 `^^^^` 下划线 | PASS |
| 128 因子量纲溢出 | `cdsl run … --experiment dim_overflow …` | exit **1**（非 101），stderr 无 `attempt to add with overflow`，`E_DIMENSION` + `-->`；无 `measure m`；无输出文件 | PASS |
| 正控制 | `cdsl run` 64 因子 `v(:vin)` 乘积 | exit **0**，写出 `dim_overflow.op1.csv`（证明不是"长度上限"） | PASS |
| 正控制 | `cdsl run` 合法 derive/measure | exit **0**，`measure masked = 2`，CSV 含 `half` 列 | PASS |

修复前同一组输入的对照（RED 证据，`target/round4/qa/red-prefix-cli.md`，用的是本轮开始时保留的
修复前二进制 `target/round4/qa/prefix-cdsl.exe`，mtime 2026-09-18 22:35）：

| 输入 | 修复前 | 修复后 |
|---|---|---|
| `check` r4-01 | exit 0 | exit 1，`E_VALUE` ×2（derive 与 measure 各一条） |
| `run` bad_sqrt | exit 0，`measure masked = 2 dimensionless`，写出 `bad_sqrt.op1.csv` | exit 1，`E_VALUE`，无文件 |
| `run` overflow | exit 0，同上是 2 与 null，写出文件 | exit 1，`E_VALUE`，无文件 |
| `check` r4-02 | exit 0 | exit 1，`E_DIMENSION` |
| `run` dim_overflow | exit **101**，`panicked at crates/circuit-core/src/units.rs:49: attempt to add with overflow` | exit **1**，`E_DIMENSION`，无 panic |

修复后直接走真实二进制的完整证据：`target/round4/qa/postfix-cli.md`（debug）与
`target/round4/qa/release-cli.md`（release，5 次调用全部 exit 1，`postfix-out`/`release-out`
目录未被创建）。

### 红线 2：debug 与定向 release（不得 panic、不得静默回绕）

| 观察 | 结果 | 判定 |
|---|---|---|
| debug：128 因子 `run` | exit 1 + `E_DIMENSION`，无 101/无 panic 文本 | PASS |
| release：`cargo build --release -p circuit-cli` 后直接跑 `target/release/cdsl.exe` | 5 次调用全部 exit 1，无 panic，无输出目录 | PASS |
| release：`cargo test --release -p circuit-cli --test r4_repro_cli` | 8 passed / 0 failed | PASS |

"不得静默回绕"由断言本身保证：release 若回绕，`run` 会 exit 0 并打印 `measure m`，
而测试要求 exit 1 + `E_DIMENSION` 且 stdout 不含该测量。

### 红线 3：R4-03 导出诊断（手工 Dataset，独立于 R4-01）

构造 `Axis::Time([0, 1e-3, 2e-3, 3e-3])`、`v(out) = [1, NaN, +inf, -inf]`、
`v(mid) = [(0,1), (inf,0), (NaN,NaN), (1,0)]` 的合法 Dataset。

| 观察 | 结果 | 判定 |
|---|---|---|
| `to_csv_with_diagnostics` | 文件保持约定空单元格（`0,1,0,1` / `0.001,,,0` / `0.002,,,` / `0.003,,1,0`），文本中无 `NaN`/`inf`；返回 2 条 warning，各点名一个信号 | PASS |
| `to_json_with_diagnostics` | 非有限分量各自为 `null`（复数的实部为 `null` 时虚部仍保留 `0.0`），文件 `diagnostics` 数组仍是 dataset 自己的；返回同样的 2 条 warning | PASS |
| 两种格式诊断一致 | `render_plain()` 列表逐一相等（去重的前提） | PASS |
| `write_datasets(Format::Both)` | 返回 `Written{ paths: 2 个文件, warnings: 2 条 }`（CSV+JSON 不重复报告，`warning_lines()` 为 2 行 `  warning[…`），落盘文本与渲染器输出逐字节一致 | PASS |
| 有限 dataset | 0 条 warning；`write_datasets` 仍写 2 个文件 | PASS |
| 回调拒绝 | 首次 `approve` 返回 Err 即停止，目录中没有文件 | PASS |

### 红线 4：CLI 与 REPL 同一表达式一致

| 比较项 | 结果 | 判定 |
|---|---|---|
| 数值（`measure ratio`） | CLI stdout 与 Session 回复的 measure 行**逐字符相同**，且为独立推导的 `-2`（`v(pos)/v(neg) = 1/(-0.5)`） | PASS |
| 导出文件 | CLI 与 Session 写出的 `num.op1.csv` **字节相同**；derive 列 `mid = 0.625 V` 与手算一致 | PASS |
| 错误类别 | CLI 与 Session 都是 `E_VALUE` | PASS |
| 错误文本 | `error[…]: …` 首行逐字符相同；`= …` 上下文键列表也相同（含 `= analysis: op1`） | PASS |
| 真实 `cdsl repl` 二进制 | stdin 脚本 `:load` + `:run` 得到同样的 `-2`，CSV 与 Session API 写出的字节相同 | PASS |
| 失败不写文件 | 两条路径的输出目录都为空 | PASS |

### 红线 5：同一实验"先有效 derive 后非法 derive"

| 场景 | 结果 | 判定 |
|---|---|---|
| 运行期失败（`derive :good` 先求值，`measure :boom, max: sqrt(v(:neg)/v(:pos))` 后失败） | exit 1 + `E_VALUE`，stdout 无 `wrote`、无 `measure boom`；预置的 `runtime_fail.op1.csv`/`.json` 内容仍是 `OLD CSV` / `OLD JSON` 哨兵，文件数仍是 2 | PASS |
| check 期失败（`derive :good` 后接常量 `sqrt(-1)`） | `run` exit 1 + `E_VALUE`；`check` 也 exit 1；预置文件未被改写、未新增 | PASS |
| 对照组（同一 harness） | 合法实验 exit 0、打印 `wrote`、用新结果替换哨兵文件（`good` 列 = 0.625） | PASS |

### 红线 6：旧功能回归（独立数值参考）

| 场景 | 独立参考（推导写在测试注释里） | 观察 | 判定 |
|---|---|---|---|
| RC 截止频率/增益 | `H = 1/(1+jωRC)`；`fc = 1/(2π·1k·100n) = 1591.5494309189535 Hz`；每个频点断言 `Re H = 1/(1+x²)`、`Im H = -x/(1+x²)`、`gain_db = -10log10(1+x²)`（x=ωRC），角点处 `0.5 - 0.5j`、`-3.010299956639812 dB` | 全部 `< 1e-9`；角点行精确存在；`v(vin) = 1∠0` | PASS |
| 电阻功率 | 5 V / (1k+1.5k) → `I = 2 mA`、`v(out) = 3 V`；`P(r1) = I²R = 4e-3 V*A`、`P(r2) = 3²/1.5k = 6e-3 V*A`、总功率 = 5 V·2 mA = 10 mW | 4e-3 / 6e-3 / 1e-2，误差 `< 1e-15`；`measure itotal = 0.002` | PASS |
| 多分析绑定 | op 下电容隔直 → `v(out) = 1 V`；AC 下 `|H(1 kHz)| = 1/sqrt(1+(2π·1k·1e-4)²)` | `binding.op1.csv` 只有 `dc_gain`（=1），`binding.ac1.csv` 只有 `ac_mag`（=公式值，`<1e-9`），两者相差 `>0.1` | PASS |
| 隐式探针 | 只 `save v(:out)`，表达式读 `v(:in)` | CSV 恰好两列 `v(out)`+`ratio`，**没有** `v(in)` 列；`ratio = 5/3`（`<1e-12`）；`check --json` 的 `implicit_probes` 含 `v(in)` | PASS |
| `output_interval` 重采样 | τ=RC=100 µs、T=2 ms：网格点 `0,250µs,…`；`v(t)=1-e^{-t/τ}`；`avg = 1-(τ/T)(1-e^{-T/τ}) = 0.95`、`max = 1-e^{-20}` | 输出网格为 250 µs 等间隔且保留 2 ms 末点；每点与解析解 `<1e-3`（插值误差 bound `max_step²/8·max|v''| ≈ 1.25e-7`）；`vavg`/`vmax` 行与无 `output_interval` 的运行**逐字符相同**；无 interval 的原始网格非均匀 | PASS |

## 4. 测试敏感性（不是"只统计新增测试"）

* 每个失败路径都断言**具体诊断类别**（`E_VALUE` / `E_DIMENSION`）与诊断键，而不是 `is_err()`。
* 每个"不该失败"的场景都有正控制（合法 64 因子乘积、合法 derive/measure、有限 dataset、成功 run 覆盖旧文件），
  因此 fixture 解析失败或环境问题不会让失败用例"因错误原因通过"。
* CLI 数值断言来自独立推导（KCL、`H(jω)`、`I²R`、指数积分），不是抄自现有测试常量；
  一致性断言用**逐字符/逐字节相等**，不用 epsilon。

## 5. 证据文件索引

| 文件 | 内容 |
|---|---|
| `target/round4/qa/red-prefix-cli.md` | 修复前二进制对四个复现输入的输出（RED） |
| `target/round4/qa/prefix-cdsl.exe` | 保留的修复前 debug 二进制（仅作证据，不参与构建） |
| `target/round4/qa/postfix-cli.md` | 修复后 debug 真实 CLI 的 5 次调用 |
| `target/round4/qa/release-cli.md` | 修复后 release 真实 CLI 的 5 次调用 |
| `target/round4/qa/release-repro-test.log` | `cargo test --release -p circuit-cli --test r4_repro_cli` |
| `target/round4/qa/qa-final-debug.log` + `.ps1` | 6 个目标的合并 debug 运行记录 |
| `target/round4/qa/run-*.log` | 各目标单独运行日志 |
| `target/round4/qa/probe-depth/`、`depth-probe*.ps1` | 深表达式栈溢出的最小复现与阈值探测 |

## 6. 遗留风险（未在本轮修复）

**预存在的 debug 栈溢出：合法但很深的表达式让 `cdsl run` 直接 abort。**

* 现象：debug 二进制 `run` 时 `thread 'main' has overflowed its stack`，退出码 `-1073741571`
  （`0xC00000FD` STATUS_STACK_OVERFLOW），无诊断、无输出。
* 最小复现：分压电路 + `op`，`derive :huge, expr: <96 个 v(:vin) 用 * 连接>`，`cdsl run f.cdsl --experiment e`。
  生成脚本见 `target/round4/qa/depth-probe3.ps1`，文件在 `target/round4/qa/probe-depth/`。
* 阈值（本机 1 MB 主线程栈）：debug 80 因子 exit 0；**96 与 127 因子 abort**；加法链同样（110/127/200 全部 abort）
  → 与量纲无关，是表达式深度。release 下 64/80/96/127 因子均 exit 0（优化后帧更小），128 因子 exit 1 `E_DIMENSION`。
* `cdsl check` 对同一文件 exit 0（解析、静态分析、打印都正常），崩溃在 run 的执行/求值链路。
* 与本轮的关系：128 因子路径（计划要求的那条）已被静态量纲检查拦下并给出 `E_DIMENSION`，
  本项不影响 R4-01..R4-03 的闭环；但它是本轮两条独立取证（QA 与 reviewer）都命中的真实缺陷。
* 处置：lead 已裁决为**预先存在、超出本轮 R4-01..R4-03 与阶段 B 范围**的独立缺陷，
  统一记录在 `docs/review-evidence/round4/findings.md`；若本轮追加有界修复任务，
  必须保持"128 因子仍是 `E_DIMENSION`（不是 `E_LIMIT`）"与"127 因子不再 abort"两条。
* QA 的测试处理：`r4_repro_cli.rs` 里要求的 128 因子断言保持不变；我的"长但合法"正控制取 64 因子
  （仍证明不是长度上限），并在注释里指向本条风险，避免把该缺陷藏在一个无法通过的用例后面。

其他限制：

* 未覆盖"CLI 端到端打印导出 warning"：需要让求解器产出非有限原始样本的确定性输入；
  渲染器、`write_datasets` 返回值与 `warning_lines()` 已独立覆盖，CLI 打印路径由 session worker 的
  T3 验收与本轮 reviewer 覆盖。
* 未重跑 workspace 级门禁（`cargo test --workspace`、`cargo clippy --workspace`、`cargo fmt --all`）：分工纪律要求由 lead 统一执行。QA 只跑了写集内 6 个测试目标的局部 `cargo clippy … -- -D warnings` 与只读 `rustfmt --check`。
* 只在 Windows + 当前默认工具链上实测；release 阈值已记录，其他平台未验证。

## 8. 验证基线（源文件 SHA256 前 16 位，2026-09-18T15:56:11.599Z）

验收在这些内容上完成：在指纹前后各取一次哈希并保持一致（无并发写入），其间重跑 6 个目标得到
同样的 42 passed / 0 failed。若之后有人再改生产代码，本文件结论即失效，需要重跑 §2 的命令。

| 文件 | SHA256-16 |
|---|---|
| crates/circuit-core/src/units.rs | 921A6AE3F1A89A55 |
| crates/circuit-core/src/plan.rs | 0F8E2F45403C9D94 |
| crates/circuit-core/src/format.rs | ADF067669F830ECB |
| crates/circuit-dsl/src/eval.rs | 4F48F7CB6BC3A628 |
| crates/circuit-dsl/src/elaborate.rs | AAB383334D33F481 |
| crates/circuit-results/src/expr.rs | C5EB03A95BC18F86 |
| crates/circuit-results/src/measure.rs | EC6DFE70123D8692 |
| crates/circuit-results/src/export.rs | DC2BDF4CC019B516 |
| crates/circuit-results/src/dataset.rs | 4D562BBF4364FC57 |
| crates/circuit-session/src/execute.rs | F751C12CAB2707E6 |
| crates/circuit-session/src/session.rs | 46AD618D604D21C3 |
| crates/circuit-cli/src/check.rs | 31DA28A6B0BB28BC |
| crates/circuit-cli/src/run.rs | 5EC5BA98D1EF62E5 |
| crates/circuit-cli/src/repl.rs | 1243D2306042215F |
| crates/circuit-cli/src/main.rs | 6C1B1680156E9C91 |
| crates/circuit-results/tests/r4_expr_policy.rs | B2CCDCB96643767E |
| crates/circuit-session/tests/r4_export_diagnostics.rs | E7A6D900DEF50E99 |
| crates/circuit-cli/tests/r4_repro_cli.rs | 8B573F3FE196EFE1 |
| crates/circuit-cli/tests/r4_cli_repl_parity.rs | 24F367A2E6B93BB2 |
| crates/circuit-cli/tests/r4_output_integrity.rs | B4A1DF3E557A8D9D |
| crates/circuit-cli/tests/r4_regressions.rs | 03EDC8EF76B1A6EF |

