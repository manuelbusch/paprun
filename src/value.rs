//! Laufzeitwerte und die exakte Nachbildung der Java-`BigDecimal`-Semantik.
//!
//! Kernstück ist [`div_scale`], das Javas `divide(divisor, scale, roundingMode)`
//! über exakte `BigInt`-Arithmetik nachbildet — ohne Precision-Parameter und
//! damit ohne Doppelrundungsrisiko.

use crate::error::Error;
use bigdecimal::BigDecimal;
use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{Signed, ToPrimitive, Zero};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::sync::{Arc, LazyLock};

/// Java-`RoundingMode` bzw. die `BigDecimal.ROUND_*`-Konstanten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundMode {
    Up,
    Down,
    Ceiling,
    Floor,
    HalfUp,
    HalfDown,
    HalfEven,
}

impl RoundMode {
    /// Akzeptiert beide Schreibweisen: `ROUND_DOWN` (BigDecimal-Konstante)
    /// und `DOWN` (RoundingMode-Enum).
    pub fn from_java_name(name: &str) -> Option<RoundMode> {
        let name = name.strip_prefix("ROUND_").unwrap_or(name);
        Some(match name {
            "UP" => RoundMode::Up,
            "DOWN" => RoundMode::Down,
            "CEILING" => RoundMode::Ceiling,
            "FLOOR" => RoundMode::Floor,
            "HALF_UP" => RoundMode::HalfUp,
            "HALF_DOWN" => RoundMode::HalfDown,
            "HALF_EVEN" => RoundMode::HalfEven,
            _ => return None,
        })
    }
}

impl From<RoundMode> for bigdecimal::RoundingMode {
    fn from(mode: RoundMode) -> Self {
        use bigdecimal::RoundingMode as R;
        match mode {
            RoundMode::Up => R::Up,
            RoundMode::Down => R::Down,
            RoundMode::Ceiling => R::Ceiling,
            RoundMode::Floor => R::Floor,
            RoundMode::HalfUp => R::HalfUp,
            RoundMode::HalfDown => R::HalfDown,
            RoundMode::HalfEven => R::HalfEven,
        }
    }
}

/// Ein Laufzeitwert des Interpreters.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Java `int`/`long` (Steuerklassen, Flags, Jahreszahlen).
    Int(i64),
    /// Java `BigDecimal`.
    Dec(BigDecimal),
    /// Java `double` (im PAP nur der Faktor `f`).
    Dbl(f64),
    /// Vergleichsergebnis; wird nie in einer Variablen gespeichert.
    Bool(bool),
    /// Konstantentabelle (`BigDecimal[]`).
    Arr(Arc<Vec<Value>>),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "int",
            Value::Dec(_) => "BigDecimal",
            Value::Dbl(_) => "double",
            Value::Bool(_) => "boolean",
            Value::Arr(_) => "BigDecimal[]",
        }
    }
}

/// Vorberechnete Zehnerpotenzen; deckt jeden im PAP vorkommenden
/// Scale-Unterschied ab, ohne pro Aufruf zu allozieren.
static POW10: LazyLock<Vec<BigInt>> = LazyLock::new(|| {
    let mut out = Vec::with_capacity(41);
    let mut value = BigInt::from(1);
    for _ in 0..=40 {
        out.push(value.clone());
        value *= 10;
    }
    out
});

/// `10^exp` als Referenz (`exp >= 0`); jenseits des Caches wird gerechnet.
fn pow10(exp: i64) -> Cow<'static, BigInt> {
    let exp = usize::try_from(exp).expect("negativer Zehnerpotenz-Exponent");
    match POW10.get(exp) {
        Some(value) => Cow::Borrowed(value),
        None => Cow::Owned(BigInt::from(10u8).pow(u32::try_from(exp).expect("Exponent zu groß"))),
    }
}

/// Rundet den Bruch `num / den` (den != 0) auf eine ganze Zahl gemäß `mode`.
/// Exakte Java-Semantik über den Divisionsrest.
fn round_quotient(num: &BigInt, den: &BigInt, mode: RoundMode) -> BigInt {
    // `div_rem` trunkiert Richtung null; der Rest trägt das Vorzeichen von `num`.
    let (quotient, remainder) = num.div_rem(den);
    if remainder.is_zero() {
        return quotient;
    }

    let negative = num.is_negative() != den.is_negative();
    // `magnitude()` liefert den Betrag ohne Allokation.
    let (rem_abs, den_abs) = (remainder.magnitude(), den.magnitude());

    let increment = match mode {
        RoundMode::Up => true,
        RoundMode::Down => false,
        RoundMode::Ceiling => !negative,
        RoundMode::Floor => negative,
        // Vergleich von 2·|Rest| mit |Divisor| entscheidet die Halb-Modi.
        RoundMode::HalfUp => (rem_abs << 1) >= *den_abs,
        RoundMode::HalfDown => (rem_abs << 1) > *den_abs,
        RoundMode::HalfEven => {
            let twice = rem_abs << 1;
            twice > *den_abs || (twice == *den_abs && quotient.is_odd())
        }
    };

    if !increment {
        quotient
    } else if negative {
        quotient - 1
    } else {
        quotient + 1
    }
}

