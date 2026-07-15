#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Exact zero-fee cash, position, and reservation projection.

use bunting_market_events::{EventPayload, Side};
use bunting_market_types::{
    CurrencyId, InstrumentId, MoneyMinor, ParticipantId, PriceTicks, QuantityLots,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum LedgerError {
    ArithmeticOverflow,
    InvalidRelease,
    UnbalancedTransaction,
    InvalidPosting,
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

/// Exact per-currency participant balance.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CurrencyBalance {
    pub settled: MoneyMinor,
    pub reserved: MoneyMinor,
    pub accrued: MoneyMinor,
    pub scheduled: MoneyMinor,
    pub fees: MoneyMinor,
    pub margin: MoneyMinor,
}

/// Exact position and valuation state for one instrument.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PositionBalance {
    pub settled: QuantityLots,
    pub reserved: QuantityLots,
    pub cost_basis: MoneyMinor,
    pub realized_pnl: MoneyMinor,
    pub unrealized_pnl: MoneyMinor,
}

/// Typed double-entry account used by journal postings.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PostingAccount {
    Cash,
    Accrued,
    Scheduled,
    Fees,
    Margin,
    Clearing,
}

/// One exact journal posting.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JournalPosting {
    pub participant_id: Option<ParticipantId>,
    pub currency_id: CurrencyId,
    pub account: PostingAccount,
    pub amount: MoneyMinor,
}

/// Economic reason for a balanced transaction.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionKind {
    Trade,
    Tender,
    Otc,
    Lease,
    Usage,
    Commission,
    MarkToMarket,
    Closeout,
    Settlement,
    Fine,
    Interest,
    Dividend,
    Adjustment,
}

/// Replayable balanced transaction.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JournalTransaction {
    pub transaction_id: u128,
    pub kind: TransactionKind,
    pub postings: Vec<JournalPosting>,
}

/// Multi-currency journal and valuation projection.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortfolioLedger {
    #[serde(with = "currency_balances")]
    balances: BTreeMap<(ParticipantId, CurrencyId), CurrencyBalance>,
    #[serde(with = "position_balances")]
    positions: BTreeMap<(ParticipantId, InstrumentId), PositionBalance>,
    journal: Vec<JournalTransaction>,
}

mod currency_balances {
    use super::{CurrencyBalance, CurrencyId, ParticipantId};
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    pub fn serialize<S>(
        value: &BTreeMap<(ParticipantId, CurrencyId), CurrencyBalance>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        value
            .iter()
            .map(|((participant, currency), balance)| (*participant, *currency, *balance))
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<(ParticipantId, CurrencyId), CurrencyBalance>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let values =
            Vec::<(ParticipantId, CurrencyId, CurrencyBalance)>::deserialize(deserializer)?;
        let mut output = BTreeMap::new();
        for (participant, currency, balance) in values {
            if output.insert((participant, currency), balance).is_some() {
                return Err(serde::de::Error::custom("duplicate currency balance"));
            }
        }
        Ok(output)
    }
}

mod position_balances {
    use super::{InstrumentId, ParticipantId, PositionBalance};
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    pub fn serialize<S>(
        value: &BTreeMap<(ParticipantId, InstrumentId), PositionBalance>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        value
            .iter()
            .map(|((participant, instrument), balance)| (*participant, *instrument, *balance))
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<(ParticipantId, InstrumentId), PositionBalance>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let values =
            Vec::<(ParticipantId, InstrumentId, PositionBalance)>::deserialize(deserializer)?;
        let mut output = BTreeMap::new();
        for (participant, instrument, balance) in values {
            if output.insert((participant, instrument), balance).is_some() {
                return Err(serde::de::Error::custom("duplicate position balance"));
            }
        }
        Ok(output)
    }
}

impl PortfolioLedger {
    /// Constructs an empty replayable portfolio ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns one exact currency balance.
    #[must_use]
    pub fn balance(&self, participant: ParticipantId, currency: CurrencyId) -> CurrencyBalance {
        self.balances
            .get(&(participant, currency))
            .copied()
            .unwrap_or_default()
    }

    /// Returns one exact position balance.
    #[must_use]
    pub fn position(
        &self,
        participant: ParticipantId,
        instrument: InstrumentId,
    ) -> PositionBalance {
        self.positions
            .get(&(participant, instrument))
            .copied()
            .unwrap_or_default()
    }

    /// Seeds a settled balance during deterministic run initialization.
    pub fn set_settled_cash(
        &mut self,
        participant: ParticipantId,
        currency: CurrencyId,
        amount: MoneyMinor,
    ) {
        self.balances
            .entry((participant, currency))
            .or_default()
            .settled = amount;
    }

    /// Seeds a settled position during deterministic run initialization.
    pub fn set_position(
        &mut self,
        participant: ParticipantId,
        instrument: InstrumentId,
        quantity: QuantityLots,
        cost_basis: MoneyMinor,
    ) {
        let position = self.positions.entry((participant, instrument)).or_default();
        position.settled = quantity;
        position.cost_basis = cost_basis;
    }

