//! What a fresh session started from a summary at the long-context turn
//! would have sent. Built and tested, shown nowhere: every surface is held
//! by owner decision D2 (`ADVICE_SHOWN`). The summary size is a core
//! constant, never a request field.

use serde::{Deserialize, Serialize};
use trace_commons_protocol::insights_usage_series::TurnRecord;

use super::analytics_constants::{
    LONG_CONTEXT_TOKENS, WHAT_IF_MIN_TURNS_AFTER, WHAT_IF_SUMMARY_TOKENS,
};
use super::markers::{shrink_turns, usage_markers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatIf {
    /// N: the first turn whose context reached the long-context threshold.
    pub from_turn: u32,
    pub turns_after: usize,
    /// Input tokens the counterfactual session would not have sent.
    pub saving_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhatIfUnavailable {
    NeverLong,
    TooFewTurnsAfter,
    ShrinkAfter,
    UnknownTurn,
}

/// N is the first turn with context at or above [`LONG_CONTEXT_TOKENS`] and
/// S is [`WHAT_IF_SUMMARY_TOKENS`]. With
/// `new_input_u = max(0, context_u - context_{u-1})`, the growth a fresh
/// session would also carry, the saving is
/// `sum over t > N of max(0, context_t - (S + sum over N < u <= t of new_input_u))`.
pub fn what_if(turns: &[TurnRecord]) -> Result<WhatIf, WhatIfUnavailable> {
    let start = turns
        .iter()
        .position(|turn| turn.context().is_some_and(|c| c >= LONG_CONTEXT_TOKENS))
        .ok_or(WhatIfUnavailable::NeverLong)?;
    let after = &turns[start + 1..];
    if after.len() < WHAT_IF_MIN_TURNS_AFTER {
        return Err(WhatIfUnavailable::TooFewTurnsAfter);
    }
    let from_turn = turns[start].ordinal;
    if shrink_turns(&usage_markers(turns))
        .iter()
        .any(|turn| *turn > from_turn)
    {
        return Err(WhatIfUnavailable::ShrinkAfter);
    }
    let mut previous = turns[start].context().expect("found by context");
    let mut carried = 0u64;
    let mut saving = 0u64;
    for turn in after {
        let context = turn.context().ok_or(WhatIfUnavailable::UnknownTurn)?;
        carried += context.saturating_sub(previous);
        saving += context.saturating_sub(WHAT_IF_SUMMARY_TOKENS + carried);
        previous = context;
    }
    Ok(WhatIf {
        from_turn,
        turns_after: after.len(),
        saving_tokens: saving,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::analytics_constants::ADVICE_SHOWN;
    use crate::insights::markers::tests::ctx;

    fn turns(contexts: &[u32]) -> Vec<TurnRecord> {
        contexts
            .iter()
            .enumerate()
            .map(|(i, c)| ctx(i as u32, *c))
            .collect()
    }

    #[test]
    fn growing_context_saves_the_context_at_n_less_the_summary_each_turn() {
        let found = what_if(&turns(&[
            100_000, 150_000, 200_000, 210_000, 220_000, 230_000,
        ]))
        .unwrap();
        assert_eq!(found.from_turn, 2);
        assert_eq!(found.turns_after, 3);
        assert_eq!(found.saving_tokens, 3 * (200_000 - WHAT_IF_SUMMARY_TOKENS));
    }

    #[test]
    fn flat_context_saves_the_same_each_turn() {
        let found = what_if(&turns(&[250_000, 250_000, 250_000, 250_000])).unwrap();
        assert_eq!(found.from_turn, 0);
        assert_eq!(found.saving_tokens, 3 * (250_000 - 40_000));
    }

    #[test]
    fn a_drop_after_n_carries_no_new_input() {
        // 200K -> 150K is not a shrink (150K is above half), so it computes.
        let found = what_if(&turns(&[200_000, 150_000, 150_000, 160_000])).unwrap();
        // new_input: 0, 0, 10K. Savings: 110K, 110K, 160K - (40K + 10K).
        assert_eq!(found.saving_tokens, 110_000 + 110_000 + 110_000);
    }

    #[test]
    fn the_threshold_is_the_long_context_constant() {
        let below = LONG_CONTEXT_TOKENS as u32 - 1;
        assert_eq!(
            what_if(&turns(&[below, below, below, below])),
            Err(WhatIfUnavailable::NeverLong)
        );
    }

    #[test]
    fn fewer_than_three_turns_after_n_is_not_computed() {
        assert_eq!(WHAT_IF_MIN_TURNS_AFTER, 3);
        assert_eq!(
            what_if(&turns(&[250_000, 250_000, 250_000])),
            Err(WhatIfUnavailable::TooFewTurnsAfter)
        );
    }

    #[test]
    fn a_shrink_after_n_is_not_computed() {
        assert_eq!(
            what_if(&turns(&[250_000, 250_000, 100_000, 120_000])),
            Err(WhatIfUnavailable::ShrinkAfter)
        );
    }

    #[test]
    fn an_unknown_turn_after_n_is_not_computed() {
        let mut series = turns(&[250_000, 250_000, 250_000, 250_000]);
        series[2].cache_read = None;
        assert_eq!(what_if(&series), Err(WhatIfUnavailable::UnknownTurn));
    }

    #[test]
    fn it_is_held_from_every_surface() {
        const { assert!(!ADVICE_SHOWN) };
    }
}
