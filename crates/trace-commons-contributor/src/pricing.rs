//! What a step would have cost at the provider's published list price.
//!
//! This is a **price, not a bill**. It says what the tokens a transcript
//! reports would have cost on the provider's metered API at the rates in the
//! table below. Most coding sessions run under a subscription, where the
//! contributor was not charged per token at all, and none of them are billed
//! through this crate. No surface may render what this module returns as
//! money the contributor spent -- "list price", "would have cost", "priced
//! at" are honest; "you spent", "your bill", "cost you" are not.
//!
//! Three rules hold everywhere in here, and the tests enforce each one:
//!
//! 1. **A model absent from the table yields `None`.** Never the price of a
//!    similar model, never an average, never a default rate. A confidently
//!    wrong number costs belief in every other number on the screen.
//! 2. **A missing token count yields `None`, never a zero.** A fabricated
//!    zero silently understates.
//! 3. **Every figure carries its provenance.** See [`PRICES`].
//!
//! What this deliberately does *not* model: the Batch API's 50% discount,
//! fast mode's premium, priority tier, and the 1.1x `inference_geo: "us"`
//! multiplier. A caller that cannot rule those out must not build a
//! [`TokenUsage`] at all -- refusing to price is the correct answer, and the
//! adapters do exactly that.

use trace_commons_protocol::trace_contribution::Decimal;

/// The tokens one step consumed, as the provider reported them.
///
/// Every field is a count the provider actually stated. A caller that did not
/// see a field reported must not pass zero for it -- it must not build this
/// struct at all, because zero here is priced as "none were used", which is a
/// claim about the world rather than an absence of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Tokens served from an existing prompt cache (a "cache hit").
    pub cache_read_tokens: u32,
    /// Tokens written into a 5-minute prompt cache.
    pub cache_write_5m_tokens: u32,
    /// Tokens written into a 1-hour prompt cache. Priced at 2x base input
    /// against the 5-minute write's 1.25x, which is why the two durations
    /// are separate fields: a caller that knows only the combined figure
    /// cannot say what it cost, and must not guess the cheaper one.
    pub cache_write_1h_tokens: u32,
}

/// One model's list price, in micro-USD per million tokens.
///
/// Micro-USD (a millionth of a dollar) rather than a float: every published
/// rate is a whole number of micro-USD ($6.25/MTok is 6_250_000), so the
/// table holds each figure exactly, and the arithmetic below stays integer
/// until the final division.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ModelPrice {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write_5m: u64,
    cache_write_1h: u64,
}

