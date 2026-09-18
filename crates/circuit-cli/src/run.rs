//! `cdsl run`: execute an experiment and write its results.
//!
//! Two shapes of run are handled:
//!
//! - an ordinary experiment, which the backend executes in one call;
//! - a **parameter** sweep, which the engine cannot do, so the design is
//!   re-elaborated once per point — with the topology checked for invariance
//!   at every step — and the per-point results are stitched into one dataset
//!   whose axis is the swept parameter.

use std::path::Path;

use circuit_backend::backend::SimulationBackend;
use circuit_backend::sweep::{SweepError, run_parameter_sweep, sweep_coordinates};
use circuit_backend::thevenin::TheveninBackend;
use circuit_core::diagnostic::{Code, Diagnostic, Diagnostics};
use circuit_core::plan::{AnalysisKind, AnalysisPlan, AnalysisTask, Probe, SweepTarget};
use circuit_core::units::Quantity;
use circuit_core::{Limits, SourceMap};
use circuit_results::dataset::{Axis, BackendInfo, Dataset, Signal};
use circuit_results::measure::{Measured, Measurement, measure_signal};
use circuit_results::{to_csv, to_json};

use crate::{EXIT_INTERNAL, EXIT_USER_ERROR, Format, check, guard_output, read_source};

pub fn run(
    file: &Path,
    experiment: Option<&str>,
    out: &Path,
    format: Format,
    verbose: bool,
) -> Result<(), u8> {
    let fe = check::front_end(file, verbose)?;
    let sources = &fe.sources;
    let compiled = &fe.compiled;

    // ---- choose the experiment ------------------------------------------
    let chosen = match experiment {
        Some(name) => match compiled.experiment(name) {
            Some(e) => e,
            None => {
                eprintln!(
                    "error[E_NAME]: no experiment named `{name}` in `{}`",
                    file.display()
                );
                let names: Vec<&str> = compiled
                    .experiments
                    .iter()
                    .map(|e| e.plan.name.as_str())
                    .collect();
                if names.is_empty() {
                    eprintln!("   = the file defines no experiments");
                } else {
                    eprintln!("   = experiments: {}", names.join(", "));
                }
                return Err(EXIT_USER_ERROR);
            }
        },
        None => match compiled.experiments.as_slice() {
            [only] => only,
            [] => {
                eprintln!(
                    "error[E_NAME]: `{}` defines no experiment to run",
                    file.display()
                );
                return Err(EXIT_USER_ERROR);
            }
            many => {
                eprintln!(
                    "error[E_ARGUMENT]: `{}` defines {} experiments; choose one with --experiment",
                    file.display(),
                    many.len()
                );
                let names: Vec<&str> = many.iter().map(|e| e.plan.name.as_str()).collect();
                eprintln!("   = experiments: {}", names.join(", "));
                return Err(EXIT_USER_ERROR);
            }
        },
    };

    // ---- execute ---------------------------------------------------------
    let backend = TheveninBackend::new();
    let mut backend = backend;

    let datasets = match parameter_sweep_of(&chosen.plan) {
        Some((name, _)) => run_parameter_sweep_experiment(
            file,
            &fe,
            &chosen.plan.name,
            &name,
            &mut backend,
            verbose,
        )?,
        None => match backend.run(&chosen.circuit, &chosen.plan) {
            Ok(mut r) => {
                // The effective parameter overrides are part of how the
                // result was produced, so they belong in its metadata.
                for d in &mut r.datasets {
                    for (name, value, _) in &chosen.plan.param_overrides {
                        d.backend = d
                            .backend
                            .clone()
                            .with_setting(format!("param.{name}"), format!("{}", value.value));
                    }
                }
                r.datasets
            }
            Err(d) => return Err(report(&d, sources)),
        },
    };

    if datasets.is_empty() {
        eprintln!("error[E_BACKEND]: the run produced no results");
        return Err(EXIT_INTERNAL);
    }

    // ---- measurements ----------------------------------------------------
    let measured = evaluate_measures(&chosen.plan, &datasets);

    // ---- write -----------------------------------------------------------
    if let Err(e) = std::fs::create_dir_all(out) {
        eprintln!("error[E_IO]: cannot create `{}`: {e}", out.display());
        return Err(EXIT_USER_ERROR);
    }

    let mut written = Vec::new();
    for d in &datasets {
        let stem = format!("{}.{}", sanitise(&d.experiment), sanitise(&d.analysis));
        if format.csv() {
            let path = out.join(format!("{stem}.csv"));
            guard_output(file, &path)?;
            match to_csv(d) {
                Ok(text) => {
                    if let Err(e) = std::fs::write(&path, text) {
                        eprintln!("error[E_IO]: cannot write `{}`: {e}", path.display());
                        return Err(EXIT_USER_ERROR);
                    }
                    written.push(path);
                }
                Err(diag) => {
                    eprintln!("{}", Diagnostics::single(diag).render(sources));
                    return Err(EXIT_USER_ERROR);
                }
            }
        }
        if format.json() {
            let path = out.join(format!("{stem}.json"));
            guard_output(file, &path)?;
            match to_json(d) {
                Ok(text) => {
                    if let Err(e) = std::fs::write(&path, text) {
                        eprintln!("error[E_IO]: cannot write `{}`: {e}", path.display());
                        return Err(EXIT_USER_ERROR);
                    }
                    written.push(path);
                }
                Err(diag) => {
                    eprintln!("{}", Diagnostics::single(diag).render(sources));
                    return Err(EXIT_USER_ERROR);
                }
            }
        }
    }

    // ---- summary ---------------------------------------------------------
    println!(
        "experiment `{}` on circuit `{}` (backend {} {})",
        chosen.plan.name,
        chosen.circuit.name,
        datasets[0].backend.name,
        datasets[0].backend.version
    );
    for d in &datasets {
        let axis = match &d.axis {
            Axis::None => "scalar".to_string(),
            Axis::Time(t) => format!("{} time points", t.len()),
            Axis::Frequency(f) => format!("{} frequency points", f.len()),
            Axis::Parameter(p) => format!("{} sweep points", p.len()),
        };
        println!(
            "  {}: {}; signals: {}",
            d.analysis,
            axis,
            d.signals
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        for diag in d.diagnostics.iter() {
            eprintln!("  warning: {}", diag.render_plain());
        }
    }
    for m in &measured {
        // `Measured::render` already includes the name.
        println!("  measure {}", m.render());
    }
    for path in &written {
        println!("  wrote {}", path.display());
    }

    Ok(())
}

/// If the plan's only DC task sweeps a parameter, return its name and sweep.
fn parameter_sweep_of(plan: &AnalysisPlan) -> Option<(String, circuit_core::plan::Sweep)> {
    let mut found = None;
    for task in &plan.tasks {
        if let AnalysisKind::Dc(spec) = &task.kind
            && let SweepTarget::Parameter { name } = &spec.sweep.target
        {
            found = Some((name.clone(), spec.sweep.clone()));
        }
    }
    found
}

/// Re-elaborate and re-run once per sweep point, then stitch.
fn run_parameter_sweep_experiment(
    file: &Path,
    fe: &check::FrontEnd,
    experiment: &str,
    parameter: &str,
    backend: &mut TheveninBackend,
    verbose: bool,
) -> Result<Vec<Dataset>, u8> {
    let _ = backend;
    let sources = &fe.sources;

    let text = read_source(file)?;
    let mut local = SourceMap::new();
    let id = local.add(file.display().to_string(), text.clone());
    let tokens = circuit_dsl::lex(id, &text).map_err(|d| report(&d, &local))?;
    let program = circuit_dsl::parse(&tokens).map_err(|d| report(&d, &local))?;

    let plan = &fe
        .compiled
        .experiment(experiment)
        .expect("caller resolved the experiment")
        .plan;

    let (_, sweep) = parameter_sweep_of(plan).expect("caller checked for a parameter sweep");
    let coordinates = sweep_coordinates(&sweep).map_err(|d| report(&d, sources))?;
    if coordinates.len() as u64 > Limits::default().max_sweep_points {
        eprintln!(
            "error[E_LIMIT]: sweep of {} points exceeds the limit of {}",
            coordinates.len(),
            Limits::default().max_sweep_points
        );
        return Err(EXIT_USER_ERROR);
    }

    if verbose {
        eprintln!(
            "sweeping parameter `{parameter}` over {} points",
            coordinates.len()
        );
    }

    let dimension = sweep.dimension;
    let limits = Limits::default();

    let outcome = run_parameter_sweep(backend, &sweep, &coordinates, |value| {
        let overrides = vec![(parameter.to_string(), Quantity::new(value, dimension))];
        let elaborated =
            circuit_dsl::elaborate_experiment(&program, experiment, &overrides, &limits)?;
        // A parameter sweep is an operating point evaluated at each value,
        // so the DC task becomes an OP for the single-point run.
        let plan = as_single_point_plan(&elaborated.plan);
        Ok((elaborated.circuit, plan))
    });

    let outcome = match outcome {
        Ok(o) => o,
        Err(SweepError::Diagnostics(d)) => return Err(report(&d, sources)),
    };

    stitch(experiment, parameter, &sweep, &outcome, &limits).map_err(|d| report(&d, sources))
}

/// Turn a one-task plan whose task is a DC parameter sweep into an OP plan.
fn as_single_point_plan(plan: &AnalysisPlan) -> AnalysisPlan {
    let tasks = plan
        .tasks
        .iter()
        .map(|t| AnalysisTask {
            id: t.id,
            kind: match &t.kind {
                AnalysisKind::Dc(_) => AnalysisKind::Op,
                other => other.clone(),
            },
            probes: t.probes.clone(),
            span: t.span,
        })
        .collect();
    AnalysisPlan {
        tasks,
        ..plan.clone()
    }
}

/// Combine per-point results into one dataset whose axis is the swept value.
fn stitch(
    experiment: &str,
    parameter: &str,
    sweep: &circuit_core::plan::Sweep,
    outcome: &circuit_backend::SweepOutcome,
    limits: &Limits,
) -> Result<Vec<Dataset>, Diagnostics> {
    let first = outcome
        .results
        .first()
        .and_then(|r| r.datasets.first())
        .ok_or_else(|| {
            Diagnostics::single(Diagnostic::error(
                Code::Backend,
                "the sweep produced no results",
            ))
        })?;

    let backend_info = BackendInfo::new(&first.backend.name, &first.backend.version)
        .with_setting("sweep", parameter.to_string())
        .with_setting("points", outcome.coordinates.len().to_string());

    // Every point must report the same signals, in the same order; otherwise
    // the columns would silently misalign.
    let template: Vec<String> = first.signals.iter().map(|s| s.name.clone()).collect();
    for (i, r) in outcome.results.iter().enumerate() {
        let Some(d) = r.datasets.first() else {
            return Err(Diagnostics::single(
                Diagnostic::error(
                    Code::Backend,
                    format!("sweep point {} produced no dataset", i + 1),
                )
                .at(sweep.span),
            ));
        };
        let names: Vec<&str> = d.signals.iter().map(|s| s.name.as_str()).collect();
        if names != template.iter().map(String::as_str).collect::<Vec<_>>() {
            return Err(Diagnostics::single(
                Diagnostic::error(
                    Code::Backend,
                    format!(
                        "sweep point {} reported signals [{}] but the first point reported [{}]",
                        i + 1,
                        names.join(", "),
                        template.join(", ")
                    ),
                )
                .at(sweep.span)
                .with_note(
                    "a parameter must not change which signals exist; if the circuit \
                     branches on the parameter it is a topology parameter and cannot be swept",
                ),
            ));
        }
    }

    let mut signals = Vec::new();
    for (col, name) in template.iter().enumerate() {
        let unit = first.signals[col].unit;
        let mut data = Vec::with_capacity(outcome.results.len());
        for r in &outcome.results {
            let d = &r.datasets[0];
            let s = &d.signals[col];
            match &s.data {
                circuit_results::Data::Real(v) => {
                    if v.len() != 1 {
                        return Err(Diagnostics::single(
                            Diagnostic::error(
                                Code::Backend,
                                format!(
                                    "sweep point for `{name}` returned {} samples, expected 1",
                                    v.len()
                                ),
                            )
                            .at(sweep.span),
                        ));
                    }
                    data.push(v[0]);
                }
                circuit_results::Data::Complex(_) => {
                    return Err(Diagnostics::single(
                        Diagnostic::error(
                            Code::Unsupported,
                            format!("cannot sweep a parameter through a complex result `{name}`"),
                        )
                        .at(sweep.span)
                        .with_note("parameter sweeps currently cover scalar analyses (op)"),
                    ));
                }
            }
        }
        signals.push(Signal::real(name.clone(), unit, data));
    }

    let dataset = Dataset::new(
        experiment.to_string(),
        format!("dc_param_{parameter}"),
        "dc",
        Axis::Parameter(outcome.coordinates.clone()),
        signals,
        backend_info,
        limits,
    )?;

    Ok(vec![dataset])
}

/// Evaluate the experiment's `measure` statements.
///
/// An experiment may run several analyses, and `max: v(:out)` is meaningful
/// against more than one of them. The rule, stated in `docs/language.md`, is
/// that a measurement is taken from the **richest analysis that can support
/// it**, preferring transient, then AC, then a DC sweep, then the operating
/// point. Without that ordering `max: v(:out)` on an RC step would silently
/// report the operating point's 0 V instead of the transient's peak.
fn evaluate_measures(plan: &AnalysisPlan, datasets: &[Dataset]) -> Vec<Measured> {
    let mut ordered: Vec<&Dataset> = datasets.iter().collect();
    ordered.sort_by_key(|d| measurement_rank(&d.kind));

    let mut out = Vec::new();
    for m in &plan.measures {
        let Some(kind) = measurement_of(m.kind) else {
            continue;
        };
        let target = match &m.target {
            Probe::NodeVoltage(_) | Probe::DifferentialVoltage { .. } | Probe::DeviceCurrent(_) => {
                m.target_name.clone()
            }
        };

        let mut done = None;
        for d in &ordered {
            if d.signal(&target).is_none() {
                continue;
            }
            match measure_signal(kind, &m.name, &target, d) {
                Ok(measured) => {
                    done = Some(measured);
                    break;
                }
                // The signal is present but this analysis cannot support the
                // reduction, e.g. `rms` of an operating point. Try the next.
                Err(_) => continue,
            }
        }
        match done {
            Some(measured) => out.push(measured),
            None => eprintln!(
                "warning[E_TYPE]: cannot measure `{}` of `{}` in this experiment",
                m.name, target
            ),
        }
    }
    out
}

/// Lower ranks are preferred when several analyses could satisfy a measurement.
fn measurement_rank(kind: &str) -> u8 {
    match kind {
        "tran" => 0,
        "ac" => 1,
        "dc" => 2,
        _ => 3,
    }
}

fn measurement_of(kind: circuit_core::plan::MeasureKind) -> Option<Measurement> {
    use circuit_core::plan::MeasureKind as K;
    match kind {
        K::Max => Some(Measurement::Max),
        K::Min => Some(Measurement::Min),
        K::Avg => Some(Measurement::Avg),
        K::Rms => Some(Measurement::Rms),
    }
}

/// Make a dataset name safe to use as a file name component.
fn sanitise(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Render diagnostics and return the user-error code.
fn report(d: &Diagnostics, sources: &SourceMap) -> u8 {
    eprintln!("{}", d.render(sources));
    EXIT_USER_ERROR
}

#[cfg(test)]
mod tests {
    use super::*;
    use circuit_core::plan::{
        AcSweep, AnalysisPlan, MeasureKind, MeasureRequest, NamedProbe, SweepKind,
    };
    use circuit_core::span::SourceSpan;
    use circuit_core::{AnalysisId, NodeId};

    #[test]
    fn sanitise_keeps_safe_characters() {
        assert_eq!(sanitise("rc_filter"), "rc_filter");
        assert_eq!(sanitise("a.b/c"), "a_b_c");
    }

    #[test]
    fn single_point_plan_turns_dc_into_op() {
        let plan = AnalysisPlan {
            name: "e".into(),
            circuit_name: "c".into(),
            tasks: vec![AnalysisTask {
                id: AnalysisId(0),
                kind: AnalysisKind::Dc(circuit_core::plan::DcSpec {
                    sweep: circuit_core::plan::Sweep {
                        target: SweepTarget::Parameter { name: "r".into() },
                        dimension: circuit_core::units::RESISTANCE,
                        start: 1.0,
                        stop: 2.0,
                        step: Some(1.0),
                        points: None,
                        kind: SweepKind::Linear,
                        include_endpoint: true,
                        span: SourceSpan::synthetic(),
                    },
                }),
                probes: vec![NamedProbe {
                    name: "v(out)".into(),
                    probe: Probe::NodeVoltage(NodeId(2)),
                    span: SourceSpan::synthetic(),
                }],
                span: SourceSpan::synthetic(),
            }],
            param_overrides: Vec::new(),
            measures: Vec::new(),
            span: SourceSpan::synthetic(),
        };

        let single = as_single_point_plan(&plan);
        assert!(matches!(single.tasks[0].kind, AnalysisKind::Op));
        // Probes must survive the conversion.
        assert_eq!(single.tasks[0].probes[0].name, "v(out)");
    }

    #[test]
    fn parameter_sweep_detection_ignores_source_sweeps() {
        let mk = |target: SweepTarget| AnalysisPlan {
            name: "e".into(),
            circuit_name: "c".into(),
            tasks: vec![AnalysisTask {
                id: AnalysisId(0),
                kind: AnalysisKind::Dc(circuit_core::plan::DcSpec {
                    sweep: circuit_core::plan::Sweep {
                        target,
                        dimension: circuit_core::units::VOLTAGE,
                        start: 0.0,
                        stop: 1.0,
                        step: Some(1.0),
                        points: None,
                        kind: SweepKind::Linear,
                        include_endpoint: true,
                        span: SourceSpan::synthetic(),
                    },
                }),
                probes: Vec::new(),
                span: SourceSpan::synthetic(),
            }],
            param_overrides: Vec::new(),
            measures: Vec::new(),
            span: SourceSpan::synthetic(),
        };

        assert!(
            parameter_sweep_of(&mk(SweepTarget::SourceValue {
                device: circuit_core::DeviceId(0),
                name: "v1".into()
            }))
            .is_none()
        );
        assert!(parameter_sweep_of(&mk(SweepTarget::Parameter { name: "r".into() })).is_some());
    }

    #[test]
    fn transient_is_preferred_for_measurements() {
        assert!(measurement_rank("tran") < measurement_rank("ac"));
        assert!(measurement_rank("ac") < measurement_rank("dc"));
        assert!(measurement_rank("dc") < measurement_rank("op"));
    }

    #[test]
    fn measurement_mapping_covers_every_kind() {
        for k in [
            MeasureKind::Max,
            MeasureKind::Min,
            MeasureKind::Avg,
            MeasureKind::Rms,
        ] {
            assert!(measurement_of(k).is_some(), "{k:?}");
        }
    }

    #[test]
    fn measure_request_round_trips_through_the_mapper() {
        let m = MeasureRequest {
            name: "vrms".into(),
            kind: MeasureKind::Rms,
            target: Probe::NodeVoltage(NodeId(1)),
            target_name: "v(out)".into(),
            span: SourceSpan::synthetic(),
            kind_span: SourceSpan::synthetic(),
        };
        assert_eq!(measurement_of(m.kind), Some(Measurement::Rms));
        let _ = AcSweep {
            start_hz: 1.0,
            stop_hz: 2.0,
            points: 1,
            kind: SweepKind::Decade,
            span: SourceSpan::synthetic(),
        };
    }
}
