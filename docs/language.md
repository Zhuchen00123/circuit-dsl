# circuit-dsl 语言规范

> 版本 0.1.0。本文描述**当前实现**的语法与语义。
> 凡未实现的能力，在 §10 单独列出，不得在别处声称支持。

源文件扩展名 `.cdsl`，UTF-8 编码。

## 1. 词法

### 1.1 空白与注释

空格、制表符、回车换行均为空白。`#` 开始行注释，直到行尾。
注释与空白不计入语法，但保留其位置用于诊断。

### 1.2 标识符

```
ident := [A-Za-z_][A-Za-z0-9_]*
```

首期仅 ASCII。标识符区分大小写。

### 1.3 关键字

```
circuit  subcircuit  experiment  param  node  instance  model
do  end  else  elsif  if  for  in
true  false
op  dc  ac  tran  save  measure
resistor  capacitor  inductor  voltage_source  current_source  diode
pulse  sin  pwl
from  to  step  points  points_per_decade  default  value  of  ports  params
```

关键字是保留字，不能用作标识符。

### 1.4 符号（symbol）

```
symbol := ':' ident
```

`:gnd` 是保留符号，始终指向全局参考地。符号与字符串是**不同类型**。

### 1.5 数值字面量

```
int      := digit+
float    := digit+ '.' digit+ (exp)?  |  digit+ exp  |  '.' digit+
exp      := ('e'|'E') ('+'|'-')? digit+
quantity := (int|float) '.' unit_suffix
```

- `1` 是整数，`1.5`、`1e3`、`1.5e-6` 是浮点数。
- `1.kohm`、`100.nF`、`1.us`、`10.Hz`、`1.V` 是**量纲字面量**。
- 量纲字面量由词法器作为**单个 token** 识别：`数字` + `.` + `单位后缀`。
  点号后面必须紧跟字母，才会被当作单位后缀。

单位后缀是**受限语法**，不是方法调用。合法形式为 `前缀? 基本单位`：

| 基本单位 | 量纲 |
|---|---|
| `V` | 电压 |
| `A` | 电流 |
| `s` | 时间 |
| `Hz` | 频率 |
| `ohm` / `Ohm` / `Ω` | 电阻 |
| `F` | 电容 |
| `H` | 电感 |

| 前缀 | 倍数 | 前缀 | 倍数 |
|---|---|---|---|
| `f` | 1e-15 | `k` `K` | 1e3 |
| `p` `P` | 1e-12 | `M` `meg` | 1e6 |
| `n` `N` | 1e-9 | `g` `G` | 1e9 |
| `u` `µ` `μ` | 1e-6 | `t` `T` | 1e12 |
| `m` | 1e-3 | `a` `A` | 1e-18 |

**大小写敏感**：`m` 是 milli（1e-3），`M` 是 mega（1e6）。
`1.mF` 是毫法，`1.MF` 是兆法。

非法后缀（`1.xV`、`1.m`、`1.kg`）是词法错误 `E_SYNTAX`。

### 1.5.1 明确拒绝的 Ruby 写法

以下写法来自 Ruby，但本语言**不支持**，且都会报 `E_SYNTAX` 而不是被静默接受：

| 写法 | 处置 | 替代 |
|---|---|---|
| `0...3`（排他区间） | 报错 | `0..3`（含两端） |
| `"r#{k}"`（字符串插值） | 报错 | `("r" + k)` |
| `%w[a b]` | 报错 | `["a", "b"]` |
| `x += 1` | 报错 | 文件里用 `param`；REPL 里用 `x = ...` |
| `a ? b : c` | 报错 | 用 `if` / `else` |
| `case` / `when` / `unless` | 报错 | 用 `if` / `elsif` / `else` |
| `.each do \|x\| ... end` | 报错 | 用 `for x in ... do` |
| `4.sqrt` | 报错 | `sqrt(4)` |
| `sqrt 4`（无括号调用） | 报错，提示写法 | `sqrt(4)` |
| `a and b` / `a or b` / `not a` | 报错，提示写法 | `a && b` / `a \|\| b` / `!a` |

拒绝词运算符与无括号调用时，诊断会**指名该写什么**，而不是只说"unexpected identifier"：

