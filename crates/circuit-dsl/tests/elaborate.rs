//! Elaboration tests: source text in, circuit and plan out.
//!
//! These are the acceptance evidence for the language features the brief
//! requires in stages 1-4. Every test drives the real front end (lex, parse,
//! elaborate) — none of them construct an AST by hand, so a change to the
//! grammar or the elaborator cannot pass by accident here.

use circuit_core::diagnostic::Code;
use circuit_core::ir::DeviceKind;
use circuit_core::plan::{AnalysisKind, Probe, SweepTarget};
use circuit_core::units;
use circuit_core::{Limits, SourceMap};
use circuit_dsl::{compile, elaborate_experiment, lex, parse};

/// Run the whole front end, returning the compiled result or the rendered
/// diagnostics.
struct Outcome {
    result: Result<circuit_dsl::Compiled, String>,
}

fn run(src: &str) -> Outcome {
    let mut sm = SourceMap::new();
    let id = sm.add("test.cdsl", src);

    let tokens = match lex(id, src) {
        Ok(t) => t,
        Err(d) => {
            return Outcome {
                result: Err(d.render(&sm)),
            };
        }
    };
    let program = match parse(&tokens) {
        Ok(p) => p,
        Err(d) => {
            return Outcome {
                result: Err(d.render(&sm)),
            };
        }
    };
    match compile(&program, &Limits::default()) {
        Ok(c) => Outcome { result: Ok(c) },
        Err(d) => Outcome {
            result: Err(d.render(&sm)),
        },
    }
}

/// Compile and expect success.
fn ok(src: &str) -> circuit_dsl::Compiled {
    match run(src).result {
        Ok(c) => c,
        Err(text) => panic!("expected success, got diagnostics:\n{text}"),
    }
}

/// Compile and expect failure; return the rendered diagnostics.
fn err(src: &str) -> String {
    match run(src).result {
        Ok(_) => panic!("expected diagnostics, but compilation succeeded"),
        Err(text) => text,
    }
}

fn assert_code(text: &str, code: Code) {
    let needle = format!("[{}]", code.as_str());
    assert!(text.contains(&needle), "expected {needle} in:\n{text}");
}

// ---------------------------------------------------------------------------
// Stage 1: the minimal closed loop
// ---------------------------------------------------------------------------

