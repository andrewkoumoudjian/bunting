//! Implements the authorized NBC limit-order matching slice.

use bunting_market_events::Side;
use bunting_market_types::{PriceTicks, QuantityLots};
use core::fmt;
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, VecDeque};

/// Maximum resting orders allowed for one participant by the selected JAR.
pub const MAX_OPEN_ORDERS: usize = 50;
/// Quantity multiple enforced by the selected JAR's external order handler.
pub const NBC_EXTERNAL_LOT_SIZE: i64 = 100;

const MAX_ID_BYTES: usize = 256;

/// Describes one resting NBC order without exposing internal queue state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenOrder {
    order_id: String,
    participant_id: String,
    side: Side,
    price: PriceTicks,
    remaining: QuantityLots,
}

impl OpenOrder {
    /// Returns the client order identifier.
    #[must_use]
    pub fn order_id(&self) -> &str {
        &self.order_id
    }
    /// Returns the participant identifier.
    #[must_use]
    pub fn participant_id(&self) -> &str {
        &self.participant_id
    }
    /// Returns the order side.
    #[must_use]
    pub const fn side(&self) -> Side {
        self.side
    }
    /// Returns the limit price in exact Bunting ticks.
    #[must_use]
    pub const fn price(&self) -> PriceTicks {
        self.price
    }
    /// Returns the unfilled quantity in external NBC units.
    #[must_use]
    pub const fn remaining(&self) -> QuantityLots {
        self.remaining
    }
}

#[derive(Clone, Debug)]
struct RestingOrder {
    public: OpenOrder,
    self_trade_allowed: bool,
    insertion_id: u64,
}

/// Reports one participant-side NBC fill.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fill {
    order_id: String,
    counterparty_order_id: String,
    participant_id: String,
    side: Side,
    price: PriceTicks,
    quantity: QuantityLots,
    remaining: QuantityLots,
    maker: bool,
}

impl Fill {
    /// Returns the filled order identifier.
    #[must_use]
    pub fn order_id(&self) -> &str {
        &self.order_id
    }
    /// Returns the counterparty order identifier.
    #[must_use]
    pub fn counterparty_order_id(&self) -> &str {
        &self.counterparty_order_id
    }
    /// Returns the participant receiving this report.
    #[must_use]
    pub fn participant_id(&self) -> &str {
        &self.participant_id
    }
    /// Returns the filled order side.
    #[must_use]
    pub const fn side(&self) -> Side {
        self.side
    }
    /// Returns the resting-price execution price.
    #[must_use]
    pub const fn price(&self) -> PriceTicks {
        self.price
    }
    /// Returns the executed quantity.
    #[must_use]
    pub const fn quantity(&self) -> QuantityLots {
        self.quantity
    }
    /// Returns this order's remaining quantity after the fill.
    #[must_use]
    pub const fn remaining(&self) -> QuantityLots {
        self.remaining
    }
    /// Reports whether this is the resting-order fill record.
    #[must_use]
    pub const fn is_maker(&self) -> bool {
        self.maker
    }
}

/// Reports the deterministic result of one accepted submission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatchOutcome {
    fills: Vec<Fill>,
    resting: Option<OpenOrder>,
}

impl MatchOutcome {
    /// Returns fill records in taker-then-maker order for every match.
    #[must_use]
    pub fn fills(&self) -> &[Fill] {
        &self.fills
    }
    /// Returns the submitted order when its remainder rested.
    #[must_use]
    pub const fn resting(&self) -> Option<&OpenOrder> {
        self.resting.as_ref()
    }
}

/// Reports whether cancel-by-ID removed a resting order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CancelOutcome {
    Canceled(OpenOrder),
    NotFound,
}

/// Reports a bounded NBC matching failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatchError {
    InvalidOrderId,
    InvalidParticipantId,
    InvalidPrice,
    InvalidQuantity,
    OrderLimitExceeded,
    SelfMatch,
    ArithmeticOverflow,
}

impl fmt::Display for MatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOrderId => f.write_str("order ID must contain 1 to 256 bytes"),
            Self::InvalidParticipantId => f.write_str("participant ID must contain 1 to 256 bytes"),
            Self::InvalidPrice => f.write_str("price must be positive"),
            Self::InvalidQuantity => f.write_str("quantity must be positive and a multiple of 100"),
            Self::OrderLimitExceeded => f.write_str("Rate Limit Exceeded: Max 50 open orders."),
            Self::SelfMatch => {
                f.write_str("Self-match prevention: You cannot match against your own order.")
            }
            Self::ArithmeticOverflow => f.write_str("matching arithmetic overflowed"),
        }
    }
}
impl std::error::Error for MatchError {}

