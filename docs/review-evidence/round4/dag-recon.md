# 阶段 B 前置只读取证：参数作用域、覆盖优先级与拓扑使用点（task-6）

- 执行者：DAG explorer（只读角色）。写集只有本文件 `docs/review-evidence/round4/dag-recon.md`。
- 仓库：`F:/codexprojects/dsl000`（Windows + PowerShell，下文相对路径均以仓库根为准）。
- 目的：为 `docs/round3-review-and-round4-plan.md` §5.1「先冻结语义」六个问题提供 **file:line 级代码取证**，
  并标出「计划假设但代码并未实现」的点。本文是契约输入，凡代码不能证明的一律写「无法取证/未实现」。

## 0. 取证方法与边界

静态取证（逐行阅读，非 grep 推断）：

| 文件 | 作用 |
|---|---|
| `crates/circuit-dsl/src/elaborate.rs` | scope 收集、push_overrides、stmt_param、实例覆盖、for/if 展开、分析参数 |
| `crates/circuit-dsl/src/eval.rs` | 名称解析、未声明名称诊断、字符串拼接 |
| `crates/circuit-session/src/session.rs` | :run 覆盖、会话变量、事务性提交 |
| `crates/circuit-session/src/execute.rs` | 覆盖链进入展开的入口、参数扫描驱动 |
| `crates/circuit-backend/src/sweep.rs`、`thevenin.rs` | 运行期拓扑不变性检查（唯一的 E_TOPO_PARAM 生产者） |
| `crates/circuit-core/src/plan.rs`、`diagnostic.rs` | SweepTarget / param_overrides 身份、错误码 |
| 各 `tests/*.rs`、`docs/language.md`、`docs/repl.md`、`docs/architecture.md` | 已固定的兼容行为与文档化语义 |

动态取证（本机实测，命令与逐条观察见 §4）：

- `cargo test -p circuit-dsl --test elaborate` → `67 passed; 0 failed`，exit 0。
- `cargo test -p circuit-cli --test repl` → `13 passed; 0 failed`，exit 0。
- 之后用 `target/debug/cdsl.exe repl`（该二进制由上面第二条命令从当前源码重建，重建时间本机时钟 2026/9/18 23:42:40）
  从 **stdin** 喂探针脚本。**全程没有创建任何探针文件**，也就没有越出写集。

并发提示：取证期间 `crates/circuit-core/src/plan.rs`（23:42:02）、`crates/circuit-dsl/src/eval.rs`（23:37:42）、
`crates/circuit-session/src/session.rs`（23:42:37）正在被其他代理修改；`crates/circuit-dsl/src/elaborate.rs`
最后修改时间为 22:15:06，未被本轮改动。本文所有作用域/覆盖结论以 elaborate.rs 为准，session/execute 的结论
以读到的当前内容为准并在引用处标注。

---

## 1. 证据表（问题 → 结论 → file:line → 代码摘录）

### Q1 参数声明如何被收集、作用域身份是什么、名称解析顺序如何

**结论（一句）**：参数声明只在「body 顺序执行」时被收集进一个 **以裸字符串为 key 的 Scope**；
每个顶层电路展开一份、每个实例各一份，但**参数身份本身没有任何实例/实验前缀**；解析只查该 scope，
没有节点/器件/函数回退。分析参数（ac/tran/dc）与结果表达式（derive/measure）都**不在**参数作用域内。

