//! Number and quantity display.
//!
//! Two formatters, one rule: what a user reads must be something the language
//! can read back.
//!
//! - [`format_number`] renders a bare `f64` (CSV fields, measurement
//!   summaries). Rust's `Display` never uses exponent notation, so `1e-300`
//!   would become a 300-character field; that case switches to exponent form,
//!   and both forms parse back to the same value.
//! - [`format_quantity`] renders a value with its dimension in engineering
//!   notation (`100 us`, `1.5 kohm`). It is what the REPL shows, and it uses
//!   [`DISPLAY_PREFIXES`], whose round trip through the parser is tested.
//!
//! Significant digits are a *display* decision: storage is always the full
//! `f64` in SI base units, and only the printed text is shortened.

use std::fmt::Write as _;

use crate::units::{DISPLAY_PREFIXES, Quantity, dimension_name};

/// Longest plain decimal form [`format_number`] will emit before switching to
/// exponent notation.
pub const MAX_PLAIN_DIGITS: usize = 32;

/// Significant digits [`format_quantity`] shows before rounding.
pub const QUANTITY_SIGNIFICANT_DIGITS: i32 = 6;

/// Format a finite number for a CSV field or a summary line.
///
/// Non-finite values are returned as `NaN`/`inf` text: the exporters check
/// [`f64::is_finite`] before calling this and emit an empty field instead.
pub fn format_number(value: f64) -> String {
    let plain = format!("{value}");
    if plain.len() <= MAX_PLAIN_DIGITS {
        plain
    } else {
        format!("{value:e}")
    }
}

/// Format a value with its dimension: `500 ohm`, `100 us`, `2`, `3.3 V`.
///
/// A named dimension gets an SI prefix chosen so the mantissa lands in
/// `[1, 1000)`; a dimensionless value gets neither prefix nor unit; a compound
/// dimension (say `V*A`) gets no prefix, because a prefix on a product is not
/// meaningful. Values too large or too small for the prefix table fall back to
/// exponent notation rather than printing a wall of digits.
pub fn format_quantity(q: Quantity) -> String {
    if !q.value.is_finite() {
        return non_finite(q.value);
    }

    if q.dimension.is_dimensionless() {
        return significant(q.value);
    }

    match dimension_name(q.dimension) {
        Some(symbol) => match scale_to_prefix(q.value) {
            Some((mantissa, prefix)) => {
                let mut out = significant(mantissa);
                out.push(' ');
                out.push_str(prefix);
                out.push_str(symbol);
                out
            }
            // Outside the prefix table: `1e20 V` beats twenty-one digits.
            None => format!("{} {symbol}", exponent_form(q.value)),
        },
        // A product such as `V*A` has no prefix convention, so leave it alone.
        None => {
            let mut out = significant(q.value);
            out.push(' ');
            let _ = write!(out, "{}", q.dimension);
            out
        }
    }
}

fn non_finite(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_string()
    } else if value > 0.0 {
        "inf".to_string()
    } else {
        "-inf".to_string()
    }
}

/// Exponent form for values no prefix or plain decimal can show compactly.
fn exponent_form(value: f64) -> String {
    format!("{value:e}")
}

/// Round to [`QUANTITY_SIGNIFICANT_DIGITS`] significant digits and trim.
///
/// Values whose magnitude is outside what a plain decimal can show compactly
/// fall back to exponent notation, because `format!("{:.17}", 1e-30)` is
/// `0.00000000000000000` — a string that has lost the value entirely.
fn significant(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    if !value.is_finite() {
        return non_finite(value);
    }

    let magnitude = value.abs().log10().floor() as i32;
    if !(-12..=15).contains(&magnitude) {
        return format!("{value:e}");
    }

    let decimals = (QUANTITY_SIGNIFICANT_DIGITS - 1 - magnitude).clamp(0, 17) as usize;
    let text = format!("{value:.decimals$}");
    match text.split_once('.') {
        Some((whole, fraction)) => {
            let fraction = fraction.trim_end_matches('0');
            if fraction.is_empty() {
                whole.to_string()
            } else {
                format!("{whole}.{fraction}")
            }
        }
        None => text,
    }
}

/// [`significant`] as a number, so callers can test whether rounding pushed a
/// mantissa out of its decade before printing it.
fn round_significant(value: f64) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    let magnitude = value.abs().log10().floor() as i32;
    let decimals = (QUANTITY_SIGNIFICANT_DIGITS - 1 - magnitude).clamp(0, 17) as usize;
    let factor = 10f64.powi(decimals as i32);
    (value * factor).round() / factor
}

