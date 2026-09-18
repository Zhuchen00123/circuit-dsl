//! Dimensions, quantities, and unit-literal parsing.
//!
//! # Design
//!
//! Internal numbers are always SI base units. A [`Quantity`] carries the
//! numeric value together with a [`Dimension`], so dimensional errors are
//! caught during elaboration rather than producing silently wrong physics.
//!
//! The base dimensions are chosen to cover exactly the device set the project
//! supports (brief §3.1): **volt**, **ampere**, **second**. Everything else is
//! derived:
//!
//! | quantity | expression | exponents (V, A, s) |
//! |---|---|---|
//! | ohm | V/A | (1, -1, 0) |
//! | farad | A·s/V | (-1, 1, 1) |
//! | henry | V·s/A | (1, -1, 1) |
//! | hertz | 1/s | (0, 0, -1) |
//!
//! Deliberately *not* modelled: length, temperature, and the other SI base
//! units. No supported device needs them, and adding unused dimensions would
//! invite the "universal device protocol" the brief warns against.
//!
//! The dimension is a runtime value, not a compile-time type parameter. The
//! brief sketches `Quantity<Dimension>`; a phantom-typed form would make the
//! elaborator monomorphize over every arithmetic combination and would fight
//! the dynamic value model the interpreter needs. The observable behaviour
//! required by the brief — every mismatched operation is reported with both
//! expected and received dimensions — is the same either way, and is covered
//! by tests below.

use std::fmt;

/// Exponents of (volt, ampere, second).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Dimension {
    pub volt: i8,
    pub amp: i8,
    pub second: i8,
}

impl Dimension {
    pub const fn new(volt: i8, amp: i8, second: i8) -> Self {
        Self { volt, amp, second }
    }

    pub const fn mul(self, rhs: Self) -> Self {
        Self {
            volt: self.volt + rhs.volt,
            amp: self.amp + rhs.amp,
            second: self.second + rhs.second,
        }
    }

    pub const fn div(self, rhs: Self) -> Self {
        Self {
            volt: self.volt - rhs.volt,
            amp: self.amp - rhs.amp,
            second: self.second - rhs.second,
        }
    }

    pub const fn pow(self, n: i8) -> Self {
        Self {
            volt: self.volt * n,
            amp: self.amp * n,
            second: self.second * n,
        }
    }

    pub const fn is_dimensionless(self) -> bool {
        self.volt == 0 && self.amp == 0 && self.second == 0
    }
}

pub const DIMENSIONLESS: Dimension = Dimension::new(0, 0, 0);
pub const VOLTAGE: Dimension = Dimension::new(1, 0, 0);
pub const CURRENT: Dimension = Dimension::new(0, 1, 0);
pub const TIME: Dimension = Dimension::new(0, 0, 1);
pub const FREQUENCY: Dimension = Dimension::new(0, 0, -1);
pub const RESISTANCE: Dimension = Dimension::new(1, -1, 0);
pub const CAPACITANCE: Dimension = Dimension::new(-1, 1, 1);
pub const INDUCTANCE: Dimension = Dimension::new(1, -1, 1);

/// The canonical display name of a dimension, or `None` if it has no
/// conventional single-symbol name.
pub fn dimension_name(d: Dimension) -> Option<&'static str> {
    Some(match (d.volt, d.amp, d.second) {
        (0, 0, 0) => "dimensionless",
        (1, 0, 0) => "V",
        (0, 1, 0) => "A",
        (0, 0, 1) => "s",
        (0, 0, -1) => "Hz",
        (1, -1, 0) => "ohm",
        (-1, 1, 1) => "F",
        (1, -1, 1) => "H",
        _ => return None,
    })
}

impl fmt::Display for Dimension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(name) = dimension_name(*self) {
            return f.write_str(name);
        }
        if self.is_dimensionless() {
            return f.write_str("dimensionless");
        }
        // Fall back to an explicit exponent form, e.g. "V^2*A^-1".
        let mut parts = Vec::new();
        for (sym, e) in [("V", self.volt), ("A", self.amp), ("s", self.second)] {
            if e == 0 {
                continue;
            }
            if e == 1 {
                parts.push(sym.to_string());
            } else {
                parts.push(format!("{sym}^{e}"));
            }
        }
        f.write_str(&parts.join("*"))
    }
}

