//! The Thevenin 0.5.0 adapter.
//!
//! # What the Phase-0 evaluation found (see `docs/backend-evaluation.md`)
//!
//! Thevenin takes a `cirq_ir::Circuit` whose fields are all public, so this
//! adapter is a **structural mapping**: the project's IR is translated into
//! Thevenin's IR in memory. No SPICE or Cirq source text is ever generated,
//! which removes a whole class of round-trip errors.
//!
//! Five behaviours of the engine shaped the code below. Each is covered by a
//! test in this module:
//!
//! 1. **`simulate_tran` prepends an operating-point plot.** The transient
//!    data is not `plots[0]`, so plots are selected by name prefix.
//! 2. **`circuit.save` is not honoured** by the single-analysis entry points.
//!    Probe subsetting is done here.
//! 3. **An unreferenced network is never reported as a node.** A linear one
//!    returns `Err(... matrix is singular, cannot solve)`, which names no node
//!    and cannot tell a legal open load from a real floating network; once a
//!    non-linear device is present, the Newton path's gmin stepping can return
//!    `Ok` with a gmin-dependent value instead. Neither tells the user which
//!    node lost its DC reference, so the DC-reference check (and its
//!    node-naming diagnostic) lives in the front end, not here. Measured this
//!    round in `docs/review-evidence/floating-audit.md` and
//!    `docs/review-evidence/backend-contract.md`.
//! 4. **`thevenin_types::Complex` is its own type**, not `num_complex`.
//! 5. **`AcSpec::phase` is in degrees**, while this project stores radians.
//!    Conversion happens in [`map_ac`].
//!
//! One op per analysis task is used: the adapter builds a fresh Thevenin
//! circuit containing exactly one analysis and calls the matching
//! single-analysis entry point. This avoids depending on how the multi-analysis
//! driver orders and names its plots.

use circuit_core::Limits;
use std::collections::HashMap;

use circuit_core::diagnostic::{Code, Diagnostic, Diagnostics};
use circuit_core::ir::{
    self, Circuit, DeviceKind, ModelKind, NodeKind, SourceSpec as IrSourceSpec, Waveform,
};
use circuit_core::plan::{
    AnalysisKind, AnalysisPlan, AnalysisTask, NamedProbe, Probe, SweepKind, SweepTarget, TranSpec,
};
use circuit_core::units::{self, Dimension, Quantity};
use circuit_results::dataset::{Axis, BackendInfo, Complex, Data, Dataset, Signal};

use cirq_ir::{
    AcAnalysis as CqAc, AcSpec as CqAcSpec, Analysis as CqAnalysis, Circuit as CqCircuit,
    Connection as CqConnection, DcAnalysis as CqDc, DcSweep as CqDcSweep,
    DeviceType as CqDeviceType, Element as CqElement, ElementKind as CqElementKind,
    FrequencyScale as CqScale, Id as CqId, Model as CqModel, Net as CqNet,
    SourceSpec as CqSourceSpec, TranAnalysis as CqTran, Value as CqValue, Waveform as CqWaveform,
};
use thevenin::circuit::{simulate_ac, simulate_dc, simulate_op, simulate_tran};
use thevenin_types::{SimPlot, SimResult, VectorData};

use crate::backend::{BackendCapabilities, SimulationBackend, SimulationResults, backend_failure};

/// Version of the engine this adapter is written against.
pub const BACKEND_VERSION: &str = "0.5.0";

/// Thevenin-backed simulator.
#[derive(Debug, Default)]
pub struct TheveninBackend {
    limits: Limits,
}

impl TheveninBackend {
    pub fn new() -> Self {
        Self {
            limits: Limits::default(),
        }
    }

    pub fn with_limits(limits: Limits) -> Self {
        Self { limits }
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }
}

// ---------------------------------------------------------------------------
// Capabilities and validation
// ---------------------------------------------------------------------------

impl TheveninBackend {
    fn caps() -> BackendCapabilities {
        BackendCapabilities {
            name: "thevenin".to_string(),
            version: BACKEND_VERSION.to_string(),
            analyses: vec!["op", "dc", "ac", "tran"],
            devices: vec![
                "resistor",
                "capacitor",
                "inductor",
                "voltage_source",
                "current_source",
                "diode",
            ],
            parameter_sweep: false,
            source_sweep: true,
            notes: vec![
                "DC sweeps of a source value are executed natively.".to_string(),
                "DC sweeps of a parameter are executed by the sweep driver in this crate, \
                 one elaboration per point."
                    .to_string(),
                "Transient output time points are solver-chosen and non-uniform; `max_step` \
                 bounds the internal step, not the output interval. A declared \
                 `output_interval:` is honoured by resampling the solver's trace after the run, \
                 so it changes neither the solve nor a declared source edge."
                    .to_string(),
                "Verified on Windows MSVC only.".to_string(),
            ],
        }
    }
}

impl SimulationBackend for TheveninBackend {
    fn capabilities(&self) -> BackendCapabilities {
        Self::caps()
    }

