//! Instrument reference data from the venue (BNP 1.1, ADR 0054 §2): tick
//! size, quoted decimals, contract multiplier and the rest. Requested once
//! per connection after `Listings`, page by page, and kept for the run.
//! Against a 1.0 venue there is none, and prices show as whole ticks.

use super::{Connection, Model};
use bunting_client::bnp_wire::{
    ClientMessage, CurrencyInfo, InstrumentInfo, Listing, ListingReference,
    MAX_REFERENCE_INSTRUMENTS, ServerMessage, speaks_1_1,
};
use std::collections::BTreeMap;

/// Instruments with reference data at most (the engine's own bound).
pub const MAX_REFERENCE: usize = 256;

#[derive(Clone, Debug, Default)]
pub(super) struct ReferenceState {
    currencies: BTreeMap<u128, CurrencyInfo>,
    instruments: BTreeMap<u128, InstrumentInfo>,
    listings: BTreeMap<Listing, ListingReference>,
    /// The page request in flight, if any.
    pending: Option<u32>,
}

impl Model {
    /// The venue's reference data for `instrument_id`, once received.
    #[must_use]
    pub fn instrument_reference(&self, instrument_id: u128) -> Option<&InstrumentInfo> {
        self.reference.instruments.get(&instrument_id)
    }

    /// The venue's reference data for `listing`, once received.
    #[must_use]
    pub fn listing_reference(&self, listing: Listing) -> Option<&ListingReference> {
        self.reference.listings.get(&listing)
    }

    /// A currency's reference data, once received.
    #[must_use]
    pub fn currency(&self, currency_id: u128) -> Option<&CurrencyInfo> {
        self.reference.currencies.get(&currency_id)
    }

    /// Decimal places to show `instrument_id`'s prices with: the venue's
    /// quoted decimals, or 0 (whole ticks) without reference data.
    #[must_use]
    pub fn quoted_decimals(&self, instrument_id: u128) -> u8 {
        self.instrument_reference(instrument_id)
            .map_or(0, |instrument| instrument.quoted_decimals)
    }

    /// Decimal places of `instrument_id`'s currency, for money such as its
    /// P&L and cash flows: the venue's minor decimals, or 0 (minor units
    /// as sent) without reference data.
    #[must_use]
    pub fn money_decimals(&self, instrument_id: u128) -> u8 {
        self.instrument_reference(instrument_id)
            .and_then(|instrument| self.currency(instrument.currency_id))
            .map_or(0, |currency| currency.minor_decimals)
    }

    /// Decimal places for money summed over every instrument (total P&L,
    /// cash): the largest currency minor decimals the venue reported, or 0.
    #[must_use]
    pub fn account_money_decimals(&self) -> u8 {
        self.reference
            .currencies
            .values()
            .map(|currency| currency.minor_decimals)
            .max()
            .unwrap_or(0)
    }

    /// `listing`'s smallest price step in ticks (1 without reference data).
    #[must_use]
    pub fn tick_size(&self, listing: Listing) -> i64 {
        self.listing_reference(listing)
            .map_or(1, |reference| reference.tick_size.max(1))
    }

    /// The first reference-data page request, on a 1.1 session.
    pub(super) fn request_reference(&mut self) -> Option<ClientMessage> {
        let Connection::Connected { welcome, .. } = self.connection() else {
            return None;
        };
        if !speaks_1_1(welcome.version) || self.reference.pending.is_some() {
            return None;
        }
        Some(self.reference_page(None))
    }

    fn reference_page(&mut self, after_instrument: Option<u128>) -> ClientMessage {
        let request_id = self.request_id();
        self.reference.pending = Some(request_id);
        ClientMessage::ReferenceDataRequest {
            request_id,
            after_instrument,
            limit: MAX_REFERENCE_INSTRUMENTS,
        }
    }

    /// Applies one `ReferenceData` page and asks for the next.
    pub(super) fn apply_reference(&mut self, message: ServerMessage) -> Vec<ClientMessage> {
        let ServerMessage::ReferenceData {
            request_id,
            next_after,
            currencies,
            instruments,
            listings,
            ..
        } = message
        else {
            return Vec::new();
        };
        if self.reference.pending != Some(request_id) {
            return Vec::new();
        }
        self.reference.pending = None;
        for currency in currencies {
            self.reference
                .currencies
                .insert(currency.currency_id, currency);
        }
        for instrument in instruments {
            if self.reference.instruments.len() == MAX_REFERENCE
                && !self
                    .reference
                    .instruments
                    .contains_key(&instrument.instrument_id)
            {
                break;
            }
            self.series_set_known_multiplier(instrument.instrument_id, instrument.multiplier);
            self.reference
                .instruments
                .insert(instrument.instrument_id, instrument);
        }
        for listing in listings {
            if self.reference.listings.len() < MAX_REFERENCE
                || self.reference.listings.contains_key(&listing.listing)
            {
                self.reference.listings.insert(listing.listing, listing);
            }
        }
        match next_after {
            Some(after) if self.reference.instruments.len() < MAX_REFERENCE => {
                vec![self.reference_page(Some(after))]
            }
            _ => Vec::new(),
        }
    }
}
