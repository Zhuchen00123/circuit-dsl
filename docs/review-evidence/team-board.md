# Team board — 本轮任务文件所有权与依赖表

仓库：`F:\codexprojects\dsl000`
Lead：lead（本文件唯一写入者）
建立时间：本轮 Wave 0 开始时。

## 0. 开工基线（Lead 实测）

- 分支 `main`，HEAD `cb5d8a2`。
- 开工时已有用户改动（必须保留）：
  - ` M RUST_CIRCUIT_DSL_PROMPT.md`
  - `?? AGENT_TEAM_EXECUTION_PROMPT.md`
  - `?? agent-team-switch.md`
  - `?? docs/prompt-review.md`
- 无 `AGENTS.md`；仓库规范见 `agent-team-switch.md`、`docs/prompt-review.md`、`RUST_CIRCUIT_DSL_PROMPT.md`。
- Lead 实跑 `cargo test --workspace --quiet`：**exit code 0**，全部测试通过（完整计数见 `baseline.md`）。
- `_probe` 被根 `Cargo.toml` 的 `workspace.exclude` 排除，必须单独运行。

## 1. 范围约束（全体适用）

- 只修正浮空节点验证证据与 RC 瞬态误差证据，补非零相位验证与产品路径回归。
- **不**新建项目、不换后端、不新增器件、不做 GUI/绘图/LSP/WASM/GUI、不搭自研求解器。
- **不**为本轮测试新增 DSL 相位语法。
- 不发布、不部署、不 commit、不 push。
- 禁止放宽阈值掩盖失败、禁止删除既有断言、禁止 `#[ignore]`。
- 每个代理都不是唯一在库中工作的代理；不得回滚/重置/覆盖他人改动。

## 2. 文件所有权表（同一文件同时只有一个写入者）

| 文件 | 当前写入者 | 状态 | 依赖 | 验收负责人 |
|---|---|---|---|---|
| `docs/review-evidence/team-board.md` | lead | writing | — | lead |
| `docs/review-evidence/baseline.md` | lead | writing | — | lead |
| `docs/review-evidence/repo-forensics.md` | A01 | reading | — | lead |
| `docs/review-evidence/floating-audit.md` | A02 | reading | — | lead |
| `docs/review-evidence/rc-reference-math.md` | A03 | reading | — | lead |
| `docs/review-evidence/backend-contract.md` | A04 | reading | — | lead |
| `docs/review-evidence/next-round-contracts.md` | A14 | reading | — | lead |
| `_probe/src/bin/robustness.rs` | A05 | pending | A02 | lead |
| `_probe/src/main.rs` | A06 | pending | A03, A04 | lead |
| `crates/circuit-dsl/tests/reference_path_regression.rs`（新建） | A07 | pending | A02 | lead |
| `crates/circuit-backend/tests/transient_reference_regression.rs`（新建） | A08 | pending | A03, A04 | lead |
| `crates/circuit-backend/tests/phase_regression.rs`（新建） | A09 | pending | A04 | lead |
| `docs/backend-evaluation.md` | A10 | pending | 实现冻结 | lead |
| `docs/testing.md` | A10 | pending | 实现冻结 | lead |
| `docs/review-evidence/numerical-review.md` | A11 | pending | 实现冻结 | lead |
| `docs/review-evidence/code-review.md` | A12 | pending | 实现冻结 | lead |
| `docs/review-evidence/cli-qa.md` | A13 | pending | 实现冻结 | lead |
| `docs/review-evidence/final-gate.md` | A15 | pending | A10–A13 | lead |
| `README.md` | lead | pending | — | lead |
| `Cargo.toml` / `Cargo.lock` / `_probe/Cargo.toml` | lead | frozen | — | lead |
| `crates/*/src/**` 生产代码 | lead | frozen | — | lead |
| `docs/review-evidence/implementation-summary.md`、`final-summary.md` | lead | pending | — | lead |

规则：新增 3 个集成测试文件各自唯一命名，已确认各 crate 无 `autotests = false`（见 repo-forensics）。

### 2.1 进度快照

| 交付物 | 拥有者 | 状态 | 实测 |
|---|---|---|---|
| `docs/review-evidence/repo-forensics.md` | a01 | **完成** | 389 passed / exit 0 独立复核一致；SHA-256 清单 41+22+7 项 |
| `docs/review-evidence/floating-audit.md` | a02 | **完成** | 三场景实测 + 11 处错误结论定位（E1–E11） |
| `docs/review-evidence/rc-reference-math.md` | a03 | **完成** | 稳定形式 + 判据允许误差表 + 归因量级判定 |
| `docs/review-evidence/backend-contract.md` | a04 | **完成** | options/tmax/uic/gmin/相位/时间轴 六项源码级证据 |
| `docs/review-evidence/next-round-contracts.md` | a14 | **完成** | 8 行差异表 + 10 条下一轮任务 |
| `_probe/src/bin/robustness.rs` | a05 | **完成** | exit 0，13/13 PASS；仓库外自检 exit 1 |
| `crates/circuit-dsl/tests/reference_path_regression.rs` | a07 | **完成** | exit 0，8 passed；仓库外反转副本 exit 101 |
| `crates/circuit-backend/tests/transient_reference_regression.rs` | a01（复用） | 进行中 | — |
| `_probe/src/main.rs` | a06 | 进行中 | — |
| `crates/circuit-backend/tests/phase_regression.rs` | a14（复用） | 进行中 | — |
| 生产区/README/architecture/language 的错误解释修正 | lead | **完成** | 7 个文件；详见 §7 D8 |
生产代码默认冻结；若发现生产缺陷由 lead 评估后转移精确写集。