    fn validate(&self, circuit: &Circuit, plan: &AnalysisPlan) -> Result<(), Diagnostics> {
        let caps = Self::caps();
        let mut ds = Diagnostics::new();

        for d in &circuit.devices {
            let kind = d.kind.name();
            if !caps.supports_device(kind) {
                ds.push(
                    Diagnostic::error(
                        Code::Unsupported,
                        format!("the backend does not support `{kind}` devices"),
                    )
                    .at(d.def_span)
                    .with_context("device", d.name.clone())
                    .with_note(format!("supported: {}", caps.devices.join(", "))),
                );
            }
        }

        for model in &circuit.models {
            if model.kind != ModelKind::Diode {
                ds.push(
                    Diagnostic::error(
                        Code::Unsupported,
                        format!(
                            "the backend does not support `{}` models",
                            model.kind.name()
                        ),
                    )
                    .at(model.span),
                );
            }
        }

        // A `save` statement applies to every analysis, so the same probe
        // appears on each task. Track the ones already reported so one mistake
        // produces one diagnostic.
        let mut seen_probes: std::collections::HashSet<(circuit_core::SourceId, u32, u32)> =
            std::collections::HashSet::new();

        for task in &plan.tasks {
            let name = task.kind.name();
            if !caps.supports_analysis(name) {
                ds.push(
                    Diagnostic::error(
                        Code::Unsupported,
                        format!("the backend does not support `{name}` analysis"),
                    )
                    .at(task.span),
                );
            }

            if let AnalysisKind::Dc(spec) = &task.kind {
                match &spec.sweep.target {
                    SweepTarget::SourceValue { device, name } => match circuit.device(*device) {
                        Some(d) if d.kind.is_source() => {}
                        Some(d) => ds.push(
                            Diagnostic::error(
                                Code::Sweep,
                                format!(
                                    "`{name}` is a {}, not a source, so it cannot be swept",
                                    d.kind.name()
                                ),
                            )
                            .at(spec.sweep.span),
                        ),
                        None => ds.push(
                            Diagnostic::error(
                                Code::Name,
                                format!("unknown device `{name}` in sweep"),
                            )
                            .at(spec.sweep.span),
                        ),
                    },
                    // Parameter sweeps are run one elaboration per point by
                    // `crate::sweep`; this backend executes a single point.
                    SweepTarget::Parameter { .. } => {}
                }
            }

            // The transient step contract is a capability question, not a
            // runtime surprise: `check` must report a parameter set this
            // backend cannot honour (task A / docs/next-iteration-plan.md).
            if let AnalysisKind::Tran(spec) = &task.kind {
                if let Some(ms) = spec.max_step
                    && (!ms.is_finite() || ms <= 0.0)
                {
                    ds.push(
                        Diagnostic::error(
                            Code::Value,
                            format!(
                                "`max_step:` must be a finite number greater than zero, got {ms}"
                            ),
                        )
                        .at(spec.span),
                    );
                } else if let Err(d) = tran_step_for(circuit, spec) {
                    ds.extend(d);
                }
            }

            // Every probe must resolve in the circuit we were handed. `save`
            // is replicated onto every task, so checking all of a task's
            // probes here would report the same problem once per analysis;
            // instead each distinct probe is checked once, below.
            for probe in &task.probes {
                let key = (probe.span.source, probe.span.start, probe.span.end);
                if !seen_probes.insert(key) {
                    continue;
                }
                match &probe.probe {
                    Probe::NodeVoltage(n) | Probe::DifferentialVoltage { pos: n, .. }
                        if circuit.node(*n).is_none() =>
                    {
                        ds.push(
                            Diagnostic::error(
                                Code::Name,
                                format!("unknown node in `{}`", probe.name),
                            )
                            .at(probe.span),
                        );
                    }
                    Probe::DeviceCurrent(d) => {
                        if let Some(dev) = circuit.device(*d) {
                            match dev.kind {
                                // The engine reports a branch current for these.
                                DeviceKind::VoltageSource | DeviceKind::Inductor => {}
                                // Derived exactly from node voltages; see
                                // `materialise_probe`.
                                DeviceKind::Resistor => {}
                                DeviceKind::Capacitor => ds.push(
                                    Diagnostic::error(
                                        Code::Unsupported,
                                        format!(
                                            "`{}` has no current the backend can report",
                                            probe.name
                                        ),
                                    )
                                    .at(probe.span)
                                    .with_context("device", dev.name.clone())
                                    .with_note(
                                        "the engine reports branch currents only for elements \
                                         that own a branch unknown (sources, inductors); a \
                                         capacitor current would have to be differentiated \
                                         from the voltage, which this project does not do",
                                    ),
                                ),
                                DeviceKind::Diode => ds.push(
                                    Diagnostic::error(
                                        Code::Unsupported,
                                        format!(
                                            "`{}` has no current the backend can report",
                                            probe.name
                                        ),
                                    )
                                    .at(probe.span)
                                    .with_context("device", dev.name.clone())
                                    .with_note(
                                        "a diode carries no branch unknown, so its current is \
                                         not available without re-deriving the model equations",
                                    ),
                                ),
                                DeviceKind::CurrentSource => ds.push(
                                    Diagnostic::error(
                                        Code::Unsupported,
                                        format!(
                                            "`{}` has no current the backend can report",
                                            probe.name
                                        ),
                                    )
                                    .at(probe.span)
                                    .with_context("device", dev.name.clone())
                                    .with_note(
                                        "the engine does not emit a branch vector for an \
                                         independent current source",
                                    ),
                                ),
                            }
                        } else {
                            ds.push(
                                Diagnostic::error(
                                    Code::Name,
                                    format!("unknown device in `{}`", probe.name),
                                )
                                .at(probe.span),
                            );
                        }
                    }
                    _ => {}
                }
            }
        }

        if ds.is_empty() { Ok(()) } else { Err(ds) }
    }

    fn run(
        &mut self,
        circuit: &Circuit,
        plan: &AnalysisPlan,
    ) -> Result<SimulationResults, Diagnostics> {
        self.validate(circuit, plan)?;

        let backend_info = BackendInfo::new("thevenin", BACKEND_VERSION)
            .with_setting("adapter", env!("CARGO_PKG_VERSION"));

        // The engine names every plot `<kind>1`, because each task is run as a
        // circuit carrying exactly one analysis. Two `ac` tasks in one
        // experiment would therefore both be called `ac1`, and the second
        // would overwrite the first's output file — which the brief forbids
        // ("results are distinguished by analysis_id and must not overwrite
        // each other"). Number them per kind instead, so the first `ac` is
        // `ac1` and the second `ac2`.
        let mut per_kind: HashMap<&str, u32> = HashMap::new();
        let mut datasets = Vec::new();
        for task in &plan.tasks {
            let kind = task.kind.name();
            let ordinal = per_kind.entry(kind).or_insert(0);
            *ordinal += 1;
            let dataset = self.run_task(circuit, plan, task, *ordinal, &backend_info)?;
            datasets.push(dataset);
        }
        Ok(SimulationResults { datasets })
    }
}

