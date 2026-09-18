# 第 2 轮设计冻结（Lead，Wave 0 → G0）

日期：本轮。仓库：`F:\codexprojects\dsl000`。基线：`cargo test --workspace` → **413 passed / 0 failed，exit 0**（Lead 实跑，日志 `target/round2-logs/baseline-workspace-test.txt`）。
上一轮结论一律视为历史基线，本轮所有判断重新取证。

## 1. 内核事实（Lead 直读 vendored 源码，Wave 1 前复核见 `kernel-contract.md`）

Thevenin 0.5.0，源码 `C:\Users\15185\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\thevenin-0.5.0\`。

| 事实 | 位置 |
|---|---|
| `h_max = t_max.unwrap_or(min(h_print, tstop/50))` | `transient.rs:799` |
| `h_min = h_print * 1e-9` | `transient.rs:1407` |
| 初始 `h = max(h_max/400, h_min)` | `transient.rs:1408` |
| 非电抗分支步长上限 `min(h_max, h_print)` | `transient.rs:1694` |
| PULSE `tr = tr.unwrap_or(tstep).max(tstep)`（tf 同） | `waveform.rs:37-38`；评估 `waveform.rs:116+` |
| breakpoint 用的 `tr_val`/`tf_val` 同样被 clamp，`period.max(tr+pw+tf).max(tstep)` | `waveform.rs:271-278` |
| `min_break = tstep * 5e-5` | `transient.rs:308` |
| 断点：步长被夹到断点距离，断点处 `step_h = step_h.min(h*0.1).max(h_min)`，且强制 Backward-Euler | `transient.rs:1432-1468` |
| 输出录制：每条被接受的内步且 `t >= t_start` 都记录一条，无输出抽样；t=0 首点单独记录 | `transient.rs:1336-1383`、`2272-2285` |
| `TranParams{tstep, tstop}` 只由 `h_print`/`t_stop` 构造 | `transient.rs:1327-1330` |

结论：`output_interval → Tran.step` 既通过 `h_max` 回退影响积分，又通过 clamp 直接改写用户声明的 rise/fall 与 breakpoint 位置。两者必须解耦。

DSL 事实：`pulse(low, high, delay?, rise, fall, width, period)` 中 `rise/fall/width/period` 在 `elaborate.rs:1659-1662` 是**必填**（`delay` 可选），只校验非负（`elaborate.rs:1679`），不校验正数、也不校验有限值；`output_interval` 在 `elaborate.rs:2446-2452` **没有任何值校验**（`max_step` 有 `> 0` 校验，`elaborate.rs:2433-2443`）。

## 2. 任务 A 冻结语义

三个概念彻底分开，任何一层都不得互相借用：

| 概念 | 载体 | 前端规则 | 传给引擎 |
|---|---|---|---|
| 输出采样 | `TranSpec.output_interval: Option<f64>` | 显式值必须有限且 `> 0`，否则 `E_VALUE`（不回退默认） | **不传**。求解后独立重采样 |
| 积分步长上界 | `TranSpec.max_step: Option<f64>` | 显式值必须有限且 `> 0` | `CqTran.tmax` |
| 引擎 print step | 适配器内部量 `h_print` | 用户不可见 | `CqTran.step` |

### 2.1 `h_print` 唯一计算规则（`thevenin.rs`，单一函数，`validate` 与 `map_analysis` 共用）

```text
span = stop_s - start_s
default_print = span / 1000                       // 旧默认，保留
waveform_bound = min over every source waveform of the declared values the engine
                 compares against tstep:
                   PULSE  : rise, fall, period     // DSL 强制声明；值 <= 0 或非有限 → 能力错误
                   EXP    : tau1, tau2 (Option)    // 仅 IR 层可达
                   PWL / SIN / SFFM / AM: 无 tstep 依赖
h_print = waveform_bound ? min(default_print, waveform_bound) : default_print
if !(h_print > 0.0) || !h_print.is_finite() → 能力错误（E_UNSUPPORTED，点名参数）
有效步长 effective_step = lte_controls ? h_max_eff : min(h_max_eff, h_print)
   其中 h_max_eff = max_step.unwrap_or(min(h_print, stop/50))
        lte_controls = 电路含电容或电感（引擎走 LTE 分支，transient.rs:1620-1681，步长只受 h_max 约束）
