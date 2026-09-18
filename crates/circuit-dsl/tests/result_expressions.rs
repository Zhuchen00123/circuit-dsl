//! Result expressions, derived signals and compound measures (round 3).
//!
//! The front end evidence for the frozen contract in
//! docs/review-evidence/round3/design-contract.md: what parses, how an
//! expression lowers, how a request binds to an analysis, which probes become
//! implicit dependencies, and which shapes are refused at check time.
//!
//! Every test drives the real front end (lex, parse, elaborate) rather than a
//! hand-built AST, so a change to the grammar or to the elaborator cannot pass
//! by accident here.

use circuit_core::diagnostic::Code;
use circuit_core::plan::{
    AnalysisBinding, AnalysisPlan, ExprIr, MeasureKind, NamedProbe, Probe, ProbeRef,
};
use circuit_core::units;
use circuit_core::{Limits, SourceMap};
use circuit_dsl::{compile, lex, parse};

/// The RC filter every test runs against. `v(:vin)`, `v(:vout)`, `v(:vin, :gnd)`
/// and `i(:r1)` all resolve on it.
const RC: &str = r#"circuit :rc do
  node :vin, :vout
  voltage_source :src, p: :vin, n: :gnd, dc: 1.V
  resistor :r1, p: :vin, n: :vout, value: 1.kohm
  capacitor :c1, p: :vout, n: :gnd, value: 100.nF
end
"#;

/// The shared circuit plus one experiment whose body is `body`.
fn experiment_with(body: &str) -> String {
    format!("{RC}\nexperiment :e, circuit: :rc do\n{body}\nend\n")
}

struct Outcome {
    result: Result<circuit_dsl::Compiled, String>,
}

/// The DC parameter-sweep experiment shape used by the sweep tests.
const SWEEP_SETUP: &str = r#"circuit :div do
  param :r, default: 1.kohm
  param :c, default: 100.nF
  node :out
  voltage_source :src, p: :out, n: :gnd, dc: 3.V
  resistor :r1, p: :out, n: :gnd, value: r
  capacitor :c1, p: :out, n: :gnd, value: c
end
"#;

/// Run a whole source, not the shared RC experiment.
fn run_source(src: &str) -> Result<circuit_dsl::Compiled, String> {
    let mut sm = SourceMap::new();
    let id = sm.add("test.cdsl", src);
    let tokens = lex(id, src).map_err(|d| d.render(&sm))?;
    let parsed = parse(&tokens).map_err(|d| d.render(&sm))?;
    compile(&parsed, &Limits::default()).map_err(|d| d.render(&sm))
}

/// Run the whole front end, rendering whatever went wrong.
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
    let parsed = match parse(&tokens) {
        Ok(p) => p,
        Err(d) => {
            return Outcome {
                result: Err(d.render(&sm)),
            };
        }
    };
    match compile(&parsed, &Limits::default()) {
        Ok(c) => Outcome { result: Ok(c) },
        Err(d) => Outcome {
            result: Err(d.render(&sm)),
        },
    }
}

/// Compile an experiment body and expect success.
fn ok(body: &str) -> circuit_dsl::Compiled {
    match run(&experiment_with(body)).result {
        Ok(c) => c,
        Err(text) => panic!("expected success, got diagnostics:\n{text}"),
    }
}

/// The plan of the single experiment.
fn plan_of(body: &str) -> AnalysisPlan {
    ok(body)
        .experiments
        .into_iter()
        .next()
        .expect("one experiment")
        .plan
}

/// Compile an experiment body and expect diagnostics.
fn err(body: &str) -> String {
    match run(&experiment_with(body)).result {
        Ok(_) => panic!("expected diagnostics, but compilation succeeded"),
        Err(text) => text,
    }
}

fn assert_code(text: &str, code: Code) {
    let needle = format!("[{}]", code.as_str());
    assert!(text.contains(&needle), "expected {needle} in:\n{text}");
}

fn probe_names(probes: &[NamedProbe]) -> Vec<String> {
    probes.iter().map(|p| p.name.clone()).collect()
}

fn read_names(probes: &[ProbeRef]) -> Vec<String> {
    probes.iter().map(|p| p.name.clone()).collect()
}

// ---------------------------------------------------------------------------
// Lowering
// ---------------------------------------------------------------------------