| # | 结论 | file:line | 代码摘录 |
|---|---|---|---|
| 1.1 | 文件级编译：每个非 subcircuit 顶层电路用**空覆盖链**展开；实验则先算 experiment_overrides 再展开它命名的电路 | `crates/circuit-dsl/src/elaborate.rs:93-121` | `if let Some(c) = el.elaborate_top_circuit(def, &[]) {…}` / `let overrides = el.experiment_overrides(def); el.elaborate_named_circuit(&def.circuit, &overrides)` |
| 1.2 | 单实验入口：实验自身 param 覆盖 + 调用方覆盖（扫描点/会话）合成一条链，**后者替换同名前者** | `elaborate.rs:152-158` | `let mut all = el.experiment_overrides(def);` / `all.retain(\|(n, _, _)\| n != name); all.push((name.clone(), *value, def.circuit.span));` |
| 1.3 | 每个顶层电路展开开始时新建一个 scope，并把覆盖链**预灌**进去，然后才跑 body | `elaborate.rs:429-434` | `let mut scope = Scope::default(); self.push_overrides(&mut scope, overrides); … self.run_body(&def.body, &mut scope, &mut top_ctx)` |
| 1.4 | push_overrides 就是逐条 scope.set，同名**后写者胜**（无去重、无维度校验、无存在性校验） | `elaborate.rs:620-624` | `for (name, value, span) in overrides { scope.set(name, *value, *span); }` |
| 1.5 | scope 的身份是**裸字符串 key**：vars: HashMap<String, Quantity>；另有 spans / declared / defaults | `elaborate.rs:192-202` | `struct Scope { vars: HashMap<String, Quantity>, spans: HashMap<String, SourceSpan>, declared: HashSet<String>, defaults: HashMap<String, Quantity> }` |
| 1.6 | 求值器看到的只有这张表：lookup → vars.get，declared_span → spans.get | `elaborate.rs:204-230` | `fn get(&self, name: &str) -> Option<Quantity> { self.vars.get(name).copied() }` |
| 1.7 | **参数名不做限定**。实例路径只用于节点/器件名（Bodies.prefix + qualify），参数表里没有前缀 | `elaborate.rs:3407-3418`、`elaborate.rs:3431-3438` | `fn qualify(&self, name: &str) -> String { if self.prefix.is_empty() { name.to_string() } else { format!("{}.{}", …) } }` |
| 1.8 | 实例隔离靠「**每个实例一个新 Scope::default()**」，不是靠限定名；子电路默认值在这个只有子电路自身参数的 scope 里求值 | `elaborate.rs:1313-1338` | `let mut inner = Scope::default(); … match self.eval(expr, &mut inner) { Ok(Value::Num(q)) => inner.set(&p.name.name, q, p.name.span), … }` |
| 1.9 | 实例 params: 的值在**外层 scope 的克隆**里求值（所以能引用父参数），逐条接受、后者可引用前者；维度与默认值比对在覆盖处完成 | `elaborate.rs:1345-1396` | `let mut eval_scope = scope.clone();` … `if let Some(prev) = previous && prev.dimension != q.dimension { … Code::Dimension … }` |
| 1.10 | 解析顺序：ExprKind::Var 只查 Variables::lookup，未命中即 E_NAME，**没有节点/器件/函数回退** | `crates/circuit-dsl/src/eval.rs:134-147` | `ExprKind::Var(name) => match vars.lookup(name) { Some(q) => …, None => { … Diagnostic::error(Code::Name, format!("`{name}` is not declared")) … } }` |
| 1.11 | 端子位置的**唯一例外**：:sym 直接当名字；写成裸词时，若不在 scope.vars 里就报 E_TYPE 并提示写 :v 或先 param :v；否则按表达式求值成名字 | `elaborate.rs:1036-1058` | `ast::ExprKind::Var(v) if !scope.vars.contains_key(v) => { … "@{arg_name}: takes a node symbol" … with_note(format!("write :{v} for the node, or declare param :{v} first")) }` |
| 1.12 | 裸名解析顺序是**上下文无关**的：先参数表，别无其他；节点必须在 resolve_node 里以 :sym/计算名出现 | `elaborate.rs:1064-1093`、`docs/language.md:174-182` | `表达式中的标识符只能是：已声明的参数名 … 未声明的标识符一律报 E_NAME` |
| 1.13 | ac 的数值参数在**空 scope** 里求值 → 参数不可用 | `elaborate.rs:2960-2966` | `let mut scope = Scope::default(); let start = self.req_quantity(call, "from", FREQUENCY, &mut scope)?;` |
| 1.14 | tran 同理（空 scope） | `elaborate.rs:3056-3057` | `let mut scope = Scope::default(); let stop = self.req_quantity(call, "stop", TIME, &mut scope)?;` |
| 1.15 | dc 的 from/to/step/points 同理（空 scope）；且 param: 只接受**符号**，不求值 | `elaborate.rs:3215-3232`、`elaborate.rs:3243-3263` | `let Some(sym) = a.value.as_symbol() else { … "param: takes a parameter symbol" … }` / `let mut scope = Scope::default();` |
| 1.16 | 结果表达式里裸名**不是**参数，而是 E_TYPE「is not a signal」→ 参数与 derive/measure 目前是两个世界 | `elaborate.rs:2404-2415` | `ExprKind::Var(name) => { self.error(Diagnostic::error(Code::Type, format!("`{name}` is not a signal")) … }` |
| 1.17 | 会话变量是**另一个命名空间**（BTreeMap），且明确不进入电路展开 | `crates/circuit-session/src/session.rs:88-90`、`session.rs:9-23` | `vars: BTreeMap<String, Quantity>, …` / 模块文档：`Elaboration starts from an empty parameter scope` |
| 1.18 | 顶层覆盖链的**名字**存在性会被校验（body 跑完之后），列表来自 scope.declared | `elaborate.rs:442-488` | `self.check_overrides_are_declared(def, &scope, overrides);` … `if scope.declared.contains(name) { continue; } … format!("circuit `{}` has no parameter `{name}`", def.name)` |
| 1.19 | 但**维度不被校验**：Scope::defaults 只在 stmt_param 里写入，全仓没有任何读取点（对照：实例覆盖在 1378-1394 校验维度） | `elaborate.rs:199-201`、`elaborate.rs:717` | `defaults: HashMap<String, Quantity>,` / `scope.defaults.insert(p.name.name.clone(), q);`（grep defaults：仅此 2 处写入，无读取） |

### Q2 重复、遮蔽、实例覆盖、会话覆盖的优先级；「生效定义」在哪一步被选定

**结论（一句）**：优先级 = **默认值 → 实例 params: → 实验 param → 扫描点覆盖 →（会话 :run name=expr 作为扫描点的同一条链尾）**，
生效值在 stmt_param 执行的那一刻选定（**body 顺序执行期**、device 参数求值之前）；
但**被覆盖的 default 表达式仍然会被求值**（只是不安装），所以「坏 default」会照样报错。