impl TheveninBackend {
    fn run_task(
        &self,
        circuit: &Circuit,
        plan: &AnalysisPlan,
        task: &AnalysisTask,
        ordinal: u32,
        backend_info: &BackendInfo,
    ) -> Result<Dataset, Diagnostics> {
        // A parameter sweep is not something this backend can do in one call:
        // changing a parameter changes a device value, which needs a fresh
        // elaboration. `crate::sweep` drives that and calls us once per point.
        if let AnalysisKind::Dc(spec) = &task.kind
            && let SweepTarget::Parameter { name } = &spec.sweep.target
        {
            return Err(Diagnostics::single(
                Diagnostic::error(
                    Code::Unsupported,
                    format!(
                        "`{name}` is a parameter sweep; run it through the parameter sweep \
                         driver so each point is elaborated separately"
                    ),
                )
                .at(spec.sweep.span)
                .with_note(
                    "the backend executes one fixed-topology point per call; \
                     see circuit_backend::sweep",
                ),
            ));
        }

        let cq = self.build_circuit(circuit, std::slice::from_ref(task))?;

        let result: SimResult = match &task.kind {
            AnalysisKind::Op => simulate_op(&cq),
            AnalysisKind::Ac(_) => simulate_ac(&cq),
            AnalysisKind::Tran(_) => simulate_tran(&cq),
            AnalysisKind::Dc(_) => simulate_dc(&cq),
        }
        .map_err(|e| Diagnostics::single(backend_failure(e.to_string())))?;

        let want = task.kind.name();
        let plot = select_plot(&result, want).ok_or_else(|| {
            Diagnostics::single(
                Diagnostic::error(
                    Code::Backend,
                    format!("the backend produced no `{want}` plot"),
                )
                .with_note(format!(
                    "plots returned: {}",
                    result
                        .plots
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            )
        })?;

        self.convert_plot(circuit, plan, task, ordinal, plot, backend_info)
    }

    // -----------------------------------------------------------------------
    // IR -> Thevenin IR
    // -----------------------------------------------------------------------

    fn build_circuit(
        &self,
        circuit: &Circuit,
        tasks: &[AnalysisTask],
    ) -> Result<CqCircuit, Diagnostics> {
        // Net ids are the same dense indices on both sides.
        let nets = circuit
            .nodes
            .iter()
            .map(|n| CqNet {
                id: CqId(n.id.0),
                name: net_name_for_backend(n),
                is_global: false,
            })
            .collect();

        let elements = circuit
            .devices
            .iter()
            .map(map_device)
            .collect::<Result<Vec<_>, _>>()?;

        let models = circuit
            .models
            .iter()
            .map(map_model)
            .collect::<Result<Vec<_>, _>>()?;

        let mut analyses = Vec::new();
        for task in tasks {
            analyses.push(map_analysis(circuit, task)?);
        }

        Ok(CqCircuit {
            name: circuit.name.clone(),
            nets,
            elements,
            models,
            analyses,
            params: Vec::new(),
            csparams: Vec::new(),
            options: Vec::new(),
            temps: Vec::new(),
            // Left empty on purpose: the engine ignores `save` in the
            // single-analysis entry points (Phase-0 finding 2), so the adapter
            // does the subsetting itself rather than relying on it.
            save: Vec::new(),
            funcs: Vec::new(),
            initial_conditions: Vec::new(),
            nodeset: Vec::new(),
            measures: Vec::new(),
            code_blocks: Vec::new(),
            raw_directives: Vec::new(),
        })
    }

    // -----------------------------------------------------------------------
    // Thevenin results -> our Dataset
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn convert_plot(
        &self,
        circuit: &Circuit,
        plan: &AnalysisPlan,
        task: &AnalysisTask,
        ordinal: u32,
        plot: &SimPlot,
        backend_info: &BackendInfo,
    ) -> Result<Dataset, Diagnostics> {
        let kind = task.kind.name();
        let (axis, axis_name) = build_axis(circuit, task, plot)?;

        let mut signals = Vec::new();
        let mut diagnostics = Diagnostics::new();

        if task.probes.is_empty() {
            // No explicit probe list: expose everything the engine produced,
            // translated into the DSL's naming.
            for vector in &plot.vecs {
                if vector.name.eq_ignore_ascii_case(&axis_name) {
                    continue;
                }
                if let Some((name, unit)) = translate_default_name(circuit, vector) {
                    signals.push(make_signal(name, unit, &vector.data));
                }
            }
        } else {
            for probe in &task.probes {
                match materialise_probe(circuit, probe, plot) {
                    Ok(Some(signal)) => signals.push(signal),
                    Ok(None) => diagnostics.push(
                        Diagnostic::error(
                            Code::Backend,
                            format!("the backend did not report `{}`", probe.name),
                        )
                        .at(probe.span)
                        .with_note(format!(
                            "available: {}",
                            plot.vecs
                                .iter()
                                .map(|v| v.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )),
                    ),
                    Err(d) => diagnostics.push(d),
                }
            }
        }

        if diagnostics.has_errors() {
            return Err(diagnostics);
        }

        // A malformed result is fatal, but the warnings gathered while
        // materialising probes must not be lost on the way out.
        //
        // A transient result additionally records the solve configuration
        // (solver step, step bound, waveform bound, raw point count) so the
        // trace can be told apart from the solver grid it was derived from.
        let mut info = backend_info.clone();
        if let AnalysisKind::Tran(spec) = &task.kind {
            for (key, value) in tran_settings(circuit, spec, axis.len()) {
                info = info.with_setting(key, value);
            }
        }
        let mut dataset = match Dataset::new(
            plan.name.clone(),
            format!("{kind}{ordinal}"),
            kind,
            axis,
            signals,
            info,
            &self.limits,
        ) {
            Ok(d) => d,
            Err(mut d) => {
                d.extend(diagnostics);
                return Err(d);
            }
        };
        for d in diagnostics {
            dataset.push_diagnostic(d);
        }
        Ok(dataset)
    }
}

// ---------------------------------------------------------------------------
// Net and device mapping
// ---------------------------------------------------------------------------

/// The engine rewrites a net literally named `gnd` to `0`, and treats the
/// literal name `0` as ground. Passing our ground through under its own name
/// keeps the two sides consistent without a rename table.
fn net_name_for_backend(node: &ir::Node) -> String {
    match node.kind {
        NodeKind::Ground => "gnd".to_string(),
        NodeKind::Normal => node.name.clone(),
    }
}

fn map_value(q: Quantity) -> CqValue {
    CqValue::Real(q.value)
}

fn map_device(device: &ir::Device) -> Result<CqElement, Diagnostics> {
    let (kind, terminal_names): (CqElementKind, &[&str]) = match device.kind {
        DeviceKind::Resistor => (
            CqElementKind::Resistor,
            &[ir::terminal::POS, ir::terminal::NEG],
        ),
        DeviceKind::Capacitor => (
            CqElementKind::Capacitor,
            &[ir::terminal::POS, ir::terminal::NEG],
        ),
        DeviceKind::Inductor => (
            CqElementKind::Inductor,
            &[ir::terminal::POS, ir::terminal::NEG],
        ),
        DeviceKind::VoltageSource => (
            CqElementKind::VoltageSource,
            &[ir::terminal::POS, ir::terminal::NEG],
        ),
        DeviceKind::CurrentSource => (
            CqElementKind::CurrentSource,
            &[ir::terminal::POS, ir::terminal::NEG],
        ),
        DeviceKind::Diode => (
            CqElementKind::Diode,
            &[ir::terminal::ANODE, ir::terminal::CATHODE],
        ),
    };

    // The engine uses `pos`/`neg` for two-terminal devices and
    // `anode`/`cathode` for a diode; the project's IR uses `p`/`n` uniformly.
    fn backend_terminal(ours: &str) -> String {
        match ours {
            ir::terminal::POS => "pos".to_string(),
            ir::terminal::NEG => "neg".to_string(),
            ir::terminal::ANODE => "anode".to_string(),
            ir::terminal::CATHODE => "cathode".to_string(),
            other => other.to_string(),
        }
    }

    let mut connections = Vec::new();
    for t in terminal_names {
        let net = device.terminal(t).ok_or_else(|| {
            Diagnostics::single(
                Diagnostic::error(
                    Code::Backend,
                    format!("device `{}` is missing terminal `{t}`", device.name),
                )
                .at(device.def_span),
            )
        })?;
        connections.push(CqConnection {
            terminal: backend_terminal(t),
            net: CqId(net.0),
        });
    }

    // R/L/C carry their value as a plain `value` parameter. Sources carry
    // theirs in `source_spec`, and a diode's behaviour comes from its model.
    let params = match device.kind {
        DeviceKind::Resistor | DeviceKind::Capacitor | DeviceKind::Inductor => {
            let value = device.param("value").ok_or_else(|| {
                Diagnostics::single(
                    Diagnostic::error(
                        Code::Backend,
                        format!("device `{}` has no `value`", device.name),
                    )
                    .at(device.def_span),
                )
            })?;
            vec![("value".to_string(), map_value(value))]
        }
        _ => Vec::new(),
    };

    let source_spec = if device.kind.is_source() {
        let spec = device.source.as_ref().ok_or_else(|| {
            Diagnostics::single(
                Diagnostic::error(
                    Code::Backend,
                    format!("source `{}` has no source specification", device.name),
                )
                .at(device.def_span),
            )
        })?;
        Some(map_source(spec, device)?)
    } else {
        None
    };

    Ok(CqElement {
        id: CqId(device.id.0),
        name: device.name.clone(),
        kind,
        connections,
        params,
        model: device.model.map(|m| CqId(m.0)),
        source_spec,
    })
}

fn map_source(spec: &IrSourceSpec, device: &ir::Device) -> Result<CqSourceSpec, Diagnostics> {
    let ac = spec.ac.map(|a| CqAcSpec {
        mag: a.magnitude.value,
        // The project stores radians; the engine wants degrees. Phase-0
        // finding 5.
        phase: a.phase_rad.to_degrees(),
    });

    let waveform = match &spec.waveform {
        None => None,
        Some(Waveform::Pulse {
            low,
            high,
            delay,
            rise,
            fall,
            width,
            period,
        }) => Some(CqWaveform::Pulse {
            v1: low.value,
            v2: high.value,
            td: Some(delay.value),
            tr: Some(rise.value),
            tf: Some(fall.value),
            pw: Some(width.value),
            per: Some(period.value),
        }),
        Some(Waveform::Sin {
            offset,
            amplitude,
            frequency,
            delay,
            damping,
            phase_rad,
        }) => Some(CqWaveform::Sin {
            v0: offset.value,
            va: amplitude.value,
            freq: Some(frequency.value),
            td: Some(delay.value),
            theta: Some(damping.value),
            // SPICE's `phi` is in degrees.
            phi: Some(phase_rad.to_degrees()),
        }),
        Some(Waveform::Pwl(points)) => Some(CqWaveform::Pwl(
            points.iter().map(|(t, v)| (t.value, v.value)).collect(),
        )),
    };

    if spec.is_empty() {
        return Err(Diagnostics::single(
            Diagnostic::error(
                Code::Value,
                format!(
                    "source `{}` has no `dc`, `ac` or `waveform`, so it has no effect",
                    device.name
                ),
            )
            .at(device.def_span)
            .with_note("give it at least one of dc:, ac:, waveform:"),
        ));
    }

    Ok(CqSourceSpec {
        dc: spec.dc.map(|d| d.value),
        ac,
        waveform,
    })
}

fn map_model(model: &ir::Model) -> Result<CqModel, Diagnostics> {
    let device_type = match model.kind {
        ModelKind::Diode => CqDeviceType::Diode,
    };
    Ok(CqModel {
        id: CqId(model.id.0),
        name: model.name.clone(),
        device_type,
        params: model
            .params
            .iter()
            .map(|(k, v)| (k.clone(), map_value(*v)))
            .collect(),
    })
}

// ---------------------------------------------------------------------------
// Analysis mapping
// ---------------------------------------------------------------------------

fn map_analysis(circuit: &Circuit, task: &AnalysisTask) -> Result<CqAnalysis, Diagnostics> {
    Ok(match &task.kind {
        AnalysisKind::Op => CqAnalysis::Op,

        AnalysisKind::Ac(spec) => CqAnalysis::Ac(CqAc {
            start: spec.start_hz,
            stop: spec.stop_hz,
            points: spec.points,
            scale: match spec.kind {
                SweepKind::Decade => CqScale::Decade,
                SweepKind::Octave => CqScale::Octave,
                SweepKind::LinearPoints | SweepKind::Linear => CqScale::Linear,
            },
        }),

        AnalysisKind::Tran(spec) => {
            // `tmax` is the engine's internal step bound (`h_max`), so it never
            // promises anything about the output grid: the engine records every
            // accepted internal step (measured, docs/review-evidence/
            // backend-contract.md). `step` is the engine's print step
            // (`h_print`), and it is *not* an output interval either — the
            // engine has no output sampling. It is used in exactly three
            // places: as the fallback for `h_max` when `tmax` is absent
            // (`h_max = t_max.unwrap_or(min(tstep, tstop/50))`), as the floor
            // of the integration step (`h_min = tstep * 1e-9`), and as the
            // lower bound the PULSE `tr`/`tf` are clamped to — which means a
            // declared rise time shorter than `step` would be silently
            // widened.
            //
            // `output_interval` therefore does **not** reach this field: it is
            // an output-sampling request, implemented by resampling the
            // solver's trace after the run (circuit-results::resample). On this
            // adapter it has no effect at all beyond being recorded as metadata
            // (`tran_settings`, applied in `convert_plot`).
            let h_print = print_step_for(circuit, spec)?;
            CqAnalysis::Tran(CqTran {
                step: h_print,
                stop: spec.stop_s,
                start: spec.start_s,
                uic: spec.uic,
                tmax: spec.max_step,
            })
        }

        AnalysisKind::Dc(spec) => {
            let device = match &spec.sweep.target {
                SweepTarget::SourceValue { device, .. } => *device,
                SweepTarget::Parameter { name } => {
                    return Err(Diagnostics::single(
                        Diagnostic::error(
                            Code::Unsupported,
                            format!("`{name}` must be swept by the parameter sweep driver"),
                        )
                        .at(spec.sweep.span),
                    ));
                }
            };
            let d = circuit.device(device).ok_or_else(|| {
                Diagnostics::single(
                    Diagnostic::error(Code::Name, "swept device does not exist")
                        .at(spec.sweep.span),
                )
            })?;
            if !d.kind.is_source() {
                return Err(Diagnostics::single(
                    Diagnostic::error(Code::Sweep, format!("`{}` is not a source", d.name))
                        .at(spec.sweep.span),
                ));
            }
            let step = spec.sweep.step.unwrap_or_else(|| {
                let n = spec.sweep.points.unwrap_or(1).max(1);
                (spec.sweep.stop - spec.sweep.start) / n as f64
            });
            CqAnalysis::Dc(CqDc {
                sweeps: vec![CqDcSweep {
                    source: CqId(device.0),
                    start: spec.sweep.start,
                    stop: spec.sweep.stop,
                    step,
                }],
            })
        }
    })
}

// ---------------------------------------------------------------------------
// Transient step contract
// ---------------------------------------------------------------------------

/// Upper bound on how many solver steps a **declared** waveform timing may
/// force through the engine's print step (`h_print`).
///
/// The engine uses `h_print` as the floor of its integration step
/// (`h_min = h_print * 1e-9`, `thevenin-0.5.0/src/transient.rs:1407`), as the
/// growth cap in the no-LTE branch (`:1694`), and as the fallback for the step
/// bound when `max_step` is absent (`:799`). A declared edge far below the
/// simulation window therefore has a real cost, and the honest answer to a
/// request the backend cannot execute within this budget is a capability
/// error — never a silently widened edge.
const MAX_PRINT_STEPS: f64 = 1_000_000.0;

/// What the TRAN mapping decided.
///
/// Computed by one function so the analysis mapping, the capability check and
/// the result metadata cannot disagree about the value that was handed to the
/// engine.
struct TranStep {
    /// Value handed to the engine as `TranAnalysis::step` (`h_print`).
    h_print: f64,
    /// Smallest declared waveform timing the engine compares against `tstep`,
    /// if this circuit declares any.
    waveform_bound: Option<f64>,
}

/// The engine clamps several waveform parameters with `.max(tstep)` /
/// `.unwrap_or(tstep)`, and uses the *clamped* values for its breakpoint table
/// too (`thevenin-0.5.0/src/waveform.rs:37-38,271-278`). This function picks
/// the print step so that clamp can never fire on a declared value.
///
/// The rule, in full:
///
/// ```text
/// span          = stop - start
/// default_print = span / 1000
/// bound         = min over sources of the declared PULSE rise, fall, period
/// h_print       = bound.min(default_print)      (default_print when none)
/// ```
///
/// `output_interval` is deliberately absent: it is an output-sampling request
/// and never reaches the solver (task A, docs/next-iteration-plan.md). It is
/// re-applied after the run in `circuit-results::resample`.
fn print_step_for(circuit: &Circuit, spec: &TranSpec) -> Result<f64, Diagnostics> {
    Ok(tran_step_for(circuit, spec)?.h_print)
}

fn tran_step_for(circuit: &Circuit, spec: &TranSpec) -> Result<TranStep, Diagnostics> {
    let mut bound: Option<f64> = None;
    for device in &circuit.devices {
        let Some(source) = device.source.as_ref() else {
            continue;
        };
        let Some(Waveform::Pulse {
            rise, fall, period, ..
        }) = source.waveform.as_ref()
        else {
            // `sin` and `pwl` have no `tstep`-dependent parameter: the engine
            // derives SIN's default frequency from `tstop` only.
            continue;
        };
        for (name, q) in [("rise", rise), ("fall", fall), ("period", period)] {
            if !q.value.is_finite() || q.value <= 0.0 {
                return Err(Diagnostics::single(
                    Diagnostic::error(
                        Code::Unsupported,
                        format!(
                            "the backend cannot honour `{name}: {}` on source `{}`: \
                             the engine substitutes its own print step for a zero or \
                             non-finite timing, which would silently change the declared \
                             waveform",
                            q.value, device.name
                        ),
                    )
                    .at(device.def_span)
                    .with_context("source", device.name.clone())
                    .with_note(
                        "give the pulse a finite, positive rise, fall and period; \
                         the engine has no ideal (zero-width) edge",
                    ),
                ));
            }
            bound = Some(bound.map_or(q.value, |b: f64| b.min(q.value)));
        }
    }

    let span = spec.stop_s - spec.start_s;
    let default_print = span / 1000.0;
    let h_print = match bound {
        Some(b) => b.min(default_print),
        None => default_print,
    };
    if !h_print.is_finite() || h_print <= 0.0 {
        return Err(Diagnostics::single(
            Diagnostic::error(
                Code::Value,
                format!(
                    "`tran` has a non-positive or non-finite window: start {} s, stop {} s",
                    spec.start_s, spec.stop_s
                ),
            )
            .at(spec.span)
            .with_note("`stop:` must be greater than `start:` and both must be finite"),
        ));
    }

    // Which step does the engine actually take?
    //
    // `h_print` only *caps* the step in the branch the engine takes when
    // neither the circuit nor the device models give it an LTE estimate to
    // work from (`transient.rs:1688-1696`, `min(h_max, h_print)`). As soon as a
    // capacitor or inductor is present the LTE path runs, and there the step is
    // bounded by `h_max` alone (`:1620-1681`). `h_max` is the declared
    // `max_step` when there is one, and otherwise `min(h_print, stop/50)`
    // (`:799`).
    //
    // This distinction matters: refusing every fine declared edge would reject
    // legitimate runs, e.g. an RC whose integration is bounded by
    // `max_step = tau/200` no matter how narrow the declared edge is.
    let effective_step = effective_step_for(circuit, spec, h_print);

    // Would the run be affordable even with no waveform constraint at all?
    //
    // This is the attribution test. When every source declares nothing, the
    // print step is the engine's own window default (`span/1000`); if the same
    // budget is exceeded there too, the cost comes from the user's explicit
    // `max_step` — their own request, and refusing it is not this contract's
    // business. Only a *declared waveform timing* can make the error below
    // fire (found in review, W6-2: a purely resistive source set with
    // `max_step: 1.ns` over 1 s used to be blamed on "declared source
    // timings").
    let effective_without_waveform = effective_step_for(circuit, spec, default_print);
    let steps = span / effective_step;
    let steps_without_waveform = span / effective_without_waveform;
    if !steps.is_finite() || (steps > MAX_PRINT_STEPS && steps_without_waveform <= MAX_PRINT_STEPS)
    {
        let declared = bound.unwrap_or(default_print);
        let mut d = Diagnostic::error(
            Code::Limit,
            format!(
                "the declared source rise/fall/period is too fine for this simulation window: \
                 honouring it would need about {:.0} solver steps, over the limit of {}",
                steps.ceil(),
                MAX_PRINT_STEPS
            ),
        )
        .at(spec.span)
        .with_context("declared waveform timing", format!("{declared} s"))
        .with_context("solver step", format!("{h_print} s"))
        .with_context("effective step", format!("{effective_step} s"));
        if let Some(ms) = spec.max_step {
            d = d.with_context("max_step", format!("{ms} s"));
        }
        return Err(Diagnostics::single(d.with_note(
            "shorten `stop:`, use a longer `rise:`/`fall:`/`period:`, or set a `max_step:` \
             no finer than the waveform needs; the edge is never widened silently to fit",
        )));
    }

    Ok(TranStep {
        h_print,
        waveform_bound: bound,
    })
}

/// The step the engine will actually take for `h_print`, given the circuit and
/// the analysis' explicit `max_step`.
///
/// One function serves both the mapping and the attribution test above, so the
/// two can never disagree about what the engine does.
fn effective_step_for(circuit: &Circuit, spec: &TranSpec, h_print: f64) -> f64 {
    let lte_controls = circuit
        .devices
        .iter()
        .any(|d| matches!(d.kind, DeviceKind::Capacitor | DeviceKind::Inductor));
    let h_max = spec
        .max_step
        .unwrap_or_else(|| h_print.min(spec.stop_s / 50.0));
    if lte_controls {
        h_max
    } else {
        h_max.min(h_print)
    }
}

/// The `tran.*` settings a result carries, so a trace can be told apart from
/// the raw solver grid it came from (plan §4 task A item 6).
fn tran_settings(circuit: &Circuit, spec: &TranSpec, raw_points: usize) -> Vec<(String, String)> {
    let Ok(step) = tran_step_for(circuit, spec) else {
        return Vec::new();
    };
    let mut out = vec![
        ("tran.solver_step".to_string(), format!("{}", step.h_print)),
        ("tran.solve_points".to_string(), raw_points.to_string()),
    ];
    if let Some(b) = step.waveform_bound {
        out.push(("tran.waveform_bound".to_string(), format!("{b}")));
    }
    if let Some(ms) = spec.max_step {
        out.push(("tran.max_step".to_string(), format!("{ms}")));
    }
    if let Some(oi) = spec.output_interval {
        out.push(("tran.output_interval".to_string(), format!("{oi}")));
    }
    out
}

// ---------------------------------------------------------------------------
// Plot selection
// ---------------------------------------------------------------------------

/// Find the plot for an analysis kind by name prefix.
///
/// Required because `simulate_tran` returns `[op1, tran1]`: taking `plots[0]`
/// would silently return the operating point instead of the transient
/// (Phase-0 finding 1).
fn select_plot<'a>(result: &'a SimResult, kind: &str) -> Option<&'a SimPlot> {
    result
        .plots
        .iter()
        .find(|p| p.name.to_ascii_lowercase().starts_with(kind))
}

// ---------------------------------------------------------------------------
// Vector lookup and signal construction
// ---------------------------------------------------------------------------

fn find_vector<'a>(plot: &'a SimPlot, name: &str) -> Option<&'a thevenin_types::SimVector> {
    plot.vecs.iter().find(|v| v.name.eq_ignore_ascii_case(name))
}