impl fmt::Debug for Dimension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// A numeric value with a dimension, always stored in SI base units.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Quantity {
    pub value: f64,
    pub dimension: Dimension,
}

impl Quantity {
    pub const fn new(value: f64, dimension: Dimension) -> Self {
        Self { value, dimension }
    }

    /// A plain number with no dimension.
    pub const fn scalar(value: f64) -> Self {
        Self {
            value,
            dimension: DIMENSIONLESS,
        }
    }

    pub fn volts(v: f64) -> Self {
        Self::new(v, VOLTAGE)
    }
    pub fn amps(a: f64) -> Self {
        Self::new(a, CURRENT)
    }
    pub fn seconds(s: f64) -> Self {
        Self::new(s, TIME)
    }
    pub fn hertz(h: f64) -> Self {
        Self::new(h, FREQUENCY)
    }
    pub fn ohms(r: f64) -> Self {
        Self::new(r, RESISTANCE)
    }
    pub fn farads(c: f64) -> Self {
        Self::new(c, CAPACITANCE)
    }
    pub fn henries(l: f64) -> Self {
        Self::new(l, INDUCTANCE)
    }

    pub fn is_finite(&self) -> bool {
        self.value.is_finite()
    }

    /// Require this quantity to be dimensionless, returning the bare number.
    pub fn as_scalar(&self) -> Result<f64, DimensionMismatch> {
        if self.dimension.is_dimensionless() {
            Ok(self.value)
        } else {
            Err(DimensionMismatch {
                expected: DIMENSIONLESS,
                received: self.dimension,
            })
        }
    }

    /// Require an exact dimension.
    pub fn require(&self, expected: Dimension) -> Result<f64, DimensionMismatch> {
        if self.dimension == expected {
            Ok(self.value)
        } else {
            Err(DimensionMismatch {
                expected,
                received: self.dimension,
            })
        }
    }

    pub fn checked_add(self, rhs: Self) -> Result<Self, DimensionMismatch> {
        if self.dimension != rhs.dimension {
            return Err(DimensionMismatch {
                expected: self.dimension,
                received: rhs.dimension,
            });
        }
        Ok(Self::new(self.value + rhs.value, self.dimension))
    }

    pub fn checked_sub(self, rhs: Self) -> Result<Self, DimensionMismatch> {
        if self.dimension != rhs.dimension {
            return Err(DimensionMismatch {
                expected: self.dimension,
                received: rhs.dimension,
            });
        }
        Ok(Self::new(self.value - rhs.value, self.dimension))
    }

    /// `self / rhs` when the result must be dimensionless (ratio of like
    /// quantities), used for gain-style expressions.
    pub fn ratio(self, rhs: Self) -> Result<f64, DimensionMismatch> {
        if self.dimension != rhs.dimension {
            return Err(DimensionMismatch {
                expected: self.dimension,
                received: rhs.dimension,
            });
        }
        Ok(self.value / rhs.value)
    }
}

// Quantity arithmetic goes through the standard operators so that `a * b`,
// `a / b`, and `-a` read naturally at call sites. Dimensions propagate as
// described above; nothing here can fail, so the operators are infallible.

impl std::ops::Mul for Quantity {
    type Output = Quantity;
    fn mul(self, rhs: Self) -> Quantity {
        Quantity::new(self.value * rhs.value, self.dimension.mul(rhs.dimension))
    }
}

impl std::ops::Div for Quantity {
    type Output = Quantity;
    fn div(self, rhs: Self) -> Quantity {
        Quantity::new(self.value / rhs.value, self.dimension.div(rhs.dimension))
    }
}

impl std::ops::Neg for Quantity {
    type Output = Quantity;
    fn neg(self) -> Quantity {
        Quantity::new(-self.value, self.dimension)
    }
}

/// Two quantities whose dimensions do not agree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DimensionMismatch {
    pub expected: Dimension,
    pub received: Dimension,
}

impl fmt::Display for DimensionMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "expected {}, received {}", self.expected, self.received)
    }
}

impl std::error::Error for DimensionMismatch {}

// ---------------------------------------------------------------------------
// Unit suffixes
// ---------------------------------------------------------------------------

/// A base unit recognised after the `.` in a quantity literal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BaseUnit {
    pub symbol: &'static str,
    pub dimension: Dimension,
}

