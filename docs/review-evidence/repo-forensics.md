# 仓库取证：可复现基线 + 环境事实（A01）

> 任务：A01 `a01-repo-forensics`（**严格只读**仓库取证）
> 仓库：`F:\codexprojects\dsl000`　分支 `main`　HEAD `cb5d8a212f66922181580900a05fb3d42abe32f2`
> 工具链：rustc / cargo `1.98.1`（`x86_64-pc-windows-msvc`），shell = PowerShell (pwsh)
> 本次唯一写入的工作区文件：本文件 `docs/review-evidence/repo-forensics.md`。未 commit、未 push、未改动任何源码 / Cargo.toml / Cargo.lock / 其他文档。
> 哈希快照时刻：**2026-09-18 19:40:56 +08:00**（§7 的清单对应此版本）
> 证据规则：每条结论都给出**文件路径 + 行号 + 我实际执行的命令 + 真实输出摘要**。转述他人结论或"上游文档声称"一律不作为证据；无法验证的写入 §9「未验证」。

---

## 0. 结论汇总（PASS / NEEDS_FIX / BLOCKED）

| # | 项 | 结论 |
|---|---|---|
| 1 | 开工前用户改动记录（任务项 1） | **PASS** —— 已逐字记录，未触碰、未清理（§1） |
| 2 | 工具链 + workspace 成员（任务项 2） | **PASS**（§3） |
| 3 | `cargo test --workspace` 精确总数（任务项 3） | **PASS：389 passed / 0 failed / 0 ignored，exit code 0**。两次独立运行 + 与 lead 原始捕获交叉核对三方一致（§4） |
| 4 | `_probe` 独立工程构建 / 运行（任务项 4、7） | **PASS** —— `--locked` 与普通 `cargo build` 均 exit 0；`--bin probe`、`--bin robustness` 均 exit 0；构建前后 `Cargo.lock`/`Cargo.toml` 哈希未变（§5） |
| 5 | 新增集成测试能否被自动发现（任务项 5） | **PASS：能**。仓库内**没有** `autotests = false` / `harness` / 显式 `[[test]]`；`tests/adapter.rs`、`tests/elaborate.rs` 已被无条件自动发现为 test target（§6） |
| 6 | SHA-256 冻结清单（任务项 6） | **PASS**（§7，41 + 22 + 7 条完整清单），但 `README.md` 正被 lead 改写 ⇒ **Wave 2 冻结前必须重跑该命令**（§7.4） |
| 7 | 文档 / 证据一致性 | **NEEDS_FIX** —— F1–F4（§8） |
| — | 我可验证范围内是否存在阻塞 | **无 BLOCKED**；未验证项集中在 §9 |

**一句话结论**：`389 / 0 / 0 + exit 0` 这一基线数字独立复核成立，可直接用作 Wave 2 的冻结锚点；但 `_probe` 里两处被称为"浮空节点"的用例，其电路拓扑都**不是**浮空节点（F4），RC 瞬态残差的文字归因与探针代码本身矛盾（F3），且 `docs/testing.md` 内部数字自相矛盾（F1）——这三项正是本轮要修的"错误数值验证证据"。

---

## 1. 审查范围与版本（任务项 1）

### 1.1 工作目录与 git 基线（开工时，我未做任何修改）

```powershell
(Get-Location).Path
git status --porcelain
git log --oneline -3
git diff --stat
git diff --stat --cached
git rev-parse HEAD
git rev-parse --abbrev-ref HEAD
```

真实输出：

```text
F:\codexprojects\dsl000
=== git status --porcelain ===
 M RUST_CIRCUIT_DSL_PROMPT.md
?? AGENT_TEAM_EXECUTION_PROMPT.md
?? agent-team-switch.md
?? docs/prompt-review.md
?? docs/review-evidence/
=== git log --oneline -3 ===
cb5d8a2 Add an interactive REPL, and settle the syntax questions it raised
61cb4d0 Implement circuit-dsl: a Ruby-flavoured DSL for analog circuit simulation
=== git diff --stat ===
 RUST_CIRCUIT_DSL_PROMPT.md | 87 +++++++++++++++++++++++++++++++++++++++++++---
 1 file changed, 83 insertions(+), 4 deletions(-)
=== git diff --stat --cached ===   （无输出）
=== rev-parse HEAD ===
cb5d8a212f66922181580900a05fb3d42abe32f2
=== branch ===
main
```

说明：
- 仓库历史只有 **2 个 commit**（`git log --oneline -3` 只返回 2 行）——不存在更早的"干净基线"可回退。
- 开工时**唯一的跟踪文件改动**是 `RUST_CIRCUIT_DSL_PROMPT.md`（+83 / −4）；其余 4 项为未跟踪文件。这些都是**用户/lead 的既有改动**，我只读、未清理、未格式化。
- `?? docs/review-evidence/` 是 lead 在 Wave 0 建立的证据目录（内含 `baseline.md`、`team-board.md`、`raw-baseline-workspace-test.txt`，见 §7.3 GROUP-C）。
- 与 `docs/review-evidence/baseline.md:8-13` 的开工快照相比，我的快照多出 `?? docs/review-evidence/`：两份快照取样时刻不同，两份都成立，**不构成矛盾**。

### 1.2 本轮审查范围

`docs/testing.md` §7 与 `docs/review-evidence/team-board.md:30-56` 定义的本轮范围（浮空节点证据、RC 瞬态证据、非零相位验证、产品路径回归、文档同步）落在以下文件上，我按只读方式取证：
- 生产代码：`crates/*/src/**`（35 个 `.rs`，§7.1）
- 集成测试：`crates/*/tests/**`（5 个 `.rs`，§7.2）
- 独立验证工程：`_probe/src/**`（3 个 `.rs`）
- 文档：`README.md`、`docs/backend-evaluation.md`、`docs/testing.md`（任务指定）+ 其余 `docs/*.md`（参考）
- 示例：`examples/*.cdsl`（7 个）

---

## 2. 实际阅读 / 执行的清单（命令 + 真实退出码）

### 2.1 执行的命令

| 命令 | 退出码 | 用途 / 报告位置 |
|---|---|---|
| `git status --porcelain` / `git log --oneline -3` / `git diff --stat` | 0 | 用户改动与版本（§1） |
| `rustc --version` / `rustc -vV` / `cargo --version` / `cargo -vV` | 0 | 工具链（§3.1） |
| `cargo metadata --no-deps --format-version 1` | 0 | workspace 成员 / 依赖 / target（§3.2） |
| `cargo test --workspace`（**第一次**，后台作业 `pwsh-3`，作业状态 completed） | **0** | 基线复核（§4） |
| `cargo test --workspace`（**第二次**，单进程捕获全部 487 行输出并程序化求和） | **0** | 精确总数（§4.2–4.3） |
| `cargo test -p circuit-backend --no-run` | 0 | 测试二进制发现证据（§6） |
| `cargo test -p circuit-dsl --no-run` | 0 | 测试二进制发现证据（§6） |
| `cargo metadata --manifest-path _probe/Cargo.toml --no-deps --format-version 1` | 0 | `_probe` 的 bin 清单与依赖约束（§5.2） |
| `cargo build --manifest-path _probe/Cargo.toml --locked` | **0** | 证明 `_probe/Cargo.lock` 已是新鲜一致的（§5.3） |
| `cargo build --manifest-path _probe/Cargo.toml`（任务指定的原样命令） | **0** | §5.3 |
| `cargo run --quiet --manifest-path _probe/Cargo.toml --bin probe` | **0** | §5.4 |
| `cargo run --quiet --manifest-path _probe/Cargo.toml --bin robustness` | **0** | §5.4 |
| `cargo nextest --version` | 见 §9 | 仅确认工具存在（0.9.145）；**未运行 nextest** |
| `Get-FileHash -Algorithm SHA256` 清单命令 | 0 | 冻结清单（§7） |

