//! Derived series for charts and the portfolio: per-listing bars from
//! the direct trade feeds, running market VWAP and volume, and the
//! participant's own cost basis and P&L from their fills.
//!
//! Everything here is **derived** from committed BNP data this client
//! has seen since it subscribed, never the venue's own valuation; the
//! BNP history and valuation gaps are recorded in
//! `docs/specs/rit-desk-window-parity.md`. In particular:
//!
//! - Feed updates (`MarketUpdate`/`OrderUpdate`) carry no run time on
//!   BNP v1. A trade is stamped with the series clock: the latest run
//!   time a stamped report carried, advanced at wall-clock rate from
//!   that stamp until the next one (the `Paced` 1× case). A recorded
//!   trade never moves when a later stamp lands, and the clock never
//!   goes backwards. Before the first stamp, times are nanoseconds
//!   since the first series event. Bar placement between stamps is
//!   therefore an estimate in runs whose clock is not paced at 1×.
//! - Only direct feeds feed the series. Tape trades are the same
//!   trades again by way of the hub and would double-count volume.
//! - Cost is the volume-weighted average price of the current
//!   position, built fill by fill (RIT's definition: the average sale
//!   price for a short). Realized P&L is booked on closed quantity
//!   with commissions counted as a loss (a rebated fee, negative on
//!   the wire, counts back). Unrealized P&L marks the open position
//!   at the best bid (long) or best ask (short) of the instrument's
//!   primary listing (venue 1 where listed, else the first venue),
//!   falling back to that listing's last trade.
//! - Trade-side P&L is kept in price ticks × lots per instrument and
//!   fees separately in the minor currency units BNP reports them
//!   in. The two meet only at the reading boundary, where money is
//!   `tick-lots × multiplier − fees` — the engine ledger's own
//!   notional (`price × quantity × multiplier`). BNP does not yet
//!   carry the multiplier (the reference-data gap), so
//!   [`Model::multiplier`] reports `(1, false)` today and money
//!   figures say whether they assume contract size 1.

use super::{FillRow, Model};
use bunting_client::bnp_wire::{Listing, Position, Side};
use std::collections::{BTreeMap, VecDeque};

/// The finest bar the series keeps, in nanoseconds of run time. Every
/// bar size offered is a whole number of these.
pub const FINEST_BAR_NS: u64 = 1_000_000_000;
/// Bar sizes a chart may ask for, in nanoseconds of run time:
/// 1 s, 5 s, 15 s, 1 min, 5 min.
pub const BAR_SIZES_NS: [u64; 5] = [
    FINEST_BAR_NS,
    5 * FINEST_BAR_NS,
    15 * FINEST_BAR_NS,
    60 * FINEST_BAR_NS,
    300 * FINEST_BAR_NS,
];
/// Finest bars kept per listing: 1 s bars for the last 4 hours of run
/// time. Older bars drop; VWAP and volume below never do.
pub const MAX_FINE_BARS: usize = 14_400;
/// Listings with a series. A session holds at most
/// [`super::MAX_FEEDS`] direct feeds, and a little room past that
/// covers feeds that came and went.
pub const MAX_SERIES_LISTINGS: usize = 64;
/// Instruments with a P&L book (one per instrument ever filled or
/// reported in an account answer).
pub const MAX_PNL_BOOKS: usize = 256;
/// Time between two samples of total P&L, in nanoseconds of run time.
pub const PNL_SAMPLE_NS: u64 = FINEST_BAR_NS;
/// P&L samples kept: one a second for 4 hours of run time.
pub const MAX_PNL_SAMPLES: usize = 14_400;
/// How long an account answer must keep disagreeing with this client's
/// fills, by the same number of lots, before its position is adopted
/// (client µs). Fill reports and account answers travel separately, and
/// a fill on a farther venue can arrive after an answer that already
/// counts it; adopting at once would mark the cost unknown for good.
const ACCOUNT_GRACE_US: u64 = 2_000_000;
/// The oldest two-sided touch a P&L sample marks from (client µs). Past
/// this the book is one-sided for real, not re-quoting, and samples use
/// the live mark like the figures do.
const SAMPLED_MARK_MAX_AGE_US: u64 = 2_000_000;

