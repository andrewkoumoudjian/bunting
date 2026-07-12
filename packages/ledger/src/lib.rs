#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Exact zero-fee cash, position, and reservation projection.

use bunting_market_events::{EventPayload, Side};
use bunting_market_types::{InstrumentId, MoneyMinor, ParticipantId, PriceTicks, QuantityLots};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum LedgerError {
    ArithmeticOverflow,
    InvalidRelease,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Account {
    pub cash: MoneyMinor,
    pub reserved_cash: MoneyMinor,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Holding {
    pub position: QuantityLots,
    pub reserved_inventory: QuantityLots,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Ledger {
    accounts: BTreeMap<ParticipantId, Account>,
    holdings: BTreeMap<(ParticipantId, InstrumentId), Holding>,
}
pub type AccountProjection = Vec<(ParticipantId, Account)>;
pub type HoldingProjection = Vec<(ParticipantId, InstrumentId, Holding)>;

/// Exact inputs for one zero-fee trade settlement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TradeSettlement {
    pub buyer: ParticipantId,
    pub seller: ParticipantId,
    pub instrument: InstrumentId,
    pub buyer_limit: PriceTicks,
    pub seller_limit: PriceTicks,
    pub execution_price: PriceTicks,
    pub quantity: QuantityLots,
}

impl Ledger {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_cash(&mut self, p: ParticipantId, cash: MoneyMinor) {
        self.accounts.entry(p).or_default().cash = cash;
    }
    pub fn set_position(&mut self, p: ParticipantId, i: InstrumentId, q: QuantityLots) {
        self.holdings.entry((p, i)).or_default().position = q;
    }
    #[must_use]
    pub fn account(&self, p: ParticipantId) -> Account {
        self.accounts.get(&p).copied().unwrap_or_default()
    }
    #[must_use]
    pub fn holding(&self, p: ParticipantId, i: InstrumentId) -> Holding {
        self.holdings.get(&(p, i)).copied().unwrap_or_default()
    }
    #[must_use]
    pub fn available_cash(&self, p: ParticipantId) -> Option<MoneyMinor> {
        let a = self.account(p);
        a.cash.checked_sub(a.reserved_cash)
    }
    #[must_use]
    pub fn available_inventory(&self, p: ParticipantId, i: InstrumentId) -> Option<QuantityLots> {
        let h = self.holding(p, i);
        h.position.checked_sub(h.reserved_inventory)
    }
    pub fn reserve(
        &mut self,
        p: ParticipantId,
        i: InstrumentId,
        side: Side,
        price: PriceTicks,
        q: QuantityLots,
    ) -> Result<(), LedgerError> {
        match side {
            Side::Buy => {
                let amount = MoneyMinor::checked_mul_price_quantity(price, q)
                    .map_err(|_| LedgerError::ArithmeticOverflow)?;
                let a = self.accounts.entry(p).or_default();
                a.reserved_cash = a
                    .reserved_cash
                    .checked_add(amount)
                    .ok_or(LedgerError::ArithmeticOverflow)?;
            }
            Side::Sell => {
                let h = self.holdings.entry((p, i)).or_default();
                h.reserved_inventory = h
                    .reserved_inventory
                    .checked_add(q)
                    .ok_or(LedgerError::ArithmeticOverflow)?;
            }
        }
        Ok(())
    }
    pub fn release(
        &mut self,
        p: ParticipantId,
        i: InstrumentId,
        side: Side,
        price: PriceTicks,
        q: QuantityLots,
    ) -> Result<(), LedgerError> {
        match side {
            Side::Buy => {
                let amount = MoneyMinor::checked_mul_price_quantity(price, q)
                    .map_err(|_| LedgerError::ArithmeticOverflow)?;
                let a = self.accounts.entry(p).or_default();
                a.reserved_cash = a
                    .reserved_cash
                    .checked_sub(amount)
                    .filter(|v| v.get() >= 0)
                    .ok_or(LedgerError::InvalidRelease)?;
            }
            Side::Sell => {
                let h = self.holdings.entry((p, i)).or_default();
                h.reserved_inventory = h
                    .reserved_inventory
                    .checked_sub(q)
                    .filter(|v| v.get() >= 0)
                    .ok_or(LedgerError::InvalidRelease)?;
            }
        }
        Ok(())
    }
    pub fn apply(&mut self, event: &EventPayload) -> Result<(), LedgerError> {
        match event {
            EventPayload::PositionChanged {
                participant_id,
                instrument_id,
                delta,
            } => {
                let h = self
                    .holdings
                    .entry((*participant_id, *instrument_id))
                    .or_default();
                h.position = h
                    .position
                    .checked_add(*delta)
                    .ok_or(LedgerError::ArithmeticOverflow)?;
            }
            EventPayload::BalanceChanged {
                participant_id,
                delta,
            } => {
                let a = self.accounts.entry(*participant_id).or_default();
                a.cash = a
                    .cash
                    .checked_add(*delta)
                    .ok_or(LedgerError::ArithmeticOverflow)?;
            }
            _ => {}
        }
        Ok(())
    }
    /// Restores exact account and holding projections.
    #[must_use]
    pub fn from_projection(accounts: AccountProjection, holdings: HoldingProjection) -> Self {
        Self {
            accounts: accounts.into_iter().collect(),
            holdings: holdings
                .into_iter()
                .map(|(participant, instrument, holding)| ((participant, instrument), holding))
                .collect(),
        }
    }

    /// Settles a zero-fee trade and releases reservations at limit prices.
    pub fn settle_trade(&mut self, trade: TradeSettlement) -> Result<(), LedgerError> {
        self.release(
            trade.buyer,
            trade.instrument,
            Side::Buy,
            trade.buyer_limit,
            trade.quantity,
        )?;
        self.release(
            trade.seller,
            trade.instrument,
            Side::Sell,
            trade.seller_limit,
            trade.quantity,
        )?;
        let notional =
            MoneyMinor::checked_mul_price_quantity(trade.execution_price, trade.quantity)
                .map_err(|_| LedgerError::ArithmeticOverflow)?;
        let buyer_account = self.accounts.entry(trade.buyer).or_default();
        buyer_account.cash = buyer_account
            .cash
            .checked_sub(notional)
            .ok_or(LedgerError::ArithmeticOverflow)?;
        let seller_account = self.accounts.entry(trade.seller).or_default();
        seller_account.cash = seller_account
            .cash
            .checked_add(notional)
            .ok_or(LedgerError::ArithmeticOverflow)?;
        let buyer_holding = self
            .holdings
            .entry((trade.buyer, trade.instrument))
            .or_default();
        buyer_holding.position = buyer_holding
            .position
            .checked_add(trade.quantity)
            .ok_or(LedgerError::ArithmeticOverflow)?;
        let seller_holding = self
            .holdings
            .entry((trade.seller, trade.instrument))
            .or_default();
        seller_holding.position = seller_holding
            .position
            .checked_sub(trade.quantity)
            .ok_or(LedgerError::ArithmeticOverflow)?;
        Ok(())
    }
    #[must_use]
    pub fn projection(&self) -> (AccountProjection, HoldingProjection) {
        (
            self.accounts.iter().map(|(p, a)| (*p, *a)).collect(),
            self.holdings
                .iter()
                .map(|((p, i), h)| (*p, *i, *h))
                .collect(),
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn reservations_release_exactly() {
        let p = ParticipantId::new(1);
        let i = InstrumentId::new(1);
        let mut l = Ledger::new();
        l.set_cash(p, MoneyMinor(100));
        l.reserve(p, i, Side::Buy, PriceTicks(10), QuantityLots(4))
            .unwrap();
        assert_eq!(l.available_cash(p), Some(MoneyMinor(60)));
        l.release(p, i, Side::Buy, PriceTicks(10), QuantityLots(4))
            .unwrap();
        assert_eq!(l.available_cash(p), Some(MoneyMinor(100)));
    }
}