### 2.2 实际读取的文件（用 read 工具逐字读取，非转述）

- `Cargo.toml`、`_probe/Cargo.toml`、6 个 `crates/*/Cargo.toml`、`.gitignore`、`_probe/.gitignore`（`git ls-files`）
- `_probe/src/main.rs`（§245–339 全读）、`_probe/src/bin/robustness.rs`（§60–99、§96–165、§215–298）、`_probe/src/bin/currents.rs`（头部）
- `crates/circuit-core/src/connectivity.rs`（§1–125 全读）
- `README.md`（§1–45）、`docs/testing.md`（§1–45、§45–184、§185–319 **全文**）、`docs/backend-evaluation.md`（§1–45、§125–204）
- `docs/review-evidence/baseline.md`、`docs/review-evidence/team-board.md`（全文）
- `docs/review-evidence/raw-baseline-workspace-test.txt`（read 工具报 "binary file"，改用 PowerShell 解码核对，见 §4.4 / F5）

---

## 3. 工具链与 workspace 成员（任务项 2）

### 3.1 工具链

```text
rustc 1.98.1 (48a229cea 2026-09-01)
binary: rustc
commit-hash: 48a229ceaefd4985c50990b14116b6d856af0985
commit-date: 2026-09-01
host: x86_64-pc-windows-msvc
release: 1.98.1
LLVM version: 22.1.8

cargo 1.98.1 (797e8a9bc 2026-08-05)
host: x86_64-pc-windows-msvc
libgit2: 1.9.4 (sys:0.21.0 vendored)
libcurl: 8.21.0-DEV (sys:0.4.90+curl-8.21.0 vendored ssl:Schannel)
os: Windows 10.0.26200 (Windows 11 Professional) [64-bit]
```

两者都在 `PATH` 上（直接调用成功，退出码 0）。注意 `docs/testing.md:53-58` 写的是 Git-Bash 风格（`export PATH="$PATH:/c/Users/15185/.cargo/bin"`）——那在本轮 pwsh 会话里**不适用**；pwsh 下 cargo 已可直接调用。

### 3.2 `cargo metadata --no-deps` 摘要（6 个成员 crate）

`workspace_root` = `F:\codexprojects\dsl000`，`target_directory` = `F:\codexprojects\dsl000\target`。

| crate | version | edition | 关键依赖 | targets（cargo 自动发现结果） |
|---|---|---|---|---|
| `circuit-core` | 0.1.0 | 2024 | thiserror 2 | lib |
| `circuit-dsl` | 0.1.0 | 2024 | circuit-core, thiserror 2 | lib + **tests/elaborate.rs** |
| `circuit-backend` | 0.1.0 | 2024 | circuit-core, circuit-results, **cirq-ir 0.5.0, thevenin 0.5.0, thevenin-types 0.5.0**, thiserror 2 | lib + **tests/adapter.rs** |
| `circuit-results` | 0.1.0 | 2024 | circuit-core, serde 1, serde_json 1, thiserror 2 | lib |
| `circuit-session` | 0.1.0 | 2024 | circuit-core, circuit-dsl, circuit-backend, circuit-results | lib + **tests/session.rs** |
| `circuit-cli` | 0.1.0 | 2024 | 5 个内部 crate, clap 4, rustyline 18, serde_json 1, thiserror 2 | **bin cdsl** + tests/e2e.rs + tests/repl.rs |

`workspace_default_members` = 全部 6 个成员（无 `default-members` 收窄）。`_probe` 不在其中：根 `Cargo.toml:11-14` 明确 `exclude = ["_probe"]`。

许可证 / rust-version 元数据（`cargo metadata ... | ConvertFrom-Json`）：

```text
circuit-core     0.1.0  MIT OR Apache-2.0   rust_version=1.85  edition=2024
circuit-dsl      0.1.0  MIT OR Apache-2.0   rust_version=1.85  edition=2024
circuit-backend  0.1.0  MIT OR Apache-2.0   rust_version=1.85  edition=2024
circuit-results  0.1.0  MIT OR Apache-2.0   rust_version=1.85  edition=2024
circuit-session  0.1.0  BSD-3-Clause        rust_version=       edition=2024   ← 不一致（F6）
circuit-cli      0.1.0  MIT OR Apache-2.0   rust_version=1.85  edition=2024
```

---

## 4. `cargo test --workspace` 精确复核（任务项 3）

### 4.1 命令与退出码

第二次运行（单进程内捕获全部输出到变量，**未用管道截断后估算**）：

```powershell
$out = & cargo test --workspace 2>&1
$code = $LASTEXITCODE
Write-Output "TOTAL_STDOUT_LINES=$($out.Count)"
Write-Output "EXITCODE=$code"
# 再逐行匹配 '^test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored' 程序化求和
```

真实输出摘要：

```text
TOTAL_STDOUT_LINES=487
EXITCODE=0
TEST_BINARIES_WITH_RESULT_LINE=16
SUM_PASSED=389
SUM_FAILED=0
SUM_IGNORED=0
```

第一次运行（后台作业 `pwsh-3`）：作业状态 `completed`、**exit code 0**，输出末行为 `EXITCODE=0`。

### 4.2 逐 test binary 的 passed（16 条 `test result:` 行）

| # | test binary（cargo 输出行） | 对应 crate/target | passed | failed | ignored |
|---|---|---|---|---|---|
| 1 | `Running unittests src\lib.rs (target\debug\deps\circuit_backend-*.exe)` | circuit-backend lib | **15** | 0 | 0 |
| 2 | `Running tests\adapter.rs (target\debug\deps\adapter-*.exe)` | circuit-backend tests/adapter.rs | **21** | 0 | 0 |
| 3 | `Running unittests src\main.rs (target\debug\deps\cdsl-*.exe)` | circuit-cli bin `cdsl` | **9** | 0 | 0 |
| 4 | `Running tests\e2e.rs (target\debug\deps\e2e-*.exe)` | circuit-cli tests/e2e.rs | **18** | 0 | 0 |
| 5 | `Running tests\repl.rs (target\debug\deps\repl-*.exe)` | circuit-cli tests/repl.rs | **11** | 0 | 0 |
| 6 | `Running unittests src\lib.rs (circuit_core-*.exe)` | circuit-core lib | **58** | 0 | 0 |
| 7 | `Running unittests src\lib.rs (circuit_dsl-*.exe)` | circuit-dsl lib | **84** | 0 | 0 |
| 8 | `Running tests\elaborate.rs (elaborate-*.exe)` | circuit-dsl tests/elaborate.rs | **67** | 0 | 0 |
| 9 | `Running unittests src\lib.rs (circuit_results-*.exe)` | circuit-results lib | **71** | 0 | 0 |
| 10 | `Running unittests src\lib.rs (circuit_session-*.exe)` | circuit-session lib | **9** | 0 | 0 |
| 11 | `Running tests\session.rs (session-*.exe)` | circuit-session tests/session.rs | **25** | 0 | 0 |
| 12 | `Doc-tests circuit_backend` | doc-test | 0 | 0 | 0 |
| 13 | `Doc-tests circuit_core` | doc-test | 0 | 0 | 0 |
| 14 | `Doc-tests circuit_dsl` | doc-test | 0 | 0 | 0 |
| 15 | `Doc-tests circuit_results` | doc-test | **1** | 0 | 0 |
| 16 | `Doc-tests circuit_session` | doc-test | 0 | 0 | 0 |

原文（每条 `test result:` 行的完整形态，示意第 1 条与第 4 条）：

```text
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.57s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
test result: ok. 58 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 84 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
test result: ok. 67 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 71 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 25 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s   （circuit_backend doc-tests）
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s   （circuit_core doc-tests）
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s   （circuit_dsl doc-tests）
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s   （circuit_results doc-test）
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s   （circuit_session doc-tests）
```