    /// Applies an exact settled position delta for physical simulation workflows.
    ///
    /// # Errors
    /// Returns an error when the resulting quantity overflows.
    pub fn adjust_position(
        &mut self,
        participant: ParticipantId,
        instrument: InstrumentId,
        delta: QuantityLots,
    ) -> Result<(), LedgerError> {
        let position = self.positions.entry((participant, instrument)).or_default();
        position.settled = position
            .settled
            .checked_add(delta)
            .ok_or(LedgerError::ArithmeticOverflow)?;
        Ok(())
    }

    /// Applies one balanced transaction atomically.
    ///
    /// # Errors
    /// Returns an error when postings are empty, unbalanced by currency, or overflow.
    pub fn post(&mut self, transaction: JournalTransaction) -> Result<(), LedgerError> {
        if transaction.postings.is_empty() {
            return Err(LedgerError::InvalidPosting);
        }
        let mut totals = BTreeMap::<CurrencyId, MoneyMinor>::new();
        for posting in &transaction.postings {
            let total = totals.entry(posting.currency_id).or_default();
            *total = total
                .checked_add(posting.amount)
                .ok_or(LedgerError::ArithmeticOverflow)?;
        }
        if totals.values().any(|total| total.get() != 0) {
            return Err(LedgerError::UnbalancedTransaction);
        }
        let mut candidate = self.clone();
        for posting in &transaction.postings {
            let Some(participant) = posting.participant_id else {
                continue;
            };
            let balance = candidate
                .balances
                .entry((participant, posting.currency_id))
                .or_default();
            let target = match posting.account {
                PostingAccount::Cash => &mut balance.settled,
                PostingAccount::Accrued => &mut balance.accrued,
                PostingAccount::Scheduled => &mut balance.scheduled,
                PostingAccount::Fees => &mut balance.fees,
                PostingAccount::Margin => &mut balance.margin,
                PostingAccount::Clearing => continue,
            };
            *target = target
                .checked_add(posting.amount)
                .ok_or(LedgerError::ArithmeticOverflow)?;
        }
        candidate.journal.push(transaction);
        *self = candidate;
        Ok(())
    }

    /// Applies an explicit mark and versioned valuation policy result.
    ///
    /// # Errors
    /// Returns an error when exact price-times-position arithmetic overflows.
    pub fn mark_position(
        &mut self,
        participant: ParticipantId,
        instrument: InstrumentId,
        mark: PriceTicks,
    ) -> Result<(), LedgerError> {
        let position = self.positions.entry((participant, instrument)).or_default();
        let value = MoneyMinor::checked_mul_price_quantity(mark, position.settled)
            .map_err(|_| LedgerError::ArithmeticOverflow)?;
        position.unrealized_pnl = value
            .checked_sub(position.cost_basis)
            .ok_or(LedgerError::ArithmeticOverflow)?;
        Ok(())
    }

    /// Calculates exact net liquidation value in one currency.
    ///
    /// # Errors
    /// Returns an error when aggregation overflows.
    pub fn net_liquidation_value(
        &self,
        participant: ParticipantId,
        currency: CurrencyId,
    ) -> Result<MoneyMinor, LedgerError> {
        let balance = self.balance(participant, currency);
        let mut total = balance
            .settled
            .checked_add(balance.accrued)
            .and_then(|value| value.checked_add(balance.scheduled))
            .and_then(|value| value.checked_add(balance.fees))
            .and_then(|value| value.checked_sub(balance.margin))
            .ok_or(LedgerError::ArithmeticOverflow)?;
        for ((owner, _), position) in &self.positions {
            if *owner == participant {
                total = total
                    .checked_add(position.unrealized_pnl)
                    .and_then(|value| value.checked_add(position.realized_pnl))
                    .ok_or(LedgerError::ArithmeticOverflow)?;
            }
        }
        Ok(total)
    }

    /// Returns the canonically ordered transaction journal.
    #[must_use]
    pub fn journal(&self) -> &[JournalTransaction] {
        &self.journal
    }
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

    #[test]
    fn portfolio_transactions_balance_and_failed_postings_roll_back() {
        let participant = ParticipantId::new(1);
        let currency = CurrencyId::new(1);
        let mut ledger = PortfolioLedger::new();
        let valid = JournalTransaction {
            transaction_id: 1,
            kind: TransactionKind::Dividend,
            postings: vec![
                JournalPosting {
                    participant_id: Some(participant),
                    currency_id: currency,
                    account: PostingAccount::Cash,
                    amount: MoneyMinor::new(25),
                },
                JournalPosting {
                    participant_id: None,
                    currency_id: currency,
                    account: PostingAccount::Clearing,
                    amount: MoneyMinor::new(-25),
                },
            ],
        };
        ledger.post(valid).unwrap();
        assert_eq!(
            ledger.balance(participant, currency).settled,
            MoneyMinor::new(25)
        );
        let before = ledger.clone();
        let invalid = JournalTransaction {
            transaction_id: 2,
            kind: TransactionKind::Adjustment,
            postings: vec![JournalPosting {
                participant_id: Some(participant),
                currency_id: currency,
                account: PostingAccount::Cash,
                amount: MoneyMinor::new(1),
            }],
        };
        assert_eq!(
            ledger.post(invalid),
            Err(LedgerError::UnbalancedTransaction)
        );
        assert_eq!(ledger, before);
    }
}
