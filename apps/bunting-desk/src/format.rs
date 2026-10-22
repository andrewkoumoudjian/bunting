//! Text for numbers, times and identifiers, shared by every panel.

use bunting_trading_state::bunting_client::bnp_wire::{Liquidity, Listing, Side};

/// `1234` µs as `1.23 ms`.
#[must_use]
pub fn duration(micros: u64) -> String {
    if micros < 1_000 {
        format!("{micros} µs")
    } else if micros < 1_000_000 {
        format!("{}.{:02} ms", micros / 1_000, (micros % 1_000) / 10)
    } else {
        format!(
            "{}.{:02} s",
            micros / 1_000_000,
            (micros % 1_000_000) / 10_000
        )
    }
}

/// Microseconds since the app started as `seconds.milliseconds`.
#[must_use]
pub fn clock(micros: u64) -> String {
    format!("{}.{:03}", micros / 1_000_000, (micros % 1_000_000) / 1_000)
}

/// The run's logical time in nanoseconds as `seconds.milliseconds`.
#[must_use]
pub fn logical(nanos: u64) -> String {
    format!(
        "{}.{:03}",
        nanos / 1_000_000_000,
        (nanos % 1_000_000_000) / 1_000_000
    )
}

#[must_use]
pub fn venue(listing: Listing) -> String {
    format!("Venue {}", listing.venue_id)
}

#[must_use]
pub const fn side(side: Side) -> &'static str {
    match side {
        Side::Buy => "Buy",
        Side::Sell => "Sell",
    }
}

#[must_use]
pub const fn liquidity(liquidity: Liquidity) -> &'static str {
    match liquidity {
        Liquidity::Maker => "maker",
        Liquidity::Taker => "taker",
    }
}

/// An average price from exact tick-lots over its lots: one
/// decimal place, two when the second is needed, never rounded
/// away to a whole tick. `lots` is signed (the position); the
/// price is positive either way.
#[must_use]
pub fn average_price(tick_lots: i128, lots: i64) -> String {
    let lots = i128::from(lots).abs().max(1);
    let hundredths = (tick_lots.abs() * 100 + lots / 2) / lots;
    let whole = hundredths / 100;
    let cents = hundredths % 100;
    let sign = if tick_lots < 0 { "-" } else { "" };
    if cents % 10 == 0 {
        format!("{sign}{whole}.{}", cents / 10)
    } else {
        format!("{sign}{whole}.{cents:02}")
    }
}

/// Prices are integer ticks on the wire; they are shown as sent.
#[must_use]
pub fn price(price: Option<i64>) -> String {
    price.map_or_else(|| "market".to_owned(), |price| price.to_string())
}

pub use bunting_trading_state::display::{fixed, parse_price, ticks};

/// [`ticks`] for an order price; `None` is a market order.
#[must_use]
pub fn price_at(price: Option<i64>, decimals: u8) -> String {
    price.map_or_else(|| "market".to_owned(), |price| ticks(price, decimals))
}

/// An average price from exact tick-lots over its lots, at the quote's
/// `decimals` plus up to two more places when the average needs them
/// (never rounded away to a whole tick). With 0 decimals this is
/// [`average_price`].
#[must_use]
pub fn average_price_at(tick_lots: i128, lots: i64, decimals: u8) -> String {
    if decimals == 0 {
        return average_price(tick_lots, lots);
    }
    let lots = i128::from(lots).abs().max(1);
    // Hundredths of a tick, rounded half up.
    let hundredths = (tick_lots.abs() * 100 + lots / 2) / lots;
    let text = fixed(hundredths, decimals.saturating_add(2));
    // Trim the extra places back while they are zero.
    let trimmed = text
        .strip_suffix("00")
        .or_else(|| text.strip_suffix('0'))
        .unwrap_or(&text);
    let sign = if tick_lots < 0 { "-" } else { "" };
    format!("{sign}{trimmed}")
}

