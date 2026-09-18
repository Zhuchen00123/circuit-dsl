//! Round-4 phase-B QA: the parameter dependency graph.
//!
//! Contract: docs/review-evidence/round4/design-contract.md §4. Written by the
//! QA worker against the frozen semantics, independent of the implementation:
//! every number is derived in a comment, and every diagnostic assertion names
//! the code, the closed path and the source location.
//!
//! These tests failed before phase B landed: a forward reference was E_NAME
//! ("a declaration in source order decides what is visible"), a self reference
//! was E_NAME rather than a cycle, and an override did not stop the default
//! expression from being evaluated.

use circuit_core::diagnostic::Code;
use circuit_core::units::{self, Quantity};
use circuit_core::{Limits, SourceMap};
use circuit_dsl::{Compiled, Elaborated, compile, elaborate_experiment, lex, parse};

fn compile_src(src: &str) -> Result<Compiled, String> {
    let mut sm = SourceMap::new();
    let id = sm.add("test.cdsl", src);
    let tokens = lex(id, src).map_err(|d| d.render(&sm))?;
    let program = parse(&tokens).map_err(|d| d.render(&sm))?;
    compile(&program, &Limits::default()).map_err(|d| d.render(&sm))
}

/// Compile and expect success.
fn ok(src: &str) -> Compiled {
    match compile_src(src) {
        Ok(c) => c,
        Err(text) => panic!("expected success, got diagnostics:\n{text}"),
    }
}

/// Compile and expect failure; return the rendered diagnostics.
fn err(src: &str) -> String {
    match compile_src(src) {
        Ok(_) => panic!("expected diagnostics, but compilation succeeded"),
        Err(text) => text,
    }
}

fn assert_code(text: &str, code: Code) {
    let needle = format!("[{}]", code.as_str());
    assert!(text.contains(&needle), "expected {needle} in:\n{text}");
}

/// Elaborate one experiment through the run path, with extra overrides.
fn experiment(
    src: &str,
    name: &str,
    overrides: &[(&str, f64, circuit_core::units::Dimension)],
) -> Result<Elaborated, String> {
    let mut sm = SourceMap::new();
    let id = sm.add("test.cdsl", src);
    let tokens = lex(id, src).map_err(|d| d.render(&sm))?;
    let program = parse(&tokens).map_err(|d| d.render(&sm))?;
    let values: Vec<(String, Quantity)> = overrides
        .iter()
        .map(|(n, v, d)| ((*n).to_string(), Quantity::new(*v, *d)))
        .collect();
    elaborate_experiment(&program, name, &values, &Limits::default()).map_err(|d| d.render(&sm))
}

fn experiment_ok(
    src: &str,
    name: &str,
    overrides: &[(&str, f64, circuit_core::units::Dimension)],
) -> Elaborated {
    match experiment(src, name, overrides) {
        Ok(e) => e,
        Err(text) => panic!("expected the experiment to elaborate, got:\n{text}"),
    }
}

fn device_value(el: &Elaborated, name: &str) -> f64 {
    let id = el
        .circuit
        .device_id(name)
        .unwrap_or_else(|| panic!("no device named {name}"));
    el.circuit
        .device(id)
        .unwrap()
        .param("value")
        .unwrap_or_else(|| panic!("device {name} has no value parameter"))
        .value
}

// ---------------------------------------------------------------------------
// Forward references, chains, diamonds
// ---------------------------------------------------------------------------

