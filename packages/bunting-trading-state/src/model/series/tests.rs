use super::*;
use crate::model::{Feed, FeedState, FillRow};
use bunting_client::bnp_wire::Liquidity;

#[allow(clippy::too_many_arguments, reason = "a fill's fields, spelled out")]
fn fill(
    model: &mut Model,
    listing: Listing,
    side: Side,
    price: i64,
    quantity: i64,
    fee: i128,
    sequence: u64,
    now_us: u64,
) {
    model.series_fill(
        &FillRow {
            sequence,
            logical_time_ns: 0,
            client_order_id: 1,
            order_id: 1,
            listing,
            side,
            price,
            quantity,
            fee,
            liquidity: Liquidity::Taker,
            received_us: now_us,
        },
        now_us,
    );
}
use bunting_client::FeedBook;
use bunting_client::bnp_wire::{Level, ListingInfo, ServerMessage};

const SEC: u64 = FINEST_BAR_NS;
const NEAR: Listing = Listing {
    venue_id: 1,
    instrument_id: 7,
};
const FAR: Listing = Listing {
    venue_id: 2,
    instrument_id: 7,
};

#[test]
fn bars_fold_trades_into_ohlcv_on_run_clock_boundaries() {
    let mut series = ListingSeries::default();
    // Second 10: three trades. Second 11: one. Second 12: a gap.
    series.trade(10 * SEC + 100, 100, 2);
    series.trade(10 * SEC + 200, 105, 3);
    series.trade(10 * SEC + 300, 99, 1);
    series.trade(11 * SEC, 101, 4);
    series.trade(13 * SEC + 1, 102, 5);

    let bars = series.bars(SEC);
    assert_eq!(bars.len(), 3, "the empty second makes no bar");
    assert_eq!(
        bars[0],
        Bar {
            start_ns: 10 * SEC,
            open: 100,
            high: 105,
            low: 99,
            close: 99,
            volume: 6,
        }
    );
    assert_eq!(
        bars[1],
        Bar {
            start_ns: 11 * SEC,
            open: 101,
            high: 101,
            low: 101,
            close: 101,
            volume: 4,
        }
    );
    assert_eq!(bars[2].start_ns, 13 * SEC);

    // Five-second bars align to the run clock's zero, not to the
    // first trade: seconds 10-13 all fall in the bucket starting 10.
    let bars = series.bars(5 * SEC);
    assert_eq!(bars.len(), 1);
    assert_eq!(
        bars[0],
        Bar {
            start_ns: 10 * SEC,
            open: 100,
            high: 105,
            low: 99,
            close: 102,
            volume: 15,
        }
    );

    // A size under the finest asks for the finest.
    assert_eq!(series.bars(1), series.bars(SEC));
    assert_eq!(series.last_price, Some(102));
}

#[test]
fn vwap_and_volume_cover_every_trade_and_outlive_the_bars() {
    let mut series = ListingSeries::default();
    assert_eq!(series.vwap_ticks(), None);
    series.trade(0, 100, 1);
    series.trade(SEC, 110, 3);
    // (100*1 + 110*3) / 4 = 107.5, rounded to 108.
    assert_eq!(series.vwap_ticks(), Some(108));
    assert_eq!(series.volume, 4);

    for second in 2..=(MAX_FINE_BARS as u64 + 2) {
        series.trade(second * SEC, 100, 1);
    }
    assert_eq!(series.bars.len(), MAX_FINE_BARS);
    assert_eq!(series.bars.front().map(|bar| bar.start_ns), Some(3 * SEC));
    // The totals kept the two evicted seconds.
    assert_eq!(series.volume, 4 + MAX_FINE_BARS as i128 + 1);
}

#[test]
fn the_series_clock_snaps_to_stamps_and_never_goes_backwards() {
    let mut clock = SeriesClock::default();
    // Before any stamp: nanoseconds since the first event.
    assert_eq!(clock.now_ns(1_000_000), 0);
    assert_eq!(clock.now_ns(3_500_000), 2_500_000_000);
    // A stamp re-anchors; wall time since it advances from it.
    clock.note_stamp(60 * SEC, 4_000_000);
    assert_eq!(clock.now_ns(5_000_000), 61 * SEC);
    // An older stamp does not re-anchor, and nothing goes backwards.
    clock.note_stamp(30 * SEC, 6_000_000);
    assert_eq!(clock.now_ns(6_000_000), 62 * SEC);
    assert_eq!(clock.now_ns(5_500_000), 62 * SEC);
}