/// Base units, longest symbol first so that greedy suffix matching picks
/// `ohm` over a hypothetical `o`, and `Hz` over `H`.
const BASE_UNITS: &[BaseUnit] = &[
    BaseUnit {
        symbol: "ohm",
        dimension: RESISTANCE,
    },
    BaseUnit {
        symbol: "Ohm",
        dimension: RESISTANCE,
    },
    BaseUnit {
        symbol: "Ω",
        dimension: RESISTANCE,
    },
    BaseUnit {
        symbol: "Hz",
        dimension: FREQUENCY,
    },
    BaseUnit {
        symbol: "V",
        dimension: VOLTAGE,
    },
    BaseUnit {
        symbol: "A",
        dimension: CURRENT,
    },
    BaseUnit {
        symbol: "F",
        dimension: CAPACITANCE,
    },
    BaseUnit {
        symbol: "H",
        dimension: INDUCTANCE,
    },
    BaseUnit {
        symbol: "s",
        dimension: TIME,
    },
];

/// A decimal SI prefix.
struct Prefix {
    symbols: &'static [&'static str],
    exponent: i32,
}

/// Prefixes, longest symbol first so that `meg` wins over `m`.
///
/// Case matters: `m` is milli and `M` is mega (brief §6). `k`/`K` and
/// `g`/`G` are accepted in both cases, matching SPICE practice.
const PREFIXES: &[Prefix] = &[
    Prefix {
        symbols: &["meg", "MEG"],
        exponent: 6,
    },
    Prefix {
        symbols: &["f", "F"],
        exponent: -15,
    },
    Prefix {
        symbols: &["p", "P"],
        exponent: -12,
    },
    Prefix {
        symbols: &["n", "N"],
        exponent: -9,
    },
    // ASCII 'u', MICRO SIGN, and GREEK SMALL LETTER MU.
    Prefix {
        symbols: &["u", "µ", "μ", "U"],
        exponent: -6,
    },
    Prefix {
        symbols: &["m"],
        exponent: -3,
    },
    Prefix {
        symbols: &["k", "K"],
        exponent: 3,
    },
    Prefix {
        symbols: &["M"],
        exponent: 6,
    },
    Prefix {
        symbols: &["g", "G"],
        exponent: 9,
    },
    Prefix {
        symbols: &["t", "T"],
        exponent: 12,
    },
    Prefix {
        symbols: &["a", "A"],
        exponent: -18,
    },
];

/// Resolve a unit suffix such as `kohm`, `nF`, `us`, `MHz`.
///
/// Returns the SI scale factor and the dimension. This is a closed lookup —
/// unit symbols are restricted syntax, not method calls (brief §6).
pub fn resolve_unit_suffix(suffix: &str) -> Option<(f64, Dimension)> {
    for base in BASE_UNITS {
        let Some(head) = suffix.strip_suffix(base.symbol) else {
            continue;
        };
        if head.is_empty() {
            return Some((1.0, base.dimension));
        }
        for prefix in PREFIXES {
            if prefix.symbols.contains(&head) {
                return Some((10f64.powi(prefix.exponent), base.dimension));
            }
        }
        // A recognised base unit with an unrecognised prefix is a mistake
        // (`1.xV`), not a reason to try a different base unit.
        return None;
    }
    None
}

/// Whether `s` is a valid unit suffix. Used by the lexer to decide whether
/// `1.` starts a quantity literal.
pub fn is_unit_suffix(s: &str) -> bool {
    resolve_unit_suffix(s).is_some()
}

/// The longest valid unit suffix that `text` starts with.
///
/// The lexer uses this to consume `kohm` from `1.kohm,` without swallowing
/// the following comma. Matching is greedy on the base-unit symbol and then
/// on the prefix, so `1.megohm` yields `megohm` while `1.mV` yields `mV`.
pub fn longest_unit_suffix(text: &str) -> Option<usize> {
    // Try every prefix length, longest first, and keep the longest that
    // resolves. A hard upper bound avoids scanning an entire line.
    const MAX: usize = 8;
    let mut best = None;
    for len in 1..=MAX.min(text.chars().count()) {
        // Take `len` characters, not bytes.
        let end = text
            .char_indices()
            .nth(len)
            .map(|(i, _)| i)
            .unwrap_or(text.len());
        let candidate = &text[..end];
        if resolve_unit_suffix(candidate).is_some() {
            best = Some(end);
        }
    }
    best
}