/// Java `a.divide(b, scale, roundingMode)`: Quotient exakt auf `scale`
/// Nachkommastellen gerundet.
pub fn div_scale(
    a: &BigDecimal,
    b: &BigDecimal,
    scale: i64,
    mode: RoundMode,
) -> Result<BigDecimal, Error> {
    if b.is_zero() {
        return Err(Error::eval("Division durch null"));
    }
    // `as_bigint_and_scale` liefert Cow, klont die Mantissen also nur bei Bedarf.
    let (ai, a_scale) = a.as_bigint_and_scale();
    let (bi, b_scale) = b.as_bigint_and_scale();
    // a/b * 10^scale = ai * 10^(scale + b_scale - a_scale) / bi
    let k = scale + b_scale - a_scale;
    let quotient = match k {
        0 => round_quotient(&ai, &bi, mode),
        k if k > 0 => round_quotient(&(&*ai * &*pow10(k)), &bi, mode),
        k => round_quotient(&ai, &(&*bi * &*pow10(-k)), mode),
    };
    Ok(BigDecimal::new(quotient, scale))
}

/// Java `a.setScale(scale, roundingMode)`: reines Umskalieren der Mantisse.
/// Beim Vergrößern des Scale werden Nullen angehängt, beim Verkleinern wird
/// durch eine Zehnerpotenz geteilt und gerundet.
pub fn set_scale(a: &BigDecimal, scale: i64, mode: RoundMode) -> Result<BigDecimal, Error> {
    let (ai, a_scale) = a.as_bigint_and_scale();
    let mantissa = match scale.cmp(&a_scale) {
        Ordering::Equal => ai.into_owned(),
        Ordering::Greater => &*ai * &*pow10(scale - a_scale),
        Ordering::Less => round_quotient(&ai, &pow10(a_scale - scale), mode),
    };
    Ok(BigDecimal::new(mantissa, scale))
}

/// Java `a.divide(b)` ohne Scale-Angabe: nur erlaubt, wenn der Quotient
/// endlich viele Nachkommastellen hat (sonst wirft Java eine
/// `ArithmeticException`). Der PAP nutzt das nur für Teiler wie 2 oder 100.
pub fn div_exact(a: &BigDecimal, b: &BigDecimal) -> Result<BigDecimal, Error> {
    if b.is_zero() {
        return Err(Error::eval("Division durch null"));
    }
    // Javas Ergebnis-Scale für exakte Division ist die "preferred scale"
    // (a.scale - b.scale), erweitert bis das Ergebnis exakt ist.
    let start = a.as_bigint_and_exponent().1 - b.as_bigint_and_exponent().1;
    let start = start.max(0);
    for scale in start..=start + 64 {
        let q = div_scale(a, b, scale, RoundMode::Down)?;
        if &q * b == *a {
            return Ok(q);
        }
    }
    Err(Error::eval(format!(
        "divide ohne Rundungsmodus: Quotient {a}/{b} ist nicht endlich darstellbar"
    )))
}

/// Java `a.longValue()`: Abschneiden der Nachkommastellen Richtung null.
pub fn long_value(a: &BigDecimal) -> Result<i64, Error> {
    let truncated = set_scale(a, 0, RoundMode::Down)?;
    let (bi, _) = truncated.as_bigint_and_exponent();
    bi.to_i64()
        .ok_or_else(|| Error::eval(format!("longValue: {a} übersteigt den i64-Bereich")))
}

/// Java `BigDecimal.valueOf(double)`: konvertiert über `Double.toString`
/// (kürzeste Round-trip-Darstellung). Rusts `{}`-Formatierung liefert dieselben
/// Ziffern; Java hängt an ganze Zahlen ein `.0` an (Scale 1), das bilden wir nach.
pub fn value_of_f64(f: f64) -> Result<BigDecimal, Error> {
    if !f.is_finite() {
        return Err(Error::eval(format!(
            "BigDecimal.valueOf({f}): Wert ist nicht endlich"
        )));
    }
    let mut s = format!("{f}");
    if !s.contains('.') {
        s.push_str(".0");
    }
    s.parse::<BigDecimal>()
        .map_err(|e| Error::eval(format!("BigDecimal.valueOf({f}): {e}")))
}

