//! ═══════════════════════════════════════════════════════════════════════
//!  RIT MARKET MAKING ENGINE
//! ═══════════════════════════════════════════════════════════════════════
//!
//!  Academic foundations implemented:
//!
//!  1. Cont–Stoikov–Talreja (2010): Laplace-transform–derived queue
//!     imbalance signals → fill-probability estimates → quote placement.
//!
//!  2. Fourier / FFT spectral analysis on order-flow time-series →
//!     signal-vs-noise ratio → adaptive spread scaling.
//!
//!  3. GARCH(1,1) conditional volatility on mid-price returns →
//!     dynamic spread width (Engle & Patton, 2001).
//!
//!  4. Avellaneda–Stoikov (2008) reservation-price + optimal spread
//!     with inventory skew, fed by (1)–(3).
//!
//!  Targets the RIT REST API v1 (localhost:9999).
//! ═══════════════════════════════════════════════════════════════════════

use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use std::collections::VecDeque;
use std::f64::consts::PI;
use std::{thread, time::Duration};

// ═══════════════════════════════════════════════════════════════════════
//  CONFIGURATION
// ═══════════════════════════════════════════════════════════════════════

const API_KEY: &str = "YOUR_API_KEY";
const BASE_URL: &str = "http://localhost:9999/v1";
const TICKER: &str = "ALGO";

// -- Avellaneda–Stoikov knobs --
const GAMMA: f64 = 0.05; // risk-aversion
const KAPPA: f64 = 1.5; // mkt-order arrival intensity
const ORDER_SIZE: i32 = 100; // shares per quote leg
const TICK_SIZE: f64 = 0.01; // minimum price increment

// -- position limits --
const MAX_POSITION: i64 = 5000;
const POS_LIMIT_BUFFER: i64 = 200; // stop quoting near hard limit

// -- GARCH(1,1) σ²_t = ω + α·r²_{t-1} + β·σ²_{t-1} --
const GARCH_OMEGA: f64 = 1e-6;
const GARCH_ALPHA: f64 = 0.10;
const GARCH_BETA: f64 = 0.85;
const GARCH_VAR_FLOOR: f64 = 1e-8;

// -- Fourier --
const FFT_WINDOW: usize = 128; // power of 2
const NOISE_CUTOFF_FRAC: f64 = 0.30; // above this fraction of Nyquist = noise

// -- Queue model --
const RATE_DECAY: f64 = 0.94; // EWMA decay for λ / θ / μ estimation

// -- Loop --
const LOOP_MS: u64 = 250;

// -- Spread clamps (in ticks) --
const MIN_SPREAD_TICKS: f64 = 1.0;
const MAX_SPREAD_TICKS: f64 = 60.0;
const MAX_QUOTE_DISTANCE_TICKS: f64 = 1.0; // Keep quotes at most one tick off the touch.
const MAX_DIRECTIONAL_SKEW_TICKS: f64 = 1.0; // Queue signal can move the centre by at most one tick.
const INVENTORY_SKEW_TICKS: f64 = 6.0; // Large enough to move quotes before hard limits bind.
const MIN_QUEUE_UNIT_SHARES: f64 = 100.0; // Queue model works in lots, not single-share states.

// -- history depths --
const MAX_MID_HISTORY: usize = 600;

// ═══════════════════════════════════════════════════════════════════════
//  API RESPONSE TYPES
// ═══════════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, Clone)]
struct CaseResp {
    #[serde(rename = "period", default)]
    _period: u32,
    #[serde(default)]
    tick: u32,
    #[serde(default)]
    ticks_per_period: u32,
    #[serde(rename = "total_periods", default)]
    _total_periods: u32,
    #[serde(default)]
    status: String,
}

#[derive(Debug, Deserialize, Clone)]
struct SecResp {
    ticker: String,
    #[serde(default)]
    position: i64,
    #[serde(rename = "last", default)]
    _last: f64,
    #[serde(rename = "bid", default)]
    _bid: f64,
    #[serde(rename = "bid_size", default)]
    _bid_size: i64,
    #[serde(rename = "ask", default)]
    _ask: f64,
    #[serde(rename = "ask_size", default)]
    _ask_size: i64,
    #[serde(default)]
    volume: i64,
    #[serde(default)]
    is_tradeable: bool,
    #[serde(default)]
    limits: Vec<i64>,
}

#[derive(Debug, Deserialize, Clone)]
struct BookLevel {
    price: f64,
    quantity: i64,
    #[serde(rename = "quantity_filled", default)]
    _quantity_filled: i64,
}

