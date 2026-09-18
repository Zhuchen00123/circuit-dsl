# A02 浮空节点证据审计（floating-audit）

审计者：a02-floating-audit（只读探证）
审计时间：2026-09-18 19:20–19:45 +08:00
仓库：F:\codexprojects\dsl000
基线版本：HEAD `cb5d8a212f66922181580900a05fb3d42abe32f2`（2026-09-18 17:05:21 +0800，subject: Add an interactive REPL, and settle the syntax questions it raised）
审计结论对应的工作区状态：审计期间 `crates/**` 未发生改动（两次 `git status --porcelain` 核对：只有 `M README.md`、`M RUST_CIRCUIT_DSL_PROMPT.md` 与 `docs/review-evidence/` 等未跟踪文件属于他人改动）。

**只读声明**：本审计未创建或修改仓库内任何生产/测试/文档文件（本报告除外）；未 commit、未 push。
所有最小实验工程与 `.cdsl` 临时文件都建在仓库外 `%TEMP%\a02\`（= `C:\Users\15185\AppData\Local\Temp\a02`）下。

---

## 0. 结论速览

| 项目 | 结论 |
|---|---|
| (a) `robustness.rs` 中被称为 float 的用例是什么、后端真实返回什么 | **PASS**（电路、数值、退出码均已实测） |
| (b) 前端在哪里、按什么规则、用什么诊断码拒绝真正无参考电势的电路；CLI 退出码 | **PASS**（源码定位 + 3 组前端实测） |
| (c) 现有测试/文档中依赖错误浮空解释的结论清单 | **PASS**（逐条定位到文件:行） |
| 文档与注释的错误是否已修正 | **NEEDS_FIX**（错误句子在 2026-09-18 19:42 仍在工作区，见 §7） |
| BLOCKED | 无 |

三句话总结：

1. `robustness.rs` 的 float 用例（`robustness.rs:132-148`）是**合法开路输出**：`gnd - v1(1V) - a - r1(1k) - b(开路)`，后端返回 `Ok`，`v(b) = 1 V`（精确值，17 位打印为 `1.00000000000000000e+00`）。node b **有**直流通路（b→r1→a→v1→gnd），旧结论「b 无对地通路 / gmin 把节点拉住」两处都错。
2. 对**真正无参考**的线性网络（孤立电阻网、孤立源岛、只经电容连接的节点、孤立电容岛），后端实测一律返回 `Err(simulation failed: failed to solve MNA system: matrix is singular, cannot solve)`，**不是**旧文档说的「返回 Ok，不报错」。
3. 前端检查（`circuit-dsl` 在 `finish_circuit` 调 `circuit-core::floating_nodes`）仍然必要，但它真实的必要性是**定位诊断**（后端 singular 错误不指向任何节点），而不是「后端会拿有限值静默成功」；诊断码是 `Code::Name` → `E_NAME`，CLI `check`/`run` 退出码 **1**。

---

## 1. 版本与文件哈希（SHA-256 前 16 位，审计期间实测）

| 文件 | 哈希 |
|---|---|
| `_probe/src/bin/robustness.rs` | `141198FC9C79F66D` |
| `crates/circuit-core/src/connectivity.rs` | `D66E2118BFF03EA2` |
| `crates/circuit-dsl/src/elaborate.rs` | `6E76B5F900676753` |
| `crates/circuit-backend/src/thevenin.rs` | `6337B9EB2A9917F5` |
| `crates/circuit-cli/src/main.rs` | `6C1B1680156E9C91` |
| `crates/circuit-cli/src/check.rs` | `081A31C4FB2BA499` |
| `crates/circuit-cli/src/run.rs` | `440245E71E7344EA` |
| `crates/circuit-cli/tests/e2e.rs` | `1A491B4499C8A9B4` |
| `docs/backend-evaluation.md` | `0D275B340A6FF6A4` |
| `docs/testing.md` | `60C9EE4C14B9B487` |
| `docs/architecture.md` | `8344A0C50DAA5A44` |
| `docs/language.md` | `A0E0B32E1F7F9055` |
| `README.md` | `D3669EE5285F7313`（审计时正被 lead 修改） |

依赖版本核对（三个 lockfile 一致，均为 **0.5.0**）：
`Cargo.lock` / `_probe/Cargo.lock` / `%TEMP%\a02\back\Cargo.lock` 中 `thevenin = "0.5.0"`、`thevenin-types = "0.5.0"`、`cirq-ir = "0.5.0"`。

库源码阅读位置：`C:\Users\15185\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\thevenin-0.5.0\`（下称 `thevenin-0.5.0/`）。

---

## 2. 实际执行的命令与真实输出

### 2.1 `_probe` robustness 实跑

命令（在仓库根）：

```powershell
cargo run --manifest-path _probe/Cargo.toml --bin robustness
```

真实输出（逐字摘录，`EXITCODE=0`）：

```text
=== Thevenin 0.5.0 robustness / isolation checks ===