/// Money in minor units shown in major units with digit grouping:
/// `money(123_456_78, 2)` is `123,456.78`; 0 decimals is [`grouped`].
#[must_use]
pub fn money(minor: i128, decimals: u8) -> String {
    if decimals == 0 {
        return grouped(minor);
    }
    let scale = 10_i128.pow(u32::from(decimals));
    let whole = grouped(minor / scale);
    let sign = if minor < 0 && minor / scale == 0 {
        "-"
    } else {
        ""
    };
    format!(
        "{sign}{whole}.{:0width$}",
        (minor % scale).unsigned_abs(),
        width = usize::from(decimals)
    )
}

/// [`money`] with an explicit `+` on positive amounts, for P&L.
#[must_use]
pub fn money_signed(minor: i128, decimals: u8) -> String {
    if minor > 0 {
        format!("+{}", money(minor, decimals))
    } else {
        money(minor, decimals)
    }
}

/// A whole number with a thousands separator: `1234567` as `1,234,567`.
#[must_use]
pub fn grouped(value: i128) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut text = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if value < 0 {
        text.push('-');
    }
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

/// [`grouped`] with an explicit `+` on positive numbers, for P&L.
#[must_use]
pub fn grouped_signed(value: i128) -> String {
    if value > 0 {
        format!("+{}", grouped(value))
    } else {
        grouped(value)
    }
}

/// "Updated 1.20 s ago": how long since `received_us`, `now_us` being the
/// desk clock.
#[must_use]
pub fn updated_ago(received_us: u64, now_us: u64) -> String {
    format!(
        "Updated {} ago",
        duration(now_us.saturating_sub(received_us))
    )
}

#[cfg(test)]
mod tests {

    #[test]
    fn average_prices_keep_their_decimals() {
        assert_eq!(super::average_price(4_220, 40), "105.5");
        assert_eq!(super::average_price(2_100, 20), "105.0");
        assert_eq!(super::average_price(10_525, 100), "105.25");
        assert_eq!(super::average_price(1, 3), "0.33");
        assert_eq!(super::average_price(4_220, -40), "105.5");
    }
    use super::*;

    #[test]
    fn prices_show_and_parse_at_the_quoted_decimals() {
        assert_eq!(ticks(10_050, 2), "100.50");
        assert_eq!(ticks(5, 2), "0.05");
        assert_eq!(ticks(-1_050, 2), "-10.50");
        assert_eq!(ticks(101, 0), "101");
        assert_eq!(price_at(None, 2), "market");
        assert_eq!(parse_price("100.5", 2), Some(10_050));
        assert_eq!(parse_price("100", 2), Some(10_000));
        assert_eq!(parse_price(".05", 2), Some(5));
        assert_eq!(parse_price("100.505", 2), None, "finer than the quote");
        assert_eq!(parse_price("1e3", 2), None);
        assert_eq!(parse_price("", 2), None);
        assert_eq!(parse_price("42", 0), Some(42));
        assert_eq!(average_price_at(1_005_050, 100, 2), "100.505");
        assert_eq!(average_price_at(1_005_000, 100, 2), "100.50");
        assert_eq!(average_price_at(2_010_000, 200, 2), "100.50");
        assert_eq!(average_price_at(100_001, 10, 2), "100.001");
        assert_eq!(money(12_345_678, 2), "123,456.78");
        assert_eq!(money(-5, 2), "-0.05");
        assert_eq!(money(-12_345, 2), "-123.45");
        assert_eq!(money_signed(250, 2), "+2.50");
        assert_eq!(money(1_500, 0), "1,500");
    }

    #[test]
    fn numbers_group_by_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
        assert_eq!(grouped(-1_234_567), "-1,234,567");
        assert_eq!(grouped_signed(1_500), "+1,500");
        assert_eq!(grouped_signed(-5), "-5");
        assert_eq!(updated_ago(1_000, 2_501_000), "Updated 2.50 s ago");
    }

    #[test]
    fn durations_read_in_the_largest_whole_unit() {
        assert_eq!(duration(812), "812 µs");
        assert_eq!(duration(1_234), "1.23 ms");
        assert_eq!(duration(2_500_000), "2.50 s");
        assert_eq!(logical(1_500_000_000), "1.500");
        assert_eq!(clock(12_345_678), "12.345");
    }
}