#[derive(Debug, Deserialize, Clone)]
struct BookResp {
    #[serde(default)]
    bids: Vec<BookLevel>,
    #[serde(default)]
    asks: Vec<BookLevel>,
}

#[derive(Debug, Deserialize, Clone)]
struct OrderResp {
    order_id: u64,
    #[serde(default)]
    ticker: String,
    #[serde(default)]
    action: String,
    #[serde(default)]
    price: f64,
    #[serde(default)]
    quantity: i32,
    #[serde(rename = "quantity_filled", default)]
    _quantity_filled: i32,
    #[serde(rename = "status", default)]
    _status: String,
}

#[derive(Debug, Deserialize, Clone)]
struct HistBar {
    #[serde(rename = "tick")]
    _tick: u32,
    #[serde(default)]
    close: f64,
}

#[derive(Debug, Deserialize, Clone)]
struct PnlResp {
    #[serde(default)]
    position: i64,
    #[serde(default)]
    avg_entry_price: f64,
    #[serde(default)]
    realized_pnl: f64,
    #[serde(default)]
    unrealized_pnl: f64,
    #[serde(default)]
    total_pnl: f64,
    #[serde(rename = "cash", default)]
    _cash: f64,
    #[serde(rename = "mark_price", default)]
    _mark_price: f64,
}

// ═══════════════════════════════════════════════════════════════════════
//  COMPLEX ARITHMETIC + RADIX-2 FFT
// ═══════════════════════════════════════════════════════════════════════

#[derive(Clone, Copy)]
struct C64 {
    re: f64,
    im: f64,
}
impl C64 {
    fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }
    fn mag_sq(self) -> f64 {
        self.re * self.re + self.im * self.im
    }
}
impl std::ops::Add for C64 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self {
            re: self.re + o.re,
            im: self.im + o.im,
        }
    }
}
impl std::ops::Sub for C64 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self {
            re: self.re - o.re,
            im: self.im - o.im,
        }
    }
}
impl std::ops::Mul for C64 {
    type Output = Self;
    fn mul(self, o: Self) -> Self {
        Self {
            re: self.re * o.re - self.im * o.im,
            im: self.re * o.im + self.im * o.re,
        }
    }
}

/// Cooley–Tukey radix-2 DIT FFT.  `buf.len()` **must** be a power of 2.
fn fft(buf: &[C64]) -> Vec<C64> {
    let n = buf.len();
    if n == 1 {
        return buf.to_vec();
    }
    let evn: Vec<C64> = buf.iter().step_by(2).copied().collect();
    let odd: Vec<C64> = buf.iter().skip(1).step_by(2).copied().collect();
    let fe = fft(&evn);
    let fo = fft(&odd);
    let mut out = vec![C64::new(0.0, 0.0); n];
    for k in 0..n / 2 {
        let ang = -2.0 * PI * (k as f64) / (n as f64);
        let tw = C64::new(ang.cos(), ang.sin()) * fo[k];
        out[k] = fe[k] + tw;
        out[k + n / 2] = fe[k] - tw;
    }
    out
}

// ═══════════════════════════════════════════════════════════════════════
//  GARCH(1,1) MODEL
// ═══════════════════════════════════════════════════════════════════════
//
//  σ²_t = ω + α · r²_{t-1} + β · σ²_{t-1}
//
//  The conditional vol feeds directly into the Avellaneda–Stoikov
//  spread formula: δ* = γσ²τ + (2/γ)·ln(1 + γ/κ).
// ═══════════════════════════════════════════════════════════════════════

struct GarchModel {
    omega: f64,
    alpha: f64,
    beta: f64,
    variance: f64,
    prev_ret: f64,
    warm: bool,
}

impl GarchModel {
    fn new() -> Self {
        Self {
            omega: GARCH_OMEGA,
            alpha: GARCH_ALPHA,
            beta: GARCH_BETA,
            variance: 1e-5, // sensible init
            prev_ret: 0.0,
            warm: false,
        }
    }

    /// Feed a new log-return and advance variance.
    fn update(&mut self, ret: f64) {
        if !self.warm {
            // bootstrap: use squared return as initial var
            self.variance = ret * ret + self.omega;
            self.warm = true;
        } else {
            self.variance =
                self.omega + self.alpha * self.prev_ret.powi(2) + self.beta * self.variance;
        }
        if self.variance < GARCH_VAR_FLOOR {
            self.variance = GARCH_VAR_FLOOR;
        }
        self.prev_ret = ret;
    }

    fn sigma(&self) -> f64 {
        self.variance.sqrt()
    }

