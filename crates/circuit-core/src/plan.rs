//! The analysis plan: what to run, over which probes, along which sweep.
//!
//! A plan is produced by elaboration and consumed by a backend. It is
//! deliberately independent of the circuit *structure* (brief §7.2): the
//! circuit says what the network is, the plan says what to measure on it.
//!
//! Probes are resolved to typed references ([`NodeId`] / [`DeviceId`]) during
//! elaboration, so a backend never sees a user-written name.

use crate::id::{AnalysisId, DeviceId, NodeId};
use crate::span::SourceSpan;
use crate::units::Quantity;

/// What a saved signal refers to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Probe {
    /// Voltage of `pos` relative to ground.
    NodeVoltage(NodeId),
    /// Voltage `pos - neg`, matching the brief's definition of `v(a, b)`.
    DifferentialVoltage { pos: NodeId, neg: NodeId },
    /// Current through a device, positive in the device's `p -> n` direction.
    DeviceCurrent(DeviceId),
}

impl Probe {
    /// The nodes whose voltages this probe reads.
    ///
    /// Empty for a current probe: which nodes a branch current depends on is
    /// a property of the circuit topology, not of the probe, so the circuit
    /// IR is needed to answer it.
    pub fn voltage_nodes(&self) -> Vec<NodeId> {
        match self {
            Probe::NodeVoltage(n) => vec![*n],
            Probe::DifferentialVoltage { pos, neg } => vec![*pos, *neg],
            Probe::DeviceCurrent(_) => Vec::new(),
        }
    }
}

/// A probe together with the name the user gave it and where it was written.
#[derive(Clone, Debug)]
pub struct NamedProbe {
    /// Display name, e.g. `v(out)` or `i(r1)`.
    pub name: String,
    pub probe: Probe,
    pub span: SourceSpan,
}

// ---------------------------------------------------------------------------
// Sweeps
// ---------------------------------------------------------------------------

/// How a sweep's points are generated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SweepKind {
    /// Fixed step from start to stop.
    Linear,
    /// `points` per decade, logarithmic.
    Decade,
    /// `points` per octave, logarithmic.
    Octave,
    /// `points` total, evenly spaced (used for frequency sweeps with a
    /// linear scale).
    LinearPoints,
}

/// What is being swept.
#[derive(Clone, Debug)]
pub enum SweepTarget {
    /// Source value, e.g. `dc from: 0.V, to: 5.V, step: 1.V` on a specific
    /// source. Reserved for DC sweeps of a source value.
    SourceValue { device: DeviceId, name: String },
    /// A non-topology parameter, swept over the experiment.
    Parameter { name: String },
}

/// A one-dimensional sweep axis.
///
/// Multi-dimensional sweeps are not implemented in this version; the type is
/// a single axis rather than a `Vec` so that a plan cannot silently claim
/// capability the elaborator does not have.
#[derive(Clone, Debug)]
pub struct Sweep {
    pub target: SweepTarget,
    pub start: f64,
    pub stop: f64,
    /// Dimension of the swept quantity, so a parameter sweep can rebuild a
    /// typed value for each point instead of guessing one.
    pub dimension: crate::units::Dimension,
    /// Step, for `Linear`.
    pub step: Option<f64>,
    /// Points per decade/octave, or total points for `LinearPoints`.
    pub points: Option<u32>,
    pub kind: SweepKind,
    /// Whether `stop` is included when it does not land exactly on a step.
    pub include_endpoint: bool,
    pub span: SourceSpan,
}

// ---------------------------------------------------------------------------
// Analysis specifications
// ---------------------------------------------------------------------------

/// A DC sweep specification.
#[derive(Clone, Debug)]
pub struct DcSpec {
    pub sweep: Sweep,
}

/// An AC small-signal specification.
///
/// `phase_rad` is stored in radians internally; the DSL accepts degrees and
/// converts at the boundary. Phase is always reported in the principal range
/// unless the exporter is explicitly asked to unwrap it.
#[derive(Clone, Debug)]
pub struct AcSweep {
    pub start_hz: f64,
    pub stop_hz: f64,
    pub points: u32,
    pub kind: SweepKind,
    pub span: SourceSpan,
}

impl AcSweep {
    /// Number of points this sweep will produce, computed the same way the
    /// backend does, so the front end can enforce result-size limits before
    /// running anything.
    pub fn point_count(&self) -> u64 {
        if self.stop_hz <= self.start_hz || self.start_hz <= 0.0 {
            return 0;
        }
        let decades = (self.stop_hz / self.start_hz).log10();
        match self.kind {
            // Points per decade/octave counts intervals, so the point count
            // is one more than the number of intervals.
            SweepKind::Decade => (decades * self.points as f64).round() as u64 + 1,
            SweepKind::Octave => {
                ((self.stop_hz / self.start_hz).log2() * self.points as f64).round() as u64 + 1
            }
            // A linear sweep's `points` is the total number of samples, which
            // the engine honours exactly. Verified against the engine in
            // `circuit-backend/tests/adapter.rs::two_analyses_of_the_same_kind_get_distinct_names`.
            SweepKind::LinearPoints | SweepKind::Linear => self.points as u64,
        }
    }
}

