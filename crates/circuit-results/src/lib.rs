//! Result datasets, result expressions, measurements and export.
//!
//! This crate is the layer between a simulation and the user's eyes. Its
//! dependency direction is `core <- results <- cli`: it depends on
//! `circuit-core` for diagnostics, dimensions and limits, and on **no**
//! backend. A backend adapter converts the engine's own types into the neutral
//! [`Dataset`] defined here.
//!
//! # What lives here
//!
//! - [`dataset`] — the neutral result: an axis, real or complex signals with
//!   units, backend metadata, and validation that rejects malformed or
//!   oversized results instead of truncating them.
//! - [`expr`] — the result expression AST (`v(a)`, `v(a,b)`, `i(r1)`,
//!   `abs`, `sqrt`, `min`, `max`, gain in dB) and its evaluator.
//! - [`measure`] — `max`, `min`, `avg`, `rms`; the latter two as time
//!   integrals on a non-uniform axis (spec §7).
//! - [`resample`] — the independent output grid for `tran output_interval:`;
//!   a view of the trace, never an input to the solver.
//! - [`export`] — CSV and JSON, with a defined policy for non-finite samples
//!   (`null` in JSON, an empty field in CSV, plus a warning).
//!
//! # Example
//!
//! ```
//! use circuit_core::units::VOLTAGE;
//! use circuit_core::Limits;
//! use circuit_results::dataset::{Axis, BackendInfo, Dataset, Signal};
//! use circuit_results::measure::{Measurement, measure_signal};
//!
//! // A transient result on a non-uniform time axis: x = t.
//! let dataset = Dataset::new(
//!     "response",
//!     "tran1",
//!     "tran",
//!     Axis::Time(vec![0.0, 1.0, 2.0, 4.0]),
//!     vec![Signal::real("v(out)", VOLTAGE, vec![0.0, 1.0, 2.0, 4.0])],
//!     BackendInfo::new("example", "0"),
//!     &Limits::default(),
//! )
//! .expect("well-formed result");
//!
//! // avg is ∫x dt / ∫dt = 8/4 = 2, not the sample mean 1.75.
//! let avg = measure_signal(Measurement::Avg, "vavg", "v(out)", &dataset).expect("time axis");
//! assert!((avg.value - 2.0).abs() < 1e-12);
//! ```

// `Diagnostic` is the project's shared user-facing error type: a message, up
// to two labelled spans, notes and context. It is ~144 bytes, which trips
// `clippy::result_large_err` on every fallible function here. Boxing it would
// push `Box<Diagnostic>` into the public API of this crate and force every
// caller (the CLI, the backend adapter) to unbox before rendering, for a size
// that is irrelevant next to the allocations the error path already makes.
#![allow(clippy::result_large_err)]

pub mod dataset;
pub mod export;
pub mod expr;
pub mod measure;
pub mod resample;

pub use dataset::{Axis, BackendInfo, Complex, Data, Dataset, Signal, normalize_signal_name};
pub use export::{
    Export, SCHEMA, non_finite_diagnostics, to_csv, to_csv_with_diagnostics, to_json,
    to_json_value, to_json_with_diagnostics,
};
pub use expr::{Expr, Value, eval, from_ir};
pub use measure::{Measured, Measurement, measure, measure_signal, reduce};
pub use resample::{OutputGrid, resample_time};

/// Number formatting lives in `circuit-core` so the REPL, the CSV writer and
/// the measurement summaries cannot drift apart. Re-exported here because this
/// is where callers have always found it.
pub use circuit_core::{MAX_PLAIN_DIGITS, format_number};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_numbers_are_not_reformatted() {
        assert_eq!(format_number(0.0), "0");
        assert_eq!(format_number(-1.5), "-1.5");
        assert_eq!(format_number(1000.0), "1000");
        assert_eq!(format_number(1e-6), "0.000001");
    }

    #[test]
    fn short_but_extreme_numbers_use_exponent_notation() {
        let text = format_number(1e-300);
        assert!(text.len() <= MAX_PLAIN_DIGITS, "{text}");
        assert_eq!(text, "1e-300");
        assert_eq!(format_number(1e300), "1e300");

        // Both forms round-trip to the same value.
        for value in [1e-300, 1e300, 1.234_567_890_123_456_7e-17] {
            let parsed: f64 = format_number(value).parse().expect("parses back");
            assert_eq!(parsed, value);
        }
    }

    #[test]
    fn non_finite_values_are_left_to_the_caller() {
        assert_eq!(format_number(f64::NAN), "NaN");
        assert_eq!(format_number(f64::INFINITY), "inf");
    }

    /// The whole layer end to end: build a result, measure it, export it.
    #[test]
    fn pipeline_from_dataset_to_files() {
        use crate::dataset::{Axis, BackendInfo, Dataset, Signal};
        use circuit_core::Limits;
        use circuit_core::units::{CURRENT, VOLTAGE};

        let dataset = Dataset::new(
            "response",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1e-3, 3e-3]),
            vec![
                Signal::real("v(out)", VOLTAGE, vec![0.0, 2.0, 2.0]),
                Signal::complex("i(r1)", CURRENT, vec![Complex::new(1e-3, 0.0); 3]),
            ],
            BackendInfo::new("thevenin", "0.5.0").with_setting("max_step", "1us"),
            &Limits::default(),
        )
        .expect("well-formed result");

        // rms over the non-uniform axis: ∫x²dt = 0.5*(0+4)*1e-3 +
        // 0.5*(4+4)*2e-3 = 1e-2, width 3e-3, so rms = sqrt(10/3).
        let rms = measure_signal(Measurement::Rms, "vrms", "v(out)", &dataset)
            .expect("a transient result has a time axis");
        assert!(
            (rms.value - (10.0f64 / 3.0).sqrt()).abs() < 1e-12,
            "{rms:?}"
        );

        let csv = to_csv(&dataset).expect("csv");
        assert!(csv.starts_with("time,v(out),i(r1)_re,i(r1)_im\n"), "{csv}");
        assert_eq!(csv.lines().count(), 4);

        let json = to_json(&dataset).expect("json");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(parsed["analysis"], serde_json::Value::from("tran1"));
        assert_eq!(
            parsed["signals"][1]["values"][0]["re"],
            serde_json::Value::from(0.001)
        );
    }
}