### 4.3 求和（精确）

- **passed 合计 = 15+21+9+18+11+58+84+67+71+9+25+0+0+0+1+0 = 389**
- **failed = 0，ignored = 0，measured = 0，filtered out = 0**
- **exit code = 0**（两次运行一致）
- 12 条 result 行 passed > 0（11 个真实二进制 + 1 个 doc-test）；4 个 doc-test target 为 0
- 按 crate 分布：core 58；dsl 84+67=151；backend 15+21=36；results 71+1=72；session 9+25=34；cli 9+18+11=38。合计 389。

**结论：lead 的 389 数字独立复核成立，精确且唯一。**

我脚本里还有一类 `SUSPECT` 行（`test a_failed_definition_leaves_the_previous_one_running ... ok` 等 2 行）——那是我的正则 `FAILED` 在 PowerShell 里**大小写不敏感**匹配到了测试名中的 "failed"，不是失败。全部 389 条都是 `ok`。

### 4.4 交叉核对

| 来源 | passed | failed | ignored | exit code |
|---|---|---|---|---|
| 我的第 1 次运行（后台作业 pwsh-3） | 389 | 0 | 0 | 0 |
| 我的第 2 次运行（单进程全量捕获） | 389 | 0 | 0 | 0 |
| lead 的原始捕获 `docs/review-evidence/raw-baseline-workspace-test.txt` | 389 | 0 | 0 | **文件内无退出码行**（见 F5） |

对 lead 的原始捕获，我用 PowerShell 解码并同样程序化求和：

```text
BYTES=55670  FIRST8=FF FE 63 00 61 00 72 00   （UTF-16LE + BOM）
RAW_RESULT_LINES=16
RAW_SUM_PASSED=389  RAW_FAILED=0  RAW_IGNORED=0
```

三方数字一致。

### 4.5 与文档的核对

- `README.md:5` 声称"实测 `cargo test --workspace` 共 **389 个测试全部通过**（0 失败）" ⇒ **与我的复核一致**。
- `docs/testing.md:105-127` 的逐 binary 表（15/21/9/18/11/58/84/67/71/9/25/1，合计 389）⇒ **与我的复核逐行一致**。
- `docs/testing.md:17-25` 的 §1 却写"elaborate.rs（**51** 个）""adapter.rs（**16** 个）""e2e.rs（**14** 个）" ⇒ **与同文件 §3 及实测矛盾**（见 F1）。
- `docs/testing.md:147` "实测约 16 秒" ⇒ **未验证**（我未计时，§9）。

---

## 5. `_probe` 独立工程（任务项 4 + 7）

### 5.1 `_probe/Cargo.toml`（原文，共 11 行）

```toml
[package]
name = "probe"
version = "0.1.0"
edition = "2024"

[dependencies]
cirq-ir = "0.5.0"
num-complex = "0.4.6"
thevenin = "0.5.0"
thevenin-cirq = "0.5.0"
thevenin-types = "0.5.0"
```

版本约束（任务项 7）：全部为 `^` 语义的 `"0.5.0"` / `"0.4.6"`（`cargo metadata` 报 `req = "^0.5.0"` / `"^0.4.6"`）。`edition = "2024"`，**无** `rust-version` 字段（`rust_version = null`）。

### 5.2 `_probe/src/**` 的全部 binary（`cargo metadata` 的 targets，非人工推断）

```text
{"kind":["bin"],"name":"currents",  "src_path":"...\_probe\src\bin\currents.rs",  "edition":"2024"}
{"kind":["bin"],"name":"probe",     "src_path":"...\_probe\src\main.rs",        "edition":"2024"}
{"kind":["bin"],"name":"robustness","src_path":"...\_probe\src\bin\robustness.rs","edition":"2024"}
```

`default_run = null`，且包名 `probe` ⇒ 裸 `cargo run` 会报"无法确定要运行哪个 binary"；必须显式 `--bin`。

`_probe/target` 存在但**不影响**：`.gitignore:3-4` 同时忽略 `/target` 与 `_probe/target`；`git ls-files _probe` 返回 6 个跟踪文件（`_probe/.gitignore`、`_probe/Cargo.lock`、`_probe/Cargo.toml`、`_probe/src/main.rs`、`_probe/src/bin/currents.rs`、`_probe/src/bin/robustness.rs`）。**`_probe/Cargo.lock` 是被 git 跟踪的**，所以构建若不新鲜就会改动一个跟踪文件——我用 `--locked` 先证明了它不需要更新（下节）。

### 5.3 构建（退出码 + 是否改动跟踪文件）

```text
=== BUILD --locked ===
   Finished  `dev` profile [unoptimized + debuginfo] target(s) in 3.34s
LOCKED_EXIT=0
=== BUILD（任务指定的原样命令）===
   Finished  `dev` profile [unoptimized + debuginfo] target(s) in 0.15s
BUILD_EXIT=0
```

`--locked` 成功 = `_probe/Cargo.lock` 与 `_probe/Cargo.toml` 已经一致，无需改写。构建前后哈希（同一命令内 `Get-FileHash` 各取一次）：

```text
构建前/后均相同：
9D4AC65AFBA02DD658334835B3656B2257B015AE395C26A2C5F94ED2A3E9AA82  F:\codexprojects\dsl000\Cargo.lock
B00E4B6CA83AD439096A36E1846F3DDB4BE77921832087F26E4DE16A4EB0E109  F:\codexprojects\dsl000\_probe\Cargo.lock
96852193E4623F2B6BBC1FB88372D2CE171358650B93C9B53716853FFFFD230C  F:\codexprojects\dsl000\_probe\Cargo.toml
```

构建后 `git status --porcelain` 与构建前逐字相同（没有新增未跟踪文件，见 §8 F8）。

### 5.4 运行退出码（只运行，未修改）

| 命令 | 退出码 | 输出规模 | 末尾结论行 |
|---|---|---|---|
| `cargo run --quiet --manifest-path _probe/Cargo.toml --bin probe` | **0** | 53 行 | `RESULT: ALL ACCEPTANCE CASES PASSED` |
| `cargo run --quiet --manifest-path _probe/Cargo.toml --bin robustness` | **0** | 34 行 | （无总结行，只有 5 个小节的打印） |

`probe` 的真实输出要点（原文摘录）：

```text
[Case 1] Resistive divider operating point
  [PASS] v(mid)  actual=6.66666667e-1  expected=6.66666667e-1  |diff|=0.000e0  tol=1.0e-9
[Case 2] RC transient (step response)
  1015 time points, t in [0.000e0, 5.000e-4] s
  [PASS] t=  25.232us (0.25tau)  v(out)=0.221061  analytic=0.223007  |diff|=1.95e-3
  [PASS] t=  50.232us (0.50tau)  v(out)=0.393362  analytic=0.394877  |diff|=1.51e-3
  [PASS] t= 100.232us (1.00tau)  v(out)=0.632056  analytic=0.632974  |diff|=9.18e-4
  [PASS] t= 200.232us (2.00tau)  v(out)=0.864641  analytic=0.864979  |diff|=3.38e-4
  [PASS] t= 300.232us (3.00tau)  v(out)=0.950204  analytic=0.950328  |diff|=1.24e-4
  worst |diff| = 1.946e-3
[Case 3] RC AC response H(jw) = 1/(1+jwRC)
  [PASS] f=1.5849e2 Hz |H|=0.995078 ph=-5.687deg ... |diff|=0.00e0
  [PASS] f=1.5849e3 Hz |H|=0.708587 ph=-44.880deg ... |diff|=5.55e-17
[Case 4] RLC AC ... worst |diff| = 2.305e-15 ; resonance f0 = 5032.921 Hz
[Case 5] Diode nonlinear operating point ... v(out) = 0.692872 V ; |diff|=3.276e-6 tol=5.0e-3
[Case 6] DC sweep ... 6 points, 全部 [PASS]

RESULT: ALL ACCEPTANCE CASES PASSED
```