/// A transient specification.
///
/// Three concepts are deliberately kept apart here (brief §8.4); conflating
/// them is what let a coarse output request silently rewrite a declared
/// source waveform in an earlier revision:
///
/// - **`max_step`** bounds the *integration* step. It is handed to the engine
///   as its step bound and never promises anything about the returned time
///   axis, which stays solver-chosen and generally non-uniform.
/// - **`output_interval`** requests *output sampling*. It is **not** a solver
///   parameter: the run is executed with the solver's own steps and the
///   returned trace is resampled afterwards (in `circuit-results`, which is
///   the layer above this one). `None` keeps the solver's own grid.
/// - The **source waveform** is whatever the circuit declares. No analysis
///   option may widen a declared edge: the backend picks a print step that
///   cannot clamp `rise`/`fall`/`period` (see
///   `circuit_backend::thevenin::print_step_for`).
#[derive(Clone, Debug)]
pub struct TranSpec {
    pub start_s: f64,
    pub stop_s: f64,
    pub max_step: Option<f64>,
    /// Requested output interval in seconds: a finite value greater than zero,
    /// or `None` for "whatever the solver produced". Validated at the input
    /// layer, and later used only to resample the delivered trace — never to
    /// choose an integration step.
    pub output_interval: Option<f64>,
    /// Use initial conditions instead of a DC operating point. Not exposed by
    /// the language yet; the backend path is unverified (see
    /// `docs/backend-evaluation.md`).
    pub uic: bool,
    pub span: SourceSpan,
}

/// Which analysis to run.
#[derive(Clone, Debug)]
pub enum AnalysisKind {
    Op,
    Dc(DcSpec),
    Ac(AcSweep),
    Tran(TranSpec),
}

impl AnalysisKind {
    pub fn name(&self) -> &'static str {
        match self {
            AnalysisKind::Op => "op",
            AnalysisKind::Dc(_) => "dc",
            AnalysisKind::Ac(_) => "ac",
            AnalysisKind::Tran(_) => "tran",
        }
    }
}

/// One analysis task: what to run and which signals to save.
#[derive(Clone, Debug)]
pub struct AnalysisTask {
    pub id: AnalysisId,
    pub kind: AnalysisKind,
    pub probes: Vec<NamedProbe>,
    pub span: SourceSpan,
}

/// Which reduction a `measure` statement asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MeasureKind {
    Max,
    Min,
    /// Time integral of the signal divided by the elapsed time.
    Avg,
    /// Square root of the time integral of the square, over the elapsed time.
    Rms,
}

impl MeasureKind {
    pub fn name(self) -> &'static str {
        match self {
            MeasureKind::Max => "max",
            MeasureKind::Min => "min",
            MeasureKind::Avg => "avg",
            MeasureKind::Rms => "rms",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "max" => MeasureKind::Max,
            "min" => MeasureKind::Min,
            "avg" => MeasureKind::Avg,
            "rms" => MeasureKind::Rms,
            _ => return None,
        })
    }

    /// Whether this reduction needs a time axis.
    ///
    /// `avg` and `rms` are integrals over time, so an analysis with no time
    /// axis (OP, DC, AC) cannot support them (brief §8.5).
    pub fn needs_time_axis(self) -> bool {
        matches!(self, MeasureKind::Avg | MeasureKind::Rms)
    }
}

/// A `measure :name, <kind>: <probe>` request.
#[derive(Clone, Debug)]
pub struct MeasureRequest {
    pub name: String,
    pub kind: MeasureKind,
    pub target: Probe,
    /// The probe as written, for reporting.
    pub target_name: String,
    pub span: SourceSpan,
    pub kind_span: SourceSpan,
}

/// A complete experiment: a circuit plus the analyses to run on it.
#[derive(Clone, Debug)]
pub struct AnalysisPlan {
    pub name: String,
    /// Name of the circuit definition this plan runs against.
    pub circuit_name: String,
    pub tasks: Vec<AnalysisTask>,
    /// Parameter overrides declared on the experiment itself, applied after
    /// subcircuit defaults and before sweep points (brief §5.3).
    pub param_overrides: Vec<(String, Quantity, SourceSpan)>,
    /// Measurements to evaluate against the results.
    pub measures: Vec<MeasureRequest>,
    pub span: SourceSpan,
}

impl AnalysisPlan {
    pub fn task(&self, id: AnalysisId) -> Option<&AnalysisTask> {
        self.tasks.iter().find(|t| t.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ac(start: f64, stop: f64, points: u32, kind: SweepKind) -> AcSweep {
        AcSweep {
            start_hz: start,
            stop_hz: stop,
            points,
            kind,
            span: SourceSpan::synthetic(),
        }
    }

    #[test]
    fn decade_point_count_matches_ngspice_convention() {
        // 2 decades at 10 points/decade -> 20 intervals -> 21 points.
        assert_eq!(ac(10.0, 1000.0, 10, SweepKind::Decade).point_count(), 21);
        // 6 decades at 10/decade -> 61 points, matching the phase-0 probe.
        assert_eq!(ac(10.0, 1e7, 10, SweepKind::Decade).point_count(), 61);
    }

    #[test]
    fn linear_point_count_is_total_points() {
        // `points: n` on a linear sweep means n samples in total, which is
        // what the engine produces.
        assert_eq!(
            ac(100.0, 1000.0, 50, SweepKind::LinearPoints).point_count(),
            50
        );
        assert_eq!(
            ac(100.0, 1000.0, 11, SweepKind::LinearPoints).point_count(),
            11
        );
    }

    #[test]
    fn degenerate_ranges_report_zero() {
        assert_eq!(ac(1000.0, 10.0, 10, SweepKind::Decade).point_count(), 0);
        assert_eq!(ac(0.0, 10.0, 10, SweepKind::Decade).point_count(), 0);
    }

    #[test]
    fn differential_probe_touches_both_nodes() {
        let p = Probe::DifferentialVoltage {
            pos: NodeId(1),
            neg: NodeId(2),
        };
        assert_eq!(p.voltage_nodes(), vec![NodeId(1), NodeId(2)]);
        assert!(Probe::DeviceCurrent(DeviceId(0)).voltage_nodes().is_empty());
    }
}