```
error[E_SYNTAX]: `and` is not an operator; write `&&`
error[E_SYNTAX]: `sqrt` is a function; call it as `sqrt(...)`
```

`0...3` 与 `"r#{k}"` 曾经被**静默误解析**——前者会变成 `0..0.3` 并被舍入成一次迭代，
后者会变成一个名字就叫 `r#{k}` 的器件。现在两者都在词法阶段报错。

### 1.6 字符串

```
string := '"' (任何非引号非换行字符)* '"'
```

首期不支持转义序列。字符串与符号不同：`"gnd"` 与 `:gnd` 类型不同。

### 1.7 标点与运算符

```
( ) [ ] { } , . : => .. 
+ - * / ! < <= > >= == != && ||
```

### 1.8 换行与续行

语句以**换行**终止，但在下列情况中换行被忽略：

1. 括号 `(` `)` `[` `]` `{` `}` 内部的任意换行；
2. 逗号之后的换行；
3. 行尾是二元运算符时（`+` `-` `*` `/` `&&` `||` `==` `!=` `<` `<=` `>` `>=`）。

这三种之外的换行都结束当前语句。因此：

```ruby
resistor :r1, p: :vin, n: :out,
  value: 1.kohm          # 逗号后续行，合法

voltage_source :v1, p: :a, n: :gnd,
  dc: 1.V +
     2.V                 # 运算符后续行，合法
```

## 2. 表达式

### 2.1 优先级（由低到高）

| 级别 | 运算符 | 结合性 |
|---|---|---|
| 1 | `\|\|` | 左 |
| 2 | `&&` | 左 |
| 3 | `==` `!=` | 左 |
| 4 | `<` `<=` `>` `>=` | 左 |
| 5 | `+` `-` | 左 |
| 6 | `*` `/` | 左 |
| 7 | 一元 `-` `+` `!` | 右 |
| 8 | 调用 `f(...)`、下标、字面量、`(...)` | — |

### 2.2 字面量

```ruby
42            1.5          1e-3         1.kohm
true          false
:name         "text"
[1, 2, 3]                             # 数组
{ input: :vin, output: :mid }         # 字典
```

数组与字典中的元素可跨行书写，逗号后可换行，末元素后可留尾逗号。

### 2.3 名称解析

表达式中的标识符只能是：

- 已声明的**参数名**（`param :r` 之后可用 `r`）；
- 内置常量：`pi`、`e`；
- 在 REPL 里还包括**会话变量**（`r = 1.kohm` 之后可用 `r`）。会话变量与电路参数
  是两个互不可见的作用域，见 `docs/repl.md` §4.3。

**未声明的标识符一律报 `E_NAME`**，不会自动变成节点、器件或函数调用。
裸标识符**不是**函数调用——函数调用必须带括号。名称解析不看上下文，不做猜测。

### 2.4 量纲运算

- `+` `-`：两侧量纲必须相同，否则 `E_DIMENSION`。
- `*` `/`：量纲指数相加/相减，结果量纲自动推导。
- 比较 `<` `<=` `>` `>=` `==` `!=`：两侧量纲必须相同。
- 需要无量纲的地方（如 `points`）**不接受**带量纲值，报 `E_DIMENSION`。
- 非有限值（`inf`、`NaN`）在展开期报 `E_VALUE`。
- **`+` 的拼接规则**：两侧只要有任意一侧是**字符串**，结果就是字符串，另一侧转文本。
  能转文本的是无量纲数（整数不带小数点）、符号、布尔；**带量纲的数、数组、字典不能转**，
  报 `E_TYPE`（否则 `"r" + 1.kohm` 与 `"r" + 1000` 无从区分）。
  `:sym + "a"` 合法（有一侧是字符串），`:sym + 1` 不合法（两侧都不是）。

内置函数：

| 函数 | 说明 |
|---|---|
| `pulse(low:, high:, delay:, rise:, fall:, width:, period:)` | 脉冲波形 |
| `sin(offset:, amplitude:, frequency:, delay:, damping:, phase:)` | 正弦波形 |
| `pwl([t0, v0, t1, v1, ...])` | 分段线性波形 |
| `v(:node)` / `v(:a, :b)` | 电压探针（仅 `save`）；名字可以是层次路径或字符串，见 §5.2 |
| `i(:device)` | 电流探针（仅 `save`）；同上 |
| `abs(x)` `sqrt(x)` `min(a,b)` `max(a,b)` | 数值函数 |

