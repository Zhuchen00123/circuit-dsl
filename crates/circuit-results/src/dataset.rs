//! Result datasets: the neutral, backend-independent form of a simulation
//! result.
//!
//! A [`Dataset`] is what a backend adapter produces and what expressions,
//! measurements and exporters consume. It mentions no solver type: the
//! adapter converts the engine's own complex/vector types into [`Data`], so
//! `circuit-results` depends only on `circuit-core` (the dependency direction
//! is `core <- results <- cli`).
//!
//! # Shape rules
//!
//! - Every signal in one dataset has the **same** sample count as the axis.
//! - [`Axis::None`] (an operating point) is *not* an axis of length zero: it
//!   means "there is no independent variable", and each signal then carries
//!   exactly one scalar. [`Dataset::sample_count`] distinguishes the two.
//! - [`Dataset::validate`] enforces the shape rules and the result-size limit.
//!   It reports diagnostics; it never truncates silently.
//!
//! # Signal names
//!
//! Names are compared case-insensitively **and ignoring whitespace**, so
//! `v(out)`, `V(OUT)` and `v ( out )` are the same signal. The elaborator and
//! the backend only have to agree on the textual probe name, not on its
//! spacing.

use std::collections::HashMap;
use std::fmt;
use std::ops::{Add, Div, Mul, Neg, Sub};

use circuit_core::units::{Dimension, FREQUENCY, TIME};
use circuit_core::{Code, Diagnostic, Diagnostics, Limits};

// ---------------------------------------------------------------------------
// Complex
// ---------------------------------------------------------------------------

/// A complex sample.
///
/// Deliberately independent of any backend's complex type: the adapter
/// converts at the boundary, so this crate can be tested without a solver.
///
/// Arithmetic is provided through the standard operator traits, so `a * b`
/// works for two samples. Division follows the usual
/// `(a+bi)/(c+di) = ((ac+bd) + (bc-ad)i) / (c^2+d^2)`.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub struct Complex {
    pub re: f64,
    pub im: f64,
}

impl Complex {
    pub const ZERO: Self = Self { re: 0.0, im: 0.0 };
    pub const ONE: Self = Self { re: 1.0, im: 0.0 };

    pub const fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }

    /// `magnitude * (cos(phase) + i*sin(phase))`.
    pub fn from_polar(magnitude: f64, phase_rad: f64) -> Self {
        Self {
            re: magnitude * phase_rad.cos(),
            im: magnitude * phase_rad.sin(),
        }
    }

    /// `|z|`, the distance from the origin.
    pub fn magnitude(self) -> f64 {
        self.re.hypot(self.im)
    }

    /// `arg z` in radians, in `(-pi, pi]`. `atan2(0, 0)` is `0`, matching the
    /// convention used for a zero phasor.
    pub fn phase_rad(self) -> f64 {
        self.im.atan2(self.re)
    }

    /// `arg z` in degrees, the unit the language accepts and prints.
    pub fn phase_deg(self) -> f64 {
        self.phase_rad().to_degrees()
    }

    pub fn is_finite(self) -> bool {
        self.re.is_finite() && self.im.is_finite()
    }

    pub fn is_zero(self) -> bool {
        self.re == 0.0 && self.im == 0.0
    }

    /// `|z|^2`, cheaper than `magnitude()` when the square is all that is
    /// needed.
    pub fn norm_sqr(self) -> f64 {
        self.re * self.re + self.im * self.im
    }

    pub fn conjugate(self) -> Self {
        Self {
            re: self.re,
            im: -self.im,
        }
    }

    /// Multiply both components by a real factor.
    pub fn scale(self, k: f64) -> Self {
        Self {
            re: self.re * k,
            im: self.im * k,
        }
    }
}

impl Add for Complex {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.re + rhs.re, self.im + rhs.im)
    }
}

impl Sub for Complex {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.re - rhs.re, self.im - rhs.im)
    }
}

impl Mul for Complex {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self::new(
            self.re * rhs.re - self.im * rhs.im,
            self.re * rhs.im + self.im * rhs.re,
        )
    }
}