#[test]
fn cost_is_the_vwap_of_the_position_through_add_partial_close_and_flip() {
    let mut book = PnlBook::known();
    book.fill(Side::Buy, 100, 10, 0, 1);
    assert_eq!(book.cost_ticks(), Some(100));
    book.fill(Side::Buy, 110, 10, 0, 2);
    assert_eq!(book.cost_ticks(), Some(105));

    // Sell half at 115: realized 10 * (115 - 105); cost unchanged.
    book.fill(Side::Sell, 115, 10, 0, 3);
    assert_eq!(book.realized_tick_lots, 100);
    assert_eq!(book.cost_ticks(), Some(105));
    assert_eq!(book.position, 10);

    // Sell 25 at 95: closes the 10 long (10 * (95 - 105) = -100) and
    // opens 15 short whose cost is the average sale price, 95.
    book.fill(Side::Sell, 95, 25, 0, 4);
    assert_eq!(book.realized_tick_lots, 0);
    assert_eq!(book.position, -15);
    assert_eq!(book.cost_ticks(), Some(95));

    // Buy back 5 at 90: realized 5 * (95 - 90); cost stays 95.
    book.fill(Side::Buy, 90, 5, 0, 5);
    assert_eq!(book.realized_tick_lots, 25);
    assert_eq!(book.cost_ticks(), Some(95));
    // Closing the rest at 95 realizes nothing more; flat has no cost.
    book.fill(Side::Buy, 95, 10, 0, 6);
    assert_eq!(book.realized_tick_lots, 25);
    assert_eq!(book.cost_ticks(), None);
}

#[test]
fn fees_stay_in_minor_units_and_meet_ticks_only_through_the_multiplier() {
    let mut book = PnlBook::known();
    book.fill(Side::Buy, 100, 10, 7, 1);
    // The fee is kept apart, in its own units, from the start.
    assert_eq!(book.realized_tick_lots, 0);
    assert_eq!(book.fees_minor, 7);
    book.fill(Side::Sell, 102, 10, -3, 2);
    assert_eq!(book.realized_tick_lots, 20);
    assert_eq!(book.fees_minor, 4, "the rebate counts back");
    // Money meets ticks only through the multiplier: 20 * 1 - 4.
    assert_eq!(book.realized_money(1), 16);
    // A x10 contract makes the trade side ten times the money and
    // leaves the fees exactly as charged: 20 * 10 - 4.
    assert_eq!(book.realized_money(10), 196);
}

#[test]
fn unrealized_marks_long_at_the_bid_and_short_at_the_ask() {
    let mut book = PnlBook::known();
    book.fill(Side::Buy, 100, 10, 0, 1);
    assert_eq!(book.unrealized_tick_lots(103), Some(30));
    assert_eq!(book.unrealized_tick_lots(98), Some(-20));

    let mut short = PnlBook::known();
    short.fill(Side::Sell, 100, 10, 0, 1);
    assert_eq!(short.unrealized_tick_lots(97), Some(30));
    assert_eq!(short.unrealized_tick_lots(101), Some(-10));
}

#[test]
fn an_account_position_without_fills_has_unknown_cost_until_flat() {
    let mut book = PnlBook::known();
    book.fill(Side::Buy, 100, 10, 0, 5);
    // An answer older than the fill changes nothing.
    book.account(0, 4, 0);
    assert_eq!(book.position, 10);
    // A newer answer the fills do not explain is adopted, costless,
    // once the same difference has outlasted the grace period.
    book.account(40, 6, 0);
    assert_eq!(book.position, 10, "not adopted on first sight");
    assert_eq!(book.cost_ticks(), Some(100));
    book.account(40, 6, ACCOUNT_GRACE_US);
    assert_eq!(book.position, 40);
    assert_eq!(book.cost_ticks(), None);
    assert_eq!(book.unrealized_tick_lots(200), None);
    // Closing it books no price gain (cost unknown), only the fee,
    // and being flat makes the next position's cost known again.
    book.fill(Side::Sell, 200, 40, 2, 7);
    assert_eq!(book.realized_tick_lots, 0);
    assert_eq!(book.fees_minor, 2);
    book.fill(Side::Buy, 50, 4, 0, 8);
    assert_eq!(book.cost_ticks(), Some(50));
}