/// The engine's branch-current vector name for a device.
fn branch_name(device: &str) -> String {
    format!("{device}#branch")
}

fn real_of(v: &thevenin_types::SimVector) -> Option<Vec<f64>> {
    match &v.data {
        VectorData::Real(d) => Some(d.clone()),
        VectorData::Complex(_) => None,
    }
}

fn complex_of(v: &thevenin_types::SimVector) -> Vec<Complex> {
    match &v.data {
        VectorData::Complex(d) => d.iter().map(|c| Complex::new(c.re, c.im)).collect(),
        VectorData::Real(d) => d.iter().map(|x| Complex::new(*x, 0.0)).collect(),
    }
}

fn make_signal(name: String, unit: Dimension, data: &VectorData) -> Signal {
    match data {
        VectorData::Real(_) => Signal::real(
            name,
            unit,
            match data {
                VectorData::Real(d) => d.clone(),
                VectorData::Complex(_) => unreachable!(),
            },
        ),
        VectorData::Complex(d) => Signal::complex(
            name,
            unit,
            d.iter().map(|c| Complex::new(c.re, c.im)).collect(),
        ),
    }
}

/// Translate an engine vector into a DSL-facing signal name and unit, for the
/// case where the analysis saved no explicit probe list.
fn translate_default_name(
    circuit: &Circuit,
    vector: &thevenin_types::SimVector,
) -> Option<(String, Dimension)> {
    let name = vector.name.as_str();
    if let Some(stripped) = name
        .strip_prefix("v(")
        .or_else(|| name.strip_prefix("V("))
        .and_then(|s| s.strip_suffix(')'))
    {
        return Some((format!("v({stripped})"), units::VOLTAGE));
    }
    if let Some(device) = name.strip_suffix("#branch") {
        // Only name currents for devices that are actually in the circuit, so
        // an internal engine vector cannot masquerade as a user-visible one.
        if circuit.device_id(device).is_some() {
            return Some((format!("i({device})"), units::CURRENT));
        }
        return None;
    }
    None
}

