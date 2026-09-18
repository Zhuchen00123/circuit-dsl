# REPL 与语法整理

> 本文记录 `cdsl repl` 的设计与**实测行为**。文中的命令、输出与限制都来自真实
> 运行；不能自动化验证的部分在 §8 单独列出。

## 1. 范围

本轮做了：

- **语法审查与收敛**：一张审查表（§2）与由此产生的改动（§3）。
- **`cdsl repl`**：表达式求值与会话变量、多行输入、定义与运行、会话命令、历史与补全。
- **共享实现**：把 `run` 的执行流程提取到 `circuit-session`，文件模式与 REPL
  只有一套求值器与一套执行器（§6）。
- 修正 `examples/voltage_divider.cdsl` 的错误注释，并同步语言规范、README 等文档。

本轮不做（记录在案，避免"顺手加上"）：

- 类、闭包、用户自定义函数、模块系统、递归。
- `while`、`each`、`times`、`break`、`continue`、后置 `if`/`unless`、三元表达式。
- 花括号块、隐式参数、字符串插值、开放类、猴子补丁、元编程、用户自定义运算符。
- 真值性（truthiness）、隐式类型提升、隐式字符串-数值互转。
- 可视化报告（本轮暂缓）。
- 用户自定义命令、脚本化 REPL、跨会话持久化。

## 2. 语法审查表

"现状"一列描述的是本轮实测过的行为（诊断文本取自实际运行）。

| 现有写法 | 决定 | 原因 |
|---|---|---|
| `do ... end` 块（circuit / subcircuit / experiment / for / if） | 保留 | Ruby 风格的核心；单一闭合词，不需要缩进敏感 |
| `if cond do ... elsif ... else ... end` | 保留，只此一种 | 不加后置 `if`/`unless`、不加三元；条件必须是布尔值（`if 1 do` → `E_TYPE`：`` `if` needs a boolean condition, found number ``） |
| `:symbol` 名称与 `label: value` 关键字参数 | 保留 | 两者不同（`value:` 不是 `:value`），语句参数唯一形式就是标签 |
| `1.kohm`、`100.nF` 量纲字面量 | 保留 | 电路可读性的核心；后缀是受限语法而非方法调用（`4.sqrt` → `` `sqrt` is not a known unit suffix ``） |
| 单位别名 `ohm`/`Ohm`/`Ω`、`u`/`µ`/`μ`/`U`、`k`/`K` | 保留，文档写明规范写法 | 这是**同一物理单位的输入编码**，不是两种语法；`Ω`/`µ` 是真实符号，`Ohm` 是 SPICE 习惯。规范写法取 `ohm`、`u`、`k` |
| `m`=milli、`M`=mega | 保留 | 已有规则与测试 |
| 表达式内调用必须带括号：`sqrt(4)`、`v(:out)` | 保留，**加强诊断** | 见 §3.1 的边界规则；`sqrt 4` 现在报 `` `sqrt` is a function; call it as `sqrt(...)` `` |
| `and` / `or` / `not` | 保持不支持，**改善诊断** | 现在报 `` `and` is not an operator; write `&&` ``（原来只报 unexpected identifier） |
| `&&` / `\|\|` / `!` | 唯一一套布尔运算符 | 不加别名 |
| `for x in 1..3 do`、`for x in [..] do` | 唯一迭代形式 | 不加 `each`/`times`/`while`；区间含两端 |
| `param :r, default: 1.kohm` | 保留，**限定出现位置** | 见 §3.3：只能出现在 circuit/subcircuit body 顶层 |
| `name = expr` | **新增，仅限 REPL 顶层** | 会话变量。文件内写它报错并指向 `param`，见 §3.2 |
| `+` 的字符串拼接 | 保留，规则写清并加测试 | 见 §3.4 |
| `instance :s, of: :c, ports: {..}, params: {..}` | 保留，仅字典形式 | 实测无位置参数形式 |
| `ac` 的 `points_per_decade:` 与 `points:` | 保留，互斥并报错 | 对数密度与线性点数是不同语义 |
| `dc` 的 `source:`/`param:`、`step:`/`points:` | 保留，互斥并报错 | 同上 |
| `save v(:a), i(:d)`、`measure :n, max: v(:out)` | 保留 | 归约名即关键字，每种归约一种写法 |
| 层次路径 `:stage1.r1` 与唯一叶名 `:internal` | 保留 | 叶名歧义时明确报错并列出完整路径 |
| 器件参数名（`p`/`n`/`value`/`dc`/`ac`/`waveform`/`model`） | 保留 | 每个器件只有一组名字，没有 `r:` 之类别名 |

