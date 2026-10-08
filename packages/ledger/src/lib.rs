#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! The single authoritative economic ledger of one Bunting run.
//!
//! Every economic fact — order reservations, matched and house fills, fees,
//! cashflows, fines and physical position adjustments — mutates this one
//! structure. Risk admission, participant reporting and scoring are read-only
//! projections of it; there is no second mutable account model.
//!
//! Units: `PriceTicks` are minor currency units per lot of an instrument with
//! multiplier 1. Cash moved by a fill is `price * quantity * multiplier` in the
//! instrument's settlement currency. Every mutation is staged and checked before
//! any balance is written, so a failed operation leaves the ledger unchanged.

use bunting_market_events::Side;
use bunting_market_types::{
    CurrencyId, InstrumentId, MoneyMinor, ParticipantId, PriceTicks, QuantityLots,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Stable ledger failure. No variant leaves a partial mutation behind.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerError {
    ArithmeticOverflow,
    InvalidRelease,
    InvalidPosting,
    UnknownInstrument,
    InsufficientCash,
    InsufficientInventory,
    MissingFxRate,
}

/// Immutable economic terms of one instrument, shared by all of its listings.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentTerms {
    /// Settlement and quotation currency.
    pub currency: CurrencyId,
    /// Positive contract multiplier applied to price-times-quantity notional.
    pub multiplier: i64,
    /// Whether positions may go below zero without reserved inventory.
    pub shortable: bool,
}

/// Converts balances in one currency into the reporting currency through the
/// committed mark of an instrument priced in the reporting currency.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FxRate {
    pub currency: CurrencyId,
    /// Instrument whose mark is the price of `minor_units_per_lot` minor units
    /// of `currency`, quoted in reporting-currency minor units.
    pub instrument_id: InstrumentId,
    pub minor_units_per_lot: i64,
}

/// Exact per-currency cash state of one participant.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CashBalance {
    /// Owned cash, including amounts reserved by open orders.
    pub balance: MoneyMinor,
    /// Portion of `balance` committed to open buy orders.
    pub reserved: MoneyMinor,
    /// Cumulative net fees paid (negative when rebates exceed fees).
    pub fees: MoneyMinor,
}

impl CashBalance {
    /// Cash not reserved by open orders.
    #[must_use]
    pub fn available(&self) -> Option<MoneyMinor> {
        self.balance.checked_sub(self.reserved)
    }
}

/// Exact position state of one participant in one instrument.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Position {
    /// Signed holding.
    pub quantity: QuantityLots,
    /// Inventory committed to open sell orders of a non-shortable instrument.
    pub reserved: QuantityLots,
    /// Open buy quantity across every listing of the instrument.
    pub open_buy: QuantityLots,
    /// Open sell quantity across every listing of the instrument.
    pub open_sell: QuantityLots,
    /// Signed average cost of the open quantity, in settlement minor units.
    pub cost_basis: MoneyMinor,
    /// Cumulative realized trading P&L, excluding fees.
    pub realized_pnl: MoneyMinor,
    /// Cumulative traded quantity.
    pub traded: QuantityLots,
}

impl Position {
    /// Inventory not reserved by open sell orders.
    #[must_use]
    pub fn available(&self) -> Option<QuantityLots> {
        self.quantity.checked_sub(self.reserved)
    }
}

/// Reservation contract of one open order, released exactly on fill or cancel.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Reservation {
    /// Cash reserved per lot of a buy order (zero when cash is unconstrained).
    pub cash_per_lot: MoneyMinor,
    /// Whether a sell order holds inventory.
    pub inventory: bool,
}

impl Reservation {
    pub const NONE: Self = Self {
        cash_per_lot: MoneyMinor::new(0),
        inventory: false,
    };
}

/// One side of a fill. A `None` party on a [`Fill`] is the venue/house.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FillParty {
    pub participant: ParticipantId,
    /// Total fee charged to this party for the fill; negative is a rebate.
    pub fee: MoneyMinor,
    /// Reservation of the open order being filled, if the party had one.
    pub order: Option<Reservation>,
    /// Rejects the fill if it would leave this buyer with negative available cash.
    pub enforce_funding: bool,
}