/// Parse a quantity literal body: the numeric text, and the unit suffix after
/// the separating `.`.
///
/// `numeric` must already be a syntactically valid decimal/scientific literal.
/// A `None` `unit` yields a dimensionless quantity.
pub fn build_quantity(numeric: &str, unit: Option<&str>) -> Result<Quantity, UnitParseError> {
    let value: f64 = numeric
        .parse()
        .map_err(|_| UnitParseError::BadNumber(numeric.to_string()))?;

    let Some(unit) = unit else {
        return Ok(Quantity::scalar(value));
    };

    let (scale, dimension) =
        resolve_unit_suffix(unit).ok_or_else(|| UnitParseError::UnknownUnit(unit.to_string()))?;
    Ok(Quantity::new(value * scale, dimension))
}

/// Failures when turning a literal into a [`Quantity`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UnitParseError {
    BadNumber(String),
    UnknownUnit(String),
}

impl fmt::Display for UnitParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnitParseError::BadNumber(s) => write!(f, "`{s}` is not a valid number"),
            UnitParseError::UnknownUnit(s) => write!(f, "`{s}` is not a known unit"),
        }
    }
}

impl std::error::Error for UnitParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn suffix(s: &str) -> (f64, Dimension) {
        resolve_unit_suffix(s).unwrap_or_else(|| panic!("`{s}` should resolve"))
    }

    #[test]
    fn base_units_resolve() {
        assert_eq!(suffix("V"), (1.0, VOLTAGE));
        assert_eq!(suffix("A"), (1.0, CURRENT));
        assert_eq!(suffix("s"), (1.0, TIME));
        assert_eq!(suffix("Hz"), (1.0, FREQUENCY));
        assert_eq!(suffix("ohm"), (1.0, RESISTANCE));
        assert_eq!(suffix("F"), (1.0, CAPACITANCE));
        assert_eq!(suffix("H"), (1.0, INDUCTANCE));
    }

    #[test]
    fn ohm_spellings_agree() {
        assert_eq!(suffix("ohm"), suffix("Ohm"));
        assert_eq!(suffix("ohm"), suffix("Ω"));
        assert_eq!(suffix("kohm").0, 1e3);
        assert_eq!(suffix("kΩ").0, 1e3);
    }

    #[test]
    fn prefixes_scale_correctly() {
        assert_eq!(suffix("kV").0, 1e3);
        assert_eq!(suffix("mV").0, 1e-3);
        assert_eq!(suffix("uV").0, 1e-6);
        assert_eq!(suffix("µV").0, 1e-6);
        assert_eq!(suffix("nF").0, 1e-9);
        assert_eq!(suffix("pF").0, 1e-12);
        assert_eq!(suffix("us").0, 1e-6);
        assert_eq!(suffix("MHz").0, 1e6);
        assert_eq!(suffix("GHz").0, 1e9);
        assert_eq!(suffix("ms").0, 1e-3);
        assert_eq!(suffix("megohm").0, 1e6);
    }

    /// The case sensitivity rule from brief §6: `m` is milli, `M` is mega.
    #[test]
    fn m_is_milli_and_m_is_mega() {
        assert_eq!(suffix("mF").0, 1e-3, "millifarad");
        assert_eq!(suffix("MF").0, 1e6, "megafarad");
        assert_eq!(suffix("mH").0, 1e-3, "millihenry");
        assert_eq!(suffix("MH").0, 1e6, "megahenry");
        assert_ne!(suffix("mHz").0, suffix("MHz").0);
    }

    #[test]
    fn unknown_suffixes_are_rejected() {
        assert!(resolve_unit_suffix("xV").is_none());
        assert!(
            resolve_unit_suffix("m").is_none(),
            "bare prefix is not a unit"
        );
        assert!(resolve_unit_suffix("kg").is_none(), "mass is not modelled");
        assert!(resolve_unit_suffix("").is_none());
        assert!(resolve_unit_suffix("Vs").is_none());
    }

    #[test]
    fn longest_suffix_stops_before_punctuation() {
        assert_eq!(longest_unit_suffix("kohm,"), Some(4));
        assert_eq!(longest_unit_suffix("kohm"), Some(4));
        assert_eq!(longest_unit_suffix("nF)"), Some(2));
        assert_eq!(longest_unit_suffix("us,"), Some(2));
        assert_eq!(longest_unit_suffix("V"), Some(1));
        assert_eq!(longest_unit_suffix("xyz"), None);
    }

    #[test]
    fn quantity_literals_build() {
        /// Compare SI values with a relative tolerance; prefix scaling is
        /// done in binary floating point, so `100 * 1e-9` is not bit-identical
        /// to `1e-7`.
        fn close(a: f64, b: f64) -> bool {
            (a - b).abs() <= 1e-12 * b.abs().max(1.0)
        }

        let q = build_quantity("1", Some("kohm")).unwrap();
        assert_eq!(q.value, 1000.0);
        assert_eq!(q.dimension, RESISTANCE);

        let c = build_quantity("100", Some("nF")).unwrap();
        assert!(close(c.value, 100e-9), "got {}", c.value);
        assert_eq!(c.dimension, CAPACITANCE);

        let t = build_quantity("1", Some("us")).unwrap();
        assert!(close(t.value, 1e-6), "got {}", t.value);
        assert_eq!(t.dimension, TIME);

        let bare = build_quantity("2.5", None).unwrap();
        assert_eq!(bare.value, 2.5);
        assert!(bare.dimension.is_dimensionless());

        let sci = build_quantity("1e-3", Some("s")).unwrap();
        assert_eq!(sci.value, 1e-3);
        assert_eq!(sci.dimension, TIME);
    }

    #[test]
    fn arithmetic_derives_dimensions() {
        let v = Quantity::volts(2.0);
        let i = Quantity::amps(1e-3);
        let r = v / i;
        assert_eq!(r.value, 2000.0);
        assert_eq!(r.dimension, RESISTANCE);

        // V * A = power; not a named dimension but must be tracked.
        let p = v * i;
        assert_eq!(p.dimension, Dimension::new(1, 1, 0));
        assert_eq!(dimension_name(p.dimension), None);
        assert_eq!(p.dimension.to_string(), "V*A");

        // F = A*s/V
        let cap = Quantity::amps(1.0) * Quantity::seconds(1.0) / Quantity::volts(1.0);
        assert_eq!(cap.dimension, CAPACITANCE);

        // H = V*s/A
        let ind = Quantity::volts(1.0) * Quantity::seconds(1.0) / Quantity::amps(1.0);
        assert_eq!(ind.dimension, INDUCTANCE);

        // Hz = 1/s
        let f = Quantity::scalar(1.0) / Quantity::seconds(1.0);
        assert_eq!(f.dimension, FREQUENCY);

        // Operators behave as derived above.
        assert_eq!((-v).value, -2.0);
        assert_eq!(v * i, p);
        assert_eq!(v / i, r);
    }

    #[test]
    fn addition_requires_matching_dimensions() {
        let v = Quantity::volts(1.0);
        let t = Quantity::seconds(1.0);
        let err = v.checked_add(t).unwrap_err();
        assert_eq!(err.expected, VOLTAGE);
        assert_eq!(err.received, TIME);
        assert_eq!(err.to_string(), "expected V, received s");

        assert!(v.checked_add(Quantity::volts(2.0)).is_ok());
        assert!(v.checked_sub(Quantity::volts(2.0)).is_ok());
    }

    #[test]
    fn dimensionless_is_required_explicitly() {
        let v = Quantity::volts(1.0);
        assert!(v.as_scalar().is_err());
        assert_eq!(Quantity::scalar(3.0).as_scalar().unwrap(), 3.0);

        // A ratio of like quantities is dimensionless.
        let a = Quantity::volts(2.0);
        let b = Quantity::volts(4.0);
        assert_eq!(a.ratio(b).unwrap(), 0.5);
        assert!(a.ratio(Quantity::amps(1.0)).is_err());
    }

    #[test]
    fn pow_scales_exponents() {
        assert_eq!(VOLTAGE.pow(2), Dimension::new(2, 0, 0));
        assert_eq!(RESISTANCE.pow(-1), Dimension::new(-1, 1, 0));
    }

    #[test]
    fn unit_parse_errors_are_specific() {
        assert_eq!(
            build_quantity("abc", None).unwrap_err(),
            UnitParseError::BadNumber("abc".into())
        );
        assert_eq!(
            build_quantity("1", Some("xV")).unwrap_err(),
            UnitParseError::UnknownUnit("xV".into())
        );
    }
}