## 3. 语法整理的具体决定

### 3.1 调用与语句的边界

一条可检验的规则：

- **语句**以关键字开头。关键字集合是封闭列表（`crates/circuit-dsl/src/token.rs`
  的 `KEYWORDS`）。语句的参数是 `label: value`，**不带括号**，逗号分隔，行尾结束。
- **表达式**是值：字面量、变量、算术/比较/布尔运算、括号调用 `f(a, b, key: v)`、
  数组 `[..]`、字典 `{..}`。表达式里调用**必须带括号**。
- `save v(:out)` 中 `save` 是语句、`v(:out)` 是表达式；`op` 是语句且不接受参数。
- 括号不出现在语句关键字之后（`node(` 之类的输入是语法错误）。

### 3.2 赋值只在 REPL

- REPL 顶层 `r = 1.kohm` 定义或重定义**会话变量**（只能是不带量纲以外的数值，
  见 §4.3）。
- 文件里写 `r = 1.kohm` 报错并给出改法：
  - 文件顶层：`assignment is not available in a source file` +
    `declare it as param :r, default: <value> in a circuit body, or param :r, value: <value> in an experiment`；
  - 电路体内：`assignment is not available in a circuit body` + `declare the parameter with param :r, default: <value>`；
  - 实验体内：`assignment is not available in an experiment` + `write param :r, value: <value> to override it for this experiment`。
- 理由：`param` 是设计的一部分（可被实例/实验/扫描点覆盖，参与拓扑不变性检查），
  会话变量是交互草稿。两者语义不同，各有唯一写法。

### 3.3 `param` 的出现位置与块作用域

实测：`for` 体内再写 `param :k` 会报 `E_DUPLICATE`（"declared twice"），因为声明集合
不随块嵌套——这既不好解释也不好用。本轮定为：

- `param` 只能出现在 **circuit / subcircuit body 顶层**，以及 experiment body 顶层的覆盖。
- 出现在 `for` / `if` 块内 → `E_UNSUPPORTED`：
  `` `param` declares a design parameter, so it belongs in the circuit body `` +
  "a parameter cannot be declared inside `for` or `if`: move it up, put the condition in the
  `value:` expression, or write one device statement per branch"。
- 于是每个电路定义**只有一个参数命名空间**：没有遮蔽，也没有"声明两次"的歧义。
- 循环变量：作用域限于该次迭代的 body，迭代结束即恢复（有测试钉住；循环外引用
  循环变量报 `E_NAME`）。`if` 分支不引入新变量。
- 名称解析不做上下文猜测：`x` 要么是当前体内已声明的参数，要么报 `E_NAME`；
  裸词永远不会被当作节点、器件或函数。

### 3.4 动态名称拼接的规则

语义未改，规则写进规范并补了测试：

- `+` 的两侧只要有**任意一侧是字符串**，结果就是字符串；另一侧按下列规则转文本：
  无量纲数 → 十进制文本（整数不带小数点），符号 → 文本，布尔 → `true`/`false`。
- **带量纲的数不转文本**：`"r" + 1.kohm` 报 `E_TYPE`（否则 `r1` 与 `r1000` 无从区分）。
- 数组、字典不转文本，报 `E_TYPE`。
- `:sym + "a"` 合法（有一侧是字符串），`:sym + 1` 不合法（两侧都不是字符串）——
  触发条件是**字符串**，符号不触发拼接。
- `str(x)` 是显式转换，规则同上。
- 不新增任何隐式转换：`"1" + 1` 得 `"11"`（拼接）而不是 `2`；数值与字符串比较报 `E_TYPE`。

### 3.5 覆盖必须命中真实参数

实测发现并修掉的一个静默降级：**覆盖一个不存在的参数会被无声忽略**。字面量电路里
`resistor :ra, ..., value: 1.kohm` 没有 `param`，此时 `:run divider r1=3.kohm` 曾经
原样跑出默认结果。现在顶层覆盖链（实验的 `param`、扫描点、会话覆盖）在 body 执行后
校验一遍：

```
error[E_NAME]: circuit `divider` has no parameter `r1`
   = declared parameters: <none>
   = an override that names nothing would silently leave every value at its default
```

实例的 `params: { .. }` 本来就有同样的检查，这条规则现在两处一致。

## 4. 使用

### 4.1 启动

