//! Bunting-owned deterministic price-time limit order book.
//!
//! One book per listing. Orders are keyed by their canonical 128-bit
//! [`OrderId`]; there is no secondary identity space. Price levels are ordered
//! maps and each level is a FIFO keyed by a monotonically increasing priority,
//! so insertion, cancellation and best-price lookup are `O(log n)` and every
//! traversal is deterministic. The book holds no clocks, randomness, interior
//! mutability or shared ownership: cloning it is a plain value copy, which is
//! what makes staged transitions and replay exact.
//!
//! Matching rules:
//! - an aggressing order trades against the best opposite price while its
//!   limit crosses, at the resting order's price;
//! - within a level, the earliest priority trades first;
//! - a display (iceberg) order exposes at most its display quantity; when the
//!   displayed slice is exhausted it is refreshed from the hidden remainder and
//!   loses time priority (moves to the back of its level);
//! - self-matching is permitted; participant-level prevention is a policy above
//!   the book.

use bunting_market_events::Side;
use bunting_market_types::{OrderId, PriceTicks, QuantityLots};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One resting order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BookOrder {
    pub order_id: OrderId,
    pub side: Side,
    pub price: PriceTicks,
    /// Total unfilled quantity, displayed and hidden.
    pub remaining: QuantityLots,
    /// Currently displayed quantity.
    pub visible: QuantityLots,
    /// Display (peak) size of an iceberg order; `None` displays everything.
    pub display: Option<QuantityLots>,
    /// Time priority within the price level; lower trades first.
    pub priority: u64,
}

/// One execution against a resting order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Match {
    pub maker: OrderId,
    pub price: PriceTicks,
    pub quantity: QuantityLots,
    /// Maker's unfilled quantity after this execution.
    pub maker_remaining: QuantityLots,
}

/// Book invariant or input failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BookError {
    DuplicateOrder,
    InvalidOrder,
    Overflow,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Level {
    queue: BTreeMap<u64, OrderId>,
    visible: QuantityLots,
    total: QuantityLots,
}

/// Deterministic price-time book for one listing.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(try_from = "BookRecord", into = "BookRecord")]
pub struct Book {
    bids: BTreeMap<PriceTicks, Level>,
    asks: BTreeMap<PriceTicks, Level>,
    orders: BTreeMap<OrderId, BookOrder>,
    next_priority: u64,
}

/// Canonical persisted form: resting orders in (side, price, priority) order.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BookRecord {
    next_priority: u64,
    orders: Vec<BookOrder>,
}

impl From<Book> for BookRecord {
    fn from(book: Book) -> Self {
        let mut orders = book.orders.into_values().collect::<Vec<_>>();
        orders.sort_by_key(|order| (order.side, order.price, order.priority));
        Self {
            next_priority: book.next_priority,
            orders,
        }
    }
}

impl TryFrom<BookRecord> for Book {
    type Error = String;

    fn try_from(record: BookRecord) -> Result<Self, Self::Error> {
        let mut book = Self {
            next_priority: record.next_priority,
            ..Self::default()
        };
        for order in record.orders {
            if order.priority >= record.next_priority {
                return Err("order priority beyond the book sequence".to_owned());
            }
            book.insert(order).map_err(|error| format!("{error:?}"))?;
        }
        Ok(book)
    }
}

const fn crosses(taker: Side, limit: PriceTicks, resting: PriceTicks) -> bool {
    match taker {
        Side::Buy => resting.get() <= limit.get(),
        Side::Sell => resting.get() >= limit.get(),
    }
}

impl Book {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of resting orders.
    #[must_use]
    pub fn len(&self) -> usize {
        self.orders.len()
    }

    #[cfg(test)]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    #[must_use]
    pub fn contains(&self, order_id: OrderId) -> bool {
        self.orders.contains_key(&order_id)
    }

    fn side(&self, side: Side) -> &BTreeMap<PriceTicks, Level> {
        match side {
            Side::Buy => &self.bids,
            Side::Sell => &self.asks,
        }
    }

