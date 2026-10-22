//! Prices and money as traders read them. Prices are integer ticks on the
//! wire and money integer minor units; the venue's reference data (BNP 1.1)
//! says how many decimal places each shows ([`crate::Model::quoted_decimals`],
//! [`crate::Model::money_decimals`]). Without it the decimals are 0 and the
//! integers show as sent. Shared by the desk and the terminal.

/// `value` in units of 10^-`decimals`, as a decimal: `fixed(-1050, 2)` is
/// `-10.50`; 0 decimals is the integer.
#[must_use]
pub fn fixed(value: i128, decimals: u8) -> String {
    if decimals == 0 {
        return value.to_string();
    }
    let scale = 10_u128.pow(u32::from(decimals));
    let sign = if value < 0 { "-" } else { "" };
    let magnitude = value.unsigned_abs();
    format!(
        "{sign}{}.{:0width$}",
        magnitude / scale,
        magnitude % scale,
        width = usize::from(decimals)
    )
}

/// A price of `ticks` at `decimals` places: 10050 at 2 is `100.50`.
#[must_use]
pub fn ticks(ticks: i64, decimals: u8) -> String {
    fixed(i128::from(ticks), decimals)
}

/// A typed price at `decimals` places back to ticks: `100.5` at 2 is
/// 10050. `None` for anything that is not a number with at most
/// `decimals` places.
#[must_use]
pub fn parse_price(text: &str, decimals: u8) -> Option<i64> {
    let text = text.trim();
    let (negative, digits) = text
        .strip_prefix('-')
        .map_or((false, text), |rest| (true, rest));
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    if fraction.len() > usize::from(decimals)
        || whole.is_empty() && fraction.is_empty()
        || !whole
            .chars()
            .chain(fraction.chars())
            .all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let scale = 10_i64.checked_pow(u32::from(decimals))?;
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let padded = format!("{fraction:0<width$}", width = usize::from(decimals));
    let fraction: i64 = if padded.is_empty() {
        0
    } else {
        padded.parse().ok()?
    };
    let value = whole.checked_mul(scale)?.checked_add(fraction)?;
    Some(if negative { -value } else { value })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_show_and_parse_at_their_decimals() {
        assert_eq!(ticks(10_050, 2), "100.50");
        assert_eq!(ticks(-5, 2), "-0.05");
        assert_eq!(ticks(101, 0), "101");
        assert_eq!(
            fixed(i128::MIN, 2),
            "-1701411834604692317316873037158841057.28",
            "never overflows"
        );
        assert_eq!(parse_price("100.5", 2), Some(10_050));
        assert_eq!(parse_price("-.05", 2), Some(-5));
        assert_eq!(parse_price("100.505", 2), None);
        assert_eq!(parse_price("7", 0), Some(7));
        assert_eq!(parse_price("7.0", 0), None);
        assert_eq!(parse_price("", 2), None);
    }
}
