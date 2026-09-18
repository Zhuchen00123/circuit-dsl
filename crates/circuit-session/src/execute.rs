//! Running an experiment, once or as a parameter sweep.
//!
//! This is the code that used to live in the CLI's `run` command. It moved
//! here because the REPL must execute experiments the same way a file does: a
//! second implementation would eventually disagree about parameters, sweep
//! stitching or measurement selection, and the disagreement would show up as
//! two different answers to the same question.
//!
//! What stays in the CLI is file I/O and presentation; what lives here is the
//! run itself.

use std::path::{Path, PathBuf};

use circuit_backend::backend::SimulationBackend;
use circuit_backend::sweep::{SweepError, run_parameter_sweep, sweep_coordinates};
use circuit_core::diagnostic::{Code, Diagnostic, Diagnostics};
use circuit_core::plan::{AnalysisKind, AnalysisPlan, AnalysisTask, Probe, Sweep, SweepTarget};
use circuit_core::units::Quantity;
use circuit_core::{Limits, SourceMap};
use circuit_results::dataset::{Axis, BackendInfo, Data, Dataset, Signal};
use circuit_results::measure::{Measured, Measurement, measure_signal};

/// What a run produced.
///
/// Two views of a transient run are kept apart on purpose (task A / plan §4):
///
/// - [`RunOutcome::datasets`] is the **raw solver output**: the time points the
///   engine itself accepted. Measurements are computed from these, so changing
///   `output_interval` can never move an `avg`/`rms`/`max`/`min`.
/// - [`RunOutcome::output_datasets`] is what the user sees and exports. It
///   equals the raw set unless the experiment asked for an `output_interval:`,
///   in which case the trace is resampled onto that uniform grid
///   (`circuit_results::resample`).
#[derive(Clone, Debug)]
pub struct RunOutcome {
    pub datasets: Vec<Dataset>,
    /// The datasets to display and write; the raw ones when no output grid was
    /// requested. Same order and names as [`RunOutcome::datasets`].
    pub output_datasets: Vec<Dataset>,
    pub measures: Vec<Measured>,
    /// Warnings attached to datasets, kept so a caller can show them.
    pub warnings: Vec<String>,
}

