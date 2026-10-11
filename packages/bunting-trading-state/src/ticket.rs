//! An order ticket: what a trader fills in before sending a `NewOrder`.

use bunting_client::bnp_wire::{Listing, NewOrder, OrderType, Side, TimeInForce};

/// The price instruction chosen on the ticket.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TicketKind {
    Limit,
    Market,
}

/// How long the order may rest, as offered on the ticket. GTD needs a run
/// time the trader cannot see yet, so it is not offered here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TicketTif {
    Gtc,
    Ioc,
    Fok,
    Day,
}

impl TicketTif {
    pub const ALL: [Self; 4] = [Self::Gtc, Self::Ioc, Self::Fok, Self::Day];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Gtc => "GTC",
            Self::Ioc => "IOC",
            Self::Fok => "FOK",
            Self::Day => "DAY",
        }
    }

    const fn to_wire(self) -> TimeInForce {
        match self {
            Self::Gtc => TimeInForce::Gtc,
            Self::Ioc => TimeInForce::Ioc,
            Self::Fok => TimeInForce::Fok,
            Self::Day => TimeInForce::Day,
        }
    }
}

/// Why a ticket cannot be sent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TicketError {
    NoListing,
    Quantity,
    Price,
    Display,
    /// Market orders cannot rest, so they cannot be post-only or icebergs.
    MarketFlags,
}

impl std::fmt::Display for TicketError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NoListing => "choose a venue listing",
            Self::Quantity => "quantity must be a positive number of lots",
            Self::Price => "limit price must be a positive number of ticks",
            Self::Display => "display quantity must be positive and at most the quantity",
            Self::MarketFlags => "a market order cannot be post-only or an iceberg",
        })
    }
}

impl std::error::Error for TicketError {}

/// The fields of one order before it is sent. The venue is part of
/// `listing`: the trader chooses where the order goes, and each venue sits
/// at its own distance (ADR 0035).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderTicket {
    pub listing: Option<Listing>,
    pub side: Side,
    pub quantity: i64,
    pub kind: TicketKind,
    pub price: i64,
    pub tif: TicketTif,
    pub post_only: bool,
    pub anonymous: bool,
    pub display_quantity: Option<i64>,
}

impl OrderTicket {
    #[must_use]
    pub const fn new(listing: Option<Listing>, side: Side) -> Self {
        Self {
            listing,
            side,
            quantity: 1,
            kind: TicketKind::Limit,
            price: 0,
            tif: TicketTif::Gtc,
            post_only: false,
            anonymous: false,
            display_quantity: None,
        }
    }

    /// The `NewOrder` this ticket describes.
    ///
    /// # Errors
    /// Returns the first field that is not valid.
    pub fn to_order(&self, client_order_id: u64) -> Result<NewOrder, TicketError> {
        let listing = self.listing.ok_or(TicketError::NoListing)?;
        if self.quantity <= 0 {
            return Err(TicketError::Quantity);
        }
        let order_type = match self.kind {
            TicketKind::Limit if self.price <= 0 => return Err(TicketError::Price),
            TicketKind::Limit => OrderType::Limit { price: self.price },
            TicketKind::Market if self.post_only || self.display_quantity.is_some() => {
                return Err(TicketError::MarketFlags);
            }
            TicketKind::Market => OrderType::Market,
        };
        if self
            .display_quantity
            .is_some_and(|display| display <= 0 || display > self.quantity)
        {
            return Err(TicketError::Display);
        }
        Ok(NewOrder {
            client_order_id,
            listing,
            side: self.side,
            quantity: self.quantity,
            order_type,
            time_in_force: match self.kind {
                // A market order never rests: the venue treats it as IOC.
                TicketKind::Market => TimeInForce::Ioc,
                TicketKind::Limit => self.tif.to_wire(),
            },
            post_only: self.post_only,
            anonymous: self.anonymous,
            display_quantity: self.display_quantity,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: Listing = Listing {
        venue_id: 2,
        instrument_id: 1,
    };

    #[test]
    fn a_limit_ticket_becomes_an_order_for_its_venue() -> Result<(), TicketError> {
        let mut ticket = OrderTicket::new(Some(LISTING), Side::Buy);
        ticket.quantity = 5;
        ticket.price = 99;
        ticket.tif = TicketTif::Day;
        ticket.display_quantity = Some(2);
        let order = ticket.to_order(7)?;
        assert_eq!(order.listing.venue_id, 2);
        assert_eq!(order.order_type, OrderType::Limit { price: 99 });
        assert_eq!(order.time_in_force, TimeInForce::Day);
        assert_eq!(order.display_quantity, Some(2));
        Ok(())
    }

    #[test]
    fn invalid_tickets_name_the_field() {
        let mut ticket = OrderTicket::new(None, Side::Sell);
        assert_eq!(ticket.to_order(1), Err(TicketError::NoListing));
        ticket.listing = Some(LISTING);
        assert_eq!(ticket.to_order(1), Err(TicketError::Price));
        ticket.kind = TicketKind::Market;
        ticket.post_only = true;
        assert_eq!(ticket.to_order(1), Err(TicketError::MarketFlags));
        ticket.post_only = false;
        ticket.quantity = 0;
        assert_eq!(ticket.to_order(1), Err(TicketError::Quantity));
        ticket.quantity = 3;
        assert_eq!(
            ticket.to_order(1).map(|order| order.time_in_force),
            Ok(TimeInForce::Ioc)
        );
    }
}