/// One bar of trades: run-time start, open/high/low/close in price
/// ticks, and the lots traded (quantities as reported).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bar {
    pub start_ns: u64,
    pub open: i64,
    pub high: i64,
    pub low: i64,
    pub close: i64,
    pub volume: i128,
}

impl Bar {
    fn first(start_ns: u64, price: i64, quantity: i64) -> Self {
        Self {
            start_ns,
            open: price,
            high: price,
            low: price,
            close: price,
            volume: i128::from(quantity),
        }
    }

    fn trade(&mut self, price: i64, quantity: i64) {
        self.high = self.high.max(price);
        self.low = self.low.min(price);
        self.close = price;
        self.volume = self.volume.saturating_add(i128::from(quantity));
    }

    fn absorb(&mut self, later: &Self) {
        self.high = self.high.max(later.high);
        self.low = self.low.min(later.low);
        self.close = later.close;
        self.volume = self.volume.saturating_add(later.volume);
    }
}

/// Total derived P&L (realized plus unrealized) at one run-time
/// instant, in minor currency units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PnlSample {
    pub time_ns: u64,
    pub total: i128,
    /// False when some open book's contract multiplier is not known
    /// (contract size 1 assumed) — see [`Model::multiplier`].
    pub multipliers_known: bool,
}

/// Derived P&L summed over every instrument, in minor currency
/// units: each instrument's tick-lots times its contract multiplier,
/// less its fills' fees as reported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PnlTotals {
    pub realized: i128,
    pub unrealized: i128,
    pub total: i128,
    /// False when some open book's multiplier is assumed to be 1.
    pub multipliers_known: bool,
}

/// The series clock: run time as last reported by a stamped message,
/// advanced with the client's clock between stamps. See the module
/// docs for why this estimate exists.
#[derive(Clone, Copy, Debug, Default)]
struct SeriesClock {
    /// `(run time ns, client µs when it was reported)` of the latest
    /// stamp that advanced the run time.
    anchor: Option<(u64, u64)>,
    /// Client µs of the first series event, the zero before any stamp.
    first_us: Option<u64>,
    /// Highest series time handed out, so stamps never go backwards.
    last_ns: u64,
}

impl SeriesClock {
    fn note_stamp(&mut self, logical_time_ns: u64, now_us: u64) {
        if self.anchor.is_none_or(|(ns, _)| logical_time_ns >= ns) {
            self.anchor = Some((logical_time_ns, now_us));
        }
    }

    /// [`Self::now_ns`] without handing the time out: what the clock
    /// would read at `now_us`, for drawing.
    fn peek_ns(&self, now_us: u64) -> u64 {
        let estimated = match (self.anchor, self.first_us) {
            (Some((ns, at_us)), _) => ns.saturating_add(now_us.saturating_sub(at_us) * 1_000),
            (None, Some(first)) => now_us.saturating_sub(first) * 1_000,
            (None, None) => 0,
        };
        self.last_ns.max(estimated)
    }

    fn now_ns(&mut self, now_us: u64) -> u64 {
        let first = *self.first_us.get_or_insert(now_us);
        let estimated = match self.anchor {
            Some((ns, at_us)) => ns.saturating_add(now_us.saturating_sub(at_us) * 1_000),
            None => now_us.saturating_sub(first) * 1_000,
        };
        self.last_ns = self.last_ns.max(estimated);
        self.last_ns
    }
}

/// One listing's trades folded into finest bars, plus the running
/// totals that outlive the bars.
#[derive(Clone, Debug, Default)]
struct ListingSeries {
    bars: VecDeque<Bar>,
    /// Sum of price × quantity over every trade seen (ticks × lots).
    traded_tick_lots: i128,
    /// Sum of quantities over every trade seen.
    volume: i128,
    last_price: Option<i64>,
}

