# 最终汇总（Lead）

日期：本轮。仓库 `F:\codexprojects\dsl000`，git HEAD `cb5d8a2`（**未 commit、未 push**，按用户要求）。

## 1. 两个 P1 证据问题的修正与实际数值结论

### P1-a 浮空节点用例（旧结论两处都错）

旧证据：`gnd - v1(1V) - a - r1(1k) - b(开路)` 被称为「浮空节点」，并据此声称「后端用 gmin 把节点拉住、返回 Ok 却不报错」。

修正后的事实（`_probe/src/bin/robustness.rs`，实跑 exit 0、13/13 子用例 PASS）：

| 场景 | 前端（产品路径） | 后端原始行为 |
|---|---|---|
| 合法开路输出 | 接受，`check`/`run` 退出码 **0**，CSV `v(a)=1, v(b)=1, i(v1)=0` | `Ok`；`v(b)=v(a)=1.0`（精确），`v1#branch=0`，派生 `i(r1)=0`；`GMIN` 1e-12→1e-3 逐位不变 |
| 整个连通块不接地 | `E_NAME` 分别点名 `a` 与 `b`，退出码 **1**（发生在后端被调用之前） | `Err(... matrix is singular, cannot solve)` |
| 节点只经电容相连 | `E_NAME` 点名 `out` + `attached but not conducting at DC: c1`，退出码 **1** | `Err(... matrix is singular, cannot solve)` |

引擎机制（从 vendored 源码取证）：线性无参考网络直接求解 → singular `Err`，**不点名节点**；
含非线性器件时走 Newton，gmin stepping 可返回**随 GMIN 变化**的 `Ok`。
两种情形都不能替代前端诊断，因此 `circuit-core::connectivity` 的检查**保留并加强了说明**（未删除、未放宽）。

### P1-b RC 瞬态误差归因（旧归因不成立）

旧证据：`tr = 1 ps` + 5 个采样点 vs 理想阶跃，把 `1.95e-3 V` 归因于「有限边沿 + 采样对齐」。

修正后的结论：**两点都不成立**。引擎把 PULSE 的 `tr` 夹紧到 `.tran` 步长（`max(tr, tstep)`，`thevenin-0.5.0/src/waveform.rs:37`），
所以声明的 1 ps **从未进入求解器**，实际是 500 ns 斜坡；而参考值本来就用实际返回时间求（对齐误差恒为 0）。
1.95e-3 V 正是**参考模型写错**产生的齐次模态残差 `C·e^{-t/τ}`，`C ≈ -2.504e-3 V`。

实测数字（`_probe`，probe exit 0；判据 `|a-e| ≤ atol + rtol·|e|`，TRAN `atol=1e-5 V`、`rtol=1e-3`）：

| 配置 | 点数 | max err | 超限 |
|---|---|---|---|
| 旧配置 vs **匹配 500 ns 斜坡** | 1015 | 6.278341e-7 V | 0 / 1015 |
| 旧配置 vs **理想阶跃** | 1015 | 2.491963e-3 V（0.25τ 处 1.945643e-3 V） | 259 / 1015 |
| 新基线 `tmax=τ/1000` vs 匹配斜坡 | 6025 | **4.999167e-7 V** | **0 / 6025** |
| `tmax=τ/200` | 1217 | 1.2490e-5 V | 3（**未达标，保留**） |
| `tmax=τ/50` | 316 | 1.9933e-4 V | 15（**未达标，保留**） |

容差实验（只改一个因素）：`tmax` 钉死 `h_max` 时三种单因素配置**逐字段完全相同** ⇒ 该配置下不可作为实验变量；
`tmax=None` 时单改 `reltol` 或 `abstol` 仍相同，只有两者同时改才改变时间轴 ⇒ 只能报告**交互作用**，不归因到单一设置。

## 2. 是否发现生产代码缺陷

**本项目自身的生产逻辑未发现缺陷**，因此 `crates/*/src` 的**逻辑一行未改**。本轮改的全部是**注释与文档中的不实陈述**：

| 文件 | 改动性质 |
|---|---|
| `crates/circuit-core/src/connectivity.rs` | 模块头注释：gmin 机制更正（区分线性/非线性，附源码位置） |
| `crates/circuit-dsl/src/elaborate.rs` | 注释：浮空检查的理由更正 |
| `crates/circuit-backend/src/thevenin.rs` | 顶部注释第 3 条 + Tran 映射注释：`step` 不是输出间隔（引擎无输出抽样） |
| `crates/circuit-cli/tests/e2e.rs` | 测试文档注释更正（断言未改） |
| `README.md` | 分压示例引用旧版本、`EXIT_INTERNAL=2` 不可达、PULSE 沿夹紧与无容差通道两条限制 |