`robustness` 的真实输出要点（原文摘录）：

```text
[a] Failure reporting: singular / ill-posed circuit
  OK: returned Err(simulation failed: failed to solve MNA system: matrix is singular, cannot solve)
  floating node v(b) = 1 (Ok returned, no ground path)
  => dangling-node detection must be done by OUR frontend
[b] Thread isolation: 8 concurrent simulations with distinct inputs   → 8/8 [PASS]
[c] Malformed circuit: terminal referencing a nonexistent net id
  OK: Err(... element `r1`: terminal `neg` references unknown net id)
[d] Save subsetting via `circuit.save`  → 3 vectors ⇒ save is NOT honoured by simulate_op
[e] Isolated island (AC-coupled block) operating point  → v(out) = 1 ; v(in) = 1
    AC on the same island: 3 频点 |diff| ≤ 2.22e-16
```

### 5.5 版本一致性核对（任务项 7）

`_probe/Cargo.toml` 声明的 5 个外部依赖 vs 锁文件实际解析值：

| crate | `_probe/Cargo.toml` 声明 | `_probe/Cargo.lock` | 根 `Cargo.lock` | 一致？ |
|---|---|---|---|---|
| `thevenin` | 0.5.0 | 0.5.0 | 0.5.0 | ✅ |
| `thevenin-types` | 0.5.0 | 0.5.0 | 0.5.0 | ✅ |
| `thevenin-cirq` | 0.5.0 | 0.5.0 | **不存在**（`circuit-backend` 不依赖它，故根锁文件无此条） | ✅（预期） |
| `cirq-ir` | 0.5.0 | 0.5.0 | 0.5.0 | ✅ |
| `num-complex` | 0.4.6 | 0.4.6 | 0.4.6 | ✅ |

根 `Cargo.lock` 的实测条目（`Select-String`）：

```text
name = "cirq-ir"        version = "0.5.0"
name = "num-complex"    version = "0.4.6"
name = "thevenin"       version = "0.5.0"
name = "thevenin-types" version = "0.5.0"
```

根 `Cargo.toml:41-43` 的 workspace 依赖同样钉 0.5.0 / 0.5.0 / 0.5.0。**`docs/backend-evaluation.md:36-40` 的"家族内所有 crate 均为 0.5.0"在本仓库依赖图上成立（`thevenin-xspice` 未使用、未出现在锁文件里，doc 也如此标注）。**

---

## 6. 新增集成测试会不会被自动发现（任务项 5）

### 6.1 结论

**PASS：会被自动发现。** 前提是把文件放在 `crates/<crate>/tests/` **直接子级**（见 6.4 的限制）。

### 6.2 证据 1：没有任何禁用自动发现的 manifest 配置

用 grep 扫描仓库内全部 `Cargo.toml`（`crates` 下 6 个 + 根 + `_probe`）：

```text
Cargo.toml:11: # `_probe` is the standalone Phase-0 backend evaluation harness. It is kept in
crates\circuit-cli\Cargo.toml:9: [[bin]]
```

即：
- **没有** `autotests = false`（也没有 `autobins` / `autoexamples` / `autobenches`）
- **没有** 任何 `[[test]]` / `[lib]` 显式声明
- **没有** `harness = ...` 设置
- 唯一的 target 显式声明是 `crates/circuit-cli/Cargo.toml:9-11` 的 `[[bin]] name = "cdsl"`

### 6.3 证据 2：既有同类文件确实是被"自动发现"的

`cargo metadata --no-deps`（无条件声明却出现 test target）：

```text
circuit-dsl     target: test elaborate   crates\circuit-dsl\tests\elaborate.rs
circuit-backend target: test adapter     crates\circuit-backend\tests\adapter.rs
circuit-session target: test session     crates\circuit-session\tests\session.rs
circuit-cli     target: test e2e         crates\circuit-cli\tests\e2e.rs
circuit-cli     target: test repl        crates\circuit-cli\tests\repl.rs
```

`cargo test -p circuit-backend --no-run` 的真实输出（EXIT=0）：

```text
    Finished `test` profile [unoptimized + debuginfo] target(s) in 5.61s
  Executable unittests src\lib.rs (target\debug\deps\circuit_backend-aa9d963fa7b8fe4e.exe)
  Executable tests\adapter.rs (target\debug\deps\adapter-a60cefed43fb5f59.exe)
```

⇒ **`cargo test -p circuit-backend` 如今发现的测试二进制名 = `unittests src\lib.rs`（lib 单元测试）+ `tests\adapter.rs`**（共 2 个；对应 §4.2 的 15 + 21 = 36 个测试）。

`cargo test -p circuit-dsl --no-run` 的真实输出（EXIT=0）：

```text
    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.22s
  Executable unittests src\lib.rs (target\debug\deps\circuit_dsl-c1fde4e0fc870376.exe)
  Executable tests\elaborate.rs (target\debug\deps\elaborate-53b2f2dd4b0e9133.exe)
```

### 6.4 对 Wave 2 的直接影响与限制

- `team-board.md:41-43` 计划新建 3 个文件：`crates/circuit-dsl/tests/reference_path_regression.rs`（A07）、`crates/circuit-backend/tests/transient_reference_regression.rs`（A08）、`crates/circuit-backend/tests/phase_regression.rs`（A09）。三者都在上表同一层级 ⇒ **不需要改任何 `Cargo.toml`，会被自动发现**。
- **我未做"新建文件后跑 cargo test"的实验**——本轮是严格只读，不允许新建测试文件。机制结论由"无禁用配置 + 既有同类文件已被自动发现"两条证据支撑。
- **限制（未在本仓库实测，属 Cargo 目录扫描语义）**：放在 `tests/` 的子目录里（如 `tests/sub/foo.rs`）**不会**被当作测试目标；只有 `tests/` 直接子级的 `.rs`，或 `tests/<dir>/main.rs`。新增文件请勿下放子目录。

---

## 7. SHA-256 冻结清单（任务项 6）

### 7.1 主清单（任务指定范围）—— GROUP-A，41 个文件

所用命令（**原样可复现**，注意本仓库是 Windows PowerShell）：

```powershell
$A = @()
$A += Get-ChildItem -Path crates -Recurse -File -Include *.rs |
      Where-Object { $_.DirectoryName -match '\\src(\\|$)' }
$A += Get-ChildItem -Path _probe\src -Recurse -File -Include *.rs
$A += Get-Item docs/backend-evaluation.md, docs/testing.md, README.md
$A | Sort-Object FullName -Unique |
  ForEach-Object { (Get-FileHash -Algorithm SHA256 $_.FullName).Hash + "  " + $_.FullName }
```

**快照时刻 `SNAPSHOT_AT` = 2026-09-18 19:40:56 +08:00**（`+08:00`）。`crates/**/src/**/*.rs` 共 35 个（core 10、backend 4、dsl 8、results 5、session 4、cli 4），`_probe/src/**` 3 个，文档 3 个。