impl ListingSeries {
    fn trade(&mut self, time_ns: u64, price: i64, quantity: i64) {
        self.traded_tick_lots = self
            .traded_tick_lots
            .saturating_add(i128::from(price) * i128::from(quantity));
        self.volume = self.volume.saturating_add(i128::from(quantity));
        self.last_price = Some(price);
        let start = time_ns - time_ns % FINEST_BAR_NS;
        match self.bars.back_mut() {
            // The clock is monotonic, so an older-looking bucket can
            // only mean the same instant after alignment; fold it in.
            Some(bar) if bar.start_ns >= start => bar.trade(price, quantity),
            _ => {
                if self.bars.len() == MAX_FINE_BARS {
                    self.bars.pop_front();
                }
                self.bars.push_back(Bar::first(start, price, quantity));
            }
        }
    }

    /// Finest bars folded into `bar_ns` buckets aligned to the run
    /// clock's zero. Seconds with no trades make no bar.
    fn bars(&self, bar_ns: u64) -> Vec<Bar> {
        let bar_ns = bar_ns.max(FINEST_BAR_NS);
        let mut folded: Vec<Bar> = Vec::new();
        for fine in &self.bars {
            let start = fine.start_ns - fine.start_ns % bar_ns;
            match folded.last_mut() {
                Some(bar) if bar.start_ns == start => bar.absorb(fine),
                _ => folded.push(Bar {
                    start_ns: start,
                    ..*fine
                }),
            }
        }
        folded
    }

    /// [`Self::bars`] with no holes, as a trading chart draws them:
    /// a period without trades is a flat bar at the previous close
    /// with no volume, and the bars run on to the period holding
    /// `through_ns`, so the latest one is the forming bar. At most
    /// [`MAX_FINE_BARS`] bars, the latest kept.
    fn continuous_bars(&self, bar_ns: u64, through_ns: u64) -> Vec<Bar> {
        let bar_ns = bar_ns.max(FINEST_BAR_NS);
        let traded = self.bars(bar_ns);
        let Some(first) = traded.first() else {
            return traded;
        };
        let last_start = traded
            .last()
            .map_or(first.start_ns, |bar| bar.start_ns)
            .max(through_ns - through_ns % bar_ns);
        let span = (last_start - first.start_ns) / bar_ns;
        let skip = span.saturating_sub(MAX_FINE_BARS as u64 - 1);
        let from = first.start_ns + skip * bar_ns;
        let mut out: Vec<Bar> = Vec::with_capacity(usize::try_from(span - skip + 1).unwrap_or(0));
        let mut next = traded.iter().peekable();
        let mut close = first.open;
        let mut start = first.start_ns;
        while start <= last_start {
            match next.peek() {
                Some(bar) if bar.start_ns == start => {
                    close = bar.close;
                    if start >= from {
                        out.push(**bar);
                    }
                    next.next();
                }
                _ => {
                    if start >= from {
                        out.push(Bar {
                            start_ns: start,
                            open: close,
                            high: close,
                            low: close,
                            close,
                            volume: 0,
                        });
                    }
                }
            }
            start += bar_ns;
        }
        out
    }

    /// Volume-weighted average traded price since subscribing, in
    /// ticks, rounded to the nearest tick.
    fn vwap_ticks(&self) -> Option<i64> {
        rounded_div(self.traded_tick_lots, self.volume)
    }
}

/// `numerator / denominator` rounded to the nearest integer (half
/// away from zero), as ticks; `None` when there is nothing to divide
/// by or the result is not a price.
fn rounded_div(numerator: i128, denominator: i128) -> Option<i64> {
    if denominator == 0 {
        return None;
    }
    let half = denominator.abs() / 2;
    let rounded = if (numerator < 0) == (denominator < 0) {
        numerator.abs().saturating_add(half) / denominator.abs()
    } else {
        -(numerator.abs().saturating_add(half) / denominator.abs())
    };
    i64::try_from(rounded).ok()
}