| # | 结论 | file:line | 代码摘录 |
|---|---|---|---|
| 2.1 | 覆盖语义就落在 stmt_param 的一行判断上：vars 里已有值 = 别人的覆盖生效，default 只记录维度、不安装 | `elaborate.rs:714-724` | `scope.defaults.insert(p.name.name.clone(), q);` `// An override already supplied a value; the default only records the expected dimension.` `if scope.vars.contains_key(&p.name.name) { return; } scope.set(&p.name.name, q, p.name.span);` |
| 2.2 | 同一 body 里重复声明同一参数 = E_DUPLICATE，并回指第一处 | `elaborate.rs:692-702` | `if !scope.declared.insert(p.name.name.clone()) { … Code::Duplicate, format!("parameter `{}` is declared twice", …) … .with_secondary(scope.span(&p.name.name), "first declared here") }` |
| 2.3 | 模块注释明确写了链序（default → instance/experiment → sweep point） | `elaborate.rs:177-191` | `The override order from brief §5.3 (default -> instance/experiment -> sweep point) falls out of this: a default is only installed when nothing has already provided a value.` |
| 2.4 | 单实验入口处再次强调「后者胜」＝ successive refinement | `elaborate.rs:153-157` | `Later entries win, which is what "successive refinement" means for the override chain default -> instance/experiment -> sweep point.` |
| 2.5 | 扫描点覆盖在驱动里构造：先删同名，再 push (parameter, value) | `crates/circuit-session/src/execute.rs:563-575` | `let mut overrides = base_overrides.to_vec(); overrides.retain(\|(n, _)\| n != parameter); overrides.push((parameter.to_string(), Quantity::new(value, dimension))); circuit_dsl::elaborate_experiment(program, experiment, &overrides, &limits)?` |
| 2.6 | 实验 param 覆盖：experiment_overrides 内**同名替换**（最后者胜，静默） | `elaborate.rs:1988-2001` | `if let Some(existing) = out.iter_mut().find(\|(n, _, _): &&mut (String, Quantity, SourceSpan)\| *n == name.name) { existing.1 = q; existing.2 = *span; } else { out.push((name.name.clone(), q, *span)); }` |
| 2.7 | 同一份实验 body 在 elaborate_experiment 里被**再求值一次**且**不去重**（两条同名都会进 param_overrides） | `elaborate.rs:2035-2046`、`elaborate.rs:2208` | `if let Ok(Value::Num(q)) = self.eval(value, &mut scope) { overrides.push((name.name.clone(), q, *span)); scope.set(&name.name, q, name.span); }` … `param_overrides: overrides,` |
| 2.8 | AnalysisPlan 把生效覆盖作为结果元数据保存，注释直接写明链序 | `crates/circuit-core/src/plan.rs:674-676` | `Parameter overrides declared on the experiment itself, applied after subcircuit defaults and before sweep points (brief §5.3).` `pub param_overrides: Vec<(String, Quantity, SourceSpan)>,` |
| 2.9 | 会话覆盖：:run <exp> name=expr；在**会话变量作用域**求值一次并定值，要求是数字 | `session.rs:484-564` | `let v = eval::eval(&value, &self.var_view())?; let Some(q) = v.as_num() else { … "override `{name}` must be a number" … }; fixed.push((name, q));` |
| 2.10 | 会话 run 把定值覆盖交给同一个 elaborate_experiment —— **CLI / REPL / 扫描点共用同一条覆盖链实现** | `session.rs:587-594` | `let request = RunRequest { program: &program, experiment, overrides, limits: &self.limits };` … `let outcome = execute::execute(&request, &mut self.backend, &self.sources)?;` |
| 2.11 | 实例覆盖：只有子电路 body 里 param 声明过的键才被接受（seen_defaults），否则 E_NAME + 已声明列表 | `elaborate.rs:1351-1372` | `if !seen_defaults.contains(&entry.key) { … Code::Name, format!("`{def_name}` has no parameter `{}`", entry.key) … }` |
| 2.12 | 无遮蔽：param 不能在 for/if 块里声明（E_UNSUPPORTED）；文档写明每份电路定义「只有一个参数命名空间」 | `elaborate.rs:667-679`、`docs/repl.md:85-94` | `if self.block_depth > 0 { … Code::Unsupported, "`param` declares a design parameter, so it belongs in the circuit body" … }` |
| 2.13 | 唯一的「遮蔽」是循环变量：写入同一张 vars 表、迭代后恢复（spans 会被循环变量覆盖，但值正确恢复） | `elaborate.rs:1543-1573` | `let previous = scope.get(&f.var.name); … scope.set(&f.var.name, q, f.var.span);` … `match previous { Some(p) => scope.set(&f.var.name, p, f.var.span), None => { scope.vars.remove(&f.var.name); } }` |
| 2.14 | 覆盖必须命中真实参数（顶层链），失败 = E_NAME + declared parameters: … | `elaborate.rs:459-488` | `for (name, _, span) in overrides { if scope.declared.contains(name) { continue; } self.error(… "an override that names nothing would silently leave every value at its default" …) }` |

**「生效定义在 lowering 之前还是之后」的精确回答**：在 stmt_param 处（body 顺序执行期间，
即每次 device 语句求值**之前**就选定并写入 scope.vars）。它不是「先建图再选定义」，而是
「先灌覆盖链，再顺序执行 body，遇到 param 时决定安装默认值与否」。结果表达式 lowering
（lower_result_expr，`elaborate.rs:2359`）完全不参与参数定义选择 —— 参数在结果表达式里根本不是可解析的名字（见 1.16）。

### Q3 今天写一个前向引用（先引用后定义）会发生什么

**结论（一句）**：**一律 E_NAME（`x` is not declared`），主标签在引用处**，且这是「与未知名称同一条代码路径」，
无法区分；E_PARAM_CYCLE 当前**没有任何生产者**。

| # | 场景 | 结论 | file:line |
|---|---|---|---|
| 3.1 | 顶层 `param :b, default: 2 * a` 而 `a` 在下一行 | E_NAME at 引用处；body 首错即停 | `elaborate.rs:631-639`（`if self.error_count > 0 { break; }`）、`elaborate.rs:714-736`、`eval.rs:134-147` |
| 3.2 | 自引用 `param :a, default: a` | 同一路径；spans 已有该名字，所以还会带上 secondary「a parameter with this name is declared here」 | `elaborate.rs:706`（先插 span）、`elaborate.rs:714`（后求值）、`eval.rs:142-144` |
| 3.3 | 实验 body 内 `param :x, value: y` 而 `y` 在后面 | E_NAME at `y`；随后还会额外报 `circuit \`c\` has no parameter \`y\``（因为 `y` 作为一条覆盖被推入顶层链） | `elaborate.rs:1988-2001`、`elaborate.rs:459-488` |
| 3.4 | 实例 `params: { r: later }` 而父电路 `param :later` 在该 instance 语句之后 | E_NAME at `later`（eval_scope 是**当时**的 scope.clone()） | `elaborate.rs:1345-1349`、`elaborate.rs:1375` |
| 3.5 | 子电路默认值引用**父**参数（哪怕父参数已声明） | E_NAME：子电路默认值只在 inner（空 scope + 本子电路更早的参数）里求值 | `elaborate.rs:1313-1338` |
| 3.6 | 依赖环诊断 | **不存在**：Code::ParamCycle 只在枚举与字符串映射里出现，全仓无构造点；`docs/language.md:255` 已写明「环在结构上无法形成」 | `crates/circuit-core/src/diagnostic.rs:61-62`、`diagnostic.rs:96`、`docs/language.md:255` |
| 3.7 | 已固定的回归测试 | `a_forward_parameter_reference_is_rejected`、`a_self_referential_parameter_is_rejected`、`parameters_are_resolved_in_declaration_order` | `crates/circuit-dsl/tests/elaborate.rs:405-445` |