impl RunOutcome {
    /// A one-line description of each dataset, as the CLI prints them.
    ///
    /// Describes the **output** view: it is what the user is about to write to
    /// disk, so the point count must match the file.
    pub fn summaries(&self) -> Vec<String> {
        self.output_datasets
            .iter()
            .map(|d| {
                let axis = match &d.axis {
                    Axis::None => "scalar".to_string(),
                    Axis::Time(t) => format!("{} time points", t.len()),
                    Axis::Frequency(f) => format!("{} frequency points", f.len()),
                    Axis::Parameter(p) => format!("{} sweep points", p.len()),
                };
                format!(
                    "{}: {}; signals: {}",
                    d.analysis,
                    axis,
                    d.signals
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .collect()
    }
}

/// Where the program to run comes from.
///
/// A parameter sweep re-elaborates the design once per point, which needs the
/// parsed program rather than an already-elaborated circuit — so the program
/// is what the caller hands over, in both cases.
pub struct RunRequest<'a> {
    pub program: &'a circuit_dsl::Program,
    pub experiment: &'a str,
    /// Parameter overrides, already evaluated and fixed. Later entries win
    /// over the experiment's own `param` statements.
    pub overrides: &'a [(String, Quantity)],
    pub limits: &'a Limits,
}

/// Execute one experiment.
///
/// Handles both shapes the engine supports: an ordinary experiment, which the
/// backend runs in one call, and a **parameter** sweep, which the engine
/// cannot do, so the design is re-elaborated once per point with the topology
/// checked for invariance at every step.
pub fn execute<B: SimulationBackend>(
    request: &RunRequest<'_>,
    backend: &mut B,
    sources: &SourceMap,
) -> Result<RunOutcome, Diagnostics> {
    let elaborated = circuit_dsl::elaborate_experiment(
        request.program,
        request.experiment,
        request.overrides,
        request.limits,
    )?;

    let datasets = match parameter_sweep_of(&elaborated.plan) {
        Some((name, _)) => sweep_experiment(request, &name, backend, sources)?,
        None => {
            let mut run = backend.run(&elaborated.circuit, &elaborated.plan)?;
            // The effective parameter overrides are part of how the result was
            // produced, so they belong in its metadata.
            for d in &mut run.datasets {
                for (name, value, _) in &elaborated.plan.param_overrides {
                    d.backend = d
                        .backend
                        .clone()
                        .with_setting(format!("param.{name}"), format!("{}", value.value));
                }
            }
            run.datasets
        }
    };

    if datasets.is_empty() {
        return Err(Diagnostics::single(Diagnostic::error(
            Code::Backend,
            "the run produced no results",
        )));
    }

    let warnings = datasets
        .iter()
        .flat_map(|d| d.diagnostics.iter().map(|w| w.render_plain()))
        .collect();
    // Measurements first, on the raw solver grid: a coarse output request
    // must not change an integral (`avg`, `rms`) or an extreme.
    let measures = evaluate_measures(&elaborated.plan, &datasets);
    let output_datasets = output_view(&elaborated.plan, &datasets, request.limits)?;

    Ok(RunOutcome {
        datasets,
        output_datasets,
        measures,
        warnings,
    })
}

/// The trace as the user asked to see it.
///
/// A transient task with an explicit `output_interval` is resampled here, after
/// the run and after the measurements, so the solver keeps its own steps and
/// the declared source waveform keeps its declared edges. Every other analysis
/// is returned unchanged.
fn output_view(
    plan: &AnalysisPlan,
    datasets: &[Dataset],
    limits: &Limits,
) -> Result<Vec<Dataset>, Diagnostics> {
    // One interval per transient task, in plan order: the backend names the
    // nth transient result `tran{n}`, which is how a dataset finds its own
    // task when an experiment runs more than one.
    let tran_intervals: Vec<Option<f64>> = plan
        .tasks
        .iter()
        .filter_map(|t| match &t.kind {
            AnalysisKind::Tran(spec) => Some(spec.output_interval),
            _ => None,
        })
        .collect();

    let mut out = Vec::with_capacity(datasets.len());
    for dataset in datasets {
        let interval = if dataset.kind == "tran" {
            let ordinal = dataset
                .analysis
                .strip_prefix("tran")
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(1);
            tran_intervals
                .get(ordinal.saturating_sub(1))
                .copied()
                .flatten()
        } else {
            None
        };
        match interval {
            Some(iv) => out.push(circuit_results::resample::resample_time(
                dataset, iv, limits,
            )?),
            None => out.push(dataset.clone()),
        }
    }
    Ok(out)
}

/// If the plan's only DC task sweeps a parameter, return its name and sweep.
pub fn parameter_sweep_of(plan: &AnalysisPlan) -> Option<(String, Sweep)> {
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
fn sweep_experiment<B: SimulationBackend>(
    request: &RunRequest<'_>,
    parameter: &str,
    backend: &mut B,
    sources: &SourceMap,
) -> Result<Vec<Dataset>, Diagnostics> {
    let elaborated = circuit_dsl::elaborate_experiment(
        request.program,
        request.experiment,
        request.overrides,
        request.limits,
    )?;
    let (_, sweep) =
        parameter_sweep_of(&elaborated.plan).expect("caller checked for a parameter sweep");
    let sweep = &sweep;

    let coordinates = sweep_coordinates(sweep)?;
    if coordinates.len() as u64 > request.limits.max_sweep_points {
        return Err(Diagnostics::single(
            Diagnostic::error(
                Code::Limit,
                format!(
                    "sweep of {} points exceeds the limit of {}",
                    coordinates.len(),
                    request.limits.max_sweep_points
                ),
            )
            .at(sweep.span),
        ));
    }

    let dimension = sweep.dimension;
    let limits = *request.limits;
    let program = request.program;
    let experiment = request.experiment;
    let base_overrides = request.overrides;

    let outcome = run_parameter_sweep(backend, sweep, &coordinates, |value| {
        let mut overrides = base_overrides.to_vec();
        overrides.retain(|(n, _)| n != parameter);
        overrides.push((parameter.to_string(), Quantity::new(value, dimension)));
        let elaborated =
            circuit_dsl::elaborate_experiment(program, experiment, &overrides, &limits)?;
        // A parameter sweep is an operating point evaluated at each value, so
        // the DC task becomes an OP for the single-point run.
        let plan = as_single_point_plan(&elaborated.plan);
        Ok((elaborated.circuit, plan))
    });

    let outcome = match outcome {
        Ok(o) => o,
        Err(SweepError::Diagnostics(d)) => return Err(*d),
    };

    let _ = sources;
    stitch(experiment, parameter, sweep, &outcome, &limits).map(|d| vec![d])
}

/// Turn a one-task plan whose task is a DC parameter sweep into an OP plan.
pub fn as_single_point_plan(plan: &AnalysisPlan) -> AnalysisPlan {
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
    sweep: &Sweep,
    outcome: &circuit_backend::SweepOutcome,
    limits: &Limits,
) -> Result<Dataset, Diagnostics> {
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
                Data::Real(v) => {
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
                Data::Complex(_) => {
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

    Dataset::new(
        experiment.to_string(),
        format!("dc_param_{parameter}"),
        "dc",
        Axis::Parameter(outcome.coordinates.clone()),
        signals,
        backend_info,
        limits,
    )
}

/// Evaluate the experiment's `measure` statements.
///
/// An experiment may run several analyses, and `max: v(:out)` is meaningful
/// against more than one of them. The rule, stated in `docs/language.md`, is
/// that a measurement is taken from the **richest analysis that can support
/// it**, preferring transient, then AC, then a DC sweep, then the operating
/// point. Without that ordering `max: v(:out)` on an RC step would silently
/// report the operating point's 0 V instead of the transient's peak.
pub fn evaluate_measures(plan: &AnalysisPlan, datasets: &[Dataset]) -> Vec<Measured> {
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

        for d in &ordered {
            if d.signal(&target).is_none() {
                continue;
            }
            match measure_signal(kind, &m.name, &target, d) {
                Ok(measured) => {
                    out.push(measured);
                    break;
                }
                // The signal is present but this analysis cannot support the
                // reduction, e.g. `rms` of an operating point. Try the next.
                Err(_) => continue,
            }
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
pub fn sanitise(s: &str) -> String {
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

/// Which files a run writes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Csv,
    Json,
    Both,
}

impl Format {
    pub fn csv(self) -> bool {
        matches!(self, Format::Csv | Format::Both)
    }

    pub fn json(self) -> bool {
        matches!(self, Format::Json | Format::Both)
    }
}

/// Write every dataset into `out`, one file per dataset per requested format.
///
/// `approve` is asked about each path *before* anything is written, so a
/// caller can refuse to overwrite its own input file. The CLI uses that; a
/// session writing results has nothing to protect.
pub fn write_datasets(
    out: &Path,
    format: Format,
    datasets: &[Dataset],
    approve: &mut dyn FnMut(&Path) -> Result<(), Diagnostics>,
) -> Result<Vec<PathBuf>, Diagnostics> {
    std::fs::create_dir_all(out).map_err(|e| {
        Diagnostics::single(Diagnostic::error(
            Code::Io,
            format!("cannot create `{}`: {e}", out.display()),
        ))
    })?;

    let mut written = Vec::new();
    for d in datasets {
        let stem = format!("{}.{}", sanitise(&d.experiment), sanitise(&d.analysis));
        let mut jobs: Vec<(PathBuf, String)> = Vec::new();
        if format.csv() {
            jobs.push((out.join(format!("{stem}.csv")), circuit_results::to_csv(d)?));
        }
        if format.json() {
            jobs.push((
                out.join(format!("{stem}.json")),
                circuit_results::to_json(d)?,
            ));
        }
        for (path, text) in jobs {
            approve(&path)?;
            std::fs::write(&path, text).map_err(|e| {
                Diagnostics::single(Diagnostic::error(
                    Code::Io,
                    format!("cannot write `{}`: {e}", path.display()),
                ))
            })?;
            written.push(path);
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use circuit_core::plan::{AcSweep, DcSpec, MeasureKind, NamedProbe, SweepKind};
    use circuit_core::span::SourceSpan;
    use circuit_core::{AnalysisId, NodeId};

    fn plan_with(target: SweepTarget) -> AnalysisPlan {
        AnalysisPlan {
            name: "e".into(),
            circuit_name: "c".into(),
            tasks: vec![AnalysisTask {
                id: AnalysisId(0),
                kind: AnalysisKind::Dc(DcSpec {
                    sweep: Sweep {
                        target,
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
        }
    }

    #[test]
    fn sanitise_keeps_safe_characters() {
        assert_eq!(sanitise("rc_filter"), "rc_filter");
        assert_eq!(sanitise("a.b/c"), "a_b_c");
    }

    #[test]
    fn single_point_plan_turns_dc_into_op() {
        let plan = plan_with(SweepTarget::Parameter { name: "r".into() });
        let single = as_single_point_plan(&plan);
        assert!(matches!(single.tasks[0].kind, AnalysisKind::Op));
        // Probes must survive the conversion.
        assert_eq!(single.tasks[0].probes[0].name, "v(out)");
    }

    #[test]
    fn parameter_sweep_detection_ignores_source_sweeps() {
        assert!(
            parameter_sweep_of(&plan_with(SweepTarget::SourceValue {
                device: circuit_core::DeviceId(0),
                name: "v1".into()
            }))
            .is_none()
        );
        assert!(
            parameter_sweep_of(&plan_with(SweepTarget::Parameter { name: "r".into() })).is_some()
        );
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
        let m = circuit_core::plan::MeasureRequest {
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