/// A forward reference inside one body is legal: declarations are collected
/// before the body runs, and the graph decides the order.
///
/// b = 2 * a = 2 * 1 kohm = 2 kohm, even though a is declared after b.
#[test]
fn a_forward_reference_resolves_to_the_declared_value() {
    let c = ok("circuit :fwd do
  param :b, default: 2 * a
  param :a, default: 1.kohm
  node :x
  voltage_source :v1, p: :x, n: :gnd, dc: 1.V
  resistor :r1, p: :x, n: :gnd, value: b
end
");
    let r1 = c.circuits[0]
        .device(c.circuits[0].device_id("r1").unwrap())
        .unwrap();
    assert_eq!(r1.param("value").unwrap().value, 2000.0);
    assert_eq!(r1.param("value").unwrap().dimension, units::RESISTANCE);
}

/// Three levels, all forward: c = a + b = 3 + 6 = 9 kohm.
#[test]
fn a_multi_level_chain_resolves_in_dependency_order() {
    let c = ok("circuit :chain do
  param :c, default: a + b
  param :b, default: 2 * a
  param :a, default: 3.kohm
  node :x
  voltage_source :v1, p: :x, n: :gnd, dc: 1.V
  resistor :r1, p: :x, n: :gnd, value: c
end
");
    let r1 = c.circuits[0]
        .device(c.circuits[0].device_id("r1").unwrap())
        .unwrap();
    // a = 3 kohm, b = 2*a = 6 kohm, c = a + b = 9 kohm.
    assert_eq!(r1.param("value").unwrap().value, 9000.0);
}

/// A diamond: base feeds two independent branches which meet at the top.
/// base = 5, left = 2*base = 10, right = 3*base = 15, top = left+right = 25 kohm.
///
/// The same graph written in the opposite declaration order must give the same
/// number: source order only breaks ties, it does not decide the value.
#[test]
fn a_diamond_evaluates_once_and_independently_of_declaration_order() {
    let forward = ok("circuit :diamond do
  param :top, default: left + right
  param :right, default: 3 * base
  param :left, default: 2 * base
  param :base, default: 5.kohm
  node :x
  voltage_source :v1, p: :x, n: :gnd, dc: 1.V
  resistor :r1, p: :x, n: :gnd, value: top
end
");
    let reverse = ok("circuit :diamond do
  param :base, default: 5.kohm
  param :left, default: 2 * base
  param :right, default: 3 * base
  param :top, default: left + right
  node :x
  voltage_source :v1, p: :x, n: :gnd, dc: 1.V
  resistor :r1, p: :x, n: :gnd, value: top
end
");
    let value = |c: &Compiled| {
        c.circuits[0]
            .device(c.circuits[0].device_id("r1").unwrap())
            .unwrap()
            .param("value")
            .unwrap()
            .value
    };
    assert_eq!(value(&forward), 25000.0);
    assert_eq!(value(&reverse), 25000.0);
}

/// A long forward chain: p_k = 2 * p_{k-1} declared in reverse, so the whole
/// chain is forward references. p_20 = 2^20 ohm = 1048576 ohm.
#[test]
fn a_long_forward_chain_is_fully_resolved() {
    let levels = 20;
    let mut src = String::from("circuit :deep do\n");
    for k in (1..=levels).rev() {
        src.push_str(&format!("  param :p{k}, default: 2 * p{}\n", k - 1));
    }
    src.push_str("  param :p0, default: 1.ohm\n");
    src.push_str("  node :x\n");
    src.push_str("  voltage_source :v1, p: :x, n: :gnd, dc: 1.V\n");
    src.push_str("  resistor :r1, p: :x, n: :gnd, value: p20\n");
    src.push_str("end\n");

    let c = ok(&src);
    let expected = 2f64.powi(levels);
    let r1 = c.circuits[0]
        .device(c.circuits[0].device_id("r1").unwrap())
        .unwrap();
    assert_eq!(r1.param("value").unwrap().value, expected);
}

// ---------------------------------------------------------------------------
// Cycles and unknown names
// ---------------------------------------------------------------------------

/// param :a, default: a is the one-node cycle: the name is declared, the
/// definition is circular, so it is E_PARAM_CYCLE, not E_NAME.
#[test]
fn a_self_reference_is_a_parameter_cycle() {
    let text = err("circuit :cyc do
  param :a, default: a
  node :x
end
");
    assert_code(&text, Code::ParamCycle);
    assert!(
        !text.contains("[E_NAME]"),
        "a declared name is not an unknown name:\n{text}"
    );
    // The declaration is located: line 2 of the source.
    assert!(text.contains("test.cdsl:2"), "{text}");
}

/// a -> b -> a: the closed path is in the message and both declarations are
/// pointed at.
#[test]
fn a_two_node_cycle_reports_the_closed_path_and_both_spans() {
    let text = err("circuit :cyc do
  param :a, default: b
  param :b, default: a
  node :x
end
");
    assert_code(&text, Code::ParamCycle);
    // A pure cycle has no unknown name in it: the declarations are all known
    // before the graph is walked, so nothing may be reported as E_NAME.
    assert!(
        !text.contains("[E_NAME]"),
        "a cycle is not an unknown name:\n{text}"
    );
    assert!(
        text.contains("a -> b -> a") || text.contains("b -> a -> b"),
        "the closed path must be printed:\n{text}"
    );
    assert!(text.contains("test.cdsl:2"), "{text}");
    assert!(text.contains("test.cdsl:3"), "{text}");
}

/// A three-node cycle names every participant and locates every declaration.
#[test]
fn a_three_node_cycle_names_the_whole_path() {
    let text = err("circuit :cyc do
  param :a, default: c
  param :b, default: a
  param :c, default: b
  node :x
end
");
    assert_code(&text, Code::ParamCycle);
    assert!(!text.contains("[E_NAME]"), "{text}");
    for name in ["a", "b", "c"] {
        assert!(text.contains(name), "cycle must name {name}:\n{text}");
    }
    for line in [2, 3, 4] {
        assert!(
            text.contains(&format!("test.cdsl:{line}")),
            "declaration on line {line} must be located:\n{text}"
        );
    }
}

/// A name that is declared nowhere is still E_NAME at the reference, with the
/// existing note; it is not a cycle and must not be reported as one.
#[test]
fn an_unknown_name_is_not_a_cycle() {
    let text = err("circuit :u do
  param :a, default: nope
  node :x
end
");
    assert_code(&text, Code::Name);
    assert!(
        !text.contains("E_PARAM_CYCLE"),
        "an undeclared name is not a cycle:\n{text}"
    );
    assert!(text.contains("not declared"), "{text}");
    assert!(text.contains("nope"), "{text}");
}

/// param :a with neither default nor override keeps today's behaviour: reading
/// it is E_NAME with the declaration in scope named.
#[test]
fn a_parameter_without_a_default_is_an_unknown_name() {
    let text = err("circuit :u do
  param :a
  node :x
  voltage_source :v1, p: :x, n: :gnd, dc: a
end
");
    assert_code(&text, Code::Name);
    assert!(!text.contains("E_PARAM_CYCLE"), "{text}");
}

// ---------------------------------------------------------------------------
// The effective definition comes first
// ---------------------------------------------------------------------------

/// An override replaces the definition, so the default is not evaluated and
/// cannot produce a diagnostic in the elaboration that uses the override.
///
/// check still judges the standalone circuit (compile with an empty chain) and
/// therefore reports E_NAME; the experiment's own override is a different
/// elaboration, and there the parameter is 1 kohm.
#[test]
fn an_override_replaces_the_default_in_the_experiment_elaboration() {
    let src = "circuit :ov do
  param :r, default: nope
  node :x
  voltage_source :v1, p: :x, n: :gnd, dc: 1.V
  resistor :r1, p: :x, n: :gnd, value: r
end
experiment :e, circuit: :ov do
  param :r, value: 1.kohm
  op
  save v(:x)
end
";
    // The standalone circuit is still checked.
    assert_code(&err(src), Code::Name);

    // With the override in effect the default is not part of the graph.
    let el = experiment_ok(src, "e", &[]);
    assert_eq!(device_value(&el, "r1"), 1000.0);
}

/// An override can also break a cycle the defaults would have: a is supplied,
/// so only b's edge remains and b = 5 + 1 = 6 ohm.
#[test]
fn an_override_breaks_a_cycle_that_the_defaults_would_have() {
    let src = "circuit :ovc do
  param :a, default: b + 1.ohm
  param :b, default: a + 1.ohm
  node :x
  voltage_source :v1, p: :x, n: :gnd, dc: 1.V
  resistor :r1, p: :x, n: :gnd, value: b
end
experiment :e, circuit: :ovc do
  param :a, value: 5.ohm
  op
  save v(:x)
end
";
    let text = err(src);
    assert_code(&text, Code::ParamCycle);
    // Declarations are known before the body runs, so the override names a
    // parameter even though the circuit body could not finish walking.
    assert!(!text.contains("[E_NAME]"), "{text}");
    let el = experiment_ok(src, "e", &[]);
    assert_eq!(device_value(&el, "r1"), 6.0);
}

// ---------------------------------------------------------------------------
// Scope identity: same-named parameters in nested instances
// ---------------------------------------------------------------------------

const TWO_INSTANCES: &str = "subcircuit :leaf, ports: [:a, :b] do
  param :r, default: 1.kohm
  resistor :rl, p: :a, n: :b, value: r
end
circuit :top do
  param :rbase, default: 1.kohm
  node :x, :y, :z
  voltage_source :v1, p: :x, n: :gnd, dc: 1.V
  resistor :r0, p: :x, n: :y, value: rbase
  instance :s1, of: :leaf, ports: { a: :y, b: :gnd }, params: { r: rbase }
  instance :s2, of: :leaf, ports: { a: :y, b: :z }, params: { r: 3.kohm }
  resistor :rz, p: :z, n: :gnd, value: 1.kohm
end
experiment :e, circuit: :top do
  op
  save v(:y)
end
";

/// Two instances of one subcircuit keep independent values: s1.r follows the
/// parent's rbase (1 kohm), s2.r keeps its own 3 kohm.
#[test]
fn same_named_parameters_of_nested_instances_do_not_pollute_each_other() {
    let el = experiment_ok(TWO_INSTANCES, "e", &[]);
    assert_eq!(device_value(&el, "r0"), 1000.0);
    assert_eq!(device_value(&el, "s1.rl"), 1000.0);
    assert_eq!(device_value(&el, "s2.rl"), 3000.0);
}

/// Overriding the base parameter recomputes everything that depends on it:
/// r0 = 2 kohm because rbase = 2 kohm, and s1.rl follows the same wire, while
/// s2.rl is a different parameter and stays at 3 kohm.
#[test]
fn an_override_recomputes_every_dependent_parameter_across_scopes() {
    let el = experiment_ok(TWO_INSTANCES, "e", &[("rbase", 2000.0, units::RESISTANCE)]);
    assert_eq!(device_value(&el, "r0"), 2000.0);
    assert_eq!(device_value(&el, "s1.rl"), 2000.0);
    assert_eq!(device_value(&el, "s2.rl"), 3000.0);
}