impl Div for Complex {
    type Output = Self;
    fn div(self, rhs: Self) -> Self {
        let d = rhs.norm_sqr();
        Self::new(
            (self.re * rhs.re + self.im * rhs.im) / d,
            (self.im * rhs.re - self.re * rhs.im) / d,
        )
    }
}

impl Neg for Complex {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.re, -self.im)
    }
}

impl From<f64> for Complex {
    fn from(value: f64) -> Self {
        Self::new(value, 0.0)
    }
}

impl From<Complex> for (f64, f64) {
    fn from(value: Complex) -> Self {
        (value.re, value.im)
    }
}

impl fmt::Display for Complex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{:+}i", self.re, self.im)
    }
}

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

/// Either a real or a complex sample vector.
///
/// The two variants are kept apart rather than storing everything as complex:
/// OP/DC/TRAN results are real, and a dataset claiming to be complex when it
/// is not would force every consumer to deal with zero imaginary parts.
#[derive(Clone, PartialEq, Debug)]
pub enum Data {
    Real(Vec<f64>),
    Complex(Vec<Complex>),
}

impl Data {
    pub fn real(values: Vec<f64>) -> Self {
        Self::Real(values)
    }

    pub fn complex(values: Vec<Complex>) -> Self {
        Self::Complex(values)
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Real(v) => v.len(),
            Self::Complex(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_complex(&self) -> bool {
        matches!(self, Self::Complex(_))
    }

    /// The samples if this is real data.
    pub fn as_real(&self) -> Option<&[f64]> {
        match self {
            Self::Real(v) => Some(v),
            Self::Complex(_) => None,
        }
    }

    /// The samples if this is complex data.
    pub fn as_complex(&self) -> Option<&[Complex]> {
        match self {
            Self::Complex(v) => Some(v),
            Self::Real(_) => None,
        }
    }

    /// Sample `i` as a complex number, promoting real data.
    pub fn sample(&self, i: usize) -> Option<Complex> {
        match self {
            Self::Real(v) => v.get(i).copied().map(Complex::from),
            Self::Complex(v) => v.get(i).copied(),
        }
    }

    /// Every sample as a real magnitude.
    ///
    /// For complex data this is `|z|`. For **real** data it is the sample
    /// itself, *not* its absolute value: a measurement like `max` over a real
    /// signal must be able to return a negative extreme. Use
    /// [`crate::expr::Expr::abs`] when the absolute value is wanted.
    pub fn magnitudes(&self) -> Vec<f64> {
        match self {
            Self::Real(v) => v.clone(),
            Self::Complex(v) => v.iter().map(|c| c.magnitude()).collect(),
        }
    }

    /// Indices of samples that are not finite (`NaN` or `+-inf`).
    ///
    /// A complex sample counts as non-finite when either component is.
    pub fn non_finite_indices(&self) -> Vec<usize> {
        match self {
            Self::Real(v) => v
                .iter()
                .enumerate()
                .filter(|(_, x)| !x.is_finite())
                .map(|(i, _)| i)
                .collect(),
            Self::Complex(v) => v
                .iter()
                .enumerate()
                .filter(|(_, c)| !c.is_finite())
                .map(|(i, _)| i)
                .collect(),
        }
    }
}

impl From<Vec<f64>> for Data {
    fn from(value: Vec<f64>) -> Self {
        Self::Real(value)
    }
}

impl From<Vec<Complex>> for Data {
    fn from(value: Vec<Complex>) -> Self {
        Self::Complex(value)
    }
}

// ---------------------------------------------------------------------------
// Axis
// ---------------------------------------------------------------------------

/// The independent variable of a result.
///
/// `None` is the operating point: there is no independent variable and each
/// signal holds one scalar. The other variants always carry their sample
/// values, so an empty sweep is an empty vector rather than a missing axis.
#[derive(Clone, PartialEq, Debug, Default)]
pub enum Axis {
    #[default]
    None,
    /// Transient time in seconds; generally non-uniform (spec §5.1).
    Time(Vec<f64>),
    /// AC frequency in hertz.
    Frequency(Vec<f64>),
    /// A DC sweep parameter, in whatever unit the swept quantity has.
    Parameter(Vec<f64>),
}

impl Axis {
    /// Number of axis samples; `0` for [`Axis::None`].
    pub fn len(&self) -> usize {
        self.samples().len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples().is_empty()
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    pub fn is_time(&self) -> bool {
        matches!(self, Self::Time(_))
    }

    /// The sample values, empty for [`Axis::None`].
    pub fn samples(&self) -> &[f64] {
        match self {
            Self::None => &[],
            Self::Time(v) | Self::Frequency(v) | Self::Parameter(v) => v,
        }
    }

    /// Stable identifier used in CSV headers and JSON: `none`, `time`,
    /// `frequency`, `parameter`.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Time(_) => "time",
            Self::Frequency(_) => "frequency",
            Self::Parameter(_) => "parameter",
        }
    }

    /// The dimension of the axis values, when it is known.
    ///
    /// A swept parameter can be a resistance, a voltage, anything; its unit is
    /// not recoverable from the values alone, so it is reported as unknown.
    pub fn unit(&self) -> Option<Dimension> {
        match self {
            Self::None | Self::Parameter(_) => None,
            Self::Time(_) => Some(TIME),
            Self::Frequency(_) => Some(FREQUENCY),
        }
    }

    pub fn first(&self) -> Option<f64> {
        self.samples().first().copied()
    }

    pub fn last(&self) -> Option<f64> {
        self.samples().last().copied()
    }

    /// Human-readable form for diagnostics.
    pub fn describe(&self) -> String {
        match self {
            Self::None => "no axis (operating point)".to_string(),
            other => format!("a {} axis", other.kind_name()),
        }
    }
}

// ---------------------------------------------------------------------------
// Signal
// ---------------------------------------------------------------------------

/// One saved signal: a name, a unit, and its samples.
#[derive(Clone, PartialEq, Debug)]
pub struct Signal {
    /// Display name, e.g. `v(out)` or `i(r1)`.
    pub name: String,
    /// The dimension of the samples. Voltages are [`circuit_core::units::VOLTAGE`],
    /// currents [`circuit_core::units::CURRENT`].
    pub unit: Dimension,
    pub data: Data,
}

impl Signal {
    pub fn new(name: impl Into<String>, unit: Dimension, data: Data) -> Self {
        Self {
            name: name.into(),
            unit,
            data,
        }
    }