/// Owns NBC-specific price-time queues and participant open-order counts.
///
/// This preserves the selected JAR's level-head self-match check, paired fill reports, and
/// pre-match order limit. Checked integer prices and quantities are Bunting-added.
#[derive(Debug, Default)]
pub struct NbcOrderBook {
    bids: BTreeMap<Reverse<PriceTicks>, VecDeque<RestingOrder>>,
    asks: BTreeMap<PriceTicks, VecDeque<RestingOrder>>,
    order_locations: HashMap<String, u64>,
    open_order_counts: HashMap<String, usize>,
    next_insertion_id: u64,
}

impl NbcOrderBook {
    /// Creates an empty NBC order book.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a participant's current resting-order count.
    #[must_use]
    pub fn open_order_count(&self, participant_id: &str) -> usize {
        self.open_order_counts
            .get(participant_id)
            .copied()
            .unwrap_or_default()
    }

    /// Submits one limit order through the bytecode-observed NBC policy.
    ///
    /// A self-match error can follow earlier matches because the selected JAR mutates each prior
    /// level before checking the next level.
    ///
    /// # Errors
    /// Returns an error for invalid checked units, the 50-order limit, self-match, or overflow.
    pub fn submit_limit(
        &mut self,
        order_id: impl Into<String>,
        participant_id: impl Into<String>,
        side: Side,
        price: PriceTicks,
        quantity: QuantityLots,
        self_trade_allowed: bool,
    ) -> Result<MatchOutcome, MatchError> {
        let order_id = order_id.into();
        let participant_id = participant_id.into();
        validate_text(&order_id, MatchError::InvalidOrderId)?;
        validate_text(&participant_id, MatchError::InvalidParticipantId)?;
        if price.get() <= 0 {
            return Err(MatchError::InvalidPrice);
        }
        if quantity.get() <= 0 || quantity.get() % NBC_EXTERNAL_LOT_SIZE != 0 {
            return Err(MatchError::InvalidQuantity);
        }
        if self.open_order_count(&participant_id) >= MAX_OPEN_ORDERS {
            return Err(MatchError::OrderLimitExceeded);
        }
        let insertion_id = self.next_insertion_id;
        self.next_insertion_id = self
            .next_insertion_id
            .checked_add(1)
            .ok_or(MatchError::ArithmeticOverflow)?;
        let mut incoming = RestingOrder {
            public: OpenOrder {
                order_id,
                participant_id,
                side,
                price,
                remaining: quantity,
            },
            self_trade_allowed,
            insertion_id,
        };
        let mut fills = Vec::new();
        match side {
            Side::Buy => match_asks(
                &mut self.asks,
                &mut self.order_locations,
                &mut self.open_order_counts,
                &mut incoming,
                &mut fills,
            )?,
            Side::Sell => match_bids(
                &mut self.bids,
                &mut self.order_locations,
                &mut self.open_order_counts,
                &mut incoming,
                &mut fills,
            )?,
        }
        let resting = if incoming.public.remaining.get() > 0 {
            let public = incoming.public.clone();
            self.order_locations
                .insert(public.order_id.clone(), insertion_id);
            increment_count(&mut self.open_order_counts, &public.participant_id)?;
            match side {
                Side::Buy => self
                    .bids
                    .entry(Reverse(price))
                    .or_default()
                    .push_back(incoming),
                Side::Sell => self.asks.entry(price).or_default().push_back(incoming),
            }
            Some(public)
        } else {
            None
        };
        Ok(MatchOutcome { fills, resting })
    }

    /// Cancels the order currently indexed by `order_id`.
    ///
    /// The selected JAR does not check participant ownership and silently ignores unknown IDs.
    #[must_use]
    pub fn cancel(&mut self, order_id: &str) -> CancelOutcome {
        let Some(insertion_id) = self.order_locations.remove(order_id) else {
            return CancelOutcome::NotFound;
        };
        if let Some(order) = remove_order(&mut self.bids, insertion_id) {
            decrement_count(&mut self.open_order_counts, &order.public.participant_id);
            return CancelOutcome::Canceled(order.public);
        }
        if let Some(order) = remove_order(&mut self.asks, insertion_id) {
            decrement_count(&mut self.open_order_counts, &order.public.participant_id);
            return CancelOutcome::Canceled(order.public);
        }
        CancelOutcome::NotFound
    }
}