| 路径（相对仓库根，`/` 分隔） | 字节 | SHA-256 |
|---|---|---|
| `_probe/src/bin/currents.rs` | 4258 | `4DF1FDBA86612C14727B5E71B9B1DA4BC763B25C0ABE2FB297B42609AF7848DF` |
| `_probe/src/bin/robustness.rs` | 10624 | `141198FC9C79F66D4618A54572ED821765ACEEAAE897CEB7E6C25090315B8B07` |
| `_probe/src/main.rs` | 20964 | `81EAD7F09867E3E5ED1EC53E49CD14901C358D978E9241A41D51B65C73BBC0A7` |
| `crates/circuit-backend/src/backend.rs` | 6285 | `6C5CCA5A962E95ABB22C3A52B33C3EB948BEB696AECBFF985F46A454BE13F2C6` |
| `crates/circuit-backend/src/lib.rs` | 1256 | `6BBF8CB8ABCA50CFF0D864DFD822B2BCE91937AD1BC216F6104FAACFD57B6A12` |
| `crates/circuit-backend/src/sweep.rs` | 16231 | `38A3D6FAB39D73EE35F2520264800762570E5E829A1FB48A097C80EA08637925` |
| `crates/circuit-backend/src/thevenin.rs` | 45720 | `6337B9EB2A9917F539DBE1B427070F8BA0AF35F66BA71139330D39A21D5B5C42` |
| `crates/circuit-cli/src/check.rs` | 6775 | `081A31C4FB2BA49994A360E7B959790BCBCCA425F21780936567B3F9DCD1B6AC` |
| `crates/circuit-cli/src/main.rs` | 5362 | `6C1B1680156E9C911AD3844396CBB21A80EE8AA76C343F27027B61FE0BD74CDA` |
| `crates/circuit-cli/src/repl.rs` | 14740 | `1243D2306042215F5961D94273E16FF7A62AA93F0D563136F609258BBE8D2072` |
| `crates/circuit-cli/src/run.rs` | 5648 | `440245E71E7344EA3AC48854D75B15E13DA699BA1427F43048192019BE3DBB4B` |
| `crates/circuit-core/src/connectivity.rs` | 9722 | `D66E2118BFF03EA2C839331AD5586D240D1861F44BF4B59B4141D3FF95181A39` |
| `crates/circuit-core/src/diagnostic.rs` | 12418 | `750B5920AD45EA59222C196876BF2E2E0D8B5CFBED727AACBB76E84371F13B1E` |
| `crates/circuit-core/src/format.rs` | 11692 | `F8936EC6366A588D703D08EA0ABE2E050D1D1160935F9B6FEC0C17F27A2696D5` |
| `crates/circuit-core/src/id.rs` | 1994 | `5C43A5A79A4122213BBE4DBD80BAC32DE4FD2F0AE7A5F1390E662106F502120D` |
| `crates/circuit-core/src/ir.rs` | 19375 | `267C3A2D88E286D14478E195865D3D8DFDA4B325E4FF26CCAA957C4EBC09CFB1` |
| `crates/circuit-core/src/lib.rs` | 1561 | `86F68B6E13505842DD665E3557F2F1D3D6A247AA6B7576BF53B8FE1EF2B7F84F` |
| `crates/circuit-core/src/limits.rs` | 2496 | `DB02EC4D1B2F9DC02DC9519758694AF4FDEB2A8034D51CF2CE240B3C6DF5FCED` |
| `crates/circuit-core/src/plan.rs` | 10435 | `E562162A41A6B0F42954C225BABB793C5BFB821137A3783BE40E6044F39609D8` |
| `crates/circuit-core/src/span.rs` | 10217 | `F75B3CD1A46BC64579F80F752D99E83E22E9AFA9F2E6449EEF7177058954389D` |
| `crates/circuit-core/src/units.rs` | 21518 | `78CA1E8626784115AAE311F7861A07F595096F6AA10653D678F838245E2C11F9` |
| `crates/circuit-dsl/src/ast.rs` | 16713 | `10AA2172674D723E4A20A80FB8BC1340E2FEC74CA0249B01577D935C2EC64FFB` |
| `crates/circuit-dsl/src/complete.rs` | 13561 | `920F83FFE0CC0F459E1F9171E8F42639EFEDF5C4E57CBA2BFAC581A0710776F9` |
| `crates/circuit-dsl/src/elaborate.rs` | 106454 | `6E76B5F9006767537526D740D1634165AB337B00CEA7A6E0BE5FB800DFE8A513` |
| `crates/circuit-dsl/src/eval.rs` | 16579 | `958E19B0A9643C74F68BF269C1C93A83528021ABFDA7E88D4B67BC678C863E20` |
| `crates/circuit-dsl/src/lexer.rs` | 40010 | `F577B693ADF522F5AF988F9E0B02813478E9298F7329BD8A0073E95DB4EDF81A` |
| `crates/circuit-dsl/src/lib.rs` | 763 | `C44386599526A16C0FA08A12B490A03A153E6D8DB8291E08DB953752612733B6` |
| `crates/circuit-dsl/src/parser.rs` | 112739 | `E62AE862D9C8A5E79100FCE6748CEFF6D041076BCEE7441F0FEF5671C98953D2` |
| `crates/circuit-dsl/src/token.rs` | 8217 | `584A35F898AE1258D669FDE99E0CB6F2ACCAA4A812C758888470C3F64A84634D` |
| `crates/circuit-results/src/dataset.rs` | 30312 | `CEB84D6730EC946B37E9AFCCD1048FF87F81E1283D5F55771C6BA8827E9C634D` |
| `crates/circuit-results/src/export.rs` | 27319 | `DC2BDF4CC019B5166D4E98B66002076A63BEE203C24C471E62285AE7A58585E7` |
| `crates/circuit-results/src/expr.rs` | 33754 | `5351AA015E9EB2F40F9C06F54F1A22F24D227B04E1023CAC426FD45271179D0E` |
| `crates/circuit-results/src/lib.rs` | 5754 | `F6F6C7C77475D907B931192459C57B7E7B3ECB6D1618226F547180285F15AD39` |
| `crates/circuit-results/src/measure.rs` | 21470 | `E20BDB321180F9D217E8BF0659CD78A0ED7E8EF9DD0D658EA668DDE2220BDB4E` |
| `crates/circuit-session/src/execute.rs` | 19510 | `1E906B9E5A668799FC23965DF3696A9732138D0DD6F917BD4FBB2C9A60E0BD9C` |
| `crates/circuit-session/src/format.rs` | 2374 | `0F1AB5931BEF1220692D56012D9FE0A92E6A43DC534597F639C19E5722995DA6` |
| `crates/circuit-session/src/lib.rs` | 820 | `03294B5DB6B049C4552C4B5F2F430AE7CCD6270036E3FA76D0AB31E1526A7E29` |
| `crates/circuit-session/src/session.rs` | 27586 | `9D1494F76505E0D8AF50E89C2726FAFBCA8DB051E7E461230FA49F4ABB2C8576` |
| `docs/backend-evaluation.md` | 12083 | `0D275B340A6FF6A49156551639FD225C7270FE6E1179393476FA22978E977444` |
| `docs/testing.md` | 24074 | `60C9EE4C14B9B48711F56C6943089EF9F7624DEF9063EE36D79A772FE7E16431` |
| `README.md` | 16008 | `D3669EE5285F7313D32F0F20D033BE7BFA41216396A547DCC50B54DC39E34DCD` |

### 7.2 补充清单（主清单未覆盖、但冻结同样需要）—— GROUP-B，22 个文件

任务指定的正则 `crates/**/src/**/*.rs` **不覆盖** `crates/*/tests/**`、`examples/**`、以及所有 `Cargo.toml`/`Cargo.lock`。本轮要新建的 3 个回归测试都在 `tests/` 下，因此我额外给出补充清单（这是**建议的冻结范围**，不是任务原文要求）：

```powershell
$B = @()
$B += Get-ChildItem -Path crates -Recurse -File -Include *.rs |
      Where-Object { $_.DirectoryName -match '\\tests(\\|$)' }
$B += Get-ChildItem -Path examples -Recurse -File
$B += Get-Item Cargo.toml, Cargo.lock, _probe/Cargo.toml, _probe/Cargo.lock
$B += Get-ChildItem -Path crates -Recurse -File -Include Cargo.toml
$B | Sort-Object FullName -Unique |
  ForEach-Object { (Get-FileHash -Algorithm SHA256 $_.FullName).Hash + "  " + $_.FullName }
```