/// One exact economic fill.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fill {
    pub instrument: InstrumentId,
    pub price: PriceTicks,
    pub quantity: QuantityLots,
    pub buyer: Option<FillParty>,
    pub seller: Option<FillParty>,
    /// Whether the fill price becomes the instrument's valuation mark. Exchange
    /// fills mark; negotiated off-book fills (tenders, OTC) do not.
    pub set_mark: bool,
}

/// Authoritative cash, position, reservation, fee, mark and P&L state.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    #[serde(with = "tuple_map")]
    terms: BTreeMap<InstrumentId, InstrumentTerms>,
    #[serde(with = "tuple_map")]
    cash: BTreeMap<(ParticipantId, CurrencyId), CashBalance>,
    #[serde(with = "tuple_map")]
    positions: BTreeMap<(ParticipantId, InstrumentId), Position>,
    /// Last committed trade price per instrument, across all listings.
    #[serde(with = "tuple_map")]
    marks: BTreeMap<InstrumentId, PriceTicks>,
}

mod tuple_map {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<K, V, S>(value: &BTreeMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        K: Serialize,
        V: Serialize,
        S: Serializer,
    {
        serializer.collect_seq(value.iter())
    }

    pub fn deserialize<'de, K, V, D>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        let mut output = BTreeMap::new();
        for (key, value) in Vec::<(K, V)>::deserialize(deserializer)? {
            if output.insert(key, value).is_some() {
                return Err(serde::de::Error::custom("duplicate ledger key"));
            }
        }
        Ok(output)
    }
}

fn overflow<T>(value: Option<T>) -> Result<T, LedgerError> {
    value.ok_or(LedgerError::ArithmeticOverflow)
}

fn negate(value: MoneyMinor) -> Result<MoneyMinor, LedgerError> {
    overflow(value.get().checked_neg().map(MoneyMinor::new))
}

/// Exact `price * quantity * multiplier` notional.
pub fn notional(
    price: PriceTicks,
    quantity: QuantityLots,
    multiplier: i64,
) -> Result<MoneyMinor, LedgerError> {
    overflow(
        i128::from(price.get())
            .checked_mul(i128::from(quantity.get()))
            .and_then(|value| value.checked_mul(i128::from(multiplier)))
            .map(MoneyMinor::new),
    )
}

fn per_lot_total(per_lot: MoneyMinor, quantity: QuantityLots) -> Result<MoneyMinor, LedgerError> {
    overflow(
        per_lot
            .get()
            .checked_mul(i128::from(quantity.get()))
            .map(MoneyMinor::new),
    )
}

impl Ledger {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers immutable instrument terms at run creation.
    pub fn configure_instrument(
        &mut self,
        instrument: InstrumentId,
        terms: InstrumentTerms,
    ) -> Result<(), LedgerError> {
        if instrument.get() == 0 || terms.currency.get() == 0 || terms.multiplier <= 0 {
            return Err(LedgerError::InvalidPosting);
        }
        self.terms.insert(instrument, terms);
        Ok(())
    }

    pub fn terms(&self, instrument: InstrumentId) -> Result<InstrumentTerms, LedgerError> {
        self.terms
            .get(&instrument)
            .copied()
            .ok_or(LedgerError::UnknownInstrument)
    }

    #[must_use]
    pub fn instruments(&self) -> &BTreeMap<InstrumentId, InstrumentTerms> {
        &self.terms
    }

    #[must_use]
    pub fn cash(&self, participant: ParticipantId, currency: CurrencyId) -> CashBalance {
        self.cash
            .get(&(participant, currency))
            .copied()
            .unwrap_or_default()
    }

