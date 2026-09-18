//! Independent output resampling for transient results (task A).
//!
//! # Why this exists
//!
//! The engine has no output-sampling option: it records **every accepted
//! internal step** (`thevenin-0.5.0/src/transient.rs:2271-2285`), and its
//! `.tran` "step" argument is a solver parameter that also acts as the lower
//! bound of the PULSE rise/fall times
//! (`thevenin-0.5.0/src/waveform.rs:37-38`). Passing a user's
//! `output_interval` there — as an earlier revision did — silently rewrote the
//! stimulus, so a coarse output request changed the physics.
//!
//! `output_interval` is therefore implemented *here*, after the run: the
//! solver keeps its own steps and the delivered trace is resampled onto the
//! requested uniform grid. The raw trace is what measurements are computed
//! from; this module only produces the view a user asked for.
//!
//! # The grid contract (docs/language.md §5.3)
//!
//! Given a raw time axis `t0 < t1 < … < tN` and an interval `iv > 0`, the
//! output grid is
//!
//! ```text
//! t0, t0 + iv, t0 + 2*iv, … (strictly less than tN), then tN
//! ```
//!
//! * the **first** point is the raw first point and is copied exactly;
//! * interior points are `t0 + k*iv`, `k >= 1`, while they stay below the raw
//!   last point;
//! * the **last** raw point is always the last output point, so the output
//!   covers the whole simulated window even when the interval does not divide
//!   it;
//! * interior values are linear interpolations between the two bracketing raw
//!   samples. Nothing is extrapolated: every output point lies inside
//!   `[t0, tN]`.
//!
//! # Size
//!
//! The output is bounded by the same [`Limits::max_result_values`] rule as any
//! other dataset: an interval small enough to produce more values is rejected
//! with `E_LIMIT`, never truncated.

use circuit_core::Limits;
use circuit_core::diagnostic::{Code, Diagnostic, Diagnostics};

use crate::dataset::{Axis, BackendInfo, Data, Dataset, Signal};

/// A uniform output grid derived from a solved time axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputGrid {
    /// First output time (the solver's first time point).
    pub first_s: f64,
    /// Requested spacing between interior points.
    pub interval_s: f64,
    /// Last output time (the solver's last time point).
    pub last_s: f64,
}

impl OutputGrid {
    /// Derive the grid from a raw axis, or `None` when there is nothing to
    /// interpolate (fewer than two points, or a degenerate interval).
    pub fn from_axis(axis: &Axis, interval_s: f64) -> Option<Self> {
        let Axis::Time(times) = axis else {
            return None;
        };
        let (first, last) = (times.first().copied()?, times.last().copied()?);
        if times.len() < 2 || !interval_s.is_finite() || interval_s <= 0.0 || last <= first {
            return None;
        }
        Some(Self {
            first_s: first,
            interval_s,
            last_s: last,
        })
    }

    /// The interior grid points `first + k*interval`, `k = 1 … n`, where `k`
    /// counts exactly the times strictly below the raw last point.
    ///
    /// One function serves both the count and the grid, so a size limit
    /// computed from [`OutputGrid::point_count`] can never disagree with the
    /// grid that is actually built. The estimate is a division; a small window
    /// around it absorbs the floating-point rounding that would otherwise make
    /// `k*interval` and `span/interval` disagree by one near the endpoint.
    fn interior_count(&self) -> u64 {
        // `is_finite` covers NaN and infinities, so the plain comparison is
        // enough for the rest.
        let span = self.last_s - self.first_s;
        if !span.is_finite() || span <= 0.0 || self.interval_s <= 0.0 {
            return 0;
        }
        let estimate = (span / self.interval_s).floor();
        if !estimate.is_finite() {
            return u64::MAX;
        }
        let base = estimate.max(0.0) as u64;
        let mut count = 0u64;
        for k in base.saturating_sub(2)..=base.saturating_add(2) {
            if k >= 1 && self.first_s + k as f64 * self.interval_s < self.last_s {
                count = count.max(k);
            }
        }
        count
    }

    /// How many points this grid produces, without building it.
    pub fn point_count(&self) -> u64 {
        let interior = self.interior_count();
        if interior == u64::MAX {
            return u64::MAX;
        }
        interior + 1 + u64::from(self.last_s > self.first_s)
    }

    /// The grid times, in ascending order.
    ///
    /// The first raw point, the interior points strictly below the last raw
    /// point, and the last raw point itself — which is why the final interval
    /// may be shorter than the requested one.
    pub fn points(&self) -> Vec<f64> {
        let interior = self.interior_count();
        let mut out = Vec::with_capacity(self.point_count() as usize);
        out.push(self.first_s);
        for k in 1..=interior {
            out.push(self.first_s + k as f64 * self.interval_s);
        }
        if self.last_s > self.first_s {
            out.push(self.last_s);
        }
        out
    }
}