fn model_with_listings() -> Model {
    let mut model = Model::new(1);
    model.listings = vec![
        ListingInfo {
            listing: FAR,
            symbol: "BNT".to_owned(),
        },
        ListingInfo {
            listing: NEAR,
            symbol: "BNT".to_owned(),
        },
    ];
    model
}

fn touch(model: &mut Model, listing: Listing, bid: i64, ask: i64) {
    let mut book = FeedBook::new(1);
    let applied = book.apply(&ServerMessage::MarketSnapshot {
        request_id: 1,
        listing,
        next_report: 1,
        bids: vec![Level {
            price: bid,
            quantity: 10,
        }],
        asks: vec![Level {
            price: ask,
            quantity: 10,
        }],
    });
    assert!(applied.is_ok(), "snapshot applies");
    model.feeds.insert(
        listing,
        Feed {
            request_id: 1,
            book,
            order_by_order: false,
            state: FeedState::Live,
            updates: 0,
            gaps: 0,
            last_message_us: None,
        },
    );
}

#[test]
fn model_marks_on_the_primary_listings_touch_then_its_last_trade() {
    let mut model = model_with_listings();
    fill(&mut model, NEAR, Side::Buy, 100, 10, 0, 1, 0);
    assert_eq!(model.cost(7), Some(100));
    assert_eq!(model.unrealized_pnl(7), None, "no touch and no trade yet");

    // Venue 2's touch is not the mark; venue 1 (primary) has only a
    // last trade, which stands in.
    touch(&mut model, FAR, 130, 131);
    model.series_trade(NEAR, 104, 1, 1_000_000);
    assert_eq!(model.unrealized_pnl(7), Some(40));

    // Venue 1's bid replaces the last trade for a long.
    touch(&mut model, NEAR, 102, 109);
    assert_eq!(model.unrealized_pnl(7), Some(20));

    // Flip short on venue 2: the book is per instrument, and a short
    // marks at venue 1's ask.
    fill(&mut model, FAR, Side::Sell, 105, 30, 0, 2, 2_000_000);
    assert_eq!(model.realized_pnl(7), 50);
    assert_eq!(model.cost(7), Some(105));
    assert_eq!(model.unrealized_pnl(7), Some(20 * (105 - 109)));
    assert_eq!(
        model.pnl_totals(),
        PnlTotals {
            realized: 50,
            unrealized: -80,
            total: -30,
            multipliers_known: false,
        }
    );
}

#[test]
fn average_cost_is_exact_never_a_whole_tick() {
    let mut model = model_with_listings();
    fill(&mut model, NEAR, Side::Buy, 105, 20, 0, 1, 0);
    fill(&mut model, NEAR, Side::Buy, 106, 20, 0, 2, 1_000_000);
    // 20 @105 + 20 @106 averages 105.5, not 105 or 106.
    assert_eq!(model.average_cost(7), Some((4_220, 40)));
    assert_eq!(
        model
            .average_cost(7)
            .map(|(tick_lots, lots)| tick_lots * 10 / i128::from(lots)),
        Some(1_055),
        "105.5 in tenths of a tick"
    );
    assert_eq!(model.cost(7), Some(106), "cost() still rounds for P&L use");
}

