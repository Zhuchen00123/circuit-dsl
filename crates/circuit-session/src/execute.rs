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
use circuit_core::plan::{
    AnalysisBinding, AnalysisKind, AnalysisPlan, AnalysisTask, ExprIr, MeasureRequest, Sweep,
    SweepTarget,
};
use circuit_core::units::Quantity;
use circuit_core::{AnalysisId, Limits, SourceMap};
use circuit_results::dataset::{Axis, BackendInfo, Data, Dataset, Signal};
use circuit_results::measure::{Measured, Measurement};

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
    /// Which raw dataset each analysis task produced: `(task id, index into
    /// `datasets`)`. A parameter sweep contributes one entry, for the swept
    /// analysis, because that whole experiment produces one stitched dataset.
    pub bindings: Vec<(AnalysisId, usize)>,
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
    let plan = &elaborated.plan;
    let sweep = plan.parameter_sweep();

    // A parameter sweep re-elaborates and re-runs the design once per point and
    // delivers exactly one stitched dataset. An expression bound to any other
    // analysis of that experiment could never be evaluated, so it is refused
    // here, before any solve, instead of quietly vanishing from the run.
    check_sweep_capabilities(plan)?;
    if let Some((parameter, _)) = &sweep {
        check_sweep_bindings(plan, parameter)?;
    }

    let mut datasets = match &sweep {
        Some((name, _)) => sweep_experiment(request, name, backend, sources)?,
        None => {
            let mut run = backend.run(&elaborated.circuit, plan)?;
            // The effective parameter overrides are part of how the result was
            // produced, so they belong in its metadata.
            for d in &mut run.datasets {
                for (name, value, _) in &plan.param_overrides {
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

    // Which task each raw dataset came from, before anything is evaluated: an
    // expression is bound to an analysis identity, not to a name guess.
    let bindings = binding_table(plan, &datasets)?;

    // Derived signals are computed on the raw solver grid, before any output
    // resampling, so a non-linear expression is never interpolated first.
    let derived = attach_derived(plan, &bindings, &mut datasets)?;

    let mut warnings: Vec<String> = datasets
        .iter()
        .flat_map(|d| d.diagnostics.iter().map(|w| w.render_plain()))
        .collect();
    // A parameter sweep delivers one dataset for the swept analysis; any other
    // analysis the experiment declares is re-run per point and then dropped.
    // Saying so is what keeps a declared analysis from vanishing in silence
    // (round-3 review L2).
    if let Some((parameter, _)) = &sweep
        && let Some(swept) = swept_task(plan)
    {
        for task in &plan.tasks {
            if task.id != swept {
                warnings.push(format!(
                    "analysis `{}` is not exported: a parameter sweep delivers only the swept \
                     dataset `dc_param_{parameter}`",
                    plan.result_name(task.id).unwrap_or_else(|| "?".into())
                ));
            }
        }
    }
    // Measurements come second, also on the raw grid: a coarse output request
    // must not change an integral (`avg`, `rms`) or an extreme.
    let measures = evaluate_measures_with(plan, &datasets, &bindings)?;
    let output_datasets = output_view(plan, &datasets, &bindings, &derived, request.limits)?;

    Ok(RunOutcome {
        datasets,
        output_datasets,
        measures,
        bindings,
        warnings,
    })
}

/// True when a task is a DC sweep over a non-topology parameter.
fn is_parameter_sweep(task: &AnalysisTask) -> bool {
    matches!(
        &task.kind,
        AnalysisKind::Dc(spec) if matches!(spec.sweep.target, SweepTarget::Parameter { .. })
    )
}

/// The task a parameter sweep actually delivers a dataset for.
fn swept_task(plan: &AnalysisPlan) -> Option<AnalysisId> {
    plan.tasks
        .iter()
        .find(|t| is_parameter_sweep(t))
        .map(|t| t.id)
}

/// Refuse a plan the sweep driver cannot execute.
///
/// Elaboration already rejects two parameter sweeps, so this is the defensive
/// half for a hand-built plan: the driver re-elaborates and re-runs the design
/// once per point and stitches one dataset, so with two sweeps the second would
/// silently take over the first one's identity and any binding written for the
/// first would be evaluated against the second (round-3 review B2).
fn check_sweep_capabilities(plan: &AnalysisPlan) -> Result<(), Diagnostics> {
    let sweeps: Vec<&AnalysisTask> = plan
        .tasks
        .iter()
        .filter(|t| is_parameter_sweep(t))
        .collect();
    if sweeps.len() <= 1 {
        return Ok(());
    }
    let names: Vec<String> = sweeps
        .iter()
        .filter_map(|t| plan.result_name(t.id))
        .collect();
    Err(Diagnostics::single(
        Diagnostic::error(
            Code::Unsupported,
            format!(
                "this plan declares {} parameter sweeps ({}); a run can drive only one",
                sweeps.len(),
                names.join(", ")
            ),
        )
        .at(sweeps[1].span)
        .with_note("put each parameter sweep in its own experiment"),
    ))
}

/// Refuse a binding the parameter-sweep driver cannot honour.
fn check_sweep_bindings(plan: &AnalysisPlan, parameter: &str) -> Result<(), Diagnostics> {
    let Some(swept) = swept_task(plan) else {
        return Ok(());
    };
    let mut found = Diagnostics::new();
    let mut check = |name: &str, binding: AnalysisBinding, span| {
        if let AnalysisBinding::Analysis(id) = binding
            && id != swept
        {
            found.push(
                Diagnostic::error(
                    Code::Unsupported,
                    format!(
                        "`{name}` is bound to analysis `{}`, but this experiment sweeps the \
                         parameter `{parameter}` and only the swept DC analysis produces a \
                         result",
                        plan.result_name(id).unwrap_or_else(|| "?".into())
                    ),
                )
                .at(span)
                .with_note(format!(
                    "bind it to `{}` or run the sweep without it",
                    plan.result_name(swept).unwrap_or_else(|| "dc1".into())
                )),
            );
        }
    };
    for derive in &plan.derives {
        check(&derive.name, derive.binding, derive.span);
    }
    for measure in &plan.measures {
        check(&measure.name, measure.binding, measure.span);
    }
    if found.has_errors() {
        Err(found)
    } else {
        Ok(())
    }
}

/// Map every analysis task to the raw dataset it produced.
///
/// A backend returns one dataset per task, in plan order; a parameter sweep is
/// the one exception, because the whole experiment collapses into a single
/// stitched dataset for the swept analysis.
fn binding_table(
    plan: &AnalysisPlan,
    datasets: &[Dataset],
) -> Result<Vec<(AnalysisId, usize)>, Diagnostics> {
    if datasets.len() == plan.tasks.len() {
        return Ok(plan
            .tasks
            .iter()
            .enumerate()
            .map(|(index, task)| (task.id, index))
            .collect());
    }
    if datasets.len() == 1
        && let Some(swept) = swept_task(plan)
    {
        // One stitched dataset for the swept analysis, whatever the driver
        // ends up calling the file (`dc_param_r`, not `dc1`).
        return Ok(vec![(swept, 0)]);
    }
    Err(Diagnostics::single(
        Diagnostic::error(
            Code::Backend,
            format!(
                "the run produced {} dataset(s) for {} analysis task(s)",
                datasets.len(),
                plan.tasks.len()
            ),
        )
        .with_note("a backend returns one dataset per task, in plan order"),
    ))
}

/// The trace as the user asked to see it.
///
/// Two things happen here, in this order:
///
/// 1. the exported signal set is selected — the `save` list when the experiment
///    has one, otherwise every signal the backend reported on its own plus the
///    derived signals of that analysis. A probe that exists *only* because an
///    expression reads it (`Dataset::implicit_only`) is never part of that set:
///    the read set and the export set are different sets, which is also what
///    `cdsl check` prints ("reads ... not exported").
/// 2. a transient task with an explicit `output_interval` is resampled, after the
///    run, after the derived signals and after the measurements, so the solver
///    keeps its own steps and a non-linear expression is never evaluated on
///    interpolated samples. Every other analysis is returned unchanged.
fn output_view(
    plan: &AnalysisPlan,
    datasets: &[Dataset],
    bindings: &[(AnalysisId, usize)],
    derived: &[Vec<String>],
    limits: &Limits,
) -> Result<Vec<Dataset>, Diagnostics> {
    let mut out = Vec::with_capacity(datasets.len());
    for (index, dataset) in datasets.iter().enumerate() {
        let task = bindings
            .iter()
            .find(|(_, i)| *i == index)
            .and_then(|(id, _)| plan.task(*id));
        let derived_names = derived.get(index).map(Vec::as_slice).unwrap_or(&[]);
        let exported = task.and_then(|t| t.exported_names());
        let signals: Vec<Signal> = dataset
            .signals
            .iter()
            .filter(|s| match &exported {
                // No `save`: everything the analysis reported, minus the
                // signals that only an expression asked for.
                None => !dataset.implicit_only.iter().any(|n| n == &s.name),
                Some(names) => {
                    names.iter().any(|n| n == &s.name) || derived_names.iter().any(|n| n == &s.name)
                }
            })
            .cloned()
            .collect();

        let view = Dataset::new(
            dataset.experiment.clone(),
            dataset.analysis.clone(),
            dataset.kind.clone(),
            dataset.axis.clone(),
            signals,
            dataset.backend.clone(),
            limits,
        )?;

        let interval = match task.map(|t| &t.kind) {
            Some(AnalysisKind::Tran(spec)) => spec.output_interval,
            _ => None,
        };
        match interval {
            Some(iv) => out.push(circuit_results::resample::resample_time(&view, iv, limits)?),
            None => out.push(view),
        }
    }
    Ok(out)
}

/// Compute every `derive` on the raw grid and append it to its analysis dataset.
///
/// Returns, per dataset index, the names that were appended, so the output view
/// can keep them even when the user saved only some signals.
fn attach_derived(
    plan: &AnalysisPlan,
    bindings: &[(AnalysisId, usize)],
    datasets: &mut [Dataset],
) -> Result<Vec<Vec<String>>, Diagnostics> {
    let mut per_dataset: Vec<Vec<String>> = vec![Vec::new(); datasets.len()];
    let index_of = |id: AnalysisId| bindings.iter().find(|(t, _)| *t == id).map(|(_, i)| *i);

    for derive in &plan.derives {
        let AnalysisBinding::Analysis(id) = derive.binding else {
            return Err(Diagnostics::single(
                Diagnostic::error(
                    Code::Unsupported,
                    format!(
                        "derived signal `{}` must be bound to one analysis",
                        derive.name
                    ),
                )
                .at(derive.span),
            ));
        };
        let Some(index) = index_of(id) else {
            return Err(Diagnostics::single(
                Diagnostic::error(
                    Code::Backend,
                    format!(
                        "no result for analysis `{}`",
                        plan.result_name(id).unwrap_or_else(|| "?".into())
                    ),
                )
                .at(derive.span),
            ));
        };

        let dataset = &mut datasets[index];
        if dataset.signal(&derive.name).is_some() {
            return Err(Diagnostics::single(
                Diagnostic::error(
                    Code::Duplicate,
                    format!(
                        "derived signal `{}` collides with a signal already in `{}`",
                        derive.name, dataset.analysis
                    ),
                )
                .at(derive.name_span)
                .with_note("rename the derived signal, or stop saving a signal with the same name"),
            ));
        }

        // Evaluate first, then push: the evaluator borrows the dataset, and a
        // failed expression must leave the raw result untouched.
        let value = {
            let runtime = circuit_results::expr::from_ir(&derive.expr);
            // The site names the definition: a failure inside the expression
            // says which derive line it came from, and says it with the same
            // words in a file run and in the REPL.
            let site = circuit_results::expr::EvalSite::derive(&derive.name);
            circuit_results::expr::eval_at(&runtime, dataset, &site)
                .map_err(|d| expression_context(d, &derive.source))?
        };
        let value = broadcast_value(value, dataset.axis.len());
        dataset
            .signals
            .push(Signal::new(derive.name.clone(), value.unit, value.data));
        dataset.backend = dataset
            .backend
            .clone()
            .with_setting(format!("derive.{}", derive.name), derive.source.clone());
        per_dataset[index].push(derive.name.clone());
    }

    Ok(per_dataset)
}

/// Expand a length-1 value over an analysis axis.
///
/// The language defines a dimensionless literal as a broadcast scalar, so a
/// constant expression (`derive :k, expr: 2`, `measure :m, avg: 2`) is expanded
/// over the axis here rather than being rejected by the dataset shape check
/// later, where the message would not mention the expression (review N3). A
/// scalar analysis (an operating point) has no axis samples and keeps length 1.
fn broadcast_value(
    value: circuit_results::expr::Value,
    samples: usize,
) -> circuit_results::expr::Value {
    use circuit_results::dataset::Data;
    if value.data.len() != 1 || samples <= 1 {
        return value;
    }
    let data = match &value.data {
        Data::Real(values) => Data::Real(vec![values[0]; samples]),
        Data::Complex(values) => Data::Complex(vec![values[0]; samples]),
    };
    circuit_results::expr::Value {
        unit: value.unit,
        data,
    }
}

/// Add the written expression to a failure raised while evaluating it.
///
/// The results layer already attaches the analysis, the kind, the signal and
/// the sample coordinate to a runtime expression error, so adding the analysis
/// again here would print the same key twice (round-3 review N2).
fn expression_context(d: Diagnostic, source: &str) -> Diagnostics {
    Diagnostics::single(d.with_context("expression", source.to_string()))
}

/// Add the measurement name to a failure raised while evaluating a target.
///
/// Only the measurement is added, for the same reason as above; the analysis
/// context of a *reduction* failure comes from [`measure_context`], because
/// the reduction layer does not attach one.
fn measure_evaluation_context(d: Diagnostic, request: &MeasureRequest) -> Diagnostics {
    Diagnostics::single(d.with_context("measure", request.name.clone()))
}

/// If the plan runs a DC parameter sweep, return its name and sweep.
///
/// Kept as a free function because callers outside this module use it; the
/// definition lives on the plan so the front end, the driver and the CLI
/// cannot disagree about whether an experiment is a sweep.
pub fn parameter_sweep_of(plan: &AnalysisPlan) -> Option<(String, Sweep)> {
    plan.parameter_sweep()
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
    // Which task the driver sweeps decides which per-point dataset is
    // stitched: with more than one analysis in the experiment the first one is
    // no longer the answer (round-3 re-review NEW-2), and the binding table
    // maps the swept analysis to this dataset.
    let swept_index = elaborated
        .plan
        .tasks
        .iter()
        .position(is_parameter_sweep)
        .ok_or_else(|| {
            Diagnostics::single(Diagnostic::error(
                Code::Backend,
                "the sweep driver was called for a plan with no parameter sweep",
            ))
        })?;
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
    stitch(experiment, parameter, sweep, swept_index, &outcome, &limits).map(|d| vec![d])
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
            implicit_probes: t.implicit_probes.clone(),
            span: t.span,
        })
        .collect();
    AnalysisPlan {
        tasks,
        ..plan.clone()
    }
}

/// Combine per-point results into one dataset whose axis is the swept value.
///
/// `swept_index` is the position of the swept task in the plan. A backend
/// returns one dataset per task in plan order, so that position is also the
/// index of the per-point dataset that belongs to the sweep - taking the first
/// one instead stitched another analysis' result whenever the experiment
/// declared more than the sweep (round-3 re-review NEW-2).
fn stitch(
    experiment: &str,
    parameter: &str,
    sweep: &Sweep,
    swept_index: usize,
    outcome: &circuit_backend::SweepOutcome,
    limits: &Limits,
) -> Result<Dataset, Diagnostics> {
    let missing = |i: usize| {
        Diagnostics::single(
            Diagnostic::error(
                Code::Backend,
                format!(
                    "sweep point {} produced no result for the swept analysis",
                    i + 1
                ),
            )
            .at(sweep.span),
        )
    };
    let first = outcome
        .results
        .first()
        .and_then(|r| r.datasets.get(swept_index))
        .ok_or_else(|| missing(0))?;

    let backend_info = BackendInfo::new(&first.backend.name, &first.backend.version)
        .with_setting("sweep", parameter.to_string())
        .with_setting("points", outcome.coordinates.len().to_string());

    // Every point must report the same signals, in the same order; otherwise
    // the columns would silently misalign. Which of them exist only because an
    // expression reads them has to be part of that comparison too, or the
    // stitched dataset would lose the provenance the output view filters on
    // (round-3 re-review NEW-1).
    let template: Vec<String> = first.signals.iter().map(|s| s.name.clone()).collect();
    let implicit_only = first.implicit_only.clone();
    for (i, r) in outcome.results.iter().enumerate() {
        let Some(d) = r.datasets.get(swept_index) else {
            return Err(missing(i));
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
        if d.implicit_only != implicit_only {
            return Err(Diagnostics::single(
                Diagnostic::error(
                    Code::Backend,
                    format!(
                        "sweep point {} reads a different set of expression-only signals than the first point",
                        i + 1
                    ),
                )
                .at(sweep.span)
                .with_note("a parameter must not change which probes an expression needs"),
            ));
        }
    }

    let mut signals = Vec::new();
    for (col, name) in template.iter().enumerate() {
        let unit = first.signals[col].unit;
        let mut data = Vec::with_capacity(outcome.results.len());
        for r in &outcome.results {
            let Some(d) = r.datasets.get(swept_index) else {
                return Err(Diagnostics::single(
                    Diagnostic::error(
                        Code::Backend,
                        "a sweep point lost its result while stitching",
                    )
                    .at(sweep.span),
                ));
            };
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

    let mut dataset = Dataset::new(
        experiment.to_string(),
        format!("dc_param_{parameter}"),
        "dc",
        Axis::Parameter(outcome.coordinates.clone()),
        signals,
        backend_info,
        limits,
    )?;
    // Provenance survives the stitch, so the output view can still tell a
    // probe the user asked for apart from one an expression merely reads.
    dataset.implicit_only = implicit_only;
    Ok(dataset)
}

/// Evaluate the experiment's `measure` statements.
///
/// Every requested measure now either produces a value or produces an error: a
/// measurement the user asked for can never disappear from a run, which is what
/// the previous "try the next analysis on any error" loop allowed.
///
/// An experiment may run several analyses, and `max: v(:out)` is meaningful
/// against more than one of them. The documented rule for a **bare probe with
/// no `analysis:`** is unchanged: the measurement is taken from the richest
/// analysis that can support it, preferring transient, then AC, then a DC
/// sweep, then the operating point, with declaration order breaking ties.
/// Without that ordering `max: v(:out)` on an RC step would report the
/// operating point's 0 V instead of the transient's peak.
///
/// Everything else is bound to one analysis identity and fails loudly when that
/// analysis cannot be evaluated.
pub fn evaluate_measures(
    plan: &AnalysisPlan,
    datasets: &[Dataset],
) -> Result<Vec<Measured>, Diagnostics> {
    let bindings = binding_table(plan, datasets)?;
    evaluate_measures_with(plan, datasets, &bindings)
}

/// Evaluate the measures against datasets whose task binding is already known.
fn evaluate_measures_with(
    plan: &AnalysisPlan,
    datasets: &[Dataset],
    bindings: &[(AnalysisId, usize)],
) -> Result<Vec<Measured>, Diagnostics> {
    let mut out = Vec::new();
    for request in &plan.measures {
        let Some(kind) = measurement_of(request.kind) else {
            continue;
        };
        let runtime = circuit_results::expr::from_ir(&request.expr);

        let measured = match request.binding {
            AnalysisBinding::Analysis(id) => {
                let Some((_, index)) = bindings.iter().find(|(task, _)| *task == id) else {
                    return Err(Diagnostics::single(
                        Diagnostic::error(
                            Code::Backend,
                            format!(
                                "no result for analysis `{}`",
                                plan.result_name(id).unwrap_or_else(|| "?".into())
                            ),
                        )
                        .at(request.span),
                    ));
                };
                let dataset = &datasets[*index];
                // The site names the measure, so a failure says which
                // measure line it came from - in a file run and in the REPL
                // alike.
                let site = circuit_results::expr::EvalSite::measure(&request.name);
                let value = circuit_results::expr::eval_at(&runtime, dataset, &site)
                    .map_err(|d| measure_evaluation_context(d, request))?;
                let value = broadcast_value(value, dataset.axis.len());
                circuit_results::measure::reduce(
                    kind,
                    &request.name,
                    &value,
                    &dataset.axis,
                    &dataset.analysis,
                )
                .map_err(|d| measure_context(d, request, dataset))?
                .with_analysis(dataset.analysis.clone())
            }
            AnalysisBinding::LegacyPreferred => {
                legacy_measure(kind, request, &runtime, datasets, bindings)?
            }
        };
        out.push(measured);
    }
    Ok(out)
}

/// The documented selection for a bare probe with no `analysis:`.
///
/// Candidates are skipped only when the analysis genuinely cannot answer: the
/// signal is not there, or the reduction needs a time axis the analysis does
/// not have. A candidate that *is* selected but fails to evaluate (a division
/// by zero, a complex extreme) is an error for the whole run - silently moving
/// on to the next analysis is what used to hide the failure.
fn legacy_measure(
    kind: Measurement,
    request: &MeasureRequest,
    runtime: &circuit_results::expr::Expr,
    datasets: &[Dataset],
    bindings: &[(AnalysisId, usize)],
) -> Result<Measured, Diagnostics> {
    let ExprIr::Probe(probe) = &request.expr else {
        return Err(Diagnostics::single(
            Diagnostic::error(
                Code::Unsupported,
                format!(
                    "`{}` needs an explicit `analysis:` because its target is an expression",
                    request.name
                ),
            )
            .at(request.span)
            .with_note("write `analysis: :ac1` after the reduction to choose the analysis"),
        ));
    };
    let signal = probe.name.clone();

    let mut candidates: Vec<usize> = bindings.iter().map(|(_, index)| *index).collect();
    candidates.sort_by_key(|index| (measurement_rank(&datasets[*index].kind), *index));

    let mut absent: Vec<String> = Vec::new();
    let mut inapplicable: Vec<String> = Vec::new();
    for index in candidates {
        let dataset = &datasets[index];
        if dataset.signal(&signal).is_none() {
            absent.push(dataset.analysis.clone());
            continue;
        }
        if kind.needs_time_axis() && !dataset.axis.is_time() {
            inapplicable.push(format!(
                "{} ({})",
                dataset.analysis,
                dataset.axis.describe()
            ));
            continue;
        }
        // Same named site as the explicitly bound path: the legacy analysis
        // choice is a selection rule, not a different way to evaluate.
        let site = circuit_results::expr::EvalSite::measure(&request.name);
        let value = circuit_results::expr::eval_at(runtime, dataset, &site)
            .map_err(|d| measure_evaluation_context(d, request))?;
        let value = broadcast_value(value, dataset.axis.len());
        return circuit_results::measure::reduce(
            kind,
            &request.name,
            &value,
            &dataset.axis,
            &dataset.analysis,
        )
        .map_err(|d| measure_context(d, request, dataset))
        .map(|m| m.with_analysis(dataset.analysis.clone()));
    }

    if !inapplicable.is_empty() {
        return Err(Diagnostics::single(
            Diagnostic::error(
                Code::Type,
                format!(
                    "`{}` needs a time axis, but no analysis here provides one for `{signal}`; \
                     tried: {}",
                    kind.name(),
                    inapplicable.join(", ")
                ),
            )
            .at(request.span)
            .with_note("avg and rms are time integrals; use max/min, or run a `tran` analysis"),
        ));
    }

    Err(Diagnostics::single(
        Diagnostic::error(
            Code::Name,
            format!(
                "no analysis of `{}` has a signal named `{signal}`",
                request.name
            ),
        )
        .at(request.span)
        .with_note(format!(
            "analyses tried, in preference order: {}",
            absent.join(", ")
        ))
        .with_note("the probe is read automatically; check the node or device name"),
    ))
}

/// Add the analysis and the measurement name to a failure in a reduction.
fn measure_context(d: Diagnostic, request: &MeasureRequest, dataset: &Dataset) -> Diagnostics {
    Diagnostics::single(
        d.with_context("analysis", dataset.analysis.clone())
            .with_context("measure", request.name.clone()),
    )
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

/// What a write produced: the files, and the warnings the renderer raised
/// while making them.
///
/// `warnings` are the **renderer's** diagnostics as values: the
/// non-finite samples that the files themselves can only express as an empty
/// CSV field or a JSON `null`. They are not a dataset's own `diagnostics` (the
/// provenance from the plan and the backend, which the JSON file also keeps);
/// a caller showing both tells the user everything the files contain. They are
/// deduplicated per dataset by `Diagnostic::render_plain` text, so writing CSV
/// and JSON of one dataset reports each warning once instead of once per file.
#[derive(Clone, Debug, Default)]
pub struct Written {
    /// Every file written, in the order it was written.
    pub paths: Vec<PathBuf>,
    /// User-visible warnings, deduplicated per dataset across formats.
    pub warnings: Vec<Diagnostic>,
}

impl Written {
    /// The warnings as the lines both front ends print, already indented for
    /// the `  warning: ...` style the CLI summary and the REPL reply share.
    ///
    /// Kept here rather than in either front end so `cdsl run` and the REPL
    /// cannot drift apart on what the same dataset warns about.
    pub fn warning_lines(&self) -> Vec<String> {
        self.warnings
            .iter()
            .map(|w| format!("  {}", w.render_plain().replace('\n', "\n  ")))
            .collect()
    }
}

/// Write every dataset into `out`, one file per dataset per requested format.
///
/// `approve` is asked about each path *before* anything is written, so a
/// caller can refuse to overwrite its own input file. The CLI uses that; a
/// session writing results has nothing to protect.
///
/// Each dataset is rendered through `to_csv_with_diagnostics` /
/// `to_json_with_diagnostics`, so a warning the renderer raises (a
/// non-finite sample is written as an empty CSV field and as JSON `null`)
/// reaches the caller in [`Written::warnings`] instead of being discarded.
///
/// Those warnings are the **renderer's**: a non-finite sample that the file can
/// only express as an empty cell or a JSON `null`. They are not the same
/// values as a dataset's own `diagnostics` array (the provenance the plan and
/// the backend attached to the dataset), which the JSON file also keeps. A
/// caller that shows both — as `cdsl run` and the REPL do — tells the user
/// everything the files contain, and the warnings are deduplicated per dataset
/// so exporting two formats does not say it twice.
///
/// Nothing is written for a dataset until every render for that dataset
/// succeeded, and a failure stops the whole call, so a run that fails earlier
/// leaves the filesystem exactly as it found it: no partial file, and never a
/// file a previous run wrote.
pub fn write_datasets(
    out: &Path,
    format: Format,
    datasets: &[Dataset],
    approve: &mut dyn FnMut(&Path) -> Result<(), Diagnostics>,
) -> Result<Written, Diagnostics> {
    std::fs::create_dir_all(out).map_err(|e| {
        Diagnostics::single(Diagnostic::error(
            Code::Io,
            format!("cannot create `{}`: {e}", out.display()),
        ))
    })?;

    let mut written = Written::default();
    for d in datasets {
        let stem = format!("{}.{}", sanitise(&d.experiment), sanitise(&d.analysis));
        // Render every requested format first: a render that fails must not
        // leave half a dataset on disk.
        let mut jobs: Vec<(PathBuf, circuit_results::Export)> = Vec::new();
        if format.csv() {
            jobs.push((
                out.join(format!("{stem}.csv")),
                circuit_results::to_csv_with_diagnostics(d)?,
            ));
        }
        if format.json() {
            jobs.push((
                out.join(format!("{stem}.json")),
                circuit_results::to_json_with_diagnostics(d)?,
            ));
        }

        // Both renderers report the same non-finite samples. The user reads a
        // warning per dataset, not per file, so deduplicate across the formats
        // of this dataset while keeping the renderer order.
        let mut seen: Vec<String> = Vec::new();
        for (_, export) in &jobs {
            for warning in &export.diagnostics {
                let key = warning.render_plain();
                if !seen.contains(&key) {
                    seen.push(key);
                    written.warnings.push(warning.clone());
                }
            }
        }

        for (path, export) in jobs {
            approve(&path)?;
            std::fs::write(&path, export.text).map_err(|e| {
                Diagnostics::single(Diagnostic::error(
                    Code::Io,
                    format!("cannot write `{}`: {e}", path.display()),
                ))
            })?;
            written.paths.push(path);
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use circuit_core::plan::{
        AcSweep, AnalysisBinding, DcSpec, DeriveRequest, MeasureKind, NamedProbe, Probe, ProbeRef,
        SweepKind,
    };
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
                implicit_probes: Vec::new(),
                span: SourceSpan::synthetic(),
            }],
            param_overrides: Vec::new(),
            derives: Vec::new(),
            measures: Vec::new(),
            span: SourceSpan::synthetic(),
        }
    }

    fn op_dataset(analysis: &str, signals: Vec<Signal>) -> Dataset {
        Dataset::new(
            "e",
            analysis,
            "op",
            Axis::None,
            signals,
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("well-formed test dataset")
    }

    fn tran_dataset(analysis: &str, signals: Vec<Signal>) -> Dataset {
        Dataset::new(
            "e",
            analysis,
            "tran",
            Axis::Time(vec![0.0, 1.0, 2.0]),
            signals,
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("well-formed test dataset")
    }

    fn voltage_probe_ref(name: &str, node: u32) -> ProbeRef {
        ProbeRef::new(
            name,
            Probe::NodeVoltage(NodeId(node)),
            SourceSpan::synthetic(),
        )
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
        let m = MeasureRequest {
            name: "vrms".into(),
            kind: MeasureKind::Rms,
            expr: ExprIr::Probe(voltage_probe_ref("v(out)", 1)),
            binding: AnalysisBinding::LegacyPreferred,
            source: "v(out)".into(),
            span: SourceSpan::synthetic(),
            kind_span: SourceSpan::synthetic(),
        };
        assert_eq!(measurement_of(m.kind), Some(Measurement::Rms));
        assert!(m.expr.is_plain_probe());
        let _ = AcSweep {
            start_hz: 1.0,
            stop_hz: 2.0,
            points: 1,
            kind: SweepKind::Decade,
            span: SourceSpan::synthetic(),
        };
    }

    /// One task, one dataset, in plan order: the normal (non-sweep) contract.
    #[test]
    fn binding_table_maps_tasks_to_datasets_in_plan_order() {
        let plan = plan_with(SweepTarget::Parameter { name: "r".into() });
        let datasets = vec![op_dataset("dc1", Vec::new())];
        let bindings = binding_table(&plan, &datasets).expect("one task, one dataset");
        assert_eq!(bindings, vec![(AnalysisId(0), 0)]);

        let wrong = binding_table(&plan, &[]).expect_err("no dataset for the task");
        assert_eq!(wrong.errors().next().unwrap().code, Code::Backend);
    }

    /// A parameter sweep delivers one stitched dataset for the swept task, even
    /// though the plan may name it `dc1`: binding is by identity, not by name.
    #[test]
    fn a_parameter_sweep_binds_its_single_dataset_to_the_swept_task() {
        let plan = plan_with(SweepTarget::Parameter { name: "r".into() });
        let datasets = vec![op_dataset("dc_param_r", Vec::new())];
        let bindings = binding_table(&plan, &datasets).expect("stitched dataset");
        assert_eq!(bindings, vec![(AnalysisId(0), 0)]);
        let swept = swept_task(&plan);
        assert_eq!(swept, Some(AnalysisId(0)));
    }

    /// A derived signal is computed on the raw grid, appended to its analysis and
    /// kept in the output view even when the experiment saved other signals only.
    #[test]
    fn derived_signals_are_evaluated_then_appended_to_the_view() {
        let mut plan = plan_with(SweepTarget::SourceValue {
            device: circuit_core::DeviceId(0),
            name: "v1".into(),
        });
        plan.derives.push(DeriveRequest {
            name: "gain".into(),
            expr: ExprIr::Div(
                Box::new(ExprIr::Probe(voltage_probe_ref("v(out)", 2))),
                Box::new(ExprIr::Probe(voltage_probe_ref("v(in)", 1))),
            ),
            binding: AnalysisBinding::Analysis(AnalysisId(0)),
            source: "v(out) / v(in)".into(),
            span: SourceSpan::synthetic(),
            name_span: SourceSpan::synthetic(),
        });
        let mut datasets = vec![op_dataset(
            "dc1",
            vec![
                Signal::real("v(out)", circuit_core::units::VOLTAGE, vec![1.0]),
                Signal::real("v(in)", circuit_core::units::VOLTAGE, vec![2.0]),
            ],
        )];
        let bindings = binding_table(&plan, &datasets).expect("one dataset");
        let derived = attach_derived(&plan, &bindings, &mut datasets).expect("gain evaluates");
        assert_eq!(derived, vec![vec!["gain".to_string()]]);
        let gain = datasets[0]
            .signal("gain")
            .expect("appended to the raw grid");
        assert_eq!(gain.data, Data::Real(vec![0.5]));

        // `save v(:out)` was declared: the output view keeps the saved signal and
        // the derived one, and never exposes the implicit dependency `v(in)`.
        let view = output_view(&plan, &datasets, &bindings, &derived, &Limits::default())
            .expect("view builds");
        let names: Vec<&str> = view[0].signals.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["v(out)", "gain"]);
    }

    /// A derived name that collides with a signal the backend produced would
    /// silently overwrite a column, so it is refused instead.
    #[test]
    fn a_derived_name_that_collides_with_an_existing_signal_is_refused() {
        let mut plan = plan_with(SweepTarget::SourceValue {
            device: circuit_core::DeviceId(0),
            name: "v1".into(),
        });
        plan.derives.push(DeriveRequest {
            name: "v(out)".into(),
            expr: ExprIr::Probe(voltage_probe_ref("v(in)", 1)),
            binding: AnalysisBinding::Analysis(AnalysisId(0)),
            source: "v(in)".into(),
            span: SourceSpan::synthetic(),
            name_span: SourceSpan::synthetic(),
        });
        let mut datasets = vec![op_dataset(
            "dc1",
            vec![Signal::real(
                "v(out)",
                circuit_core::units::VOLTAGE,
                vec![1.0],
            )],
        )];
        let bindings = binding_table(&plan, &datasets).expect("one dataset");
        let error = attach_derived(&plan, &bindings, &mut datasets).expect_err("collision");
        assert_eq!(error.errors().next().unwrap().code, Code::Duplicate);
    }

    /// The defensive half of the single-sweep rule: a hand-built plan with two
    /// parameter sweeps is refused before any solve, even though elaboration
    /// already rejects the same shape in a DSL source.
    #[test]
    fn a_hand_built_plan_with_two_sweeps_is_refused() {
        let mut plan = plan_with(SweepTarget::Parameter { name: "r".into() });
        plan.tasks.push(AnalysisTask {
            id: AnalysisId(1),
            kind: plan.tasks[0].kind.clone(),
            probes: Vec::new(),
            implicit_probes: Vec::new(),
            span: SourceSpan::synthetic(),
        });
        let error = check_sweep_capabilities(&plan).expect_err("two sweeps cannot share one run");
        assert_eq!(error.errors().next().unwrap().code, Code::Unsupported);
        let rendered = error.render_plain();
        assert!(
            rendered.contains("dc1") && rendered.contains("dc2"),
            "{rendered}"
        );

        // One sweep is exactly what the driver is for.
        plan.tasks.pop();
        assert!(check_sweep_capabilities(&plan).is_ok());
    }

    /// The stitch must take the swept task's dataset, not the first task's, and
    /// must carry the expression-only provenance with it (re-review NEW-1/NEW-2).
    #[test]
    fn the_stitch_takes_the_swept_dataset_and_keeps_its_provenance() {
        use circuit_backend::{SimulationResults, SweepOutcome};

        // Two analyses; the parameter sweep is the second one.
        let mut plan = plan_with(SweepTarget::Parameter { name: "r".into() });
        plan.tasks.insert(
            0,
            AnalysisTask {
                id: AnalysisId(1),
                kind: AnalysisKind::Op,
                probes: Vec::new(),
                implicit_probes: Vec::new(),
                span: SourceSpan::synthetic(),
            },
        );
        let swept_index = plan
            .tasks
            .iter()
            .position(is_parameter_sweep)
            .expect("the plan has a parameter sweep");
        assert_eq!(swept_index, 1, "the swept task is not the first task");

        // Per point: the op task reports v(a); the swept task reports v(out)
        // and reads i(r1) only for an expression.
        let point = |value: f64| SimulationResults {
            datasets: vec![
                op_dataset(
                    "op1",
                    vec![Signal::real(
                        "v(a)",
                        circuit_core::units::VOLTAGE,
                        vec![9.0],
                    )],
                ),
                {
                    let mut d = op_dataset(
                        "dc1",
                        vec![
                            Signal::real("v(out)", circuit_core::units::VOLTAGE, vec![value]),
                            Signal::real(
                                "i(r1)",
                                circuit_core::units::CURRENT,
                                vec![value / 1000.0],
                            ),
                        ],
                    );
                    d.implicit_only = vec!["i(r1)".to_string()];
                    d
                },
            ],
        };
        let outcome = SweepOutcome {
            coordinates: vec![1000.0, 2000.0],
            results: vec![point(1.0), point(2.0)],
        };
        let sweep = plan.tasks[swept_index].kind.clone();
        let AnalysisKind::Dc(spec) = &sweep else {
            panic!("the swept task is a DC sweep");
        };

        let stitched = stitch(
            "e",
            "r",
            &spec.sweep,
            swept_index,
            &outcome,
            &Limits::default(),
        )
        .expect("the sweep stitches");

        // The swept analysis' signals, not the first task's.
        let names: Vec<&str> = stitched.signals.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["v(out)", "i(r1)"]);
        assert_eq!(stitched.analysis, "dc_param_r");
        assert_eq!(stitched.axis, Axis::Parameter(vec![1000.0, 2000.0]));
        // Provenance survives, so the expression-only column stays out of the
        // export even on the sweep path.
        assert_eq!(stitched.implicit_only, vec!["i(r1)".to_string()]);

        let datasets = std::slice::from_ref(&stitched);
        let bindings = binding_table(&plan, datasets).expect("one stitched dataset");
        let view = output_view(
            &plan,
            datasets,
            &bindings,
            &[Vec::new()],
            &Limits::default(),
        )
        .expect("the view builds");
        let exported: Vec<&str> = view[0].signals.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(exported, vec!["v(out)"], "i(r1) is read, not exported");
    }

    /// A probe that only an expression asked for is read but never exported:
    /// the read set and the export set are different sets (round-3 review B1).
    #[test]
    fn an_expression_only_signal_is_read_but_not_exported_without_a_save() {
        let mut plan = plan_with(SweepTarget::SourceValue {
            device: circuit_core::DeviceId(0),
            name: "v1".into(),
        });
        // The experiment writes no `save`, so the backend reports its own
        // signals; the expression adds `i(r1)` for its own use only.
        plan.tasks[0].probes.clear();
        plan.tasks[0].implicit_probes.push(NamedProbe {
            name: "i(r1)".into(),
            probe: Probe::DeviceCurrent(circuit_core::DeviceId(0)),
            span: SourceSpan::synthetic(),
        });
        let mut datasets = vec![tran_dataset(
            "tran1",
            vec![
                Signal::real("v(out)", circuit_core::units::VOLTAGE, vec![1.0, 2.0, 3.0]),
                Signal::real("i(r1)", circuit_core::units::CURRENT, vec![0.5, 0.5, 0.5]),
            ],
        )];
        datasets[0].implicit_only = vec!["i(r1)".to_string()];

        let bindings = binding_table(&plan, &datasets).expect("one task, one dataset");
        let derived = attach_derived(&plan, &bindings, &mut datasets).expect("no derives");
        let view = output_view(&plan, &datasets, &bindings, &derived, &Limits::default())
            .expect("the view builds");
        let names: Vec<&str> = view[0].signals.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["v(out)"],
            "an implicit dependency must not become an export column"
        );
        // It is still there for the evaluator, which is why it was read at all.
        assert!(datasets[0].signal("i(r1)").is_some());
    }

    /// A constant expression is a broadcast scalar, so it becomes a column on
    /// every sample instead of failing the view shape check (review N3).
    #[test]
    fn a_constant_derived_signal_is_broadcast_over_the_axis() {
        let mut plan = plan_with(SweepTarget::SourceValue {
            device: circuit_core::DeviceId(0),
            name: "v1".into(),
        });
        plan.derives.push(DeriveRequest {
            name: "k".into(),
            expr: ExprIr::Number(2.0),
            binding: AnalysisBinding::Analysis(AnalysisId(0)),
            source: "2".into(),
            span: SourceSpan::synthetic(),
            name_span: SourceSpan::synthetic(),
        });
        let mut datasets = vec![tran_dataset(
            "tran1",
            vec![Signal::real(
                "v(out)",
                circuit_core::units::VOLTAGE,
                vec![1.0, 2.0, 3.0],
            )],
        )];
        let bindings = binding_table(&plan, &datasets).expect("one dataset");
        let derived = attach_derived(&plan, &bindings, &mut datasets).expect("constant evaluates");
        assert_eq!(derived, vec![vec!["k".to_string()]]);
        let k = datasets[0].signal("k").expect("appended to the raw grid");
        assert_eq!(k.data, Data::Real(vec![2.0, 2.0, 2.0]));

        let view = output_view(&plan, &datasets, &bindings, &derived, &Limits::default())
            .expect("the view builds");
        assert!(view[0].signal("k").is_some());
    }

    /// A requested measure can never disappear: an analysis that selects the
    /// signal but cannot integrate it is reported, not skipped.
    #[test]
    fn a_legacy_measure_that_no_analysis_can_answer_is_an_error() {
        let mut plan = plan_with(SweepTarget::Parameter { name: "r".into() });
        plan.measures.push(MeasureRequest {
            name: "vrms".into(),
            kind: MeasureKind::Rms,
            expr: ExprIr::Probe(voltage_probe_ref("v(out)", 2)),
            binding: AnalysisBinding::LegacyPreferred,
            source: "v(out)".into(),
            span: SourceSpan::synthetic(),
            kind_span: SourceSpan::synthetic(),
        });
        // The dataset has the signal, but a scalar axis cannot be integrated.
        let datasets = vec![op_dataset(
            "dc1",
            vec![Signal::real(
                "v(out)",
                circuit_core::units::VOLTAGE,
                vec![1.0],
            )],
        )];
        let error = evaluate_measures(&plan, &datasets).expect_err("rms needs a time axis");
        assert_eq!(error.errors().next().unwrap().code, Code::Type);
        assert!(
            error.errors().next().unwrap().message.contains("time axis"),
            "{}",
            error.errors().next().unwrap().message
        );

        // A signal no analysis has is a name error naming the analyses tried.
        let mut missing = plan.clone();
        missing.measures[0].expr = ExprIr::Probe(voltage_probe_ref("v(nowhere)", 9));
        let error = evaluate_measures(&missing, &datasets).expect_err("no such signal");
        assert_eq!(error.errors().next().unwrap().code, Code::Name);
        let rendered = error.render_plain();
        assert!(rendered.contains("dc1"), "{rendered}");
        assert!(rendered.contains("v(nowhere)"), "{rendered}");
    }

    /// Contract 1.4: the warnings the renderer raises while writing reach the
    /// caller instead of being thrown away with the rendered text.
    #[test]
    fn written_datasets_carry_the_renderer_warnings_once_per_dataset() {
        let dir = std::env::temp_dir().join("cdsl_written_warnings");
        let _ = std::fs::remove_dir_all(&dir);
        let dataset = tran_dataset(
            "tran1",
            vec![Signal::real(
                "v(out)",
                circuit_core::units::VOLTAGE,
                vec![0.0, f64::NAN, 2.0],
            )],
        );

        let written =
            write_datasets(&dir, Format::Both, &[dataset], &mut |_| Ok(())).expect("write");

        // One file per requested format, and the warning appears once even
        // though both renderers raised it.
        assert_eq!(written.paths.len(), 2, "{:?}", written.paths);
        assert_eq!(written.warnings.len(), 1, "{:?}", written.warnings);
        let warning = &written.warnings[0];
        assert_eq!(warning.code, Code::Value);
        assert_eq!(warning.severity.as_str(), "warning");
        assert!(warning.message.contains("v(out)"), "{}", warning.message);

        // The documented rendering is unchanged: an empty CSV field, JSON null,
        // and the JSON file keeps its own diagnostics array.
        assert_eq!(
            std::fs::read_to_string(dir.join("e.tran1.csv")).expect("csv"),
            "time,v(out)\n0,0\n1,\n2,2\n"
        );
        let json = std::fs::read_to_string(dir.join("e.tran1.json")).expect("json");
        assert!(json.contains("null"), "{json}");
        assert!(json.contains("diagnostics"), "{json}");

        // The text both front ends print is built here, once.
        let lines = written.warning_lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].starts_with("  warning["), "{}", lines[0]);
        assert!(lines[0].contains("v(out)"), "{}", lines[0]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A refused path stops the batch before anything is written, and an
    /// existing file is never removed: the writer only ever adds.
    #[test]
    fn a_refused_write_leaves_the_directory_untouched() {
        let dir = std::env::temp_dir().join("cdsl_written_refusal");
        let _ = std::fs::remove_dir_all(&dir);
        let dataset = tran_dataset(
            "tran1",
            vec![Signal::real(
                "v(out)",
                circuit_core::units::VOLTAGE,
                vec![1.0, 2.0, 3.0],
            )],
        );

        let error = write_datasets(&dir, Format::Both, &[dataset], &mut |_| {
            Err(Diagnostics::single(Diagnostic::error(
                Code::Io,
                "refusing to write this path",
            )))
        })
        .expect_err("the refusal stops the write");

        assert_eq!(error.errors().next().unwrap().code, Code::Io);
        assert!(
            std::fs::read_dir(&dir)
                .expect("directory exists")
                .next()
                .is_none(),
            "nothing may be written when a path is refused"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