/// Resample a transient dataset onto `interval_s`.
///
/// Returns the dataset unchanged (a clone) when the axis is not a time axis,
/// already has fewer than two points, or the interval is not usable — those
/// cases are reported by the caller's input validation, not silently here.
pub fn resample_time(
    dataset: &Dataset,
    interval_s: f64,
    limits: &Limits,
) -> Result<Dataset, Diagnostics> {
    if !interval_s.is_finite() || interval_s <= 0.0 {
        return Err(Diagnostics::single(
            Diagnostic::error(
                Code::Value,
                format!("output interval {interval_s} s is not a finite positive number"),
            )
            .with_note("this must be rejected at the input layer, not here"),
        ));
    }

    let Axis::Time(times) = &dataset.axis else {
        return Ok(dataset.clone());
    };
    let Some(grid) = OutputGrid::from_axis(&dataset.axis, interval_s) else {
        // A single-sample or degenerate axis has nothing to interpolate; the
        // raw grid is returned unchanged rather than padded or extended.
        return Ok(dataset.clone());
    };

    let values = grid
        .point_count()
        .saturating_mul(dataset.signals.len().max(1) as u64);
    if values > limits.max_result_values {
        return Err(Diagnostics::single(
            Diagnostic::error(
                Code::Limit,
                format!(
                    "an output interval of {interval_s} s over this trace needs {} points \
                     ({} values), over the limit of {}",
                    grid.point_count(),
                    values,
                    limits.max_result_values
                ),
            )
            .with_context("interval", format!("{interval_s} s"))
            .with_note(
                "no values were truncated: use a longer `output_interval:`, a shorter \
                 `stop:`, or raise the result limit",
            ),
        ));
    }

    let out_times = grid.points();
    let mut signals = Vec::with_capacity(dataset.signals.len());
    for signal in &dataset.signals {
        let data = match &signal.data {
            Data::Real(raw) => Data::Real(interpolate(raw, times, &out_times)),
            Data::Complex(raw) => {
                let re: Vec<f64> = raw.iter().map(|c| c.re).collect();
                let im: Vec<f64> = raw.iter().map(|c| c.im).collect();
                let re = interpolate(&re, times, &out_times);
                let im = interpolate(&im, times, &out_times);
                Data::Complex(
                    re.into_iter()
                        .zip(im)
                        .map(|(re, im)| crate::dataset::Complex::new(re, im))
                        .collect(),
                )
            }
        };
        signals.push(Signal::new(signal.name.clone(), signal.unit, data));
    }

    // `tran.output_interval` is already recorded by the backend that ran the
    // analysis; this layer records only what it changed.
    let info: BackendInfo = dataset
        .backend
        .clone()
        .with_setting("tran.output_grid", "resampled-linear".to_string())
        .with_setting("tran.output_points", out_times.len().to_string());

    let mut resampled = Dataset::new(
        dataset.experiment.clone(),
        dataset.analysis.clone(),
        dataset.kind.clone(),
        Axis::Time(out_times),
        signals,
        info,
        limits,
    )?;
    for d in &dataset.diagnostics {
        resampled.push_diagnostic(d.clone());
    }
    Ok(resampled)
}