#[test]
fn a_derive_lowers_probe_reads_into_the_plan_ir() {
    let plan = plan_of(
        "ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  derive :gain, expr: v(:vout) / v(:vin)",
    );
    assert_eq!(plan.derives.len(), 1);
    let derive = &plan.derives[0];
    assert_eq!(derive.name, "gain");
    assert_eq!(derive.binding, AnalysisBinding::Analysis(plan.tasks[0].id));

    let ExprIr::Div(numerator, denominator) = &derive.expr else {
        panic!("expected a division, got {:?}", derive.expr);
    };
    let ExprIr::Probe(top) = &**numerator else {
        panic!("expected a probe read");
    };
    // The name is the canonical one the backend produces, not the source text.
    assert_eq!(top.name, "v(vout)");
    assert_eq!(top.dimension(), units::VOLTAGE);
    let ExprIr::Probe(bottom) = &**denominator else {
        panic!("expected a probe read");
    };
    assert_eq!(bottom.name, "v(vin)");

    // The written expression is kept for reporting and export metadata.
    assert!(derive.source.contains("v(vout)"), "{}", derive.source);
    assert_eq!(derive.expr.static_dimension(), Some(units::DIMENSIONLESS));
    assert_eq!(derive.expr.static_dimension_error(), None);
    assert!(!derive.span.is_synthetic());
    assert!(!derive.name_span.is_synthetic());
}

#[test]
fn a_two_node_probe_becomes_a_differential_probe() {
    let plan = plan_of(
        "op
  derive :difference, expr: v(:vin, :gnd)",
    );
    let ExprIr::Probe(probe) = &plan.derives[0].expr else {
        panic!("expected one probe read");
    };
    assert_eq!(probe.name, "v(vin,gnd)");
    assert!(matches!(probe.probe, Probe::DifferentialVoltage { .. }));
    assert_eq!(probe.dimension(), units::VOLTAGE);

    // A device current resolves through the same path.
    let plan = plan_of(
        "op
  derive :branch, expr: i(:r1) * 2",
    );
    let ExprIr::Mul(left, _) = &plan.derives[0].expr else {
        panic!("expected a multiplication");
    };
    let ExprIr::Probe(current) = &**left else {
        panic!("expected a probe read");
    };
    assert_eq!(current.name, "i(r1)");
    assert_eq!(current.dimension(), units::CURRENT);
}

#[test]
fn functions_and_nested_arithmetic_lower_to_the_matching_nodes() {
    let plan = plan_of(
        "ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  derive :gain_db, expr: gain_db(v(:vout), v(:vin))
  derive :peak, expr: max(abs(v(:vout) * 2 + v(:vin)), abs(v(:vin)))
  derive :scaled, expr: -sqrt(4.0) * abs(v(:vout))",
    );

    let ExprIr::GainDb {
        numerator,
        denominator,
    } = &plan.derives[0].expr
    else {
        panic!("expected gain_db");
    };
    assert_eq!(numerator.static_dimension(), Some(units::VOLTAGE));
    assert_eq!(denominator.static_dimension(), Some(units::VOLTAGE));
    assert_eq!(
        plan.derives[0].expr.static_dimension(),
        Some(units::DIMENSIONLESS)
    );

    let ExprIr::Max(left, right) = &plan.derives[1].expr else {
        panic!("expected max");
    };
    // max(abs((v(:vout) * 2) + v(:vin)), abs(v(:vin)))
    let ExprIr::Abs(inner) = &**left else {
        panic!("expected abs as the first argument");
    };
    let ExprIr::Add(sum_left, sum_right) = &**inner else {
        panic!("expected a sum inside abs");
    };
    assert!(matches!(**sum_left, ExprIr::Mul(_, _)));
    assert!(matches!(**sum_right, ExprIr::Probe(_)));
    assert!(matches!(**right, ExprIr::Abs(_)));
    assert_eq!(
        plan.derives[1].expr.static_dimension(),
        Some(units::VOLTAGE)
    );

    // Unary minus survives, sqrt(4.0) is a plain number, abs keeps the unit.
    let ExprIr::Mul(left, right) = &plan.derives[2].expr else {
        panic!("expected a multiplication");
    };
    assert!(matches!(**left, ExprIr::Neg(_)));
    assert!(matches!(**right, ExprIr::Abs(_)));
    assert_eq!(
        plan.derives[2].expr.static_dimension(),
        Some(units::VOLTAGE)
    );
}