if span / effective_step > MAX_PRINT_STEPS (1e6) → 能力错误（附需要的步数）
```

> 步数判据必须区分 LTE 分支与无-LTE 分支：`h_print` 只在无电容/电感时通过
> `transient.rs:1694` 真正压住积分步；含电抗时步长由 `h_max` 决定。若对两者用同一
> 判据，会误拒「rise=1ps + max_step=τ/200」这类完全合法的配置（已有集成测试覆盖该
> 场景，Wave 1 首次运行时被拒，据此修正）。

不变量（将成为测试断言）：
- `output_interval` **不进入** `h_print`，也不进入 `CqTran` 的其它字段；改变它只改变输出网格。
- `max_step` 只进入 `tmax`；它也不改变 `h_print`。
- 对已声明的 `rise/fall/period`，`h_print <= 该值`，因此引擎的 `.max(tstep)` clamp 不触发，声明波形与 breakpoint 位置按声明值执行。
- `waveform_bound` 为空时，行为与旧默认一致（`span/1000`），不引入回归。
- **不允许把 `step` 设成 0**：违反上面的能力检查直接报错。

### 2.2 独立输出重采样（`circuit-results/src/resample.rs`，新模块）

只在 `output_interval: Some(iv)` 时启用，且只作用于 `Axis::Time` 的 tran 数据集。

| 项 | 规则 |
|---|---|
| 网格起点 | 原始时间轴首点 `t0`（恒保留，值直接复制） |
| 网格内部点 | `t0 + k*iv`，`k = 1,2,…` 且 `< t_last` |
| 终点 | 原始末点 `t_last` **恒保留**（即使不在网格上），因此输出覆盖完整仿真窗口 |
| 插值 | 相邻两个原始样本上的线性插值；首末点取原值 |
| 外推 | 禁止：每个输出点都落在 `[t0, t_last]` 内 |
| 规模 | 输出标量值总数 = 输出点数 × 信号数；超过 `limits.max_result_values` → `E_LIMIT`（不截断、不抽样） |
| 未指定 | `None` 时输出网格 = 原始求解网格，逐点原样 |
| 空/退化轴 | 少于 2 个原始点时不重采样，保留原样（无插值可用） |

### 2.3 测量语义（要求 6）

- `measure` 的 `avg/rms` 仍在**原始求解网格**上做梯形积分；`RunOutcome.datasets` 保持原始数据不变。
- 新增 `RunOutcome.output_datasets`：展示与导出用（重采样后）。CLI 文件模式与 REPL 都走它。
- 由此保证：改 `output_interval` 只改变用户看到的采样点，**不改变** `avg/rms/max/min` 的数值。

### 2.4 元数据（要求 6/计划第 6 条）

`BackendInfo.settings` 记录，使原始数据与重采样数据可区分：
`tran.solver_step`（实际 `h_print`）、`tran.max_step`（如有）、`tran.waveform_bound`（如有）、
`tran.solve_points`、`tran.output_grid`（`solver` / `resampled-linear`）、`tran.output_interval`（如有）、`tran.output_points`。

## 3. 任务 B 冻结范围

1. 产品路径回归：非零 `delay` + 有限 `rise/fall` + ≥2 个周期，全有效区间对照独立解析解（分段斜坡-指数解，`expm1` 稳定形式）。
2. 分别研究：`max_step`（产品路径可控）、断点重启（第 1 个断点后首个接受步为 BE）、实际生效容差（产品路径无容差通道 → 只能在内核层 `_probe` 报告，作为限制）。
3. 保留全部失败样本；不删点、不放宽阈值。不满足 §17 的配置作为“限制/诊断实验”与必过回归分开报告。
4. 若归因于第三方内核：给出最小复现 + 评估最小适配方案，不重写、不替换内核。

## 4. 明确不做

参数 DAG、结果表达式接入（任务 C/D）、新器件、GUI、LSP、后端替换、发布/提交/推送。也不重复修已通过的相位与浮空测试。
