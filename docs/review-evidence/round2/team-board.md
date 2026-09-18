# 第 2 轮团队板与文件所有权（Lead 维护）

仓库 `F:\codexprojects\dsl000`。规则：同一个文件只能有一个当前写入者；同一目录的不同文件可并行。
所有命名子代理**空白上下文启动**，任务正文自包含。任何代理不得 `git commit/push/checkout/reset/clean`，
不得修改 `Cargo.toml`/`Cargo.lock`，不得撤销他人修改。

## 文件所有权表（本文件为唯一权威）

| 路径 | 当前唯一写入者 | 说明 |
|---|---|---|
| `crates/circuit-core/src/plan.rs` | lead | `TranSpec` 语义与文档、公共计划类型 |
| `crates/circuit-dsl/src/elaborate.rs` | lead | `output_interval`/`max_step` 值校验 |
| `crates/circuit-backend/src/thevenin.rs` | lead | `h_print` 计算、能力检查、元数据 |
| `crates/circuit-results/src/resample.rs`（新） | lead | 独立输出重采样 |
| `crates/circuit-results/src/lib.rs` | lead | 模块注册与重导出 |
| `crates/circuit-results/src/dataset.rs` | lead | `BackendInfo::setting` 读取器 |
| `crates/circuit-session/src/execute.rs` | lead | `RunOutcome.output_datasets`、重采样挂接、测量语义 |
| `crates/circuit-session/src/session.rs` | lead | REPL 路径一致（导出/展示改用 output 数据集） |
| `crates/circuit-cli/src/run.rs` | lead | 文件模式导出 output 数据集 |
| `examples/rc_filter.cdsl` | w7-docs | 注释中的旧“边沿被拉宽到 500ns”说明已过时 |
| `README.md`、`docs/review-evidence/round2/team-board.md`、`design-freeze.md`、`implementation-summary.md`、`final-gate.md` | lead | 交付与门禁证据 || `crates/circuit-backend/tests/transient_reference_regression.rs` | w1-transient-contract | 旧契约测试重写 + 保留其它断言 |
| `crates/circuit-backend/tests/adapter.rs` | w1-transient-contract | 受影响 tran 断言 |
| `crates/circuit-backend/tests/phase_regression.rs` | w1-transient-contract | 受影响 tran 断言（不改相位结论） |
| `crates/circuit-backend/tests/output_interval_regression.rs`（新） | w1-transient-contract | 任务 A 产品路径回归（后端层） |
| `crates/circuit-session/tests/tran_output_interval.rs`（新） | w1-transient-contract | 任务 A 会话层：raw vs output、测量不变 |
| `crates/circuit-dsl/tests/tran_option_validation.rs`（新） | w2-dsl-validation | 任务 A 输入层拒绝 |
| `crates/circuit-backend/tests/source_breakpoint_regression.rs`（新） | w3-breakpoint | 任务 B 产品路径回归 |
| `_probe/src/bin/breakpoint_study.rs`（新） | w3-breakpoint | 任务 B 内核/容差实验 |
| `_probe/src/bin/tran_contract.rs`（新） | w3-breakpoint | 底层行为钉住（tstep clamp）保留在独立工程 |
| `docs/language.md` | w7-docs | 瞬态参数语义、输出网格契约、限制 |
| `docs/backend-evaluation.md` | w7-docs | 映射说明、能力错误、容差/断点限制 |
| `docs/testing.md` | w7-docs | 测试分布与判据 |
| `docs/review-evidence/round2/kernel-contract.md` | r1-kernel-recon | 只读取证报告 |
| `docs/review-evidence/round2/product-path.md` | r2-product-path | 只读取证报告 |
| `docs/review-evidence/round2/test-inventory.md` | r3-test-inventory | 只读取证报告 |
| `docs/review-evidence/round2/repro-baseline.md` | r4-repro-qa | 修复前真实 CLI 复现 |
| `docs/review-evidence/round2/numerical-review.md` | w4-numeric-review | 独立数值复核 |
| `docs/review-evidence/round2/code-review.md` | w6-code-review | 独立代码审核 |
| `docs/review-evidence/round2/cli-qa.md` | w5-cli-qa | 真实 CLI/REPL QA |
| `docs/review-evidence/round2/final-summary.md` | lead（汇总 w8 复核） | 最终复核 |
| `target/round2-*/` | 各自子目录 | 临时产物，不入版本管理 |

## 波次

- **Wave 0**：r1/r2/r3/r4 并行只读取证 + Lead 基线实测。
- **G0**：设计冻结（`design-freeze.md`）+ 所有权表（本文件）。
- **Wave 1**：Lead 实现任务 A 产品代码；w1/w2/w3 并行写各自测试文件。
- **Wave 2**：冻结哈希 → w4 数值复核、w6 代码审核、w5 CLI QA。
- **Wave 3**：w7 文档；Lead 全量门禁；w8 最终证据复核。

## 状态（收尾，全部完成）

| 任务 | 所有者 | 状态 | 产出 |
|---|---|---|---|
| task-1 内核契约取证 | r1-kernel-recon | completed | `kernel-contract.md` |
| task-2 产品路径取证 | r2-product-path | completed | `product-path.md` |
| task-3 测试清点 | r3-test-inventory | 交付完成（board 未标记） | `test-inventory.md`（修复前快照） |
| task-4 复现基线 | r4-repro-qa | completed | `repro-baseline.md` |
| task-5 任务 A 后端+会话回归 | w1-transient-contract | completed | `output_interval_regression.rs`、`tran_output_interval.rs`、`transient_reference_regression.rs` |
| task-6 DSL 层校验回归 | w2-dsl-validation | completed | `tran_option_validation.rs` |
| task-7 任务 B 断点回归 | w3-breakpoint | completed（含 rev3 格式化返修） | `source_breakpoint_regression.rs`、`_probe/src/bin/{breakpoint_study,tran_contract}.rs` |
| task-8 真实 CLI/REPL QA | w5-cli-qa | completed | `cli-qa.md` |
| task-9 独立数值复核 | r4-repro-qa | completed | `numerical-review.md`（rev4 终检 17/17） |
| task-10 独立代码审核 | r3-test-inventory | completed（rev3 复核 PASS） | `code-review.md` |
| task-11 文档同步 | r2-product-path | completed | `language.md`、`backend-evaluation.md`、`testing.md`、`architecture.md`、`examples/rc_filter.cdsl` |
| 任务 A+B 生产代码与整合 | lead | completed | 见 `implementation-summary.md` §6；门禁见 `final-gate.md` |

## 冻结版本链

rev1（20:59 候选）→ rev2（`thevenin.rs` 注释-only）→ rev3（clippy 门禁 + E_LIMIT 归因 + capability note + rustfmt）→ **rev4**（代码审核建议的归因回归测试；产品代码同 rev3）。
每次冻结外写入都通知了全部在审者，并附哈希、真实命令与不变性证据。当前权威清单：`target/round2-logs/freeze-manifest.txt`。