**记录在案的引擎层限制（不做代码修复，明确列为未通过/未验证）**：
源断点后首个接受步被强制回退成 Backward-Euler（`h1 = tmax/10`，局部误差 `≈(V0/T)·h1²/(2τ)`）；
τ=100 µs、T=1 µs、`tmax=τ/200` 时实测 3/1217 点超判据（max 1.2490e-5 V > 允许 1.0012e-5 V）；
产品回归未覆盖运行中源断点，独立复核实测 `delay=100 µs`、`tmax=τ/100` 时 10 点超限、max 4.991676e-5 V。
**未放宽任何阈值**，已写入 `docs/backend-evaluation.md` §5/§7 与 `docs/testing.md` §5/§7。

## 3. 完成的自动化测试 / CLI QA / 独立审核

### 新增 24 个测试（4 个新测试目标，各自唯一写入者）

| 目标 | 数量 | 关键断言 |
|---|---|---|
| `crates/circuit-dsl/tests/reference_path_regression.rs` | 8 | 真实前端：合法开路接受；孤立电阻网/只经电容/仅电流源被 `E_NAME` 拒绝并点名阻断器件；任务书里「电容到地」那个电路其实合法（反例守护） |
| `crates/circuit-backend/tests/transient_reference_regression.rs` | 4 | 参考解自检（ODE 残差 ≤5.1e-15、独立 RK4 ≤1.33e-13 V）；产品适配路径 5025 点逐点满足 §17（max 7.99e-8 V）；1 ps 被夹到 500 ns 的引擎行为被钉住；`max_step` 到达引擎（点数 516/5025/50115 严格递增） |
| `crates/circuit-backend/tests/phase_regression.rs` | 6 | 非零 AC 相位多角度实/虚部；删 `to_degrees()` 必红（分离哨兵 2.544e-1）；`sin` 的 `phi` 是度不是弧度（误读为弧度差 2.3e-1）；符号/实虚交换/漏换算三类错误各 2.5e-1~5e-1 间距 |
| `crates/circuit-dsl/tests/phase_syntax_regression.rs` | 6 | DSL 层 `sin(phase:)` 度→弧度（`phase: 90` → `1.57079632679489656`；删 `to_radians()` 必失败 exit 101）；`phase:` 必须无量纲；钉住 AC 源当前无相位语法 |

失败方向都做了**仓库外副本反事实实验**（改坏一处 → 对应测试 exit 101/1）。

### 门禁（Lead 与最终门禁复核代理各跑一遍，均为 exit 0）

```text
cargo test --workspace                                exit 0   413 passed / 0 failed / 0 ignored
cargo clippy --workspace --all-targets -- -D warnings  exit 0
cargo fmt --all -- --check                             exit 0
cargo run --manifest-path _probe/Cargo.toml --bin probe       exit 0（6/6 Case PASS）
cargo run --manifest-path _probe/Cargo.toml --bin robustness  exit 0（13/13 子用例 PASS）
cargo fmt --manifest-path _probe/Cargo.toml -- --check        exit 0
```

历史基线 389（开工实测一致）→ 413（+24）。原始日志见 `docs/review-evidence/raw-final-workspace-*.txt`。

### 真实 CLI QA（独立代理，跑 `target/release/cdsl.exe`）

成功：`voltage_divider` CSV `5,3,0.002,-0.002`；`rc_filter` 三分析（op/ac 121 点/tran 1015 点）measure 独立重算相对差 ~1e-16；
`diode_rectifier` OP `v(vout)=0.6928715252252958`；**合法开路输出 exit 0、CSV `1,1,0`**。
失败：E_IO / E_SYNTAX / E_DIMENSION / 浮空 / 孤立电阻网 全部**退出码 1** 且诊断定位到节点与阻断器件。
发现 2 条 LOW：退出码 2 全仓库无返回点（已在 README 与 testing.md 更正）；`--out` 指向已存在文件时报「无法创建目录」而非防覆盖诊断（行为安全，已记为限制）。

### 独立审核

- **数值复核**（`numerical-review.md`）：**PASS**。独立重推解析解 + 自写 RK4 + 60 位 Decimal 复算；
  逐项复现所有数字（6025 / 4.999167e-7 / 0 超限 / 2.491963e-3 / 259 / 1.945643e-3 / 316·1217·6025 点）。
  确认**判据未被放宽、未筛样本、无循环论证**。
