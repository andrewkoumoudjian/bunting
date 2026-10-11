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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_in_the_largest_whole_unit() {
        assert_eq!(duration(812), "812 µs");
        assert_eq!(duration(1_234), "1.23 ms");
        assert_eq!(duration(2_500_000), "2.50 s");
        assert_eq!(logical(1_500_000_000), "1.500");
        assert_eq!(clock(12_345_678), "12.345");
    }
}