/// Split `value` into a mantissa in `[1, 1000)` and the prefix symbol that
/// scales it back, or `None` when no prefix in the table can do that.
///
/// The mantissa is rounded *before* the range check: `999.9999 ns` rounds to
/// `1000 ns` at six significant digits, and `1000` is not a mantissa this
/// formatter is willing to print, so it steps up to `1 us`.
fn scale_to_prefix(value: f64) -> Option<(f64, &'static str)> {
    if value == 0.0 {
        return Some((0.0, ""));
    }

    let magnitude = value.abs().log10().floor() as i32;
    // Prefixes step by three decades; round down to the nearest step.
    let mut exponent = magnitude.div_euclid(3) * 3;

    loop {
        let &(table_exponent, symbol) = DISPLAY_PREFIXES.iter().find(|(e, _)| *e == exponent)?;
        let mantissa = round_significant(value / 10f64.powi(table_exponent));
        if mantissa.abs() >= 1000.0 {
            exponent += 3;
            continue;
        }
        if mantissa != 0.0 && mantissa.abs() < 1.0 {
            exponent -= 3;
            continue;
        }
        return Some((mantissa, symbol));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::{CAPACITANCE, FREQUENCY, RESISTANCE, TIME, VOLTAGE, resolve_unit_suffix};

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

    #[test]
    fn quantities_use_an_engineering_prefix() {
        assert_eq!(format_quantity(Quantity::ohms(500.0)), "500 ohm");
        assert_eq!(format_quantity(Quantity::ohms(1000.0)), "1 kohm");
        assert_eq!(format_quantity(Quantity::ohms(1500.0)), "1.5 kohm");
        assert_eq!(format_quantity(Quantity::ohms(2_200_000.0)), "2.2 Mohm");
        assert_eq!(format_quantity(Quantity::farads(100e-9)), "100 nF");
        assert_eq!(format_quantity(Quantity::farads(1e-12)), "1 pF");
        assert_eq!(format_quantity(Quantity::seconds(1e-6)), "1 us");
        assert_eq!(format_quantity(Quantity::seconds(1e-4)), "100 us");
        assert_eq!(format_quantity(Quantity::volts(3.3)), "3.3 V");
        assert_eq!(format_quantity(Quantity::volts(0.005)), "5 mV");
        assert_eq!(format_quantity(Quantity::hertz(1e6)), "1 MHz");
        assert_eq!(format_quantity(Quantity::henries(1e-3)), "1 mH");
        assert_eq!(format_quantity(Quantity::amps(0.02)), "20 mA");
    }

    #[test]
    fn quantities_round_to_six_significant_digits() {
        // 1 kohm / 3 as the REPL would show it.
        assert_eq!(format_quantity(Quantity::ohms(1000.0 / 3.0)), "333.333 ohm");
        // Trailing zeros are not printed.
        assert_eq!(format_quantity(Quantity::ohms(1000.5)), "1.0005 kohm");
        // A value that rounds up to 1000 steps up a prefix instead of showing
        // a four-digit mantissa.
        assert_eq!(format_quantity(Quantity::seconds(999.9999e-9)), "1 us");
    }

    #[test]
    fn dimensionless_and_compound_dimensions_have_no_prefix() {
        assert_eq!(format_quantity(Quantity::scalar(2.0)), "2");
        assert_eq!(format_quantity(Quantity::scalar(1234.5)), "1234.5");
        // V*A (power) has no conventional single symbol, so no prefix either.
        let power = Quantity::volts(2.0) * Quantity::amps(3.0);
        assert_eq!(format_quantity(power), "6 V*A");
    }

    #[test]
    fn values_outside_the_prefix_table_use_exponent_notation() {
        assert_eq!(format_quantity(Quantity::volts(1e20)), "1e20 V");
        assert_eq!(format_quantity(Quantity::volts(1e-21)), "1e-21 V");
    }

    #[test]
    fn zero_keeps_its_unit() {
        assert_eq!(format_quantity(Quantity::volts(0.0)), "0 V");
        assert_eq!(format_quantity(Quantity::scalar(0.0)), "0");
        assert_eq!(format_quantity(Quantity::new(0.0, TIME)), "0 s");
    }

    /// Whatever the formatter prints must be what the language spells: the
    /// prefix and the unit symbol must resolve when read back.
    #[test]
    fn formatted_quantities_use_spellable_units() {
        let cases = [
            Quantity::ohms(1500.0),
            Quantity::farads(100e-9),
            Quantity::seconds(1e-4),
            Quantity::volts(0.005),
            Quantity::hertz(1e6),
            Quantity::henries(1e-3),
            Quantity::amps(0.02),
        ];
        for q in cases {
            let text = format_quantity(q);
            let (mantissa, suffix) = text.split_once(' ').expect("mantissa and unit");
            assert!(
                resolve_unit_suffix(suffix).is_some(),
                "`{suffix}` of `{text}` must resolve"
            );
            assert_eq!(
                resolve_unit_suffix(suffix).expect("checked").1,
                q.dimension,
                "`{text}` must have the dimension it started with"
            );
            let mantissa: f64 = mantissa.parse().expect("the mantissa parses");
            let (scale, _) = resolve_unit_suffix(suffix).expect("checked");
            let back = mantissa * scale;
            assert!(
                (back - q.value).abs() <= 1e-6 * q.value.abs().max(f64::MIN_POSITIVE),
                "`{text}` reads back as {back}, not {}",
                q.value
            );
        }
    }

    /// `V*A` is the one dimension whose display has no unit suffix to resolve.
    #[test]
    fn compound_dimensions_do_not_claim_a_unit() {
        let power = Quantity::volts(2.0) * Quantity::amps(3.0);
        let text = format_quantity(power);
        let suffix = text.split_once(' ').expect("mantissa and unit").1;
        assert_eq!(suffix, "V*A");
        assert!(resolve_unit_suffix(suffix).is_none());
    }

    #[test]
    fn capacitance_and_voltage_prefixes() {
        assert_eq!(format_quantity(Quantity::new(1e-6, CAPACITANCE)), "1 uF");
        assert_eq!(format_quantity(Quantity::new(1e3, VOLTAGE)), "1 kV");
        assert_eq!(
            format_quantity(Quantity::new(1e3, RESISTANCE)),
            "1 kohm",
            "resistance keeps the ASCII spelling the language uses"
        );
        assert_eq!(format_quantity(Quantity::new(1e6, FREQUENCY)), "1 MHz");
    }
}