[a] Failure reporting: singular / ill-posed circuit
  OK: returned Err(simulation failed: failed to solve MNA system: matrix is singular, cannot solve)

  floating node v(b) = 1 (Ok returned, no ground path)
  => dangling-node detection must be done by OUR frontend

[b] Thread isolation: 8 concurrent simulations with distinct inputs
  ... (8× PASS, 与本次无关)
[c] Malformed circuit: terminal referencing a nonexistent net id
  OK: Err(simulation failed: ... terminal `neg` references unknown net id)
[d] Save subsetting via `circuit.save`
  save=["v(mid)"] -> 3 vectors: ["v(mid)", "v(in)", "v1#branch"]
[e] Isolated island (AC-coupled block) operating point
  v(out) = 1
  v(in) = 1
  AC on the same island (low-pass: C1 is out->gnd):
    f=1.0000e0   Hz |H|=0.999980 expected=0.999980 |diff|=2.22e-16
    f=3.9811e2   Hz |H|=0.371214 expected=0.371214 |diff|=0.00e0
    f=1.0000e5   Hz |H|=0.001592 expected=0.001592 |diff|=0.00e0
```

### 2.2 仓库外最小后端实验

工程：`%TEMP%\a02\back`（`Cargo.toml 依赖: cirq-ir 0.5.0 / thevenin 0.5.0 / thevenin-types 0.5.0`，`--offline`；`src/main.rs` 直接构造 `cirq_ir::Circuit` 并调用 `thevenin::circuit::simulate_op`，构造方式与 `_probe/src/bin/robustness.rs` 的 `net()/conn()/resistor()/vsource()` 相同）。
**如何绕开前端**：`simulate_op` 接收裸 `cirq_ir::Circuit`，本实验没有经过 `circuit-dsl` 的词法/语义/`floating_nodes` 检查，直接调用后端入口；所有向量用 `format!("{:.17e}", x)` 打印，避免显示精度掩盖差异。

命令：`cd %TEMP%\a02\back; cargo run --offline`（退出码 0）

真实输出（逐字）：

```text
### scenario 1: open load  v1(a->gnd,1V) r1(a->b,1k) ###
  result = Ok
    plots: 1
    plot 'op1' has 3 vecs
      v(b) = [1.00000000000000000e0]
      v(a) = [1.00000000000000000e0]
      v1#branch = [0.00000000000000000e0]

### scenario 1g: same, option GMIN=1e-3 (linear OP, diag_gmin forced 0?) ###
  result = Ok
      v(b) = [1.00000000000000000e0]
      v(a) = [1.00000000000000000e0]
      v1#branch = [0.00000000000000000e0]

### scenario 2a: isolated resistor network, NO terminal on net 0 (nets 1,2) ###
  result = Err(simulation failed: failed to solve MNA system: matrix is singular, cannot solve)

### scenario 2b: isolated resistor network, nets 0,1, r1 across 0-1 (no declared ground) ###
  result = Err(simulation failed: failed to solve MNA system: matrix is singular, cannot solve)

### scenario 2c: isolated island WITH a voltage source (v1 and r1 in parallel, nets 1,2) ###
  result = Err(simulation failed: failed to solve MNA system: matrix is singular, cannot solve)

### scenario 3: capacitor-coupled node  v1(in->gnd,1V) r1(in->out,1k) c1(out->gnd,1uF) ###
  result = Ok
      v(out) = [1.00000000000000000e0]
      v(in) = [1.00000000000000000e0]
      v1#branch = [0.00000000000000000e0]

### scenario 4: node only through a capacitor  v1(in->gnd,1V) r1(in->gnd,1k) c1(in->out,1uF) ###
  result = Err(simulation failed: failed to solve MNA system: matrix is singular, cannot solve)

### scenario 4g: scenario 4 with GMIN=1e-3 ###
  result = Err(simulation failed: failed to solve MNA system: matrix is singular, cannot solve)

### scenario 5: isolated island made of a capacitor only (c1 across nets 1,2, no source) ###
  result = Err(simulation failed: failed to solve MNA system: matrix is singular, cannot solve)
