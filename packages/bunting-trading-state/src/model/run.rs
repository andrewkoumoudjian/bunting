//! The run stream (ADR 0042) and deals (ADR 0043) in the model: run
//! status, listing halts, news, tenders and OTC negotiations, each kept as
//! the venue last committed it, and the requests that answer tenders and
//! negotiate.

use super::{Model, NoticeLevel};
use bunting_client::bnp_wire::{
    ClientMessage, NegotiationInfo, NegotiationStatus, OtcDecision, ServerMessage, Side,
    TenderInfo, TenderStatus,
};
use std::collections::VecDeque;

/// News items kept; older ones are dropped first.
pub const MAX_NEWS: usize = 256;
/// Tenders and negotiations kept each; finished ones are dropped first.
pub const MAX_DEALS: usize = 512;

/// One news item, as the venue committed it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewsItem {
    pub news_id: u128,
    pub headline: String,
    pub body: String,
    /// Run time of the commit.
    pub logical_time_ns: u64,
    pub received_us: u64,
}

/// What the venue last said about a tender or negotiation, and the
/// committed sequence that said it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Dealt<T> {
    pub info: T,
    pub sequence: u64,
}

#[derive(Clone, Debug, Default)]
pub(super) struct RunState {
    status: Option<String>,
    halted: Vec<u128>,
    news: VecDeque<NewsItem>,
    tenders: Vec<Dealt<TenderInfo>>,
    negotiations: Vec<Dealt<NegotiationInfo>>,
    /// Latest run time any stamped report carried.
    run_time_ns: u64,
}

const fn tender_open(info: &TenderInfo) -> bool {
    matches!(info.status, TenderStatus::Open)
}

const fn negotiation_open(info: &NegotiationInfo) -> bool {
    matches!(
        info.status,
        NegotiationStatus::Proposed | NegotiationStatus::Countered
    )
}

/// Keeps `info` if it is newer than what is held for its key, dropping the
/// oldest finished entry (else the oldest) past [`MAX_DEALS`].
fn upsert<T: Copy>(
    list: &mut Vec<Dealt<T>>,
    info: T,
    sequence: u64,
    key: impl Fn(&T) -> u128,
    open: impl Fn(&T) -> bool,
) {
    let id = key(&info);
    if let Some(held) = list.iter_mut().find(|held| key(&held.info) == id) {
        if sequence >= held.sequence {
            *held = Dealt { info, sequence };
        }
        return;
    }
    if list.len() >= MAX_DEALS {
        let drop = list.iter().position(|held| !open(&held.info)).unwrap_or(0);
        list.remove(drop);
    }
    list.push(Dealt { info, sequence });
}

impl Model {
    // ---- reading ---------------------------------------------------------

    /// The run's lifecycle as the venue last reported it (`active`,
    /// `paused`, `terminated`, …); `None` before any report.
    #[must_use]
    pub fn run_status(&self) -> Option<&str> {
        self.run.status.as_deref()
    }

    /// Whether every listing of `instrument_id` is halted.
    #[must_use]
    pub fn is_halted(&self, instrument_id: u128) -> bool {
        self.run.halted.contains(&instrument_id)
    }

    /// News for this session, oldest first.
    #[must_use]
    pub const fn news(&self) -> &VecDeque<NewsItem> {
        &self.run.news
    }

    /// Tenders offered to this participant, oldest first.
    #[must_use]
    pub fn tenders(&self) -> &[Dealt<TenderInfo>] {
        &self.run.tenders
    }

    /// OTC negotiations this participant is party to, oldest first.
    #[must_use]
    pub fn negotiations(&self) -> &[Dealt<NegotiationInfo>] {
        &self.run.negotiations
    }

    /// The latest run time a committed report carried. The run clock has
    /// moved on since; this is when the venue last said anything.
    #[must_use]
    pub const fn run_time_ns(&self) -> u64 {
        self.run.run_time_ns
    }

    // ---- requests --------------------------------------------------------

    /// Reloads every visible tender and negotiation.
    pub fn request_deals(&mut self) -> ClientMessage {
        ClientMessage::DealsRequest {
            request_id: self.request_id(),
        }
    }

    /// Accepts or declines a tender. The venue answers `DealDone` once the
    /// decision commits, then the tender's new state.
    pub fn decide_tender(&mut self, tender_id: u128, accept: bool, now_us: u64) -> ClientMessage {
        self.notice(
            now_us,
            NoticeLevel::Info,
            format!(
                "tender {tender_id}: {} sent",
                if accept { "accept" } else { "decline" }
            ),
        );
        ClientMessage::TenderDecision {
            request_id: self.deal_request_id(),
            tender_id,
            accept,
        }
    }