| 路径 | 字节 | SHA-256 |
|---|---|---|
| `_probe/Cargo.lock` | 36160 | `B00E4B6CA83AD439096A36E1846F3DDB4BE77921832087F26E4DE16A4EB0E109` |
| `_probe/Cargo.toml` | 184 | `96852193E4623F2B6BBC1FB88372D2CE171358650B93C9B53716853FFFFD230C` |
| `Cargo.lock` | 42488 | `9D4AC65AFBA02DD658334835B3656B2257B015AE395C26A2C5F94ED2A3E9AA82` |
| `Cargo.toml` | 1345 | `59EB09B7E5647416728F0D9B86C3E84C01A2001E170E1B3B72171F49A6DC5B71` |
| `crates/circuit-backend/Cargo.toml` | 474 | `123A33CE2E5E242A8520FE447F7688C290D8214FEBAA404A7401F7241AF37C05` |
| `crates/circuit-backend/tests/adapter.rs` | 43811 | `6AE48B621274BA826F97B4EAED193449D535692DD1D24836A3518BB7C1E494D0` |
| `crates/circuit-cli/Cargo.toml` | 517 | `9ED32A0AAD23A1B6FAD65086257E7A2153D25A376D7E6C9A57C41BBA523B638B` |
| `crates/circuit-cli/tests/e2e.rs` | 20808 | `1A491B4499C8A9B484E8886CE30CB06CF885E35221011605471AD21EB8117ECB` |
| `crates/circuit-cli/tests/repl.rs` | 8576 | `D7B3C4081471AEBBC0E4D39C7B6E7C03C569DD1F3AD1E151E66CBA1AFCBB11BA` |
| `crates/circuit-core/Cargo.toml` | 180 | `D77CA9DB2C55483DAD27A9AF385AFEA90FF1DB077CA19FC6F0F156A6B88B35B6` |
| `crates/circuit-dsl/Cargo.toml` | 294 | `FC9CE089A834172B3DF52CF4A4D8BD4901BBC7CEC7DB29DE93CE26E5671915F4` |
| `crates/circuit-dsl/tests/elaborate.rs` | 46816 | `5D5F94DCA7860064CAAD7488E1EFC6AA8416D575D1AA3C95E739814445FE213E` |
| `crates/circuit-results/Cargo.toml` | 334 | `CE2A99A086365A699216822F426DCB3CB060BF756EED0A30FFE7C90ACE660254` |
| `crates/circuit-session/Cargo.toml` | 384 | `456DD738AAC1CB2AF8ED3DDCEB04CF7C0E7BB62841BC44D3A92C8C15E2517B6B` |
| `crates/circuit-session/tests/session.rs` | 18075 | `E847E46EE0039439931325E21D3F0EF14465842D2638C891F85A917AB1617970` |
| `examples/diode_rectifier.cdsl` | 1619 | `6C0315A02E38D2C82D6E2CC26E4AF3A3BF8695EF4D90071D2518728ECD7CC1DE` |
| `examples/ladder.cdsl` | 2192 | `B59F58A6A51B6B179C657CA80D1AFD196447D37BEF98583606B93A7D83228639` |
| `examples/parameter_sweep.cdsl` | 912 | `2FBD5812C3BE6DD02D043BEEE8F9EF464926A27DACAB36FB866DC6B61D076E54` |
| `examples/rc_filter.cdsl` | 1180 | `E30217A738986259824DAADE5C40BD7A6A4800C5A49646932107A2DB3DE3157D` |
| `examples/rlc.cdsl` | 1443 | `BF31D782282A155935BC2BC112C6CBB025BBCA20C3937854F1F45A4B65354F07` |
| `examples/two_stage.cdsl` | 2552 | `7719AEDCB60EED4F40DC23DE5BD488ADBEA0CDEF52335F59999ECBC3B2544102` |
| `examples/voltage_divider.cdsl` | 841 | `005B65C54086630EC2788C00515ECFDF0517B9922F614072C0D2F13519F78822` |

### 7.3 参考清单：`docs/*.md`（含 lead 的证据文件，仅参考；非任务指定）

| 文件名 | 字节 | SHA-256 |
|---|---|---|
| `architecture.md` | 34120 | `8344A0C50DAA5A44EA9248965715ABE9529DADF45DA9270C64031D7B5602A238` |
| `backend-evaluation.md` | 12083 | `0D275B340A6FF6A49156551639FD225C7270FE6E1179393476FA22978E977444` |
| `environment.md` | 4193 | `A34586B71510589826BC2D618E78B60A065B6723CA95E8243BCDB0CE969D331B` |
| `language.md` | 18332 | `A0E0B32E1F7F9055229F583CDFE7BCCC0677005521C7B505C5828C53223FF925` |
| `prompt-review.md` | 5675 | `D65B50C2203D48FEFEEF45FF4CA80D08BFB3A1EC49999C2085AB1D3C5D5646CC` |
| `repl.md` | 20132 | `A963AF714CB7149AF68E408A5F8AD2EC040627FD69B5F9E926160D10AFAEBEC1` |
| `testing.md` | 24074 | `60C9EE4C14B9B48711F56C6943089EF9F7624DEF9063EE36D79A772FE7E16431` |

`docs/review-evidence/` 下三个 lead 文件在同一时段的哈希（用于交叉核对，非常量）：`baseline.md` = `93957141A71E5FB39A737C34DA054BF22121449CF0698527DF7E35CC1E71FD25`（3226 B）、`raw-baseline-workspace-test.txt` = `3926A80C6C67D6CC97FE741AFF328B52E379C807E03578A8C644FF09CFBC37F7`（55670 B）、`team-board.md` = `F5423F1D011B0E61D0AA148AB8BAB24B354BC728EE389E0DE825E05D75B06BDA`（5459 B）。

### 7.4 漂移警告（**必须遵守**）

**`README.md` 正在被 lead 改写，我观察到了实际漂移**：

| 时刻 | `README.md` SHA-256 | 字节 |
|---|---|---|
| 19:37（我的首次清单） | `B6C5197B5334C47FD9CF0A9259131C7BDF77A669210A7C2F919D003E9396FDC5` | 15683 |
| **2026-09-18 19:40:56 +08:00（本清单采用值）** | **`D3669EE5285F7313D32F0F20D033BE7BFA41216396A547DCC50B54DC39E34DCD`** | **16008** |

同期 `git status --porcelain` 出现 ` M README.md`（开工时没有），`git diff --stat README.md` = `1 file changed, 7 insertions(+), 4 deletions(-)`；`README.md` 的 `LastWriteTime` = `2026-09-18 19:38:56`。`docs/testing.md`（17:02:02）与 `docs/backend-evaluation.md`（14:53:28）在两次取样间**未变**，其哈希可视为稳定。

⇒ **Wave 2 做"A/B 两轮实跑必须对应同一候选版本"时，必须在冻结那一刻重跑 §7.1 + §7.2 的命令**，并把新哈希写入 `docs/review-evidence/final-gate.md`。本清单是 **2026-09-18 19:40:56 +08:00 时刻**的快照，不是最终冻结值。此外，A07/A08/A09 新建的 3 个测试文件必须进入冻结清单（它们此刻尚不存在）。

---

## 8. 发现清单（严重性 + 位置 + 证据）

严重性定义：**高** = 会让本轮交付的结论错误或无法复现；**中** = 文档/证据与实际不符，会误导后续轮次；**低** = 卫生/一致性/可读性问题；**信息** = 需知但非缺陷。

### F1（中）`docs/testing.md` 文件内部数字自相矛盾：§1 的三个数字是旧值

