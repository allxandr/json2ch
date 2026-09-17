#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TickParseError {
    #[error("empty decimal string")]
    Empty,
    #[error("invalid decimal string")]
    ParsingError,
    #[error("overflow")]
    Overflow,
}

/// Parses a non-negative decimal string into fixed-point ticks:
/// real_value = ticks / 10^scale
pub fn decimal_str_to_ticks(s: &str, scale: u32) -> Result<i64, TickParseError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(TickParseError::Empty);
    }

    let (whole, frac) = match s.split_once('.') {
        None => (s, ""),
        Some((w, f)) => (w, f),
    };

    if whole.is_empty() && frac.is_empty() {
        return Err(TickParseError::Empty);
    }

    let whole_ticks: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| TickParseError::ParsingError)?
    };

    let frac = if (scale as usize) < frac.len() {
        &frac[..scale as usize]
    } else {
        frac
    };
    let mut frac_ticks: i64 = if frac.is_empty() {
        0
    } else {
        frac.parse().map_err(|_| TickParseError::ParsingError)?
    };

    // Pad "5" with scale=8  -> 50000000
    let pad = scale as usize - frac.len();
    for _ in 0..pad {
        frac_ticks = frac_ticks.checked_mul(10).ok_or(TickParseError::Overflow)?;
    }

    let multiplier = 10_i64.checked_pow(scale).ok_or(TickParseError::Overflow)?;

    whole_ticks
        .checked_mul(multiplier)
        .and_then(|w| w.checked_add(frac_ticks))
        .ok_or(TickParseError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_scales_by_pow10() {
        assert_eq!(decimal_str_to_ticks("1", 2).unwrap(), 100);
        assert_eq!(decimal_str_to_ticks("42000", 2).unwrap(), 4_200_000);
    }

    #[test]
    fn exact_fractional_digits() {
        assert_eq!(decimal_str_to_ticks("76014.81", 2).unwrap(), 7_601_481);
        assert_eq!(decimal_str_to_ticks("1.47181", 5).unwrap(), 147_181);
    }

    #[test]
    fn pads_short_fraction() {
        assert_eq!(decimal_str_to_ticks("1.5", 2).unwrap(), 150);
        assert_eq!(decimal_str_to_ticks("0.5", 8).unwrap(), 50_000_000);
    }

    #[test]
    fn truncates_extra_fractional_digits() {
        assert_eq!(decimal_str_to_ticks("1.239", 2).unwrap(), 123);
    }

    #[test]
    fn empty_is_error() {
        assert_eq!(decimal_str_to_ticks("", 2), Err(TickParseError::Empty));
        assert_eq!(decimal_str_to_ticks("   ", 2), Err(TickParseError::Empty));
    }

    #[test]
    fn non_numeric_is_error() {
        assert_eq!(
            decimal_str_to_ticks("abc", 2),
            Err(TickParseError::ParsingError)
        );
    }
}