/// Anthropic's published list prices, in micro-USD per million tokens.
///
/// **Source:** <https://platform.claude.com/docs/en/about-claude/pricing>,
/// the "Model pricing" table (columns: base input, 5m cache writes, 1h cache
/// writes, cache hits and refreshes, output tokens).
/// **Fetched:** 2026-09-02.
///
/// Prices change. When they do, an entry here becomes wrong silently -- there
/// is no runtime check that can catch it -- so re-read the source page before
/// trusting a figure much older than its fetch date, and update the date when
/// you do.
///
/// Keys are Claude API model ids with any dated-snapshot suffix removed (see
/// [`normalize_model_id`]). Only Anthropic models are listed: they are the
/// only ones whose token counts this crate's adapters report completely
/// enough to price. A model that is not in this table is not priced.
const PRICES: &[(&str, ModelPrice)] = &[
    // Claude Fable 5.1 / Mythos 5.1: $10 / $12.50 / $20 / $0.25 / $50.
    // The cache-hit rate is 0.025x base input on these two models, against
    // the 0.1x every other model uses -- which is why each rate is stored
    // rather than derived from a multiplier.
    (
        "claude-fable-5-1",
        ModelPrice {
            input: 10_000_000,
            output: 50_000_000,
            cache_read: 250_000,
            cache_write_5m: 12_500_000,
            cache_write_1h: 20_000_000,
        },
    ),
    (
        "claude-mythos-5-1",
        ModelPrice {
            input: 10_000_000,
            output: 50_000_000,
            cache_read: 250_000,
            cache_write_5m: 12_500_000,
            cache_write_1h: 20_000_000,
        },
    ),
    // Claude Fable 5 / Mythos 5: $10 / $12.50 / $20 / $1 / $50.
    (
        "claude-fable-5",
        ModelPrice {
            input: 10_000_000,
            output: 50_000_000,
            cache_read: 1_000_000,
            cache_write_5m: 12_500_000,
            cache_write_1h: 20_000_000,
        },
    ),
    (
        "claude-mythos-5",
        ModelPrice {
            input: 10_000_000,
            output: 50_000_000,
            cache_read: 1_000_000,
            cache_write_5m: 12_500_000,
            cache_write_1h: 20_000_000,
        },
    ),
    // Claude Opus 5 / 4.8 / 4.7 / 4.6 / 4.5: $5 / $6.25 / $10 / $0.50 / $25.
    (
        "claude-opus-5",
        ModelPrice {
            input: 5_000_000,
            output: 25_000_000,
            cache_read: 500_000,
            cache_write_5m: 6_250_000,
            cache_write_1h: 10_000_000,
        },
    ),
    (
        "claude-opus-4-8",
        ModelPrice {
            input: 5_000_000,
            output: 25_000_000,
            cache_read: 500_000,
            cache_write_5m: 6_250_000,
            cache_write_1h: 10_000_000,
        },
    ),
    (
        "claude-opus-4-7",
        ModelPrice {
            input: 5_000_000,
            output: 25_000_000,
            cache_read: 500_000,
            cache_write_5m: 6_250_000,
            cache_write_1h: 10_000_000,
        },
    ),
    (
        "claude-opus-4-6",
        ModelPrice {
            input: 5_000_000,
            output: 25_000_000,
            cache_read: 500_000,
            cache_write_5m: 6_250_000,
            cache_write_1h: 10_000_000,
        },
    ),
    (
        "claude-opus-4-5",
        ModelPrice {
            input: 5_000_000,
            output: 25_000_000,
            cache_read: 500_000,
            cache_write_5m: 6_250_000,
            cache_write_1h: 10_000_000,
        },
    ),
    // Claude Opus 4.1 / 4 (retired on the first-party API):
    // $15 / $18.75 / $30 / $1.50 / $75.
    (
        "claude-opus-4-1",
        ModelPrice {
            input: 15_000_000,
            output: 75_000_000,
            cache_read: 1_500_000,
            cache_write_5m: 18_750_000,
            cache_write_1h: 30_000_000,
        },
    ),
    (
        "claude-opus-4",
        ModelPrice {
            input: 15_000_000,
            output: 75_000_000,
            cache_read: 1_500_000,
            cache_write_5m: 18_750_000,
            cache_write_1h: 30_000_000,
        },
    ),
    // Claude Sonnet 5: $2 / $2.50 / $4 / $0.20 / $10. The $2/$10 rate was
    // announced as introductory pricing through 2026-08-31; the source page
    // states the scheduled increase to $3/$15 will not occur and that these
    // are now the standard prices.
    (
        "claude-sonnet-5",
        ModelPrice {
            input: 2_000_000,
            output: 10_000_000,
            cache_read: 200_000,
            cache_write_5m: 2_500_000,
            cache_write_1h: 4_000_000,
        },
    ),
    // Claude Sonnet 4.6 / 4.5 / 4: $3 / $3.75 / $6 / $0.30 / $15.
    (
        "claude-sonnet-4-6",
        ModelPrice {
            input: 3_000_000,
            output: 15_000_000,
            cache_read: 300_000,
            cache_write_5m: 3_750_000,
            cache_write_1h: 6_000_000,
        },
    ),
    (
        "claude-sonnet-4-5",
        ModelPrice {
            input: 3_000_000,
            output: 15_000_000,
            cache_read: 300_000,
            cache_write_5m: 3_750_000,
            cache_write_1h: 6_000_000,
        },
    ),
    (
        "claude-sonnet-4",
        ModelPrice {
            input: 3_000_000,
            output: 15_000_000,
            cache_read: 300_000,
            cache_write_5m: 3_750_000,
            cache_write_1h: 6_000_000,
        },
    ),
    // Claude Haiku 4.5: $1 / $1.25 / $2 / $0.10 / $5.
    (
        "claude-haiku-4-5",
        ModelPrice {
            input: 1_000_000,
            output: 5_000_000,
            cache_read: 100_000,
            cache_write_5m: 1_250_000,
            cache_write_1h: 2_000_000,
        },
    ),
    // Claude Haiku 3.5 (retired on the first-party API):
    // $0.80 / $1 / $1.60 / $0.08 / $4.
    (
        "claude-3-5-haiku",
        ModelPrice {
            input: 800_000,
            output: 4_000_000,
            cache_read: 80_000,
            cache_write_5m: 1_000_000,
            cache_write_1h: 1_600_000,
        },
    ),
];

/// Micro-USD per million tokens divided by tokens-per-million: a price in
/// micro-USD per million tokens, multiplied by a token count, is 10^12 times
/// the price in USD.
const MICRO_USD_PER_MTOK_SCALE: u32 = 12;

/// Strips a dated-snapshot suffix, so `claude-haiku-4-5-20251001` prices as
/// `claude-haiku-4-5`.
///
/// A dated id and its dateless form are the same model at the same price --
/// Anthropic publishes one price row for both -- so this is not a fallback to
/// a *similar* model, which the module's rules forbid. Only a trailing `-`
/// plus exactly eight digits is removed; anything else is left alone and will
/// simply miss the table.
fn normalize_model_id(model: &str) -> &str {
    match model.rsplit_once('-') {
        Some((head, tail)) if tail.len() == 8 && tail.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => model,
    }
}