### Q4 哪些 AST/IR 构造把参数放在「拓扑位置」（只列代码里真实存在的）

**结论（一句）**：今天真实存在的拓扑使用点只有三类 —— **if 条件**、**for 迭代源（列表/范围）**、
**计算出来的名字（device / node / instance 名与器件端子）**。参数的**取值位置**（`value:` / `dc:` / `ac:` / `waveform:` / model args /
实例 params: 值）本身不是拓扑位置，只有当它的值回头喂给上面三类时才成为拓扑依赖。
`dc param:` 的扫描目标只接受**符号**，不使用参数表达式；ac/tran/dc 的数值参数根本无法引用参数。

| # | 构造 | 拓扑性质与代码位置 | 代码摘录 |
|---|---|---|---|
| 4.1 | if 条件（Stmt::If → stmt_if）：用当前 scope 求值，决定展开哪一支 | 决定存在哪些语句 → 节点/器件集合 → 拓扑；条件必须是 Bool，否则 E_TYPE | `elaborate.rs:1577-1611`：`match self.eval(cond, scope) { Ok(v) => match v.as_bool() { Some(true) => { … self.run_body(body, scope, ctx); return; } … } }` |
| 4.2 | for 的**列表**迭代源（ForIter::List） | 迭代次数 = 展开份数 → 拓扑 | `elaborate.rs:1456-1479`：`ForIter::List(expr) => match self.eval(expr, scope) { Ok(v) => match v.as_array() { … } }` |
| 4.3 | for 的**范围**边界（ForIter::Range）：两端必须是**无量纲整数** | 同上；范围可以是参数（如 `1..taps`），实际展开数决定拓扑 | `elaborate.rs:1480-1525`：`let (a, b) = match (self.eval(start, scope), self.eval(end, scope)) {…}` … `if !a.dimension.is_dimensionless() \|\| !b.dimension.is_dimensionless() { … Code::Dimension, "a loop range must be dimensionless" … }` |
| 4.4 | **计算名**（device/node/instance 名）：resolve_name 对括号表达式求值，要求名字是字符串/符号且是合法标识符 | 生成的名字进入节点表/器件表 → 拓扑 | `elaborate.rs:376-415`；`elaborate.rs:766`（ctx.qualify）、`elaborate.rs:793`（device 名）、`elaborate.rs:1236`（instance 名） |
| 4.5 | **器件端子**：node_arg → resolve_name → resolve_node | 连线 → 拓扑 | `elaborate.rs:1036-1059`、`elaborate.rs:1064-1093` |
| 4.6 | 参数的**取值位置**（不是拓扑位置本身）：无源器件 `value:`、源 `dc:`/`ac:`、`waveform:`、model 参数 | 只影响数值；若其值再参与 4.1–4.5 才成为拓扑依赖 | `elaborate.rs:867-889`、`elaborate.rs:900-905`、`elaborate.rs:1745-1751`、`elaborate.rs:1118-1180` |
| 4.7 | 实例 params: 值：在外层 scope 求值、可在实例内部继续影响 4.1–4.5 | 是参数的**跨层通道**，也是「拓扑依赖沿层级传播」的唯一现成路径 | `elaborate.rs:1345-1397`、`elaborate.rs:1442`（`self.run_body(&def.body, &mut inner, &mut inner_ctx)`） |
| 4.8 | `dc param: :name`：目标只是**符号**，进入 SweepTarget::Parameter { name: String } | 参数名是**跨作用域 ID 的裸字符串用法**；运行时按字符串比较删除同名覆盖 | `elaborate.rs:3215-3232`、`plan.rs:406-414`、`execute.rs:566-568` |
| 4.9 | 拓扑参数扫描的**唯一防线是运行期**：每个扫描点重新展开后比较（节点集合，器件名/种类/排序端子） | check 看不到；validate 对参数扫描直接放行 | `crates/circuit-backend/src/sweep.rs:31-61`、`sweep.rs:227-249`、`crates/circuit-backend/src/thevenin.rs:200-202`（`SweepTarget::Parameter { .. } => {}`）、`crates/circuit-cli/src/check.rs:26-77` |
| 4.10 | 计划提到的其它语法（「数量展开」之外的新构造） | **不存在**：Stmt 只有 Param/Node/Device/Model/Instance/For/If 七种；没有 while、没有数组长度声明、没有条件元件 | `crates/circuit-dsl/src/ast.rs:94-104` |
| 4.11 | 「参数影响分析规模」 | **不可表达**：ac/tran/dc 的数值参数在空 scope 求值（见 1.13–1.15）；实测 `points: n` 报 E_NAME | 同 1.13–1.15 |

### Q5 REPL 失败事务性：失败的参数更新之后会怎样

**结论（一句）**：**REPL 是事务式的、且 :run 根本不写会话状态** —— 失败的覆盖求值 / 未知覆盖 / 失败运行之后，
circuits/experiments/vars 一字未改，下一次成功运行给出与失败前**完全相同**的结果（实测）。