fn match_asks(
    levels: &mut BTreeMap<PriceTicks, VecDeque<RestingOrder>>,
    locations: &mut HashMap<String, u64>,
    counts: &mut HashMap<String, usize>,
    incoming: &mut RestingOrder,
    fills: &mut Vec<Fill>,
) -> Result<(), MatchError> {
    loop {
        let Some(price) = levels.first_key_value().map(|(price, _)| *price) else {
            return Ok(());
        };
        if price > incoming.public.price || incoming.public.remaining.get() == 0 {
            return Ok(());
        }
        match_level(levels, price, locations, counts, incoming, fills)?;
    }
}

fn match_bids(
    levels: &mut BTreeMap<Reverse<PriceTicks>, VecDeque<RestingOrder>>,
    locations: &mut HashMap<String, u64>,
    counts: &mut HashMap<String, usize>,
    incoming: &mut RestingOrder,
    fills: &mut Vec<Fill>,
) -> Result<(), MatchError> {
    loop {
        let Some(key) = levels.first_key_value().map(|(price, _)| *price) else {
            return Ok(());
        };
        if key.0 < incoming.public.price || incoming.public.remaining.get() == 0 {
            return Ok(());
        }
        match_level(levels, key, locations, counts, incoming, fills)?;
    }
}

fn match_level<K: Copy + Ord>(
    levels: &mut BTreeMap<K, VecDeque<RestingOrder>>,
    key: K,
    locations: &mut HashMap<String, u64>,
    counts: &mut HashMap<String, usize>,
    incoming: &mut RestingOrder,
    fills: &mut Vec<Fill>,
) -> Result<(), MatchError> {
    let Some(queue) = levels.get_mut(&key) else {
        return Ok(());
    };
    if let Some(maker) = queue.front()
        && maker.public.participant_id == incoming.public.participant_id
        && !incoming.self_trade_allowed
        && !maker.self_trade_allowed
    {
        return Err(MatchError::SelfMatch);
    }
    while incoming.public.remaining.get() > 0 {
        let Some(maker) = queue.front_mut() else {
            break;
        };
        let executed = incoming
            .public
            .remaining
            .get()
            .min(maker.public.remaining.get());
        incoming.public.remaining = QuantityLots::new(
            incoming
                .public
                .remaining
                .get()
                .checked_sub(executed)
                .ok_or(MatchError::ArithmeticOverflow)?,
        );
        maker.public.remaining = QuantityLots::new(
            maker
                .public
                .remaining
                .get()
                .checked_sub(executed)
                .ok_or(MatchError::ArithmeticOverflow)?,
        );
        fills.push(fill_for(incoming, maker, executed, false));
        fills.push(fill_for(maker, incoming, executed, true));
        if maker.public.remaining.get() == 0 {
            let Some(maker) = queue.pop_front() else {
                break;
            };
            locations.remove(&maker.public.order_id);
            decrement_count(counts, &maker.public.participant_id);
        }
    }
    if queue.is_empty() {
        levels.remove(&key);
    }
    Ok(())
}

fn fill_for(order: &RestingOrder, counterparty: &RestingOrder, quantity: i64, maker: bool) -> Fill {
    Fill {
        order_id: order.public.order_id.clone(),
        counterparty_order_id: counterparty.public.order_id.clone(),
        participant_id: order.public.participant_id.clone(),
        side: order.public.side,
        price: if maker {
            order.public.price
        } else {
            counterparty.public.price
        },
        quantity: QuantityLots::new(quantity),
        remaining: order.public.remaining,
        maker,
    }
}

fn increment_count(
    counts: &mut HashMap<String, usize>,
    participant_id: &str,
) -> Result<(), MatchError> {
    let count = counts.entry(participant_id.to_owned()).or_default();
    *count = count.checked_add(1).ok_or(MatchError::ArithmeticOverflow)?;
    Ok(())
}
fn decrement_count(counts: &mut HashMap<String, usize>, participant_id: &str) {
    if let Some(count) = counts.get_mut(participant_id)
        && *count > 0
    {
        *count -= 1;
    }
}
fn remove_order<K: Copy + Ord>(
    levels: &mut BTreeMap<K, VecDeque<RestingOrder>>,
    insertion_id: u64,
) -> Option<RestingOrder> {
    let key = levels.iter().find_map(|(key, queue)| {
        queue
            .iter()
            .any(|order| order.insertion_id == insertion_id)
            .then_some(*key)
    })?;
    let queue = levels.get_mut(&key)?;
    let position = queue
        .iter()
        .position(|order| order.insertion_id == insertion_id)?;
    let removed = queue.remove(position);
    if queue.is_empty() {
        levels.remove(&key);
    }
    removed
}
fn validate_text(value: &str, error: MatchError) -> Result<(), MatchError> {
    if value.is_empty() || value.len() > MAX_ID_BYTES {
        return Err(error);
    }
    Ok(())
}

// Rust guideline compliant 2026-02-21
