//! Measurements: `max`, `min`, `avg` and `rms` over a result signal.
//!
//! The definitions are fixed by the language specification (§7) and are
//! **not** all sample statistics:
//!
//! | measurement | definition |
//! |---|---|
//! | `max` / `min` | sample extreme; needs no axis |
//! | `avg` | `∫x dt / ∫dt` over the analysis axis |
//! | `rms` | `sqrt(∫x² dt / ∫dt)` over the analysis axis |
//!
//! The time axis a transient solver returns is generally **non-uniform**
//! (spec §5.1), so `avg` and `rms` are trapezoidal integrals, not arithmetic
//! means over the samples. `avg` and `rms` therefore require a time axis: on
//! an operating point there is nothing to integrate, and running `rms` on a
//! DC sweep would silently compute the wrong number. Both cases are reported
//! as [`Code::Type`].
//!
//! # Complex signals
//!
//! `avg` and `rms` are defined on the **magnitude** of a complex signal
//! (`|z|` and `|z|²`): an integral needs no ordering. `max` and `min` do need
//! one, and a complex number has none, so reducing a complex value to an
//! extreme is reported as [`Code::Type`] with the advice to write `abs(...)`
//! first — the reduction never silently ranks magnitudes. For real signals the
//! sample value itself is the magnitude (`magnitudes()` does not take an
//! absolute value of real data), so `max` of a negative voltage is that
//! negative voltage — use `abs(v(...))` in the measurement expression when a
//! magnitude is wanted.

use circuit_core::units::Dimension;
use circuit_core::{Code, Diagnostic};

use crate::dataset::{Axis, Dataset};
use crate::expr::{self, EvalSite, Expr, Value};
use crate::format_number;

/// The four measurements the language defines.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Measurement {
    Max,
    Min,
    Avg,
    Rms,
}

impl Measurement {
    /// The keyword as written in the DSL.
    pub fn name(self) -> &'static str {
        match self {
            Self::Max => "max",
            Self::Min => "min",
            Self::Avg => "avg",
            Self::Rms => "rms",
        }
    }

    /// Parse a measurement keyword, case-insensitively.
    pub fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "max" => Some(Self::Max),
            "min" => Some(Self::Min),
            "avg" => Some(Self::Avg),
            "rms" => Some(Self::Rms),
            _ => None,
        }
    }

    /// Whether this measurement is a time integral and therefore needs a time
    /// axis.
    pub fn needs_time_axis(self) -> bool {
        matches!(self, Self::Avg | Self::Rms)
    }
}

/// A completed measurement.
#[derive(Clone, PartialEq, Debug)]
pub struct Measured {
    /// The measurement's name, e.g. `vout_rms`.
    pub name: String,
    pub value: f64,
    /// The dimension of `value`: the signal's dimension for `max`/`min`, and
    /// the signal's dimension for `avg`/`rms` as well (the time integral
    /// cancels the time dimension of `dt`).
    pub unit: Dimension,
    /// The analysis identity the value was taken from (`ac1`), empty when it
    /// is not known (a hand-built value, or a reduction with no dataset).
    pub analysis: String,
}

impl Measured {
    /// A measurement with no analysis identity yet: [`reduce`] fills it from
    /// the dataset it reduced, and [`Measured::with_analysis`] sets it by hand.
    pub fn new(name: impl Into<String>, value: f64, unit: Dimension) -> Self {
        Self {
            name: name.into(),
            value,
            unit,
            analysis: String::new(),
        }
    }

    /// Record which analysis the value was taken from.
    pub fn with_analysis(mut self, analysis: impl Into<String>) -> Self {
        self.analysis = analysis.into();
        self
    }

    /// `name = value unit`, e.g. `vrms = 1.2345 V`.
    ///
    /// Deliberately unchanged by the analysis identity: this text is what the
    /// existing output and tests match. Use [`Measured::render_with_analysis`]
    /// when the identity matters.
    pub fn render(&self) -> String {
        format!(
            "{} = {} {}",
            self.name,
            format_number(self.value),
            self.unit
        )
    }

    /// `name = value unit (ac1)`, or exactly [`Measured::render`] when the
    /// analysis is not known.
    pub fn render_with_analysis(&self) -> String {
        if self.analysis.is_empty() {
            self.render()
        } else {
            format!("{} ({})", self.render(), self.analysis)
        }
    }
}