#[test]
fn samples_land_once_per_run_second_and_stay_bounded() {
    let mut model = model_with_listings();
    model.series_stamp(0, 0);
    fill(&mut model, NEAR, Side::Buy, 100, 10, 5, 1, 0);
    // Two fills in the same run second leave one sample, the latest.
    fill(&mut model, NEAR, Side::Sell, 100, 10, 5, 2, 500_000);
    assert_eq!(model.pnl_series().len(), 1);
    assert_eq!(model.pnl_series().back().map(|s| s.total), Some(-10));

    for second in 1..=(MAX_PNL_SAMPLES as u64 + 1) {
        model.series_sample(second * 1_000_000);
    }
    assert_eq!(model.pnl_series().len(), MAX_PNL_SAMPLES);
    assert!(
        model.pnl_series().front().is_some_and(|s| s.time_ns >= SEC),
        "the oldest samples dropped"
    );
}

#[test]
fn series_and_books_stop_at_their_bounds() {
    let mut model = Model::new(1);
    for venue in 0..=(MAX_SERIES_LISTINGS as u128) {
        model.series_trade(
            Listing {
                venue_id: venue,
                instrument_id: 1,
            },
            100,
            1,
            0,
        );
    }
    assert_eq!(model.series.listings.len(), MAX_SERIES_LISTINGS);
    // A listing already held still records past the bound.
    let held = Listing {
        venue_id: 0,
        instrument_id: 1,
    };
    model.series_trade(held, 101, 2, 1_000_000);
    assert_eq!(model.market_volume(held), 3);

    for instrument in 0..=(MAX_PNL_BOOKS as u128) {
        fill(
            &mut model,
            Listing {
                venue_id: 1,
                instrument_id: instrument,
            },
            Side::Buy,
            1,
            1,
            0,
            1,
            0,
        );
    }
    assert_eq!(model.series.books.len(), MAX_PNL_BOOKS);
}

#[test]
fn money_pnl_is_tick_lots_times_the_multiplier_minus_fees() {
    let mut model = model_with_listings();
    // BNP carries no multiplier yet: assumed 1, and the totals say so.
    assert_eq!(model.multiplier(7), (1, false));
    model.series_set_multiplier(7, 10);
    assert_eq!(model.multiplier(7), (10, true));

    // Buy 10 at 100 (fee 7 minor), sell 10 at 102 (rebate 3 minor):
    // 20 tick-lots of price gain, 4 minor of net fees.
    fill(&mut model, NEAR, Side::Buy, 100, 10, 7, 1, 0);
    fill(&mut model, NEAR, Side::Sell, 102, 10, -3, 2, 1_000_000);
    assert_eq!(model.realized_pnl(7), 20 * 10 - 4);

    // Long again at 100, marked at the primary listing's last trade
    // of 103: 30 tick-lots become 300 minor.
    fill(&mut model, NEAR, Side::Buy, 100, 10, 0, 3, 2_000_000);
    model.series_trade(NEAR, 103, 1, 3_000_000);
    assert_eq!(model.unrealized_pnl(7), Some(300));
    let totals = model.pnl_totals();
    assert_eq!(totals.realized, 196);
    assert_eq!(totals.unrealized, 300);
    assert_eq!(totals.total, 496);
    assert!(totals.multipliers_known);
    assert_eq!(
        model.pnl_series().back().map(|sample| sample.total),
        Some(196),
        "samples are money too (the last fill predates the mark)"
    );
    assert!(
        model
            .pnl_series()
            .back()
            .is_some_and(|sample| sample.multipliers_known)
    );
}

#[test]
fn the_pnl_series_is_zero_until_the_first_fill() {
    let mut model = model_with_listings();
    // The first fill lands at run time 36 s (its stamp says so).
    model.series_stamp(36 * SEC, 0);
    fill(&mut model, NEAR, Side::Buy, 100, 10, 10, 1, 0);
    let samples: Vec<(u64, i128)> = model
        .pnl_series()
        .iter()
        .map(|sample| (sample.time_ns, sample.total))
        .collect();
    assert_eq!(
        samples,
        vec![(0, 0), (36 * SEC, -10)],
        "zero at run start, then the fill's own figure — never backfilled"
    );
    assert!(
        model
            .pnl_series()
            .front()
            .is_some_and(|s| s.multipliers_known)
    );
}