```

注意向量顺序：OP 输出按矩阵索引**降序**（`v(b)` 在 `v(a)` 前；`v(out)` 在 `v(in)` 前），与 `thevenin-0.5.0/src/simulate.rs:27-38` 的注释一致。

### 2.3 前端（CLI 产品路径）实验

`.cdsl` 文件全部在 `%TEMP%\a02\front\`；CLI 为 `F:\codexprojects\dsl000\target\debug\cdsl.exe`（`cargo build -p circuit-cli --offline` 实测已是最新，编译退出码 0）。

| 文件 | 电路 | `check` 退出码 | `run` 退出码 | 诊断 |
|---|---|---|---|---|
| `scenario1_open_load.cdsl` | `node :a,:b; v1 a→gnd 1V; r1 a→b 1k` | **0** | **0** | 无 |
| `scenario2_isolated.cdsl` | `node :a,:b; r1 a→b 1k`（没有任何端子接 gnd） | **1** | **1** | `E_NAME` × 2（节点 a、b） |
| `scenario3_cap_coupled.cdsl` | `node :in,:out; v1 in→gnd 1V; r1 in→out 1k; c1 out→gnd 1uF` | **0** | **0** | 无 |
| `scenario3b_only_cap.cdsl` | `node :in,:out; v1 in→gnd 1V; r1 in→gnd 1k; c1 in→out 1uF`（out 只经电容连接） | **1** | **1** | `E_NAME` × 1（节点 out） |

scenario 2 的真实 stderr（`cdsl check`，`EXITCODE=1`）：

```text
error[E_NAME]: node `a` has no DC path to ground, so its operating point is undefined
  --> ...\scenario2_isolated.cdsl:2:8
   |
2 |   node :a, :b
  |        ^^
   = a capacitor or current source does not provide a DC reference path; add a resistor, inductor, voltage source or diode to ground
error[E_NAME]: node `b` has no DC path to ground, so its operating point is undefined
  --> ...\scenario2_isolated.cdsl:2:12
   |
2 |   node :a, :b
  |            ^^
   = a capacitor or current source does not provide a DC reference path; add a resistor, inductor, voltage source or diode to ground
```

scenario 3b 的真实 stderr（`EXITCODE=1`）：

```text
error[E_NAME]: node `out` has no DC path to ground, so its operating point is undefined
  --> ...\scenario3b_only_cap.cdsl:2:13
   |
2 |   node :in, :out
  |             ^^^^
   = a capacitor or current source does not provide a DC reference path; add a resistor, inductor, voltage source or diode to ground
   = attached but not conducting at DC: c1
```

产品路径数值（`target/debug/cdsl.exe run ... --out <temp>`，退出码 0）：

```text
scenario1 opcheck.op1.csv:  v(a),v(b),i(v1)  ->  1,1,0
scenario3 opcheck.op1.csv:  v(in),v(out),i(v1) -> 1,1,0
```

（CSV 是低精度显示；高精度值见 §2.2 的后端原始向量。）

### 2.4 相关局部测试（只跑与本审计相关的两个目标）

命令与真实结果：

```text
cargo test --offline -p circuit-core --lib connectivity
  -> test result: ok. 9 passed; 0 failed; 0 ignored
     （含 ground_itself_is_never_reported、a_capacitor_does_not_provide_a_dc_path、
       a_coupled_node_with_a_bias_resistor_is_fine、a_declared_but_unused_node_is_reported_separately 等）
cargo test --offline -p circuit-cli --test e2e
  -> test result: ok. 18 passed; 0 failed; 0 ignored
     （含 check_rejects_a_node_with_no_dc_path_to_ground、check_accepts_an_ac_coupled_stage_with_a_bias_resistor）
```

### 2.5 后端源码证据（thevenin 0.5.0）

| 位置 | 内容 |
|---|---|
| `thevenin-0.5.0/src/circuit.rs:107-114` | `simulate_op`：`assemble(circuit)` → `nr_options_from_circuit` → `simulate::simulate_op_with_mna` |
| `thevenin-0.5.0/src/simulate.rs:14-23` | `simulate_op_with_mna` → 无 nodeset 时走 `solve_op_raw_with_opts` |
| `thevenin-0.5.0/src/simulate.rs:69-72` | 文档注释：`diag_gmin` 对直流工作点**总是被强制为 0**（对齐 ngspice `CKTdiagGmin = 0`） |
| `thevenin-0.5.0/src/simulate.rs:77-83` | 线性电路（`!mna.has_nonlinear()`）直接 `mna.system.solve()` —— 这条路径**不向矩阵对角加 gmin** |
| `thevenin-0.5.0/src/simulate.rs:99-103` | 非线性电路走 NR 时把 `diag_gmin: 0.0` 强制覆盖 |
| `thevenin-0.5.0/src/newton.rs:361-363` | 另一条（非线性 InitJct / NR）路径里确实存在 `system.matrix.add(i, i, attempt.diag_gmin)` 的对角 gmin —— 但直流 OP 路径把它置 0 |
| `thevenin-0.5.0/src/newton.rs:420-533, 957-975` | gmin stepping 是**非线性 NR 失败后的兜底**（direct NR → gmin stepping → source stepping），线性电路根本不进入 |

本次所有实验电路都只含 R / C / 独立电压源（线性），因此走的是 `simulate.rs:77-83` 的直接 LU 路径。据此可以说：**旧文档/注释把 `v(b)=1` 归因于「gmin/漏电把节点拉住」，与源码不符**。至于非线性电路或其他分析类型是否会因 gmin 掩盖浮空节点，本轮**未验证**（见 §9）。

### 2.6 只读核对

```text
git rev-parse HEAD                       -> cb5d8a212f66922181580900a05fb3d42abe32f2（审计开始与结束一致）
git status --porcelain                   ->  M README.md /  M RUST_CIRCUIT_DSL_PROMPT.md / ?? AGENT_TEAM_EXECUTION_PROMPT.md /
                                            ?? agent-team-switch.md / ?? docs/prompt-review.md / ?? docs/review-evidence/