/// One instrument's derived cost basis: the filled position's
/// volume-weighted cost, kept exact as `cost_tick_lots` over the open
/// lots, and what closing has realized so far.
#[derive(Clone, Copy, Debug, Default)]
struct PnlBook {
    /// Signed lots from fills (long positive).
    position: i64,
    /// Sum of price × lots for the lots still open.
    cost_tick_lots: i128,
    /// False when the venue reports a position this client's fills
    /// do not explain (filled before it connected, an endowment):
    /// the lots are known, their cost is not, until fills flatten
    /// the position and open a known one.
    cost_known: bool,
    /// Closed quantity's price P&L, ticks × lots. Fees are not in
    /// here: they arrive in minor currency units, and the two are
    /// only comparable through the contract multiplier.
    realized_tick_lots: i128,
    /// Every fill's fee, summed in minor currency units as reported
    /// (negative is a rebate).
    fees_minor: i128,
    /// Highest committed sequence of a fill in this book.
    last_fill_sequence: u64,
    /// The venue's position minus this book's, from an account answer
    /// the fills did not explain, and when that difference was first
    /// seen (client µs). Adopted once it outlasts [`ACCOUNT_GRACE_US`].
    unexplained: Option<(i64, u64)>,
}

impl PnlBook {
    fn known() -> Self {
        Self {
            cost_known: true,
            ..Self::default()
        }
    }

    /// `numerator / |position|` booked against the open lots: the
    /// exact share of `numerator` that `lots` of them carry.
    fn share(numerator: i128, lots: i128, open: i128) -> i128 {
        if open == 0 {
            0
        } else {
            numerator.saturating_mul(lots) / open
        }
    }

    fn fill(&mut self, side: Side, price: i64, quantity: i64, fee: i128, sequence: u64) {
        self.last_fill_sequence = self.last_fill_sequence.max(sequence);
        self.fees_minor = self.fees_minor.saturating_add(fee);
        let delta: i64 = match side {
            Side::Buy => quantity,
            Side::Sell => -quantity,
        };
        if delta == 0 {
            return;
        }
        let lots = i128::from(delta.unsigned_abs());
        let price = i128::from(price);
        let open = i128::from(self.position.unsigned_abs());
        if self.position == 0 || (delta > 0) == (self.position > 0) {
            // Opening or adding: the new lots join at their price.
            if self.position == 0 {
                self.cost_known = true;
                self.cost_tick_lots = 0;
            }
            if self.cost_known {
                self.cost_tick_lots = self.cost_tick_lots.saturating_add(price * lots);
            }
            self.position = self.position.saturating_add(delta);
            return;
        }
        // Closing some or all of the position, perhaps flipping it.
        let closed = lots.min(open);
        if self.cost_known {
            // Signed so a long gains when the sale beats cost and a
            // short gains when the buy-back undercuts it.
            let sign: i128 = if self.position > 0 { 1 } else { -1 };
            let gain = Self::share(
                price
                    .saturating_mul(open)
                    .saturating_sub(self.cost_tick_lots),
                closed,
                open,
            );
            self.realized_tick_lots = self.realized_tick_lots.saturating_add(sign * gain);
            self.cost_tick_lots -= Self::share(self.cost_tick_lots, closed, open);
        }
        self.position = self.position.saturating_add(delta);
        if lots > open {
            // The remainder opens the other way at this fill's price,
            // and its cost is known whatever came before.
            self.cost_known = true;
            self.cost_tick_lots = price * (lots - open);
        } else if self.position == 0 {
            self.cost_known = true;
            self.cost_tick_lots = 0;
        }
    }