| # | 结论 | file:line | 代码摘录 |
|---|---|---|---|
| 5.1 | Session::run 全程不修改 self 的定义/变量，只把 &mut self.backend 借给执行器 | `session.rs:567-659` | `let program = self.program(); … let outcome = execute::execute(&request, &mut self.backend, &self.sources)?;` |
| 5.2 | 覆盖表达式求值失败 → 直接 Err，还没进入运行 | `session.rs:533-561` | `let tokens = circuit_dsl::lex(source, text)?; … let v = eval::eval(&value, &self.var_view())?;` |
| 5.3 | 会话变量赋值是「先校验后写」：求值 → 必须数字 → 必须有限 → 才 insert | `session.rs:240-264` | `if !q.is_finite() { return Err(… "a variable must be a finite number" …); } self.vars.insert(name.clone(), q);` |
| 5.4 | 定义替换是「先编译候选状态，成功才提交」 | `session.rs:273-311` | `circuit_dsl::compile(&candidate, &self.limits).map_err(…)?; self.circuits = circuits; self.experiments = experiments;` |
| 5.5 | :run 覆盖值本身**不做有限性检查**（与 5.3 不对称）：非有限值会进入 scope.vars，最终在下游 device 处报 E_VALUE | `session.rs:545-560` vs `session.rs:252-257` | 覆盖路径：`let Some(q) = v.as_num() else { … }; fixed.push((name, q));`（无 is_finite） |
| 5.6 | 测试断言「会话不被一次运行修改」 | `crates/circuit-session/tests/session.rs:399-402` | `And the session is unchanged: the next run without an override is back to the default, because a run never edits the definitions.` `let again = s.run("divider", &[], None).unwrap(); assert!((v_out_of(&again) - 2.5).abs() < 1e-9);` |
| 5.7 | 测试断言失败 run 不写文件、不打印失败 measure、会话继续 | `crates/circuit-cli/tests/repl.rs:302-347` | `assert!(!text.contains("wrote"), "nothing may be written"); … assert!(text.contains("x = 2"), "The session carried on after the failed run.");` |

### Q6 现有测试已固定的兼容行为

见第 2 节清单（每条含测试名 + 断言 + 位置）。

---

## 2. 阶段 B 必须遵守的既有行为清单（不得破坏）

> 每条给出「测试名 / 文档」与断言要点。标 `[实测]` 的是我本轮用真实 CLI/REPL 观察过的。

### A. 作用域与可见性

| 编号 | 必须保持 | 证据 |
|---|---|---|
| A1 | 参数必须**先声明后引用**；未声明名字报 E_NAME 且带 note「an undeclared name is never treated as a node, device or function call」 | `an_undeclared_parameter_is_an_error` `elaborate.rs:385-394`；`eval.rs:137-141`；`[实测]` |
| A2 | 声明顺序求值、可引用更早参数 | `parameters_are_resolved_in_declaration_order` `elaborate.rs:405-421`（断言 r1.value == 2000 Ω） |
| A3 | 自引用报 E_NAME（不是环） | `a_self_referential_parameter_is_rejected` `elaborate.rs:424-432` |
| A4 | 前向引用报 E_NAME | `a_forward_parameter_reference_is_rejected` `elaborate.rs:435-445`（同时断言 "not declared"）；`[实测]` |
| A5 | param 不得出现在 for/if 块里 → E_UNSUPPORTED + 两句改法提示 | `a_parameter_cannot_be_declared_inside_a_block` `elaborate.rs:1466-1480` |
| A6 | 循环变量不逃逸；可以复用外层参数名并在循环后恢复原值 | `a_loop_variable_does_not_escape_its_loop` `elaborate.rs:1485-1498`；`a_loop_variable_may_reuse_a_name_from_the_enclosing_body` `elaborate.rs:1501-1520`（断言循环后 k == 5000 Ω） |
| A7 | 会话变量不进电路；电路参数不是会话变量 | `a_session_variable_does_not_leak_into_a_circuit` `session.rs(tests):343-354`；`a_circuit_parameter_is_not_visible_to_the_session` `session.rs(tests):357-363`；`a_variable_never_reaches_a_circuit` `repl.rs(tests):258-272`；`docs/repl.md:198-208` |

### B. 覆盖链与优先级

| 编号 | 必须保持 | 证据 |
|---|---|---|
| B1 | default → 实例/实验覆盖 → 扫描点；后者胜 | `elaborate.rs:153-157`、`elaborate.rs:177-191`、`plan.rs:674-676` |
| B2 | 实例覆盖按实例生效、互不污染；未覆盖的实例保留默认值 | `subcircuit_instances_are_isolated_and_parameters_override` `elaborate.rs:469-558`（stage1 r=2 kΩ / stage2 r=1 kΩ）；`[实测]` |
| B3 | 实验 param 覆盖电路默认值；扫描点又覆盖实验 param | `[实测]`：实验 `param :r, value: 2.kohm` + `dc param: :r, 0.5k..2k` → `measure big = 2.25 V`（= 3·1.5/(0.5+1.5)，即 r=0.5 kΩ 的点胜出） |
| B4 | 覆盖必须命中已声明参数（顶层链 E_NAME + declared 列表；实例侧同样） | `an_override_must_name_a_declared_parameter` `elaborate.rs:1570-1600`；`an_experiment_override_must_name_a_declared_parameter` `elaborate.rs:1605-1621`；`an_unknown_parameter_override_is_reported` `elaborate.rs:592-605`；`an_override_for_a_parameter_that_does_not_exist_is_refused` `session.rs(tests):406-414` |
| B5 | 实例覆盖维度不符 → E_DIMENSION（在覆盖处） | `an_override_with_the_wrong_dimension_is_reported` `elaborate.rs:609-622` |
| B6 | 一次成功的覆盖只改数值、不改拓扑 | `re_elaborating_with_an_override_changes_the_value_not_the_topology` `elaborate.rs:1171-1219`（逐个 device 比较 name/kind/terminals） |
| B7 | 会话 :run name=expr 生效并在摘要打印 override name = value；再次不带覆盖运行回到默认 | `an_explicit_override_changes_the_simulation` `session.rs(tests):370-403`；`a_session_defines_runs_changes_a_parameter_and_runs_again` `repl.rs(tests):74-107`；`[实测]` |
| B8 | 顶层覆盖按**裸名**只作用于顶层电路的参数，不进入实例同名参数 | `[实测]`：`circuit :top` 的 `param :r` 被 `:run e r=5.kohm` 改成 5 kΩ（v(out)=833.333 mV），而 `subcircuit :lp` 的同名 r 仍是 1 kΩ（i(x.xr)=833.333 µA） |
| B9 | 重复声明同一参数 = E_DUPLICATE（同 body 内） | `elaborate.rs:692-702`；`[实测]`（带 "first declared here" 次标签） |

### C. 运行/扫描契约