    /// Seed the model from a batch of returns (e.g. price history on start).
    fn seed(&mut self, returns: &[f64]) {
        for &r in returns {
            self.update(r);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  FOURIER SPECTRAL ANALYSER
// ═══════════════════════════════════════════════════════════════════════
//
//  Applied to the signed order-flow series (buy_vol − sell_vol proxied
//  by signed mid-price changes).  Outputs:
//
//  • noise_ratio ∈ (0,∞) : high-freq energy / low-freq energy.
//    Used as a multiplicative spread scale (more noise → wider spread).
//
//  • dominant_period : if a clear spectral peak exists it hints at
//    algorithmic order-slicing; we can anticipate flow reversals.
// ═══════════════════════════════════════════════════════════════════════

struct FourierAnalyser {
    buf: VecDeque<f64>,
    window: usize,
    cutoff_frac: f64,
    noise_ratio: f64,
    dominant_period: Option<f64>,
}

impl FourierAnalyser {
    fn new() -> Self {
        Self {
            buf: VecDeque::with_capacity(FFT_WINDOW + 1),
            window: FFT_WINDOW,
            cutoff_frac: NOISE_CUTOFF_FRAC,
            noise_ratio: 1.0,
            dominant_period: None,
        }
    }

    fn push(&mut self, val: f64) {
        self.buf.push_back(val);
        if self.buf.len() > self.window {
            self.buf.pop_front();
        }
    }

    /// Recompute PSD and derived metrics.  Call once per main-loop tick.
    fn analyse(&mut self) {
        if self.buf.len() < self.window {
            return; // not enough data yet
        }

        // build Hann-windowed complex input
        let n = self.window;
        let input: Vec<C64> = self
            .buf
            .iter()
            .rev()
            .take(n)
            .enumerate()
            .map(|(i, &v)| {
                let w = 0.5 * (1.0 - (2.0 * PI * i as f64 / (n as f64 - 1.0)).cos());
                C64::new(v * w, 0.0)
            })
            .collect();

        let spectrum = fft(&input);
        let half = n / 2;

        // power spectral density (one-sided)
        let psd: Vec<f64> = spectrum[1..=half].iter().map(|c| c.mag_sq()).collect();
        let total_energy: f64 = psd.iter().sum();
        if total_energy < 1e-30 {
            self.noise_ratio = 1.0;
            self.dominant_period = None;
            return;
        }

        let cutoff_bin = ((half as f64) * self.cutoff_frac).ceil() as usize;
        let low_energy: f64 = psd[..cutoff_bin.min(psd.len())].iter().sum();
        let high_energy: f64 = total_energy - low_energy;

        self.noise_ratio = if low_energy > 1e-30 {
            1.0 + (high_energy / low_energy).min(4.0) // clamp
        } else {
            3.0
        };

        // find dominant frequency (peak in low-freq band)
        let (peak_bin, _peak_pow) = psd[..cutoff_bin.min(psd.len())]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap_or((0, &0.0));

        self.dominant_period = if peak_bin > 0 {
            Some(n as f64 / (peak_bin + 1) as f64)
        } else {
            None
        };
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  CONT–STOIKOV–TALREJA QUEUE MODEL  (Laplace-transform framework)
// ═══════════════════════════════════════════════════════════════════════
//
//  Each best-level queue is a birth-death process:
//    birth  λ  (limit orders arriving)
//    death  nδ + θ  (n·cancel_rate + market_order_rate)
//
//  The Laplace transform of the first-passage-time to zero gives
//  closed-form fill- and direction-probabilities.  We use the key
//  outputs:
//
//  • queue_imbalance  QI = q_b^α / (q_b^α + q_a^α)
//        → P(mid ↑)   (the limiting result of the Laplace analysis
//          when rates are symmetric and proportional to queue size).
//
//  • fill_probability for a new order joining the back of the bid
//    or ask queue, given estimated θ, δ, λ arrival rates.
//
//  Rates are estimated online via EWMA on observed book changes +
//  volume deltas between snapshots.
// ═══════════════════════════════════════════════════════════════════════

struct QueueModel {
    // estimated Poisson rates (EWMA)
    lambda_bid: f64, // limit buy arrival rate
    lambda_ask: f64, // limit sell arrival rate
    theta: f64,      // market order rate (symmetric)
    mu: f64,         // cancellation rate per unit

    // previous snapshot state (for rate estimation)
    prev_bid_px: f64,
    prev_ask_px: f64,
    prev_bid_sz: f64,
    prev_ask_sz: f64,
    prev_volume: i64,
    avg_trade_size: f64,
    initialised: bool,

    // output signals
    qi: f64,             // queue imbalance  ∈ (0,1);  >0.5 → bid heavy → expect up
    fill_prob_bid: f64,  // P(our bid fills before mid moves)
    fill_prob_ask: f64,  // P(our ask fills before mid moves)
    spread_capture: f64, // P(both legs fill = round-trip)
}

impl QueueModel {
    fn new() -> Self {
        Self {
            lambda_bid: 5.0,
            lambda_ask: 5.0,
            theta: 3.0,
            mu: 2.0,
            prev_bid_px: 0.0,
            prev_ask_px: 0.0,
            prev_bid_sz: 0.0,
            prev_ask_sz: 0.0,
            prev_volume: 0,
            avg_trade_size: ORDER_SIZE as f64,
            initialised: false,
            qi: 0.5,
            fill_prob_bid: 0.5,
            fill_prob_ask: 0.5,
            spread_capture: 0.25,
        }
    }

    /// Update rate estimates and derived signals from a new book snapshot.
    fn update(&mut self, book: &BookResp, total_volume: i64) {
        let (bb, ba) = best_bid_ask(book);
        let (bsz, asz) = best_sizes(book);

        if !self.initialised {
            self.prev_bid_px = bb;
            self.prev_ask_px = ba;
            self.prev_bid_sz = bsz;
            self.prev_ask_sz = asz;
            self.prev_volume = total_volume;
            self.initialised = true;
            self.compute_signals(bsz, asz);
            return;
        }

        let d = RATE_DECAY;

        // --- estimate market-order rate from volume change ---
        let vol_delta = (total_volume - self.prev_volume).max(0) as f64;
        self.theta = d * self.theta + (1.0 - d) * vol_delta;
        if vol_delta > 0.0 {
            self.avg_trade_size = d * self.avg_trade_size + (1.0 - d) * vol_delta.max(1.0);
        }

        // --- estimate limit arrival & cancel rates from queue changes ---
        if (bb - self.prev_bid_px).abs() < TICK_SIZE * 0.5 {
            // bid price unchanged → queue changed by arrivals / cancels
            let dq = bsz - self.prev_bid_sz;
            if dq > 0.0 {
                self.lambda_bid = d * self.lambda_bid + (1.0 - d) * dq;
            } else {
                // departures net of any volume executed at bid
                let exec_est = (vol_delta * 0.5).min((-dq).max(0.0));
                let cancel_est = (-dq - exec_est).max(0.0);
                self.mu = d * self.mu + (1.0 - d) * cancel_est / self.prev_bid_sz.max(1.0);
            }
        } else {
            // bid price moved → level was fully consumed → high θ
            self.theta = d * self.theta + (1.0 - d) * self.prev_bid_sz;
        }

        if (ba - self.prev_ask_px).abs() < TICK_SIZE * 0.5 {
            let dq = asz - self.prev_ask_sz;
            if dq > 0.0 {
                self.lambda_ask = d * self.lambda_ask + (1.0 - d) * dq;
            }
        }

        // clamp rates to sane values
        self.lambda_bid = self.lambda_bid.clamp(0.1, 1e4);
        self.lambda_ask = self.lambda_ask.clamp(0.1, 1e4);
        self.theta = self.theta.clamp(0.1, 1e4);
        self.mu = self.mu.clamp(1e-4, 10.0);

        self.compute_signals(bsz, asz);

        // save snapshot
        self.prev_bid_px = bb;
        self.prev_ask_px = ba;
        self.prev_bid_sz = bsz;
        self.prev_ask_sz = asz;
        self.prev_volume = total_volume;
    }

    /// Compute the three key signals from current queue sizes + rates.
    fn compute_signals(&mut self, qb: f64, qa: f64) {
        // ── Queue Imbalance (CST limiting result) ──────────────────
        //  QI = q_b^α / (q_b^α + q_a^α)     α=1 → linear imbalance
        //  QI > 0.5 ⟹ more resting bids ⟹ support ⟹ expect ↑
        let alpha = 1.0; // can tune
        let qba = qb.max(1.0).powf(alpha);
        let qaa = qa.max(1.0).powf(alpha);
        self.qi = qba / (qba + qaa);

        // ── Fill probability (birth-death first-passage approx) ────
        //  For order at BACK of bid queue of depth qb:
        //
        //  P(fill) ≈ Π_{k=1}^{qb} θ / (θ + k·μ + λ_ask)
        //
        //  where λ_ask penalises the chance ask queue grows (absorbs
        //  selling pressure) before market sells reach us.
        //
        //  This is the discrete analog of the Laplace-transform ratio
        //  from CST (2010) §4.2, using the generating-function
        //  factorisation of the first-passage-time distribution.
        self.fill_prob_bid = self.fill_probability(qb, self.lambda_ask);
        self.fill_prob_ask = self.fill_probability(qa, self.lambda_bid);

        self.spread_capture = self.fill_prob_bid * self.fill_prob_ask;
    }

    /// Laplace-inspired fill probability for an order at the back
    /// of a queue of depth `q`, competing against arrival rate
    /// `lambda_opp` on the opposite side.
    fn fill_probability(&self, q: f64, lambda_opp: f64) -> f64 {
        // The CST birth-death chain is defined over queue states, not individual
        // shares. Collapse the visible queue into a small number of lot-sized
        // states so the first-passage approximation stays numerically stable on
        // simulator books with thousands of displayed shares.
        let queue_unit = self.avg_trade_size.max(MIN_QUEUE_UNIT_SHARES).max(q / 8.0);
        let n = (q / queue_unit).ceil().max(1.0) as usize;
        let service_rate = (self.theta * 0.5).max(1e-3);
        let lambda_opp = lambda_opp.max(1e-3);
        let cancel_chunk_rate =
            (self.mu * self.avg_trade_size.max(MIN_QUEUE_UNIT_SHARES)).max(1e-3);
        let mut log_prob = 0.0_f64;
        for k in 1..=n {
            let adverse_rate = lambda_opp + k as f64 * cancel_chunk_rate;
            log_prob += (service_rate / (service_rate + adverse_rate)).ln();
        }
        log_prob.exp().clamp(1e-3, 0.999)
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  RIT REST-API CLIENT
// ═══════════════════════════════════════════════════════════════════════

struct RitClient {
    client: Client,
    base: String,
}

impl RitClient {
    fn new() -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-API-Key",
            HeaderValue::from_str(API_KEY).expect("bad API key"),
        );
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(5))
            .build()
            .expect("http client init failed");
        Self {
            client,
            base: BASE_URL.to_string(),
        }
    }

    fn get_case(&self) -> Option<CaseResp> {
        self.client
            .get(format!("{}/case", self.base))
            .send()
            .ok()?
            .json()
            .ok()
    }

    fn get_security(&self) -> Option<SecResp> {
        let resp: Vec<SecResp> = self
            .client
            .get(format!("{}/securities", self.base))
            .query(&[("ticker", TICKER)])
            .send()
            .ok()?
            .json()
            .ok()?;
        resp.into_iter().find(|s| s.ticker == TICKER)
    }

    fn get_book(&self, depth: usize) -> Option<BookResp> {
        self.client
            .get(format!("{}/securities/book", self.base))
            .query(&[("ticker", TICKER), ("limit", &depth.to_string())])
            .send()
            .ok()?
            .json()
            .ok()
    }

    fn get_open_orders(&self) -> Vec<OrderResp> {
        self.client
            .get(format!("{}/orders", self.base))
            .query(&[("status", "OPEN")])
            .send()
            .ok()
            .and_then(|r| r.json().ok())
            .unwrap_or_default()
    }

    fn get_history(&self, limit: usize) -> Vec<HistBar> {
        self.client
            .get(format!("{}/securities/history", self.base))
            .query(&[("ticker", TICKER), ("limit", &limit.to_string())])
            .send()
            .ok()
            .and_then(|r| r.json().ok())
            .unwrap_or_default()
    }

    fn get_pnl(&self) -> Option<PnlResp> {
        self.client
            .get(format!("{}/pnl", self.base))
            .send()
            .ok()?
            .json()
            .ok()
    }

    fn place_limit(&self, action: &str, price: f64, qty: i32) -> Option<OrderResp> {
        let params = [
            ("ticker", TICKER.to_string()),
            ("type", "LIMIT".to_string()),
            ("quantity", qty.to_string()),
            ("price", format!("{:.2}", price)),
            ("action", action.to_string()),
        ];
        self.client
            .post(format!("{}/orders", self.base))
            .form(&params)
            .send()
            .ok()?
            .json()
            .ok()
    }

    fn cancel_order(&self, id: u64) -> bool {
        self.client
            .delete(format!("{}/orders/{}", self.base, id))
            .send()
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  HELPERS
// ═══════════════════════════════════════════════════════════════════════

/// Best bid and ask prices from order-book snapshot.
fn best_bid_ask(b: &BookResp) -> (f64, f64) {
    let bid = b.bids.first().map(|l| l.price).unwrap_or(0.0);
    let ask = b.asks.first().map(|l| l.price).unwrap_or(f64::MAX);
    (bid, ask)
}

/// Best bid and ask total sizes (first level).
fn best_sizes(b: &BookResp) -> (f64, f64) {
    let bsz = b.bids.first().map(|l| l.quantity as f64).unwrap_or(1.0);
    let asz = b.asks.first().map(|l| l.quantity as f64).unwrap_or(1.0);
    (bsz, asz)
}

/// Total visible depth on each side (all levels).
fn total_depth(b: &BookResp) -> (f64, f64) {
    let bd: f64 = b.bids.iter().map(|l| l.quantity as f64).sum();
    let ad: f64 = b.asks.iter().map(|l| l.quantity as f64).sum();
    (bd, ad)
}

/// Volume-weighted best-price estimate of the short-term fair value.
fn microprice(b: &BookResp) -> Option<f64> {
    let bid = b.bids.first()?;
    let ask = b.asks.first()?;
    let denom = (bid.quantity + ask.quantity) as f64;
    if denom <= 0.0 {
        return None;
    }
    Some((ask.price * bid.quantity as f64 + bid.price * ask.quantity as f64) / denom)
}

/// Round price to nearest tick.
fn round_tick(price: f64) -> f64 {
    (price / TICK_SIZE).round() * TICK_SIZE
}

// ═══════════════════════════════════════════════════════════════════════
//  MARKET MAKING ENGINE
// ═══════════════════════════════════════════════════════════════════════

struct MarketMaker {
    api: RitClient,
    garch: GarchModel,
    fourier: FourierAnalyser,
    queue: QueueModel,

    // price history
    mid_history: VecDeque<f64>,
    last_mid: f64,
    last_tick: u32,

    // tracking
    cycle: u64,
}

impl MarketMaker {
    fn new() -> Self {
        Self {
            api: RitClient::new(),
            garch: GarchModel::new(),
            fourier: FourierAnalyser::new(),
            queue: QueueModel::new(),
            mid_history: VecDeque::with_capacity(MAX_MID_HISTORY + 1),
            last_mid: 0.0,
            last_tick: 0,
            cycle: 0,
        }
    }

    // ───────────────────────────────────────────────────────────────
    //  BOOT-STRAP: seed GARCH from available price history
    // ───────────────────────────────────────────────────────────────
    fn seed_from_history(&mut self) {
        let bars = self.api.get_history(300);
        if bars.len() < 2 {
            return;
        }
        let closes: Vec<f64> = bars.iter().map(|b| b.close).filter(|&c| c > 0.0).collect();
        let returns: Vec<f64> = closes.windows(2).map(|w| (w[1] / w[0]).ln()).collect();
        self.garch.seed(&returns);
        for &c in &closes {
            self.mid_history.push_back(c);
        }
        if let Some(&last) = closes.last() {
            self.last_mid = last;
        }
        println!(
            "[BOOT] Seeded GARCH with {} returns → σ = {:.6}",
            returns.len(),
            self.garch.sigma()
        );
    }

    // ───────────────────────────────────────────────────────────────
    //  MAIN LOOP
    // ───────────────────────────────────────────────────────────────
    fn run(&mut self) {
        println!("══════════════════════════════════════════════════");
        println!("  RIT Market Making Engine    ticker={}", TICKER);
        println!("══════════════════════════════════════════════════");

        self.seed_from_history();

        loop {
            // 0. pacing
            thread::sleep(Duration::from_millis(LOOP_MS));

            // 1. case status
            let case = match self.api.get_case() {
                Some(c) => c,
                None => {
                    eprintln!("[WARN] cannot reach API – retrying…");
                    continue;
                }
            };
            if case.status != "ACTIVE" {
                println!("[WAIT] case status = {}", case.status);
                continue;
            }
            // avoid re-processing same tick if loop is faster than sim
            if case.tick == self.last_tick && self.cycle > 0 {
                continue;
            }
            self.last_tick = case.tick;

            // 2. security info (position, last, volume)
            let sec = match self.api.get_security() {
                Some(s) => s,
                None => continue,
            };
            if !sec.is_tradeable {
                continue;
            }

            // 3. order book
            let book = match self.api.get_book(20) {
                Some(b) if !b.bids.is_empty() && !b.asks.is_empty() => b,
                _ => continue,
            };

            // ── derived data ──────────────────────────────────────
            let (bb, ba) = best_bid_ask(&book);
            let mid = (bb + ba) / 2.0;
            if mid <= 0.0 {
                continue;
            }
            let last = sec._last;
            let micro = microprice(&book).unwrap_or(mid);
            let pnl = self.api.get_pnl();

            // 4. update mid history + GARCH
            if self.last_mid > 0.0 {
                let ret = (mid / self.last_mid).ln();
                self.garch.update(ret);
                self.fourier.push(ret);
            }
            self.last_mid = mid;
            self.mid_history.push_back(mid);
            if self.mid_history.len() > MAX_MID_HISTORY {
                self.mid_history.pop_front();
            }

            // 5. update Fourier analyser
            self.fourier.analyse();

            // 6. update queue model
            self.queue.update(&book, sec.volume);

            // ── compute optimal quotes ────────────────────────────
            let sigma = self.garch.sigma();
            let tau = (case.ticks_per_period.saturating_sub(case.tick)) as f64
                / case.ticks_per_period.max(1) as f64;
            let tau = tau.max(0.01); // avoid zero

            let position = sec.position;
            let step_horizon = 1.0 / case.ticks_per_period.max(1) as f64;
            let sigma_step_price = (mid * sigma * step_horizon.sqrt()).max(TICK_SIZE * 0.25);
            let inventory_frac = (position as f64 / MAX_POSITION as f64).clamp(-1.0, 1.0);
            let micro_signal = (micro - mid).clamp(
                -MAX_DIRECTIONAL_SKEW_TICKS * TICK_SIZE,
                MAX_DIRECTIONAL_SKEW_TICKS * TICK_SIZE,
            );

            // Inventory should move the quote centre before hard limits bind.
            // Use a position fraction rather than raw shares so the skew is
            // expressed in price ticks, not in share-count units.
            let inventory_shift =
                inventory_frac * INVENTORY_SKEW_TICKS * TICK_SIZE * (0.5 + 0.5 * tau);

            // Use one-step price volatility for spread sizing. The A-S term acts as
            // a liquidity premium on top of that volatility floor.
            let liquidity_term = (2.0 / KAPPA) * (1.0 + GAMMA).ln();
            let base_half_spread = 0.5 * TICK_SIZE + sigma_step_price + liquidity_term * 0.5;

            // ── Fourier noise adjustment ──────────────────────────
            //  Widen spread when high-frequency noise is elevated
            //  (more adverse selection → need more edge).
            let noise_scale = self.fourier.noise_ratio; // ≥ 1.0

            // ── Queue-model directional bias ──────────────────────
            //  QI > 0.5 ⟹ bid-heavy book → fair value above mid, and vice versa.
            let queue_shift =
                ((self.queue.qi - 0.5) * 2.0 * MAX_DIRECTIONAL_SKEW_TICKS * TICK_SIZE).clamp(
                    -MAX_DIRECTIONAL_SKEW_TICKS * TICK_SIZE,
                    MAX_DIRECTIONAL_SKEW_TICKS * TICK_SIZE,
                );

            // ── Fill-probability adjustment ───────────────────────
            //  Low fill probability widens that side by up to one extra tick.
            let fp_adj_bid = (1.0 - self.queue.fill_prob_bid).clamp(0.0, 1.0) * TICK_SIZE;
            let fp_adj_ask = (1.0 - self.queue.fill_prob_ask).clamp(0.0, 1.0) * TICK_SIZE;

            // ── combine into final quotes ─────────────────────────
            let centre = mid + micro_signal + queue_shift - inventory_shift;
            let half = base_half_spread * noise_scale;

            let raw_bid = centre - half - fp_adj_bid;
            let raw_ask = centre + half + fp_adj_ask;

            // enforce min / max spread
            let spread = raw_ask - raw_bid;
            let min_spread = MIN_SPREAD_TICKS * TICK_SIZE;
            let max_spread = MAX_SPREAD_TICKS * TICK_SIZE;
            let (adj_bid, adj_ask) = if spread < min_spread {
                let centre = (raw_bid + raw_ask) / 2.0;
                (centre - min_spread / 2.0, centre + min_spread / 2.0)
            } else if spread > max_spread {
                let centre = (raw_bid + raw_ask) / 2.0;
                (centre - max_spread / 2.0, centre + max_spread / 2.0)
            } else {
                (raw_bid, raw_ask)
            };

            let touch_clamp = MAX_QUOTE_DISTANCE_TICKS * TICK_SIZE;
            let clamped_bid = adj_bid.max(bb - touch_clamp).min(bb);
            let clamped_ask = adj_ask.min(ba + touch_clamp).max(ba);

            let mut bid_price = round_tick(clamped_bid);
            let mut ask_price = round_tick(clamped_ask);

            // sanity: bid must be below ask
            if bid_price >= ask_price {
                bid_price = bb;
                ask_price = ba;
            }

            // ── position-limit gating ─────────────────────────────
            let pos_limit = sec
                .limits
                .get(1)
                .copied()
                .unwrap_or(MAX_POSITION)
                .min(MAX_POSITION);
            let neg_limit = sec
                .limits
                .first()
                .copied()
                .unwrap_or(-MAX_POSITION)
                .max(-MAX_POSITION);

            let can_buy = position < (pos_limit - POS_LIMIT_BUFFER);
            let can_sell = position > (neg_limit + POS_LIMIT_BUFFER);

            // adaptive order size: reduce near limits
            let buy_size = if can_buy {
                let room = (pos_limit - position) as i32;
                ORDER_SIZE.min(room).max(0)
            } else {
                0
            };
            let sell_size = if can_sell {
                let room = (position - neg_limit) as i32;
                ORDER_SIZE.min(room).max(0)
            } else {
                0
            };

            // ── cancel stale orders ───────────────────────────────
            let open = self.api.get_open_orders();
            let mut kept_bid = false;
            let mut kept_ask = false;
            for o in &open {
                if o.ticker != TICKER {
                    continue;
                }
                let keep = match o.action.as_str() {
                    "BUY" if (o.price - bid_price).abs() <= TICK_SIZE * 1.5 && !kept_bid => {
                        kept_bid = true;
                        true
                    }
                    "SELL" if (o.price - ask_price).abs() <= TICK_SIZE * 1.5 && !kept_ask => {
                        kept_ask = true;
                        true
                    }
                    _ => false,
                };
                if !keep {
                    self.api.cancel_order(o.order_id);
                }
            }

            // refresh open list after cancels
            let open = self.api.get_open_orders();
            let have_bid = open.iter().any(|o| {
                o.ticker == TICKER
                    && o.action == "BUY"
                    && (o.price - bid_price).abs() <= TICK_SIZE * 1.5
            });
            let have_ask = open.iter().any(|o| {
                o.ticker == TICKER
                    && o.action == "SELL"
                    && (o.price - ask_price).abs() <= TICK_SIZE * 1.5
            });

            // ── place new quotes ──────────────────────────────────
            if !have_bid && buy_size > 0 {
                if let Some(resp) = self.api.place_limit("BUY", bid_price, buy_size) {
                    log_order("BID", &resp);
                }
            }
            if !have_ask && sell_size > 0 {
                if let Some(resp) = self.api.place_limit("SELL", ask_price, sell_size) {
                    log_order("ASK", &resp);
                }
            }

            // ── console dashboard ─────────────────────────────────
            self.cycle += 1;
            let (td_bid, td_ask) = total_depth(&book);
            let pnl_position = pnl.as_ref().map(|value| value.position).unwrap_or(position);
            let avg_entry = pnl
                .as_ref()
                .map(|value| value.avg_entry_price)
                .unwrap_or(0.0);
            let realized = pnl.as_ref().map(|value| value.realized_pnl).unwrap_or(0.0);
            let unrealized = pnl
                .as_ref()
                .map(|value| value.unrealized_pnl)
                .unwrap_or(0.0);
            let total_pnl = pnl.as_ref().map(|value| value.total_pnl).unwrap_or(0.0);
            println!(
                "[{:>5}] tick={:<4} mid={:.2}  σ={:.5}  τ={:.3}  \
                 bb/ba={:.2}/{:.2}  last={:.2}  micro={:.2}  \
                 QI={:.3}  FPb={:.3} FPa={:.3}  noise={:.2}  \
                 pos={:<6}  avg={:.2}  pnl R/U/T={:.2}/{:.2}/{:.2}  \
                 bid={:.2}×{}  ask={:.2}×{}  depth B/A={}/{}",
                self.cycle,
                case.tick,
                mid,
                sigma,
                tau,
                bb,
                ba,
                last,
                micro,
                self.queue.qi,
                self.queue.fill_prob_bid,
                self.queue.fill_prob_ask,
                self.fourier.noise_ratio,
                pnl_position,
                avg_entry,
                realized,
                unrealized,
                total_pnl,
                bid_price,
                buy_size,
                ask_price,
                sell_size,
                td_bid as i64,
                td_ask as i64,
            );
        }
    }
}

fn log_order(tag: &str, o: &OrderResp) {
    println!(
        "        → {} id={} {:.2}×{}",
        tag, o.order_id, o.price, o.quantity
    );
}

// ═══════════════════════════════════════════════════════════════════════
//  ENTRY POINT
// ═══════════════════════════════════════════════════════════════════════

fn main() {
    let mut mm = MarketMaker::new();
    mm.run();
}