```

`crates/**` 与 `_probe/**` 全程无修改；本次未执行 `cargo test --workspace`（lead 的基线职责，389 passed / exit 0 采信 lead 实测，本报告不据此声称亲验）。

---

## 3. (a) `robustness.rs` 的 float 用例到底是什么

### 3.1 电路（源码逐字）

`_probe/src/bin/robustness.rs:101-149` `check_a_convergence_failure()` 的**第二个子用例**（第一个是双电压源冲突）：

```rust
132:    // A floating node with no DC path to ground.
133:    let mut c2 = base("float", vec![net(0, "gnd"), net(1, "a"), net(2, "b")]);
134:    c2.elements.push(vsource(0, "v1", 1, 0, 1.0, None));
135:    c2.elements.push(resistor(1, "r1", 1, 2, 1_000.0));
136:    c2.analyses.push(Analysis::Op);
137:    match simulate_op(&c2) {
138:        Ok(r) => {
139:            let p = r.plot().expect("plot");
140:            let vb = p.vector("v(b)").map(|v| v.data.as_real()[0]).unwrap_or(f64::NAN);
144:            println!("  floating node v(b) = {vb} (Ok returned, no ground path)");
145:            println!("  => dangling-node detection must be done by OUR frontend\n");
```

拓扑：`net0=gnd`，`net1=a`，`net2=b`；`v1` 的 p=a、n=gnd、dc=1 V；`r1` 的 p=a、n=b、1 kΩ；分析 `Op`。
**没有电容，也没有电流源**。node b 的唯一天然连接是 r1（另一端是 a），而 a 被理想电压源钉在 1 V、其 n 端就是地——即 b 存在经由 r1→a→v1→gnd 的直流通路。

### 3.2 后端真实返回

- `Ok`；plot 名 `op1`；向量 `v(b)=1`、`v(a)=1`、`v1#branch=0`。
- 高精度复跑（§2.2 scenario 1）：`v(b) = 1.00000000000000000e+00`，`v1#branch = 0.00000000000000000e+00`。
- 物理解释（无需后端内部证据即成立）：b 只有一条电阻支路，KCL 给出 `v(b) = v(a) = 1 V`，流过 r1 的电流为 0。这是**唯一确定解**，不是被任何漏电"拉住"。
- 决定性反证：把该电路的 `GMIN` 从默认 1e-12 改为 **1e-3**（与 `1/R = 1e-3 S` 同量级），若真存在 1e-3 S 的对地漏电，`v(b)` 会变成约 0.5；实测 `v(b)` 仍是精确 `1.00000000000000000e+00`（§2.2 scenario 1g）。
- 该二进制整体退出码 **0**（无断言失败路径）。

### 3.3 另一个易混淆的用例：`check_e_isolated_island`

`robustness.rs:225-253` 的电路与 float **不是同一个**：`v1(in→gnd, 1V, ac 1V)`、`r1(in→out, 1k)`、`c1(out→gnd, 1uF)`，先 OP 再 AC。真实输出 `v(out) = 1`、`v(in) = 1`；AC 三点的 `|diff|` 分别为 `2.22e-16 / 0 / 0`（与 `1/√(1+(ωRC)²)` 对照）。
该电路的 `out` 同样**有**直流通路（r1→in→v1→gnd），所以 OP 值 1 V 也是精确解；它并不是"只经电容耦合"的节点。

---

## 4. (b) 前端在哪里、按什么规则、用什么代码拒绝

### 4.1 调用链

1. `crates/circuit-cli/src/main.rs:103-125`：`check` / `run` 分派；`EXIT_OK=0`、`EXIT_USER_ERROR=1`、`EXIT_INTERNAL=2`（`main.rs:24-26`，`121-124` 用 `ExitCode::from` 返回）。
2. `crates/circuit-cli/src/check.rs:25-76` `front_end()`：读文件 → `circuit_dsl::lex/parse/compile`；`compile` 返回 `Err(d)` 时打印诊断并 `return Err(EXIT_USER_ERROR)`（`check.rs:45-51`）——**此时后端 validate/execute 还没有被调用**（`check.rs:55-62` 的 backend 校验与 `run.rs:24` 的 `circuit_session::execute` 都排在其后）。
3. `crates/circuit-dsl/src/elaborate.rs:462` `finish_circuit()`：构造 `Circuit` 后，`484-519` 遍历 `circuit_core::floating_nodes(&circuit)`，为每个命中节点报 `Diagnostic::error(Code::Name, ...)`（`elaborate.rs:504`）；`elaborate.rs:521-523` 只要 `error_count > 0` 就 `return None`，compile 因此失败。
4. `crates/circuit-core/src/connectivity.rs:64-116` `floating_nodes()`：从 `GROUND`（`connectivity.rs:70-74`）出发，只沿 `conducts_dc` 器件洪泛（`75-88`）；不可达的节点按 `FloatingKind::Unused`（无任何器件，`96-103`）或 `FloatingKind::AcCoupledOnly`（有器件但不导通，`104-113`）报出。
5. 规则定义 `connectivity.rs:32-40` `conducts_dc`：Resistor / Inductor / VoltageSource / Diode = 真；**Capacitor / CurrentSource = 假**（注释表 `connectivity.rs:11-24`）。

### 4.2 诊断码与文本

- 代码：`Code::Name` → `as_str() = "E_NAME"`（`crates/circuit-core/src/diagnostic.rs:43-79`、`82-104`）。
- **注意**：`diagnostic.rs` 的 `Code` 枚举里**没有**任何专门的连通性/浮空代码（枚举共 18 个变体：Syntax…Io）；浮空检查复用了 `Name`。
- 文本：`elaborate.rs:487-502`
  - `Unused`：`node `X` is declared but nothing connects to it`
  - `AcCoupledOnly`：`node `X` has no DC path to ground, so its operating point is undefined`
  - 阻断器件附注（仅当 `blocking` 非空）：`attached but not conducting at DC: c1`（`507-517`）
- grep 结果：`floating_nodes` 在整个 `crates/` 中只有**一个生产调用点** `crates/circuit-dsl/src/elaborate.rs:484`（其余为 `connectivity.rs` 单元测试与 `lib.rs:28` 的重导出）。

### 4.3 退出码实测

见 §2.3 表：合法电路 `check/run` = 0；被诊断的电路 `check/run` = **1**；没有任何用例返回 2。

### 4.4 诊断文本的一处准确性问题（低严重性）

对**完全孤立**的电阻网络（scenario 2），节点其实挂着电阻，但报的是 `AcCoupledOnly` 分支 + 建议「add a resistor, inductor, voltage source or diode to ground」（`elaborate.rs:499-501`）。此时 `blocking` 为空（没有不导通器件），附注不打印。文本会让用户以为问题是"电容/电流源"，而真实问题是"整个连通块都不接地"。建议（不改规则）：`AcCoupledOnly` 在 `blocking.is_empty()` 时换一条措辞，例如 "no device in this node's DC-connected block reaches ground"。

---

## 5. (c) 三场景的前端 + 后端对照总表

| 场景 | 电路 | 前端诊断码 / 文本摘要 | CLI 退出码 | 后端原始返回（绕开前端直调 `simulate_op`） |
|---|---|---|---|---|
| **1 合法开路输出** | `gnd - v1(1V) - a - r1(1k) - b(开路)` | 无（接受） | check=0，run=0 | `Ok`；`v(b)=1.0`、`v(a)=1.0`、`v1#branch=0.0`（17 位精确）；GMIN=1e-3 时不变 |
| **2 完全孤立电阻网** | `node a,b; r1 a-b`（无端子接 gnd/0） | `E_NAME` ×2：`node `a`/`b` has no DC path to ground, so its operating point is undefined` | check=1，run=1 | `Err(simulation failed: failed to solve MNA system: matrix is singular, cannot solve)`（三种形态：nets{1,2}；nets{0,1}；含电压源的孤立岛） |
| **3 电容旁路的节点**（任务书给的电路） | `v1(in→gnd) r1(in→out,1k) c1(out→gnd,1uF)` | **无（接受）**——out 经 r1→in→v1 仍有直流通路 | check=0，run=0 | `Ok`；`v(out)=1.0`、`v(in)=1.0`、`v1#branch=0.0` |
| **3b 真正的"只经电容连接"** | `v1(in→gnd) r1(in→gnd,1k) c1(in→out,1uF)` | `E_NAME` ×1：`node `out` has no DC path to ground`；附注 `attached but not conducting at DC: c1` | check=1，run=1 | `Err(... matrix is singular, cannot solve)`（另测：GMIN=1e-3 仍然 Err；孤立电容岛同样 Err） |

**对任务书假设的一处纠正**：任务书把场景 3（`v1 - r1 - out`，`C1` 从 out 到 gnd）当作"仅通过电容耦合的节点"。实测不是：out 经 r1 接到 in，in 被 v1 钉住，**这就是一条直流通路**，前端正确地接受它，后端也给出确定值 1 V。真正会触发前端拒绝的形态是 3b（节点只经电容与网络相连）。

---

## 6. 后端是否"gmin 掩盖浮空"——只记录现象 + 源码证据

**现象**（不改仓库文件的仓库外实验）：
- 真无参考的 4 种线性形态全部 `Err(singular)`，没有出现有限值（§2.2 scenario 2a/2b/2c/4/5）。
- 唯一返回 `Ok` 的"悬空"用例（scenario 1）其实不是浮空节点；其值 1 V 是精确解，且对 `GMIN`（1e-12 → 1e-3）不敏感。

**源码证据**（`thevenin-0.5.0`，版本经三个 lockfile 核对为 0.5.0）：
- 直流 OP 的线性分支走 `mna.system.solve()`（`src/simulate.rs:77-83`），且 `diag_gmin` 被强制 0（`src/simulate.rs:69-72`、`99-103`）——**这条路径不加对角 gmin**。
- 对角 gmin（`system.matrix.add(i, i, attempt.diag_gmin)`）存在于 `src/newton.rs:361-363` 所属的 NR 路径，而 gmin stepping 是 NR 失败后的兜底（`src/newton.rs:420-533, 957-975`），线性电路不进入。

**结论**：旧文档/注释"后端用 gmin/漏电把（这个）节点拉住了"在**本轮实测的线性电路**上不成立，机制与结果两侧都有反证。
**未取得证据的部分**：非线性器件电路、AC/TRAN 分析下是否存在 gmin 掩盖浮空节点的情形——未验证，不下结论。

---

## 7. 旧结论错误定位清单（哪句话错、错在哪、应改成什么）

> 以下行号均为 2026-09-18 19:42 工作区版本（哈希见 §1）。**这些句子目前仍然存在**。

### E1 `docs/backend-evaluation.md:141-142`（严重：高）
- 原文：`实测：`v1 - r1 - b`（b 无对地通路）时 `v(b) = 1` 而**不是**错误。后端用 gmin/漏电把节点拉住了。`
- 错在：(i) "b 无对地通路"——b 经 r1→a→v1→gnd 有直流通路；(ii) "后端用 gmin/漏电把节点拉住了"——线性 OP 不加 gmin（`thevenin-0.5.0/src/simulate.rs:69-104`），且实测 `GMIN=1e-3` 时 `v(b)` 不变。`v(b)=1` 是 KCL 的精确解。
- 应改成：`实测：`v1 - r1 - b`（b 经 r1 与电压源到地，是合法开路输出）时 `v(b) = 1 V`，与解析解一致；该值不受 `GMIN`（1e-12 → 1e-3）影响——线性 OP 不对矩阵对角加 gmin。此例不能作为"后端掩盖浮空"的证据。`

### E2 `docs/backend-evaluation.md:139`（严重：高）
- 原文（表格行）：`| 悬空节点（无直流通路） | **返回 `Ok`，不报错** ⚠️ |`
- 错在：实测 4 种真无参考的线性形态都是 `Err("... matrix is singular, cannot solve")`，不是 `Ok`。
- 应改成：`| 真无参考的线性网络（整个连通块不接地 / 节点只经电容连接） | `Err("... matrix is singular, cannot solve")`（实测 4 种形态）⚠️ 报错但不指向节点 |` 并补一行 `| 合法开路输出（节点经 R/L/V/二极管可达地） | `Ok`，值为确定解（`v(b)=1 V`）✅ |`

### E3 `docs/backend-evaluation.md:144` 与 `:196`（第 6 节第 3 条）（严重：中）
- 原文：`**因此悬空节点检测必须由本项目前端完成**` / `3. **悬空节点后端不报错**，必须由前端做直流参考通路检查。`
- 错在：理由错。后端**会**失败，只是失败信息不定位、且无法区分"合法开路"与"真无参考"。
- 应改成：`后端对真无参考网络会以 singular 失败，但错误不指向任何节点，也不能表达"哪个节点缺直流参考、被哪些器件阻断"；前端因此自己做可达性检查并报出定位到节点的 E_NAME。`

### E4 `crates/circuit-core/src/connectivity.rs:5-7`（模块头，严重：中）
- 原文：`the Phase-0 evaluation found that the engine will not tell us: Thevenin's gmin stepping keeps an unreferenced node finite and returns `Ok` (`docs/backend-evaluation.md` §4.6).`
- 错在：机制错（见 E1）、结论错（见 E2），且引用的 §4.6 本身就是错的来源。
- 应改成：`后端对真无参考的线性网络会报 matrix is singular，但该错误不定位到节点，也无法区分"合法开路输出"与"真正无参考"；因此这里做可达性检查，是为了给出定位到节点与阻断器件的诊断。`（检查逻辑 `64-116` 本身正确，**不要改规则**。）

### E5 `crates/circuit-dsl/src/elaborate.rs:478-483`（严重：中）
- 原文：`// The engine will not report an undetermined node: its gmin stepping keeps a floating node finite and returns success (see docs/backend-evaluation.md §4.6).`
- 错在：同 E1/E2。
- 应改成：`// The engine fails on a truly unreferenced network with a singular-matrix error, but that error names no node and cannot tell a legal open load from a real floating network; the DC-reference check here turns it into a located E_NAME.`

### E6 `crates/circuit-backend/src/thevenin.rs:17-19`（顶部注释第 3 条，严重：中）
- 原文：`3. **Floating nodes are not an error.** The engine's gmin keeps an unreferenced node finite, so the DC-reference check lives in the front end, not here.`
- 错在：同 E1/E2（"gmin keeps an unreferenced node finite" 无实测支持；真无参考线性网络会 Err）。
- 应改成：`3. **An unreferenced network fails with a singular-matrix error, but the error names no node.** The adapter therefore keeps the DC-reference check in the front end, which reports the offending node and the blocking devices. A legal open load (node reachable through DC-conducting devices) returns its exact value.`

### E7 `crates/circuit-cli/tests/e2e.rs:126-128`（测试文档注释，严重：中）
- 原文：`/// The engine will not say so — its gmin stepping keeps the node finite and returns success — so the front end has to catch it.`
- 错在：该用例（节点只经电容连接）实测后端**返回 Err(singular)**，不是 success。
- 应改成：`/// The engine's failure for this topology is an unlocated singular-matrix error, so the front end catches it and reports the node and the blocking capacitor.`
- 注意：断言本身（`status==1`、含 `no DC path to ground`、位置 `floating.cdsl:2`、含 `c1`，`e2e.rs:144-159`）**不依赖这条错误解释**，实测全部通过；修正只需改注释。

### E8 `docs/architecture.md:325`（严重：中）
- 原文：`| 悬空节点**不报错**（gmin 把无直流通路的节点拉住） | ... | 引擎会返回貌似正常的有限值，所以只能在前端发现 |`
- 错在：同 E1/E2。另外该行写 `circuit-core::connectivity` 单元测试 `（8 个）`，实测是 **9 个**（多了 `ground_itself_is_never_reported`）。
- 应改成：`| 真无参考的线性网络：引擎报 `matrix is singular, cannot solve`，但不指向节点、也不区分合法开路 | 由前端做直流参考通路检查并报 `E_NAME` | 单靠后端错误信息无法告诉用户哪个节点缺直流参考 | ...（9 个单元测试）|`

### E9 `docs/language.md:433-434`（严重：中）
- 原文：`Phase-0 实测（见 `docs/backend-evaluation.md` §4.6）表明引擎不会报这种电路，它的 gmin 处理会让节点取到一个看似正常的有限值。`
- 错在：同 E1/E2。
- 应改成：`Phase-0 实测表明：引擎对真无参考的线性网络会以 `matrix is singular, cannot solve` 失败，但该错误不指向任何节点，也无法区分合法开路输出；因此前端自己做直流参考通路检查并给出定位诊断。`（`language.md:425-431` 的规则描述本身正确。）

### E10 `README.md:227`（严重：中）
- 原文：`引擎的 gmin 处理会让这类节点取到看似正常的有限值并照常返回结果（实测见 `docs/backend-evaluation.md` §4.6）`
- 错在：同 E1/E2。
- 应改成：`引擎对真无参考的线性网络会以 singular 失败，但错误不指向节点、也不区分合法开路输出；因此前端在展开结束时做直流参考通路可达性检查并报 `E_NAME`。`
- 备注：README.md 在审计期间正被 lead 修改（`git status` 显示 ` M`），行号以审计时工作区为准。

### E11 `_probe/src/bin/robustness.rs:132 / 144 / 145`（严重：中；A05 的写入范围）
- 原文：`// A floating node with no DC path to ground.` / `println!("  floating node v(b) = {vb} (Ok returned, no ground path)");` / `=> dangling-node detection must be done by OUR frontend`
- 错在：(i) b 有直流通路；(ii) 该例不是浮空反例而是**合法开路正例**；(iii) 第 145 行的结论方向对但理由错（不是"后端不报错"）。
- 应改成：把该子用例重命名/重述为「open load: v(b) = 1 V（确定解，r1 无电流）」，并新增真正的反例（真无参考电阻网 / 只经电容连接的节点，两者实测后端都 Err(singular)）。

### 未发现问题的相关位置（避免误改）
- `docs/testing.md:271`：只陈述"检查已实现 + 两个 e2e 用例"，与实测一致，**不需要改**。
- `docs/prompt-review.md:30-44`：已经正确指出该证据错误（其第 34 行的判断与本次实测一致），**不需要改**。
- `crates/circuit-core/src/connectivity.rs` 的规则与 9 个单元测试、`crates/circuit-cli/tests/e2e.rs` 的两个浮空用例断言：均正确，**不要因为本轮证据修正而删除或放宽**。

---

## 8. 结论

- **(a) PASS**：float 用例 = `gnd-v1(1V)-a-r1(1k)-b` 的合法开路输出；后端 `Ok`，`v(b)=1`（精确），`v1#branch=0`；`robustness` 退出码 0。
- **(b) PASS**：拒绝规则位于 `crates/circuit-core/src/connectivity.rs:32-116`，由 `crates/circuit-dsl/src/elaborate.rs:484-519` 在 `finish_circuit` 阶段调用，诊断码 `Code::Name` = `E_NAME`（没有专门的连通性代码），CLI `check`/`run` 退出码 **1**，且发生在进入后端之前。
- **(c) PASS**：依赖错误浮空解释的位置共 11 处（E1–E11），已逐条给出原文、错因与建议改法。
- **整体：NEEDS_FIX**——证据本身已经取得且可复现，但错误句子在 2026-09-18 19:42 的工作区中仍然存在，需由对应写入者（_probe → A05；docs/backend-evaluation.md、docs/testing.md → A10；README/架构与语言文档/生产区注释 → lead 评估后转移写集）落地修正。
- **BLOCKED：无**。

---

## 9. 未验证项与限制

1. **未跑全量 workspace 测试**。只跑了 `cargo test -p circuit-core --lib connectivity`（9 passed）与 `cargo test -p circuit-cli --test e2e`（18 passed）。389 passed / exit 0 的基线来自 lead 实测，本报告未复验。
2. **未做求解器内部插桩**。我只读源码（`thevenin-0.5.0/src/simulate.rs:69-104`）并观察返回值，没有验证 LU 分解内部行为；"不加对角 gmin"的依据是源码 + `GMIN` 不敏感实验，不是对矩阵元素的直接观测。
3. **非线性电路 / AC / TRAN 下的浮空行为未测**。我的反证只覆盖"只含 R、C、V 的线性电路 + OP"。
4. **产品路径只测 OP**。产品侧的场景 1/3 只跑了 `op`；未测 `tran`/`ac` 下对同类节点的处理。
5. **`docs/architecture.md` 的"8 个单元测试"与实际 9 个的差异**：按当前工作区文件内容统计，未追溯该数字写入时的历史版本。
6. **电气源（受控源等）与电流源参与浮空判定**只做了阅读（`connectivity.rs:32-40` 把电流源列为不导通），没有为"电流源 + 节点"构造后端最小实验。
7. **所有实验使用 `--offline` + 本地 registry 缓存**：`thevenin 0.5.0` 源码读自 `C:\Users\15185\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\thevenin-0.5.0`；未核对 registry 缓存与 crates.io 发布物的校验和（版本号与 `Cargo.lock` 一致，但未做 checksum 级比对）。
8. **本报告的行号/哈希对 `README.md` 可能过期**：该文件在审计期间正被 lead 写入。

## 10. 复现步骤（全部落在仓库外）

```powershell
# 0) 基线版本
cd F:\codexprojects\dsl000; git rev-parse HEAD      # cb5d8a212f66922181580900a05fb3d42abe32f2

# 1) 跑 _probe robustness（只读）
cargo run --manifest-path _probe/Cargo.toml --bin robustness   # 关注 [a] 的 "floating node v(b) = 1" 与 [e]

# 2) 前端三场景（.cdsl 在 %TEMP%\a02\front）
cargo build -p circuit-cli --offline
target\debug\cdsl.exe check %TEMP%\a02\front\scenario1_open_load.cdsl   # exit 0
target\debug\cdsl.exe check %TEMP%\a02\front\scenario2_isolated.cdsl     # exit 1, E_NAME x2
target\debug\cdsl.exe check %TEMP%\a02\front\scenario3_cap_coupled.cdsl  # exit 0（有直流通路）
target\debug\cdsl.exe check %TEMP%\a02\front\scenario3b_only_cap.cdsl    # exit 1, E_NAME x1
target\debug\cdsl.exe run   %TEMP%\a02\front\scenario1_open_load.cdsl --out %TEMP%\a02\front\res1

# 3) 后端原始返回（仓库外 cargo 工程，--offline）
cd %TEMP%\a02\back; cargo run --offline
```

`%TEMP%\a02\back\src\main.rs` 与四个 `.cdsl` 的完整内容属于本次证据产物，位于仓库外；如需入库请由 lead 决定（本代理只写本报告）。