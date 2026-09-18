# 实现摘要（Lead 汇总，基于各代理实测证据）

日期：本轮。仓库：`F:\codexprojects\dsl000`，git HEAD `cb5d8a2`（工作区未提交，按用户要求不 commit / 不 push）。

## 1. 两个 P1 证据问题的修正

### P1-a 浮空节点用例

| 项 | 修正前 | 修正后（有实测证据） |
|---|---|---|
| 用例定性 | `robustness.rs` 把 `gnd-v1(1V)-a-r1(1k)-b` 称为“浮空节点” | 定性为**合法开路输出**：b 经 r1→a→v1→gnd 有直流通路 |
| 结论 | “后端用 gmin/漏电把节点拉住，返回 Ok 却不报错” | 该电路是确定的，`v(b)=v(a)=1 V`、`v1#branch=0`、派生 `i(r1)=(v(a)-v(b))/R=0`（1e-12 级），且 `GMIN` 从 1e-12 到 1e-3 结果逐位不变 |
| 反例 | 缺失（旧 `check_e_isolated_island` 的电路其实也有直流通路） | 新增两个真反例并断言后端返回 `Err(... matrix is singular, cannot solve)`：①整个连通块不接地 ②节点只经电容相连 |
| 退出行为 | `robustness` 无任何断言、恒 exit 0 | 13 个个例全部断言，任一 FAIL 打印失败清单并 exit 1（仓库外自检副本实测 exit 1） |

引擎机制（本轮从 vendored 源码取证，含非线性例外）：
线性无参考网络直接求解 → `Err(matrix is singular, cannot solve)`，**不点名节点**；
含非线性器件时走 Newton，`diag_gmin` 在 OP 被强制 0、gmin stepping 可用对角线 gmin 返回**随 GMIN 变化**的 `Ok`。
两种情形都不能替代前端诊断，因此 `circuit-core::connectivity` 的检查保留并加强说明（**没有**因为旧证据错就删除或放宽）。

### P1-b RC 瞬态误差归因

| 项 | 修正前 | 修正后 |
|---|---|---|
| 激励 | PULSE `tr = 1 ps`（以为很陡） | 实测被引擎夹紧：`tr.unwrap_or(tstep).max(tstep)`（`thevenin-0.5.0/src/waveform.rs:37`） |
| 参考解 | 5 个采样点 vs 理想阶跃 `1-e^{-t/τ}` | 全区间逐点 vs **分段有限斜坡解析解**（`x + expm1(-x)` 稳定形式） |
| 结论 | “0.00195 V 来自有限边沿与采样对齐” | 两者都不成立：1 ps 相对 1e-8 量级；参考值本就用实际返回时间（对齐误差恒为 0）。真实机理是**实际 500 ns 斜坡的齐次模态** `C·e^{-t/τ}`，`C≈-2.504e-3 V` |
| 判据 | 固定 0.01 V 绝对阈值 | §17 判据 `|a-e| ≤ atol + rtol·|e|`（TRAN: 1e-5 V / 1e-3），全区间逐点 |

关键数字（`_probe/src/main.rs`，probe exit 0）：
- 旧配置复现：匹配 500 ns 斜坡参考 → 1015 点 0 超限、max 6.278341e-7 V；
  理想阶跃参考 → max 2.491963e-3 V、259/1015 超限，且 0.25τ 处 1.945643e-3 V（即历史 1.95e-3）。
- 新基线（td=100 µs、T=1 µs、tmax=τ/1000=100 ns）：6025 点、max **4.999167e-7 V**、**0/6025 超限**，时间轴严格递增无重复。
- `max_step` 三档：τ/50 → 316 点 / 1.9933e-4 V / 15 点超限（**NOT-MET，保留**）；
  τ/200 → 1217 点 / 1.2490e-5 V / 3 点超限（**NOT-MET，保留**）；τ/1000 → 6025 点 / 4.9992e-7 V / 0 超限。
- 容差：`tmax` 钉死 `h_max` 时三种单因素配置**逐字段完全相同**（不可作为实验变量）；
  `tmax=None` 时单改 `reltol` 或 `abstol` 仍相同，只有两者同时改才改变时间轴 ⇒ 只能报告**交互作用**。

## 2. 产品路径回归（新增三个测试文件，各自唯一写入者）

