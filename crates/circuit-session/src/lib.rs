//! Session state and experiment execution.
//!
//! Two things live here, both deliberately free of any terminal dependency so
//! they can be tested directly and reused by an editor front end:
//!
//! - [`session`] — the definitions and variables an interactive session holds,
//!   the commands it understands, and the rules for replacing definitions.
//! - [`execute`] — running an experiment, once or as a parameter sweep, which
//!   the file-mode CLI and the REPL both use so there is exactly one answer to
//!   "what does this experiment produce".
//!
//! Pipeline position: `core <- results <- backend <- session <- cli`.

pub mod execute;
pub mod format;
pub mod session;

pub use execute::{
    Format, RunOutcome, RunRequest, evaluate_measures, execute, parameter_sweep_of, write_datasets,
};
pub use session::{Options, Reply, Session, completions};