/// The primary listing's book with only an ask side, as while an
/// agent re-quotes.
fn one_sided(model: &mut Model, listing: Listing, ask: i64) {
    let mut book = FeedBook::new(1);
    let applied = book.apply(&ServerMessage::MarketSnapshot {
        request_id: 1,
        listing,
        next_report: 1,
        bids: Vec::new(),
        asks: vec![Level {
            price: ask,
            quantity: 10,
        }],
    });
    assert!(applied.is_ok(), "snapshot applies");
    model.feeds.insert(
        listing,
        Feed {
            request_id: 1,
            book,
            order_by_order: false,
            state: FeedState::Live,
            updates: 0,
            gaps: 0,
            last_message_us: None,
        },
    );
}

#[test]
fn samples_mark_from_the_last_two_sided_book_not_a_requote_instant() {
    let mut model = model_with_listings();
    touch(&mut model, NEAR, 99, 101);
    model.series_stamp(0, 0);
    // Long 10 at 100, marked at the 99 bid: -10.
    fill(&mut model, NEAR, Side::Buy, 100, 10, 0, 1, 0);

    // A trade at 90, then the bid vanishes (a re-quote): the live
    // mark falls back to that trade, but the second's sample must
    // keep the last two-sided bid.
    model.series_trade(NEAR, 90, 1, 200_000);
    one_sided(&mut model, NEAR, 101);
    model.series_stamp(SEC, 1_000_000);
    model.series_sample(1_000_000);
    assert_eq!(
        model.pnl_totals().unrealized,
        -100,
        "the live figure does use the fallback mark"
    );

    // Two-sided again at 97: the next sample marks there.
    touch(&mut model, NEAR, 97, 99);
    model.series_stamp(2 * SEC, 2_000_000);
    model.series_sample(2_000_000);

    let totals: Vec<i128> = model.pnl_series().iter().map(|s| s.total).collect();
    assert_eq!(totals, vec![-10, -10, -30]);
}

#[test]
fn an_account_answer_ahead_of_its_fill_report_keeps_the_cost() {
    // The venue's answer already counts a fill on a farther venue
    // whose report is still in flight: the gap closes when it lands.
    let mut book = PnlBook::known();
    book.fill(Side::Buy, 100, 10, 0, 5);
    book.account(20, 9, 1_000_000);
    book.fill(Side::Buy, 102, 10, 0, 8);
    book.account(20, 10, 1_500_000);
    assert_eq!(book.position, 20);
    assert_eq!(book.cost_ticks(), Some(101), "cost stays known");
    // Long after, the same answer is not an unexplained difference.
    book.account(20, 11, 1_000_000 + 10 * ACCOUNT_GRACE_US);
    assert_eq!(book.cost_ticks(), Some(101));
}

#[test]
fn a_difference_that_changes_restarts_the_grace_period() {
    let mut book = PnlBook::known();
    book.account(30, 1, 0);
    book.account(35, 2, ACCOUNT_GRACE_US);
    assert_eq!(book.position, 0, "a new difference starts its own wait");
    book.account(35, 3, 2 * ACCOUNT_GRACE_US);
    assert_eq!(book.position, 35);
    assert_eq!(book.cost_ticks(), None);
}

#[test]
fn a_book_one_sided_for_longer_than_a_requote_does_not_freeze_samples() {
    let mut model = model_with_listings();
    touch(&mut model, NEAR, 99, 101);
    model.series_stamp(0, 0);
    fill(&mut model, NEAR, Side::Buy, 100, 10, 0, 1, 0);
    // The bid side goes away for good and trades keep printing lower.
    one_sided(&mut model, NEAR, 101);
    for second in 1..=5_u64 {
        let at = second * 1_000_000;
        model.series_trade(NEAR, 100 - i64::try_from(second).unwrap_or(0), 1, at);
        model.series_stamp(second * SEC, at);
        model.series_sample(at);
    }
    let totals: Vec<i128> = model.pnl_series().iter().map(|s| s.total).collect();
    // Seconds 1-2 keep the last two-sided bid (99); from second 3 the
    // sampled touch is stale and the last trade marks the position.
    assert_eq!(totals, vec![-10, -10, -10, -30, -40, -50]);
}