    /// Every currency balance of one participant in currency order.
    pub fn cash_balances(
        &self,
        participant: ParticipantId,
    ) -> impl Iterator<Item = (CurrencyId, CashBalance)> + '_ {
        self.cash
            .range((participant, CurrencyId::new(0))..=(participant, CurrencyId::new(u128::MAX)))
            .map(|((_, currency), balance)| (*currency, *balance))
    }

    #[must_use]
    pub fn position(&self, participant: ParticipantId, instrument: InstrumentId) -> Position {
        self.positions
            .get(&(participant, instrument))
            .copied()
            .unwrap_or_default()
    }

    /// Every position of one participant in instrument order.
    pub fn positions(
        &self,
        participant: ParticipantId,
    ) -> impl Iterator<Item = (InstrumentId, Position)> + '_ {
        self.positions
            .range(
                (participant, InstrumentId::new(0))..=(participant, InstrumentId::new(u128::MAX)),
            )
            .map(|((_, instrument), position)| (*instrument, *position))
    }

    #[must_use]
    pub fn mark(&self, instrument: InstrumentId) -> Option<PriceTicks> {
        self.marks.get(&instrument).copied()
    }

    /// Sets the committed valuation mark of an instrument.
    pub fn set_mark(
        &mut self,
        instrument: InstrumentId,
        mark: PriceTicks,
    ) -> Result<(), LedgerError> {
        self.terms(instrument)?;
        if mark.get() <= 0 {
            return Err(LedgerError::InvalidPosting);
        }
        self.marks.insert(instrument, mark);
        Ok(())
    }

    /// Posts a signed external cashflow (endowment, fine, interest, transfer).
    pub fn post_cash(
        &mut self,
        participant: ParticipantId,
        currency: CurrencyId,
        amount: MoneyMinor,
    ) -> Result<(), LedgerError> {
        if currency.get() == 0 {
            return Err(LedgerError::InvalidPosting);
        }
        let entry = self.cash(participant, currency);
        let balance = overflow(entry.balance.checked_add(amount))?;
        self.cash
            .insert((participant, currency), CashBalance { balance, ..entry });
        Ok(())
    }

    /// Seeds an opening position with an explicit signed cost basis.
    pub fn seed_position(
        &mut self,
        participant: ParticipantId,
        instrument: InstrumentId,
        quantity: QuantityLots,
        cost_basis: MoneyMinor,
    ) -> Result<(), LedgerError> {
        self.terms(instrument)?;
        let entry = self.position(participant, instrument);
        self.positions.insert(
            (participant, instrument),
            Position {
                quantity,
                cost_basis,
                ..entry
            },
        );
        Ok(())
    }

    /// Cash reserved per lot for a buy at `price` with the given fee bound.
    pub fn cash_per_lot(
        &self,
        instrument: InstrumentId,
        price: PriceTicks,
        fee_per_lot: MoneyMinor,
    ) -> Result<MoneyMinor, LedgerError> {
        let terms = self.terms(instrument)?;
        let lot = notional(price, QuantityLots::new(1), terms.multiplier)?;
        overflow(lot.checked_add(MoneyMinor::new(fee_per_lot.get().max(0))))
    }

    /// Admits an open order: reserves its cash or inventory and records open quantity.
    pub fn open_order(
        &mut self,
        participant: ParticipantId,
        instrument: InstrumentId,
        side: Side,
        quantity: QuantityLots,
        reservation: Reservation,
    ) -> Result<(), LedgerError> {
        self.change_open(participant, instrument, side, quantity, reservation, true)
    }

    /// Releases the unfilled remainder of an order on cancel or expiry.
    pub fn close_order(
        &mut self,
        participant: ParticipantId,
        instrument: InstrumentId,
        side: Side,
        quantity: QuantityLots,
        reservation: Reservation,
    ) -> Result<(), LedgerError> {
        self.change_open(participant, instrument, side, quantity, reservation, false)
    }

    fn change_open(
        &mut self,
        participant: ParticipantId,
        instrument: InstrumentId,
        side: Side,
        quantity: QuantityLots,
        reservation: Reservation,
        open: bool,
    ) -> Result<(), LedgerError> {
        let terms = self.terms(instrument)?;
        if quantity.get() <= 0 {
            return Err(LedgerError::InvalidPosting);
        }
        let mut position = self.position(participant, instrument);
        let mut cash = self.cash(participant, terms.currency);
        let signed = |value: QuantityLots| {
            if open {
                Some(value)
            } else {
                QuantityLots::new(0).checked_sub(value)
            }
        };
        let delta = overflow(signed(quantity))?;
        match side {
            Side::Buy => {
                position.open_buy = overflow(position.open_buy.checked_add(delta))?;
                let amount = per_lot_total(reservation.cash_per_lot, quantity)?;
                cash.reserved = if open {
                    overflow(cash.reserved.checked_add(amount))?
                } else {
                    overflow(cash.reserved.checked_sub(amount))?
                };
                if cash.reserved.get() < 0 {
                    return Err(LedgerError::InvalidRelease);
                }
                if open && overflow(cash.available())?.get() < 0 {
                    return Err(LedgerError::InsufficientCash);
                }
            }
            Side::Sell => {
                position.open_sell = overflow(position.open_sell.checked_add(delta))?;
                if reservation.inventory {
                    position.reserved = overflow(position.reserved.checked_add(delta))?;
                    if position.reserved.get() < 0 {
                        return Err(LedgerError::InvalidRelease);
                    }
                    if open && overflow(position.available())?.get() < 0 {
                        return Err(LedgerError::InsufficientInventory);
                    }
                }
            }
        }
        if position.open_buy.get() < 0 || position.open_sell.get() < 0 {
            return Err(LedgerError::InvalidRelease);
        }
        self.positions.insert((participant, instrument), position);
        self.cash.insert((participant, terms.currency), cash);
        Ok(())
    }

    /// Settles one fill atomically and records its price as the instrument mark.
    ///
    /// Releases the filled portion of each party's open-order reservation, moves
    /// notional cash and fees, and updates signed positions with average-cost
    /// realization. A house party (`None`) is the clearing counterparty.
    pub fn settle(&mut self, fill: Fill) -> Result<(), LedgerError> {
        let terms = self.terms(fill.instrument)?;
        if fill.quantity.get() <= 0 || fill.price.get() <= 0 {
            return Err(LedgerError::InvalidPosting);
        }
        let notional = notional(fill.price, fill.quantity, terms.multiplier)?;
        let mut cash = BTreeMap::<ParticipantId, CashBalance>::new();
        let mut positions = BTreeMap::<ParticipantId, Position>::new();
        for (party, side) in [(fill.buyer, Side::Buy), (fill.seller, Side::Sell)] {
            let Some(party) = party else {
                continue;
            };
            let participant = party.participant;
            let mut balance = cash
                .get(&participant)
                .copied()
                .unwrap_or_else(|| self.cash(participant, terms.currency));
            let mut position = positions
                .get(&participant)
                .copied()
                .unwrap_or_else(|| self.position(participant, fill.instrument));
            if let Some(order) = party.order {
                match side {
                    Side::Buy => {
                        position.open_buy = overflow(position.open_buy.checked_sub(fill.quantity))?;
                        balance.reserved = overflow(
                            balance
                                .reserved
                                .checked_sub(per_lot_total(order.cash_per_lot, fill.quantity)?),
                        )?;
                    }
                    Side::Sell => {
                        position.open_sell =
                            overflow(position.open_sell.checked_sub(fill.quantity))?;
                        if order.inventory {
                            position.reserved =
                                overflow(position.reserved.checked_sub(fill.quantity))?;
                        }
                    }
                }
                if position.open_buy.get() < 0
                    || position.open_sell.get() < 0
                    || position.reserved.get() < 0
                    || balance.reserved.get() < 0
                {
                    return Err(LedgerError::InvalidRelease);
                }
            }
            let (flow, delta) = match side {
                Side::Buy => (negate(notional)?, fill.quantity),
                Side::Sell => (
                    notional,
                    overflow(QuantityLots::new(0).checked_sub(fill.quantity))?,
                ),
            };
            balance.balance = overflow(
                balance
                    .balance
                    .checked_add(flow)
                    .and_then(|value| value.checked_sub(party.fee)),
            )?;
            balance.fees = overflow(balance.fees.checked_add(party.fee))?;
            position = apply_fill(position, delta, fill.price, terms.multiplier)?;
            position.traded = overflow(position.traded.checked_add(fill.quantity))?;
            if side == Side::Buy
                && party.enforce_funding
                && overflow(balance.available())?.get() < 0
            {
                return Err(LedgerError::InsufficientCash);
            }
            if side == Side::Sell && !terms.shortable && overflow(position.available())?.get() < 0 {
                return Err(LedgerError::InsufficientInventory);
            }
            cash.insert(participant, balance);
            positions.insert(participant, position);
        }
        for (participant, balance) in cash {
            self.cash.insert((participant, terms.currency), balance);
        }
        for (participant, position) in positions {
            self.positions
                .insert((participant, fill.instrument), position);
        }
        if fill.set_mark {
            self.marks.insert(fill.instrument, fill.price);
        }
        Ok(())
    }

    /// Converts `input` units of one instrument into `output` units of another
    /// without cash, transferring the removed cost basis to the output holding.
    pub fn convert(
        &mut self,
        participant: ParticipantId,
        input: Option<(InstrumentId, QuantityLots)>,
        output: Option<(InstrumentId, QuantityLots)>,
    ) -> Result<(), LedgerError> {
        let mut transferred = MoneyMinor::new(0);
        let mut staged = Vec::new();
        if let Some((instrument, quantity)) = input {
            self.terms(instrument)?;
            let mut position = self.position(participant, instrument);
            if quantity.get() <= 0 || overflow(position.available())? < quantity {
                return Err(LedgerError::InsufficientInventory);
            }
            if position.quantity.get() > 0 {
                transferred = MoneyMinor::new(overflow(
                    position
                        .cost_basis
                        .get()
                        .checked_mul(i128::from(quantity.get()))
                        .and_then(|value| value.checked_div(i128::from(position.quantity.get()))),
                )?);
            }
            position.cost_basis = overflow(position.cost_basis.checked_sub(transferred))?;
            position.quantity = overflow(position.quantity.checked_sub(quantity))?;
            staged.push((instrument, position));
        }
        if let Some((instrument, quantity)) = output {
            self.terms(instrument)?;
            if quantity.get() <= 0 {
                return Err(LedgerError::InvalidPosting);
            }
            let mut position = staged.iter().find(|(id, _)| *id == instrument).map_or_else(
                || self.position(participant, instrument),
                |(_, value)| *value,
            );
            position.quantity = overflow(position.quantity.checked_add(quantity))?;
            position.cost_basis = overflow(position.cost_basis.checked_add(transferred))?;
            staged.retain(|(id, _)| *id != instrument);
            staged.push((instrument, position));
        }
        for (instrument, position) in staged {
            self.positions.insert((participant, instrument), position);
        }
        Ok(())
    }

    /// Marked value of one position in its settlement currency. An instrument
    /// without a committed mark is carried at cost.
    pub fn position_value(
        &self,
        instrument: InstrumentId,
        position: &Position,
    ) -> Result<MoneyMinor, LedgerError> {
        let terms = self.terms(instrument)?;
        self.mark(instrument)
            .map_or(Ok(position.cost_basis), |mark| {
                notional(mark, position.quantity, terms.multiplier)
            })
    }

    /// Unrealized P&L of one position at its committed mark.
    pub fn unrealized_pnl(
        &self,
        participant: ParticipantId,
        instrument: InstrumentId,
    ) -> Result<MoneyMinor, LedgerError> {
        let position = self.position(participant, instrument);
        overflow(
            self.position_value(instrument, &position)?
                .checked_sub(position.cost_basis),
        )
    }

    /// Net liquidation value of one participant in one currency: cash plus the
    /// marked value of positions settling in that currency.
    pub fn net_liquidation_value(
        &self,
        participant: ParticipantId,
        currency: CurrencyId,
    ) -> Result<MoneyMinor, LedgerError> {
        let mut total = self.cash(participant, currency).balance;
        for (instrument, position) in self.positions(participant) {
            if self.terms(instrument)?.currency == currency {
                total = overflow(total.checked_add(self.position_value(instrument, &position)?))?;
            }
        }
        Ok(total)
    }

    /// Net liquidation value converted into one reporting currency.
    ///
    /// Foreign amounts convert through the committed FX instrument mark; integer
    /// division truncates toward zero. A held currency without a rate fails closed.
    pub fn reporting_value(
        &self,
        participant: ParticipantId,
        reporting: CurrencyId,
        rates: &[FxRate],
    ) -> Result<MoneyMinor, LedgerError> {
        let mut currencies = self
            .cash_balances(participant)
            .map(|(currency, _)| currency)
            .collect::<Vec<_>>();
        for (instrument, _) in self.positions(participant) {
            currencies.push(self.terms(instrument)?.currency);
        }
        currencies.sort_unstable();
        currencies.dedup();
        let mut total = MoneyMinor::new(0);
        for currency in currencies {
            let value = self.net_liquidation_value(participant, currency)?;
            let converted = if currency == reporting {
                value
            } else if value.get() == 0 {
                MoneyMinor::new(0)
            } else {
                let rate = rates
                    .iter()
                    .find(|rate| rate.currency == currency)
                    .ok_or(LedgerError::MissingFxRate)?;
                let mark = self
                    .mark(rate.instrument_id)
                    .ok_or(LedgerError::MissingFxRate)?;
                if rate.minor_units_per_lot <= 0 {
                    return Err(LedgerError::MissingFxRate);
                }
                MoneyMinor::new(overflow(
                    value
                        .get()
                        .checked_mul(i128::from(mark.get()))
                        .and_then(|amount| {
                            amount.checked_div(i128::from(rate.minor_units_per_lot))
                        }),
                )?)
            };
            total = overflow(total.checked_add(converted))?;
        }
        Ok(total)
    }

    /// Participants with any recorded balance, in identity order.
    #[must_use]
    pub fn participants(&self) -> Vec<ParticipantId> {
        let mut participants = self
            .cash
            .keys()
            .map(|(participant, _)| *participant)
            .chain(self.positions.keys().map(|(participant, _)| *participant))
            .collect::<Vec<_>>();
        participants.dedup();
        participants.sort_unstable();
        participants.dedup();
        participants
    }
}