## 3. 资源调度

- 完整 workspace 测试 / `cargo fmt --all` / `cargo clippy --workspace` 只由 lead 执行（Wave 2 集中时段）。
- 子代理只跑自己受影响 crate 的局部测试：`cargo test -p <crate>` 或 `cargo test --test <file>`。
- `_probe` 是独立工程，编译互不加锁；A05/A06 同时编译可能争用 `_probe/target`，由 lead 串行化其首次 build（已知共享同一 target 目录，Cargo 自身加锁，等待不是失败）。
- 实验输出目录：`_probe/out/`（A06 独占 `_probe/out/rc-tran/`），A13 独占 `docs/review-evidence/cli-qa-output/`。禁止覆盖他人输出。

## 4. 关键架构决策（lead）

- **D1**：`max_step` 经适配层映射到 Thevenin `TranAnalysis::tmax`（`thevenin.rs:767`），是唯一已接入产品路径的求解器数值控制。
- **D2**：产品 IR 目前**没有**容差（RELTOL/ABSTOL）通道：`build_circuit` 写死 `options: Vec::new()`（`thevenin.rs:431`）。因此容差敏感性实验只能在 `_probe` 直接构造 `cirq_ir::Circuit` 时做；产品路径的容差控制列为**未实现/下一轮**，本轮不得声称已验证。
- **D3**：相位约定为「项目内部弧度 → 后端度」，`map_source` 对 `AcSpec.phase` 与 `Sin.phi` 都做 `to_degrees()`。非零相位验证必须同时覆盖 AC 与 SIN 两条路径，并核对实/虚部而不只幅度。
- **D4**：`_probe` 两个二进制必须「失败即非零退出」，且保留可观察的 PASS/FAIL 明细。
- **D5**：本轮不改 `crates/*/src` 公共 API；测试只通过既有公开接口。

## 5. 关卡

- **G0**：基线、写集、依赖、用户改动确认完成。
- **G1**：修正用例与新增回归测试实跑通过；失败路径返回非零；无删除断言/无 ignore/无偷偷放宽阈值。
- **G2**：数值复核 + 代码审核无未处理阻断项；CLI QA 覆盖成功与失败路径；所有证据对应同一候选版本（SHA-256 冻结）。
- **G3**：文档与实测一致；最终审核对应最终 diff；完成清单与未验证项明确。

## 6. 资源实测约束与代理复用（重要）

实测：本会话 Team **成员上限 = 8**（第 9 个 spawn 返回 `Team member limit 8 reached`）。
因此改为**复用已完成的代理**执行后续任务（用 `send_message` 派发新的自包含任务），
而不是继续 spawn。用户要求的「命名子代理从空白上下文启动」在 spawn 时已满足（`context: fresh`）；
复用时不引入 lead 的历史上下文，任务正文全部自包含。

当前 8 个成员与角色映射：

| 成员 | 第一轮角色 | 第二轮角色（复用） | 状态 |
|---|---|---|---|
| a01-repo-forensics | A01 仓库取证（只读） | **A08 瞬态适配回归实现** | 复用中 |
| a02-floating-audit | A02 浮空证据审计（只读） | **A11 数值独立复核**（预留） | inactive |
| a03-rc-reference-math | A03 数学参考审计（只读） | **A10 文档同步**（预留） | idle |
| a04-backend-contract | A04 后端接口取证（只读） | **A12 代码审核**（预留） | running |
| a05-floating-cases | A05 浮空用例实现 | — | running |
| a06-rc-tran | A06 瞬态验证实现 | — | running |
| a07-topology-regression | A07 拓扑产品回归 | — | running |
| a14-contract-gap | A14 契约差异分析（只读） | **A09 相位适配回归实现** | 复用中 |

后续仍需 A13（CLI QA）与 A15（最终门禁复核），由先完成的实现代理复用承担。
lead 保持独占：README、team-board、baseline、final-summary、implementation-summary、manifests、生产代码修复。

## 7. 新证据带来的范围修订（lead 决策）

- **D6**：thevenin 0.5.0 的 `src/waveform.rs` 把 PULSE 的 `tr` 夹紧到 `.tran` 步长 `max(tr, tstep)`。
  ⇒ 旧探针「1 ps 上升沿」实际是 500 ns 斜坡；1.95e-3 V 残差与 `C·e^{-t/τ}`（C ≈ −2.50e-3 V）一致。
  所有新瞬态验证必须以**匹配的有限斜坡**为参考，且 `tr >= output_interval` 以避免被夹紧；
  被夹紧的情形另写一条可复现证据测试。
- **D7**：前端浮空诊断码是 `E_NAME`（`Code::Name`），在 `elaborate.rs` 的 `finish_circuit` 中产生，
  **发生在后端被调用之前**；CLI 退出码 1。`_probe` 直连后端，因此它只能观察后端原始行为。
- **D8**：lead 已完成生产区/文档中错误浮空解释的修正：
  `crates/circuit-core/src/connectivity.rs`（模块头）、`crates/circuit-dsl/src/elaborate.rs`（注释）、
  `crates/circuit-backend/src/thevenin.rs`（顶部注释第 3 条）、`crates/circuit-cli/tests/e2e.rs`（测试注释）、
  README、`docs/architecture.md`（两处，含过时的「相位无数值测试」与「8 个单测」）、`docs/language.md`。
  `docs/backend-evaluation.md` 与 `docs/testing.md` 由 A10 负责（见 §2）。
