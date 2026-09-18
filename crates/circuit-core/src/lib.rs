//! Core data structures shared by the whole project.
//!
//! This crate owns:
//!
//! - [`span`] and [`diagnostic`] — source locations and the user-facing
//!   diagnostic type, so every layer reports errors in the same format.
//! - [`units`] — dimensions and quantities, and the unit-literal grammar.
//! - [`id`] — typed identifiers.
//! - [`ir`] — the circuit IR.
//! - [`plan`] — the analysis plan.
//! - [`limits`] — elaboration and result-size bounds.
//!
//! It depends on **no** backend and no parser. The dependency direction is
//! `core <- {dsl, backend, results} <- cli`.

pub mod connectivity;
pub mod diagnostic;
pub mod id;
pub mod ir;
pub mod limits;
pub mod plan;
pub mod span;
pub mod units;

pub use connectivity::{FloatingKind, FloatingNode, conducts_dc, floating_nodes};
pub use diagnostic::{Code, Diagnostic, Diagnostics, Severity};
pub use id::{AnalysisId, CircuitId, DeviceId, GROUND, ModelId, NodeId};
pub use ir::{
    AcSpec, Circuit, Device, DeviceKind, InstanceStep, Model, ModelKind, Node, NodeKind,
    SourceSpec, Waveform,
};
pub use limits::Limits;
pub use plan::{
    AcSweep, AnalysisKind, AnalysisPlan, AnalysisTask, MeasureKind, MeasureRequest, NamedProbe,
    Probe, Sweep, SweepKind, SweepTarget, TranSpec,
};
pub use span::{SourceId, SourceMap, SourceSpan, Spanned};
pub use units::{Dimension, Quantity};