## 3. 程序结构

一个 `.cdsl` 文件由若干**顶层定义**组成，顺序无关：

```
program := (circuit_def | subcircuit_def | experiment_def)*
```

## 4. 电路与子电路

```ruby
circuit :name do
  <body>
end

subcircuit :name, ports: [:a, :b] do
  <body>
end
```

`subcircuit` 必须声明 `ports`。`circuit` 的端口是全局的 `:gnd` 与自身声明的节点。

### 4.1 body 语句

```ruby
param :r, default: 1.kohm
node :vin, :vout
resistor       :name, p: <节点>, n: <节点>, value: <量纲>
capacitor      :name, p: <节点>, n: <节点>, value: <量纲>
inductor       :name, p: <节点>, n: <节点>, value: <量纲>
voltage_source :name, p: <节点>, n: <节点>, dc: <电压>, ac: <电压>, waveform: <波形>
current_source :name, p: <节点>, n: <节点>, dc: <电流>, ac: <电流>, waveform: <波形>
diode          :name, p: <节点>, n: <节点>, model: :模型名
model          :name, type: :diode, is: <电流>, n: <数>
instance       :name, of: :子电路, ports: { 端口: 节点 }, params: { 参数: 值 }
for <变量> in <数组或范围> do <语句> end
if <条件> do <语句> [else <语句>] end
```

- 器件 `p`/`n` 端子同时定义**电流正方向 p → n**。
- 二极管 `p` 是阳极，`n` 是阴极。
- `ac:` 的值是 AC 小信号幅度，与 `dc:` 和 `waveform:` 相互独立。
- 节点必须显式 `node` 声明（`:gnd` 除外）。写到未声明节点报 `E_NAME`。

### 4.2 参数

- `param` 只能在 body 内声明，可引用**之前**声明的参数。
- 默认值 `default:` 可省略；省略时必须由实例或实验提供覆盖。
- 参数**按声明顺序**求值，只能引用在它**之前**声明的参数。因此依赖关系必然是有向无环的：环在结构上无法形成。引用尚未声明的参数（包括自引用、前向引用）报 `E_NAME`。`E_PARAM_CYCLE` 在错误码表中保留，但当前没有任何代码路径会产生它。
- 覆盖顺序：**默认值 → 实例或实验覆盖 → 扫描点覆盖**。
- 影响条件、循环次数或连线的参数是**拓扑参数**；扫描拓扑参数报 `E_TOPO_PARAM`。
- `param` 只能出现在 **circuit / subcircuit body 的顶层**（以及 experiment body 顶层的
  `param :r, value: ...` 覆盖）。写在 `for` / `if` 块内报 `E_UNSUPPORTED` 并说明改法——
  参数是设计的一部分，不是控制流里的临时量；需要在分支里取不同值就把条件写进
  `value:` 表达式，或按分支写两条器件语句。
- **覆盖必须命中真实参数**。实例的 `params: { .. }` 与顶层的覆盖链（实验的 `param`、
  扫描点、REPL 的 `:run name=value`）都会校验：名字不在该 body 声明的参数里就报
  `E_NAME`（`` circuit `c` has no parameter `x` `` + `declared parameters: ...`）。
  否则一个拼错的覆盖会什么都不改，却让运行看起来成功了。
- 生成的名字同样可以用作端子：`resistor ("r" + k), p: ("mid" + k), n: ("mid" + (k + 1)), r: ...`
  于是循环可以搭出梯形网络、链式结构这类"节点也是生成出来的"拓扑。
  端子处的名字必须是字符串或符号，求值成数字报 `E_TYPE`；名字非法（含 `.`、为空）报 `E_VALUE`；
  名字指向未声明节点报 `E_NAME`。端子是裸词（`n: out`）时提示写 `:out` 或先声明同名参数。

### 4.3 生成的名称

器件名、节点名与实例名通常是字面量（`:r1` 或 `"r1"`），也可以写成**括号表达式**：

```ruby
for k in 1..3 do
  resistor ("r" + k), p: :a, n: :b, value: k * 1.kohm
end
```