| 编号 | 必须保持 | 证据 |
|---|---|---|
| C1 | 参数扫描每点重新展开并拼接成**一个**数据集，分析名 dc_param_<参数名>，轴 Axis::Parameter | `a_parameter_sweep_yields_one_stitched_dataset` `crates/circuit-session/tests/expression_flow.rs:454-501`（断言 4 点、命名、measure=2.25）；`a_sweep_experiment_runs_through_the_session_too` `session.rs(tests):440-464`；`a_parameter_sweep_binds_to_the_stitched_dataset` `crates/circuit-cli/tests/expression_qa.rs:1327`；`[实测]`「dc_param_r: 4 sweep points」 |
| C2 | 扫描点的数值必须与解析式一致 | `run_parameter_sweep_is_exact` `crates/circuit-cli/tests/e2e.rs:520-551`（逐点比对 3·1500/(r+1500)，断言 8 行） |
| C3 | 每个实验最多一个参数扫描；两个 dc param: 在 check 期拒绝，并提示拆实验 | `two_parameter_sweeps_are_refused_at_check_time` `crates/circuit-dsl/tests/result_expressions.rs:733-749`；`one_parameter_sweep_is_accepted` `:717-730` |
| C4 | 源扫描不算参数扫描，两者并存合法 | `a_source_sweep_is_not_a_parameter_sweep` `result_expressions.rs:752-761` |
| C5 | 单点执行器必须拒绝参数扫描（要求走 per-point 驱动） | `a_parameter_sweep_is_refused_by_the_single_point_executor` `crates/circuit-backend/tests/adapter.rs:1145-1180`；`thevenin.rs:364-381` |
| C6 | 绑到非扫描分析上的 derive/measure 在**求解之前**被 E_UNSUPPORTED 拒绝（消息含 "only the swept DC analysis"） | `crates/circuit-session/src/execute.rs:120-123`、`:244-282`；`a_binding_to_a_non_swept_analysis_is_refused_before_the_run` `crates/circuit-cli/tests/expression_qa.rs:1389-1409`（断言 exit 1 + E_UNSUPPORTED + 消息） |
| C7 | 扫描点的拓扑不变性**运行期**必须仍然被检查（E_TOPO_PARAM，指出差异点与参考点） | `crates/circuit-backend/src/sweep.rs:31-61`、`:227-249`；`docs/architecture.md:463-470`；`[实测]`「sweeping this parameter changes the circuit topology at 2 / node mid2 appears only at this point」 |
| C8 | 所有示例仍能 check 通过（含 parameter_sweep.cdsl、two_stage.cdsl 的实例参数覆盖） | `check_accepts_every_example` `e2e.rs:55-75`（列表见 `e2e.rs:62`）；`run_two_stage_exercises_the_composition_features` `e2e.rs:400-431` |
| C9 | 结果表达式**不能**引用参数（报 E_TYPE "is not a signal"），derive 之间也不能互相引用 | `elaborate.rs:2404-2415`；`docs/language.md:665`（§7.8 当前限制）、`docs/language.md:563`；`[实测]` 探针 S |

### D. 会话事务性

| 编号 | 必须保持 | 证据 |
|---|---|---|
| D1 | 失败的 run 不写文件、不打印失败 measure、会话继续 | `a_failed_run_is_reported_and_writes_nothing` `repl.rs(tests):302-347`；`[实测]` |
| D2 | 失败的定义替换不改变任何既有定义/变量；commit 前整状态编译 | `a_definition_is_replaced_only_when_it_compiles` `session.rs(tests):233-269`；`a_failed_definition_leaves_variables_alone` `:272-281`；`a_redefinition_that_would_break_an_experiment_is_refused` `:284-`；`a_failed_definition_leaves_the_previous_one_running` `repl.rs(tests):152-175` |
| D3 | 语法错误后会话继续；脚本模式用 exit code 反映失败 | `a_syntax_error_is_reported_and_the_session_continues` `repl.rs(tests):139-149` |
| D4 | :run 不缓存定义，改完定义再跑必须看到新结果 | `a_run_uses_the_current_definition_not_a_cached_one` `session.rs(tests):426-437` |

---

## 3. 与 plan §5.1 / §5.2 有冲突、或代码未支持的点

> 本节是本文最重要的部分：下面每条都是「计划/任务书假设了、但今天的代码不是这样」的地方。
> 阶段 B 必须**显式选择**：改代码、或改计划。

### 3.1 §5.1.1「不能仅用裸字符串作为跨作用域节点 ID」——今天全是裸字符串（冲突）

- scope 的 key 是 `HashMap<String, Quantity>`（`elaborate.rs:193-196`）；
- 顶层覆盖链是 `Vec<(String, Quantity, SourceSpan)>`（`elaborate.rs:424`、`plan.rs:676`）；
- 驱动/会话覆盖是 `&[(String, Quantity)]`（`execute.rs:92`、`session.rs:570`）；
- 扫描目标是 `SweepTarget::Parameter { name: String }`（`plan.rs:412-413`）。

**现状为什么「看起来」没问题**：每个实例一份独立 scope（`elaborate.rs:1313`），跨 scope 根本没有解析路径，
所以同名参数天然不冲突（实测 B8）。**但一旦阶段 B 要建跨实例/跨层的依赖图**，裸名就无法区分
`top.r` 与 `top.stage1.r` —— 契约里必须给出限定身份的编码规则（并说明它如何与现有
check_overrides_are_declared 的裸名比较兼容）。

### 3.2 §5.1.3「不能让被覆盖的无效默认表达式误伤有效覆盖」——今天恰恰会误伤（冲突，已有实测）

- stmt_param **先求值 default，再判断是否被覆盖**（`elaborate.rs:714` → `720-723`），错误照样 self.error（`:736`）；
- 更硬的一条：compile() 对每个顶层电路**永远用空覆盖**展开（`elaborate.rs:97` `elaborate_top_circuit(def, &[])`），
  而 `cdsl check` 走的正是 compile（`check.rs:46`）。