    /// Proposes an OTC trade: this participant takes `side`.
    #[expect(
        clippy::too_many_arguments,
        reason = "one argument per field of the proposal"
    )]
    pub fn propose_otc(
        &mut self,
        counterparty_id: u128,
        instrument_id: u128,
        side: Side,
        quantity: i64,
        price: i64,
        expires_at_ns: u64,
        now_us: u64,
    ) -> ClientMessage {
        self.notice(
            now_us,
            NoticeLevel::Info,
            format!("OTC proposal to participant {counterparty_id} sent"),
        );
        // The client's own negotiation ID: unique for the run, like a
        // client order ID; the venue names the negotiation from it.
        let negotiation_id = self.deal_request_id();
        ClientMessage::OtcPropose {
            request_id: self.deal_request_id(),
            negotiation_id,
            counterparty_id,
            instrument_id,
            side,
            quantity,
            price,
            expires_at_ns,
        }
    }

    /// Counters the negotiation awaiting this participant.
    pub fn counter_otc(
        &mut self,
        negotiation_id: u128,
        quantity: i64,
        price: i64,
    ) -> ClientMessage {
        ClientMessage::OtcCounter {
            request_id: self.deal_request_id(),
            negotiation_id,
            quantity,
            price,
        }
    }

    /// Accepts, rejects or breaks a negotiation.
    pub fn decide_otc(&mut self, negotiation_id: u128, decision: OtcDecision) -> ClientMessage {
        ClientMessage::OtcDecision {
            request_id: self.deal_request_id(),
            negotiation_id,
            decision,
        }
    }

    /// Request IDs for deals are unique per participant for the run (a
    /// reused one commits nothing), so they come from the same run-unique
    /// counter as client order IDs.
    fn deal_request_id(&mut self) -> u64 {
        let id = self.next_client_order_id;
        self.next_client_order_id = self.next_client_order_id.saturating_add(1);
        id
    }

    // ---- applying --------------------------------------------------------

    /// Applies a run-stream or deals message; others are ignored.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per run-stream or deals message, each a few lines"
    )]
    pub(super) fn run_message(&mut self, message: ServerMessage, now_us: u64) {
        if let Some(stamp) = message.stamp() {
            self.run.run_time_ns = self.run.run_time_ns.max(stamp.logical_time_ns);
        }
        match message {
            ServerMessage::RunStatus { status, .. } => {
                self.notice(now_us, NoticeLevel::Info, format!("run {status}"));
                self.run.status = Some(status);
            }
            ServerMessage::ListingHalt {
                instrument_id,
                halted,
                ..
            } => {
                self.run.halted.retain(|id| *id != instrument_id);
                if halted {
                    self.run.halted.push(instrument_id);
                }
                let symbol = self
                    .instruments()
                    .into_iter()
                    .find(|instrument| instrument.instrument_id == instrument_id)
                    .map_or_else(|| instrument_id.to_string(), |instrument| instrument.symbol);
                self.notice(
                    now_us,
                    NoticeLevel::Warning,
                    format!(
                        "{symbol} {} on every venue",
                        if halted { "halted" } else { "resumed" }
                    ),
                );
            }
            ServerMessage::News {
                stamp,
                news_id,
                headline,
                body,
            } => {
                if self.run.news.iter().any(|item| item.news_id == news_id) {
                    return;
                }
                if self.run.news.len() >= MAX_NEWS {
                    self.run.news.pop_front();
                }
                self.notice(now_us, NoticeLevel::Info, format!("news: {headline}"));
                self.run.news.push_back(NewsItem {
                    news_id,
                    headline,
                    body,
                    logical_time_ns: stamp.logical_time_ns,
                    received_us: now_us,
                });
            }
            ServerMessage::Tender { stamp, tender } => {
                if tender_open(&tender)
                    && !self
                        .run
                        .tenders
                        .iter()
                        .any(|held| held.info.tender_id == tender.tender_id)
                {
                    self.notice(
                        now_us,
                        NoticeLevel::Warning,
                        format!(
                            "tender offered: {} {} at {}",
                            match tender.side {
                                Side::Buy => "buy",
                                Side::Sell => "sell",
                            },
                            tender.quantity,
                            tender.price
                        ),
                    );
                }
                upsert(
                    &mut self.run.tenders,
                    tender,
                    stamp.sequence,
                    |info| info.tender_id,
                    tender_open,
                );
            }
            ServerMessage::Negotiation { stamp, negotiation } => upsert(
                &mut self.run.negotiations,
                negotiation,
                stamp.sequence,
                |info| info.negotiation_id,
                negotiation_open,
            ),
            ServerMessage::Deals {
                run_sequence,
                tenders,
                negotiations,
                ..
            } => {
                for tender in tenders {
                    upsert(
                        &mut self.run.tenders,
                        tender,
                        run_sequence,
                        |info| info.tender_id,
                        tender_open,
                    );
                }
                for negotiation in negotiations {
                    upsert(
                        &mut self.run.negotiations,
                        negotiation,
                        run_sequence,
                        |info| info.negotiation_id,
                        negotiation_open,
                    );
                }
            }
            ServerMessage::DealDone { run_sequence, .. } => self.notice(
                now_us,
                NoticeLevel::Info,
                format!("deal request committed at sequence {run_sequence}"),
            ),
            _ => {}
        }
    }
}