/// Build the signal a probe asks for.
///
/// A differential voltage is computed here: the engine reports only node
/// voltages relative to ground, so `v(a, b)` is `v(a) - v(b)` (spec §5.2).
fn materialise_probe(
    circuit: &Circuit,
    probe: &NamedProbe,
    plot: &SimPlot,
) -> Result<Option<Signal>, Diagnostic> {
    match &probe.probe {
        Probe::NodeVoltage(node) => {
            let node_name = circuit.node_name(*node);
            let vector = find_vector(plot, &format!("v({node_name})")).ok_or_else(|| {
                Diagnostic::error(
                    Code::Backend,
                    format!("no voltage reported for `{node_name}`"),
                )
                .at(probe.span)
            })?;
            Ok(Some(make_signal(
                probe.name.clone(),
                units::VOLTAGE,
                &vector.data,
            )))
        }

        Probe::DifferentialVoltage { pos, neg } => {
            let pn = circuit.node_name(*pos);
            let nn = circuit.node_name(*neg);

            // Prefer a directly reported differential vector if one exists.
            if let Some(v) = find_vector(plot, &format!("v({pn},{nn})"))
                .or_else(|| find_vector(plot, &format!("v({pn}, {nn})")))
            {
                return Ok(Some(make_signal(
                    probe.name.clone(),
                    units::VOLTAGE,
                    &v.data,
                )));
            }

            let pv = find_vector(plot, &format!("v({pn})")).ok_or_else(|| {
                Diagnostic::error(Code::Backend, format!("no voltage reported for `{pn}`"))
                    .at(probe.span)
            })?;
            let nv = find_vector(plot, &format!("v({nn})")).ok_or_else(|| {
                Diagnostic::error(Code::Backend, format!("no voltage reported for `{nn}`"))
                    .at(probe.span)
            })?;

            if pv.len() != nv.len() {
                return Err(Diagnostic::error(
                    Code::Backend,
                    format!(
                        "cannot form `{}`: `v({pn})` has {} samples but `v({nn})` has {}",
                        probe.name,
                        pv.len(),
                        nv.len()
                    ),
                )
                .at(probe.span));
            }

            let data = match (&pv.data, &nv.data) {
                (VectorData::Real(a), VectorData::Real(b)) => {
                    Data::Real(a.iter().zip(b).map(|(x, y)| x - y).collect())
                }
                _ => {
                    let a = complex_of(pv);
                    let b = complex_of(nv);
                    Data::Complex(
                        a.iter()
                            .zip(&b)
                            .map(|(x, y)| Complex::new(x.re - y.re, x.im - y.im))
                            .collect(),
                    )
                }
            };
            Ok(Some(Signal {
                name: probe.name.clone(),
                unit: units::VOLTAGE,
                data,
            }))
        }

        Probe::DeviceCurrent(device) => {
            let d = circuit
                .device(*device)
                .ok_or_else(|| Diagnostic::error(Code::Name, "unknown device").at(probe.span))?;

            // Preferred path: the engine reports the branch current directly.
            if let Some(vector) = find_vector(plot, &branch_name(&d.name)) {
                return Ok(Some(make_signal(
                    probe.name.clone(),
                    units::CURRENT,
                    &vector.data,
                )));
            }

            // The engine emits no branch vector for a resistor. Ohm's law on
            // the two terminal voltages is exact for a linear resistor and
            // needs no model information, so it is a derivation this project
            // is willing to make. It is cross-checked against the engine's own
            // source current in `tests/adapter.rs::divider_op_and_current_direction`
            // (DC) and `derived_resistor_current_agrees_with_source_current_in_ac`
            // (AC). Without it, `i(r1)` would simply be unavailable, which is
            // what the brief forbids faking.
            if d.kind == DeviceKind::Resistor {
                return derive_resistor_current(circuit, d, probe, plot).map(Some);
            }

            Err(Diagnostic::error(
                Code::Backend,
                format!("no branch current reported for `{}`", d.name),
            )
            .at(probe.span)
            .with_note(
                "the engine reports branch currents only for elements that own a branch \
                 unknown (sources and inductors)",
            ))
        }
    }
}