#[test]
fn continuous_bars_fill_quiet_periods_and_run_to_now() {
    let mut series = ListingSeries::default();
    series.trade(10 * SEC, 100, 2);
    series.trade(10 * SEC + 5, 103, 1);
    series.trade(13 * SEC, 101, 4);

    let bars = series.continuous_bars(SEC, 15 * SEC + 400);
    let starts: Vec<u64> = bars.iter().map(|bar| bar.start_ns / SEC).collect();
    assert_eq!(starts, vec![10, 11, 12, 13, 14, 15], "one bar per second");
    let flat = Bar {
        start_ns: 11 * SEC,
        open: 103,
        high: 103,
        low: 103,
        close: 103,
        volume: 0,
    };
    assert_eq!(bars[1], flat, "a quiet second holds the previous close");
    assert_eq!(bars[3].close, 101);
    assert_eq!(bars[5].close, 101, "the forming bar carries the last close");
    assert_eq!(bars[5].volume, 0);

    // Wider bars fold first, then fill.
    let five: Vec<u64> = series
        .continuous_bars(5 * SEC, 21 * SEC)
        .iter()
        .map(|bar| bar.start_ns / SEC)
        .collect();
    assert_eq!(five, vec![10, 15, 20]);
}

#[test]
fn continuous_bars_stay_bounded_after_a_long_silence() {
    let mut series = ListingSeries::default();
    series.trade(0, 100, 1);
    let bars = series.continuous_bars(SEC, 100_000 * SEC);
    assert_eq!(bars.len(), MAX_FINE_BARS);
    assert_eq!(bars.last().map(|bar| bar.start_ns), Some(100_000 * SEC));
    assert!(bars.iter().all(|bar| bar.close == 100));
    assert!(
        ListingSeries::default()
            .continuous_bars(SEC, SEC)
            .is_empty()
    );
}

fn connect(model: &mut Model, run_id: u128, run_sequence: u64) {
    use bunting_client::bnp_wire::{ResumeStatus, Role, Welcome};
    let _ = model.apply(
        crate::model::SessionEvent::Connected(Welcome {
            version: 1,
            run_id,
            participant_id: 1,
            role: Role::Participant,
            heartbeat_ms: 1_000,
            max_frame_bytes: 65_536,
            resume: ResumeStatus::Replaying,
            run_sequence,
        }),
        0,
    );
}

#[test]
fn a_ledger_record_restores_cost_and_pnl_after_a_relaunch() -> Result<(), String> {
    let mut first = model_with_listings();
    connect(&mut first, 7, 0);
    fill(&mut first, NEAR, Side::Buy, 100, 10, 0, 5, 0);
    fill(&mut first, NEAR, Side::Buy, 104, 10, 1, 6, 0);
    first.last_sequence = 6;
    let record = first.ledger_record().ok_or("a record once a book exists")?;
    let restored = LedgerRecord::from_json(&record.to_json()).ok_or("round trip")?;
    assert_eq!(restored, record);

    // Relaunch on the same run: the books come back, a replayed fill at
    // or below the cursor is not counted twice, a later one is.
    let mut second = model_with_listings();
    second.preload_ledger(restored.clone());
    connect(&mut second, 7, 9);
    assert_eq!(second.average_cost(7), Some((2_040, 20)));
    fill(&mut second, NEAR, Side::Buy, 104, 10, 1, 6, 0);
    assert_eq!(
        second.average_cost(7),
        Some((2_040, 20)),
        "duplicate skipped"
    );
    fill(&mut second, NEAR, Side::Sell, 110, 20, 0, 8, 0);
    assert_eq!(second.realized_pnl(7), 160 - 1, "8 x 20 lots less the fee");

    // A different run (or one behind the cursor) ignores the record.
    let mut other = model_with_listings();
    other.preload_ledger(restored.clone());
    connect(&mut other, 8, 100);
    assert_eq!(other.average_cost(7), None);
    let mut fresh = model_with_listings();
    fresh.preload_ledger(restored);
    connect(&mut fresh, 7, 3);
    assert_eq!(
        fresh.average_cost(7),
        None,
        "run 7 restarted below the cursor"
    );
    Ok(())
}