- 位置：`docs/testing.md:17`（"elaborate.rs（**51** 个）"）、`:19`（"adapter.rs（**16** 个）"）、`:23`（"e2e.rs（**14** 个）"）
- 与同文件冲突：`docs/testing.md:111`（adapter 21）、`:113`（e2e 18）、`:117`（elaborate 67）；也与 §4.2 的实测一致（21/18/67）
- 证据命令：`cargo test --workspace`（exit 0）→ `test result: ok. 21 passed ...`（adapter）、`18 passed`（e2e）、`67 passed`（elaborate）
- 影响：§1 是"测试策略"的入口叙述，读者会被 51/16/14 误导；同文件 §3 却写的是正确值
- 归属建议：A10（`docs/testing.md` 写入者，`team-board.md:45`）

### F2（低）`docs/testing.md` §7「未验证的部分」里塞进了一条"已实现"的陈述

- 位置：`docs/testing.md:261` 标题 `## 7. 未验证的部分`，其下 `:263` 明说"明确列出，**不声称**"，但 `:271` 的条目开头是"**悬空节点检查已实现**：…"
- 该条目的**内容**经我独立核实为**真**：
  - `crates/circuit-core/src/connectivity.rs:64` `pub fn floating_nodes(circuit: &Circuit) -> Vec<FloatingNode>`，判据表在 `:11-24`，`conducts_dc` 实现在 `:32-40`
  - 调用点：`crates/circuit-dsl/src/elaborate.rs:484` `for f in circuit_core::floating_nodes(&circuit) {`
  - 单元测试：`connectivity.rs` 内有 **9 个** `#[test]`（行 174/189/202/216/235/248/261/276/292）
  - 端到端用例：`crates/circuit-cli/tests/e2e.rs:130` `check_rejects_a_node_with_no_dc_path_to_ground`、`:165` `check_accepts_an_ac_coupled_stage_with_a_bias_resistor` —— 两个都在
- 问题只在**归属与措辞**：这条已实现/已验证的事实被放进"未验证"章节。同时 `docs/architecture.md:325` 把它说成"单元测试（**8** 个）"，实际是 9 个 ⇒ 跨文件数字不一致
- 归属建议：A10（`docs/testing.md`）；`docs/architecture.md` 不在 `team-board.md` 的所有权表里，需 lead 指派

### F3（高）RC 瞬态残差的文字归因与探针代码矛盾（本轮两大 P1 之一）

- 文档断言：`docs/backend-evaluation.md:175-176` "用例 2 的残差来自脉冲有限上升沿（1 ps）与**输出采样对齐**，在默认 `RELTOL=1e-3` 下属于预期量级，**不是**后端错误"；`docs/testing.md:163-164` 与 `:201-206` 重复同一归因
- 探针代码（`_probe/src/main.rs`，我逐行读了 245-339）：
  - `:263-264` `tr: Some(1e-12), tf: Some(1e-12)` ⇒ 上升沿确实是 1 ps
  - `:274,278` `step: tau / 200.0`、`tmax: Some(tau / 200.0)`，τ = R·C = 1 kΩ × 100 nF = 1e-4 s ⇒ 步长 500 ns
  - `:311-321` 用 `t.iter()` 选**最接近**目标时刻的**引擎自己的采样点** `t[idx]`
  - **`:322` `let expected = 1.0 - (-t[idx] / tau).exp();`** ⇒ 解析解取的是**同一个 `t[idx]`**
- 实测（我的 `probe` 运行，exit 0）：`t=25.232us (0.25tau) v(out)=0.221061 analytic=0.223007 |diff|=1.95e-3`，`worst |diff| = 1.946e-3`
- 我的算术（**明确标注为我的计算，不是新测量**）：
  - 因为 `expected` 与 `vout[idx]` 取自同一 `t[idx]`，**"输出采样对齐"对这条比较的贡献恒为 0**——比较在时间上是对齐的
  - 1 ps 上升沿相对 τ 的量级是 `1e-12 / 1e-4 = 1e-8` 相对量（≈1e-8 V），与实测的 1.9e-3 V 差 **5 个数量级**，无法解释
  - 反推等效时间滞后：`dv/dt = e^{-t/τ}/τ = 7770 V/s`（在 t=0.2523τ 处）；`1.946e-3 / 7770 = 2.50e-7 s = 250 ns ≈ 0.5 × 500 ns 步长`
  ⇒ 观察到的现象更像"数值解相对解析解整体滞后约半个输出步长"，而不是"采样对齐"；**具体成因（积分方法/时间步进/输出时刻语义）我没有做实验去判定，属 §9 未验证**，应由 A03（`docs/review-evidence/rc-reference-math.md`）与 A06（`_probe/src/main.rs`）用受控实验定论
- 归属建议：`_probe/src/main.rs` → A06（`team-board.md:40`）；`docs/backend-evaluation.md`/`docs/testing.md` → A10

### F4（高）`_probe` 里两处"浮空节点"用例的电路**都不是**浮空节点（本轮两大 P1 之二）

判据来源（项目自己的规则，不是我发明的）：`crates/circuit-core/src/connectivity.rs:11-24` —— "每个非地节点必须能经**直流导通**器件到达地"；`conducts_dc`（`:32-40`）：电阻/电感/**电压源**/二极管 = true，电容/电流源 = false。

**（a）`_probe/src/bin/robustness.rs:132-145`**
- 代码：`:132` 注释 "A floating node with no DC path to ground"；`:133` `base("float", [gnd(0), a(1), b(2)])`；`:134` `vsource(0,"v1", a(1) → gnd(0), dc=1.0)`；`:135` `resistor(1,"r1", a(1) → b(2), 1000.0)`
- 拓扑：`b — r1 — a — v1 — gnd`。电阻与独立电压源**都** `conducts_dc = true` ⇒ 洪泛（`connectivity.rs:75-88`）会从地到达 b ⇒ **b 不是浮空节点**
- 物理：b 开路（无负载），r1 上电流为 0 ⇒ `v(b) = v(a) = 1 V` 是**正确解析解**，不是"gmin/漏电把节点拉住"
- 我的实测（`cargo run --manifest-path _probe/Cargo.toml --bin robustness`，exit 0）：`floating node v(b) = 1 (Ok returned, no ground path)` ⇒ 数值与解析解一致，但**打印的解释文字是错的**
- 与既有复核一致：`docs/prompt-review.md:30-34`（P1：浮空用例的解释错误）、`docs/review-evidence/baseline.md:67`

**（e）`_probe/src/bin/robustness.rs:224-253`（同样的问题，容易被漏掉）**
- 代码：`:227-229` 注释 "no DC path from out to gnd"、"the classic 'floating through a capacitor' case"；`:231` `v1: in(1) → gnd(0)`；`:232` `r1: in(1) → out(2), 1k`；`:233-241` `c1: out(2) → gnd(0), 1 µF`
- 拓扑：`out — r1 — in — v1 — gnd` ⇒ out **有**直流参考通路（电阻 + 电压源）⇒ 也**不是**浮空节点；OP 时电容开路、回路电流为 0 ⇒ `v(out) = v(in) = 1 V` 同样是**正确结果**
- 我的实测输出：`[e] ... v(out) = 1 ; v(in) = 1`
- ⇒ 若要把"电容通路不等于直流参考通路"作为正例，这两个电路都不能用；真正的正例应当是"到地**只能**经过电容/电流源"的节点（例如去掉 r1、只留 c1 的节点），而那需要改 `_probe` 源码（A05/A06 的写集）
- 归属建议：`_probe/src/bin/robustness.rs` → A05（`team-board.md:39`）；判定与正例设计 → A02（`docs/review-evidence/floating-audit.md`）

### F5（低）证据卫生：lead 的"原始捕获"是 UTF-16LE 且不含退出码