/// Derive a resistor's current from its terminal voltages: `i = (v(p) - v(n)) / R`.
///
/// This is exact for the linear element the IR models, and it is the only
/// derivation this adapter performs. It exists because the engine reports no
/// branch current for resistors (see `_probe/src/bin/currents.rs`), and the
/// brief forbids inventing currents — a derivation is acceptable only if it is
/// verified, which the KCL test in `tests/adapter.rs` does by comparing this
/// result against the engine's own source current in a series loop.
fn derive_resistor_current(
    circuit: &Circuit,
    device: &ir::Device,
    probe: &NamedProbe,
    plot: &SimPlot,
) -> Result<Signal, Diagnostic> {
    let p = device.pos().ok_or_else(|| {
        Diagnostic::error(
            Code::Backend,
            format!("`{}` has no positive terminal", device.name),
        )
        .at(probe.span)
    })?;
    let n = device.neg().ok_or_else(|| {
        Diagnostic::error(
            Code::Backend,
            format!("`{}` has no negative terminal", device.name),
        )
        .at(probe.span)
    })?;

    let pn = circuit.node_name(p);
    let nn = circuit.node_name(n);

    // The engine treats ground as the reference and never emits a vector for
    // it, so a resistor with a grounded terminal has to be handled explicitly:
    // its terminal voltage is zero by definition. Without this, deriving `i(r)`
    // failed for the very common case of a resistor to ground.
    let vp = if p == circuit.ground() {
        None
    } else {
        Some(find_vector(plot, &format!("v({pn})")).ok_or_else(|| {
            Diagnostic::error(
                Code::Backend,
                format!(
                    "no voltage reported for `{pn}`, needed to derive `{}`",
                    probe.name
                ),
            )
            .at(probe.span)
        })?)
    };
    let vn = if n == circuit.ground() {
        None
    } else {
        Some(find_vector(plot, &format!("v({nn})")).ok_or_else(|| {
            Diagnostic::error(
                Code::Backend,
                format!(
                    "no voltage reported for `{nn}`, needed to derive `{}`",
                    probe.name
                ),
            )
            .at(probe.span)
        })?)
    };

    // Both present and mismatched is a real inconsistency; one absent is
    // ground and is expected.
    if let (Some(a), Some(b)) = (vp, vn)
        && a.len() != b.len()
    {
        return Err(Diagnostic::error(
            Code::Backend,
            format!(
                "cannot derive `{}`: `v({pn})` has {} samples but `v({nn})` has {}",
                probe.name,
                a.len(),
                b.len()
            ),
        )
        .at(probe.span));
    }

    let resistance = device
        .param("value")
        .ok_or_else(|| {
            Diagnostic::error(
                Code::Backend,
                format!("`{}` has no resistance value", device.name),
            )
            .at(probe.span)
        })
        .and_then(|q| {
            q.require(units::RESISTANCE).map_err(|e| {
                Diagnostic::error(
                    Code::Dimension,
                    format!("`{}` resistance is not an ohmic value", device.name),
                )
                .at(probe.span)
                .with_dims(e.expected, e.received)
            })
        })?;

    if resistance == 0.0 {
        return Err(Diagnostic::error(
            Code::Value,
            format!(
                "cannot derive the current of `{}`: its resistance is zero",
                device.name
            ),
        )
        .at(probe.span));
    }

    // An AC analysis makes the terminal voltages complex; the quotient by a
    // real resistance stays complex. A grounded terminal contributes zero,
    // and the sign depends on which terminal it was.
    // `v - 0` for one grounded terminal; the sign follows which side it was.
    let one_sided = |v: &thevenin_types::SimVector, sign: f64| match &v.data {
        VectorData::Real(a) => Data::Real(a.iter().map(|x| sign * x / resistance).collect()),
        VectorData::Complex(_) => Data::Complex(
            complex_of(v)
                .iter()
                .map(|c| Complex::new(sign * c.re / resistance, sign * c.im / resistance))
                .collect(),
        ),
    };

    let data = match (vp, vn) {
        (None, None) => Data::Real(vec![0.0]),
        (Some(v), None) => one_sided(v, 1.0),
        (None, Some(v)) => one_sided(v, -1.0),
        (Some(a), Some(b)) => match (&a.data, &b.data) {
            (VectorData::Real(x), VectorData::Real(y)) => {
                Data::Real(x.iter().zip(y).map(|(p, q)| (p - q) / resistance).collect())
            }
            _ => {
                let x = complex_of(a);
                let y = complex_of(b);
                Data::Complex(
                    x.iter()
                        .zip(&y)
                        .map(|(p, q)| {
                            Complex::new((p.re - q.re) / resistance, (p.im - q.im) / resistance)
                        })
                        .collect(),
                )
            }
        },
    };

    Ok(Signal {
        name: probe.name.clone(),
        unit: units::CURRENT,
        data,
    })
}