/// Applies a signed execution delta with proportional average-cost realization.
///
/// Integer divisions round toward zero; the remaining basis keeps the exact
/// unallocated difference so no value disappears across partial closes.
fn apply_fill(
    mut position: Position,
    delta: QuantityLots,
    price: PriceTicks,
    multiplier: i64,
) -> Result<Position, LedgerError> {
    let old_qty = position.quantity.get();
    let change = delta.get();
    let next_qty = overflow(position.quantity.checked_add(delta))?;
    let opposite = (old_qty > 0 && change < 0) || (old_qty < 0 && change > 0);
    let closed = if opposite {
        old_qty.unsigned_abs().min(change.unsigned_abs())
    } else {
        0
    };
    let closed_delta = if old_qty > 0 {
        -i128::from(closed)
    } else {
        i128::from(closed)
    };
    let unit = overflow(i128::from(price.get()).checked_mul(i128::from(multiplier)))?;
    let old_basis = position.cost_basis.get();
    let removed_basis = if closed == 0 {
        0
    } else {
        overflow(old_basis.checked_mul(i128::from(closed)))? / i128::from(old_qty.unsigned_abs())
    };
    let realized = overflow(
        closed_delta
            .checked_mul(unit)
            .and_then(i128::checked_neg)
            .and_then(|flow| flow.checked_sub(removed_basis)),
    )?;
    let opened_delta = overflow(i128::from(change).checked_sub(closed_delta))?;
    let opened_basis = overflow(opened_delta.checked_mul(unit))?;
    let new_basis = overflow(
        old_basis
            .checked_sub(removed_basis)
            .and_then(|value| value.checked_add(opened_basis)),
    )?;
    position.quantity = next_qty;
    position.cost_basis = MoneyMinor::new(new_basis);
    position.realized_pnl = overflow(position.realized_pnl.checked_add(MoneyMinor::new(realized)))?;
    Ok(position)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const CAD: CurrencyId = CurrencyId::new(1);
    const USD: CurrencyId = CurrencyId::new(2);
    const STOCK: InstrumentId = InstrumentId::new(10);
    const BOND: InstrumentId = InstrumentId::new(11);
    const FX: InstrumentId = InstrumentId::new(12);
    const ALICE: ParticipantId = ParticipantId::new(1);
    const BOB: ParticipantId = ParticipantId::new(2);

    fn ledger() -> Ledger {
        let mut ledger = Ledger::new();
        ledger
            .configure_instrument(
                STOCK,
                InstrumentTerms {
                    currency: CAD,
                    multiplier: 1,
                    shortable: false,
                },
            )
            .unwrap();
        ledger
            .configure_instrument(
                BOND,
                InstrumentTerms {
                    currency: USD,
                    multiplier: 10,
                    shortable: true,
                },
            )
            .unwrap();
        ledger
            .configure_instrument(
                FX,
                InstrumentTerms {
                    currency: CAD,
                    multiplier: 1,
                    shortable: true,
                },
            )
            .unwrap();
        ledger
            .post_cash(ALICE, CAD, MoneyMinor::new(10_000))
            .unwrap();
        ledger.post_cash(BOB, CAD, MoneyMinor::new(10_000)).unwrap();
        ledger
            .seed_position(BOB, STOCK, QuantityLots::new(10), MoneyMinor::new(100))
            .unwrap();
        ledger
    }

    fn buy_reservation(ledger: &Ledger, limit: i64, fee: i128) -> Reservation {
        Reservation {
            cash_per_lot: ledger
                .cash_per_lot(STOCK, PriceTicks::new(limit), MoneyMinor::new(fee))
                .unwrap(),
            inventory: false,
        }
    }

    #[test]
    fn reservations_release_exactly_and_reject_overcommitment() {
        let mut ledger = ledger();
        let reservation = buy_reservation(&ledger, 12, 1);
        ledger
            .open_order(ALICE, STOCK, Side::Buy, QuantityLots::new(4), reservation)
            .unwrap();
        assert_eq!(
            ledger.cash(ALICE, CAD).available(),
            Some(MoneyMinor::new(10_000 - 52))
        );
        assert_eq!(ledger.position(ALICE, STOCK).open_buy, QuantityLots::new(4));
        let before = ledger.clone();
        assert_eq!(
            ledger.open_order(
                ALICE,
                STOCK,
                Side::Buy,
                QuantityLots::new(1_000),
                reservation
            ),
            Err(LedgerError::InsufficientCash)
        );
        assert_eq!(ledger, before);
        assert_eq!(
            ledger.open_order(
                ALICE,
                STOCK,
                Side::Sell,
                QuantityLots::new(1),
                Reservation {
                    cash_per_lot: MoneyMinor::new(0),
                    inventory: true
                }
            ),
            Err(LedgerError::InsufficientInventory)
        );
        assert_eq!(ledger, before);
        ledger
            .close_order(ALICE, STOCK, Side::Buy, QuantityLots::new(4), reservation)
            .unwrap();
        assert_eq!(ledger.cash(ALICE, CAD).reserved, MoneyMinor::new(0));
        assert_eq!(ledger.position(ALICE, STOCK).open_buy, QuantityLots::new(0));
        assert_eq!(
            ledger.close_order(ALICE, STOCK, Side::Buy, QuantityLots::new(1), reservation),
            Err(LedgerError::InvalidRelease)
        );
    }

    #[test]
    fn fill_conserves_cash_charges_fees_and_realizes_pnl_once() {
        let mut ledger = ledger();
        let buy = buy_reservation(&ledger, 20, 2);
        let sell = Reservation {
            cash_per_lot: MoneyMinor::new(0),
            inventory: true,
        };
        ledger
            .open_order(ALICE, STOCK, Side::Buy, QuantityLots::new(4), buy)
            .unwrap();
        ledger
            .open_order(BOB, STOCK, Side::Sell, QuantityLots::new(4), sell)
            .unwrap();
        ledger
            .settle(Fill {
                instrument: STOCK,
                price: PriceTicks::new(15),
                quantity: QuantityLots::new(4),
                buyer: Some(FillParty {
                    participant: ALICE,
                    fee: MoneyMinor::new(8),
                    order: Some(buy),
                    enforce_funding: true,
                }),
                seller: Some(FillParty {
                    participant: BOB,
                    fee: MoneyMinor::new(-4),
                    order: Some(sell),
                    enforce_funding: true,
                }),
                set_mark: true,
            })
            .unwrap();
        assert_eq!(
            ledger.cash(ALICE, CAD).balance,
            MoneyMinor::new(10_000 - 60 - 8)
        );
        assert_eq!(ledger.cash(ALICE, CAD).reserved, MoneyMinor::new(0));
        assert_eq!(
            ledger.cash(BOB, CAD).balance,
            MoneyMinor::new(10_000 + 60 + 4)
        );
        assert_eq!(ledger.cash(ALICE, CAD).fees, MoneyMinor::new(8));
        let bob = ledger.position(BOB, STOCK);
        assert_eq!(bob.quantity, QuantityLots::new(6));
        assert_eq!(bob.reserved, QuantityLots::new(0));
        assert_eq!(bob.cost_basis, MoneyMinor::new(60));
        assert_eq!(bob.realized_pnl, MoneyMinor::new(20));
        assert_eq!(ledger.mark(STOCK), Some(PriceTicks::new(15)));
        // Seller NLV: cash + 6 * 15; realized P&L is already in cash.
        assert_eq!(
            ledger.net_liquidation_value(BOB, CAD).unwrap(),
            MoneyMinor::new(10_064 + 90)
        );
        let total_before = 20_000 + 100;
        let total_after = ledger.net_liquidation_value(ALICE, CAD).unwrap().get()
            + ledger.net_liquidation_value(BOB, CAD).unwrap().get();
        // Value moves only by fees and the remark of Bob's opening basis (100 -> 150).
        assert_eq!(total_after, total_before - 8 + 4 + 50);
    }

    #[test]
    fn failed_settlement_never_partially_mutates() {
        let mut ledger = ledger();
        let before = ledger.clone();
        let result = ledger.settle(Fill {
            instrument: STOCK,
            price: PriceTicks::new(10),
            quantity: QuantityLots::new(1),
            buyer: Some(FillParty {
                participant: ALICE,
                fee: MoneyMinor::new(0),
                order: Some(buy_reservation(&before, 10, 0)),
                enforce_funding: true,
            }),
            seller: None,
            set_mark: true,
        });
        assert_eq!(result, Err(LedgerError::InvalidRelease));
        assert_eq!(ledger, before);
    }

    #[test]
    fn multiplier_short_positions_and_house_fills_use_one_ledger() {
        let mut ledger = ledger();
        ledger
            .settle(Fill {
                instrument: BOND,
                price: PriceTicks::new(100),
                quantity: QuantityLots::new(3),
                buyer: None,
                seller: Some(FillParty {
                    participant: ALICE,
                    fee: MoneyMinor::new(0),
                    order: None,
                    enforce_funding: false,
                }),
                set_mark: false,
            })
            .unwrap();
        assert_eq!(ledger.position(ALICE, BOND).quantity, QuantityLots::new(-3));
        assert_eq!(ledger.cash(ALICE, USD).balance, MoneyMinor::new(3_000));
        assert_eq!(
            ledger.position(ALICE, BOND).cost_basis,
            MoneyMinor::new(-3_000)
        );
        ledger.set_mark(BOND, PriceTicks::new(90)).unwrap();
        assert_eq!(
            ledger.unrealized_pnl(ALICE, BOND).unwrap(),
            MoneyMinor::new(300)
        );
        assert_eq!(
            ledger.net_liquidation_value(ALICE, USD).unwrap(),
            MoneyMinor::new(300)
        );
        assert_eq!(
            ledger.reporting_value(ALICE, CAD, &[]),
            Err(LedgerError::MissingFxRate)
        );
        ledger.set_mark(FX, PriceTicks::new(135)).unwrap();
        let rates = [FxRate {
            currency: USD,
            instrument_id: FX,
            minor_units_per_lot: 100,
        }];
        assert_eq!(
            ledger.reporting_value(ALICE, CAD, &rates).unwrap(),
            MoneyMinor::new(10_000 + 405)
        );
    }

    #[test]
    fn conversion_transfers_cost_basis_without_cash() {
        let mut ledger = ledger();
        ledger
            .convert(
                BOB,
                Some((STOCK, QuantityLots::new(5))),
                Some((FX, QuantityLots::new(2))),
            )
            .unwrap();
        assert_eq!(ledger.position(BOB, STOCK).quantity, QuantityLots::new(5));
        assert_eq!(ledger.position(BOB, STOCK).cost_basis, MoneyMinor::new(50));
        assert_eq!(ledger.position(BOB, FX).cost_basis, MoneyMinor::new(50));
        assert_eq!(ledger.cash(BOB, CAD).balance, MoneyMinor::new(10_000));
        let before = ledger.clone();
        assert_eq!(
            ledger.convert(BOB, Some((STOCK, QuantityLots::new(50))), None),
            Err(LedgerError::InsufficientInventory)
        );
        assert_eq!(ledger, before);
    }

    #[test]
    fn serialization_round_trips_and_rejects_duplicates() {
        let ledger = ledger();
        let json = serde_json::to_string(&ledger).unwrap();
        assert_eq!(serde_json::from_str::<Ledger>(&json).unwrap(), ledger);
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let cash = value["cash"].as_array_mut().unwrap();
        cash.push(cash[0].clone());
        assert!(serde_json::from_value::<Ledger>(value).is_err());
    }
}