    /// The venue's account answer at `run_sequence`, received at
    /// `now_us`: adopt a position the fills in this book do not explain,
    /// cost unknown, once the same difference has lasted
    /// [`ACCOUNT_GRACE_US`]. A difference that fills still in flight
    /// explain closes before then. An answer older than the latest fill
    /// says nothing.
    fn account(&mut self, position: i64, run_sequence: u64, now_us: u64) {
        if run_sequence < self.last_fill_sequence {
            return;
        }
        if position == self.position {
            self.unexplained = None;
            return;
        }
        let difference = position.saturating_sub(self.position);
        match self.unexplained {
            Some((held, since))
                if held == difference && now_us.saturating_sub(since) >= ACCOUNT_GRACE_US =>
            {
                self.unexplained = None;
                self.position = position;
                self.cost_tick_lots = 0;
                self.cost_known = position == 0;
            }
            Some((held, _)) if held == difference => {}
            _ => self.unexplained = Some((difference, now_us)),
        }
    }

    /// Average cost of the open position in ticks, rounded to the
    /// nearest tick.
    fn cost_ticks(&self) -> Option<i64> {
        if !self.cost_known {
            return None;
        }
        rounded_div(
            self.cost_tick_lots,
            i128::from(self.position.unsigned_abs()),
        )
    }

    /// Realized P&L in minor currency units at contract multiplier
    /// `multiplier`: tick-lots times the multiplier, less fees.
    fn realized_money(&self, multiplier: i64) -> i128 {
        self.realized_tick_lots
            .saturating_mul(i128::from(multiplier))
            .saturating_sub(self.fees_minor)
    }