/// Evaluate `expr` against `dataset` and reduce it with `kind`.
///
/// The measurement's own name is the evaluation site: a diagnostic raised
/// inside the expression says which `measure` asked for the value.
pub fn measure(
    kind: Measurement,
    name: &str,
    expr: &Expr,
    dataset: &Dataset,
) -> Result<Measured, Diagnostic> {
    let value = expr::eval_at(expr, dataset, &EvalSite::measure(name))?;
    reduce(kind, name, &value, &dataset.axis, &dataset.analysis)
}

/// Look up a signal by name and reduce it. Shorthand for
/// `measure(kind, name, &Expr::signal(signal), dataset)`.
pub fn measure_signal(
    kind: Measurement,
    name: &str,
    signal: &str,
    dataset: &Dataset,
) -> Result<Measured, Diagnostic> {
    measure(kind, name, &Expr::signal(signal), dataset)
}

/// Reduce an already-evaluated value.
///
/// `analysis` is the analysis id the value came from. It is what the "no time
/// axis" diagnostic points at, and it is stamped on the result so display and
/// export can say which analysis a measure belongs to.
pub fn reduce(
    kind: Measurement,
    name: &str,
    value: &Value,
    axis: &Axis,
    analysis: &str,
) -> Result<Measured, Diagnostic> {
    let measured = match kind {
        Measurement::Max | Measurement::Min => extremes(kind, name, value)?,
        Measurement::Avg | Measurement::Rms => integral(kind, name, value, axis, analysis)?,
    };
    Ok(measured.with_analysis(analysis))
}

/// Sample extremes. No axis is needed: this is a property of the samples, not
/// of where they were taken.
///
/// The samples must be real: a complex value has no ordering, so it is a
/// [`Code::Type`] error (see the module docs) rather than an implicit ranking
/// of magnitudes.
///
/// A `NaN` anywhere makes the result `NaN` rather than being skipped — the
/// exporter turns it into `null` and warns, so a non-finite result stays
/// visible instead of silently becoming a plausible-looking number.
fn extremes(kind: Measurement, name: &str, value: &Value) -> Result<Measured, Diagnostic> {
    if value.is_complex() {
        return Err(Diagnostic::error(
            Code::Type,
            format!(
                "cannot take the {} of `{name}`: the value is complex, and a complex number has no ordering",
                kind.name()
            ),
        )
        .with_note("apply abs(...) first to reduce the magnitude of a complex signal"));
    }
    let samples = value.magnitudes();
    if samples.is_empty() {
        return Err(Diagnostic::error(
            Code::Value,
            format!(
                "cannot take the {} of `{name}`: it has no samples",
                kind.name()
            ),
        ));
    }
    if samples.iter().any(|x| x.is_nan()) {
        return Ok(Measured::new(name, f64::NAN, value.unit));
    }
    // NaN is already ruled out, so the identity elements are safe and the
    // extremes do not depend on which sample happens to come first.
    let extreme = match kind {
        Measurement::Max => samples.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        Measurement::Min => samples.iter().copied().fold(f64::INFINITY, f64::min),
        Measurement::Avg | Measurement::Rms => {
            // `reduce` routes these to `integral`; report rather than return a
            // sample that is not the statistic that was asked for.
            return Err(Diagnostic::error(
                Code::Type,
                format!("`{}` is not a sample extreme", kind.name()),
            ));
        }
    };
    Ok(Measured::new(name, extreme, value.unit))
}