/// What `usage` would have cost on `model` at the provider's list price, or
/// `None` if this crate cannot say.
///
/// `None` means exactly one thing: no honest answer is available -- the model
/// is not in [`PRICES`]. It never means "free", and a caller must not render
/// it as `$0.00`.
pub fn list_price_usd(model: &str, usage: &TokenUsage) -> Option<Decimal> {
    let key = normalize_model_id(model);
    let price = PRICES
        .iter()
        .find(|(id, _)| *id == key)
        .map(|(_, price)| *price)?;

    // u128 throughout: the largest term a u32 count can produce is about
    // 4.3e9 * 7.5e7 = 3.2e17, so even summing all five cannot approach u128's
    // range, and no term is ever rounded before the single division below.
    let total_scaled: u128 = u128::from(usage.input_tokens) * u128::from(price.input)
        + u128::from(usage.output_tokens) * u128::from(price.output)
        + u128::from(usage.cache_read_tokens) * u128::from(price.cache_read)
        + u128::from(usage.cache_write_5m_tokens) * u128::from(price.cache_write_5m)
        + u128::from(usage.cache_write_1h_tokens) * u128::from(price.cache_write_1h);

    // `Decimal` holds 28 significant digits and `total_scaled` has at most
    // 19, so this conversion cannot lose one.
    let usd =
        Decimal::from_i128_with_scale(i128::try_from(total_scaled).ok()?, MICRO_USD_PER_MTOK_SCALE);
    Some(usd.normalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn usd(text: &str) -> Decimal {
        Decimal::from_str(text).unwrap()
    }

    fn tokens(input: u32, output: u32) -> TokenUsage {
        TokenUsage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 0,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
        }
    }

    /// A round million on Claude Opus 5 is the table's own headline figure:
    /// $5 in, $25 out. Every expected value in this module's tests is worked
    /// out by hand from the published rates, never by calling
    /// `list_price_usd` -- a test that computed its expectation with the
    /// function under test would pass against any table at all.
    #[test]
    fn one_million_tokens_costs_the_published_rate() {
        assert_eq!(
            list_price_usd("claude-opus-5", &tokens(1_000_000, 0)),
            Some(usd("5"))
        );
        assert_eq!(
            list_price_usd("claude-opus-5", &tokens(0, 1_000_000)),
            Some(usd("25"))
        );
    }

    /// Counts that are not round numbers, so a table entry off by a factor of
    /// ten or a scale slip in the division would show up.
    /// 12_345 * $2/MTok = $0.024690; 6_789 * $10/MTok = $0.067890.
    #[test]
    fn awkward_counts_price_to_the_exact_cent_fraction() {
        assert_eq!(
            list_price_usd("claude-sonnet-5", &tokens(12_345, 6_789)),
            Some(usd("0.09258"))
        );
    }

    /// Ground truth from outside this crate: the worked example on the source
    /// pricing page prices 10,000 uncached input + 40,000 cache-read +
    /// 15,000 output tokens on Claude Opus 5 at $0.05 + $0.02 + $0.375.
    /// (The page's total also includes $0.08 of Managed Agents session
    /// runtime, which is not a token price and not modelled here.)
    #[test]
    fn matches_the_published_worked_example() {
        let usage = TokenUsage {
            input_tokens: 10_000,
            output_tokens: 15_000,
            cache_read_tokens: 40_000,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
        };
        assert_eq!(list_price_usd("claude-opus-5", &usage), Some(usd("0.445")));
    }

    /// The two cache durations are priced differently -- 1.25x base input for
    /// a 5-minute write, 2x for an hour -- so a caller that knew only the
    /// combined count would understate by up to 1.6x. This is the assertion
    /// that fails if the two ever collapse into one rate.
    #[test]
    fn a_one_hour_cache_write_is_not_priced_as_a_five_minute_one() {
        let five_minute = TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_5m_tokens: 1_000_000,
            cache_write_1h_tokens: 0,
        };
        let one_hour = TokenUsage {
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 1_000_000,
            ..five_minute
        };
        // $6.25/MTok against $10/MTok on Claude Opus 5.
        assert_eq!(
            list_price_usd("claude-opus-5", &five_minute),
            Some(usd("6.25"))
        );
        assert_eq!(list_price_usd("claude-opus-5", &one_hour), Some(usd("10")));
    }

    /// Fable 5.1 reads cache at 0.025x base input where every other model
    /// reads at 0.1x. Deriving cache rates from a single multiplier would
    /// misprice one of these two.
    #[test]
    fn cache_read_rates_are_per_model_not_a_shared_multiplier() {
        let usage = TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 1_000_000,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
        };
        assert_eq!(
            list_price_usd("claude-fable-5-1", &usage),
            Some(usd("0.25"))
        );
        assert_eq!(list_price_usd("claude-fable-5", &usage), Some(usd("1")));
    }

    /// The rule that protects every other number on the screen: a model the
    /// table does not list is not priced at a neighbour's rate, an average,
    /// or a default.
    #[test]
    fn an_unlisted_model_is_not_priced() {
        for model in [
            "gpt-5",
            "gemini-3-pro",
            "claude-opus-9",
            "claude-opus",
            "",
            "claude-sonnet-5-turbo",
        ] {
            assert_eq!(
                list_price_usd(model, &tokens(1_000_000, 1_000_000)),
                None,
                "{model} must not be priced"
            );
        }
    }

    /// A model whose price would be a near miss is the case a "similar
    /// model" fallback would get wrong, so it gets its own assertion: Sonnet
    /// 4.6 and Sonnet 5 differ by 1.5x and neither may stand in for an
    /// unlisted third.
    #[test]
    fn near_neighbours_do_not_stand_in_for_each_other() {
        assert_eq!(
            list_price_usd("claude-sonnet-4-6", &tokens(1_000_000, 0)),
            Some(usd("3"))
        );
        assert_eq!(
            list_price_usd("claude-sonnet-5", &tokens(1_000_000, 0)),
            Some(usd("2"))
        );
        assert_eq!(
            list_price_usd("claude-sonnet-4-7", &tokens(1_000_000, 0)),
            None
        );
    }

    #[test]
    fn a_dated_snapshot_prices_as_its_dateless_id() {
        assert_eq!(
            list_price_usd("claude-haiku-4-5-20251001", &tokens(1_000_000, 0)),
            Some(usd("1"))
        );
        assert_eq!(
            list_price_usd("claude-3-5-haiku-20241022", &tokens(1_000_000, 0)),
            Some(usd("0.8"))
        );
    }

    /// Only an eight-digit date is a snapshot suffix. Trimming anything else
    /// would let an unrelated id fall through onto a price.
    #[test]
    fn only_an_eight_digit_suffix_is_stripped() {
        assert_eq!(normalize_model_id("claude-opus-5"), "claude-opus-5");
        assert_eq!(
            normalize_model_id("claude-haiku-4-5-20251001"),
            "claude-haiku-4-5"
        );
        // Seven digits, nine digits, and a non-numeric tail all stay put.
        assert_eq!(
            normalize_model_id("claude-haiku-4-5-2025100"),
            "claude-haiku-4-5-2025100"
        );
        assert_eq!(
            normalize_model_id("claude-haiku-4-5-202510011"),
            "claude-haiku-4-5-202510011"
        );
        assert_eq!(
            normalize_model_id("claude-haiku-4-5-preview"),
            "claude-haiku-4-5-preview"
        );
        assert_eq!(
            list_price_usd("claude-haiku-4-5-2025100", &tokens(1, 1)),
            None
        );
    }

    /// Zero of everything is a real answer -- the provider reported counts
    /// and they were all zero -- and it is the one case where $0 is honest.
    /// Absent counts never reach this function; the adapters return `None`
    /// before building a [`TokenUsage`].
    #[test]
    fn all_zero_counts_price_at_zero() {
        assert_eq!(
            list_price_usd("claude-opus-5", &tokens(0, 0)),
            Some(Decimal::ZERO)
        );
    }

    /// The largest counts a `u32` can hold, on the most expensive model,
    /// must still produce a number rather than overflowing or saturating.
    #[test]
    fn the_largest_representable_usage_does_not_overflow() {
        let usage = TokenUsage {
            input_tokens: u32::MAX,
            output_tokens: u32::MAX,
            cache_read_tokens: u32::MAX,
            cache_write_5m_tokens: u32::MAX,
            cache_write_1h_tokens: u32::MAX,
        };
        // 4_294_967_295 * (15 + 75 + 1.5 + 18.75 + 30) / 1e6 dollars.
        assert_eq!(
            list_price_usd("claude-opus-4-1", &usage),
            Some(usd("602369.16312375"))
        );
    }

    /// Every row must be reachable through the lookup, and no two rows may
    /// claim the same id -- a duplicate key would silently shadow whichever
    /// price came second.
    #[test]
    fn every_table_row_is_reachable_and_unique() {
        let mut seen: Vec<&str> = Vec::new();
        for (id, _) in PRICES {
            assert!(!seen.contains(id), "{id} appears twice in the price table");
            seen.push(id);
            assert_eq!(normalize_model_id(id), *id, "{id} is not in normal form");
            assert!(
                list_price_usd(id, &tokens(1_000_000, 0)).is_some(),
                "{id} is in the table but does not price"
            );
        }
        assert_eq!(seen.len(), PRICES.len());
    }
}
