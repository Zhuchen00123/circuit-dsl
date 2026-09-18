# Thevenin 0.5.0 后端数值契约取证（a04-backend-contract，只读）

日期：本轮 wave 1；角色：a04-backend-contract（只读后端接口取证）。
本文件是本轮**唯一**由本代理写入的仓库文件。

## 0. 结论摘要

- **PASS**：Thevenin 0.5.0 的 options / tmax / uic / gmin / AC 相位 / 时间轴 六项契约均已取得**引擎源码行号级证据 + 独立最小工程实测**，无一项依赖上游文档或他人转述。
- **NEEDS_FIX（P1，证据/归因错误）**：
  1. `_probe/src/main.rs` Case 2 把 1.95e-3 归因为“有限边沿 + 采样对齐”是**错的**。真因是引擎把 PULSE 的 tr/tf **下限钳到 tstep**（`waveform.rs:37`），解析解对照的是理想阶跃；相对**实际斜坡输入**的误差只有 ~1e-8 V（实测）。
  2. `docs/backend-evaluation.md` §4.6 “悬空节点 → 返回 Ok，不报错”对**线性电路是错的**：真正的无参考网络返回 `Err(... matrix is singular, cannot solve)`（实测 F1–F8）。返回 Ok 的情形只在**存在非线性器件**时出现，且节点电压由 `options.gmin` 决定（实测 C5）。
  3. `_probe/src/bin/robustness.rs:132-148` 的 `base("float")` 用例（v1–r1–b，b 悬空）是**合法开路输出**（i=0 ⇒ v(b)=v(a)，无需求解 gmin），不能作为“gmin 掩盖浮空”的证据。
- **NEEDS_FIX（P2，文档/注释与实测不符）**：适配层注释 `crates/circuit-backend/src/thevenin.rs:757-761` 说 `step` 是“请求的输出间隔”；在本引擎的 Circuit 路径里 `t_step` **不做输出抽取**，返回的时间轴是**每一个被接受的内部步**。能力说明 `thevenin.rs:104-112` 是对的。
- 产品路径现状（已核实）：`build_circuit` 写死 `options: Vec::new()`（`thevenin.rs:436`，SHA256 81D4D6E5…），DSL 写死 `uic: false`（`elaborate.rs:2458`，SHA256 426FECC6…）⇒ **RELTOL/ABSTOL/VNTOL/GMIN/CHGTOL/TRTOL/METHOD 全部恒为引擎默认**，本轮没有任何产品级容差通道。

## 1. 审查范围与版本（哈希 / git rev）

- 仓库：F:/codexprojects/dsl000，`git rev-parse HEAD` = **cb5d8a212f66922181580900a05fb3d42abe32f2**（branch main）。
- 取证期间工作树**已被其他代理修改**（`git status --porcelain` 实测）：
  `M README.md / M RUST_CIRCUIT_DSL_PROMPT.md / M crates/circuit-backend/src/thevenin.rs / M crates/circuit-cli/tests/e2e.rs / M crates/circuit-core/src/connectivity.rs / M crates/circuit-dsl/src/elaborate.rs / M docs/architecture.md / M docs/language.md / ?? docs/review-evidence/`
  ⇒ 本报告对**仓库文件**的行号均按取证时刻内容给出，并附 SHA256 前 16 位；引擎（crates.io 缓存）文件不可变，行号稳定。
- `cargo test --workspace` 基线 389 passed / 0 failed / exit 0 由 Lead 实测记录于 `docs/review-evidence/baseline.md`；本代理**未**重跑（避免与其他代理并发改动互相干扰，见第 5 节限制）。

| 文件 | SHA256 前 16 位 | 取证时刻 mtime |
|---|---|---|
| crates/circuit-backend/src/thevenin.rs | 81D4D6E573EE9F45 | 19:45:22 |
| crates/circuit-core/src/plan.rs | E562162A41A6B0F4 | 15:38:45 |
| crates/circuit-backend/tests/adapter.rs | 6AE48B621274BA82 | 15:38:45 |
| crates/circuit-core/src/connectivity.rs | 035C87B6492EC57C | (本轮已改动) |
| crates/circuit-dsl/src/elaborate.rs | 426FECC6A33C1EC8 | (本轮已改动) |
| _probe/src/main.rs | 81EAD7F09867E3E5 | 14:15 |
| Cargo.lock | 9D4AC65AFBA02DD6 | — |

引擎源码根：`C:/Users/15185/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`，版本由 `Cargo.lock` 证实：`name = "thevenin" / version = "0.5.0" / source = registry+https://github.com/rust-lang/crates.io-index`。

## 2. 实际执行/阅读了什么（只读）