- **代码审核**（`code-review.md`）：**PASS（代码/测试）**。无越界写入、无削弱断言（`_probe` 的 a–e 检查是被**加强**）、
  无 `#[ignore]`/注释掉的断言/静默跳过；3 条反事实破坏实验全部让对应测试失败（exit 101）。
  4 条发现：F1（冻结漂移，P2）→ 已重新冻结并在清单 §0 披露；F2（`docs/architecture.md` 相位表述不实，P3）→ 已修正，
  并据此补了 DSL 层 `sin(phase:)` 测试；F3（计数口径，P4）→ 已修正；
  **F4（声称 `examples/rc_filter.cdsl:11` 的行号引用漂移）经 Lead 与最终门禁代理各自独立复核：不成立（误报）** —
  `thevenin.rs:772-775` 确实就是 `let step = spec.output_interval…` 块（`tmax` 在 `:781`），文件哈希与冻结值一致。
- **最终门禁复核**（`final-gate.md`）：**PASS**。6/6 门禁 exit 0、413 与声称一致、13 个代码/测试文件与清单一致、
  五套证据无矛盾（两处「看似冲突」核实为口径差异：407 vs 413 是测试加入时间差；1015 vs 1016 是 `_probe` 与产品路径的 `stop` 口径差）。
  无未处理阻断项。

## 4. 修改文件与证据文档

**新增测试**：`crates/circuit-dsl/tests/reference_path_regression.rs`、
`crates/circuit-dsl/tests/phase_syntax_regression.rs`、
`crates/circuit-backend/tests/transient_reference_regression.rs`、
`crates/circuit-backend/tests/phase_regression.rs`

**修改**：`_probe/src/main.rs`、`_probe/src/bin/robustness.rs`、
`crates/circuit-core/src/connectivity.rs`、`crates/circuit-dsl/src/elaborate.rs`、
`crates/circuit-backend/src/thevenin.rs`、`crates/circuit-cli/tests/e2e.rs`、
`README.md`、`docs/architecture.md`、`docs/language.md`、`docs/backend-evaluation.md`、
`docs/testing.md`、`examples/rc_filter.cdsl`（注释）

**证据文档**（`docs/review-evidence/`）：`team-board.md`（所有权/依赖/进度）、`baseline.md`、
`freeze-manifest.md`（最终哈希 + 门禁 + 漂移披露）、`repo-forensics.md`、`floating-audit.md`、
`rc-reference-math.md`、`backend-contract.md`、`next-round-contracts.md`、`numerical-review.md`、
`code-review.md`、`cli-qa.md`、`final-gate.md`、`implementation-summary.md`、`final-summary.md`。

## 5. 尚未验证事项与下一轮任务

**未验证（明确不声称）**：
- 运行中的源断点（`delay > 0`）的瞬态精度：已知超判据，最小复现与数字已记录，未修复。
- 产品路径**没有** RELTOL/ABSTOL 通道（适配层 `options` 恒空）；DSL 也没有容差语法。
- AC 源相位没有 DSL 语法，非零 AC 相位只在 IR 层验证；`sin(phase:)` 端到端（含 C/L 的状态电路）未覆盖。
- `uic` 产品路径不可达；退出码 2 不可达；`run --format json` 单独模式未验证；`diode_rectifier` 的 DC 扫描未逐点核对。
- 含非线性器件时前端判定与后端 gmin 行为之间的差异；AC/TRAN 下的浮空路径。

**下一轮（只读代理 `next-round-contracts.md` 已形成 10 条任务单）**：
1. **参数 DAG**：当前按声明顺序求值、前向引用报 `E_NAME`；`E_PARAM_CYCLE` 只有码没有生产者；拓扑参数无静态标记（扫描到第二个差异点才报错）。
2. **结果表达式接入**：`circuit-results::expr` 求值器完备，但 `measure` 只收 `v()/i()` 探针，用户路径缺失。
3. README 分压示例回归测试；`E_PARAM_CYCLE` 依赖链诊断；拓扑参数静态拒绝；`save` 逐分析校验；UTF-8 BOM 源文件处置。
4. 断点重启步的误差控制（限制断点后首步长 / 二阶起步）与运行中源断点的产品回归。
5. 适配层容差通道（`options` 暴露 `reltol/abstol`）与对应文档。