    /// P&L of closing the open position at `mark` (ticks × lots),
    /// excluding commissions.
    fn unrealized_tick_lots(&self, mark: i64) -> Option<i128> {
        if !self.cost_known || self.position == 0 {
            return None;
        }
        let sign: i128 = if self.position > 0 { 1 } else { -1 };
        let open = i128::from(self.position.unsigned_abs());
        Some(sign * (i128::from(mark).saturating_mul(open) - self.cost_tick_lots))
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct SeriesState {
    clock: SeriesClock,
    listings: BTreeMap<Listing, ListingSeries>,
    books: BTreeMap<u128, PnlBook>,
    /// Contract multipliers by instrument, once BNP carries them
    /// (the reference-data gap); empty today.
    multipliers: BTreeMap<u128, i64>,
    /// The last two-sided touch (bid, ask) per instrument with a
    /// book and when it was seen (client µs), refreshed while sampling:
    /// agents re-quoting leave a one-sided or thinned book for an
    /// instant, and sampling that instant spikes the series. Samples
    /// mark from here instead while it is fresh.
    sampled_marks: BTreeMap<u128, (i64, i64, u64)>,
    samples: VecDeque<PnlSample>,
}

impl Model {
    // ---- reading: market series --------------------------------------

    /// `listing`'s bars of `bar_ns` run-time nanoseconds (see
    /// [`BAR_SIZES_NS`]), oldest first, from the trades its direct
    /// feed carried since this client subscribed. Run seconds with
    /// no trades make no bar. At most [`MAX_FINE_BARS`] finest bars
    /// are behind the answer.
    #[must_use]
    pub fn bars(&self, listing: Listing, bar_ns: u64) -> Vec<Bar> {
        self.series
            .listings
            .get(&listing)
            .map_or_else(Vec::new, |series| series.bars(bar_ns))
    }

    /// [`Self::bars`] as a chart draws them: every period from the
    /// first trade through the one holding client time `now_us` has a
    /// bar, a period without trades being a flat bar at the previous
    /// close with no volume, so candles sit side by side on the run
    /// clock and the latest one forms live.
    #[must_use]
    pub fn continuous_bars(&self, listing: Listing, bar_ns: u64, now_us: u64) -> Vec<Bar> {
        self.series
            .listings
            .get(&listing)
            .map_or_else(Vec::new, |series| {
                series.continuous_bars(bar_ns, self.series.clock.peek_ns(now_us))
            })
    }

    /// The last price `listing` traded at on its direct feed.
    #[must_use]
    pub fn last_price(&self, listing: Listing) -> Option<i64> {
        self.series
            .listings
            .get(&listing)
            .and_then(|series| series.last_price)
    }

    /// Market VWAP of every trade in `listing` since subscribing, in
    /// ticks rounded to the nearest tick; `None` before any trade.
    /// Derived: trades before this client connected are not in it
    /// (the BNP history gap).
    #[must_use]
    pub fn market_vwap(&self, listing: Listing) -> Option<i64> {
        self.series
            .listings
            .get(&listing)
            .and_then(ListingSeries::vwap_ticks)
    }

    /// Cumulative lots traded in `listing` since subscribing
    /// (quantities as reported). Derived, like [`Self::market_vwap`].
    #[must_use]
    pub fn market_volume(&self, listing: Listing) -> i128 {
        self.series
            .listings
            .get(&listing)
            .map_or(0, |series| series.volume)
    }

    // ---- reading: own P&L (derived; see the module docs) --------------

    /// Volume-weighted average cost of the current filled position
    /// in `instrument_id`, in ticks; `None` when flat, or when the
    /// venue's reported position has no fills behind it here.
    #[must_use]
    pub fn cost(&self, instrument_id: u128) -> Option<i64> {
        self.series
            .books
            .get(&instrument_id)
            .and_then(PnlBook::cost_ticks)
    }

    /// The open position's exact average cost as `(tick-lots,
    /// lots)`: their quotient is the volume-weighted price in
    /// ticks, unrounded. `None` when the cost is unknown (the
    /// lots were filled before this client connected) or flat.
    #[must_use]
    pub fn average_cost(&self, instrument_id: u128) -> Option<(i128, i64)> {
        let book = self.series.books.get(&instrument_id)?;
        if !book.cost_known || book.position == 0 {
            return None;
        }
        Some((book.cost_tick_lots, book.position))
    }

    /// The contract multiplier that turns `instrument_id`'s
    /// tick-lots into minor currency units, and whether it is known.
    /// BNP v1 carries no instrument reference data (the gap in
    /// `docs/specs/rit-desk-window-parity.md`), so this is `(1,
    /// false)` — contract size 1, assumed — until it does.
    #[must_use]
    pub fn multiplier(&self, instrument_id: u128) -> (i64, bool) {
        self.series
            .multipliers
            .get(&instrument_id)
            .map_or((1, false), |multiplier| (*multiplier, true))
    }

    /// P&L booked on closed quantity in `instrument_id`, less every
    /// fill's fee, in minor currency units at the instrument's
    /// contract multiplier (assumed 1 while unknown).
    #[must_use]
    pub fn realized_pnl(&self, instrument_id: u128) -> i128 {
        let (multiplier, _) = self.multiplier(instrument_id);
        self.series
            .books
            .get(&instrument_id)
            .map_or(0, |book| book.realized_money(multiplier))
    }

    /// P&L of closing the open position in `instrument_id` now: at
    /// the best bid if long, the best ask if short, of its primary
    /// listing (venue 1 where listed, else the first venue), or that
    /// listing's last trade when the touch is missing, in minor
    /// currency units at the contract multiplier (assumed 1 while
    /// unknown). Excludes commissions. `None` when flat, cost is
    /// unknown, or no mark exists yet.
    #[must_use]
    pub fn unrealized_pnl(&self, instrument_id: u128) -> Option<i128> {
        let book = self.series.books.get(&instrument_id)?;
        let (multiplier, _) = self.multiplier(instrument_id);
        book.unrealized_tick_lots(self.mark(instrument_id, book.position)?)
            .map(|tick_lots| tick_lots.saturating_mul(i128::from(multiplier)))
    }

    /// Derived P&L summed over every instrument, in minor currency
    /// units. An instrument whose unrealized figure cannot be
    /// derived (unknown cost, no mark) contributes only its realized
    /// part. `multipliers_known` is false when any book's contract
    /// multiplier is assumed to be 1.
    #[must_use]
    pub fn pnl_totals(&self) -> PnlTotals {
        let mut realized: i128 = 0;
        let mut unrealized: i128 = 0;
        let mut multipliers_known = true;
        for (instrument_id, book) in &self.series.books {
            let (multiplier, known) = self.multiplier(*instrument_id);
            multipliers_known &= known;
            realized = realized.saturating_add(book.realized_money(multiplier));
            if book.position != 0
                && let Some(mark) = self.mark(*instrument_id, book.position)
                && let Some(open) = book.unrealized_tick_lots(mark)
            {
                unrealized = unrealized.saturating_add(open.saturating_mul(i128::from(multiplier)));
            }
        }
        PnlTotals {
            realized,
            unrealized,
            total: realized.saturating_add(unrealized),
            multipliers_known,
        }
    }

    /// [`Self::pnl_totals`] with each open position marked from
    /// the last two-sided book the sampler saw at most
    /// [`SAMPLED_MARK_MAX_AGE_US`] before `now_us`: what the samples
    /// record, so a momentarily one-sided book (an agent re-quoting)
    /// cannot spike the series, and a book one-sided for longer cannot
    /// freeze it.
    fn sampled_totals(&self, now_us: u64) -> PnlTotals {
        let mut totals = self.pnl_totals();
        // Recompute unrealized from the sampled marks.
        let mut unrealized: i128 = 0;
        for (instrument_id, book) in &self.series.books {
            if book.position == 0 {
                continue;
            }
            let (multiplier, _) = self.multiplier(*instrument_id);
            let mark = self
                .series
                .sampled_marks
                .get(instrument_id)
                .filter(|(_, _, seen)| now_us.saturating_sub(*seen) <= SAMPLED_MARK_MAX_AGE_US)
                .map(|(bid, ask, _)| if book.position > 0 { *bid } else { *ask })
                .or_else(|| self.mark(*instrument_id, book.position));
            if let Some(mark) = mark
                && let Some(open) = book.unrealized_tick_lots(mark)
            {
                unrealized = unrealized.saturating_add(open.saturating_mul(i128::from(multiplier)));
            }
        }
        totals.unrealized = unrealized;
        totals.total = totals.realized.saturating_add(unrealized);
        totals
    }

    /// Total derived P&L sampled on the run clock, oldest first: at
    /// most one sample per [`PNL_SAMPLE_NS`], [`MAX_PNL_SAMPLES`] kept.
    /// The first sample is preceded by a zero at run time 0 —
    /// before the first fill this participant's P&L is 0, and the
    /// first fill's figure must not be read back to the start.
    #[must_use]
    pub const fn pnl_series(&self) -> &VecDeque<PnlSample> {
        &self.series.samples
    }

    /// The price closing a position of `position` lots (signed) in
    /// `instrument_id` would cross at: the primary listing's touch
    /// on the closing side, else its last trade.
    fn mark(&self, instrument_id: u128, position: i64) -> Option<i64> {
        let listing = self.primary_listing(instrument_id)?;
        let touch = self.feeds.get(&listing).and_then(|feed| {
            if position > 0 {
                feed.book.best_bid()
            } else {
                feed.book.best_ask()
            }
        });
        touch.map_or_else(|| self.last_price(listing), |level| Some(level.price))
    }

    /// Venue 1's listing of the instrument where it has one, else
    /// its first listing in venue order.
    fn primary_listing(&self, instrument_id: u128) -> Option<Listing> {
        let mut listings = self
            .listings
            .iter()
            .filter(|info| info.listing.instrument_id == instrument_id)
            .map(|info| info.listing);
        let first = listings.next()?;
        if first.venue_id == 1 {
            return Some(first);
        }
        let mut primary = first;
        for listing in listings {
            if listing.venue_id == 1 || listing.venue_id < primary.venue_id {
                primary = listing;
            }
            if primary.venue_id == 1 {
                break;
            }
        }
        Some(primary)
    }

    // ---- applying ------------------------------------------------------

    pub(super) fn series_stamp(&mut self, logical_time_ns: u64, now_us: u64) {
        self.series.clock.note_stamp(logical_time_ns, now_us);
    }

    pub(super) fn series_trade(
        &mut self,
        listing: Listing,
        price: i64,
        quantity: i64,
        now_us: u64,
    ) {
        let time_ns = self.series.clock.now_ns(now_us);
        if !self.series.listings.contains_key(&listing)
            && self.series.listings.len() == MAX_SERIES_LISTINGS
        {
            return;
        }
        self.series
            .listings
            .entry(listing)
            .or_default()
            .trade(time_ns, price, quantity);
    }

    pub(super) fn series_fill(&mut self, fill: &FillRow, now_us: u64) {
        let instrument_id = fill.listing.instrument_id;
        if !self.series.books.contains_key(&instrument_id)
            && self.series.books.len() == MAX_PNL_BOOKS
        {
            return;
        }
        self.series
            .books
            .entry(instrument_id)
            .or_insert_with(PnlBook::known)
            .fill(
                fill.side,
                fill.price,
                fill.quantity,
                fill.fee,
                fill.sequence,
            );
        self.series_sample(now_us);
    }

    pub(super) fn series_account(
        &mut self,
        positions: &[Position],
        run_sequence: u64,
        now_us: u64,
    ) {
        for position in positions {
            if !self.series.books.contains_key(&position.instrument_id)
                && (position.position == 0 || self.series.books.len() == MAX_PNL_BOOKS)
            {
                continue;
            }
            self.series
                .books
                .entry(position.instrument_id)
                .or_insert_with(PnlBook::known)
                .account(position.position, run_sequence, now_us);
        }
    }

    /// Sets an instrument's contract multiplier as a future BNP
    /// reference-data message will; only tests can, today.
    #[cfg(test)]
    pub(super) fn series_set_multiplier(&mut self, instrument_id: u128, multiplier: i64) {
        self.series.multipliers.insert(instrument_id, multiplier);
    }

    /// Records total P&L for the current run second, replacing that
    /// second's sample while it is still the latest.
    pub(super) fn series_sample(&mut self, now_us: u64) {
        if self.series.books.is_empty() {
            return;
        }
        // Refresh the two-sided marks first: this runs on every
        // tick, so by the time a second's sample settles it holds
        // that second's last two-sided touch.
        let mut two_sided: Vec<(u128, i64, i64)> = Vec::new();
        for instrument_id in self.series.books.keys() {
            if let Some(listing) = self.primary_listing(*instrument_id)
                && let Some(feed) = self.feeds.get(&listing)
                && let (Some(bid), Some(ask)) = (feed.book.best_bid(), feed.book.best_ask())
            {
                two_sided.push((*instrument_id, bid.price, ask.price));
            }
        }
        for (instrument_id, bid, ask) in two_sided {
            self.series
                .sampled_marks
                .insert(instrument_id, (bid, ask, now_us));
        }
        let time_ns = self.series.clock.now_ns(now_us);
        // Before the first fill the participant's P&L is zero; say
        // so at run time 0 rather than letting the first sample
        // read as the value since the run began.
        if self.series.samples.is_empty() && time_ns >= PNL_SAMPLE_NS {
            self.series.samples.push_back(PnlSample {
                time_ns: 0,
                total: 0,
                multipliers_known: true,
            });
        }
        let totals = self.sampled_totals(now_us);
        if let Some(last) = self.series.samples.back_mut()
            && last.time_ns / PNL_SAMPLE_NS == time_ns / PNL_SAMPLE_NS
        {
            last.time_ns = time_ns;
            last.total = totals.total;
            last.multipliers_known = totals.multipliers_known;
            return;
        }
        if self.series.samples.len() == MAX_PNL_SAMPLES {
            self.series.samples.pop_front();
        }
        self.series.samples.push_back(PnlSample {
            time_ns,
            total: totals.total,
            multipliers_known: totals.multipliers_known,
        });
    }
}

#[cfg(test)]
mod tests;