/// Linear interpolation of `raw` (sampled at `times`) onto `out_times`.
///
/// Both axes are ascending; `out_times` is inside the raw range by
/// construction. The bisection keeps this `O(n log n)` rather than `O(n²)` for
/// a long trace.
fn interpolate(raw: &[f64], times: &[f64], out_times: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(out_times.len());
    if raw.is_empty() || times.is_empty() {
        return out;
    }
    let mut lo = 0usize; // index of the raw sample at or before the current t
    for &t in out_times {
        if t <= times[0] {
            out.push(raw[0]);
            continue;
        }
        if t >= times[times.len() - 1] {
            out.push(raw[raw.len() - 1]);
            continue;
        }
        // Advance `lo` until times[lo] <= t < times[lo + 1].
        while lo + 1 < times.len() && times[lo + 1] <= t {
            lo += 1;
        }
        while lo > 0 && times[lo] > t {
            lo -= 1;
        }
        let (t0, t1) = (times[lo], times[lo + 1]);
        if t1 <= t0 {
            // A non-advancing raw axis cannot be interpolated; the sample at
            // `lo` is the only defensible value and `Dataset::validate`
            // already rejects malformed axes elsewhere.
            out.push(raw[lo]);
            continue;
        }
        let w = (t - t0) / (t1 - t0);
        let (v0, v1) = (raw[lo], raw[lo + 1]);
        out.push(v0 + (v1 - v0) * w);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use circuit_core::units::VOLTAGE;

    fn tran_dataset(times: Vec<f64>, values: Vec<f64>) -> Dataset {
        Dataset::new(
            "e",
            "tran1",
            "tran",
            Axis::Time(times),
            vec![Signal::real("v(out)", VOLTAGE, values)],
            BackendInfo::new("thevenin", "0.5.0"),
            &Limits::default(),
        )
        .expect("well-formed result")
    }

    fn times_of(d: &Dataset) -> Vec<f64> {
        d.axis.samples().to_vec()
    }

    fn real_of(d: &Dataset) -> Vec<f64> {
        match &d.signals[0].data {
            Data::Real(v) => v.clone(),
            Data::Complex(_) => panic!("expected real data"),
        }
    }

    #[test]
    fn grid_starts_at_the_first_point_and_keeps_the_last() {
        let d = tran_dataset(vec![0.0, 0.3, 0.7, 1.0], vec![0.0, 0.3, 0.7, 1.0]);
        let out = resample_time(&d, 0.25, &Limits::default()).unwrap();
        // 0, 0.25, 0.5, 0.75 then the raw last point 1.0.
        assert_eq!(times_of(&out), vec![0.0, 0.25, 0.5, 0.75, 1.0]);
    }

    #[test]
    fn a_grid_point_on_the_last_sample_is_not_duplicated() {
        let d = tran_dataset(vec![0.0, 0.5, 1.0], vec![0.0, 0.5, 1.0]);
        let out = resample_time(&d, 0.5, &Limits::default()).unwrap();
        assert_eq!(times_of(&out), vec![0.0, 0.5, 1.0]);
    }

    #[test]
    fn interior_values_are_linear_interpolations() {
        // x = t on a non-uniform axis; every interpolated value is its time.
        let d = tran_dataset(vec![0.0, 1.0, 3.0], vec![0.0, 1.0, 3.0]);
        let out = resample_time(&d, 0.5, &Limits::default()).unwrap();
        let t = times_of(&out);
        let v = real_of(&out);
        for (ti, vi) in t.iter().zip(&v) {
            assert!((vi - ti).abs() < 1e-12, "{ti} -> {vi}");
        }
    }

    #[test]
    fn nothing_is_extrapolated_beyond_the_raw_axis() {
        let d = tran_dataset(vec![0.0, 1.0], vec![0.0, 1.0]);
        let out = resample_time(&d, 0.3, &Limits::default()).unwrap();
        let t = times_of(&out);
        assert_eq!(t.first().copied(), Some(0.0));
        assert_eq!(t.last().copied(), Some(1.0));
        assert!(t.iter().all(|x| (0.0..=1.0).contains(x)));
    }

    #[test]
    fn the_interval_becomes_the_output_point_count() {
        // A 2 us solver trace at 1 ns; asking for 100 ns must collapse it to
        // about 21 points. The exact count can differ by one at the last ulp
        // (the raw endpoint and `20 * 100 ns` need not be the same f64), so the
        // contract asserted here is: about one point per interval, endpoints
        // preserved, and the count equal to the grid that is built. The exact
        // 21 for the engine's own axis is asserted on the product path in
        // `circuit-session/tests/tran_output_interval.rs`.
        let raw: Vec<f64> = (0..=2000).map(|i| i as f64 * 1e-9).collect();
        let raw_values = raw.clone();
        let d = tran_dataset(raw, raw_values);
        let out = resample_time(&d, 100e-9, &Limits::default()).unwrap();
        assert!(
            (21..=22).contains(&out.axis.len()),
            "expected about 21 output points, got {}",
            out.axis.len()
        );
        assert_eq!(d.axis.len(), 2001, "the raw dataset is untouched");
        assert_eq!(out.axis.first(), Some(0.0));
        assert_eq!(out.axis.last(), d.axis.last());
        // Uniform interior spacing, with a last interval no longer than one
        // interval (it ends on the raw endpoint).
        let t = times_of(&out);
        for pair in t.windows(2) {
            assert!(pair[1] > pair[0], "times must ascend: {pair:?}");
            assert!(
                pair[1] - pair[0] <= 100e-9 + 1e-21,
                "interval exceeded: {pair:?}"
            );
        }
        let grid = OutputGrid::from_axis(&d.axis, 100e-9).unwrap();
        assert_eq!(grid.point_count() as usize, out.axis.len());
    }

    #[test]
    fn the_point_count_matches_the_built_grid() {
        for (first, last, iv) in [
            (0.0, 1.0, 0.3),
            (0.0, 1.0, 0.25),
            (0.0, 1.0, 1.0),
            (0.0, 1.0, 2.0),
            (1e-6, 2e-6, 1e-7),
        ] {
            let d = tran_dataset(vec![first, (first + last) / 2.0, last], vec![0.0, 0.5, 1.0]);
            let grid = OutputGrid::from_axis(&d.axis, iv).unwrap();
            let out = resample_time(&d, iv, &Limits::default()).unwrap();
            assert_eq!(
                grid.point_count() as usize,
                out.axis.len(),
                "first={first} last={last} iv={iv}"
            );
            assert_eq!(
                times_of(&out),
                grid.points(),
                "first={first} last={last} iv={iv}"
            );
        }
    }

    #[test]
    fn an_oversized_output_is_rejected_not_truncated() {
        let raw: Vec<f64> = (0..1000).map(|i| i as f64).collect();
        let d = tran_dataset(raw.clone(), raw);
        let tight = Limits {
            max_result_values: 100,
            ..Limits::default()
        };
        let err = resample_time(&d, 0.001, &tight).expect_err("should exceed the limit");
        assert!(err.iter().any(|e| e.code == Code::Limit), "{err:?}");
    }

    #[test]
    fn a_non_time_axis_is_returned_unchanged() {
        let d = Dataset::new(
            "e",
            "ac1",
            "ac",
            Axis::Frequency(vec![1.0, 10.0]),
            vec![Signal::real("v(out)", VOLTAGE, vec![1.0, 0.1])],
            BackendInfo::new("thevenin", "0.5.0"),
            &Limits::default(),
        )
        .unwrap();
        let out = resample_time(&d, 1e-6, &Limits::default()).unwrap();
        assert_eq!(times_of(&out), vec![1.0, 10.0]);
    }

    #[test]
    fn a_degenerate_axis_is_returned_unchanged() {
        let d = tran_dataset(vec![0.0, 1.0], vec![0.0, 1.0]);
        let single = Dataset::new(
            "e",
            "tran1",
            "tran",
            Axis::Time(vec![0.0]),
            vec![Signal::real("v(out)", VOLTAGE, vec![0.0])],
            BackendInfo::new("thevenin", "0.5.0"),
            &Limits::default(),
        )
        .unwrap();
        let out = resample_time(&single, 1e-6, &Limits::default()).unwrap();
        assert_eq!(out.axis.len(), 1);
        assert_eq!(times_of(&out), vec![0.0]);
        let _ = d;
    }

    #[test]
    fn complex_signals_are_resampled_component_wise() {
        use crate::dataset::Complex;
        let d = Dataset::new(
            "e",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1.0]),
            vec![Signal::complex(
                "i(r1)",
                circuit_core::units::CURRENT,
                vec![Complex::new(0.0, 0.0), Complex::new(2.0, 4.0)],
            )],
            BackendInfo::new("thevenin", "0.5.0"),
            &Limits::default(),
        )
        .unwrap();
        let out = resample_time(&d, 0.5, &Limits::default()).unwrap();
        match &out.signals[0].data {
            Data::Complex(v) => {
                assert_eq!(v.len(), 3);
                assert!((v[1].re - 1.0).abs() < 1e-12);
                assert!((v[1].im - 2.0).abs() < 1e-12);
            }
            Data::Real(_) => panic!("expected complex data"),
        }
    }

    #[test]
    fn metadata_records_the_output_grid() {
        let d = tran_dataset(vec![0.0, 1e-3], vec![0.0, 1.0]);
        let out = resample_time(&d, 1e-4, &Limits::default()).unwrap();
        assert_eq!(
            out.backend.setting("tran.output_grid"),
            Some("resampled-linear")
        );
        assert_eq!(out.backend.setting("tran.output_points"), Some("11"));
        // The interval itself is recorded once, by the backend that ran the
        // analysis; the resampler must not add a second copy.
        assert_eq!(out.backend.setting("tran.output_interval"), None);
    }

    #[test]
    fn a_rejected_interval_is_an_error_not_a_default() {
        let d = tran_dataset(vec![0.0, 1.0], vec![0.0, 1.0]);
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let err = resample_time(&d, bad, &Limits::default())
                .expect_err("a non-positive or non-finite interval must be an error");
            assert!(!err.is_empty(), "{bad}");
        }
    }
}