/// Plain-Darstellung ohne wissenschaftliche Notation, Scale-erhaltend
/// (entspricht Java `toPlainString`).
pub fn to_plain_string(d: &BigDecimal) -> String {
    d.to_plain_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn dec(s: &str) -> BigDecimal {
        BigDecimal::from_str(s).unwrap()
    }

    fn ds(a: &str, b: &str, scale: i64, mode: RoundMode) -> String {
        to_plain_string(&div_scale(&dec(a), &dec(b), scale, mode).unwrap())
    }

    fn ss(a: &str, scale: i64, mode: RoundMode) -> String {
        to_plain_string(&set_scale(&dec(a), scale, mode).unwrap())
    }

    #[test]
    fn divide_basic() {
        assert_eq!(ds("100", "3", 2, RoundMode::Down), "33.33");
        assert_eq!(ds("100", "3", 2, RoundMode::Up), "33.34");
        assert_eq!(ds("2", "3", 0, RoundMode::Up), "1");
        assert_eq!(ds("2", "3", 0, RoundMode::Down), "0");
        // Scale-Erhalt: exakte Quotienten bekommen trotzdem den Ziel-Scale
        assert_eq!(ds("1", "4", 4, RoundMode::Down), "0.2500");
    }

    #[test]
    fn divide_negative() {
        assert_eq!(ds("-7", "2", 0, RoundMode::Down), "-3");
        assert_eq!(ds("-7", "2", 0, RoundMode::Floor), "-4");
        assert_eq!(ds("-7", "2", 0, RoundMode::Ceiling), "-3");
        assert_eq!(ds("-7", "2", 0, RoundMode::Up), "-4");
        assert_eq!(ds("7", "-2", 0, RoundMode::Floor), "-4");
    }

    #[test]
    fn divide_ties() {
        assert_eq!(ds("5", "2", 0, RoundMode::HalfUp), "3");
        assert_eq!(ds("5", "2", 0, RoundMode::HalfDown), "2");
        assert_eq!(ds("5", "2", 0, RoundMode::HalfEven), "2");
        assert_eq!(ds("7", "2", 0, RoundMode::HalfEven), "4");
        assert_eq!(ds("-5", "2", 0, RoundMode::HalfUp), "-3");
        assert_eq!(ds("-5", "2", 0, RoundMode::HalfDown), "-2");
    }

    #[test]
    fn divide_by_zero() {
        assert!(div_scale(&dec("1"), &dec("0"), 2, RoundMode::Down).is_err());
    }

    #[test]
    fn set_scale_java_semantics() {
        // Der klassische double-Fallstrick: exakt gerechnet ist 2.005 → 2.01.
        assert_eq!(ss("2.005", 2, RoundMode::HalfUp), "2.01");
        assert_eq!(ss("-7.5", 0, RoundMode::HalfUp), "-8");
        assert_eq!(ss("-7.5", 0, RoundMode::HalfDown), "-7");
        assert_eq!(ss("-7.5", 0, RoundMode::Down), "-7");
        assert_eq!(ss("-7.5", 0, RoundMode::Floor), "-8");
        assert_eq!(ss("1.004999", 2, RoundMode::HalfUp), "1.00");
        // Scale vergrößern hängt Nullen an
        assert_eq!(ss("3.1", 3, RoundMode::Down), "3.100");
    }

    #[test]
    fn div_exact_terminating() {
        assert_eq!(
            to_plain_string(&div_exact(&dec("5"), &dec("2")).unwrap()),
            "2.5"
        );
        assert_eq!(
            to_plain_string(&div_exact(&dec("300"), &dec("100")).unwrap()),
            "3"
        );
        assert!(div_exact(&dec("1"), &dec("3")).is_err());
    }

    #[test]
    fn long_value_truncates() {
        assert_eq!(long_value(&dec("3.99")).unwrap(), 3);
        assert_eq!(long_value(&dec("-3.99")).unwrap(), -3);
        assert_eq!(long_value(&dec("0")).unwrap(), 0);
    }

    #[test]
    fn value_of_double_shortest_roundtrip() {
        assert_eq!(to_plain_string(&value_of_f64(0.07).unwrap()), "0.07");
        assert_eq!(to_plain_string(&value_of_f64(0.4).unwrap()), "0.4");
        assert_eq!(to_plain_string(&value_of_f64(1.0).unwrap()), "1.0");
        assert_eq!(to_plain_string(&value_of_f64(0.9163).unwrap()), "0.9163");
    }

    #[test]
    fn round_mode_names() {
        assert_eq!(
            RoundMode::from_java_name("ROUND_DOWN"),
            Some(RoundMode::Down)
        );
        assert_eq!(
            RoundMode::from_java_name("HALF_UP"),
            Some(RoundMode::HalfUp)
        );
        assert_eq!(
            RoundMode::from_java_name("ROUND_HALF_EVEN"),
            Some(RoundMode::HalfEven)
        );
        assert_eq!(RoundMode::from_java_name("FOO"), None);
    }
}