实测：`param :r, default: nope` + 实验 `param :r, value: 1.kohm` → **E_NAME: `nope` is not declared**，
电路根本无法定义（更谈不上被覆盖）。
另一实测：`param :r`（无 default）+ 实验覆盖 → E_NAME `r` is not declared`（并带 secondary 指向 `param :r`）。

**含义**：§5.1.3 想要的语义（先定有效定义、再建图、坏默认不误伤）需要改 compile / stmt_param 的既有行为；
这属于**行为变更**，必须在契约里写清影响面（现有测试 `an_undeclared_parameter_is_an_error`、`a_forward_parameter_reference_is_rejected`
会不会被牵连）。

### 3.3 §5.1.4「前向引用、未知名称、依赖环要有不同诊断」——今天三者无法区分（未实现）

- 前向引用 = 未知名称 = E_NAME（同一条 `eval.rs:134-147`）；
- 环诊断 E_PARAM_CYCLE **无生产者**（`diagnostic.rs:62` 仅枚举，`:96` 仅字符串映射；全仓 grep 无构造点）；
- `docs/language.md:255` 已经把「无环、前向引用报 E_NAME、E_PARAM_CYCLE 不可达」写成**文档化行为**。

阶段 B 引入前向引用后，这两条文档必须同步改，否则文档与实现互相打脸。

### 3.4 §5.1.5「REPL 失败是否保留原状态；推荐沿用现有事务式成功提交」——现状已满足，但两点需注意

1. :run 是**运行期覆盖**，不是持久化「参数更新」；没有「失败更新污染下一次成功执行」的机制（实测 D1/D4）。
2. 若要引入真正的「参数更新命令」，注意今天**不存在** param 命令；会话覆盖路径也**不校验有限性**
   （`session.rs:545-560` 对照 `session.rs:252-257`），本机实测 `:run e r=1e308*1e308*1.ohm` 会把 inf 灌进
   scope.vars，最后由 device 的 check_finite 报 E_VALUE（`elaborate.rs:1674-1680`）。

### 3.5 §5.1.6 / §5.2「拓扑使用点」——真实存在的只有三类，且 dc param: 不是参数表达式（部分冲突）

- 真实存在：if 条件（`elaborate.rs:1579`）、for 迭代源（`:1457`、`:1481`）、计算名（`:376-415`、`:1036-1059`）。
- 计划说的「数量/重复展开」= for 的迭代源，确实存在，不必臆测新语法。
- 计划的「dc param: 扫描目标」**不是**参数使用点：它只接受符号（`elaborate.rs:3215-3232`），
  不构造参数依赖边。它是「扫描目标身份」的裸名字符串；§5.2 想要的「扫描 b 也要被识别」
  必须**从被扫描参数出发沿依赖图反向**判定谁依赖它，而不是从 dc 语句里找参数引用。
- 「分析规模参数化」（`points: n`、`tran stop: t`）**今天不可表达**（空 scope，实测 E_NAME）。

### 3.6 §5.2「check 拒绝会改变拓扑的参数扫描」——未实现（冲突，且是运行期行为变更）

- 唯一的 E_TOPO_PARAM 生产者在运行期：`sweep.rs:236`，由 run_parameter_sweep 的逐点比较触发（`:227-249`）；
- `cdsl check` = compile + backend.validate（`check.rs:26-77`），而 validate 对
  `SweepTarget::Parameter { .. }` **直接放行**（`thevenin.rs:200-202`）；
- `docs/review-evidence/next-round-contracts.md:81` 已经记录过同一结论：`check` exit 0、`run` exit 1 = E_TOPO_PARAM。

实测：`for k in 1..taps` 的电路 + `dc param: :taps, from: 1, to: 3, step: 1`
→ 定义（≡ check 级）**成功**，`:run sweep` 报 E_TOPO_PARAM（差异点 2，`node `mid2` appears only at this point`）。
阶段 B 要在 check 期拒绝，必须同时：
(a) 保留运行期防线（`docs/architecture.md:463-470` 与 C7 测试仍要过），
(b) 更新 `docs/language.md:257` / `docs/architecture.md` 的表述。

### 3.7 任务书里「session.rs 的 param 命令」——**不存在**（任务书假设与代码不符）

`COMMANDS = &[":help", ":load", ":list", ":quit", ":exit", ":reset", ":run"]`（`session.rs:70-72`），
command() 的 match 只有这些分支（`session.rs:322-335`）。param 只是 **circuit/subcircuit/experiment body 的语句**：
- 电路里：`param :r, default: …`（parser `param_stmt`，`parser.rs:955-977`）；
- 实验里：`param :r, value: …`（`parser.rs:1354-1375`）。

会话侧的「参数覆盖」只有 `:run <exp> name=expr`（`session.rs:484-564`），且 name=expr 的 name
不要求匹配任何已声明键（存在性在展开期由 check_overrides_are_declared 判）。契约若沿用「会话 param 命令」
的措辞，会指到一个不存在的入口。

### 3.8 其它「计划假设但代码未实现」清单

| # | 计划/任务书假设 | 代码事实 | 位置 |
|---|---|---|---|
| a | 存在「参数 DAG / 拓扑依赖传播」 | 不存在；求值就是 scope 顺序执行 | `elaborate.rs:631-664` |
| b | 存在「环诊断」 | E_PARAM_CYCLE 无生产者 | `diagnostic.rs:62,96` |
| c | 顶层覆盖会校验**维度** | Scope::defaults 写了从不读；维度错误只在 device 处暴露 | `elaborate.rs:199-201,717`；实测 `:run e r=5.nF` → `E_DIMENSION: ra.value needs ohm, found F`（指向 device，不指向覆盖） |
| d | 实例 params: 重复键会被拒绝 | 不报错，**最后者胜** | `elaborate.rs:1351-1397`（无重复检查）；实测 `params: { r: 1.kohm, r: 3.kohm }` → 3 kΩ |
| e | 实验 param 重复会被拒绝 | 不报错，**最后者胜**；param_overrides 里会留下**两条**同名记录 | `elaborate.rs:1992-2000`（替换）与 `:2039-2043`（追加，不去重）；实测两条 `param :r` → v(out)=1 V（r=4 kΩ 胜） |
| f | 「同名局部参数不能产生误报」已有保障 | 实例内同名互不影响成立，但**没有任何测试**覆盖「顶层参数 + 实例同名参数 + 顶层覆盖」的三元组合 | 实测见 B8；现有测试只有 `elaborate.rs:469`（实例之间） |
| g | 「扫描 b 也必须被识别」 | 需要新建反向依赖判定；今天只有运行期逐点比较 | 见 3.6 |

---

## 4. 实测记录（命令、输入、观察、退出码）

统一方式（**只读、不落盘**）：`<here-string> \| & .\target\debug\cdsl.exe repl`。REPL 在 stdin EOF 后退出；
有任一失败输入时 exit code = 1，全成功 = 0。

| 探针 | 输入要点 | 观察结果 | exit |
|---|---|---|---|
| A | `param :b, default: 2 * a` 在前、`param :a` 在后 | `error[E_NAME]: `a` is not declared` @ 2:26` | 1 |
| B | 同一 body 两个 `param :r` | `error[E_DUPLICATE]: parameter `r` is declared twice` + 次标签 first declared here` | 1 |
| C | `ac … points: n`（n 是参数） | `error[E_NAME]: `n` is not declared`（分析参数不在参数作用域） | 1 |
| D | 实例 params: { r: later }，param :later 在实例语句之后 | `error[E_NAME]: `later` is not declared` | 1 |
| E | 实验 param :x, value: y，param :y 在下一行 | `E_NAME: `y` is not declared` + `E_NAME: circuit `c` has no parameter `y`` | 1 |
| F | for k in 1..taps 的电路 + dc param: :taps, from: 1, to: 3, step: 1 | 定义成功（≡ check 级放行）；:run → `error[E_TOPO_PARAM]: sweeping this parameter changes the circuit topology at 2`（node mid2 appears only at this point） | 1 |
| G | 值参数扫描 dc param: :r, 0.5k..2k | `dc_param_r: 4 sweep points` | 0 |
| H2 | 实验 param :r, value: 2.kohm + 同一参数扫描 + measure :big, max: v(:out) | `measure big = 2.25 V (dc_param_r)` = 3·1.5/(0.5+1.5) → **扫描点胜出** | 0 |
| I | param :r, default: nope + 实验 param :r, value: 1.kohm | `error[E_NAME]: `nope` is not declared`；电路定义失败 | 1 |
| J | param :r（无 default，且无覆盖） | `error[E_NAME]: `r` is not declared` + 次标签 a parameter with this name is declared here | 1 |
| K | circuit :top 与 subcircuit :lp 都有 param :r；:run e / :run e r=5.kohm | 默认 v(out)=500 mV；覆盖后 v(out)=833.333 mV 而 i(x.xr)=833.333 µA（实例 r 仍 1 kΩ） | 0 |
| L | x = 42 → x = :notanumber → x → 两次失败 :run → :run e | E_TYPE: a variable must be a number；x 仍为 42；E_NAME: circuit c has no parameter r9；E_NAME: zzz is not declared @ <override>:1:3；最后的 :run e 仍是 2.5 V | 1 |
| M | 实验 body 两条 param :r（3 kΩ / 4 kΩ） | 无诊断；v(out)=1 V ⇒ r=4 kΩ（最后者胜） | 0 |
| N | 实例 params: { r: 1.kohm, r: 3.kohm } | 无诊断；v(b)=3 V、i(x.r1)=1 mA ⇒ r=3 kΩ（最后者胜） | 0 |
| O | :run e r=1e308*10 | E_DIMENSION: ra.value needs ohm, found dimensionless（覆盖值无量纲且为 inf） | 1 |
| P | 子电路 param :r, default: rr，父电路有 param :rr | E_NAME: rr is not declared（子电路默认值看不到父作用域） | 1 |
| Q | :run e r=1e308*1e308*1.ohm | E_VALUE: value is not a finite number（在 device 的 value: 处，不是覆盖处） | 1 |
| R | :run e r=5.nF（默认 1 kΩ） | E_DIMENSION: ra.value needs ohm, found F（指向 device，不指向覆盖） | 1 |
| S | derive :d, expr: v(:out) * r（r 是电路参数） | `error[E_TYPE]: `r` is not a signal`（+ note: signals are written v(:node)…） | 1 |

测试命令（本轮实测）：

| 命令 | 结果 |
|---|---|
| `cargo test -p circuit-dsl --test elaborate` | `67 passed; 0 failed`，exit 0（LASTEXITCODE=0） |
| `cargo test -p circuit-cli --test repl` | `13 passed; 0 failed`，exit 0（同上；该命令同时把 target/debug/cdsl.exe 重建到当前源码） |

---

## 5. 无法取证 / 代码未实现（明确列出，不猜）

1. **参数依赖图、拓扑依赖传播、拓扑参数静态标记**：仓库中不存在任何实现（circuit-dsl 内 grep `topolog`
   只命中一处注释 `elaborate.rs:136`）。
2. **check 期拒绝拓扑参数扫描**：不存在；只有运行期 `sweep.rs:236`。
3. **环诊断 / 闭合路径输出**：E_PARAM_CYCLE 无生产者，无法从代码取证其「应有形状」。
4. **前向引用与未知名称的区分诊断**：今天不可区分（同一 E_NAME），阶段 B 需要新设计，代码里没有参照物。
5. **跨作用域参数 ID**：今天不存在；裸字符串是唯一的身份表示，无法从代码取证「限定名」该怎么拼。
6. **会话 param 命令 / 持久化参数更新**：不存在（`session.rs:70-72`）。
7. **backend 内部状态在一次失败 run 后是否完全无副作用**：无测试覆盖；我只能证明「后续成功 run 的数值与首次一致」
   （探针 L/K），不能证明后端内部无残留。
8. **:run 覆盖的有限性校验**：代码里没有（`session.rs:545-560`），只能证明非有限值最终在下游报错（探针 O/Q）。
9. **实例 params: 重复键 / 实验 param 重复的「正确」处理**：代码今天是静默最后者胜（探针 M/N），
   没有任何测试或文档固定它 —— 阶段 B 可以选择收紧，但要作为行为变更记录。