/// Time integral over the axis, by the trapezoidal rule.
fn integral(
    kind: Measurement,
    name: &str,
    value: &Value,
    axis: &Axis,
    analysis: &str,
) -> Result<Measured, Diagnostic> {
    let Axis::Time(times) = axis else {
        return Err(Diagnostic::error(
            Code::Type,
            format!(
                "`{}` requires a time axis, but analysis `{analysis}` has {}",
                kind.name(),
                axis.describe()
            ),
        )
        .with_note("avg and rms are defined as time integrals (docs/language.md §7)")
        .with_note("use max/min for analyses without a time axis"));
    };

    let samples = value.magnitudes();
    if times.len() != samples.len() {
        return Err(Diagnostic::error(
            Code::Value,
            format!(
                "cannot integrate `{name}`: the time axis has {} samples but the signal has {}",
                times.len(),
                samples.len()
            ),
        ));
    }
    if times.len() < 2 {
        return Err(Diagnostic::error(
            Code::Value,
            format!(
                "cannot integrate `{name}`: the time axis of `{analysis}` needs at least two points, found {}",
                times.len()
            ),
        ));
    }

    let mut area = 0.0; // ∫ x dt
    let mut area_sq = 0.0; // ∫ x² dt
    let mut width = 0.0; // ∫ dt
    for i in 0..times.len() - 1 {
        let (t0, t1) = (times[i], times[i + 1]);
        let (x0, x1) = (samples[i], samples[i + 1]);
        let dt = t1 - t0;
        width += dt;
        area += 0.5 * (x0 + x1) * dt;
        area_sq += 0.5 * (x0 * x0 + x1 * x1) * dt;
    }

    // A time axis that does not advance (all points equal, or decreasing)
    // would divide by zero or flip the sign of the result.
    if !width.is_finite() || width <= 0.0 {
        return Err(Diagnostic::error(
            Code::Value,
            format!(
                "cannot integrate `{name}`: the time axis of `{analysis}` does not advance (total width {})",
                format_number(width)
            ),
        )
        .with_note("avg and rms divide by the total time interval"));
    }

    // `∫x dt / ∫dt` cancels the time dimension of `dt`, so `avg` and `rms`
    // keep the signal's own unit.
    let reduced = match kind {
        Measurement::Avg => area / width,
        Measurement::Rms => (area_sq / width).sqrt(),
        Measurement::Max | Measurement::Min => unreachable!("handled by `extremes`"),
    };
    Ok(Measured::new(name, reduced, value.unit))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{BackendInfo, Complex, Signal};
    use circuit_core::Limits;
    use circuit_core::units::{CURRENT, VOLTAGE};

    /// A transient dataset with a deliberately **non-uniform** time axis.
    fn non_uniform() -> Dataset {
        Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1.0, 2.0, 4.0]),
            vec![Signal::real("v(out)", VOLTAGE, vec![0.0, 1.0, 2.0, 4.0])],
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("well formed")
    }

    fn assert_close(got: f64, want: f64, tol: f64) {
        assert!(
            (got - want).abs() <= tol,
            "expected {want} (+-{tol}), got {got}"
        );
    }

    fn sample_mean(values: &[f64]) -> f64 {
        values.iter().sum::<f64>() / values.len() as f64
    }

    /// The required test: on a non-uniform axis the integral-based `avg` must
    /// be right *and* must differ from the arithmetic sample mean.
    #[test]
    fn avg_is_a_time_integral_not_a_sample_mean() {
        // x = t on t = {0, 1, 2, 4}. The trapezoid rule is exact for a linear
        // signal: ∫t dt over [0,4] = 8, width 4, so avg = 2.
        let ds = non_uniform();
        let measured =
            measure_signal(Measurement::Avg, "vavg", "v(out)", &ds).expect("tran has a time axis");
        assert_close(measured.value, 2.0, 1e-12);
        assert_eq!(measured.unit, VOLTAGE);
        assert_eq!(measured.render(), "vavg = 2 V");

        // The sample arithmetic mean is 7/4, which is a different number: a
        // uniform-axis-only implementation would have passed the wrong test.
        let arithmetic = sample_mean(&[0.0, 1.0, 2.0, 4.0]);
        assert_close(arithmetic, 1.75, 1e-12);
        assert!(
            (measured.value - arithmetic).abs() > 0.2,
            "the integral-based average ({}) must differ from the sample mean ({arithmetic})",
            measured.value
        );
    }

    /// The same grid for `rms`, including the sample-based value it must not
    /// return.
    #[test]
    fn rms_is_a_time_integral_not_a_sample_rms() {
        let ds = non_uniform();
        let measured =
            measure_signal(Measurement::Rms, "vrms", "v(out)", &ds).expect("tran has a time axis");
        // Trapezoid: (0,1): 0.5*(0+1)*1 = 0.5; (1,2): 0.5*(1+4)*1 = 2.5;
        // (2,4): 0.5*(4+16)*2 = 20 -> ∫x²dt = 23, width 4 -> rms = sqrt(5.75).
        assert_close(measured.value, 5.75f64.sqrt(), 1e-12);
        assert_close(measured.value, 2.397_915_761_656_359_6, 1e-12);

        let sample_rms = (sample_mean(&[0.0, 1.0, 4.0, 16.0])).sqrt();
        assert_close(sample_rms, 5.25f64.sqrt(), 1e-12);
        assert!(
            (measured.value - sample_rms).abs() > 0.05,
            "integral rms {} must differ from the sample rms {sample_rms}",
            measured.value
        );
    }

    /// A constant signal is the case where an integral and a sample mean
    /// agree, which pins down the normalization.
    #[test]
    fn avg_and_rms_of_a_constant_are_that_constant() {
        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1e-3, 5e-3, 2e-2]),
            vec![Signal::real("v(out)", VOLTAGE, vec![3.0; 4])],
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("well formed");
        let avg = measure_signal(Measurement::Avg, "a", "v(out)", &ds).expect("avg");
        let rms = measure_signal(Measurement::Rms, "r", "v(out)", &ds).expect("rms");
        assert_close(avg.value, 3.0, 1e-12);
        assert_close(rms.value, 3.0, 1e-12);
    }

    /// A ramp whose integral is known analytically over two unequal segments.
    #[test]
    fn trapezoid_handles_two_unequal_segments() {
        // x = 2t on t = {0, 1, 3}: exact ∫2t dt = t² = 9; width 3; avg = 3.
        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1.0, 3.0]),
            vec![Signal::real("v(out)", VOLTAGE, vec![0.0, 2.0, 6.0])],
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("well formed");
        let avg = measure_signal(Measurement::Avg, "a", "v(out)", &ds).expect("avg");
        assert_close(avg.value, 3.0, 1e-12);
    }

    #[test]
    fn max_and_min_are_sample_extremes() {
        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1.0, 2.0]),
            vec![Signal::real("v(out)", VOLTAGE, vec![-2.0, 5.0, 1.0])],
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("well formed");
        let hi = measure_signal(Measurement::Max, "vmax", "v(out)", &ds).expect("max");
        assert_close(hi.value, 5.0, 1e-12);
        assert_eq!(hi.unit, VOLTAGE);
        let lo = measure_signal(Measurement::Min, "vmin", "v(out)", &ds).expect("min");
        assert_close(lo.value, -2.0, 1e-12);
    }

    /// Round 3: a complex value is no longer reduced through its magnitude.
    /// |3+4i| = 5 etc. are not silently ranked: the user is told to write
    /// abs(...), which then works.
    #[test]
    fn complex_max_and_min_are_rejected_with_an_abs_hint() {
        let ds = Dataset::new(
            "exp",
            "ac1",
            "ac",
            Axis::Frequency(vec![1.0, 10.0, 100.0]),
            vec![Signal::complex(
                "v(out)",
                VOLTAGE,
                vec![
                    Complex::new(3.0, 4.0),
                    Complex::new(0.0, 2.0),
                    Complex::new(1.0, 0.0),
                ],
            )],
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("well formed");

        let err = measure_signal(Measurement::Max, "vmax", "v(out)", &ds)
            .expect_err("a complex value has no maximum");
        assert_eq!(err.code, Code::Type);
        assert!(err.message.contains("complex"), "{}", err.message);
        assert!(
            err.notes.iter().any(|n| n.contains("abs")),
            "the note should point at abs(): {:?}",
            err.notes
        );
        let err = measure_signal(Measurement::Min, "vmin", "v(out)", &ds)
            .expect_err("a complex value has no minimum");
        assert_eq!(err.code, Code::Type);

        // The advice works: abs(...) makes the value real, and then the
        // extremes are defined again.
        let hi = measure(Measurement::Max, "vmax", &Expr::voltage("out").abs(), &ds)
            .expect("abs() makes the value real");
        assert_close(hi.value, 5.0, 1e-12);
        let lo = measure(Measurement::Min, "vmin", &Expr::voltage("out").abs(), &ds)
            .expect("abs() makes the value real");
        assert_close(lo.value, 1.0, 1e-12);

        // avg/rms over a complex signal are still defined on the magnitude;
        // the frequency axis is what makes these a type error.
        let err = measure_signal(Measurement::Rms, "vrms", "v(out)", &ds)
            .expect_err("frequency is not time");
        assert_eq!(err.code, Code::Type);
    }

    #[test]
    fn complex_avg_still_integrates_the_magnitude() {
        // |3| = 3 and |0+4i| = 4 over t = {0, 1}: the trapezoid average of the
        // magnitude is 3.5. avg/rms keep their documented magnitude meaning.
        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1.0]),
            vec![Signal::complex(
                "v(out)",
                VOLTAGE,
                vec![Complex::new(3.0, 0.0), Complex::new(0.0, 4.0)],
            )],
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("well formed");
        let avg = measure_signal(Measurement::Avg, "vavg", "v(out)", &ds).expect("avg");
        assert_close(avg.value, 3.5, 1e-12);
        assert_eq!(avg.unit, VOLTAGE);
    }

    /// Round 3: every measurement carries the analysis it was taken from.
    #[test]
    fn measured_records_the_analysis_identity() {
        let ds = non_uniform();
        let avg =
            measure_signal(Measurement::Avg, "vavg", "v(out)", &ds).expect("tran has a time axis");
        assert_eq!(avg.analysis, "tran1");
        assert_eq!(
            avg.render(),
            "vavg = 2 V",
            "render() is unchanged by the identity"
        );
        assert_eq!(avg.render_with_analysis(), "vavg = 2 V (tran1)");

        // max/min take the identity from the dataset too.
        let hi = measure_signal(Measurement::Max, "vmax", "v(out)", &ds).expect("max");
        assert_eq!(hi.analysis, "tran1");

        // reduce() fills it from its own parameter.
        let value = Value::real(VOLTAGE, vec![1.0, 5.0]);
        let reduced = reduce(Measurement::Max, "vmax", &value, &Axis::None, "dc1").expect("max");
        assert_eq!(reduced.analysis, "dc1");
        assert_eq!(reduced.render(), "vmax = 5 V");
        assert_eq!(reduced.render_with_analysis(), "vmax = 5 V (dc1)");

        // An unknown identity renders exactly like render().
        let plain = Measured::new("x", 1.0, VOLTAGE);
        assert_eq!(plain.analysis, "");
        assert_eq!(plain.render_with_analysis(), "x = 1 V");
        let named = Measured::new("x", 1.0, VOLTAGE).with_analysis("ac1");
        assert_eq!(named.render_with_analysis(), "x = 1 V (ac1)");
    }

    #[test]
    fn complex_rms_uses_the_magnitude_over_a_time_axis() {
        // Magnitudes 3, 4 over t = {0, 1}: ∫x²dt/∫dt = (9+16)/2 = 12.5,
        // rms = sqrt(12.5) ~ 3.5355.
        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1.0]),
            vec![Signal::complex(
                "v(out)",
                VOLTAGE,
                vec![Complex::new(3.0, 0.0), Complex::new(4.0, 0.0)],
            )],
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("well formed");
        let rms = measure_signal(Measurement::Rms, "vrms", "v(out)", &ds).expect("rms");
        assert_close(rms.value, 12.5f64.sqrt(), 1e-12);
    }

    #[test]
    fn avg_on_an_operating_point_is_a_type_error() {
        let ds = Dataset::new(
            "exp",
            "op1",
            "op",
            Axis::None,
            vec![Signal::real("v(out)", VOLTAGE, vec![1.0])],
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("well formed");
        let err = measure_signal(Measurement::Avg, "vavg", "v(out)", &ds)
            .expect_err("there is no time to integrate over");
        assert_eq!(err.code, Code::Type);
        assert!(err.message.contains("time axis"), "{}", err.message);
        assert!(err.message.contains("op1"), "{}", err.message);
        assert!(err.message.contains("operating point"), "{}", err.message);

        // max/min still work on an operating point.
        let hi = measure_signal(Measurement::Max, "vmax", "v(out)", &ds).expect("max");
        assert_close(hi.value, 1.0, 1e-12);
    }

    #[test]
    fn rms_on_a_dc_sweep_is_a_type_error() {
        let ds = Dataset::new(
            "exp",
            "dc1",
            "dc",
            Axis::Parameter(vec![0.0, 1.0, 2.0]),
            vec![Signal::real("v(out)", VOLTAGE, vec![0.0, 1.0, 2.0])],
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("well formed");
        let err = measure_signal(Measurement::Rms, "vrms", "v(out)", &ds)
            .expect_err("a parameter sweep is not time");
        assert_eq!(err.code, Code::Type);
        assert!(err.message.contains("parameter"), "{}", err.message);
    }

    #[test]
    fn degenerate_time_axis_does_not_divide_by_zero() {
        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![1e-3, 1e-3, 1e-3]),
            vec![Signal::real("v(out)", VOLTAGE, vec![1.0, 2.0, 3.0])],
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("well formed");
        let err = measure_signal(Measurement::Avg, "vavg", "v(out)", &ds).expect_err("zero width");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("does not advance"), "{}", err.message);
        assert!(!err.message.contains("inf"), "{}", err.message);
    }

    #[test]
    fn a_single_time_point_cannot_be_integrated() {
        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![1e-3]),
            vec![Signal::real("v(out)", VOLTAGE, vec![1.0])],
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("well formed");
        let err = measure_signal(Measurement::Avg, "vavg", "v(out)", &ds)
            .expect_err("one point has no width");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("at least two"), "{}", err.message);
    }

    #[test]
    fn measure_evaluates_an_expression_then_reduces_it() {
        let ds = non_uniform();
        let err = measure(
            Measurement::Avg,
            "vab_avg",
            &Expr::differential("out", "not_saved"),
            &ds,
        )
        .expect_err("v(not_saved) is not in the dataset");
        assert_eq!(err.code, Code::Name);
    }

    #[test]
    fn measure_can_reduce_an_expression_over_two_signals() {
        // v(a) - v(b) = 2 - 0 = 2 V, integrated over a flat axis -> 2.
        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1.0, 2.0]),
            vec![
                Signal::real("v(a)", VOLTAGE, vec![2.0, 2.0, 2.0]),
                Signal::real("v(b)", VOLTAGE, vec![0.0, 0.0, 0.0]),
            ],
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("well formed");
        let measured = measure(Measurement::Avg, "vrms", &Expr::differential("a", "b"), &ds)
            .expect("both signals exist");
        assert_close(measured.value, 2.0, 1e-12);
        assert_eq!(measured.unit, VOLTAGE);
    }

    #[test]
    fn measurement_keywords_round_trip() {
        for m in [
            Measurement::Max,
            Measurement::Min,
            Measurement::Avg,
            Measurement::Rms,
        ] {
            assert_eq!(Measurement::parse(m.name()), Some(m));
        }
        assert_eq!(Measurement::parse("RMS"), Some(Measurement::Rms));
        assert_eq!(Measurement::parse("median"), None);
        assert!(Measurement::Avg.needs_time_axis());
        assert!(Measurement::Rms.needs_time_axis());
        assert!(!Measurement::Max.needs_time_axis());
        assert!(!Measurement::Min.needs_time_axis());
    }

    #[test]
    fn reduce_reports_an_empty_signal() {
        let value = Value::real(CURRENT, Vec::new());
        let err = reduce(Measurement::Max, "imax", &value, &Axis::None, "op1")
            .expect_err("nothing to reduce");
        assert_eq!(err.code, Code::Value);
        assert!(err.message.contains("no samples"), "{}", err.message);
    }

    /// Round 4 (R4-01): a signal sample that is already NaN is refused by the
    /// read that sees it.
    ///
    /// Round 3 let the NaN through and reduced it to a NaN measurement; the
    /// round-4 value policy (contract §1.3.3) is stricter on purpose, so an
    /// illegal sample can never be carried into a measurement or a column. The
    /// sample is still not skipped: it is named, with its coordinate, and the
    /// diagnostic says which `measure` asked for the value.
    #[test]
    fn nan_samples_are_refused_when_read_rather_than_becoming_a_nan_measurement() {
        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1.0, 2.0]),
            vec![Signal::real("v(out)", VOLTAGE, vec![1.0, f64::NAN, 3.0])],
            BackendInfo::new("test", "0"),
            &Limits::default(),
        )
        .expect("well formed");
        let err = measure_signal(Measurement::Max, "vmax", "v(out)", &ds)
            .expect_err("a NaN sample has no maximum");
        assert_eq!(err.code, Code::Value, "{}", err.render_plain());
        let text = err.render_plain();
        assert!(text.contains("non-finite"), "{text}");
        assert!(text.contains("v(out)"), "{text}");
        assert!(text.contains("time = 1"), "{text}");
        assert!(
            text.contains("measure `vmax`"),
            "the site must be named: {text}"
        );
    }
}
