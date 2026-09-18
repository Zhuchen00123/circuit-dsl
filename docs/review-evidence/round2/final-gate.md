# 第 2 轮最终门禁与独立复核（Lead）

权威版本：**rev4**（`target/round2-logs/freeze-manifest.txt`，16 个文件 + `target\debug\cdsl.exe` 哈希）。
本文件是最终门禁记录，配合 `implementation-summary.md`（实施与验收）、`design-freeze.md`（契约）、`team-board.md`（所有权）阅读。

## 1. Lead 亲自实跑的门禁（真实退出码，2026-09-18 rev4 状态）

| 命令 | 退出码 | 结果 |
|---|---|---|
| `cargo test --workspace` | **0** | 457 passed / 0 failed（24 个 `test result:` 行；日志 `target/round2-logs/rev4-workspace-test.txt`） |
| `cargo clippy --workspace --all-targets -- -D warnings` | **0** | 0 条告警（`target/round2-logs/final-clippy.txt`） |
| `cargo fmt --all -- --check` | **0** | 无差异（`target/round2-logs/final-fmt.txt`） |
| `cargo fmt --manifest-path _probe/Cargo.toml -- --check` | **0** | 无差异 |
| `cargo run --manifest-path _probe/Cargo.toml --bin probe` | **0** | 6/6 Case PASS（NOT-MET 行原样保留） |
| `cargo run --manifest-path _probe/Cargo.toml --bin robustness` | **0** | 13/13 子用例 PASS |
| `cargo run --manifest-path _probe/Cargo.toml --bin breakpoint_study` | **0** | 14/14 contract checks；2 条 §17 NOT-MET 保留 |
| `cargo run --manifest-path _probe/Cargo.toml --bin tran_contract` | **0** | 12/12 pins |

CLI 端到端（rev4 二进制 `5B688194F32D9020FA591647A41324C047C6882CB1933D9481F2B76248A3024B`）：

| 场景 | 退出码 | 关键观测 |
|---|---|---|
| `run --experiment coarse`（`output_interval: 100.ns`） | 0 | `tran1: 21 time points`；末点 `v(out)=0.01975231511815381`（与 fine 逐位相同） |
| `run --experiment fine`（`output_interval: 1.ns`） | 0 | 2001 点 |
| `check negative-interval.cdsl`（`-1.ns`） | 1 | `E_VALUE`，不回退默认值 |
| `check examples/rc_filter.cdsl` | 0 | 声明 `rise: 1.ns` 被兑现（1019 点） |
| REPL `:load` + `:run coarse --out` | 0 | 与文件模式 CSV/JSON **逐字节相同** |

## 2. 独立复核与 QA（各自独立于实现者）

| 复核 | 结论 | 关键数字 |
|---|---|---|
| 数值复核 `numerical-review.md`（r4） | **PASS** | 4 条独立参考路线互检 ≤1.7e-13 V；6 档配置误差表与点数**零差异**；253 条超限样本逐字段一致；§17 与 `PROMPT.md:620` 逐字核对未放宽；`#[ignore]` 全仓 0 处；rev4 清单 **17/17** 一致 |
| 代码审核 `code-review.md`（r3） | 首轮 **NEEDS_FIX** → rev3 复核 **PASS** | W6-1 clippy 门禁（含删 fingerprint 强制重检 exit 0）、W6-2 归因错误（6 例矩阵全符合）均已关闭；追问的 LOW 项已在 rev4 补断言并做判别力对照 |
| CLI/REPL QA `cli-qa.md`（w5） | **PASS** | 87 条命令、退出码集合 {0,1}；rev2 复验与 rev1 逐字节相同；失败路径 stdout 0 字节、零结果文件；examples `check` 7/7、`run` 9/9 |
| 内核取证 `kernel-contract.md`（r1） | **PASS** | `tstep` 全部使用点、断点重启规则、输出录制规则均有文件:行号 |
| 产品路径取证 `product-path.md`（r2） | **PASS** | 重采样层次、双视图接口、测量口径的调用点清单 |