```bash
cdsl repl                       # 空会话
cdsl repl examples/rc_filter.cdsl   # 先载入一个文件
```

一段可复制的完整会话（下面是从真实运行复制的输出）：

```text
$ cdsl repl
cdsl> r = 1.kohm
r = 1 kohm
cdsl> c = 100.nF
c = 100 nF
cdsl> tau = r * c
tau = 100 us
cdsl> circuit :divider do
....>   param :r1, default: 1.kohm
....>   param :r2, default: 1.kohm
....>   node :in, :out
....>   voltage_source :v1, p: :in, n: :gnd, dc: 5.V
....>   resistor :ra, p: :in, n: :out, value: r1
....>   resistor :rb, p: :out, n: :gnd, value: r2
....> end
defined circuit `divider`
cdsl> experiment :divider, circuit: :divider do
....>   op
....>   save v(:out), i(:ra)
....> end
defined experiment `divider`
cdsl> :run divider
experiment `divider` (backend thevenin 0.5.0)
  op1: scalar; signals: v(out), i(ra)
  v(out) = 2.5 V
  i(ra) = 2.5 mA
cdsl> :run divider r1=3.kohm
experiment `divider` (backend thevenin 0.5.0)
  override r1 = 3 kohm
  op1: scalar; signals: v(out), i(ra)
  v(out) = 1.25 V
  i(ra) = 1.25 mA
cdsl> :quit
```

### 4.2 命令

命令不是 `.cdsl` 语言的一部分：只有固定的七个词被当作命令，其余一律按语言解析，
所以 `:vin` 是一个符号值（求值得到 `:vin`）。想拿这些词本身当符号，写括号形式
`(:help)`。

| 命令 | 行为 |
|---|---|
| `:help` | 命令列表与语言要点 |
| `:load <file> [--replace]` | 原子载入：默认遇到同名定义就报错并列出冲突名，`--replace` 才替换。失败什么都不改；不动会话变量 |
| `:list` | 当前定义与会话变量 |
| `:run <exp> [name=expr ...] [--out DIR]` | 执行实验；标量分析直接列出各信号的值，多点的分析提示用 `--out` 写出 |
| `:reset` | 清空定义与变量，并报告清掉了多少 |
| `:quit` / `:exit` | 退出（Ctrl+D 亦可） |

启动时的额外提示（都是实测的）：

```text
cdsl> load x.cdsl          # 少写了冒号
error[E_SYNTAX]: `load` is a command; write `:load`
   = commands are not part of the language, so they always start with `:`

cdsl> op                   # 把 body 语句敲在提示符下
error[E_SYNTAX]: `op` is a body statement, not a session input
   = write it inside `experiment :name, circuit: :name do ... end`
```

### 4.3 作用域边界

- 会话变量只用于交互计算。
- 电路、子电路、实验**不隐式捕获**会话变量：展开从空参数作用域开始。
  `r = 5.kohm` 之后，电路里引用未声明的 `r` 仍然报 `` `r` is not declared ``。
- 反向也成立：电路参数不是会话变量，`r1` 在提示符下报未声明。
- 会话值进入仿真只有一条路：`:run <exp> name=expr`，其中 `expr` 在**会话作用域**
  求值一次并在运行开始时定值；摘要里会打印 `override name = value`，所以这次运行
  可以照着读回来、复现出来。
- 命名空间：变量与定义互不遮蔽（变量 `divider` 与 `circuit :divider` 可以并存）。
- 覆盖值必须是数值：`x = :vin`、`y = [1, 2]` 都报 `` a variable must be a number ``。

### 4.4 显示格式

- 量纲值：工程计数法 + 规范单位，**6 位有效数字**，前缀取自
  `circuit-core::units::DISPLAY_PREFIXES`（与解析表有往返测试，不会漂移）：
  `1.kohm / 2` → `500 ohm`；`1.kohm * 100.nF` → `100 us`；`2.2e6 ohm` → `2.2 Mohm`。
- 无量纲：不加前缀也不加单位：`1 + 1` → `2`。
- 复合量纲（如 `V*A`）：不加前缀，写成 `6 V*A`。
- 其余值：`:vin`、`"r1"`、`true`、`[1, 2]`、`{ a: 1 }`。
- 超出前缀范围（`1e20 V`）退回指数形式。
- 存储始终是 SI 全精度，只有显示做有效数字截断。

## 5. 完备性判定：三态

`crates/circuit-dsl/src/complete.rs`：

