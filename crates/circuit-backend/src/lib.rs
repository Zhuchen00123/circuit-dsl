// `circuit_core::Diagnostic` is the project's single user-facing error type and
// is intentionally not boxed; see the note in `circuit-dsl` for the reasoning.
#![allow(clippy::result_large_err)]

//! Simulation backends.
//!
//! [`backend`] holds the contract ([`SimulationBackend`], capabilities, error
//! classification). [`thevenin`] holds the one implementation, which maps the
//! project's IR onto Thevenin 0.5.0 in memory. [`sweep`] drives parameter
//! sweeps, which the engine cannot do in a single call.
//!
//! The crate sits between `circuit-core` (which must stay free of third-party
//! types) and `circuit-results` (which must stay free of backends):
//!
//! ```text
//! core <- results <- backend <- cli
//! ```
//!
//! Selection, version pinning and the measurements behind them are recorded in
//! `docs/backend-evaluation.md`; nothing here should be trusted beyond what
//! that document proves was actually run.

pub mod backend;
pub mod sweep;
pub mod thevenin;

pub use backend::{
    BackendCapabilities, SimulationBackend, SimulationResults, backend_failure,
    classify_backend_failure,
};
pub use sweep::{SweepError, SweepOutcome, run_parameter_sweep, sweep_coordinates};
pub use thevenin::{BACKEND_VERSION, TheveninBackend};