- `+` 在任一操作数是字符串时做拼接；具体能转什么见 §2.4——**带量纲的数不转文本**。
- `str(x)` 把数值、符号或布尔值转成字符串（带量纲的数同样不接受）。
- 生成的名称必须是合法标识符（首字符为字母或下划线，其余为字母、数字、下划线），否则报 `E_VALUE`。
- 同一 body 内生成的名称重复报 `E_DUPLICATE`。循环变量配合拼接是保证名称唯一且确定的常规做法。
- **参数名不能是表达式**，必须字面书写，否则报 `E_TYPE`。

### 4.4 层次实例

- 子电路实例化必须给出全部端口，缺失或多余报 `E_PORT`。
- 实例内部节点与外部隔离；同名节点在不同实例中是不同节点。
- 展开后的名字用 `.` 连接实例路径与局部名：实例 `stage1` 里的 `r1` 是 `stage1.r1`，
  它内部的节点 `internal` 是 `stage1.internal`。诊断与结果表头都用这个名字。
- 递归实例化报 `E_RECURSION` 并显示调用链。

### 4.5 循环与条件

- `for <var> in <array> do` 遍历数组元素。
- `for <var> in <a>..<b> do` 遍历整数区间，**含两端**。
- 循环变量是整数，可在表达式与实例名中使用。
- 循环变量的作用域是**该次迭代的 body**：迭代结束即恢复（若同名参数存在）或消失。
  循环之后引用它会报 `E_NAME`，它不是在循环外留下值的办法。
- 条件必须是布尔值：`if 1 do` 报 `E_TYPE`，没有真值性（truthiness）。
- 动态生成的名称必须唯一，否则报 `E_DUPLICATE`。
- 展开步数、器件数、层次深度受 `Limits` 限制，超限报 `E_LIMIT`。
- 不支持 `while`、递归函数、`break`、`continue`。

## 5. 实验

```ruby
experiment :name, circuit: :电路名 do
  <分析语句>
  save <探针列表>
  param :r, value: 2.kohm      # 实验级参数覆盖
  measure :name, <测量>        # 见 §7
end
```

### 5.1 分析语句

```ruby
op
dc source: :器件名, from: 0.V, to: 5.V, step: 1.V
dc param: :r, from: 1.kohm, to: 5.kohm, step: 1.kohm
ac from: 10.Hz, to: 10.MHz, points_per_decade: 50
ac from: 10.Hz, to: 10.MHz, points: 100          # 线性等分
tran stop: 30.us, max_step: 50.ns
tran start: 10.us, stop: 30.us, max_step: 50.ns
tran stop: 1.ms, max_step: 10.ns, output_interval: 1.us
```

一个实验可声明**多个**分析。同种分析按出现顺序编号：两个 `ac` 分别是 `ac1` 与
`ac2`，导出文件也据此区分，不会互相覆盖。

**DC**：`source:` 与 `param:` 二选一。要求 `step` 非零、方向与
`from`/`to` 一致、点数不超过上限。`to` 不落在步长整数倍上时，
最后一点**不超过** `to`。

**AC**：频率必须为正且 `to > from`。`points_per_decade` 与 `points`
二选一。相位以弧度内部存储，输入输出用度。

**TRAN**：`stop` 必须大于 `start` ≥ 0。瞬态的三个概念互相独立：

- `max_step`（可选）是**求解器的最大内部步长**（适配层把它映射到引擎的 `tmax`）。它不是输出
  间隔：求解器可以取更小的步，返回的时间轴一般**非均匀**。
- `output_interval`（可选）是**输出采样间隔**，只决定用户看到的采样点。它**不进入求解器**：
  求解结束后，原始时间轴被重采样到以原始首点为起点、间隔为 `output_interval` 的网格上
  （完整契约见 §5.3）。省略它时，输出就是求解器自己的时间轴。
- 激励波形由电路里声明的 `pulse(...)`/`sin(...)`/`pwl(...)` 决定，**任何分析参数都不得展宽
  它**。适配层取引擎的 print step = `min(span/1000, 所有源声明的 rise/fall/period 的最小值)`，
  因此引擎对 PULSE 边沿的 `.max(tstep)` 夹取不会落在声明值上，声明 `rise: 1.ns` 就执行 1 ns。