#[test]
fn voltage_divider_elaborates() {
    let c = ok(r#"
circuit :divider do
  node :vin, :vout
  voltage_source :src, p: :vin, n: :gnd, dc: 5.V
  resistor :r1, p: :vin, n: :vout, value: 1.kohm
  resistor :r2, p: :vout, n: :gnd, value: 2.kohm
end

experiment :dc_op, circuit: :divider do
  op
  save v(:vin), v(:vout), i(:r1)
end
"#);

    assert_eq!(c.circuits.len(), 1);
    let circuit = &c.circuits[0];
    assert_eq!(circuit.name, "divider");
    // ground + vin + vout
    assert_eq!(circuit.nodes.len(), 3);
    assert_eq!(circuit.devices.len(), 3);

    let r1 = circuit.device(circuit.device_id("r1").unwrap()).unwrap();
    assert_eq!(r1.kind, DeviceKind::Resistor);
    assert_eq!(r1.param("value").unwrap().value, 1000.0);
    assert_eq!(r1.param("value").unwrap().dimension, units::RESISTANCE);
    // p -> n fixes the positive current direction.
    assert_eq!(r1.pos(), circuit.node_id("vin"));
    assert_eq!(r1.neg(), circuit.node_id("vout"));

    let src = circuit.device(circuit.device_id("src").unwrap()).unwrap();
    assert_eq!(src.kind, DeviceKind::VoltageSource);
    assert_eq!(src.source.as_ref().unwrap().dc.unwrap().value, 5.0);

    assert_eq!(c.experiments.len(), 1);
    let e = &c.experiments[0];
    assert_eq!(e.plan.name, "dc_op");
    assert_eq!(e.plan.tasks.len(), 1);
    assert!(matches!(e.plan.tasks[0].kind, AnalysisKind::Op));
    assert_eq!(e.plan.tasks[0].probes.len(), 3);

    let names: Vec<&str> = e.plan.tasks[0]
        .probes
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(names, vec!["v(vin)", "v(vout)", "i(r1)"]);
}

/// The `rc_filter` example from the language specification.
#[test]
fn rc_filter_example_elaborates() {
    let c = ok(r#"
circuit :rc_filter do
  param :r, default: 1.kohm
  param :c, default: 100.nF

  node :vin, :vout

  voltage_source :input, p: :vin, n: :gnd,
    dc: 0.V,
    ac: 1.V,
    waveform: pulse(low: 0.V, high: 1.V,
                    delay: 1.us, rise: 10.ns,
                    fall: 10.ns, width: 5.us,
                    period: 10.us)

  resistor :r1, p: :vin, n: :vout, value: r
  capacitor :c1, p: :vout, n: :gnd, value: c
end

experiment :response, circuit: :rc_filter do
  op
  ac from: 10.Hz, to: 10.MHz, points_per_decade: 50
  tran stop: 30.us, max_step: 50.ns

  save v(:vin), v(:vout), i(:input)
end
"#);

    let circuit = &c.circuits[0];
    let r1 = circuit.device(circuit.device_id("r1").unwrap()).unwrap();
    // The parameter default must have been substituted.
    assert_eq!(r1.param("value").unwrap().value, 1000.0);
    let c1 = circuit.device(circuit.device_id("c1").unwrap()).unwrap();
    // `100.nF` scales in binary floating point, so compare with a tolerance.
    let cval = c1.param("value").unwrap();
    assert!(
        (cval.value - 100e-9).abs() < 1e-20,
        "capacitance is {}",
        cval.value
    );
    assert_eq!(cval.dimension, units::CAPACITANCE);

    // The source carries dc, ac and a waveform simultaneously.
    let input = circuit.device(circuit.device_id("input").unwrap()).unwrap();
    let spec = input.source.as_ref().unwrap();
    assert_eq!(spec.dc.unwrap().value, 0.0);
    assert_eq!(spec.ac.unwrap().magnitude.value, 1.0);
    match spec.waveform.as_ref().unwrap() {
        circuit_core::Waveform::Pulse {
            low,
            high,
            width,
            period,
            ..
        } => {
            assert_eq!(low.value, 0.0);
            assert_eq!(high.value, 1.0);
            // Unit scaling is done in binary floating point.
            assert!(
                (width.value - 5e-6).abs() < 1e-18,
                "width = {}",
                width.value
            );
            assert!(
                (period.value - 1e-5).abs() < 1e-18,
                "period = {}",
                period.value
            );
        }
        other => panic!("expected a pulse, got {other:?}"),
    }

    let e = &c.experiments[0];
    assert_eq!(e.plan.tasks.len(), 3);

    match &e.plan.tasks[1].kind {
        AnalysisKind::Ac(a) => {
            assert_eq!(a.start_hz, 10.0);
            assert_eq!(a.stop_hz, 10e6);
            assert_eq!(a.points, 50);
            // 6 decades at 50 points/decade -> 301 points.
            assert_eq!(a.point_count(), 301);
        }
        other => panic!("expected ac, got {other:?}"),
    }
    match &e.plan.tasks[2].kind {
        AnalysisKind::Tran(t) => {
            assert!((t.stop_s - 30e-6).abs() < 1e-20, "stop = {}", t.stop_s);
            assert_eq!(t.max_step.map(|m| m > 0.0), Some(true));
            // `max_step` must not become an output interval.
            assert_eq!(t.output_interval, None);
        }
        other => panic!("expected tran, got {other:?}"),
    }

    // `save` applies to every analysis in the experiment.
    for task in &e.plan.tasks {
        assert_eq!(task.probes.len(), 3, "probes must reach every task");
        assert_eq!(task.probes[2].name, "i(input)");
    }
}

/// A differential probe is recorded as `v(a, b)`, not two single probes.
#[test]
fn differential_probe_is_typed_as_such() {
    let c = ok(r#"
circuit :d do
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  resistor :r1, p: :a, n: :b, value: 1.kohm
  resistor :r2, p: :b, n: :gnd, value: 1.kohm
end
experiment :x, circuit: :d do
  op
  save v(:a, :b)
end
"#);
    let p = &c.experiments[0].plan.tasks[0].probes[0];
    assert_eq!(p.name, "v(a,b)");
    assert!(matches!(p.probe, Probe::DifferentialVoltage { .. }));
}

// ---------------------------------------------------------------------------
// Stage 1: locatable diagnostics
// ---------------------------------------------------------------------------

#[test]
fn a_wrong_dimension_is_reported_with_both_dimensions() {
    let text = err(r#"
circuit :bad do
  node :a, :b
  resistor :r1, p: :a, n: :b, value: 10.ms
end
"#);
    assert_code(&text, Code::Dimension);
    // Expected and received must both be named.
    assert!(text.contains("ohm"), "{text}");
    assert!(text.contains("s"), "{text}");
    // And the location must be given.
    assert!(text.contains("test.cdsl:4"), "{text}");
}

#[test]
fn an_undeclared_node_is_an_error_not_a_new_node() {
    let text = err(r#"
circuit :typo do
  node :vin
  voltage_source :v1, p: :vin, n: :gnd, dc: 1.V
  resistor :r1, p: :vin, n: :vot, value: 1.kohm
end
"#);
    assert_code(&text, Code::Name);
    assert!(text.contains("vot"), "{text}");
    // The message must tell the user how to fix it.
    assert!(text.contains("declare it"), "{text}");
}

#[test]
fn a_duplicate_device_names_both_locations() {
    let text = err(r#"
circuit :dup do
  node :a
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  resistor :r1, p: :a, n: :gnd, value: 1.kohm
  resistor :r1, p: :a, n: :gnd, value: 2.kohm
end
"#);
    assert_code(&text, Code::Duplicate);
    assert!(text.contains("first defined here"), "{text}");
}

#[test]
fn non_positive_passives_are_rejected_rather_than_clamped() {
    for value in ["0.ohm", "-1.kohm"] {
        let text = err(&format!(
            r#"
circuit :z do
  node :a
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  resistor :r1, p: :a, n: :gnd, value: {value}
end
"#
        ));
        assert_code(&text, Code::Value);
        assert!(
            text.contains("greater than zero") || text.contains("needs"),
            "for {value}:\n{text}"
        );
    }
}

#[test]
fn a_missing_required_argument_is_reported() {
    let text = err(r#"
circuit :m do
  node :a, :b
  resistor :r1, p: :a, n: :b
end
"#);
    assert_code(&text, Code::Argument);
    assert!(text.contains("value"), "{text}");
}

#[test]
fn an_unknown_argument_lists_what_is_accepted() {
    let text = err(r#"
circuit :u do
  node :a, :b
  resistor :r1, p: :a, n: :b, value: 1.kohm, resistance: 2.kohm
end
"#);
    assert_code(&text, Code::Argument);
    assert!(text.contains("accepted"), "{text}");
}

#[test]
fn an_unknown_function_is_reported() {
    let text = err(r#"
circuit :f do
  param :x, default: frobnicate(2)
end
"#);
    assert_code(&text, Code::Name);
    assert!(text.contains("frobnicate"), "{text}");
}

/// An undeclared bare name is never silently turned into a value.
#[test]
fn an_undeclared_parameter_is_an_error() {
    let text = err(r#"
circuit :p do
  node :a, :b
  resistor :r1, p: :a, n: :b, value: r
end
"#);
    assert_code(&text, Code::Name);
    assert!(text.contains("not declared"), "{text}");
}

// ---------------------------------------------------------------------------
// Stage 3: parameters
// ---------------------------------------------------------------------------

/// Parameters are declared in order and may refer to earlier ones, which
/// makes a dependency cycle structurally impossible. A self-reference
/// therefore fails as an ordinary undeclared name, with the declaration in
/// scope named in the diagnostic.
#[test]
fn parameters_are_resolved_in_declaration_order() {
    let c = ok(r#"
circuit :chain do
  param :base, default: 1.kohm
  param :doubled, default: 2 * base
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  resistor :r1, p: :a, n: :b, value: doubled
  resistor :r2, p: :b, n: :gnd, value: base
end
"#);
    let r1 = c.circuits[0]
        .device(c.circuits[0].device_id("r1").unwrap())
        .unwrap();
    assert_eq!(r1.param("value").unwrap().value, 2000.0);
    assert_eq!(r1.param("value").unwrap().dimension, units::RESISTANCE);
}

#[test]
fn a_self_referential_parameter_is_rejected() {
    let text = err(r#"
circuit :cyc do
  param :a, default: a
  node :x
end
"#);
    assert_code(&text, Code::Name);
}

#[test]
fn a_forward_parameter_reference_is_rejected() {
    let text = err(r#"
circuit :fwd do
  param :b, default: 2 * a
  param :a, default: 1.kohm
  node :x
end
"#);
    assert_code(&text, Code::Name);
    assert!(text.contains("not declared"), "{text}");
}

#[test]
fn dimensionless_contexts_reject_quantities() {
    let text = err(r#"
circuit :d do
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  resistor :r1, p: :a, n: :b, value: 1.kohm
end
experiment :e, circuit: :d do
  ac from: 10.Hz, to: 1.MHz, points_per_decade: 5.kohm
end
"#);
    assert_code(&text, Code::Dimension);
}

// ---------------------------------------------------------------------------
// Stage 3: subcircuits and hierarchy
// ---------------------------------------------------------------------------

/// The `two_stage` example from the specification: instance-local nodes,
/// per-instance parameters, and complete port binding.
#[test]
fn subcircuit_instances_are_isolated_and_parameters_override() {
    let c = ok(r#"
subcircuit :lowpass, ports: [:input, :output, :ground] do
  param :r, default: 1.kohm
  param :c, default: 100.nF

  node :internal
  resistor :r1, p: :input, n: :internal, value: r
  resistor :r2, p: :internal, n: :output, value: r
  capacitor :c1, p: :output, n: :ground, value: c
end

circuit :two_stage do
  node :vin, :mid, :out
  voltage_source :src, p: :vin, n: :gnd, dc: 1.V

  instance :stage1, of: :lowpass,
    ports: { input: :vin, output: :mid, ground: :gnd },
    params: { r: 2.kohm, c: 47.nF }

  instance :stage2, of: :lowpass,
    ports: { input: :mid, output: :out, ground: :gnd }
end
"#);

    let circuit = &c.circuits[0];
    assert_eq!(circuit.name, "two_stage");

    // Two instances of a three-device subcircuit, plus the source.
    assert_eq!(circuit.devices.len(), 7);

    // Node isolation: each instance's `internal` node is its own, and neither
    // collides with the top-level `mid`.
    let n1 = circuit.node_id("stage1.internal").expect("stage1.internal");
    let n2 = circuit.node_id("stage2.internal").expect("stage2.internal");
    assert_ne!(n1, n2, "instance-local nodes must not be shared");
    assert_ne!(n1, circuit.node_id("mid").unwrap());
    assert_eq!(circuit.node(n1).unwrap().local_name, "internal");

    let s1 = circuit.device_id("stage1.r1").unwrap();
    let s2 = circuit.device_id("stage2.r1").unwrap();
    let d1 = circuit.device(s1).unwrap();
    let d2 = circuit.device(s2).unwrap();
    assert_eq!(
        d1.neg(),
        Some(n1),
        "r1 sits between the port and the internal node"
    );
    assert_eq!(d2.neg(), Some(n2));

    // Parameter override applies per instance.
    assert_eq!(
        d1.param("value").unwrap().value,
        2000.0,
        "stage1 overrides r"
    );
    assert_eq!(
        d2.param("value").unwrap().value,
        1000.0,
        "stage2 keeps the default"
    );
    assert_eq!(d1.param("value").unwrap().dimension, units::RESISTANCE);

    // Ports are bound to the caller's nodes.
    assert_eq!(d1.pos(), circuit.node_id("vin"));
    assert_eq!(d2.pos(), circuit.node_id("mid"));
    let s1r2 = circuit
        .device(circuit.device_id("stage1.r2").unwrap())
        .unwrap();
    assert_eq!(s1r2.pos(), Some(n1));
    assert_eq!(s1r2.neg(), circuit.node_id("mid"));
    let s2c1 = circuit
        .device(circuit.device_id("stage2.c1").unwrap())
        .unwrap();
    assert_eq!(
        s2c1.neg(),
        circuit.node_id("gnd"),
        "the ground port stays global"
    );

    // The instance chain is recorded, for diagnostics.
    assert_eq!(d1.instance_path.len(), 1);
    assert_eq!(d1.instance_path[0].instance, "stage1");
    assert_eq!(d1.instance_display("two_stage"), "two_stage.stage1.r1");

    // The subcircuit's ports are not nodes of their own: they are the
    // caller's nodes under a local name.
    assert!(circuit.node_id("stage1.output").is_none());
    assert!(circuit.node_id("stage1.input").is_none());
}

#[test]
fn a_missing_port_binding_is_reported() {
    let text = err(r#"
subcircuit :lp, ports: [:input, :output] do
  resistor :r1, p: :input, n: :output, value: 1.kohm
end
circuit :top do
  node :a, :b
  instance :x, of: :lp, ports: { input: :a }
end
"#);
    assert_code(&text, Code::Port);
    assert!(text.contains("output"), "{text}");
}

#[test]
fn an_unknown_port_binding_is_reported() {
    let text = err(r#"
subcircuit :lp, ports: [:input, :output] do
  resistor :r1, p: :input, n: :output, value: 1.kohm
end
circuit :top do
  node :a, :b
  instance :x, of: :lp, ports: { input: :a, output: :b, extra: :a }
end
"#);
    assert_code(&text, Code::Port);
    assert!(text.contains("extra"), "{text}");
    assert!(text.contains("declared ports"), "{text}");
}

#[test]
fn an_unknown_parameter_override_is_reported() {
    let text = err(r#"
subcircuit :lp, ports: [:input, :output] do
  param :r, default: 1.kohm
  resistor :r1, p: :input, n: :output, value: r
end
circuit :top do
  node :a, :b
  instance :x, of: :lp, ports: { input: :a, output: :b }, params: { q: 2.kohm }
end
"#);
    assert_code(&text, Code::Name);
    assert!(text.contains("declared parameters"), "{text}");
}

/// A parameter override with the wrong dimension must be caught, not applied.
#[test]
fn an_override_with_the_wrong_dimension_is_reported() {
    let text = err(r#"
subcircuit :lp, ports: [:input, :output] do
  param :r, default: 1.kohm
  resistor :r1, p: :input, n: :output, value: r
end
circuit :top do
  node :a, :b
  instance :x, of: :lp, ports: { input: :a, output: :b }, params: { r: 5.nF }
end
"#);
    assert_code(&text, Code::Dimension);
    assert!(text.contains("ohm"), "{text}");
}

#[test]
fn recursion_is_reported_with_the_call_chain() {
    let text = err(r#"
subcircuit :a, ports: [:i, :o] do
  instance :inner, of: :b, ports: { i: :i, o: :o }
end
subcircuit :b, ports: [:i, :o] do
  instance :inner, of: :a, ports: { i: :i, o: :o }
end
circuit :top do
  node :x, :y
  instance :t, of: :a, ports: { i: :x, o: :y }
end
"#);
    assert_code(&text, Code::Recursion);
    assert!(text.contains("call chain"), "{text}");
}

#[test]
fn a_circuit_cannot_be_instantiated_as_a_subcircuit() {
    let text = err(r#"
circuit :plain do
  node :a
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
end
circuit :top do
  node :x, :y
  instance :t, of: :plain, ports: { a: :x }
end
"#);
    assert_code(&text, Code::Name);
    assert!(text.contains("not a subcircuit"), "{text}");
}

#[test]
fn an_experiment_must_name_a_real_circuit() {
    let text = err(r#"
circuit :real do
  node :a
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
end
experiment :e, circuit: :imaginary do
  op
end
"#);
    assert_code(&text, Code::Name);
    assert!(text.contains("imaginary"), "{text}");
    assert!(text.contains("defined circuits"), "{text}");
}

// ---------------------------------------------------------------------------
// Stage 3: loops and conditionals
// ---------------------------------------------------------------------------

/// A range loop with several devices per iteration, each named from the loop
/// variable, produces a fully expanded circuit.
#[test]
fn a_loop_expands_into_distinct_devices() {
    let c = ok(r#"
circuit :ladder do
  node :n1, :n2
  voltage_source :v1, p: :n1, n: :gnd, dc: 1.V
  for i in 1..2 do
    resistor ("rs" + i), p: :n1, n: :gnd, value: i * 1.kohm
    resistor ("rp" + i), p: :n1, n: :n2, value: i * 2.kohm
  end
  resistor :rload, p: :n2, n: :gnd, value: 1.kohm
end
"#);
    let circuit = &c.circuits[0];
    // One source, two iterations of a two-device body, and the load.
    assert_eq!(circuit.devices.len(), 6);
    for name in ["rs1", "rs2", "rp1", "rp2"] {
        assert!(
            circuit.device_id(name).is_some(),
            "missing `{name}`; have {:?}",
            circuit
                .devices
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>()
        );
    }
    // The loop variable is usable as a value, not only as part of a name.
    let rs2 = circuit.device(circuit.device_id("rs2").unwrap()).unwrap();
    assert_eq!(rs2.param("value").unwrap().value, 2000.0);
}

#[test]
fn a_loop_over_a_list_repeats_the_body() {
    let c = ok(r#"
circuit :many do
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  for k in [2, 3, 4] do
    resistor ("r" + k), p: :a, n: :b, value: 1.kohm
  end
end
"#);
    let circuit = &c.circuits[0];
    // Three iterations, three distinct devices, named from the loop variable.
    for expected in ["r2", "r3", "r4"] {
        assert!(
            circuit.device_id(expected).is_some(),
            "expected a device `{expected}`; have {:?}",
            circuit
                .devices
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(circuit.devices.len(), 4, "source plus three resistors");
}

#[test]
fn duplicate_names_from_a_loop_are_reported() {
    let text = err(r#"
circuit :many do
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  for k in [1, 2, 3] do
    resistor :r, p: :a, n: :b, value: 1.kohm
  end
end
"#);
    assert_code(&text, Code::Duplicate);
}

#[test]
fn a_loop_variable_can_name_devices_distinctly() {
    let c = ok(r#"
circuit :ok_loop do
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  for k in 1..3 do
    resistor ("rx" + k), p: :a, n: :b, value: k * 1.kohm
  end
end
"#);
    let circuit = &c.circuits[0];
    // `1..3` is inclusive, so three devices carry the loop value as their value.
    for (name, ohms) in [("rx1", 1000.0), ("rx2", 2000.0), ("rx3", 3000.0)] {
        let id = circuit
            .device_id(name)
            .unwrap_or_else(|| panic!("missing `{name}`"));
        let d = circuit.device(id).unwrap();
        assert_eq!(d.param("value").unwrap().value, ohms, "for {name}");
        assert_eq!(d.param("value").unwrap().dimension, units::RESISTANCE);
    }
}

#[test]
fn a_computed_name_must_be_a_legal_identifier() {
    let text = err(r#"
circuit :bad_name do
  node :a
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  for k in 1..2 do
    resistor ("2r" + k), p: :a, n: :gnd, value: 1.kohm
  end
end
"#);
    assert_code(&text, Code::Value);
    assert!(text.contains("not a usable name"), "{text}");
}

#[test]
fn conditional_expansion_follows_the_taken_branch() {
    let c = ok(r#"
circuit :cond do
  param :n, default: 4
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  if n > 2 do
    resistor :big, p: :a, n: :b, value: 2.kohm
  else
    resistor :small, p: :a, n: :b, value: 1.kohm
  end
end
"#);
    let circuit = &c.circuits[0];
    assert!(
        circuit.device_id("big").is_some(),
        "the taken branch must elaborate"
    );
    assert!(
        circuit.device_id("small").is_none(),
        "the untaken branch must not"
    );
}

#[test]
fn a_non_boolean_condition_is_reported() {
    let text = err(r#"
circuit :bad_cond do
  node :a
  if 1.kohm do
    resistor :r, p: :a, n: :gnd, value: 1.kohm
  end
end
"#);
    assert_code(&text, Code::Type);
    assert!(text.contains("boolean"), "{text}");
}

#[test]
fn comparing_different_dimensions_is_reported() {
    let text = err(r#"
circuit :cmp do
  node :a
  if 1.kohm > 1.us do
    resistor :r, p: :a, n: :gnd, value: 1.kohm
  end
end
"#);
    assert_code(&text, Code::Dimension);
}

// ---------------------------------------------------------------------------
// Stage 2: analyses
// ---------------------------------------------------------------------------

#[test]
fn ac_needs_a_positive_increasing_range() {
    let text = err(r#"
circuit :a do
  node :x
  voltage_source :v1, p: :x, n: :gnd, ac: 1.V
end
experiment :e, circuit: :a do
  ac from: 1.MHz, to: 1.kHz, points_per_decade: 10
end
"#);
    assert_code(&text, Code::Sweep);
}

#[test]
fn ac_rejects_two_point_specifications() {
    let text = err(r#"
circuit :a do
  node :x
  voltage_source :v1, p: :x, n: :gnd, ac: 1.V
end
experiment :e, circuit: :a do
  ac from: 1.Hz, to: 1.kHz, points: 10, points_per_decade: 10
end
"#);
    assert_code(&text, Code::Argument);
}

#[test]
fn ac_rejects_a_frequency_written_with_the_wrong_unit() {
    let text = err(r#"
circuit :a do
  node :x
  voltage_source :v1, p: :x, n: :gnd, ac: 1.V
end
experiment :e, circuit: :a do
  ac from: 1.ms, to: 1.kHz, points_per_decade: 10
end
"#);
    assert_code(&text, Code::Dimension);
}

#[test]
fn tran_validates_its_window() {
    let text = err(r#"
circuit :a do
  node :x
  voltage_source :v1, p: :x, n: :gnd, dc: 1.V
end
experiment :e, circuit: :a do
  tran stop: 1.us, start: 5.us
end
"#);
    assert_code(&text, Code::Sweep);
}

#[test]
fn a_pulse_that_cannot_fit_its_period_is_reported() {
    let text = err(r#"
circuit :a do
  node :x
  voltage_source :v1, p: :x, n: :gnd,
    waveform: pulse(low: 0.V, high: 1.V, delay: 5.us, rise: 1.ns,
                    fall: 1.ns, width: 10.us, period: 10.us)
end
"#);
    assert_code(&text, Code::Value);
    assert!(text.contains("period"), "{text}");
}

#[test]
fn an_experiment_without_an_analysis_is_reported() {
    let text = err(r#"
circuit :a do
  node :x
  voltage_source :v1, p: :x, n: :gnd, dc: 1.V
end
experiment :e, circuit: :a do
  save v(:x)
end
"#);
    assert_code(&text, Code::Argument);
    assert!(text.contains("no analysis"), "{text}");
}

// ---------------------------------------------------------------------------
// DC sweeps
// ---------------------------------------------------------------------------

#[test]
fn a_dc_source_sweep_resolves_the_device() {
    let c = ok(r#"
circuit :a do
  node :x, :y
  voltage_source :v1, p: :x, n: :gnd, dc: 0.V
  resistor :r1, p: :x, n: :y, value: 1.kohm
  resistor :r2, p: :y, n: :gnd, value: 1.kohm
end
experiment :e, circuit: :a do
  dc source: :v1, from: 0.V, to: 5.V, step: 1.V
  save v(:y)
end
"#);
    match &c.experiments[0].plan.tasks[0].kind {
        AnalysisKind::Dc(d) => {
            assert_eq!(d.sweep.start, 0.0);
            assert_eq!(d.sweep.stop, 5.0);
            assert_eq!(d.sweep.step, Some(1.0));
            match &d.sweep.target {
                SweepTarget::SourceValue { name, .. } => assert_eq!(name, "v1"),
                other => panic!("expected a source sweep, got {other:?}"),
            }
        }
        other => panic!("expected dc, got {other:?}"),
    }
}

#[test]
fn a_dc_sweep_of_a_non_source_is_reported() {
    let text = err(r#"
circuit :a do
  node :x, :y
  voltage_source :v1, p: :x, n: :gnd, dc: 0.V
  resistor :r1, p: :x, n: :y, value: 1.kohm
end
experiment :e, circuit: :a do
  dc source: :r1, from: 0.V, to: 5.V, step: 1.V
end
"#);
    assert_code(&text, Code::Sweep);
    assert!(text.contains("not a source"), "{text}");
}

#[test]
fn a_dc_sweep_needs_a_step_or_points() {
    let text = err(r#"
circuit :a do
  node :x
  voltage_source :v1, p: :x, n: :gnd, dc: 0.V
end
experiment :e, circuit: :a do
  dc source: :v1, from: 0.V, to: 5.V
end
"#);
    assert_code(&text, Code::Argument);
}

// ---------------------------------------------------------------------------
// Measurements
// ---------------------------------------------------------------------------

#[test]
fn measurements_resolve_their_target() {
    let c = ok(r#"
circuit :a do
  node :x, :y
  voltage_source :v1, p: :x, n: :gnd, dc: 1.V
  resistor :r1, p: :x, n: :y, value: 1.kohm
  capacitor :c1, p: :y, n: :gnd, value: 1.uF
end
experiment :e, circuit: :a do
  tran stop: 10.ms, max_step: 10.us
  save v(:y)
  measure :ymax, max: v(:y)
  measure :yrms, rms: v(:y)
  measure :yavg, avg: v(:y)
  measure :ymin, min: v(:y)
end
"#);
    let m = &c.experiments[0].plan.measures;
    assert_eq!(m.len(), 4);
    assert_eq!(m[0].name, "ymax");
    assert_eq!(m[1].kind.name(), "rms");
    assert!(m[1].kind.needs_time_axis());
    assert!(!m[0].kind.needs_time_axis());
}

#[test]
fn an_unknown_measurement_kind_is_reported() {
    let text = err(r#"
circuit :a do
  node :x
  voltage_source :v1, p: :x, n: :gnd, dc: 1.V
end
experiment :e, circuit: :a do
  op
  measure :m, median: v(:x)
end
"#);
    assert_code(&text, Code::Unsupported);
    assert!(text.contains("available: max, min, avg, rms"), "{text}");
}

#[test]
fn a_probe_on_an_unknown_node_is_reported_with_the_available_ones() {
    let text = err(r#"
circuit :a do
  node :x
  voltage_source :v1, p: :x, n: :gnd, dc: 1.V
end
experiment :e, circuit: :a do
  op
  save v(:nope)
end
"#);
    assert_code(&text, Code::Name);
    assert!(text.contains("nodes:"), "{text}");
}

#[test]
fn a_probe_on_an_unknown_device_is_reported() {
    let text = err(r#"
circuit :a do
  node :x
  voltage_source :v1, p: :x, n: :gnd, dc: 1.V
end
experiment :e, circuit: :a do
  op
  save i(:nope)
end
"#);
    assert_code(&text, Code::Name);
    assert!(text.contains("devices:"), "{text}");
}

#[test]
fn saving_the_same_probe_twice_is_reported() {
    let text = err(r#"
circuit :a do
  node :x
  voltage_source :v1, p: :x, n: :gnd, dc: 1.V
end
experiment :e, circuit: :a do
  op
  save v(:x), v(:x)
end
"#);
    assert_code(&text, Code::Duplicate);
}

// ---------------------------------------------------------------------------
// Diodes
// ---------------------------------------------------------------------------

#[test]
fn a_diode_requires_a_declared_model() {
    let text = err(r#"
circuit :r do
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 5.V
  diode :d1, p: :b, n: :gnd, model: :missing
end
"#);
    assert_code(&text, Code::Name);
    assert!(text.contains("declare it"), "{text}");
}

#[test]
fn a_diode_with_a_model_elaborates() {
    let c = ok(r#"
circuit :r do
  node :a, :b
  model :dmod, type: :diode, is: 1e-14.A, n: 1
  voltage_source :v1, p: :a, n: :gnd, dc: 5.V
  resistor :r1, p: :a, n: :b, value: 1.kohm
  diode :d1, p: :b, n: :gnd, model: :dmod
end
"#);
    let circuit = &c.circuits[0];
    let d = circuit.device(circuit.device_id("d1").unwrap()).unwrap();
    assert_eq!(d.kind, DeviceKind::Diode);
    assert!(d.model.is_some());
    assert_eq!(d.pos(), circuit.node_id("b"), "p is the anode");
    assert_eq!(d.neg(), circuit.node_id("gnd"), "n is the cathode");

    let m = circuit.model(d.model.unwrap()).unwrap();
    assert_eq!(m.name, "dmod");
    assert_eq!(m.params["is"].value, 1e-14);
}

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

#[test]
fn the_device_limit_is_enforced() {
    let mut src = String::from(
        "circuit :big do\n  node :a, :b\n  voltage_source :v1, p: :a, n: :gnd, dc: 1.V\n",
    );
    // Distinct names so the duplicate check is not what fires.
    for i in 0..200 {
        src.push_str(&format!("  resistor :r{i}, p: :a, n: :b, value: 1.kohm\n"));
    }
    src.push_str("end\n");

    let mut sm = SourceMap::new();
    let id = sm.add("t.cdsl", src.as_str());
    let tokens = lex(id, &src).unwrap();
    let program = parse(&tokens).unwrap();
    let err = compile(&program, &Limits::for_tests()).unwrap_err();
    let text = err.render(&sm);
    assert_code(&text, Code::Limit);
    assert!(text.contains("devices"), "{text}");
}

#[test]
fn the_loop_limit_is_enforced() {
    let text = err(r#"
circuit :big_loop do
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  for i in 1..1000000 do
    resistor :r, p: :a, n: :b, value: 1.kohm
  end
end
"#);
    assert_code(&text, Code::Limit);
}

// ---------------------------------------------------------------------------
// Sweep-point re-elaboration (what the parameter sweep driver calls)
// ---------------------------------------------------------------------------

#[test]
fn re_elaborating_with_an_override_changes_the_value_not_the_topology() {
    let src = r#"
circuit :param_circuit do
  param :r, default: 1.kohm
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  resistor :r1, p: :a, n: :b, value: r
  resistor :r2, p: :b, n: :gnd, value: 1.kohm
end
experiment :e, circuit: :param_circuit do
  op
  save v(:b), i(:r1)
end
"#;
    let mut sm = SourceMap::new();
    let id = sm.add("t.cdsl", src);
    let tokens = lex(id, src).unwrap();
    let program = parse(&tokens).unwrap();

    let base = elaborate_experiment(&program, "e", &[], &Limits::default()).expect("default");

    let swept = elaborate_experiment(
        &program,
        "e",
        &[("r".to_string(), units::Quantity::ohms(4700.0))],
        &Limits::default(),
    )
    .expect("override");

    let r1_base = base
        .circuit
        .device(base.circuit.device_id("r1").unwrap())
        .unwrap();
    let r1_swept = swept
        .circuit
        .device(swept.circuit.device_id("r1").unwrap())
        .unwrap();
    assert_eq!(r1_base.param("value").unwrap().value, 1000.0);
    assert_eq!(r1_swept.param("value").unwrap().value, 4700.0);

    // Topology must be identical, which is what makes a parameter sweep legal.
    assert_eq!(base.circuit.nodes.len(), swept.circuit.nodes.len());
    assert_eq!(base.circuit.devices.len(), swept.circuit.devices.len());
    for (a, b) in base.circuit.devices.iter().zip(&swept.circuit.devices) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.terminals, b.terminals);
    }
}

#[test]
fn an_unknown_experiment_name_is_reported() {
    let src = "circuit :a do\n  node :x\n  voltage_source :v1, p: :x, n: :gnd, dc: 1.V\nend\n";
    let mut sm = SourceMap::new();
    let id = sm.add("t.cdsl", src);
    let tokens = lex(id, src).unwrap();
    let program = parse(&tokens).unwrap();
    let err = elaborate_experiment(&program, "nope", &[], &Limits::default()).unwrap_err();
    assert!(err.render(&sm).contains("nope"));
}

// ---------------------------------------------------------------------------
// Names computed at elaboration time
// ---------------------------------------------------------------------------

#[test]
fn a_loop_can_build_a_ladder_from_computed_node_names() {
    // A computed node name is only useful if it can then be connected to; a
    // rung whose nodes cannot be named has to be written out by hand.
    let c = ok(r#"
circuit :ladder do
  param :taps, default: 3
  node :top, :bot
  for k in 1..taps do
    node ("mid" + k)
  end
  voltage_source :src, p: :top, n: :gnd, dc: 3.V
  resistor :r0, p: :top, n: ("mid" + 1), value: 1.kohm
  for k in 1..(taps - 1) do
    resistor ("r" + k), p: ("mid" + k), n: ("mid" + (k + 1)), value: 1.kohm
  end
  resistor ("rt" + taps), p: ("mid" + taps), n: :bot, value: 1.kohm
  for k in 1..taps do
    resistor ("rs" + k), p: ("mid" + k), n: :gnd, value: 2.kohm
  end
  resistor :rbot, p: :bot, n: :gnd, value: 1.kohm
end
"#);
    let circuit = &c.circuits[0];
    for name in ["mid1", "mid2", "mid3"] {
        assert!(
            circuit.node_id(name).is_some(),
            "node {name} was not created: {:?}",
            circuit.nodes.iter().map(|n| &n.name).collect::<Vec<_>>()
        );
    }
    // Three rungs plus the tail, the head, the bottom leg, the source and the
    // three shunts.
    assert_eq!(circuit.devices.len(), 9, "devices: {:?}", circuit.devices);
    // The chain really is a chain: r1 couples mid1 to mid2.
    let r1 = circuit
        .device(circuit.device_id("r1").expect("r1"))
        .unwrap();
    let mid1 = circuit.node_id("mid1").unwrap();
    let mid2 = circuit.node_id("mid2").unwrap();
    let terminals: Vec<_> = r1.terminals.iter().map(|(_, n)| *n).collect();
    assert!(
        terminals.contains(&mid1) && terminals.contains(&mid2),
        "{r1:?}"
    );
}

#[test]
fn a_terminal_that_is_a_bare_word_is_told_to_write_a_colon() {
    // `n: out` is the common typo; the message must name both fixes rather
    // than reporting an unknown variable.
    let text = err(r#"
circuit :typo do
  node :out
  resistor :r1, p: :out, n: out, value: 1.kohm
end
"#);
    assert_code(&text, Code::Type);
    assert!(text.contains(":out"), "{text}");
    assert!(text.contains("param :out"), "{text}");
}

#[test]
fn a_computed_terminal_must_evaluate_to_a_name() {
    let text = err(r#"
circuit :bad do
  node :out
  resistor :r1, p: :out, n: (1 + 2), value: 1.kohm
end
"#);
    assert_code(&text, Code::Type);
    assert!(text.contains("string or symbol"), "{text}");
}

#[test]
fn a_computed_terminal_naming_an_undeclared_node_is_reported_as_such() {
    let text = err(r#"
circuit :bad do
  node :out
  resistor :r1, p: :out, n: ("nope" + 1), value: 1.kohm
end
"#);
    assert_code(&text, Code::Name);
    assert!(text.contains("nope1"), "{text}");
}

#[test]
fn a_computed_terminal_cannot_smuggle_in_a_dotted_or_empty_name() {
    for bad in [r#"("a" + "." + "b")"#, r#""""#] {
        let src = format!(
            "circuit :bad do\n  node :out\n  resistor :r1, p: :out, n: {bad}, value: 1.kohm\nend\n"
        );
        let text = err(&src);
        assert_code(&text, Code::Value);
    }
}

// ---------------------------------------------------------------------------
// Hierarchy paths in probes
// ---------------------------------------------------------------------------

const TWO_STAGE: &str = r#"
subcircuit :lowpass, ports: [:input, :output, :ground] do
  node :internal
  resistor :r1, p: :input, n: :internal, value: 1.kohm
  resistor :r2, p: :internal, n: :output, value: 1.kohm
end
circuit :top do
  node :vin, :out
  voltage_source :src, p: :vin, n: :gnd, dc: 3.V
  instance :stage1, of: :lowpass, ports: { input: :vin, output: :out, ground: :gnd }
end
"#;

#[test]
fn a_hierarchy_path_reaches_inside_an_instance() {
    let c = ok(&format!(
        "{TWO_STAGE}
experiment :x, circuit: :top do
  op
  save v(:stage1.internal), i(:stage1.r1)
end
"
    ));
    let circuit = &c.circuits[0];
    let probes = &c.experiments[0].plan.tasks[0].probes;

    assert_eq!(probes[0].name, "v(stage1.internal)");
    assert!(matches!(
        probes[0].probe,
        Probe::NodeVoltage(n) if Some(n) == circuit.node_id("stage1.internal")
    ));

    assert_eq!(probes[1].name, "i(stage1.r1)");
    let r1 = circuit.device_id("stage1.r1").expect("stage1.r1");
    assert!(matches!(probes[1].probe, Probe::DeviceCurrent(d) if d == r1));
}

#[test]
fn a_hierarchy_path_may_be_written_as_a_string() {
    // The two spellings must denote the same thing, or one of them is a trap.
    // They are compared across two experiments because saving one probe twice
    // under two spellings is, correctly, a duplicate.
    let c = ok(&format!(
        "{TWO_STAGE}
experiment :symbols, circuit: :top do
  op
  save v(:stage1.internal), i(:stage1.r1)
end
experiment :strings, circuit: :top do
  op
  save v(\"stage1.internal\"), i(\"stage1.r1\")
end
"
    ));
    let by_symbol = &c.experiment("symbols").unwrap().plan.tasks[0].probes;
    let by_string = &c.experiment("strings").unwrap().plan.tasks[0].probes;
    assert_eq!(by_symbol[0].name, "v(stage1.internal)");
    assert_eq!(by_symbol[0].name, by_string[0].name);
    assert_eq!(by_symbol[1].name, by_string[1].name);
    assert!(matches!(by_string[0].probe, Probe::NodeVoltage(_)));
    assert!(matches!(by_string[1].probe, Probe::DeviceCurrent(_)));
}

#[test]
fn an_ambiguous_leaf_name_is_refused_and_lists_the_paths() {
    // Silently answering with the first instance's node is the failure this
    // guards: the value would be wrong and nothing would say so.
    let text = err(r#"
subcircuit :lowpass, ports: [:input, :output, :ground] do
  node :internal
  resistor :r1, p: :input, n: :internal, value: 1.kohm
  resistor :r2, p: :internal, n: :output, value: 1.kohm
end
circuit :top do
  node :vin, :mid, :out
  voltage_source :src, p: :vin, n: :gnd, dc: 3.V
  instance :stage1, of: :lowpass, ports: { input: :vin, output: :mid, ground: :gnd }
  instance :stage2, of: :lowpass, ports: { input: :mid, output: :out, ground: :gnd }
end
experiment :x, circuit: :top do
  op
  save v(:internal), i(:r1)
end
"#);
    assert_code(&text, Code::Name);
    assert!(text.contains("names 2 different nodes"), "{text}");
    assert!(text.contains("v(:stage1.internal)"), "{text}");
    assert!(text.contains("v(:stage2.internal)"), "{text}");
    assert!(text.contains("names 2 different devices"), "{text}");
    assert!(text.contains("i(:stage2.r1)"), "{text}");
}

#[test]
fn an_unambiguous_leaf_name_still_resolves() {
    // The ambiguity rule must not break the single-instance convenience.
    let c = ok(&format!(
        "{TWO_STAGE}
experiment :x, circuit: :top do
  op
  save v(:internal)
end
"
    ));
    let probes = &c.experiments[0].plan.tasks[0].probes;
    assert_eq!(probes[0].name, "v(stage1.internal)");
}

#[test]
fn a_hierarchy_path_that_matches_nothing_is_reported() {
    let text = err(&format!(
        "{TWO_STAGE}
experiment :x, circuit: :top do
  op
  save v(:stage9.internal)
end
"
    ));
    assert_code(&text, Code::Name);
    assert!(text.contains("stage9.internal"), "{text}");
}
