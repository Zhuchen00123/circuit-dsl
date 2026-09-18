//! The backend contract.
//!
//! A backend turns `(Circuit, AnalysisPlan)` into result datasets. The
//! interface is deliberately narrow: a backend may not change the circuit, may
//! not invent signals, and must declare what it supports up front so that
//! unsupported requests fail loudly instead of being silently ignored
//! (brief §7.2).
//!
//! Only one implementation exists ([`crate::thevenin::TheveninBackend`]).
//! The trait is kept because it is the seam that keeps `circuit-core` free of
//! third-party types, not because a second backend is planned.

use circuit_core::diagnostic::{Code, Diagnostic, Diagnostics};
use circuit_core::ir::Circuit;
use circuit_core::plan::AnalysisPlan;
use circuit_results::Dataset;

/// What a backend can do. Reported by `cdsl capabilities`.
#[derive(Clone, Debug)]
pub struct BackendCapabilities {
    pub name: String,
    pub version: String,
    /// Analysis kinds, e.g. `["op", "dc", "ac", "tran"]`.
    pub analyses: Vec<&'static str>,
    /// Device kinds, by IR name.
    pub devices: Vec<&'static str>,
    /// Whether `run` can execute a sweep over a non-topology parameter.
    pub parameter_sweep: bool,
    /// Whether a DC sweep over a source value is handled natively.
    pub source_sweep: bool,
    /// Anything a user should know before relying on this backend.
    pub notes: Vec<String>,
}

impl BackendCapabilities {
    pub fn supports_analysis(&self, kind: &str) -> bool {
        self.analyses.contains(&kind)
    }

    pub fn supports_device(&self, kind: &str) -> bool {
        self.devices.contains(&kind)
    }
}

/// One backend invocation's worth of results.
#[derive(Clone, Debug)]
pub struct SimulationResults {
    /// One dataset per analysis task, keyed by the task's id.
    pub datasets: Vec<Dataset>,
}

impl SimulationResults {
    pub fn empty() -> Self {
        Self {
            datasets: Vec::new(),
        }
    }

    pub fn dataset_for(&self, analysis: &str) -> Option<&Dataset> {
        self.datasets.iter().find(|d| d.analysis == analysis)
    }
}

/// A simulation backend.
pub trait SimulationBackend {
    fn capabilities(&self) -> BackendCapabilities;

    /// Reject requests this backend cannot honour, before any work happens.
    ///
    /// Returning `Ok(())` here means `run` must not then fail for an
    /// "unsupported" reason. Diagnostics must name the offending construction
    /// and its source location where one is available.
    fn validate(&self, circuit: &Circuit, plan: &AnalysisPlan) -> Result<(), Diagnostics>;

    /// Execute every task in the plan.
    ///
    /// Implementations must not mutate the circuit and must report failure
    /// rather than returning empty-but-successful results (brief §8.1).
    fn run(
        &mut self,
        circuit: &Circuit,
        plan: &AnalysisPlan,
    ) -> Result<SimulationResults, Diagnostics>;
}

// ---------------------------------------------------------------------------
// Error classification
// ---------------------------------------------------------------------------

/// Classify a backend failure message into a diagnostic code.
///
/// The mapping is intentionally conservative: a message is only given a
/// specific code when it unambiguously says so. Everything else becomes
/// [`Code::Backend`], and the original text is preserved verbatim in the
/// diagnostic so nothing is lost or invented (brief §9).
pub fn classify_backend_failure(message: &str) -> Code {
    let m = message.to_ascii_lowercase();
    if m.contains("singular") {
        Code::Singular
    } else if m.contains("converge") || m.contains("convergence") {
        Code::Convergence
    } else if m.contains("not supported") || m.contains("unsupported") {
        Code::Unsupported
    } else {
        Code::Backend
    }
}

/// Build a diagnostic from a raw backend failure.
pub fn backend_failure(message: impl Into<String>) -> Diagnostic {
    let message = message.into();
    let code = classify_backend_failure(&message);
    let headline = match code {
        Code::Singular => "the circuit has no unique solution (singular system)",
        Code::Convergence => "the solver did not converge",
        Code::Unsupported => "the backend does not support this circuit or analysis",
        _ => "the simulation backend failed",
    };
    Diagnostic::error(code, headline).with_note(format!("backend: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn singular_is_recognised() {
        assert_eq!(
            classify_backend_failure(
                "simulation failed: failed to solve MNA system: matrix is singular, cannot solve"
            ),
            Code::Singular
        );
        assert_eq!(
            classify_backend_failure("singular matrix: zero pivot at row 3"),
            Code::Singular
        );
    }

    #[test]
    fn convergence_is_recognised() {
        assert_eq!(
            classify_backend_failure("Newton-Raphson failed to converge after 100 iterations"),
            Code::Convergence
        );
    }

    #[test]
    fn unsupported_is_recognised() {
        assert_eq!(
            classify_backend_failure("unsupported element for MNA assembly: element `x`: ..."),
            Code::Unsupported
        );
    }

    /// An unrecognised failure must not be given a confident code, and the
    /// original text must survive into the diagnostic.
    #[test]
    fn unknown_failures_keep_their_text() {
        let d = backend_failure("something entirely new went wrong");
        assert_eq!(d.code, Code::Backend);
        let rendered = d.render_plain();
        assert!(
            rendered.contains("something entirely new went wrong"),
            "{rendered}"
        );
    }

    #[test]
    fn capabilities_answer_queries() {
        let caps = BackendCapabilities {
            name: "x".into(),
            version: "1".into(),
            analyses: vec!["op", "ac"],
            devices: vec!["resistor"],
            parameter_sweep: true,
            source_sweep: true,
            notes: Vec::new(),
        };
        assert!(caps.supports_analysis("op"));
        assert!(!caps.supports_analysis("tran"));
        assert!(caps.supports_device("resistor"));
        assert!(!caps.supports_device("mosfet"));
    }
}
