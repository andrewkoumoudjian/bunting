//! A deterministic fundamental-value path for the built-in agents.
//!
//! RIT-class cases (LT3, Volatility) move the price as
//! `P_t = P_{t-1} * (1 + r_t)` with `r_t ~ N(0, sigma_step)`, where the
//! annualised volatility is spread over the steps in a year and may change
//! on a schedule (spec `rit-case-primitives.md` section 7).
//!
//! **Determinism choice: fixed-point integers only, no floating point.**
//! Every draw is a pure function of `(seed, step index)` (counter-based
//! `SplitMix64`), and a normal draw is the Irwin-Hall sum of twelve uniforms
//! minus six (mean 0, variance exactly 1, tails truncated at 6 sigma). Floats
//! would need `ln`/`cos`/`sqrt` (Box-Muller), which libm implementations do
//! not promise to reproduce bit for bit across platforms; integers do.
//!
//! The price recursion is path dependent, so `fundamental(t)` is a pure
//! function of the config and `t` by recomputation from step zero. A single
//! cursor `(step, price)` is kept as a cache, extended while time moves
//! forward and rebuilt from the seed when asked for an earlier step or after
//! a restart (the cursor is never serialised), so results never depend on
//! history. Memory is O(1). Cost is O(steps) once per restart.

use bunting_market_types::PriceTicks;
use serde::{Deserialize, Serialize};

const Q: u32 = 32;
const ONE_Q: i128 = 1_i128 << Q;
/// Longest path a rebuild replays; later times hold the last price.
const MAX_STEPS: u64 = 200_000_000;
/// Regime entries a path may carry.
const MAX_REGIMES: usize = 64;
const DEFAULT_MAX_PRICE: i64 = 1_000_000_000;

/// How the agents' fundamental moves. All parameters are integers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FundamentalPath {
    /// Random walk with simple returns `P_t = P_{t-1} * (1 + r_t)`.
    RandomWalk {
        /// Seeds the counter-based draws.
        seed: u64,
        /// Logical nanoseconds between price moves (the price is constant
        /// within a step). Must be positive.
        step_ns: u64,
        /// Base annualised volatility in basis points (2500 = 25%).
        annual_volatility_bps: u32,
        /// Trading days in a year (252).
        trading_days_per_year: u32,
        /// Logical nanoseconds that make one trading day of the case.
        run_ns_per_day: u64,
        /// Volatility changes: from `from_ns` (strictly increasing, run
        /// time) the given annualised volatility applies.
        #[serde(default)]
        regimes: Vec<VolatilityRegime>,
        /// Lowest price in ticks (at least 1). Default 1.
        #[serde(default = "default_min")]
        min_price_ticks: i64,
        /// Highest price in ticks. Default one billion.
        #[serde(default = "default_max")]
        max_price_ticks: i64,
    },
}

