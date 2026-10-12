//! RIT's one-click trading in the Book Trader, as gestures on a book row.
//!
//! With one-click on (it starts off, as in RIT), a click on a row sends an
//! order at once, to the venue whose book was clicked:
//!
//! - left-click a bid: buy `volume` with a limit at that price + `offset`
//!   ticks; left-click an ask: sell at that price − `offset`;
//! - right-click the ask side: market buy `volume`; the bid side: market
//!   sell `volume`;
//! - shift+right-click: sweep, an immediate-or-cancel limit at the clicked
//!   price for everything displayed up to it (RIT's "swipe");
//! - alt+left-click: cancel your orders at that price, whether one-click is
//!   on or not.
//!
//! With one-click off, a left-click only puts the price on the ticket.

use bunting_trading_state::bunting_client::bnp_wire::{Listing, Side};
use bunting_trading_state::{OrderTicket, TicketKind, TicketTif};

/// Largest volume or offset the fields take.
pub const MAX_STEP: i64 = 1_000_000;

/// One Book Trader's one-click settings (RIT's lightning toggle, V and O).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OneClick {
    pub enabled: bool,
    /// Quantity of each one-click order.
    pub volume: i64,
    /// Ticks added to a clicked bid or taken from a clicked ask.
    pub offset: i64,
}

impl Default for OneClick {
    fn default() -> Self {
        Self {
            enabled: false,
            volume: 100,
            offset: 0,
        }
    }
}

/// How a book row was clicked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Gesture {
    Left,
    Right,
    ShiftRight,
    AltLeft,
}

impl Gesture {
    /// The gesture a mouse click makes, from its button and modifiers.
    #[must_use]
    pub const fn from_click(right: bool, shift: bool, alt: bool) -> Self {
        match (right, shift, alt) {
            (true, true, _) => Self::ShiftRight,
            (true, false, _) => Self::Right,
            (false, _, true) => Self::AltLeft,
            (false, _, false) => Self::Left,
        }
    }
}

/// What a click on a book row asks for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Intent {
    /// Send this order now.
    Order(OrderTicket),
    /// Cancel your live orders on `listing` on `side` at `price`.
    Cancel {
        listing: Listing,
        side: Side,
        price: i64,
    },
    /// Put this price on the ticket.
    Pick(i64),
}

/// The clicked row: its venue, its side of the book, its price, and the
/// quantity displayed from the best price through this one on that side.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Row {
    pub listing: Listing,
    pub side: Side,
    pub price: i64,
    pub through: i64,
}

/// What `gesture` on `row` does with these settings, if anything.
#[must_use]
pub fn intent(settings: OneClick, row: Row, gesture: Gesture) -> Option<Intent> {
    if gesture == Gesture::AltLeft {
        return Some(Intent::Cancel {
            listing: row.listing,
            side: row.side,
            price: row.price,
        });
    }
    if !settings.enabled {
        return (gesture == Gesture::Left).then_some(Intent::Pick(row.price));
    }
    // Clicking the ask side trades against asks: a buy. And the reverse.
    let taking = match row.side {
        Side::Buy => Side::Sell,
        Side::Sell => Side::Buy,
    };
    let mut ticket = OrderTicket::new(Some(row.listing), row.side);
    ticket.quantity = settings.volume;
    match gesture {
        Gesture::Left => {
            ticket.price = match row.side {
                Side::Buy => row.price.checked_add(settings.offset)?,
                Side::Sell => row.price.checked_sub(settings.offset)?,
            };
        }
        Gesture::Right => {
            ticket.side = taking;
            ticket.kind = TicketKind::Market;
        }
        Gesture::ShiftRight => {
            ticket.side = taking;
            ticket.price = row.price;
            ticket.quantity = row.through;
            ticket.tif = TicketTif::Ioc;
        }
        Gesture::AltLeft => return None,
    }
    Some(Intent::Order(ticket))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEAR: Listing = Listing {
        venue_id: 1,
        instrument_id: 7,
    };

    fn on() -> OneClick {
        OneClick {
            enabled: true,
            volume: 10,
            offset: 2,
        }
    }

    fn row(side: Side, price: i64) -> Row {
        Row {
            listing: NEAR,
            side,
            price,
            through: 25,
        }
    }

    fn order(intent: Option<Intent>) -> OrderTicket {
        match intent {
            Some(Intent::Order(ticket)) => ticket,
            // Fails every assertion that follows.
            _ => OrderTicket::new(None, Side::Buy),
        }
    }

    #[test]
    fn left_click_joins_with_the_offset() {
        let bid = order(intent(on(), row(Side::Buy, 100), Gesture::Left));
        assert_eq!(
            (bid.side, bid.kind, bid.price, bid.quantity),
            (Side::Buy, TicketKind::Limit, 102, 10)
        );
        assert_eq!(bid.listing, Some(NEAR));
        let ask = order(intent(on(), row(Side::Sell, 105), Gesture::Left));
        assert_eq!((ask.side, ask.price), (Side::Sell, 103));
    }

    #[test]
    fn right_click_takes_the_clicked_side_at_market() {
        let buy = order(intent(on(), row(Side::Sell, 105), Gesture::Right));
        assert_eq!(
            (buy.side, buy.kind, buy.quantity),
            (Side::Buy, TicketKind::Market, 10)
        );
        let sell = order(intent(on(), row(Side::Buy, 100), Gesture::Right));
        assert_eq!((sell.side, sell.kind), (Side::Sell, TicketKind::Market));
    }

    #[test]
    fn shift_right_click_sweeps_through_the_price() {
        let sweep = order(intent(on(), row(Side::Sell, 107), Gesture::ShiftRight));
        assert_eq!(
            (
                sweep.side,
                sweep.kind,
                sweep.price,
                sweep.quantity,
                sweep.tif
            ),
            (Side::Buy, TicketKind::Limit, 107, 25, TicketTif::Ioc)
        );
    }

    #[test]
    fn alt_click_cancels_and_off_only_picks_the_price() {
        let off = OneClick::default();
        assert_eq!(
            intent(off, row(Side::Buy, 100), Gesture::AltLeft),
            Some(Intent::Cancel {
                listing: NEAR,
                side: Side::Buy,
                price: 100
            })
        );
        assert_eq!(
            intent(off, row(Side::Buy, 100), Gesture::Left),
            Some(Intent::Pick(100))
        );
        assert_eq!(intent(off, row(Side::Buy, 100), Gesture::Right), None);
        assert_eq!(Gesture::from_click(true, true, false), Gesture::ShiftRight);
        assert_eq!(Gesture::from_click(false, false, true), Gesture::AltLeft);
    }
}