#[test]
fn the_dependency_set_is_deduplicated_in_first_seen_order() {
    let plan = plan_of(
        "ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  derive :ratio, expr: v(:vout) / v(:vin) * v(:vout)",
    );
    assert_eq!(
        read_names(&plan.derives[0].expr.probes()),
        vec!["v(vout)".to_string(), "v(vin)".to_string()]
    );
    // Deduplicated by name, and read from exactly one task.
    assert_eq!(
        probe_names(&plan.tasks[0].implicit_probes),
        vec!["v(vout)".to_string(), "v(vin)".to_string()]
    );
}

// ---------------------------------------------------------------------------
// What the result language refuses
// ---------------------------------------------------------------------------

#[test]
fn an_unknown_function_lists_the_available_ones() {
    let text = err("op
  derive :x, expr: nosuch(v(:vin))");
    assert_code(&text, Code::Name);
    assert!(text.contains("unknown function `nosuch`"), "{text}");
    assert!(text.contains("gain_db(a, b)"), "{text}");
}

#[test]
fn a_bare_identifier_is_refused_as_a_signal_reference() {
    let text = err("op
  derive :x, expr: r1 + 1");
    assert_code(&text, Code::Type);
    assert!(text.contains("`r1` is not a signal"), "{text}");
    assert!(text.contains("v(:node)"), "{text}");
    // The version limit must be stated where a user would hit it.
    assert!(
        text.contains("cannot be referenced by another result expression"),
        "{text}"
    );
}

#[test]
fn a_literal_with_a_unit_is_refused() {
    let text = err("op
  derive :x, expr: 1.kohm");
    assert_code(&text, Code::Type);
    assert!(text.contains("the unit ohm"), "{text}");
}

#[test]
fn comparisons_and_logic_are_refused() {
    for body in [
        "op
  derive :x, expr: v(:vin) > 1",
        "op
  derive :x, expr: v(:vin) == 1",
        "op
  derive :x, expr: v(:vin) && 1",
        "op
  derive :x, expr: !v(:vin)",
    ] {
        let text = err(body);
        assert_code(&text, Code::Type);
        assert!(
            text.contains("not available in a result expression")
                || text.contains("not a result expression"),
            "{body} gave:\n{text}"
        );
    }
}

#[test]
fn arrays_dicts_strings_and_symbols_are_refused() {
    for (body, what) in [
        (
            "op
  derive :x, expr: [1, 2]",
            "an array",
        ),
        (
            "op
  derive :x, expr: { a: 1 }",
            "a dictionary",
        ),
        (
            "op
  derive :x, expr: :vin",
            "the bare symbol",
        ),
        (
            "op
  derive :x, expr: \"vin\"",
            "a string",
        ),
    ] {
        let text = err(body);
        assert_code(&text, Code::Type);
        assert!(text.contains(what), "{body} gave:\n{text}");
    }
}

#[test]
fn mismatched_units_are_a_dimension_error() {
    let text = err("op
  derive :x, expr: v(:vin) + i(:r1)");
    assert_code(&text, Code::Dimension);
    assert!(text.contains("the units differ"), "{text}");

    // A gain needs like quantities on both sides.
    let text = err("op
  derive :x, expr: gain_db(v(:vin), i(:r1))");
    assert_code(&text, Code::Dimension);
    assert!(text.contains("ratio of like quantities"), "{text}");
}

#[test]
fn a_wrong_argument_count_is_an_argument_error() {
    let text = err("op
  derive :x, expr: gain_db(v(:vin))");
    assert_code(&text, Code::Argument);
    assert!(text.contains("takes 2 argument(s), found 1"), "{text}");

    let text = err("op
  derive :x, expr: min(v(:vin))");
    assert_code(&text, Code::Argument);
    assert!(text.contains("usage: min(a, b)"), "{text}");
}

#[test]
fn an_unknown_probe_is_reported_by_the_save_resolution_rules() {
    let text = err("op
  derive :x, expr: v(:nope) / v(:vin)");
    assert_code(&text, Code::Name);
    assert!(text.contains("unknown node `:nope`"), "{text}");
}

// ---------------------------------------------------------------------------
// Binding
// ---------------------------------------------------------------------------

#[test]
fn one_analysis_binds_implicitly() {
    let plan = plan_of(
        "op
  derive :peak, expr: v(:vin)
  measure :scaled, max: v(:vin) * 2",
    );
    let only = plan.tasks[0].id;
    assert_eq!(plan.derives[0].binding, AnalysisBinding::Analysis(only));
    assert_eq!(plan.measures[0].binding, AnalysisBinding::Analysis(only));
}

#[test]
fn a_new_expression_needs_a_binding_when_several_analyses_run() {
    let text = err("ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  op
  measure :peak, max: abs(v(:vout))");
    assert_code(&text, Code::Ambiguous);
    assert!(text.contains("ac1") && text.contains("op1"), "{text}");
    assert!(text.contains("this measure"), "{text}");
    // The fix must be the exact text a user has to type.
    assert!(text.contains("add `analysis: :ac1`"), "{text}");

    // A derive is always a new expression, so even a bare probe read must be
    // bound explicitly; the legacy exception does not apply to it.
    let text = err("ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  op
  derive :x, expr: v(:vout)");
    assert_code(&text, Code::Ambiguous);
    assert!(text.contains("this derive"), "{text}");
}

#[test]
fn a_bare_probe_measure_keeps_the_legacy_rule() {
    let plan = plan_of(
        "ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  tran stop: 1.ms, max_step: 10.us
  measure :vmax, max: v(:vout)",
    );
    assert_eq!(plan.measures[0].binding, AnalysisBinding::LegacyPreferred);
    // The legacy rule may pick any analysis, so every task must be able to
    // produce the probe it reads.
    assert_eq!(plan.tasks.len(), 2);
    for task in &plan.tasks {
        assert_eq!(
            probe_names(&task.implicit_probes),
            vec!["v(vout)".to_string()],
            "task {:?}",
            task.id
        );
    }
}

#[test]
fn an_explicit_binding_selects_the_analysis() {
    let plan = plan_of(
        "ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  ac from: 1.kHz, to: 10.kHz, points_per_decade: 10
  tran stop: 1.ms, max_step: 10.us
  derive :gain, expr: v(:vout) / v(:vin), analysis: :ac2
  measure :mean, avg: v(:vout), analysis: :tran1",
    );
    assert_eq!(plan.analysis_names(), vec!["ac1", "ac2", "tran1"]);
    assert_eq!(
        plan.derives[0].binding,
        AnalysisBinding::Analysis(plan.tasks[1].id)
    );
    assert_eq!(
        plan.measures[0].binding,
        AnalysisBinding::Analysis(plan.tasks[2].id)
    );
    // Only the bound task reads the dependency.
    assert!(plan.tasks[0].implicit_probes.is_empty());
    assert_eq!(
        probe_names(&plan.tasks[1].implicit_probes),
        vec!["v(vout)".to_string(), "v(vin)".to_string()]
    );
    assert_eq!(
        probe_names(&plan.tasks[2].implicit_probes),
        vec!["v(vout)".to_string()]
    );
}

#[test]
fn an_unknown_analysis_id_lists_the_available_ones() {
    let text = err("ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  op
  derive :x, expr: v(:vout), analysis: :tran1");
    assert_code(&text, Code::Name);
    assert!(
        text.contains("`tran1` is not an analysis of this experiment"),
        "{text}"
    );
    assert!(text.contains("available analyses: ac1, op1"), "{text}");

    // A bare kind is not an identity: the ordinal is part of the name.
    let text = err("ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  derive :x, expr: v(:vout), analysis: :ac");
    assert_code(&text, Code::Name);
    assert!(text.contains("`ac` is not an analysis"), "{text}");
    assert!(text.contains("e.g. `:ac1`"), "{text}");

    // An identity is a literal name, never a computed one.
    let text = err("ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  derive :x, expr: v(:vout), analysis: (\"ac\" + 1)");
    assert_code(&text, Code::Type);
    assert!(text.contains("must be written literally"), "{text}");
}

// ---------------------------------------------------------------------------
// Implicit probe dependencies
// ---------------------------------------------------------------------------

#[test]
fn an_expression_reads_probes_it_did_not_save_without_exporting_them() {
    let plan = plan_of(
        "ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  save v(:vin)
  derive :gain, expr: v(:vout) / v(:vin)
  measure :peak, max: abs(v(:vout))",
    );
    let task = &plan.tasks[0];
    assert_eq!(probe_names(&task.probes), vec!["v(vin)".to_string()]);
    // `v(vin)` is already saved, so it is not requested twice; `v(vout)` is a
    // dependency that never becomes an exported column.
    assert_eq!(
        probe_names(&task.implicit_probes),
        vec!["v(vout)".to_string()]
    );
    assert_eq!(
        probe_names(&task.read_probes()),
        vec!["v(vin)".to_string(), "v(vout)".to_string()]
    );
    assert_eq!(task.exported_names(), Some(vec!["v(vin)".to_string()]));
}

#[test]
fn dependencies_are_collected_without_any_save_statement() {
    let plan = plan_of(
        "tran stop: 1.ms, max_step: 10.us
  derive :gain, expr: gain_db(v(:vout), v(:vin))",
    );
    let task = &plan.tasks[0];
    assert!(task.probes.is_empty());
    assert_eq!(
        task.exported_names(),
        None,
        "no save keeps the backend default"
    );
    assert_eq!(
        probe_names(&task.implicit_probes),
        vec!["v(vout)".to_string(), "v(vin)".to_string()]
    );
}

#[test]
fn every_save_probe_still_reaches_every_task() {
    let plan = plan_of(
        "op
  ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  save v(:vin)
  measure :peak, max: abs(v(:vout)), analysis: :op1
  measure :vmax, max: v(:vout)",
    );
    assert_eq!(plan.tasks.len(), 2);
    for task in &plan.tasks {
        assert_eq!(probe_names(&task.probes), vec!["v(vin)".to_string()]);
        // The legacy measure may read from either analysis, so both tasks
        // must be able to produce `v(vout)`.
        assert_eq!(
            probe_names(&task.implicit_probes),
            vec!["v(vout)".to_string()]
        );
    }
}

// ---------------------------------------------------------------------------
// Names and reductions
// ---------------------------------------------------------------------------

#[test]
fn derived_signal_names_are_unique() {
    let text = err("op
  derive :gain, expr: v(:vin)
  derive :gain, expr: v(:vout) * 2");
    assert_code(&text, Code::Duplicate);
    assert!(
        text.contains("`gain` is already defined as a derived signal"),
        "{text}"
    );
}

#[test]
fn a_derived_signal_cannot_take_a_saved_probe_name() {
    let text = err("op
  save v(:vin)
  derive \"v(vin)\", expr: v(:vout) * 2");
    assert_code(&text, Code::Duplicate);
    assert!(text.contains("already saved as a probe"), "{text}");
}

#[test]
fn a_measure_and_a_derive_cannot_share_a_name() {
    let text = err("tran stop: 1.ms, max_step: 10.us
  derive :peak, expr: v(:vout) + 1
  measure :peak, max: abs(v(:vout))");
    assert_code(&text, Code::Duplicate);
    assert!(
        text.contains("already defined as a derived signal"),
        "{text}"
    );
}

#[test]
fn measurements_are_named_once() {
    let text = err("tran stop: 1.ms, max_step: 10.us
  measure :peak, max: v(:vout)
  measure :peak, min: v(:vout)");
    assert_code(&text, Code::Duplicate);
    assert!(
        text.contains("`peak` is already defined as a measurement"),
        "{text}"
    );
}

#[test]
fn a_computed_result_name_is_refused_rather_than_elided() {
    let text = err("op
  derive (\"g\" + 1), expr: v(:vin)");
    assert_code(&text, Code::Type);
    assert!(
        text.contains("derived signal name must be written literally"),
        "{text}"
    );
}

#[test]
fn an_empty_result_name_is_refused() {
    let text = err("op
  derive \"\", expr: v(:vin)");
    assert_code(&text, Code::Value);
    assert!(text.contains("needs a name"), "{text}");
}

#[test]
fn a_time_integral_needs_an_analysis_with_a_time_axis() {
    let text = err("ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  measure :mean, avg: abs(v(:vout)), analysis: :ac1");
    assert_code(&text, Code::Type);
    assert!(text.contains("needs a time axis"), "{text}");
    assert!(text.contains("`ac1`"), "{text}");

    let text = err("op
  measure :mean, rms: abs(v(:vin)), analysis: :op1");
    assert_code(&text, Code::Type);
    assert!(text.contains("needs a time axis"), "{text}");

    // A transient analysis is the one that can support them.
    let plan = plan_of(
        "tran stop: 1.ms, max_step: 10.us
  measure :mean, avg: abs(v(:vin)), analysis: :tran1",
    );
    assert_eq!(plan.measures[0].kind, MeasureKind::Avg);
    assert!(plan.measures[0].kind.needs_time_axis());
}

#[test]
fn a_complex_reduction_must_be_made_real_first() {
    let text = err("ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  measure :peak, max: v(:vout), analysis: :ac1");
    assert_code(&text, Code::Type);
    assert!(text.contains("is not ordered"), "{text}");
    assert!(text.contains("apply `abs(...)` first"), "{text}");

    let text = err("ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  measure :floor, min: v(:vout) / v(:vin), analysis: :ac1");
    assert_code(&text, Code::Type);
    assert!(text.contains("is not ordered"), "{text}");

    // The same reductions over values known to be real are accepted.
    let plan = plan_of(
        "ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  measure :peak, max: abs(v(:vout)), analysis: :ac1
  measure :floor, min: gain_db(v(:vout), v(:vin)), analysis: :ac1",
    );
    assert_eq!(plan.measures.len(), 2);
    assert_eq!(plan.measures[0].kind, MeasureKind::Max);
    assert_eq!(plan.measures[1].kind, MeasureKind::Min);
    assert!(plan.measures[0].expr.is_statically_real());
    assert!(plan.measures[1].expr.is_statically_real());
}

#[test]
fn a_real_analysis_needs_no_abs_for_a_reduction() {
    let plan = plan_of(
        "tran stop: 1.ms, max_step: 10.us
  measure :peak, max: v(:vout), analysis: :tran1",
    );
    assert_eq!(
        plan.measures[0].binding,
        AnalysisBinding::Analysis(plan.tasks[0].id)
    );
}

#[test]
fn a_legacy_measure_is_exempt_from_the_static_reduction_rules() {
    // An AC-only experiment cannot support `max` over a bare probe, but the
    // legacy form does not name an analysis, so the check stage cannot know:
    // the run-time path reports it instead of the front end guessing.
    let plan = plan_of(
        "ac from: 100.Hz, to: 100.kHz, points_per_decade: 10
  measure :peak, max: v(:vout)",
    );
    assert_eq!(plan.measures[0].binding, AnalysisBinding::LegacyPreferred);
}

#[test]
fn the_kind_is_still_validated_before_the_expression() {
    let text = err("op
  measure :m, median: v(:vin)");
    assert_code(&text, Code::Unsupported);
    assert!(text.contains("available: max, min, avg, rms"), "{text}");
}

// ---------------------------------------------------------------------------
// Parameter sweeps
// ---------------------------------------------------------------------------

#[test]
fn one_parameter_sweep_is_accepted() {
    let src = format!(
        "{SWEEP_SETUP}\nexperiment :e, circuit: :div do\n  dc param: :r, from: 1.kohm, to: 3.kohm, step: 1.kohm\n  derive :d, expr: v(:out) * 2, analysis: :dc1\nend\n"
    );
    let compiled = run_source(&src).expect("one sweep is what the driver can run");
    let plan = &compiled.experiments[0].plan;
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.result_name(plan.tasks[0].id).as_deref(), Some("dc1"));
    assert_eq!(
        plan.derives[0].binding,
        AnalysisBinding::Analysis(plan.tasks[0].id),
        "the swept analysis is the one the dataset comes from"
    );
}

#[test]
fn two_parameter_sweeps_are_refused_at_check_time() {
    // A run re-elaborates and re-runs the design once per point and stitches a
    // single dataset, so a second sweep would silently take over the first
    // one's identity and a binding written for the first would be evaluated
    // against the second (round-3 review B2).
    let src = format!(
        "{SWEEP_SETUP}\nexperiment :e, circuit: :div do\n  dc param: :r, from: 1.kohm, to: 3.kohm, step: 1.kohm\n  dc param: :c, from: 10.nF, to: 30.nF, step: 10.nF\nend\n"
    );
    let text = run_source(&src).expect_err("two sweeps cannot share one run");
    assert_code(&text, Code::Unsupported);
    assert!(text.contains("dc1, dc2"), "{text}");
    assert!(text.contains("only one"), "{text}");
    assert!(
        text.contains("its own experiment"),
        "the diagnostic must say what to do instead:\n{text}"
    );
}

#[test]
fn a_source_sweep_is_not_a_parameter_sweep() {
    // Only a non-topology parameter sweep needs the per-point driver; a source
    // sweep is executed by the backend in one call, so it is untouched by the
    // single-sweep rule.
    let src = format!(
        "{SWEEP_SETUP}\nexperiment :e, circuit: :div do\n  dc source: :src, from: 0.V, to: 3.V, step: 1.V\n  dc param: :r, from: 1.kohm, to: 3.kohm, step: 1.kohm\nend\n"
    );
    let compiled = run_source(&src).expect("a source sweep plus a parameter sweep is fine");
    assert_eq!(compiled.experiments[0].plan.tasks.len(), 2);
}