显式写出的值必须合法：

- `max_step` / `output_interval` 为 0、负数或非有限（`NaN`、`inf`）→ `E_VALUE`，**不回退默认值**；
- 声明源的 `rise`/`fall`/`period` 为 0 或非有限 → `E_UNSUPPORTED`（引擎没有理想零宽边沿）；
- 声明的边沿相对仿真窗口过细 → `E_LIMIT`，但只针对**由声明波形导致的**超预算：当"存在已声明的
  `rise`/`fall`/`period`"**且**"没有波形约束时同一 1e6 步预算不会被突破"时，诊断给出所需步数并带
  `declared waveform timing` / `solver step` / `effective step`（如有 `max_step` 再加一项）context。
  **只看 `max_step` 的配置不会被这条拒绝**：`max_step` 是用户自己的请求，例如纯直流源 + RC +
  `tran stop: 1.s, max_step: 1.ns` 在 rev3 实测 `cdsl check` **exit 0**；后端也**不会**为了把运行
  压进预算而静默展宽边沿（没有运行期步数上限，见 §5.3）。

最小示例：

```ruby
tran stop: 1.ms, max_step: 10.ns, output_interval: 1.us
```

含义：积分步长上限 10 ns，输出每 1 µs 一个采样点；两者互不影响（改 `output_interval` 不改变
波形与测量，只改变输出采样）。

### 5.2 探针

```ruby
save v(:vin), v(:vout), v(:a, :b), i(:input)
```

- `v(:a)` 是节点 `a` 对地的电压。
- `v(:a, :b)` 定义为 `Va - Vb`。
- `i(:dev)` 是器件电流，正方向为器件的 `p → n`。
- 探针名可以是字面符号、层次路径或字符串：`v(:out)`、`v(:stage1.internal)`、`v("stage1.r1")`
  三种写法都行，后两种指的是同一个东西。
- 叶名（不带 `.` 的局部名）在**唯一**时可以直接用：只有一个实例里有 `internal` 时
  `v(:internal)` 等价于 `v(:stage1.internal)`。一旦有两个实例都叫这个名字，就是
  `E_NAME`：`` `internal` names 2 different nodes ``，并列出应改写的完整路径——
  不会静默挑一个。
- 探针引用未声明的节点或器件报 `E_NAME`；重复探针报 `E_DUPLICATE`。

### 5.3 输出采样与输出网格（`output_interval`）

`output_interval:` 只在给出时启用，且只作用于 `tran`。求解器交出它自己的时间轴，输出视图在
**求解之后**由独立的重采样得到（实现：`circuit-results` 的 `resample` 模块）：

| 项 | 规则 |
|---|---|
| 网格起点 | 原始（求解器）时间轴的首点 `t0`，值直接复制 |
| 内部点 | `t0 + k·output_interval`（`k = 1, 2, …`），只保留严格小于原始末点者 |
| 终点 | 原始末点 `t_last` **恒保留**，因此最后一段可能短于 `output_interval` |
| 插值 | 相邻两个原始样本上的线性插值；首末点取原值 |
| 外推 | 禁止：每个输出点都落在 `[t0, t_last]` 内 |
| 规模 | 输出值总数（输出点数 × 信号数）超过 `Limits::max_result_values` → `E_LIMIT`，不截断、不抽样 |
| 省略时 | 输出网格 = 原始求解网格，逐点原样 |
| 退化轴 | 原始点少于 2 个时无可插值，输出保持原样 |

三条可依赖的性质：

1. **改 `output_interval` 不改变激励波形与物理解**：它不进入求解器，不影响 PULSE 的
   `rise`/`fall`，也不会延长或缩短仿真窗口。
2. **改 `output_interval` 不改变测量**：`avg`/`rms`/`max`/`min` 一律在**原始求解网格**上计算
   （§7）；重采样只改变展示与导出的采样点。
3. **文件模式与 REPL 一致**：两者共用同一条执行路径，导出与摘要都用输出网格。结果元数据里
   两种数据可区分——`cdsl run --format json` 的 `backend.settings` 记录
   `tran.solver_step`、`tran.solve_points`、`tran.waveform_bound`、`tran.max_step`，以及
   重采样层的 `tran.output_grid = resampled-linear`、`tran.output_points`。