## 3. 两个已确认问题的回归（自动化）

| 问题 | 回归位置 | 判别力证据 |
|---|---|---|
| P1-a：`output_interval` 静默展宽声明边沿 | `output_interval_regression.rs::output_interval_does_not_change_the_solved_trace`、`::a_declared_ten_nanosecond_edge_is_delivered_on_the_raw_grid`；`transient_reference_regression.rs::declared_rise_below_output_interval_is_not_widened` | 三跑原始网格逐位相同；50 ns 处 `v(vin)=1.0`（旧值 `0.5002375000000003`）；旧契约参考 257/1024 超限而声明边沿参考 0 超限 |
| P1-b：非法 `output_interval` 静默回退 | `tran_option_validation.rs`（15 例）；`tran_output_interval.rs` | `-1.ns`/`0.s`/inf/NaN → `E_VALUE` 且整体 `Err`、不产生计划；CLI `check`/`run` exit 1、stdout 0 字节 |
| 附加：能力错误不得误归因 | `output_interval_regression.rs::the_step_budget_blames_the_waveform_not_the_users_max_step` | 把守卫改回"只看步数"→ 该测试 FAIL exit 101；恢复后通过（文件哈希回到 rev3 值） |

## 4. 未完成 / 未验证清单（准确交接）

1. 任务 C（结果表达式接入 `MeasureRequest.target`）与任务 D（参数 DAG）**本轮未做**，按计划留待下一轮。
2. 断点重启精度只在单 RC + 单一激励上建立了可达标界 `h_max ≤ 10·sqrt(2·atol·τ·T/V0)`；多极点/电感/二极管/极端 T·τ 比未验证；超界配置（τ/200 为 1.25×、τ/50 为 19.5×）保留为限制实验。
3. 产品路径**无容差通道**（`RELTOL/ABSTOL/VNTOL/GMIN` 恒为引擎默认）；容差实验只在 `_probe` 完成，且证明容差改动**不改变**断点重启步与 §17 结论。
4. 重采样边界组合中 `uic=true` 未验证；`start_s ≠ 0`、同实验多个 `tran` 已由 Lead 实测（分别 11 点、21/2015 点）；`Axis::Parameter` 不走重采样（代码保证，未单测）。
5. 重采样规模 `E_LIMIT` 只有 `run` 能报（`check` 为静态检查）；单点/退化轴原样返回且不写 `output_grid` 元数据。
6. 非法 `output_interval` 会伴随一条级联 `E_ARGUMENT`（分析任务被丢弃），不影响退出码。
7. 平台与构建：仅 Windows MSVC / debug profile；release、非 Windows、`cargo nextest`、性能与内存未测。
8. `EXIT_INTERNAL=2` 不可达（源码静态判定：定义 1 处、无构造点），未通过注入内部错误实证。
9. `max_step` 无运行期步数上限（用户显式请求）；≈1e9 步组合未实跑。
10. `crates/circuit-backend/tests/transient_reference_regression.rs` 的 `effective_edge()` 辅助函数名仍是旧映射语义（只用于 rise ≥ interval 的同值用例，结论不受影响）；属命名遗留，未改以免再次触发冻结。
11. `docs/review-evidence/round2/test-inventory.md` 是修复前快照，其引用的 `transient_reference_regression.rs:684` 已被 `:725 …is_not_widened` 取代（代码审核 W6-8 已记录）。

## 5. 工作区状态

git HEAD 仍为 `cb5d8a2`；本轮**未 commit / 未 push / 未发布 / 未 rebase**；工作区未提交成果（含上一轮遗留）完整保留。
本轮新增/修改文件清单见 `implementation-summary.md` §6；`git status --short` 可见 `crates/`、`docs/`、`examples/`、`README.md`、`_probe/src/bin/` 下的全部改动。