    fn side_mut(&mut self, side: Side) -> &mut BTreeMap<PriceTicks, Level> {
        match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        }
    }

    /// Best resting price on one side.
    #[must_use]
    pub fn best(&self, side: Side) -> Option<PriceTicks> {
        match side {
            Side::Buy => self.bids.keys().next_back().copied(),
            Side::Sell => self.asks.keys().next().copied(),
        }
    }

    /// Whether an order on `taker` side at `limit` would trade immediately.
    #[must_use]
    pub fn would_cross(&self, taker: Side, limit: PriceTicks) -> bool {
        self.best(taker.opposite())
            .is_some_and(|best| crosses(taker, limit, best))
    }

    /// Total quantity, displayed and hidden, executable for an aggressor.
    #[must_use]
    pub fn executable(&self, taker: Side, limit: Option<PriceTicks>) -> QuantityLots {
        let levels: Box<dyn Iterator<Item = (&PriceTicks, &Level)>> = match taker {
            Side::Buy => Box::new(self.asks.iter()),
            Side::Sell => Box::new(self.bids.iter().rev()),
        };
        levels
            .take_while(|(price, _)| limit.is_none_or(|limit| crosses(taker, limit, **price)))
            .fold(QuantityLots::new(0), |total, (_, level)| {
                total
                    .checked_add(level.total)
                    .unwrap_or(QuantityLots::new(i64::MAX))
            })
    }

    /// Executes an aggressor against the opposite side and returns the matches
    /// and the unfilled remainder. Nothing rests; callers decide what to do with
    /// the remainder.
    pub fn execute(
        &mut self,
        taker: Side,
        limit: Option<PriceTicks>,
        quantity: QuantityLots,
    ) -> Result<(Vec<Match>, QuantityLots), BookError> {
        if quantity.get() <= 0 {
            return Err(BookError::InvalidOrder);
        }
        let maker_side = taker.opposite();
        let mut remaining = quantity;
        let mut matches = Vec::new();
        while remaining.get() > 0 {
            let Some(price) = self.best(maker_side) else {
                break;
            };
            if limit.is_some_and(|limit| !crosses(taker, limit, price)) {
                break;
            }
            let maker_id = *self
                .side(maker_side)
                .get(&price)
                .and_then(|level| level.queue.values().next())
                .ok_or(BookError::InvalidOrder)?;
            let mut maker = *self.orders.get(&maker_id).ok_or(BookError::InvalidOrder)?;
            let fill = QuantityLots::new(remaining.get().min(maker.visible.get()));
            if fill.get() <= 0 {
                return Err(BookError::InvalidOrder);
            }
            remaining = remaining.checked_sub(fill).ok_or(BookError::Overflow)?;
            maker.remaining = maker
                .remaining
                .checked_sub(fill)
                .ok_or(BookError::Overflow)?;
            maker.visible = maker.visible.checked_sub(fill).ok_or(BookError::Overflow)?;
            matches.push(Match {
                maker: maker_id,
                price,
                quantity: fill,
                maker_remaining: maker.remaining,
            });
            self.remove(maker_id)?;
            if maker.remaining.get() > 0 {
                if maker.visible.get() == 0 {
                    // Refresh the iceberg slice at the back of the level.
                    let peak = maker.display.unwrap_or(maker.remaining);
                    maker.visible = QuantityLots::new(peak.get().min(maker.remaining.get()));
                    maker.priority = self.take_priority()?;
                }
                self.insert(maker)?;
            }
        }
        Ok((matches, remaining))
    }

    /// Rests a new order at the back of its price level.
    pub fn rest(
        &mut self,
        order_id: OrderId,
        side: Side,
        price: PriceTicks,
        quantity: QuantityLots,
        display: Option<QuantityLots>,
    ) -> Result<(), BookError> {
        if quantity.get() <= 0
            || price.get() <= 0
            || display.is_some_and(|peak| peak.get() <= 0 || peak > quantity)
        {
            return Err(BookError::InvalidOrder);
        }
        if self.orders.contains_key(&order_id) {
            return Err(BookError::DuplicateOrder);
        }
        let priority = self.take_priority()?;
        self.insert(BookOrder {
            order_id,
            side,
            price,
            remaining: quantity,
            visible: display.unwrap_or(quantity),
            display,
            priority,
        })
    }

    /// Removes a resting order and returns it.
    pub fn cancel(&mut self, order_id: OrderId) -> Option<BookOrder> {
        let order = *self.orders.get(&order_id)?;
        self.remove(order_id).ok()?;
        Some(order)
    }

    /// Displayed quantity per price level, best first.
    #[must_use]
    pub fn depth(&self, side: Side) -> Vec<(PriceTicks, QuantityLots)> {
        let levels: Box<dyn Iterator<Item = (&PriceTicks, &Level)>> = match side {
            Side::Buy => Box::new(self.bids.iter().rev()),
            Side::Sell => Box::new(self.asks.iter()),
        };
        levels
            .map(|(price, level)| (*price, level.visible))
            .collect()
    }

    /// Resting orders on one side in matching priority, best price first.
    pub fn orders(&self, side: Side) -> impl Iterator<Item = &BookOrder> + '_ {
        let levels: Box<dyn Iterator<Item = &Level>> = match side {
            Side::Buy => Box::new(self.bids.values().rev()),
            Side::Sell => Box::new(self.asks.values()),
        };
        levels
            .flat_map(|level| level.queue.values())
            .filter_map(|order_id| self.orders.get(order_id))
    }

    fn take_priority(&mut self) -> Result<u64, BookError> {
        let priority = self.next_priority;
        self.next_priority = priority.checked_add(1).ok_or(BookError::Overflow)?;
        Ok(priority)
    }

    fn insert(&mut self, order: BookOrder) -> Result<(), BookError> {
        if order.remaining.get() <= 0
            || order.visible.get() <= 0
            || order.visible > order.remaining
            || order.price.get() <= 0
        {
            return Err(BookError::InvalidOrder);
        }
        if self.orders.contains_key(&order.order_id) {
            return Err(BookError::DuplicateOrder);
        }
        let level = self.side_mut(order.side).entry(order.price).or_default();
        if level.queue.insert(order.priority, order.order_id).is_some() {
            return Err(BookError::DuplicateOrder);
        }
        level.visible = level
            .visible
            .checked_add(order.visible)
            .ok_or(BookError::Overflow)?;
        level.total = level
            .total
            .checked_add(order.remaining)
            .ok_or(BookError::Overflow)?;
        self.orders.insert(order.order_id, order);
        Ok(())
    }

    fn remove(&mut self, order_id: OrderId) -> Result<(), BookError> {
        let order = self
            .orders
            .remove(&order_id)
            .ok_or(BookError::InvalidOrder)?;
        let side = self.side_mut(order.side);
        let level = side.get_mut(&order.price).ok_or(BookError::InvalidOrder)?;
        level.queue.remove(&order.priority);
        // Callers may have already reduced the order; recompute the level
        // aggregate from the stored copy, which still reflects the book.
        level.visible = level
            .visible
            .checked_sub(order.visible)
            .ok_or(BookError::Overflow)?;
        level.total = level
            .total
            .checked_sub(order.remaining)
            .ok_or(BookError::Overflow)?;
        if level.queue.is_empty() {
            side.remove(&order.price);
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use orderbook_rs::orderbook::modifications::OrderQuantity;
    use std::str::FromStr;

    fn id(value: u128) -> OrderId {
        OrderId::new(value)
    }

    fn qty(value: i64) -> QuantityLots {
        QuantityLots::new(value)
    }

    fn px(value: i64) -> PriceTicks {
        PriceTicks::new(value)
    }

    #[test]
    fn price_then_time_priority_and_maker_price() {
        let mut book = Book::new();
        book.rest(id(1), Side::Sell, px(101), qty(5), None).unwrap();
        book.rest(id(2), Side::Sell, px(100), qty(5), None).unwrap();
        book.rest(id(3), Side::Sell, px(100), qty(5), None).unwrap();
        let (matches, remaining) = book.execute(Side::Buy, Some(px(101)), qty(12)).unwrap();
        assert_eq!(remaining, qty(0));
        assert_eq!(
            matches
                .iter()
                .map(|fill| (fill.maker, fill.price, fill.quantity))
                .collect::<Vec<_>>(),
            vec![
                (id(2), px(100), qty(5)),
                (id(3), px(100), qty(5)),
                (id(1), px(101), qty(2)),
            ]
        );
        assert_eq!(book.depth(Side::Sell), vec![(px(101), qty(3))]);
        assert_eq!(matches[2].maker_remaining, qty(3));
    }

    #[test]
    fn limit_stops_at_non_crossing_levels_and_market_sweeps() {
        let mut book = Book::new();
        book.rest(id(1), Side::Buy, px(99), qty(4), None).unwrap();
        book.rest(id(2), Side::Buy, px(98), qty(4), None).unwrap();
        let (matches, remaining) = book.execute(Side::Sell, Some(px(99)), qty(6)).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(remaining, qty(2));
        let (matches, remaining) = book.execute(Side::Sell, None, qty(10)).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(remaining, qty(6));
        assert!(book.is_empty());
    }

    #[test]
    fn iceberg_refresh_loses_priority_and_hides_quantity() {
        let mut book = Book::new();
        book.rest(id(1), Side::Sell, px(100), qty(10), Some(qty(3)))
            .unwrap();
        book.rest(id(2), Side::Sell, px(100), qty(2), None).unwrap();
        assert_eq!(book.depth(Side::Sell), vec![(px(100), qty(5))]);
        assert_eq!(book.executable(Side::Buy, Some(px(100))), qty(12));
        let (matches, _) = book.execute(Side::Buy, None, qty(4)).unwrap();
        assert_eq!(
            matches
                .iter()
                .map(|fill| (fill.maker, fill.quantity))
                .collect::<Vec<_>>(),
            vec![(id(1), qty(3)), (id(2), qty(1))]
        );
        // The refreshed iceberg slice is now behind order 2.
        let order = book
            .orders(Side::Sell)
            .map(|order| order.order_id)
            .collect::<Vec<_>>();
        assert_eq!(order, vec![id(2), id(1)]);
        assert_eq!(book.depth(Side::Sell), vec![(px(100), qty(4))]);
    }

    #[test]
    fn cancel_and_full_width_identities_never_alias() {
        let mut book = Book::new();
        let low = id(7);
        let high = id((1 << 64) | 7);
        book.rest(low, Side::Buy, px(10), qty(1), None).unwrap();
        book.rest(high, Side::Buy, px(10), qty(2), None).unwrap();
        assert_eq!(book.cancel(high).unwrap().remaining, qty(2));
        assert!(book.contains(low));
        assert_eq!(book.depth(Side::Buy), vec![(px(10), qty(1))]);
        assert_eq!(
            book.rest(low, Side::Buy, px(11), qty(1), None),
            Err(BookError::DuplicateOrder)
        );
    }

    #[test]
    fn persisted_form_is_canonical_and_validated() {
        let mut book = Book::new();
        book.rest(id(2), Side::Sell, px(105), qty(3), Some(qty(1)))
            .unwrap();
        book.rest(id(1), Side::Buy, px(100), qty(4), None).unwrap();
        let json = serde_json::to_string(&book).unwrap();
        let restored: Book = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, book);
        assert_eq!(serde_json::to_string(&restored).unwrap(), json);
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        value["next_priority"] = serde_json::json!(0);
        assert!(serde_json::from_value::<Book>(value).is_err());
    }

    /// Deterministic 64-bit LCG so the oracle needs no randomness dependency.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self, bound: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % bound
        }
    }

    fn oracle_side(side: Side) -> orderbook_rs::Side {
        match side {
            Side::Buy => orderbook_rs::Side::Buy,
            Side::Sell => orderbook_rs::Side::Sell,
        }
    }

    fn oracle_depth(
        oracle: &orderbook_rs::DefaultOrderBook,
        side: Side,
    ) -> Vec<(PriceTicks, QuantityLots)> {
        let snapshot = oracle.create_snapshot(usize::MAX);
        let levels = match side {
            Side::Buy => &snapshot.bids,
            Side::Sell => &snapshot.asks,
        };
        let mut depth = levels
            .iter()
            .map(|level| {
                (
                    PriceTicks::new(i64::try_from(level.price().as_u128()).unwrap()),
                    QuantityLots::new(i64::try_from(level.visible_quantity().as_u64()).unwrap()),
                )
            })
            .collect::<Vec<_>>();
        match side {
            Side::Buy => depth.sort_by(|left, right| right.0.cmp(&left.0)),
            Side::Sell => depth.sort_by(|left, right| left.0.cmp(&right.0)),
        }
        depth
    }

    /// Differential conformance against OrderBook-rs 0.10.3 for the shared
    /// semantics: GTC limit, market and cancel with price-time priority.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one oracle loop keeps both books' inputs and assertions side by side"
    )]
    fn matches_orderbook_rs_on_random_limit_market_and_cancel_streams() {
        for seed in 1..=20_u64 {
            let mut rng = Lcg(seed);
            let mut book = Book::new();
            let oracle = orderbook_rs::DefaultOrderBook::with_clock(
                "ORACLE",
                std::sync::Arc::new(orderbook_rs::StubClock::starting_at(1)),
            );
            let mut live = Vec::<u64>::new();
            for step in 0..400_u64 {
                let order_id = step + 1;
                let side = if rng.next(2) == 0 {
                    Side::Buy
                } else {
                    Side::Sell
                };
                let quantity = rng.next(20) + 1;
                match rng.next(10) {
                    0..=5 => {
                        let price = 95 + rng.next(11);
                        let (matches, remaining) = book
                            .execute(
                                side,
                                Some(px(i64::try_from(price).unwrap())),
                                qty(i64::try_from(quantity).unwrap()),
                            )
                            .unwrap();
                        if remaining.get() > 0 {
                            book.rest(
                                id(u128::from(order_id)),
                                side,
                                px(i64::try_from(price).unwrap()),
                                remaining,
                                None,
                            )
                            .unwrap();
                            live.push(order_id);
                        }
                        let (_, result) = oracle
                            .add_limit_order_with_result(
                                pricelevel::Id::sequential(order_id),
                                u128::from(price),
                                quantity,
                                oracle_side(side),
                                orderbook_rs::TimeInForce::Gtc,
                                None,
                            )
                            .unwrap();
                        let expected = result
                            .map(|trades| {
                                orderbook_rs::TradeInfo::from_trade_result(&trades, None)
                                    .transactions
                                    .iter()
                                    .map(|fill| {
                                        (
                                            u128::from(
                                                pricelevel::Id::from_str(&fill.maker_order_id)
                                                    .unwrap()
                                                    .as_u64()
                                                    .unwrap(),
                                            ),
                                            i64::try_from(fill.price).unwrap(),
                                            i64::try_from(fill.quantity).unwrap(),
                                        )
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        let actual = matches
                            .iter()
                            .map(|fill| (fill.maker.get(), fill.price.get(), fill.quantity.get()))
                            .collect::<Vec<_>>();
                        assert_eq!(actual, expected, "seed {seed} step {step}");
                    }
                    6..=7 if !live.is_empty() => {
                        let index = usize::try_from(rng.next(live.len() as u64)).unwrap();
                        let target = live.swap_remove(index);
                        let ours = book
                            .cancel(id(u128::from(target)))
                            .map(|order| order.remaining);
                        let theirs = oracle
                            .cancel_order(pricelevel::Id::sequential(target))
                            .unwrap()
                            .map(|order| qty(i64::try_from(order.quantity()).unwrap()));
                        assert_eq!(ours, theirs, "seed {seed} step {step}");
                    }
                    _ => {
                        if book.best(side.opposite()).is_none() {
                            continue;
                        }
                        let (matches, _) = book
                            .execute(side, None, qty(i64::try_from(quantity).unwrap()))
                            .unwrap();
                        let result = oracle
                            .submit_market_order(
                                pricelevel::Id::sequential(order_id),
                                quantity,
                                oracle_side(side),
                            )
                            .unwrap();
                        let executed = result
                            .executed_quantity()
                            .map_or(0, pricelevel::Quantity::as_u64);
                        let ours = matches
                            .iter()
                            .map(|fill| u64::try_from(fill.quantity.get()).unwrap())
                            .sum::<u64>();
                        assert_eq!(ours, executed, "seed {seed} step {step}");
                    }
                }
                live.retain(|order| book.contains(id(u128::from(*order))));
                assert_eq!(book.depth(Side::Buy), oracle_depth(&oracle, Side::Buy));
                assert_eq!(book.depth(Side::Sell), oracle_depth(&oracle, Side::Sell));
            }
        }
    }
}