例：`tran stop: 10.us, max_step: 10.ns, output_interval: 100.ns` 返回约 101 个输出点；
把 `output_interval` 改成 `10.ns` 只是把同一条解画得更密，`measure :vavg, avg: v(:out)`
的值不变。

**`check` 与 `run` 的边界**（实测）：`cdsl check` 是静态的 parse/elaborate，不求解、不重采样，
因此**重采样规模的 `E_LIMIT` 只有 `run` 会报**；而 `max_step`/`output_interval` 的 `E_VALUE`、
声明边沿的 `E_UNSUPPORTED`、以及**由声明波形（`rise`/`fall`/`period`）导致的**步数预算 `E_LIMIT`
在 `check` 阶段即报。`max_step` 本身造成的步数超预算**不会**被拒绝（用户显式请求）。
不要把它读成"`check` 能捕获全部运行时限制"。

**运行期没有步数上限（既有限制）**：本项目不为求解过程设步数保护。即使 `check` 通过，极小的
`max_step` 配合长窗口也可能需要极多求解步（例如纯直流源 + RC + `tran stop: 1.s, max_step: 1.ns`
约 1e9 步），运行时间可能非常长；`Limits::max_result_values` 只在结果生成后生效，不阻止求解本身。
该组合的**实际运行时长未实测**（修复前同样没有该保护，不是本轮引入）。

## 6. CLI 与结果

```
cdsl check <file> [--verbose]
cdsl run <file> --experiment <name> [--out <dir>] [--format csv|json|both]
cdsl repl [<file>]
cdsl capabilities
cdsl --version
```

- 退出码：成功 0，用户错误 1，内部错误 2。
- 诊断走 stderr，数据与摘要走 stdout。
- `cdsl repl` 是交互式会话：表达式即时求值、`name = value` 定义会话变量、可以逐条
  定义电路/子电路/实验并 `:run`。**赋值与会话命令只在 REPL 里存在**，`.cdsl` 文件
  的语法不受影响。完整说明见 `docs/repl.md`。
- 管道输入时（stdin 不是终端）REPL 逐行读取、不启用行编辑，有输入报错则以 1 退出，
  因此一段会话可以写进脚本或测试。
- CSV 的复数列拆成 `_re` / `_im` 两列。
- JSON 保留单位、轴类型与后端元数据。
- 非有限值导出为 `null`（JSON）与空字段（CSV），并给出警告。

结果中的单位与轴：

| 分析 | 轴 | 数据类型 |
|---|---|---|
| OP | 无 | 实数 |
| DC | 扫描参数（实数） | 实数 |
| AC | 频率 Hz（对数或线性） | 复数 |
| TRAN | 时间 s（非均匀；给了 `output_interval` 则是等间隔网格，末点保留） | 实数 |

## 7. 测量

```ruby
measure :vmax, max: v(:out)
measure :vmin, min: v(:out)
measure :vavg, avg: v(:out)
measure :vrms, rms: v(:out)
```

- `avg` 与 `rms` 在**非均匀时间轴**上按时间积分计算，不是样本算术平均：
  - `avg = ∫x dt / ∫dt`
  - `rms = sqrt(∫x² dt / ∫dt)`
- `max` / `min` 取样本极值。
- 对没有时间轴的分析使用 `avg`/`rms` 报 `E_TYPE`。
- **一个实验可以声明多个分析**，同一探针可能同时存在于多个结果里。测量的选取规则是：
  取**能支持该测量的最丰富分析**，优先级依次为 `tran` > `ac` > `dc` > `op`。
  否则 `max: v(:out)` 会取到工作点的 0 V 而不是瞬态峰值。

## 8. 诊断

格式：

```text
error[E_DIMENSION]: resistor.value 需要电阻量纲，实际为时间
  --> examples/filter.cdsl:8:48
   |
 8 | resistor :r1, p: :vin, n: :out, value: 10.ms
   |                                         ^^^^^
   = instance: top.filter.r1
   = expected: ohm
   = received: s
```

错误码见 `crates/circuit-core/src/diagnostic.rs` 的 `Code`。