    pub fn real(name: impl Into<String>, unit: Dimension, values: Vec<f64>) -> Self {
        Self::new(name, unit, Data::Real(values))
    }

    pub fn complex(name: impl Into<String>, unit: Dimension, values: Vec<Complex>) -> Self {
        Self::new(name, unit, Data::Complex(values))
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn is_complex(&self) -> bool {
        self.data.is_complex()
    }
}

/// Canonical form used for signal lookup: lower-cased with whitespace removed.
pub fn normalize_signal_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

// ---------------------------------------------------------------------------
// Backend metadata
// ---------------------------------------------------------------------------

/// Which engine produced the data, and with which settings.
///
/// This travels into the JSON export so a result file can be reproduced.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct BackendInfo {
    pub name: String,
    pub version: String,
    /// Solver settings, e.g. `("abstol", "1e-12")`. Ordered pairs rather than
    /// a map: the adapter decides the order and duplicates are allowed.
    pub settings: Vec<(String, String)>,
}

impl BackendInfo {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            settings: Vec::new(),
        }
    }

    pub fn with_setting(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.settings.push((key.into(), value.into()));
        self
    }

    /// Read back a setting by key, for tests and for callers that report the
    /// solve configuration next to a result.
    pub fn setting(&self, key: &str) -> Option<&str> {
        self.settings
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

// ---------------------------------------------------------------------------
// Dataset
// ---------------------------------------------------------------------------

/// One analysis' worth of results.
///
/// Not `PartialEq`: [`Diagnostic`] carries free-form notes and is compared by
/// rendering, not by value.
#[derive(Clone, Debug)]
pub struct Dataset {
    /// Experiment name, e.g. `response`.
    pub experiment: String,
    /// Analysis id/name within the experiment, e.g. `tran1`.
    pub analysis: String,
    /// `"op" | "dc" | "ac" | "tran"`.
    pub kind: String,
    pub axis: Axis,
    pub signals: Vec<Signal>,
    /// Diagnostics that belong to this result: backend warnings, export
    /// warnings. Errors normally stop the run before a dataset exists.
    pub diagnostics: Vec<Diagnostic>,
    pub backend: BackendInfo,
}

impl Dataset {
    /// Build a dataset, rejecting one that violates the shape rules or the
    /// result-size limit.
    ///
    /// The returned diagnostics describe *every* problem found, not just the
    /// first, so a `run` can report all of them at once.
    pub fn new(
        experiment: impl Into<String>,
        analysis: impl Into<String>,
        kind: impl Into<String>,
        axis: Axis,
        signals: Vec<Signal>,
        backend: BackendInfo,
        limits: &Limits,
    ) -> Result<Self, Diagnostics> {
        let dataset = Self {
            experiment: experiment.into(),
            analysis: analysis.into(),
            kind: kind.into(),
            axis,
            signals,
            diagnostics: Vec::new(),
            backend,
        };
        dataset.validate(limits)?;
        Ok(dataset)
    }

    /// Check the shape rules and the result-size limit.
    ///
    /// Nothing is truncated: a dataset that is too large is rejected with a
    /// [`Code::Limit`] diagnostic and the caller decides what to do (spec §9).
    pub fn validate(&self, limits: &Limits) -> Result<(), Diagnostics> {
        let mut diags = Diagnostics::new();
        let no_axis = self.axis.is_none();
        let axis_len = self.axis.len();

        for signal in &self.signals {
            let expected = if no_axis { 1 } else { axis_len };
            if signal.data.len() != expected {
                let shape = if no_axis {
                    "an operating-point result (no axis) expects exactly 1 sample".to_string()
                } else {
                    format!(
                        "the {} axis has {} samples",
                        self.axis.kind_name(),
                        axis_len
                    )
                };
                diags.push(
                    Diagnostic::error(
                        Code::Value,
                        format!(
                            "signal `{}` has {} samples, but {}",
                            signal.name,
                            signal.data.len(),
                            shape
                        ),
                    )
                    .with_context("analysis", self.analysis.clone())
                    .with_note("results are never truncated or padded silently"),
                );
            }
        }

        // Duplicate names make every lookup ambiguous, and lookups are
        // case-insensitive, so compare on the normalized form.
        let mut seen: HashMap<String, &str> = HashMap::with_capacity(self.signals.len());
        for signal in &self.signals {
            let key = normalize_signal_name(&signal.name);
            if let Some(first) = seen.insert(key, &signal.name) {
                diags.push(
                    Diagnostic::error(
                        Code::Duplicate,
                        format!(
                            "signal `{}` is saved more than once (already present as `{}`)",
                            signal.name, first
                        ),
                    )
                    .with_context("analysis", self.analysis.clone())
                    .with_note("signal names are compared case-insensitively"),
                );
            }
        }

        let values = self.scalar_value_count();
        if values > limits.max_result_values {
            diags.push(
                Diagnostic::error(
                    Code::Limit,
                    format!(
                        "result for analysis `{}` holds {} scalar values, over the limit of {}",
                        self.analysis, values, limits.max_result_values
                    ),
                )
                .with_context("limit", "max_result_values")
                .with_note("no values were truncated; reduce the probes or the sweep resolution"),
            );
        }

        if diags.is_empty() { Ok(()) } else { Err(diags) }
    }

    /// Look up a signal by name, case-insensitively and ignoring whitespace.
    pub fn signal(&self, name: &str) -> Option<&Signal> {
        let key = normalize_signal_name(name);
        self.signals
            .iter()
            .find(|s| normalize_signal_name(&s.name) == key)
    }

    /// Names as written, in dataset order. Used in "available signals" notes.
    pub fn signal_names(&self) -> Vec<&str> {
        self.signals.iter().map(|s| s.name.as_str()).collect()
    }

    /// The number of samples in the result.
    ///
    /// For [`Axis::None`] this is the signals' shared length (1 after
    /// validation), which is what distinguishes an operating point with one
    /// scalar from a genuinely empty sweep.
    pub fn sample_count(&self) -> usize {
        if self.axis.is_none() {
            self.signals.iter().map(|s| s.data.len()).max().unwrap_or(0)
        } else {
            self.axis.len()
        }
    }

    /// Number of scalar values held, counting a complex sample as two.
    ///
    /// This is what [`Limits::max_result_values`] bounds.
    pub fn scalar_value_count(&self) -> u64 {
        self.signals
            .iter()
            .map(|s| {
                let n = s.data.len() as u64;
                match s.data {
                    Data::Real(_) => n,
                    Data::Complex(_) => n.saturating_mul(2),
                }
            })
            .fold(0u64, u64::saturating_add)
    }

    /// Whether any signal or axis value is not finite.
    pub fn has_non_finite(&self) -> bool {
        self.axis.samples().iter().any(|x| !x.is_finite())
            || self
                .signals
                .iter()
                .any(|s| !s.data.non_finite_indices().is_empty())
    }

    /// Record an extra diagnostic (a backend warning, say) on this result.
    pub fn push_diagnostic(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use circuit_core::units::{CURRENT, VOLTAGE};

    fn backend() -> BackendInfo {
        BackendInfo::new("test", "0.1")
    }

    fn op_dataset(values: Vec<f64>) -> Dataset {
        Dataset::new(
            "exp",
            "op1",
            "op",
            Axis::None,
            vec![Signal::real("v(out)", VOLTAGE, values)],
            backend(),
            &Limits::default(),
        )
        .expect("op dataset should validate")
    }

    #[test]
    fn complex_magnitude_and_phase() {
        let z = Complex::new(3.0, 4.0);
        assert!((z.magnitude() - 5.0).abs() < 1e-12);
        assert!((z.phase_rad() - 0.927_295_218_001_612_2).abs() < 1e-12);
        assert!((z.phase_deg() - 53.130_102_354_155_98).abs() < 1e-9);
        assert!((z.norm_sqr() - 25.0).abs() < 1e-12);

        // The negative real axis has phase pi, and the origin is defined as 0.
        assert!((Complex::new(-1.0, 0.0).phase_rad() - std::f64::consts::PI).abs() < 1e-12);
        assert_eq!(Complex::ZERO.phase_rad(), 0.0);
        assert!((Complex::new(0.0, -1.0).phase_rad() + std::f64::consts::FRAC_PI_2).abs() < 1e-12);

        let polar = Complex::from_polar(2.0, std::f64::consts::FRAC_PI_2);
        assert!(polar.re.abs() < 1e-12);
        assert!((polar.im - 2.0).abs() < 1e-12);
    }

    #[test]
    fn complex_arithmetic_is_complex() {
        let a = Complex::new(1.0, 2.0);
        let b = Complex::new(3.0, -1.0);

        assert_eq!(a + b, Complex::new(4.0, 1.0));
        assert_eq!(a - b, Complex::new(-2.0, 3.0));
        assert_eq!(a * b, Complex::new(5.0, 5.0));
        assert_eq!(a.conjugate(), Complex::new(1.0, -2.0));
        assert_eq!(-a, Complex::new(-1.0, -2.0));

        // (1+2i)/(3-i) = (1+2i)(3+i)/10 = (1 + 7i)/10
        let q = a / b;
        assert!((q.re - 0.1).abs() < 1e-12, "{q}");
        assert!((q.im - 0.7).abs() < 1e-12, "{q}");

        // (1+i)/(1-i) = i, a unit vector: used again for the dB tests.
        let r = Complex::new(1.0, 1.0) / Complex::new(1.0, -1.0);
        assert!(r.re.abs() < 1e-12);
        assert!((r.im - 1.0).abs() < 1e-12);
    }

    #[test]
    fn data_reports_shape_and_finiteness() {
        let real = Data::Real(vec![1.0, f64::NAN, 3.0]);
        assert_eq!(real.len(), 3);
        assert!(!real.is_complex());
        assert_eq!(real.as_real().map(<[f64]>::len), Some(3));
        assert!(real.as_complex().is_none());
        assert_eq!(real.non_finite_indices(), vec![1]);
        // NaN never compares equal, so check the samples individually.
        let magnitudes = real.magnitudes();
        assert_eq!(magnitudes.len(), 3);
        assert_eq!(magnitudes[0], 1.0);
        assert!(magnitudes[1].is_nan());
        assert_eq!(magnitudes[2], 3.0);

        let complex = Data::Complex(vec![
            Complex::new(3.0, 4.0),
            Complex::new(f64::INFINITY, 0.0),
        ]);
        assert_eq!(complex.magnitudes(), vec![5.0, f64::INFINITY]);
        assert_eq!(complex.non_finite_indices(), vec![1]);
        assert_eq!(complex.sample(0), Some(Complex::new(3.0, 4.0)));
        assert_eq!(complex.sample(9), None);

        // A real sample promotes to a complex one with a zero imaginary part.
        assert_eq!(
            Data::Real(vec![2.0]).sample(0),
            Some(Complex::new(2.0, 0.0))
        );
    }

    #[test]
    fn axis_reports_kind_and_unit() {
        assert_eq!(Axis::None.kind_name(), "none");
        assert_eq!(Axis::None.len(), 0);
        assert!(Axis::None.unit().is_none());
        assert_eq!(Axis::None.describe(), "no axis (operating point)");

        let time = Axis::Time(vec![0.0, 1.0, 3.0]);
        assert_eq!(time.kind_name(), "time");
        assert_eq!(time.len(), 3);
        assert!(time.is_time());
        assert_eq!(time.unit(), Some(TIME));
        assert_eq!(time.first(), Some(0.0));
        assert_eq!(time.last(), Some(3.0));

        let freq = Axis::Frequency(vec![10.0, 100.0]);
        assert_eq!(freq.unit(), Some(FREQUENCY));
        assert_eq!(freq.describe(), "a frequency axis");

        assert!(Axis::Parameter(vec![1.0]).unit().is_none());
    }

    #[test]
    fn signal_lookup_ignores_case_and_whitespace() {
        let ds = op_dataset(vec![1.0]);
        assert!(ds.signal("v(out)").is_some());
        assert!(ds.signal("V(OUT)").is_some());
        assert!(ds.signal("v ( out )").is_some());
        assert!(ds.signal("v(in)").is_none());
        assert_eq!(ds.signal_names(), vec!["v(out)"]);
        assert_eq!(normalize_signal_name(" V ( A , B ) "), "v(a,b)");
    }

    #[test]
    fn operating_point_requires_exactly_one_sample() {
        let err = Dataset::new(
            "exp",
            "op1",
            "op",
            Axis::None,
            vec![Signal::real("v(out)", VOLTAGE, vec![1.0, 2.0])],
            backend(),
            &Limits::default(),
        )
        .expect_err("two samples without an axis is an error");
        assert_eq!(err.len(), 1);
        let d = err.iter().next().expect("one diagnostic");
        assert_eq!(d.code, Code::Value);
        assert!(d.message.contains("2 samples"), "{}", d.message);
        assert!(d.message.contains("exactly 1"), "{}", d.message);
    }

    #[test]
    fn signal_length_must_match_the_axis() {
        let err = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1e-3, 2e-3]),
            vec![Signal::real("v(out)", VOLTAGE, vec![0.0, 1.0])],
            backend(),
            &Limits::default(),
        )
        .expect_err("length mismatch is an error");
        let d = err.iter().next().expect("one diagnostic");
        assert_eq!(d.code, Code::Value);
        assert!(d.message.contains("v(out)"), "{}", d.message);
        assert!(
            d.message.contains("time axis has 3 samples"),
            "{}",
            d.message
        );
    }

    #[test]
    fn duplicate_signal_names_are_rejected_case_insensitively() {
        let err = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1.0]),
            vec![
                Signal::real("v(out)", VOLTAGE, vec![0.0, 1.0]),
                Signal::real("V(OUT)", VOLTAGE, vec![1.0, 2.0]),
            ],
            backend(),
            &Limits::default(),
        )
        .expect_err("duplicate names are an error");
        assert!(
            err.iter().any(|d| d.code == Code::Duplicate),
            "{}",
            err.render_plain()
        );
    }

    #[test]
    fn oversized_results_are_rejected_without_truncation() {
        // Limits::for_tests allows 10_000 scalar values.
        let limits = Limits::for_tests();
        let samples = 200;
        let signals: Vec<Signal> = (0..60)
            .map(|i| Signal::real(format!("v(n{i})"), VOLTAGE, vec![0.0; samples]))
            .collect();
        let err = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0; samples]),
            signals,
            backend(),
            &limits,
        )
        .expect_err("over the limit");
        let d = err
            .iter()
            .find(|d| d.code == Code::Limit)
            .expect("limit diagnostic");
        assert!(d.message.contains("12000"), "{}", d.message);
        assert!(d.message.contains("10000"), "{}", d.message);
        assert!(
            d.notes.iter().any(|n| n.contains("truncated")),
            "the note must say nothing was truncated: {d:?}"
        );
    }

    #[test]
    fn complex_samples_count_twice_towards_the_limit() {
        let signals = vec![Signal::complex("v(out)", VOLTAGE, vec![Complex::ONE; 10])];
        let ds = Dataset::new(
            "exp",
            "ac1",
            "ac",
            Axis::Frequency(vec![1.0; 10]),
            signals,
            backend(),
            &Limits::for_tests(),
        )
        .expect("20 scalar values is well under 10_000");
        assert_eq!(ds.scalar_value_count(), 20);
        assert_eq!(ds.sample_count(), 10);
    }

    #[test]
    fn sample_count_distinguishes_op_from_empty_sweep() {
        let op = op_dataset(vec![3.0]);
        assert_eq!(op.sample_count(), 1);
        assert!(op.axis.is_none());

        let empty = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(Vec::new()),
            Vec::new(),
            backend(),
            &Limits::default(),
        )
        .expect("an empty sweep is shaped correctly");
        assert_eq!(empty.sample_count(), 0);
        assert!(!empty.axis.is_none());
    }

    #[test]
    fn validation_accepts_a_well_formed_dataset() {
        let ds = Dataset::new(
            "response",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1e-3, 2e-3]),
            vec![
                Signal::real("v(out)", VOLTAGE, vec![0.0, 0.5, 1.0]),
                Signal::complex("i(r1)", CURRENT, vec![Complex::ONE; 3]),
            ],
            BackendInfo::new("thevenin", "0.5.0").with_setting("max_step", "50ns"),
            &Limits::default(),
        )
        .expect("valid dataset");
        assert_eq!(ds.scalar_value_count(), 9);
        assert!(!ds.has_non_finite());
        assert_eq!(ds.backend.settings.len(), 1);
    }

    #[test]
    fn non_finite_detection_covers_axis_and_signals() {
        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, f64::NAN]),
            vec![Signal::real("v(out)", VOLTAGE, vec![0.0, 1.0])],
            backend(),
            &Limits::default(),
        )
        .expect("valid");
        assert!(ds.has_non_finite());

        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1.0]),
            vec![Signal::real("v(out)", VOLTAGE, vec![0.0, f64::INFINITY])],
            backend(),
            &Limits::default(),
        )
        .expect("valid");
        assert!(ds.has_non_finite());
    }

    #[test]
    fn diagnostics_can_be_attached_to_a_result() {
        let mut ds = op_dataset(vec![1.0]);
        ds.push_diagnostic(Diagnostic::warning(Code::Value, "backend warning"));
        assert_eq!(ds.diagnostics.len(), 1);
    }
}
