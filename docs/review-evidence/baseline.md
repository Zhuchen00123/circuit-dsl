# 基线证据（Lead 实测，Wave 0）

日期：本轮开工时。
仓库：`F:\codexprojects\dsl000`，分支 `main`，HEAD `cb5d8a2`。

## 1. Git 状态（开工快照，未做任何修改与清理）

```text
 M RUST_CIRCUIT_DSL_PROMPT.md
?? AGENT_TEAM_EXECUTION_PROMPT.md
?? agent-team-switch.md
?? docs/prompt-review.md
```

以上用户改动在本轮**全部保留**，任何代理不得回滚、删除或格式化它们。

仓库内**没有** `AGENTS.md`。适用的仓库/协作规范来源：
`RUST_CIRCUIT_DSL_PROMPT.md`（原始产品目标）、`docs/prompt-review.md`（本轮复核记录）、
`agent-team-switch.md`（Team 环境切换记录）、`docs/architecture.md`、`docs/language.md`、`docs/testing.md`。

## 2. Rust 工具链

（见 `repo-forensics.md` 的独立取证；Lead 侧确认命令可用，未修改全局工具链。）

## 3. workspace 测试基线

命令（完整输出保存在 `docs/review-evidence/raw-baseline-workspace-test.txt`）：

```powershell
cargo test --workspace
```

逐 test binary 结果（passed）：

| binary 序号 | passed |
|---|---|
| 1 | 15 |
| 2 | 21 |
| 3 | 9 |
| 4 | 18 |
| 5 | 11 |
| 6 | 58 |
| 7 | 84 |
| 8 | 67 |
| 9 | 71 |
| 10 | 9 |
| 11 | 25 |
| 12 | 1（doc-test） |
| 其余（unit 汇总/空 harness） | 0 ×4 |

合计 **389 passed, 0 failed, 0 ignored**，`EXIT=0`。

这与 `docs/prompt-review.md` 记录的历史基线（389 通过，含 1 个 doc-test）**一致**：
工作区没有未提交的代码改动，历史基线在当前检出仍然成立。

## 4. `_probe` 基线

- `_probe` 被根 `Cargo.toml` 的 `workspace.exclude = ["_probe"]` 排除，`cargo test --workspace` **不会**覆盖它。
- `_probe/Cargo.toml`：`edition = "2024"`，依赖 `cirq-ir 0.5.0`、`num-complex 0.4.6`、`thevenin 0.5.0`、
  `thevenin-cirq 0.5.0`、`thevenin-types 0.5.0`。
- binaries：`probe`（`src/main.rs`）、`robustness`（`src/bin/robustness.rs`）、`currents`（`src/bin/currents.rs`）。
- 运行方式：`cargo run --manifest-path _probe/Cargo.toml --bin probe` / `--bin robustness`。
- 当前 `probe` 与 `robustness` 的退出码与输出见 `repo-forensics.md`、`backend-contract.md`、`floating-audit.md`。

## 5. 开工时已知的两个 P1 证据问题（本轮必须修正）

1. `_probe/src/bin/robustness.rs` 的 `base("float", ...)` 用例把「合法开路输出」当成「浮空节点/gmin 掩盖」的证据。
2. `_probe/src/main.rs::case2_rc_tran` 用 1 ps 上升沿 + 理想阶跃解在 5 个采样点对照，把 1.95e-3 V 偏差
   归因于「有限边沿与采样对齐」，没有依据。

## 6. Lead 已确认的架构事实（用于派工）

- `crates/circuit-backend/src/thevenin.rs:431`：`build_circuit(...)` 写死 `options: Vec::new()` —
  **产品路径目前没有 RELTOL/ABSTOL 通道**。
- `crates/circuit-backend/src/thevenin.rs:762-768`：`TranSpec.max_step` → `CqTran.tmax`，
  `output_interval` → `step`（默认 `span/1000`）。
- `crates/circuit-backend/src/thevenin.rs:649-654, 682-690`：`AcSpec.phase` 与 `Sin.phi` 均 `to_degrees()`。
- `crates/circuit-core/src/connectivity.rs`：`floating_nodes()` 以 `conducts_dc` 洪泛，
  电容/电流源不算直流通路。