**浮空节点**：展开结束时会做一次直流参考通路检查（`circuit-core` 的 `connectivity` 模块）。
判据是"能否经**直流导通**的器件到达地"，而不是"图上是否有连线"——电容与独立电流源
在直流下不导通，因此不构成参考通路。导通器件为电阻、电感、电压源与二极管，
阻断器件为电容与独立电流源。两类问题分别报出：

- 节点没有任何器件连接：`node ... is declared but nothing connects to it`
- 节点只有电容或电流源连接：`node ... has no DC path to ground, ...`

这样做是必要的：实测（见 `docs/review-evidence/floating-audit.md`）表明引擎对真无参考的线性网络
会以 `matrix is singular, cannot solve` 失败，但该错误不指向任何节点，也无法区分合法开路输出；
前端因此自己做可达性检查，给出定位到节点与阻断器件的 `E_NAME`。
（旧文档曾写「引擎的 gmin 处理会让节点取到看似正常的有限值」，该说法在本轮线性电路实测中不成立，已废弃。）

无法定位时保留后端原始信息，不编造故障器件。

## 9. 数值与安全边界

- 内部数值统一为 SI 基本单位。
- 普通 R/L/C 要求**严格正值**；零值与负值报 `E_VALUE`，
  不会替换成很小的正数。
- 瞬态选项必须显式合法：`max_step` / `output_interval` 为 0、负数或非有限报 `E_VALUE`，
  **不回退默认值**；`output_interval` 只影响输出采样（§5.3），不进入求解器。
- 声明的源边沿必须可执行：`rise`/`fall`/`period` 为 0 或非有限报 `E_UNSUPPORTED`；
  **由声明波形导致的**步数预算超限（需要超过 1e6 个求解步）报 `E_LIMIT`，诊断给出所需步数与
  归因 context；纯 `max_step` 造成的步数不会被这条拒绝（用户显式请求），运行期也没有步数上限
  （§5.3）。不静默展宽边沿、不静默截断输出。
- DSL 没有文件读写、网络、外部命令权限。
- 首期不支持 `include` 与模型文件。
- 仿真过程中不执行 DSL 脚本；展开完成后拓扑固定。

## 10. 尚未实现 / 未验证

以下**不是**本版本的能力，不得在文档或 CLI 中声称支持：

- MOSFET、BJT、受控源、行为源、开关、互感。
- SPICE 网表导入/导出。
- 噪声、灵敏度、Monte Carlo、优化、参数拟合。
- 多参数联合扫描（仅单参数）。
- `while`、递归、用户自定义函数。
- 复数的完整结果表达式（如任意传递函数表达式）。
- `include` 与外部模型文件。
- REPL 的语法高亮、多行编辑、`:save` 回写文件、跨会话持久化（历史文件除外）。
- 文件模式下的赋值：`name = value` 只属于 REPL，文件里用 `param`。
- `uic`（跳过工作点）——后端路径未验证。
- **运行期没有步数保护**：极小 `max_step` 配合长窗口可以通过 `check`（`max_step` 是用户显式请求，
  不由波形契约拒绝），实际求解步数可能极多、运行时间非常长；该组合的运行时长**未实测**（§5.3）。
- **运行中源断点的精度不是普适保证**：引擎在源断点后的首个被接受步强制 Backward-Euler
  重启（`h1 = min(2·h_before, h_max)·0.1`），局部误差 ≈ `(V0/T)·h1²/(2τ)`。实测（τ = 100 µs、
  上升斜坡 `V0/T = 1e6 V/s`、§17 判据 `atol = 1e-5 V` / `rtol = 1e-3`）：`max_step = τ/1000`
  → 3129 点 0 超限、`τ/500` → 1629 点 0 超限；`τ/200` → 3/729 超限（max 1.248959e-5 V）；
  `τ/50` → 250/309 超限（max 7.331775e-4 V）。经验界
  `h_max ≤ 10·sqrt(2·atol·τ·T/V0)` 只对该激励推导，**不是通用保证**。产品路径没有容差通道
  （`RELTOL`/`ABSTOL`/`TRTOL` 不可达）。详见 `docs/backend-evaluation.md` §5.1。
- 跨平台：仅在 Windows MSVC 上构建验证过。