const fn default_min() -> i64 {
    1
}
const fn default_max() -> i64 {
    DEFAULT_MAX_PRICE
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VolatilityRegime {
    pub from_ns: u64,
    pub annual_volatility_bps: u32,
}

impl FundamentalPath {
    #[must_use]
    pub fn is_valid(&self, start: PriceTicks) -> bool {
        let Self::RandomWalk {
            step_ns,
            trading_days_per_year,
            run_ns_per_day,
            regimes,
            min_price_ticks,
            max_price_ticks,
            ..
        } = self;
        *step_ns > 0
            && *trading_days_per_year > 0
            && *run_ns_per_day >= *step_ns
            && regimes.len() <= MAX_REGIMES
            && regimes
                .windows(2)
                .all(|pair| pair[0].from_ns < pair[1].from_ns)
            && *min_price_ticks >= 1
            && min_price_ticks <= max_price_ticks
            && (*min_price_ticks..=*max_price_ticks).contains(&start.get())
    }
}

/// `SplitMix64` over `seed + (counter + 1) * golden-gamma`.
fn splitmix(seed: u64, counter: u64) -> u64 {
    let mut z = seed.wrapping_add(counter.wrapping_add(1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Standard normal draw for a step in Q32 (Irwin-Hall, twelve uniforms).
fn normal_q32(seed: u64, step: u64) -> i64 {
    let mut sum: i64 = 0;
    for word in 0..6_u64 {
        let bits = splitmix(seed, step.wrapping_mul(6).wrapping_add(word));
        // Two 32-bit uniforms in [0, 2^32).
        sum += i64::from(u32::try_from(bits >> 32).unwrap_or(0));
        sum += i64::from(u32::try_from(bits & 0xFFFF_FFFF).unwrap_or(0));
    }
    sum - 6 * (1_i64 << Q)
}

/// Integer square root.
fn isqrt(value: u128) -> u128 {
    if value < 2 {
        return value;
    }
    let mut x = value;
    let mut y = u128::midpoint(x, 1);
    while y < x {
        x = y;
        y = u128::midpoint(x, value / x);
    }
    x
}

/// Per-step volatility in Q32 for an annualised volatility in bps.
fn step_sigma_q32(bps: u32, days: u32, ns_per_day: u64, step_ns: u64) -> i128 {
    let steps_per_year = (u128::from(days) * u128::from(ns_per_day) / u128::from(step_ns)).max(1);
    let root_q32 = isqrt(steps_per_year << (2 * Q)).max(1);
    let annual_q32 = (i128::from(bps) << Q) / 10_000;
    (annual_q32 << Q) / i128::try_from(root_q32).unwrap_or(i128::MAX)
}

/// Evaluates a [`FundamentalPath`]. The cursor is a cache only.
#[derive(Clone, Debug)]
pub struct FundamentalWalker {
    path: Option<FundamentalPath>,
    start: PriceTicks,
    /// `(step, price in Q32 ticks)` of the cache.
    cursor: (u64, i128),
}

impl FundamentalWalker {
    #[must_use]
    pub fn new(path: Option<FundamentalPath>, start: PriceTicks) -> Self {
        Self {
            path,
            start,
            cursor: (0, i128::from(start.get()) << Q),
        }
    }

    /// The fundamental at run time `now_ns`: a pure function of the config
    /// and `now_ns`, whatever was asked before.
    pub fn at(&mut self, now_ns: u64) -> PriceTicks {
        let Some(FundamentalPath::RandomWalk {
            seed,
            step_ns,
            annual_volatility_bps,
            trading_days_per_year,
            run_ns_per_day,
            regimes,
            min_price_ticks,
            max_price_ticks,
        }) = &self.path
        else {
            return self.start;
        };
        let target = (now_ns / step_ns).min(MAX_STEPS);
        if target < self.cursor.0 {
            self.cursor = (0, i128::from(self.start.get()) << Q);
        }
        let (mut step, mut price) = self.cursor;
        let lo = i128::from(*min_price_ticks) << Q;
        let hi = i128::from(*max_price_ticks) << Q;
        let sigma_for = |regime: usize| {
            let bps = regime
                .checked_sub(1)
                .map_or(*annual_volatility_bps, |i| regimes[i].annual_volatility_bps);
            step_sigma_q32(bps, *trading_days_per_year, *run_ns_per_day, *step_ns)
        };
        let regime_at =
            |step: u64| regimes.partition_point(|r| r.from_ns <= step.saturating_mul(*step_ns));
        let mut regime = regime_at(step);
        let mut sigma = sigma_for(regime);
        while step < target {
            // The move from `step` to `step + 1` uses the volatility in
            // force at the start of the step.
            let now_regime = regime_at(step);
            if now_regime != regime {
                regime = now_regime;
                sigma = sigma_for(regime);
            }
            let z = i128::from(normal_q32(*seed, step));
            let ret = ((sigma * z) >> Q).max(-(ONE_Q / 2));
            price = (price + ((price * ret) >> Q)).clamp(lo, hi);
            step += 1;
        }
        self.cursor = (step, price);
        let rounded = (price + (ONE_Q / 2)) >> Q;
        PriceTicks::new(i64::try_from(rounded).unwrap_or(*max_price_ticks))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: u64 = 500_000_000;

    fn walk(seed: u64, regimes: Vec<VolatilityRegime>, max: i64) -> FundamentalPath {
        FundamentalPath::RandomWalk {
            seed,
            step_ns: STEP,
            annual_volatility_bps: 4_000,
            trading_days_per_year: 252,
            run_ns_per_day: 60_000_000_000,
            regimes,
            min_price_ticks: 1,
            max_price_ticks: max,
        }
    }

    fn series(path: FundamentalPath, start: i64, steps: u64) -> Vec<i64> {
        let mut walker = FundamentalWalker::new(Some(path), PriceTicks::new(start));
        (0..steps).map(|k| walker.at(k * STEP).get()).collect()
    }

    #[test]
    fn same_config_same_path_and_different_seeds_differ() {
        let a = series(walk(1, vec![], 1_000_000), 100, 2_000);
        assert_eq!(a, series(walk(1, vec![], 1_000_000), 100, 2_000));
        assert_ne!(a, series(walk(2, vec![], 1_000_000), 100, 2_000));
    }

    #[test]
    fn absent_path_is_constant() {
        let mut walker = FundamentalWalker::new(None, PriceTicks::new(100));
        assert_eq!(walker.at(0).get(), 100);
        assert_eq!(walker.at(9_000_000_000_000).get(), 100);
    }

    #[test]
    fn a_restarted_walker_and_an_earlier_query_agree_with_the_original() {
        let path = walk(7, vec![], 1_000_000);
        let mut original = FundamentalWalker::new(Some(path.clone()), PriceTicks::new(100));
        let late = original.at(900_000_000_000);
        // Restart: a fresh walker (the cursor is not serialised).
        let mut restarted = FundamentalWalker::new(Some(path.clone()), PriceTicks::new(100));
        assert_eq!(restarted.at(900_000_000_000), late);
        // The past, asked later, rebuilds from the seed and agrees.
        let mut fresh = FundamentalWalker::new(Some(path), PriceTicks::new(100));
        let early = fresh.at(100_000_000_000);
        assert_eq!(original.at(100_000_000_000), early);
        assert_eq!(original.at(900_000_000_000), late);
    }

    /// Annualised standard deviation of per-step log returns, in bps,
    /// measured on a very high price so tick rounding is negligible.
    #[allow(clippy::float_arithmetic, clippy::cast_precision_loss)]
    fn realised_bps(path: FundamentalPath, from: u64, to: u64) -> f64 {
        let mut walker = FundamentalWalker::new(Some(path), PriceTicks::new(500_000_000));
        let prices: Vec<f64> = (from..=to)
            .map(|k| walker.at(k * STEP).get() as f64)
            .collect();
        let returns: Vec<f64> = prices.windows(2).map(|w| (w[1] / w[0]).ln()).collect();
        let n = returns.len() as f64;
        let mean = returns.iter().sum::<f64>() / n;
        let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n;
        let steps_per_year: f64 = 252.0 * 60.0 / 0.5;
        var.sqrt() * steps_per_year.sqrt() * 10_000.0
    }

    #[test]
    fn empirical_volatility_matches_the_configured_one() {
        let measured = realised_bps(walk(11, vec![], i64::MAX / 4), 0, 20_000);
        assert!((measured - 4_000.0).abs() < 200.0, "measured {measured}");
    }

    #[test]
    fn a_regime_change_changes_the_step_sizes() {
        let regimes = vec![VolatilityRegime {
            from_ns: 5_000_000_000_000,
            annual_volatility_bps: 8_000,
        }];
        let path = walk(11, regimes, i64::MAX / 4);
        let before = realised_bps(path.clone(), 0, 9_999);
        let after = realised_bps(path, 10_001, 20_000);
        assert!((before - 4_000.0).abs() < 300.0, "before {before}");
        assert!((after - 8_000.0).abs() < 600.0, "after {after}");
    }

    #[test]
    fn the_price_is_clamped_and_never_below_one_tick() {
        let path = FundamentalPath::RandomWalk {
            seed: 3,
            step_ns: STEP,
            annual_volatility_bps: 30_000,
            trading_days_per_year: 252,
            run_ns_per_day: 60_000_000_000,
            regimes: vec![],
            min_price_ticks: 90,
            max_price_ticks: 110,
        };
        let values = series(path, 100, 5_000);
        assert!(values.iter().all(|p| (90..=110).contains(p)));
        assert!(values.contains(&90) || values.contains(&110));
    }

    #[test]
    fn validation_rejects_bad_paths() {
        let bad = FundamentalPath::RandomWalk {
            seed: 1,
            step_ns: 0,
            annual_volatility_bps: 1,
            trading_days_per_year: 1,
            run_ns_per_day: 1,
            regimes: vec![],
            min_price_ticks: 1,
            max_price_ticks: 10,
        };
        assert!(!bad.is_valid(PriceTicks::new(5)));
        assert!(walk(1, vec![], 1_000_000).is_valid(PriceTicks::new(100)));
    }
}
