//! Share of input read from cache, one value per source and never blended.
//! Pure: no I/O and no copy.
//!
//! Claude reports cache reads and writes as separate input categories; Codex
//! and the OpenAI-shaped ledger facade report cached input as a subset of
//! input. Each is normalized here, so a shell never does the arithmetic.

use serde::{Deserialize, Serialize};

use super::analytics_constants::BEST_WEEK_MIN_EARLIER_WEEKS;

/// Which wire shape a ledger call's counters came in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerFacade {
    /// Cache read and write are separate from input.
    Anthropic,
    /// Cached input is included in input.
    OpenAi,
}

/// Input read from cache over all input, kept as its parts for drill-down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheShare {
    pub numerator: u64,
    pub denominator: u64,
}

impl CacheShare {
    fn new(numerator: u64, denominator: u64) -> Option<Self> {
        (denominator > 0 && numerator <= denominator).then_some(Self {
            numerator,
            denominator,
        })
    }

    /// cache_read / (uncached + cache_read + cache_write).
    pub fn claude(uncached: u64, cache_read: u64, cache_write: u64) -> Option<Self> {
        Self::new(cache_read, uncached + cache_read + cache_write)
    }

    /// cached_input / input; cached input is a subset of input.
    pub fn codex(input: u64, cached_input: u64) -> Option<Self> {
        Self::new(cached_input, input)
    }

    pub fn ledger(
        facade: LedgerFacade,
        input: u64,
        cache_read: u64,
        cache_write: u64,
    ) -> Option<Self> {
        match facade {
            LedgerFacade::Anthropic => Self::claude(input, cache_read, cache_write),
            LedgerFacade::OpenAi => Self::codex(input, cache_read),
        }
    }

    /// Per mille, rounded half up.
    pub fn permille(&self) -> u64 {
        let numerator = u128::from(self.numerator) * 2_000 + u128::from(self.denominator);
        (numerator / (u128::from(self.denominator) * 2)) as u64
    }

    pub fn merge(&mut self, other: CacheShare) {
        self.numerator += other.numerator;
        self.denominator += other.denominator;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BestWeekUnavailable {
    InsufficientHistory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BestWeek {
    pub previous_best_permille: u64,
    pub is_new_best: bool,
}

/// "Your best week" against earlier weeks, where `None` is a week that was
/// not comparable. The caller passes this week only when it is comparable.
pub fn best_week(
    this: CacheShare,
    earlier: &[Option<CacheShare>],
) -> Result<BestWeek, BestWeekUnavailable> {
    let comparable: Vec<u64> = earlier.iter().flatten().map(CacheShare::permille).collect();
    if comparable.len() < BEST_WEEK_MIN_EARLIER_WEEKS {
        return Err(BestWeekUnavailable::InsufficientHistory);
    }
    let previous_best_permille = comparable.into_iter().max().unwrap_or(0);
    Ok(BestWeek {
        previous_best_permille,
        is_new_best: this.permille() > previous_best_permille,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_share_counts_writes_in_the_denominator() {
        let share = CacheShare::claude(100, 600, 300).unwrap();
        assert_eq!(share.numerator, 600);
        assert_eq!(share.denominator, 1_000);
        assert_eq!(share.permille(), 600);
    }

    #[test]
    fn codex_cached_input_is_a_subset_of_input() {
        let share = CacheShare::codex(1_000, 250).unwrap();
        assert_eq!(share.permille(), 250);
        // A subset larger than its whole is inconsistent, not 100%.
        assert_eq!(CacheShare::codex(100, 101), None);
    }

    #[test]
    fn an_empty_denominator_is_unavailable_not_zero() {
        assert_eq!(CacheShare::claude(0, 0, 0), None);
        assert_eq!(CacheShare::codex(0, 0), None);
    }

    #[test]
    fn ledger_facades_normalize_differently() {
        // OpenAI facade: cached is inside input.
        let openai = CacheShare::ledger(LedgerFacade::OpenAi, 1_000, 400, 0).unwrap();
        assert_eq!(openai.permille(), 400);
        // Anthropic facade: cache read and write are separate from input.
        let anthropic = CacheShare::ledger(LedgerFacade::Anthropic, 1_000, 400, 600).unwrap();
        assert_eq!(anthropic.permille(), 200);
        assert_eq!(CacheShare::ledger(LedgerFacade::OpenAi, 10, 11, 0), None);
    }

    #[test]
    fn permille_rounds_half_up() {
        assert_eq!(CacheShare::codex(2_000, 1).unwrap().permille(), 1); // 0.5
        assert_eq!(CacheShare::codex(3_000, 1).unwrap().permille(), 0); // 0.33
        assert_eq!(CacheShare::codex(3, 2).unwrap().permille(), 667);
    }

    #[test]
    fn shares_merge_by_summing_parts() {
        let mut total = CacheShare::codex(100, 50).unwrap();
        total.merge(CacheShare::codex(300, 50).unwrap());
        assert_eq!(total.permille(), 250);
    }

    fn weeks(permilles: &[Option<u64>]) -> Vec<Option<CacheShare>> {
        permilles
            .iter()
            .map(|p| p.map(|p| CacheShare::codex(1_000, p).unwrap()))
            .collect()
    }

    #[test]
    fn best_week_needs_three_earlier_comparable_weeks() {
        let this = CacheShare::codex(1_000, 700).unwrap();
        assert_eq!(
            best_week(this, &weeks(&[Some(600), Some(500), None])),
            Err(BestWeekUnavailable::InsufficientHistory)
        );
        let best = best_week(this, &weeks(&[Some(600), Some(500), None, Some(650)])).unwrap();
        assert_eq!(
            best,
            BestWeek {
                previous_best_permille: 650,
                is_new_best: true
            }
        );
        assert_eq!(BEST_WEEK_MIN_EARLIER_WEEKS, 3);
    }

    #[test]
    fn matching_the_previous_best_is_not_a_new_best() {
        let this = CacheShare::codex(1_000, 650).unwrap();
        let best = best_week(this, &weeks(&[Some(650), Some(500), Some(400)])).unwrap();
        assert!(!best.is_new_best);
    }
}