// ---------------------------------------------------------------------------
// Axis construction
// ---------------------------------------------------------------------------
fn build_axis(
    circuit: &Circuit,
    task: &AnalysisTask,
    plot: &SimPlot,
) -> Result<(Axis, String), Diagnostics> {
    let missing = |what: &str| {
        Diagnostics::single(
            Diagnostic::error(
                Code::Backend,
                format!("the backend did not report a {what} axis"),
            )
            .at(task.span),
        )
    };

    Ok(match &task.kind {
        AnalysisKind::Op => (Axis::None, String::new()),

        AnalysisKind::Tran(_) => {
            let v = find_vector(plot, "time").ok_or_else(|| missing("time"))?;
            let d = real_of(v).ok_or_else(|| missing("real-valued time"))?;
            (Axis::Time(d), v.name.clone())
        }

        AnalysisKind::Ac(_) => {
            let v = find_vector(plot, "frequency").ok_or_else(|| missing("frequency"))?;
            let d = real_of(v).ok_or_else(|| missing("real-valued frequency"))?;
            (Axis::Frequency(d), v.name.clone())
        }

        AnalysisKind::Dc(spec) => {
            // The engine names the sweep vector `v-sweep`. Fall back to a
            // device-parameter vector (`@v1[dc]`) if the primary is absent.
            let v = find_vector(plot, "v-sweep")
                .or_else(|| plot.vecs.iter().find(|v| v.name.starts_with('@')))
                .ok_or_else(|| missing("sweep"))?;
            let d = real_of(v).ok_or_else(|| missing("real-valued sweep"))?;
            let unit = match &spec.sweep.target {
                SweepTarget::SourceValue { device, .. } => circuit
                    .device(*device)
                    .map(|d| match d.kind {
                        DeviceKind::VoltageSource => units::VOLTAGE,
                        DeviceKind::CurrentSource => units::CURRENT,
                        _ => units::DIMENSIONLESS,
                    })
                    .unwrap_or(units::DIMENSIONLESS),
                SweepTarget::Parameter { .. } => units::DIMENSIONLESS,
            };
            let _ = unit;
            (Axis::Parameter(d), v.name.clone())
        }
    })
}