命令 1（仓库内，只读运行）：`cargo run --manifest-path _probe/Cargo.toml --bin probe`
结果：`EXIT=0`，末行 `RESULT: ALL ACCEPTANCE CASES PASSED`。Case 2 关键输出（原文照抄）：
 ``
   plots: op1[3 vectors], tran1[4 vectors]; using 'tran1'
   1015 time points, t in [0.000e0, 5.000e-4] s
   t=  25.232us (0.25tau)  v(out)=0.221061  analytic=0.223007  |diff|=1.95e-3
   t=  50.232us (0.50tau)  v(out)=0.393362  analytic=0.394877  |diff|=1.51e-3
   t= 100.232us (1.00tau)  v(out)=0.632056  analytic=0.632974  |diff|=9.18e-4
   t= 200.232us (2.00tau)  v(out)=0.864641  analytic=0.864979  |diff|=3.38e-4
   t= 300.232us (3.00tau)  v(out)=0.950204  analytic=0.950328  |diff|=1.24e-4
   worst |diff| = 1.946e-3
 ``
其它 5 个用例（OP 分压器、RC AC、RLC AC、二极管 OP、DC 扫描）全部 PASS：DC 扫描 6 点、RC AC 61 点、RLC AC 81 点、worst AC |diff| 2.3e-15。

命令 2–4（**仓库外**最小 cargo 工程 `C:/Users/15185/AppData/Local/Temp/a04/`，依赖 cirq-ir/thevenin/thevenin-types 0.5.0，`cargo run --offline`，三个 bin：`a04probe` / `confirm` / `liveness`，全部 `EXIT=0`）：
- 直接调用产品适配层使用的**同一批入口**：`thevenin::circuit::{simulate_op, simulate_tran, simulate_ac}`（适配层调用点 `thevenin.rs:361-366`）。
- 电路完全由 `cirq_ir::Circuit` Rust 值构造，无 SPICE 文本。

阅读的源码：`crates/circuit-backend/src/thevenin.rs` 全文（改动前后各读一遍复核行号）、`crates/circuit-core/src/plan.rs` 全文、`crates/circuit-backend/tests/adapter.rs:240-390`、`crates/circuit-dsl/src/elaborate.rs:2385-2465`、`crates/circuit-core/src/connectivity.rs:1-60`、`docs/backend-evaluation.md`（§4）、`_probe/src/main.rs`（Case 2）、`_probe/src/bin/robustness.rs`（float 用例）。
引擎源码：`thevenin-0.5.0/src/{circuit.rs, mna_ir.rs, transient.rs, newton.rs, simulate.rs, waveform.rs, ac.rs}` 相关段、`cirq-ir-0.5.0/src/lib.rs`（AcSpec/Waveform/TranAnalysis/AcAnalysis/Circuit）、`thevenin-types-0.5.0/src/lib.rs`（SimPlot/SimVector/VectorData）。

## 3. 契约项逐条（契约项 → 源码证据 → 实测 → 对适配层/测试的含义）

### A. `Circuit.options: Vec<(String, Value)>` 是否真的到达求解器