| 文件 | 测试数 | 覆盖 |
|---|---|---|
| `crates/circuit-dsl/tests/reference_path_regression.rs` | 8 | 真实前端：合法开路输出被接受；孤立电阻网/只经电容/仅有电流源被 `E_NAME` 拒绝并点名节点与阻断器件；任务书里那个「电容到地」电路其实合法（反例守护） |
| `crates/circuit-backend/tests/transient_reference_regression.rs` | 4 | 参考解自检（ODE 残差 + 独立 RK4 ≤1.33e-13 V）；产品适配路径 5025 点逐点满足 §17（max 7.99e-8 V、0 超限）；声明 1 ps 被夹到 500 ns 的引擎行为被钉住（T_eff 参考 0 超限 vs 声明值参考 2.491958e-3 V / 259 超限）；`max_step` 到达引擎（点数 516/5025/50115 严格递增） |
| `crates/circuit-backend/tests/phase_regression.rs` | 6 | 非零 AC 相位多角度实/虚部（+30/+60/−45°）、`to_degrees` 被删必红（分离哨兵 2.544e-1）、RC 非零相位（误差 1.14e-16）、`sin` 的 `phi` 是度不是弧度（若按弧度误差 2.3e-1）、三类错误（符号/实虚交换/漏换算）分离 |
| `crates/circuit-dsl/tests/phase_syntax_regression.rs` | 6 | **DSL 层** `sin(..., phase:)` 的度→弧度换算（`phase: 90` → `1.57079632679489656`；0/+30/+90/−45/180 全覆盖；删 `to_radians()` 必失败 exit 101）；`phase:` 必须无量纲（`E_DIMENSION`）；钉住 AC 源当前**没有**相位语法 |

## 3. 生产代码与文档

**生产逻辑未改动**（本轮没有发现本项目自身的生产逻辑缺陷）。改动全部是**注释与文档的不实陈述**：

| 文件 | 改动 | 原因 |
|---|---|---|
| `crates/circuit-core/src/connectivity.rs` | 模块头注释 | 旧注释照抄了错误的 gmin 机制；改为区分线性/非线性两种引擎行为并给出源码位置 |
| `crates/circuit-dsl/src/elaborate.rs` | 注释 | 同上（浮空检查的理由） |
| `crates/circuit-backend/src/thevenin.rs` | 顶部注释第 3 条 + Tran 映射注释 | 浮空机制更正；`step` 不是“输出间隔”（引擎无输出抽样），`tmax` 不保证输出网格 |
| `crates/circuit-cli/tests/e2e.rs` | 测试文档注释 | 旧注释声称“引擎不报错、gmin 把节点拉住”，与实测相反；断言本身未改 |
| `README.md` | 分压示例、退出码契约、已知限制 | 引用的示例注释是旧版本；`EXIT_INTERNAL=2` 全仓库无返回点；补 PULSE 沿夹紧与无容差通道两条限制 |
| `docs/architecture.md` | 两处表格行 | 相位行（AC 有测试、DSL 的 `sin(phase:)` 缺测试）、浮空行（真无参考会 singular、单测 9 个） |
| `docs/language.md` | 浮空段 | 同 gmin 更正 |
| `docs/backend-evaluation.md`、`docs/testing.md`、`examples/rc_filter.cdsl` | A10 同步 | §4.5/§4.6/§5/§6/§7、测试分布与数值表、容差理由、退出码 2、示例注释 |

## 4. 门禁（Lead 实跑）

```text
cargo test --workspace                              exit 0   413 passed / 0 failed / 0 ignored（14 个 test binary + 5 个 doc-test 目标）
cargo clippy --workspace --all-targets -- -D warnings exit 0
cargo fmt --all -- --check                          exit 0
cargo run --manifest-path _probe/Cargo.toml --bin probe        exit 0（6/6 Case PASS）
cargo run --manifest-path _probe/Cargo.toml --bin robustness   exit 0（13/13 子用例 PASS）
cargo fmt --manifest-path _probe/Cargo.toml -- --check         exit 0
```

历史基线 389 passed（本轮开工实测一致）；新增 24 个测试（8 + 4 + 6 + 6）后为 **413 passed**。
原始输出保存在 `docs/review-evidence/raw-final-workspace-{test,clippy,fmt}.txt`（首轮门禁的日志为 `raw-workspace-*.txt`，保留作对照）。

## 5. 已知限制（本轮未解决，如实保留）

1. **源断点重启的精度限制**：源断点后首个接受步被强制回退成 Backward-Euler（`h1 = tmax/10`），
   局部误差约 `(V0/T)·h1²/(2τ)`。τ=100 µs、T=1 µs、`tmax=τ/200` 时实测 3/1217 点超 §17（max 1.2490e-5 V，允许 1.0012e-5 V）。
   产品路径回归电路固定 `delay=0`，**未覆盖运行中的源断点**；独立复核代理实测 `delay=100 µs`、`tmax=τ/100` 时
   10 点超限、max 4.991676e-5 V。已写入两份文档的“未验证/限制”，并给出最小复现，**未放宽任何阈值**。
2. **产品路径没有容差通道**：`thevenin.rs` 的 `build_circuit` 把 `options` 传空，`RELTOL/ABSTOL` 恒为引擎默认；DSL 也无语法。
3. **AC 源相位没有 DSL 语法**：`elaborate.rs` 写死 `phase_rad: 0.0`，非零 AC 相位只能在 IR 层验证。
4. **`uic` 未暴露**；**退出码 2 不可达**（`EXIT_INTERNAL` 无返回点，实测 0 与 1）；仅 Windows MSVC 验证。
5. **`_probe` 的 NOT-MET 档不影响其退出码**（Case 2 判据以推荐配置为准，NOT-MET 原样保留并标注）。
