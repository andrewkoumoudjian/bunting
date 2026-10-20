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

/// Prices are integer ticks on the wire; they are shown as sent.
#[must_use]
pub fn price(price: Option<i64>) -> String {
    price.map_or_else(|| "market".to_owned(), |price| price.to_string())
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
    use super::*;

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