- `docs/review-evidence/raw-baseline-workspace-test.txt`：`BYTES=55670`，首 8 字节 `FF FE 63 00 61 00 72 00` ⇒ **UTF-16LE + BOM**；`HAS_NUL=True`。read 工具拒绝读取（报 `cannot read ...: binary file`），我只能用 PowerShell 解码后核对
- 文件内**没有**退出码行（我用 `'EXITCODE=0|exit code: 0'` 匹配 ⇒ `RAW_HAS_EXITCODE0=False`），而 `docs/review-evidence/baseline.md:51` 断言 `EXIT=0`
- 影响：退出码这一关键事实在该文件里不可自证（可以由我的两次独立运行补上，§4.1）
- 建议（lead）：重开一次捕获为 UTF-8（`... | Out-File -Encoding utf8`）并把 `EXITCODE=n` 写进文件；或在我的 §4.1 上引用退出码

### F6（低-中）`circuit-session` 的 package 元数据不继承 workspace（许可证不一致）

- `crates/circuit-session/Cargo.toml:1-6`：`version = "0.1.0"`、`edition = "2024"`、`license = "BSD-3-Clause"` 全部**硬编码**，且**没有** `rust-version`；另外 5 个 crate 都用 `version.workspace = true` / `license.workspace = true` / `rust-version.workspace = true`
- `Cargo.toml:16-20` 的 `[workspace.package]` 声明 `license = "MIT OR Apache-2.0"`、`rust-version = "1.85"`
- 实测（`cargo metadata --no-deps ... | ConvertFrom-Json`）：`circuit-session | 0.1.0 | BSD-3-Clause | rust_version=(空)`，其余 5 个均为 `MIT OR Apache-2.0` + `1.85`
- 影响：同一工作区两种许可证 + 一个 crate 无 MSRV 声明（不影响本轮测试结论；发布/合规层面不一致）。**我未修改**（只读），需 lead 决策是否本轮处理

### F7（信息）`robustness` 二进制没有任何非零退出路径 —— 与 lead 的 D4 冲突

- `_probe/src/bin/robustness.rs:91-99`：`fn main()` 顺序调用 5 个 `check_*` 函数后返回 `()`；全文**没有** `std::process::exit`、`ExitCode`、断言，也没有把结果汇总成布尔量（`Select-String -Pattern 'process::exit|ExitCode|all_ok'` 在 robustness.rs 中**零命中**，仅 6 处 `.expect(...)` 会在 panic 时以 101 退出）
- 对照 `_probe/src/main.rs:169-185`：`let mut all_ok = true; ... all_ok &= caseN_*(); ... if all_ok { ... } else { std::process::exit(1); }` ⇒ `probe` 有失败即非零的通道
- `team-board.md:70` 的 D4 要求"两个二进制必须失败即非零退出" ⇒ **当前 `robustness` 不满足**（今天 exit 0 并不能证明它的 5 项检查都通过——它不断言任何东西）
- 归属建议：A05（`_probe/src/bin/robustness.rs`）

### F8（信息）只读纪律的执行结果（含一次需要声明的偏差）

- 我用到的所有命令都只在 `target/` / `_probe/target/`（两者均被 `.gitignore:3-4` 忽略）下产生构建产物；**跟踪文件零改动**：`Cargo.lock` 与 `_probe/Cargo.lock` 在整轮取证前后哈希完全相同（§5.3）
- 取证期间 `git status --porcelain` 的唯一变化是 ` M README.md`——那**不是我**改的（我的写集只有本文件），是 lead 在同时写作（§7.4）
- **声明**：我在组织报告数据时，为跨步骤传递哈希表，在 `docs/review-evidence/` 下短暂创建过两个临时文件（`.tmp-hashes.json`、`.snap.json`），并在同一次程序内**立即删除**；最终该目录只有本报告与 lead 的三个文件（`git status` 未出现任何临时文件）。除此之外没有越界写入。

---

## 9. 未验证项与限制（明确不声称）

1. **`cargo nextest run --workspace` 的 388 数字**：`docs/testing.md:126-127` 声称 "nextest 报 388，差值就是那个 doc-test"。我只确认 `cargo nextest --version` ⇒ `cargo-nextest 0.9.145 (00af4550e 2026-09-16)`（已安装），**未运行 nextest**（`team-board.md:60` 把完整 workspace 测试留给 lead 的 Wave 2 集中时段）。该 388 属 **未验证**。
2. **总耗时**：`docs/testing.md:147` "`cargo test --workspace` 实测约 16 秒"——我未计时 ⇒ 未验证。（我只记录了单条 result 行的 `finished in` 值，e2e 最长 3.57 s。）
3. **`cargo clippy --workspace --all-targets -- -D warnings` = 0 警告**、**`cargo fmt --all -- --check` 无差异**（`docs/testing.md:95-103`、`README.md:5`）：我**没有**运行这两条（不在我的任务清单内，且会与 lead 的资源约定冲突）⇒ 未验证。
4. **F3 的成因判定**：我只证明"采样对齐贡献为 0、1 ps 上升沿量级不足、等效滞后 ≈ 250 ns"，**没有**做受控实验（改 `tmax`/`RELTOL`/上升沿看误差标度）来定论残差来源 ⇒ 成因未验证，归 A03/A06。
5. **"新增测试文件被自动发现"的新建实验**：只读约束下未新建文件实测（§6.4）。
6. **多平台**：只在 Windows MSVC + rustc 1.98.1 上验证；Linux/macOS 未验证（与 `docs/testing.md:265-267` 一致）。
7. **`_probe/target` 里的既有产物**：我给哈希时只覆盖源码与清单，不覆盖构建产物（不属于版本控制，且每次构建都会变）。
8. **并发写入**：`README.md` 在取证期间被 lead 改写；`docs/review-evidence/` 目录可能还有其他代理新增文件。除 `README.md` 漂移外，我未观察到其他已跟踪文件的哈希变化（`docs/testing.md`、`docs/backend-evaluation.md` 两次取样一致）。
9. **非零相位验证**（本轮任务之一）：我只记录到 lead 在 `baseline.md:77` 提到的换算位置（`crates/circuit-backend/src/thevenin.rs` 的 `to_degrees()`）；我自己**没有**做相位数值实验 ⇒ 未验证（属 A04/A09）。
10. **`docs/backend-evaluation.md` §5 里 Phase-0 六用例的其他数字**：我复跑了 `probe` 并核对了 Case 1/2/3/4/5/6 的输出片段与文档一致（§5.4），但**没有**逐位比对文档中每一个数值的末位。

---

## 10. 剩余风险（给 lead）

| # | 风险 | 应对 |
|---|---|---|
| R1 | `README.md` 正在被改写 ⇒ Wave 2 的"同一候选版本"冻结会失效 | 冻结那一刻重跑 §7.1+§7.2，并在 `final-gate.md` 记录新哈希；把 A07/A08/A09 新建的 3 个文件加进清单 |
| R2 | F3/F4 是"证据本身错了"，不是"文档写错了"：只改文字会留下未验证的数值结论 | 先由 A02/A03 给出可复核的判据与正例设计，再由 A05/A06 改 `_probe`，最后由 A10 同步 `docs/`；三者顺序不能反 |
| R3 | `robustness` 无非零退出路径（F7）⇒ 整改后仍可能"看起来通过" | 让 A05 把每项检查返回布尔并汇总 `process::exit(1)`，验收时**故意造一个失败**看退出码是否为 1 |
| R4 | 任务指定的 `crates/**/src/**/*.rs` 冻结范围**漏掉 tests/**、examples/**、Cargo.toml/lock** | 用 §7.2 的 GROUP-B 作为冻结范围（22 个文件） |
| R5 | `docs/testing.md` §1 与 §3 数字不一致（F1）、`architecture.md:325` 的 8 vs 9（F2）⇒ 文档出处的可信度被削弱 | A10 统一为实测值；建议在文档里直接给"复现命令 + 实测输出"而不是只给结论数字 |
| R6 | `circuit-session` 许可证/MSRV 与其他 crate 不一致（F6） | lead 决策：本轮不动（只读基线已记录），或另行派单 |

---

**报告结束**（A01 `a01-repo-forensics`，写入者：本代理唯一；本文件不在任何其他代理的写集内）。