**源码证据（唯一解析点）**：`thevenin-0.5.0/src/mna_ir.rs:107-133`
 ``
 pub fn nr_options_from_circuit(circuit: &Circuit) -> NrOptions {
     let mut opts = NrOptions::default();
     for (name, value) in &circuit.options {
         let v = match value { Value::Real(f) => *f, Value::Integer(i) => *i as f64, Value::Bool(b) => {...}, _ => continue };
         match name.to_uppercase().as_str() {
             "GMIN" => opts.gmin = v, "ABSTOL" => opts.abstol = v,
             "RELTOL" => opts.reltol = v, "VNTOL" => opts.vntol = v, ... } }
 ``
（键名 `to_uppercase()` ⇒ 小写 `reltol` 也生效；`Value::String` 被 `_ => continue` 静默忽略。）

**空 options 时求解器用的默认容差（明确回答）**：`thevenin-0.5.0/src/newton.rs:166-220` 的 `impl Default for NrOptions`：
 `abstol=1e-12, reltol=1e-3, vntol=1e-6, itl1=100, itl2=200, itl4=10, itl5=0, gmin=1e-12, diag_gmin=1e-12, chgtol=1e-14, rshunt=0.0, gshunt=0.0, gminsteps=10, trtol=7.0, pivtol=1e-13, pivrel=1e-3`；积分方法默认 `Trapezoidal`（`mna_ir.rs:646-655`）。

**reltol/abstol 是否真的改变积分步长控制（明确回答）**：**是，通道存在且实测可触发**——
- 传递链：`circuit.options` → `nr_options_from_circuit`（mna_ir.rs:107）→ `tran_params_from_circuit`（mna_ir.rs:632）→ `TranRunParams.nr_opts`（transient.rs:662-672）→ `run_tran` 绑定 `circuit_nr_opts`（transient.rs:783）→ `let nr_options = circuit_nr_opts;`（transient.rs:1326）→ LTE 步长估计 `estimate_new_timestep(... nr_options.reltol, .abstol, .chgtol, .trtol)`（transient.rs:1621-1634、1664-1677）。
- LTE 公式（`transient.rs:409-432`）：`vol_tol = abstol + reltol*max(|i_cur|,|i_prev|)`；`chg_tol = reltol*max(|q0|,|q1|,chgtol)/h`；`tol = trtol*max(vol_tol,chg_tol)`；`h_new = sqrt(tol / max(abstol, lte_est))`，`lte_est = (1/12)*|Q 二阶差商|`。
- 实测（`liveness`，RC τ=100 µs，PWL 1 ns 斜坡，`step=5τ, tmax=None ⇒ h_max = min(tstep, tstop/50) = 1e-5 s`）：
  reltol=1e-2/1e-3/1e-6/1e-7/1e-8 → n=62（步长被 h_max 顶住）；reltol=1e-9 → n=64；reltol=1e-12 → n=73；
  reltol=1e-12 + abstol=1e-15 → **n=266，dt_max 1e-5→5.97e-6**；reltol=0 + abstol=1e-15 → **n=1428，dt_max=1.548e-6**，v(1τ) 误差 2.803e-4→8.8e-8；trtol=0.007/7000 → n 不变。
  ⇒ **options 的容差确实进入 LTE 步长控制**。
- **但在产品 Case-2 配置下不可见**：`step=tmax=τ/200` 时把 reltol 从 1e-3 改到 1e-6、abstol 1e-3↔1e-12，返回时间轴完全不变（n=1015、dt_min=1.25e-10、dt_max=5e-7）；E1（空 options）与 E3（显式 reltol=1e-3,abstol=1e-12）逐点 `max|dv| = 0.000e0`。因为步长被 `h_max=tmax` 钉死，LTE 从不 binding。

**对适配层/测试的含义**：
- 想调容差，只需把 `CqCircuit.options` 填上即可生效（无需改引擎）；当前 `thevenin.rs:436` 恒空 ⇒ 产品永远跑默认容差。
- **线性**电路（R/L/C/V/I，本项目除二极管外全部器件）`mna.has_nonlinear()==false`：OP 与瞬态均走**直接线性求解**，不进 NR 收敛判据（`simulate.rs:77-97`、`transient.rs:3515-3527`）⇒ 对纯线性电路改 reltol/abstol/vntol 只能通过 LTE 生效，改不出“更严格的 NR 收敛”。
- 任何“调紧容差所以更准”的断言，必须同时给出“步长是否被 tmax/tstep 顶住”的证据，否则是假阳性（3.A 实测反例：容差变 1e9 倍，输出一位不动）。

### B. tmax / step / stop / start 的真实用途（输出打印网格 vs 内部步长）

**源码证据**：
- `transient.rs:775-799`：`if h_print <= 0.0 || t_stop <= 0.0 { Err }`；`let h_max = t_max.unwrap_or_else(|| h_print.min(t_stop / 50.0));` ⇒ **tmax 是内部步长上限；缺省时 h_max = min(tstep, tstop/50)**。
- `transient.rs:1407-1408`：`let h_min = h_print * 1e-9; let mut h = (h_max / 400.0).max(h_min);` ⇒ 初值 h_max/400，之后每步最多 ×2（`MAX_GROW=2.0`，`transient.rs:369`）。
- `transient.rs:1690-1694`（无 LTE 的兜底分支）：`h = (step_h*MAX_GROW).min(h_max).min(h_print).max(h_min);` ⇒ 只有该分支里 tstep 才直接限制步长。
- `transient.rs:305-314`：`min_break = tran.tstep * 5e-5`（断点去重窗口）。
- `transient.rs:2271-2285`：**每接受一个内部步就 `record_point`**；`record_point`（2386-2420）只做 `time_vec.push(t)`。**没有任何按 tstep 抽取输出的代码**（grep `h_print` 全部 7 处：777/792/799/1328/1407/1694/2307，无一处抽样）。

**实测**：
- Case 2 配置 n=1015（≙ 5e-4/5e-7 + 起步细步），dt_min=1.25e-10=h_max/400，dt_max=5e-7=tmax。
- `tmax` 扫描：τ/20→117 点，τ/200→1015，τ/2000→10015，τ/20000→100015（dt_max 与 tmax 一一对应）。
- `step` 扫描（tmax 固定 τ/200）：τ/20→1015，τ/200→1015，τ/2000→**1017**（仅因 h_min、min_break、脉冲突沿位置变化，输出仍不抽样）。
- `tmax=None`、step=τ/200：n=1015、dt_max=5e-7=min(step, stop/50)。

**对适配层/测试的含义**：
- 适配层把 `TranSpec.output_interval` 映射到引擎 `step`、把 `max_step` 映射到 `tmax`（`thevenin.rs:757-774`）在“边界语义”上是对的（`max_step` 不承诺输出网格），但 `output_interval` 在引擎里**几乎是惰性的**：仅当 `tmax=None` 时通过 h_max 影响步长，`tmax=Some` 时只剩 h_min/min_break 的次级效应。
- 真正决定精度的是 `tmax`（内部步长）与 `step`（脉冲突沿下限，见 G）；结果点数由求解器决定，不能拿点数断言“输出间隔”。
- DSL 侧 `elaborate.rs:2395` 允许 `output_interval`、`max_step`、`start`、`stop`，写入 `TranSpec`（`plan.rs:154-165`；`plan.rs:150-152` 已正确说明 max_step 不是输出间隔）。

### C. uic 语义与默认初值来源（是否先跑 OP 作为初值）

**源码证据**：`transient.rs:808-831`
 ``
 let mut solution = if let Some(state) = &start_state { state.solution.clone() }
     else if uic { vec![0.0; dim] }
     else { let mut sol = if nodeset.is_empty() { solve_op_raw_with_opts(&mna, &circuit_nr_opts)? }
            else { crate::simulate::solve_op_raw_with_nodeset(&mna, &circuit_nr_opts, &nodeset)? };
            sol.resize(dim, 0.0); sol };
 if start_state.is_none() { /* .ic 覆盖: 节点电压、电容 IC、电感 IC */ }
 ``
⇒ **uic=false 时先算 DC OP 作为初值；uic=true 时跳过 OP，初值 = 全 0 向量**，随后叠加 `.ic` / 器件 IC（`transient.rs:833-858`）。
另：`thevenin::circuit::simulate_tran`（`circuit.rs:134-150`）无论 uic 与否都会**额外**算一遍 OP 作为 `op1` plot 放进结果（该 OP 不进 `run_tran` 的初值路径）——这正是适配层必须按名字选 plot 的原因（`thevenin.rs:13-14, 370`）。

**实测**：
- `uic=true` 与 `uic=false` 在 Case-2 RC 上输出**逐点相同**（n=1015、worst |diff|=1.945643e-3；因为 0 初值与 OP 初值都是 0）。
- 决断性实测：仅电容接地的悬空节点电路 `uic=true` 成功（n=209，v(a)≡0），`uic=false` 返回 `Err(matrix is singular)` ⇒ **uic=true 确实跳过 OP 求解**。

**对适配层/测试的含义**：`plan.rs:161-163` 已注明 uic “语言未暴露、后端路径未验证”；`elaborate.rs:2458` 写死 `uic: false` ⇒ **产品路径无法触发 uic=true**（引擎能力存在）。若将来暴露，测试应断言“初值 = 0 而非 OP”，而不是只比对波形。

### D. 浮空/无参考节点与 gmin 是否加在对角线

**gmin 加在对角线——有源码证据**：`thevenin-0.5.0/src/newton.rs:361-364`
 ``
 // Add diagonal Gmin from each node to ground for numerical stability.
 for i in 0..num_nodes {
     system.matrix.add(i, i, attempt.diag_gmin);
 }
 ``
取值来源分三条路径：
- 非线性 OP：首轮 `diag_gmin = options.diag_gmin`，但 `simulate.rs:99-102` 在 OP 里强制 `diag_gmin = 0.0`；失败后 gmin stepping 从 1e-2 递降到 `gmin_target = options.gmin`，最后以 `diag_gmin = gmin_target` 做终解（`newton.rs:431-524`）。
- 非线性瞬态步：`diag_gmin = options.gmin`（`newton.rs:844-849`，注释：use options.gmin as a minimal diagonal shunt）。
- **线性电路（产品路径常态）根本不走 NR**：OP 直接 `mna.system.solve()`（`simulate.rs:77-97`），瞬态单次线性求解（`transient.rs:3515-3527`），且瞬态 load 闭包的 `gmin` 形参在 `transient.rs:2606` 被 `let _ = gmin;` 显式丢弃 ⇒ **线性电路没有任何 gmin 补在对角线上**。

**实测（本轮要修正的第二类错误证据）**：

| 用例 | 电路 | options | 结果 |
|---|---|---|---|
| F1 | 岛 a–b 仅由 1 kΩ 相连 + 1 pA 注入 a（线性） | 空 | `Err: matrix is singular, cannot solve` |
| F2/F3/F4 | 同上 | GMIN=1e-3 / GMIN=1.0 / RSHUNT=1k | **仍然 Err**（线性 OP 不读 options） |
| F5/F6 | 同上 `simulate_tran(uic=true)` | 空 / GMIN=1e-3 | **Err**（线性瞬态丢弃 gmin） |
| F7 | 同上 `simulate_tran(uic=false)` | 空 | Err（先算 OP） |
| F8 | 仅一个电容接地的节点（无直流通路） | 空 | `Err`（DC 无对角元） |
| F9/F10 | 同上，瞬态 | 空，uic=true / false | uic=true **Ok**（电容伴随电导提供通路），uic=false **Err** |
| C5 | **同一岛 + 一个二极管**（触发 NR） | 默认 | **Ok: v(a)=5.000000e-1 = I/(2·gmin_default)，ratio 1.0000** |
| C5 | 同上 | GMIN=1e-6 | Ok: v(a)=5.002499e-7（解析 5e-7，ratio 1.0005） |
| C5 | 同上 | GMIN=1e-3 | Ok: v(a)=6.666667e-10（含 1 kΩ 并联修正 (1+gR)/(1+gR/2)=1.3333，实测 ratio 1.3333） |

**结论**：
1. “引擎的 gmin 让无参考节点保持有限并返回 Ok”（`thevenin.rs` 旧注释、`connectivity.rs` 旧注释、`docs/backend-evaluation.md` §4.6）**只对含非线性器件的电路成立**；线性电路是 `Err(matrix is singular)`，且该 Err **不点名任何节点**。
2. 前端（`circuit-core::connectivity::floating_nodes`，`connectivity.rs:36-44` 的 `conducts_dc` 洪泛）仍然必需，但理由应写成“引擎错误不定位 + 非线性路径会用 gmin 给出一个看似合理的值”。
3. `_probe/src/bin/robustness.rs:132-148` 的 `v1–r1–b`（b 只接电阻一端）**不是浮空网络**：i=0 ⇒ v(b)=v(a)=1 V 由电路方程唯一确定，不需要 gmin；把它当“gmin 掩盖”的证据不成立。

### E. AC 相位与 SIN 的 phi：单位、转换式、符号约定、实虚部构造

**`AcSpec.phase` 是度**：`cirq-ir-0.5.0/src/lib.rs:348-354`：`pub struct AcSpec { pub mag: f64, /// Phase in degrees. Defaults to 0.0 when not specified. pub phase: f64 }`。
精确转换与实虚部构造（`thevenin-0.5.0/src/mna_ir.rs:496-498`，用于 `collect_ac_excitations_from_circuit`）：
 ``
 let phase_rad = ac.phase * std::f64::consts::PI / 180.0;
 let real = ac.mag * phase_rad.cos();
 let imag = ac.mag * phase_rad.sin();
 ``
⇒ 度→弧度 = `phase·π/180`，激励复数 = `mag·(cos φ + j sin φ)`（**正相位 ⇒ 正虚部**，SPICE 的 `∠φ = e^{+jφ}` 约定）。

**实测（E9，RC 低通，`v1 ac mag=1`，10 Hz–1 MHz，10 pts/decade ⇒ 51 点）**：
- φ=0°：`v(in)` = (1.000000000e0, 0.000000000e0)；φ=90°：(6.123233996e-17, 1.000000000e0)；φ=−90°：(6.123233996e-17, −1.000000000e0)；φ=45°：(0.707106781, 0.707106781)；四种相位下 `max|H_num − 1/(1+jωRC)| = 4.4e-16` ⇒ 相位只整体旋转激励，不改变传函。

**SIN 的 `phi` 也是度**：`waveform.rs:158-176`
 ``
 /// Before td: v0 + va * sin(phi)   After td: v0 + va * sin(2*pi*freq*(t-td) + phi) * exp(-theta*(t-td))
 fn eval_sin(v0: f64, va: f64, freq: f64, td: f64, theta: f64, phi_deg: f64, t: f64) {
     let phi_rad = phi_deg * PI / 180.0;
     if t <= td { v0 + va * phi_rad.sin() } else { ... (2.0*PI*freq*dt + phi_rad).sin() * damping ... } }
 ``
实测（E10，电阻分压 + `SIN(v0=0,va=1,f=1 kHz,td=0,theta=0,phi=90)`，1009 点）：t=0 采样 = **5.000000000e-1**；`max|v − 0.5·sin(ωt+φ度→弧度)| = 5.55e-17`；反假设（phi 当弧度）`max|v − 0.5·sin(ωt+90 rad)| = 2.302e-1` ⇒ **phi 确定是度**。

**PULSE 的 tr/tf 有 `max(tr_user, tstep)` 下限**（`waveform.rs:25-44` 求值、`waveform.rs:271-272` 断点）：`tr.unwrap_or(tran.tstep).max(tran.tstep)`。这条是 Case-2 误差的真正来源，见 G。

**对适配层/测试的含义**：适配层 `thevenin.rs:653-659`（`phase: a.phase_rad.to_degrees()`）与 `thevenin.rs:687-694`（`phi: Some(phase_rad.to_degrees())`）与引擎约定一致；**非零相位的产品路径**回归测试不属本代理范围（见第 5 节）。

### F. 瞬态返回时间轴：是否严格递增？是否可能重复？

**源码证据**：
- 每个被接受的步把时间**累加**一个正步长：`transient.rs:1698-1701`（`h_prev = step_h; t += step_h;`）；
- 步长下界 `h_min = h_print*1e-9 > 0`（`transient.rs:1407`），各分支都 `.max(h_min)`；循环条件 `while t < t_stop - h_min`（`transient.rs:1422`）；
- 输出：`if t >= t_start { record_point(t, ...) }`（`transient.rs:2271-2285`），`record_point` 只 append（`transient.rs:2400`）；t=0 初值点仅在 `t_start <= 0` 时记录（`transient.rs:1336-1337`）。
- 被拒绝的步不推进时间、不记录（`transient.rs:1591-1611、1636-1651`）。
⇒ **时间轴严格递增、不会出现重复点**。

**实测（E11，4 种配置）**：Case-2 配置 / tmax=τ/2000 / uic=true / tmax=None 全部 `non_increasing=0, duplicates=0`，dt_min>0。

**对适配层/测试的含义**：`adapter.rs:353-362` 的“非均匀”断言成立（实测步长从 1.25e-10 单调放大到 5e-7）；measure 的 avg/rms 必须做积分近似而不是等间隔求和（`plan.rs:229-233` 已声明 needs_time_axis）。返回轴只含 `[t_start, stop]` 内的点。

### G. RC 瞬态误差归因（修正 `_probe` Case 2 的 1.95e-3）

**真因（源码）**：`thevenin-0.5.0/src/waveform.rs:33-44` 把 PULSE 的上/下降沿钳到 `tran.tstep`：`tr.unwrap_or(tran.tstep).max(tran.tstep)`。Case 2 的 `tstep = tau/200 = 5e-7 s`（用户的 1 ps 被忽略），而解析解对照的是**理想阶跃** `1−e^{−t/τ}`。
对幅度 1 V、上升时间 `tr` 的斜坡，RC 精确解：`t≤tr: v = t/tr − (τ/tr)(1−e^{−t/τ})`；`t>tr: v = 1 − (τ/tr)(1−e^{−tr/τ})·e^{−(t−tr)/τ}`；一阶展开 `v ≈ 1 − (1 + tr_eff/(2τ))·e^{−t/τ}`，即与理想阶跃的偏差 `A·e^{−t/τ}, A ≈ tr_eff/(2τ)`。

**实测 C1（固定 `tmax=τ/2000`，只变 `step` ⇒ 只变 tr_eff）**：

| step | tr_eff=step | A 实测（5 采样点 min~max） | step/(2τ) | worst vs 理想阶跃 | worst vs 精确斜坡解 |
|---|---|---|---|---|---|
| τ/20 | 5e-6 | 2.5422e-2 | 2.5e-2 | 1.980e-2 | 1.376e-8 |
| τ/200 | 5e-7 | 2.5041e-3~2.5042e-3 | 2.5e-3 | 1.951e-3 | 1.174e-8 |
| τ/2000 | 5e-8 | 2.4998e-4~2.5004e-4 | 2.5e-4 | 1.947e-4 | 6.187e-9 |
| τ/20000 | 5e-9 | 2.4938e-5~2.4995e-5 | 2.5e-5 | 1.947e-5 | 7.709e-9 |

实测 C2（固定 `step=τ/1000`，变用户 tr）：tr=1e-12 与 1e-7（都 < step）→ A=5.001e-4=step/(2τ)；tr=2e-6 → A=1.0067e-2≈tr/(2τ)；tr=2e-5 → A=1.0701e-1≈tr/(2τ)；三种情况相对**精确斜坡解**的误差都 ~1e-8 V。
另实测（E5）：固定 tstep=τ/200、把 tmax 从 τ/20 扫到 τ/20000，worst |diff| 始终 1.92e-3~1.95e-3、A 基本不变 ⇒ 该误差**与内部步长/tmax 无关**。
⇒ 1.95e-3 **不是积分误差、不是采样对齐误差**，而是“解析参考用了理想阶跃、引擎输入是 tr=tstep 的斜坡”的定义性差异；它与 tstep（`output_interval`）线性相关。

**对适配层/测试的含义**：
- `adapter.rs:253-366` 的 `rc_transient_matches_analytic`（`max_step=output_interval=τ/200`，阈值 1e-2）目前以 1.95e-3 通过，余量仅 ~5×。若有人把 `output_interval` 放宽到 τ/20（DSL 允许），误差会线性放大到 ~1.98e-2 并**直接失败**，而失败原因与“求解器不准”无关。
- 建议：对照改成“引擎 vs 同 tr_eff 的精确斜坡解”（实测可达 1e-8），或显式断言 `误差 ≈ (step/2τ)·e^{−t/τ}`，并在注释写明 tr 下限来自 `waveform.rs:37`。
- `_probe/src/main.rs` 的用法改法：只把 PULSE 的 `tr` 设小**不够**（会被钳到 tstep）；要么用 `Pwl` 斜坡（无下限，实测可用），要么把 `step` 设小，要么改对照公式。

## 4. 发现清单（严重性 / 位置）

| # | 严重性 | 位置 | 问题 | 证据 |
|---|---|---|---|---|
| 1 | P1 | `_probe/src/main.rs` case2（`docs/review-evidence/baseline.md` §5 已记录） | 1.95e-3 归因错误（“有限边沿与采样对齐”）；真因是 `waveform.rs:37` 的 tr 下限，且引擎相对精确斜坡解只有 ~1e-8 V | 本报告 3.G |
| 2 | P1 | `docs/backend-evaluation.md` §4.6（:131-146，本轮未被修改） | “悬空节点 → 返回 Ok，不报错”对线性电路错误 | 3.D 表 F1–F10 |
| 3 | P1 | `_probe/src/bin/robustness.rs:132-148` | `base("float")` 是合法开路负载，不能当 gmin 掩盖证据 | 3.D 结论 3 |
| 4 | P2 | `crates/circuit-backend/src/thevenin.rs:757-761` | 注释“`step` 是 print step，即请求的输出间隔”在本引擎路径下误导：`t_step` 不抽取输出 | 3.B（`transient.rs:2271-2285`，无抽样代码） |
| 5 | P2 | `docs/backend-evaluation.md` §4.5（:121-129） | 只写“options 可传 RELTOL/ABSTOL/GMIN”，未写适配层 `thevenin.rs:436` 恒空 ⇒ 产品无容差通道、恒为默认 | 3.A |
| 6 | P3 | `thevenin.rs:104-112` 能力说明 | 内容正确，但没有记录 `PULSE tr 下限 = tstep` 这条精度契约 | 3.G |
| 7 | P3 | `crates/circuit-core/src/plan.rs:161-163` | uic 标注“后端路径未验证”：本轮已给出源码 + F9/F10 实测（uic=true 跳过 OP），可从“未验证”升级为已取证 | 3.C |

说明：第 1、3 条所在文件的**上游文档注释**已被其他代理在 19:45 前后同步修正（`thevenin.rs:17-24`、`connectivity.rs:5-11`、`elaborate.rs:479-490` 现已改成与本节一致的说法）；`docs/backend-evaluation.md` §4.6 与 `_probe` 两处代码仍待改。

## 5. 未验证项与限制

1. **非零相位的产品路径**（DSL `ac phase:` → `AcSweep` → 适配层 `to_degrees` → 引擎）**未验证**：本轮只在引擎层验证了 `AcSpec.phase` 的度/符号约定（3.E），未跑 CLI/DSL 端到端相位用例。
2. **产品路径的浮空节点行为**（前端 `floating_nodes` 诊断如何呈现引擎 `Err(matrix is singular)`，以及含二极管电路上前端是否漏判）**未验证**；本轮验证的是引擎层。
3. **容差经过产品路径**：`thevenin.rs:436` 写死空 `options`，“填 reltol 会改变产品输出”只能由引擎层实验外推（同一入口 `simulate_tran`、同一解析点 `nr_options_from_circuit`），**没有**产品端到端证据。
4. **`cargo test --workspace` 未由本代理重跑**：取证期间其他代理正在并发修改 `thevenin.rs / connectivity.rs / elaborate.rs / e2e.rs`，重跑不能代表任何一方最终状态；最终回归应由 Lead 在冻结后执行。
5. **被拒绝的步（rejected steps）不可观测**：`record_point` 只在接受时记录，返回时间轴看不到 `transient.rs:1591-1611、1636-1651` 的重试过程。
6. **次要选项未实测**：`ITL5 / SRCSTEPS / PIVTOL / PIVREL / ITERATIVE_REFINEMENT / RSHUNT / GSHUNT` 只在源码层读到（`mna_ir.rs:134-170`、`newton.rs:365-381`）；其中 `PIVTOL/PIVREL` 被接受但按 no-op 处理并打 warning（`mna_ir.rs:146-161`），未做数值验证。
7. **未验证** `uic=true + 非零 .ic` 的产品语义（DSL 既不暴露 `uic` 也不暴露 `.ic`）。
8. 引擎源码取自本机 registry 缓存（crates.io，0.5.0），**未**与远端发布包做校验和比对；`Cargo.lock` 的 version/source 与实际阅读目录一致。
9. 引擎为巨大 crate，本轮只精读了列出的 7 个文件；未覆盖 BSIM/VBIC/MESFET 等器件的 LTE 分支（对产品当前器件集不相关）。

## 6. 产品路径可依赖契约（速查）

| 项 | 产品是否可控 | 引擎语义 | 关键行号 |
|---|---|---|---|
| tran stop/start | 是（DSL） | 时间区间；只有 `t >= t_start` 才记录输出 | `elaborate.rs:2396-2423` / `transient.rs:2271-2272` |
| tran max_step | 是（DSL `max_step`） | = 引擎 `tmax`，内部步长上限；None ⇒ min(tstep, tstop/50) | `thevenin.rs:772` / `transient.rs:799` |
| tran output_interval | 是（DSL） | = 引擎 `tstep`：h_max 下限、h_min=1e-9·tstep、min_break=5e-5·tstep、**PULSE tr/tf 下限**；不抽取输出 | `thevenin.rs:763-768` / `transient.rs:799,1407,308` / `waveform.rs:37` |
| uic | **否**（`elaborate.rs:2458` 硬编码 false） | true 跳过 OP、初值全 0；false 先算 OP | `transient.rs:818-831` |
| RELTOL/ABSTOL/VNTOL/GMIN/CHGTOL/TRTOL/METHOD | **否**（`options` 恒空） | 解析入 NrOptions；驱动 LTE 步长与 NR 收敛判据；线性电路不进 NR | `thevenin.rs:436` / `mna_ir.rs:107-133` / `newton.rs:166-220,246-256` / `transient.rs:1621-1634` |
| AC 相位 | 是（DSL；本轮未端到端验证） | 度；real=mag·cos φ，imag=mag·sin φ | `thevenin.rs:658` / `mna_ir.rs:496-498` |
| SIN phi | 是（DSL；本轮未端到端验证） | 度；v=v0+va·sin(2πf(t−td)+φ)·e^{−θ(t−td)} | `thevenin.rs:694` / `waveform.rs:162-176` |
| 输出时间轴 | 由求解器决定 | 每个被接受的内部步一个点；严格递增、无重复 | `transient.rs:2271-2285, 2386-2420, 1700` |
| 结果 plot 选择 | 适配层自担 | `simulate_tran` 返回 [op1, tran1]；`ac`/`dc` 各自一 plot | `circuit.rs:134-150` / `thevenin.rs:370, 826` |

## 7. 结论

- **契约取证本身：PASS**（3.A–3.G 七项均有“引擎源码 文件:行 + 代码片段 + 本机实测数字”）。
- **对本轮既有证据的裁定：NEEDS_FIX**——P1 三条（`_probe` Case 2 归因、`docs/backend-evaluation.md` §4.6、`_probe` robustness float 用例）须按 3.G / 3.D 改写；相关 Rust 文档注释已被其他代理部分修正，`backend-evaluation.md` 与 `_probe` 代码仍待改。
- **BLOCKED：无**（不依赖任何不可访问的资源；未以“上游文档说支持”或“别的代理说通过”作为结论依据）。

## 8. 复现方式

仓库内（只读）：
 ``
 cargo run --manifest-path _probe/Cargo.toml --bin probe      # EXIT=0；Case 2 worst |diff|=1.946e-3
 ``
仓库外（本报告全部新实测；不写入仓库、不影响工作区）：
 ``
 C:/Users/15185/AppData/Local/Temp/a04/Cargo.toml            # cirq-ir 0.5.0 + thevenin 0.5.0 + thevenin-types 0.5.0；含空 [workspace]
 C:/Users/15185/AppData/Local/Temp/a04/src/main.rs           # E1-E11：options/tmax/step/uic/gmin/AC 相位/SIN phi/时间轴
 C:/Users/15185/AppData/Local/Temp/a04/src/bin/confirm.rs    # C1-C5：tr 下限归因、LTE/h_max、非线性 gmin
 C:/Users/15185/AppData/Local/Temp/a04/src/bin/liveness.rs   # reltol/abstol/trtol 是否改变返回时间轴
 cargo run --offline --manifest-path C:/Users/15185/AppData/Local/Temp/a04/Cargo.toml --bin confirm   # EXIT=0
 cargo run --offline --manifest-path C:/Users/15185/AppData/Local/Temp/a04/Cargo.toml --bin liveness  # EXIT=0
 ``
实验电路要点（复现必需）：`R=1 kΩ`、`C=100 nF`（τ=100 µs）、`stop=5τ`、源 `PULSE(0 1 td=0 tr=tf=1e-12 pw=10 per=20)` 或 `PWL(0,0)(1e-9,1)`、`Analysis::Tran(TranAnalysis{step, stop, start, uic, tmax})`，入口 `thevenin::circuit::simulate_tran`；浮空岛 = 两侧节点仅由 1 kΩ 相连 + 从地注入 1 pA 到一侧（线性）/再加一只二极管（非线性）。
斜坡精确解（对照公式，实测用）：`t≤tr: v = t/tr − (τ/tr)(1−e^{−t/τ})`；`t>tr: v = 1 − (τ/tr)(1−e^{−tr/τ})e^{−(t−tr)/τ}`，其中 `tr = max(user_tr, tstep)`。

（本报告仅写入 `docs/review-evidence/backend-contract.md`；未修改仓库任何其它文件，未 commit/push，仓库外临时工程位于 %TEMP%/a04。）