```rust
pub enum Completeness {
    Complete,
    Incomplete { reason: &'static str, span: SourceSpan },
    Invalid(Diagnostics),
}
pub fn assess(source: SourceId, text: &str) -> Completeness
```

判定顺序：

1. **词法失败 → `Invalid`**。词法错误都是最终的（未知单位后缀、非法字符、`...`、
   字符串插值、未闭合字符串——字符串不能跨行是语言规则，所以它不算续行）。
2. **解析器先判**。解析通过 → `Complete`。解析失败且**parser 是在输入末尾报的错**
   （`Parsed::ran_out`，即报错时光标停在 EOF）并且 token 流结构上是开的
   （未闭合的括号 / 未闭合的 `do` 块 / 行尾是逗号、运算符、`:`、`..`、`=`、`=>`）
   → `Incomplete` 并给出原因与开括号的位置。
3. 其余 → `Invalid`，把真实语法错误交出来。

之所以让解析器先判：单看结构会把 `for k in 1..3 do` 判成"未完成"（它确实留着一个
未闭合的 `do`），但 `for` 在会话提示符下**永远不可能是合法的**——它必须立刻报错。

实测对照：

| 输入 | 判定 |
|---|---|
| `circuit :d do` / `circuit :d do` + 两行 | `Incomplete`（unclosed `do` block） |
| `r = sqrt(` / `r = min(1,` / `r = [1, 2,` / `r = { a:` | `Incomplete`（unclosed `(` / `[` / `{`） |
| `r = 1 +` / `r = 1 *` / `r =` / `a == ` / `b = 1 \|\|` | `Incomplete`（行尾是运算符） |
| `node 5` / `end` / `)` / `1 +* 2` | `Invalid`（真实语法错误） |
| `for k in 1..3 do` / `op` / `node :a` | `Invalid`（body 语句，附所在位置提示） |
| `r = "abc` / `r = 1.xV` / `r = 0...3` | `Invalid`（词法错误是最终的） |
| `r = "do"` / `# do not close this` | `Complete`（token 级扫描不会被字符串/注释骗到） |

## 6. 分层与文件

依赖方向：`core <- results <- backend <- session <- cli`。

- `crates/circuit-dsl/src/eval.rs`（新）：表达式求值器，**唯一**一套值语义。
  名字查找走 `Variables` trait，展开器传入自己的参数作用域（带声明位置），
  会话传入变量表。`elaborate.rs` 的原有调用点通过一层薄包装保持不变。
- `crates/circuit-dsl/src/complete.rs`（新）：§5 的三态判定。
- `crates/circuit-dsl/src/parser.rs`：`=` 成为 token；新增 `parse_input` /
  `Parsed` / `Input`；`param` 位置与五处针对性提示。
- `crates/circuit-session/`（新 crate）：
  - `execute.rs`：执行一次实验或参数扫描、测量求值、结果写出。**从 CLI 的
    `run.rs` 提取**，文件模式与 REPL 共用。
  - `session.rs`：会话状态、命令、定义替换规则。
  - `format.rs`：REPL 的值显示。
- `crates/circuit-cli/src/repl.rs`（新）：终端层。stdin 不是 TTY 时退化为普通逐行
  读取，因此一段真实会话可以用管道驱动、可被测试。
- `crates/circuit-core/src/format.rs`（新）：`format_number` 从 `circuit-results`
  下沉到这里（`circuit-results` 再导出，保持 API 不变），并新增 `format_quantity`。
- `crates/circuit-cli/src/run.rs`：改为调用共享执行器，只保留文件 I/O、写出与摘要；
  `check::FrontEnd` 现在把已解析的 `Program` 一并返回，扫描路径不再二次读盘解析
  （顺带修掉了原来一次运行解析两遍的问题）。

## 7. 测试与验证

实测：`cargo test --workspace` 共 **389 个测试全部通过**（0 失败）；
`cargo nextest run --workspace` 报 388（nextest 不跑 doc-test）；
`cargo clippy --workspace --all-targets -- -D warnings` 无输出；
`cargo fmt --all -- --check` 无差异。

本轮新增的测试（对比上一轮结束时的 309 个，共 +80）：

| 位置 | 数量 | 覆盖 |
|---|---|---|
| `circuit-core` `format`（新）+ `units` | +12 | 工程计数法、6 位有效数字、无量纲与复合量纲、极端量级回退、零保留单位、**显示前缀与解析表往返** |
| `circuit-dsl` `complete`（新） | +12 | 三态判定的每一类：未闭合块/括号/行尾运算符、真实错误、词法错误是最终的、多行续行后完成、字符串与注释里的 `do`、空输入、`do` 位置可定位、body 语句的归属提示 |
| `circuit-dsl` `parser` | +5 | `parse_input` 的四种输入、赋值目标必须是普通名字、文件内赋值（顶层/电路体/实验体）与 `param` 指向、针对 `for`/`op`/`load`/`run`/`sqrt`/`and`/`not` 的提示、`=` 不吞掉 `==`/`!=`/`<=`/`>=`/`=>` |
| `circuit-dsl` `tests/elaborate.rs` | +6 | `param` 不能声明在块内、循环变量不逃逸、循环变量可与外层同名并恢复、拼接规则（数值/符号/布尔可转，带量纲数/数组不可转）、覆盖必须命中已声明参数、实验的 `param` 覆盖同样受限 |
| `circuit-session`（新 crate） | +34 | 执行器与显示（9）；会话行为（25）：变量保存与重赋值、量纲传播与量纲错误、多行续行与真实错误区分、取消输入保留会话、诊断指向 `<repl:N>`、定义替换（成功/失败回滚/破坏实验时拒绝）、命名空间分离、前向引用提示、会话变量不泄漏、参数不泄漏到会话、覆盖改数值、未知参数覆盖被拒、用当前定义重新展开、扫描实验、命令行为、空输入 |
| `circuit-cli` `tests/repl.rs`（新）+ bin 单元测试 | +17 | 真实进程管道会话（11）：定义→运行→改参→再运行（断言两次数值不同）、表达式与变量、续行提示、语法错误后会话继续（退出码 1）、失败定义不影响旧定义、`:load` 示例并运行、重复载入冲突与 `--replace`、`:run --out` 写出文件、`:quit` 与 EOF、未知命令与符号、变量不进入电路；交互路径里可测的部分（6）：取消输入的效果、提示符切换、补全候选、历史文件往返、历史路径、`:quit`/报错的步骤分类 |

回归：原有测试全部保持通过；文件模式的 CLI 行为除"未知参数覆盖现在会报错"这一处
刻意的修正外没有变化（该修正是为了不再静默忽略，见 §3.5）。

## 8. 交互行为的验证边界

终端按键本身需要真实 TTY，无法在自动化测试里模拟（本轮尝试过用 `winpty` 分配控制台
来驱动真实交互路径，因为拿不到控制台尺寸而失败）。因此把交互路径里**可以测的部分
拆出来测了**，并明确列出剩下的部分：

| 项目 | 验证情况 |
|---|---|
| Ctrl+C 取消当前输入、保留会话 | **有测试**：`an_interrupt_drops_the_input_and_keeps_the_session` 断言半条定义被丢弃、变量仍在、下一条输入按新输入读。按键→`Interrupted` 这一步是 rustyline 的契约，不在本项目测试范围内 |
| 提示符切换（主提示 / 续行） | **有测试**：`the_prompt_follows_what_the_session_is_waiting_for` 断言 `cdsl> ` → `....> ` → `cdsl> ` 的切换；终端上的实际显示未覆盖 |
| 结束输入（Ctrl+D）与 `:quit` | **有测试**：`:quit` 与管道 EOF 走同一退出路径（`quitting_and_end_of_input_both_exit_cleanly`），`classify` 把 `Eof` 映射为退出。真实按键未覆盖 |
| 历史 | **有测试**：内存缓冲与文件的往返（`history_round_trips_through_a_file`）、路径在 home 目录下。↑ 键行为未覆盖 |
| 补全 | **有测试**：`Helper::complete` 对命令、定义名、变量名的候选与起始位置。Tab 键行为未覆盖 |
| 真实终端下的整段会话 | **未验证**（本轮无法分配 TTY）。管道模式下的同一条会话逻辑有 11 个端到端测试 |

也就是说：**会话行为**有测试，**按键到行为的最后一跳**没有。后者是 rustyline 的
职责，本项目只负责把错误变体映射到正确的动作，那部分有测试。

## 9. 后续设计方向（本轮不做）

- `:save <file>` 把当前定义写回 `.cdsl`（需要先决定注释与格式的保留策略）。
- 结果表达式的 `measure` 语法接入（求值器 `circuit-results::expr` 已就绪）。
- AC 相位语法（IR 支持，语言无法表达）。
- 受控源（VCVS/VCCS）与更多器件模型。
- 若确有需要，为块引入局部值绑定——届时应让"设计参数"与"局部值"成为两个明确的
  概念，而不是给同一个语义两种写法。
