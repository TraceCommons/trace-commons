//! The per-session route tally: where a feed T session's proxy calls went,
//! as counts, folded from the proxy ledger this daemon already reads and
//! from each managed route's ledger, each by its own cursor
//! (`insights_week::FoldPlans`).
//!
//! - Product owner, 2026-10-28: Insights may show money. The charge below
//!   is kept by the hour its calls started in, and `insights_week`'s
//!   `near_ai_credit` sums it by window, task, tool and folder
//!   (`insights_credit`).
//!
//! - Owner question Q3 (default taken): the tally is persisted in the counter
//!   store, counts only, so the split outlives the ledger's 24-hour window.
//!   It is kept for the store's weeks, capped like its rows, and removed with
//!   the store on unenroll.
//! - Owner decision D15, extended to feed T (default taken): a tally is keyed
//!   by the keyed harness-session digest under the counter key, the same
//!   digest a counter row stores, so the two join inside the daemon and
//!   nowhere else.
//! - Owner decision D3, settled 2026-10-09: `insights_ledger_feed` is on by
//!   default, so the fold runs by default; nothing is folded while a
//!   contributor has turned it off.
//! - Owner decision D5, settled 2026-10-20: a call's `cost_usd` is read
//!   only for calls through the NEAR AI route (proof labels `verified`,
//!   `gateway_only`, `unattested`, `pending`, `unavailable` and `failed`),
//!   and kept as that session's one NEAR AI charge, in integer USD micros,
//!   with the one model its calls named as served, if they named one; held
//!   (`LEDGER_MONEY_IN_INSIGHTS` false) none is taken. The ledger keeps a
//!   call for 24 hours, so the charge is taken when the call is folded or
//!   not at all. `outside` calls and calls with no proof label add no
//!   charge: an `outside` call is not NEAR AI's, and the proxy prices it
//!   whether or not anyone was billed; an unlabelled call's route is
//!   unknown. No list
//!   price is kept here; the answers price stored tokens when they are
//!   built. A session in a Never folder keeps no tally at all
//!   (`insights_week::NeverSessions`).
//!
//! Every category label here comes from IronWire's proof label and never
//! from a backend or model name; the one model name a tally keeps is its
//! charge's served model, and no backend is kept. Only `verified` is proof;
//! `gateway_only` names the relay, not the model. A session with no tally is `unobserved`, never
//! "outside": no proxy record is not evidence of where a call went.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use trace_commons_protocol::insights_usage_series::{
    DigestKey, KeyedDigest, harness_session_digest,
};

use super::inference_map::{Speakers, attribute};
use crate::insights::ledger_series::call_tokens;
use crate::routing::{ProofStatus, RoutedExchange};
use crate::source::{SOURCE_CLAUDE_CODE, SOURCE_CODEX};

/// One figure per proof bucket.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouteBuckets {
    /// `verified`, the only proof.
    pub verified: u64,
    /// `gateway_only`, `unattested`, `pending` or `unavailable`: through the
    /// proxy's NEAR AI route, not proven.
    pub routed_unverified: u64,
    /// `failed`: a receipt that did not check out.
    pub check_failed: u64,
    /// `outside`: not a NEAR AI backend.
    pub outside: u64,
    /// No proof label recorded (an older proxy, or one this build does not
    /// know). Not `outside`.
    pub unrecorded: u64,
}

impl RouteBuckets {
    fn slot(&mut self, proof: Option<ProofStatus>) -> &mut u64 {
        match proof {
            Some(ProofStatus::Verified) => &mut self.verified,
            Some(
                ProofStatus::GatewayOnly
                | ProofStatus::Unattested
                | ProofStatus::Pending
                | ProofStatus::Unavailable,
            ) => &mut self.routed_unverified,
            Some(ProofStatus::Failed) => &mut self.check_failed,
            Some(ProofStatus::Outside) => &mut self.outside,
            None => &mut self.unrecorded,
        }
    }

    fn routed(&self) -> u64 {
        self.verified
            .saturating_add(self.routed_unverified)
            .saturating_add(self.check_failed)
    }

    fn labelled(&self) -> u64 {
        self.routed().saturating_add(self.outside)
    }

    /// Every call or token, labelled or not.
    pub(crate) fn total(&self) -> u64 {
        self.labelled().saturating_add(self.unrecorded)
    }
}

/// One session's NEAR AI charge, summed from the ledger's `cost_usd` as
/// each call through the NEAR AI route is folded: one per tally, whatever
/// models served the calls.
///
/// Known only when every call counted here carried a valid charge and the
/// sum never overflowed: a call with no charge makes the whole session's
/// charge unknown, never zero.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NearAiCharge {
    /// The sum of the charged calls, in integer USD micros.
    pub usd_micros: u64,
    /// Calls whose charge is in `usd_micros`.
    pub charged_calls: u64,
    /// Calls with no charge, or one that is not a finite, non-negative
    /// amount below `u64::MAX` micros.
    pub calls_without_charge: u64,
    /// The sum overflowed `u64`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub overflowed: bool,
    /// The one model every call counted here named as served, normalized
    /// as the answers normalize labels (`served_label`): the only thing the
    /// Models answers attribute a charge to a model by. `None` once any
    /// call named none, or two calls named different ones; it stays `None`
    /// after that. Never the requested model: that is not who served.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub served_model: Option<String>,
    /// The same calls by the clock hour they started in (unix seconds over
    /// 3600, UTC), so a window (today, the last 31 days, a week) takes only
    /// the calls made inside it (product owner, 2026-10-28: Insights shows
    /// where NEAR AI credit went). Empty in a tally written before hours
    /// were kept, and left out of the file then; such a charge is read as
    /// one hour, the tally's last call's ([`NearAiCharge::hours`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_hour: BTreeMap<i64, HourCharge>,
    /// The same calls by the model each one named as served, then by hour
    /// (product owner, 2026-10-29: NEAR AI credit by model), so a session
    /// that used two models splits between them. Empty in a tally written
    /// before models were kept, and left out of the file then; such a
    /// charge is read as one group, `served_model`'s
    /// ([`NearAiCharge::within_by_model`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_model: Vec<ModelCharge>,
}

/// One served model's share of a session's NEAR AI charge, by the clock
/// hour its calls started in. `model` is normalized as `served_model` is;
/// `None` holds the calls that named no model.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelCharge {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub by_hour: BTreeMap<i64, HourCharge>,
}

/// One clock hour's share of a session's NEAR AI charge. Known only when
/// every call in it carried a valid charge and the sum never overflowed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HourCharge {
    /// Calls through the NEAR AI route started in this hour.
    pub calls: u64,
    /// The sum of the charged calls, in integer USD micros.
    pub usd_micros: u64,
    /// Calls with no valid charge.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub calls_without_charge: u64,
    /// The sum overflowed `u64`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub overflowed: bool,
    /// The calls' tokens (uncached input, cache reads, cache writes and
    /// output, normalized per facade as feed L reads them), so the NEAR AI
    /// card compares with Claude's and Codex's (product owner,
    /// 2026-10-28). `None` (unknown, never 0) once any call's counters are
    /// unknown, and for an hour kept before tokens were.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    /// The tokens of the calls whose counters are known, and how many such
    /// calls there were (product owner, 2026-10-28: a figure with some
    /// unknown calls shows its known part as a minimum). Absent from an
    /// hour kept before them, which reads its `tokens`, if any, as all of
    /// its calls'.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tokens_known: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub calls_with_tokens: u64,
}

/// A figure as far as it is known: the sum of the calls that carried one,
/// and whether some calls did not (the sum is then a minimum). `None` when
/// no call carried one, or the sum overflowed: unknown, never zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KnownPart {
    pub sum: u64,
    pub partial: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(value: &u64) -> bool {
    *value == 0
}

impl HourCharge {
    /// Add `other` into this one; an overflow makes the sum unknown.
    pub(crate) fn absorb(&mut self, other: &HourCharge) {
        let (mine, theirs) = (self.token_parts(), other.token_parts());
        self.tokens_known = mine.0.saturating_add(theirs.0);
        self.calls_with_tokens = mine.1.saturating_add(theirs.1);
        self.tokens = if self.calls == 0 {
            other.tokens
        } else {
            self.tokens
                .zip(other.tokens)
                .and_then(|(a, b)| a.checked_add(b))
        };
        self.calls = self.calls.saturating_add(other.calls);
        self.calls_without_charge = self
            .calls_without_charge
            .saturating_add(other.calls_without_charge);
        self.overflowed |= other.overflowed;
        match self.usd_micros.checked_add(other.usd_micros) {
            Some(sum) => self.usd_micros = sum,
            None => self.overflowed = true,
        }
    }

    /// The charge, or `None` when it is unknown: never zero for a call
    /// that carried no charge.
    #[must_use]
    pub(crate) fn known(&self) -> Option<u64> {
        (self.calls_without_charge == 0 && !self.overflowed).then_some(self.usd_micros)
    }

    /// The charge as far as it is known: the charged calls' sum, partial
    /// when some calls carried none. `None` when none did.
    #[must_use]
    pub(crate) fn known_part(&self) -> Option<KnownPart> {
        let charged = self.calls.saturating_sub(self.calls_without_charge);
        if self.overflowed || (self.calls > 0 && charged == 0) {
            return None;
        }
        Some(KnownPart {
            sum: self.usd_micros,
            partial: self.calls_without_charge > 0,
        })
    }

    /// The tokens as far as they are known, as [`Self::known_part`].
    #[must_use]
    pub(crate) fn tokens_part(&self) -> Option<KnownPart> {
        let (sum, with) = self.token_parts();
        if self.calls > 0 && with == 0 {
            return None;
        }
        Some(KnownPart {
            sum,
            partial: with < self.calls,
        })
    }

    /// The known tokens and the calls they came from; an hour kept before
    /// these were counted reads a complete `tokens` as all of its calls'.
    fn token_parts(&self) -> (u64, u64) {
        match (self.calls_with_tokens, self.tokens) {
            (0, Some(tokens)) => (tokens, self.calls),
            _ => (self.tokens_known, self.calls_with_tokens),
        }
    }
}

/// The unix hour `at` falls in.
#[must_use]
pub(crate) fn unix_hour(at: DateTime<Utc>) -> i64 {
    at.timestamp().div_euclid(3600)
}

impl NearAiCharge {
    /// The charge by hour; a charge kept before hours were is one hour,
    /// `last_call_at`'s, holding the whole of it.
    #[must_use]
    pub(crate) fn hours(&self, last_call_at: DateTime<Utc>) -> BTreeMap<i64, HourCharge> {
        if !self.by_hour.is_empty() {
            return self.by_hour.clone();
        }
        let calls = self.charged_calls.saturating_add(self.calls_without_charge);
        if calls == 0 {
            return BTreeMap::new();
        }
        BTreeMap::from([(
            unix_hour(last_call_at),
            HourCharge {
                calls,
                usd_micros: self.usd_micros,
                calls_without_charge: self.calls_without_charge,
                overflowed: self.overflowed,
                tokens: None,
                tokens_known: 0,
                calls_with_tokens: 0,
            },
        )])
    }

    /// The calls started in `[from, to)`, summed; `None` when none was.
    /// An hour is in the window when it starts inside it.
    #[must_use]
    pub(crate) fn within(
        &self,
        last_call_at: DateTime<Utc>,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Option<HourCharge> {
        sum_within(&self.hours(last_call_at), from, to)
    }

    /// The charge by served model; a charge kept before models were is one
    /// group, `served_model`'s, holding the whole of it.
    fn models(
        &self,
        last_call_at: DateTime<Utc>,
    ) -> Vec<(Option<&str>, BTreeMap<i64, HourCharge>)> {
        if !self.by_model.is_empty() {
            return self
                .by_model
                .iter()
                .map(|group| (group.model.as_deref(), group.by_hour.clone()))
                .collect();
        }
        let hours = self.hours(last_call_at);
        if hours.is_empty() {
            return Vec::new();
        }
        vec![(self.served_model.as_deref(), hours)]
    }

    /// [`Self::within`] by the model that served the calls: one entry per
    /// model with a call started in `[from, to)`, `None` for the calls that
    /// named none. The entries add up to [`Self::within`].
    #[must_use]
    pub(crate) fn within_by_model(
        &self,
        last_call_at: DateTime<Utc>,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Vec<(Option<&str>, HourCharge)> {
        self.models(last_call_at)
            .into_iter()
            .filter_map(|(model, hours)| Some((model, sum_within(&hours, from, to)?)))
            .collect()
    }

    /// The charge, or `None` when it is unknown.
    #[must_use]
    pub(crate) fn known(&self) -> Option<u64> {
        (self.calls_without_charge == 0 && !self.overflowed).then_some(self.usd_micros)
    }

    fn add(
        &mut self,
        (usd, tokens): (Option<f64>, Option<u64>),
        (served, model): (Option<String>, Option<String>),
        at: DateTime<Utc>,
        last_call_at: DateTime<Utc>,
    ) {
        let earlier = self.charged_calls.saturating_add(self.calls_without_charge) > 0;
        // A tally kept before models were: its charge so far becomes one
        // group, the model it kept, so the groups keep summing to the whole.
        if self.by_model.is_empty() && earlier {
            self.by_model = vec![ModelCharge {
                model: self.served_model.clone(),
                by_hour: self.hours(last_call_at),
            }];
        }
        // A tally kept before hours were: its charge so far becomes one
        // hour, so the hours keep summing to the whole.
        if self.by_hour.is_empty() && earlier {
            self.by_hour = self.hours(last_call_at);
        }
        let micros = usd.and_then(usd_micros);
        let call = HourCharge {
            calls: 1,
            usd_micros: micros.unwrap_or(0),
            calls_without_charge: u64::from(micros.is_none()),
            overflowed: false,
            tokens,
            tokens_known: tokens.unwrap_or(0),
            calls_with_tokens: u64::from(tokens.is_some()),
        };
        self.by_hour.entry(unix_hour(at)).or_default().absorb(&call);
        let group = match self.by_model.iter().position(|group| group.model == model) {
            Some(index) => &mut self.by_model[index],
            None => {
                self.by_model.push(ModelCharge {
                    model,
                    by_hour: BTreeMap::new(),
                });
                self.by_model.last_mut().expect("just pushed")
            }
        };
        group
            .by_hour
            .entry(unix_hour(at))
            .or_default()
            .absorb(&call);
        if !earlier {
            self.served_model = served;
        } else if self.served_model != served {
            self.served_model = None;
        }
        match micros {
            Some(micros) => {
                self.charged_calls = self.charged_calls.saturating_add(1);
                match self.usd_micros.checked_add(micros) {
                    Some(sum) => self.usd_micros = sum,
                    None => self.overflowed = true,
                }
            }
            None => self.calls_without_charge = self.calls_without_charge.saturating_add(1),
        }
    }
}

/// The hours of `hours` that start in `[from, to)`, summed; `None` when
/// none had a call.
fn sum_within(
    hours: &BTreeMap<i64, HourCharge>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Option<HourCharge> {
    let (first, end) = (hour_at_or_after(from), hour_at_or_after(to));
    let mut sum: Option<HourCharge> = None;
    for (_, hour) in hours.range(first..end) {
        sum.get_or_insert_with(HourCharge::default).absorb(hour);
    }
    sum
}

/// The first whole hour starting at or after `at`.
fn hour_at_or_after(at: DateTime<Utc>) -> i64 {
    let seconds = at.timestamp();
    let hour = seconds.div_euclid(3600);
    if seconds.rem_euclid(3600) == 0 && at.timestamp_subsec_nanos() == 0 {
        hour
    } else {
        hour + 1
    }
}

/// A ledger amount in integer USD micros, rounded to the nearest; `None`
/// when it is not finite, is negative, or does not fit.
fn usd_micros(usd: f64) -> Option<u64> {
    if !usd.is_finite() || usd < 0.0 {
        return None;
    }
    let micros = (usd * 1_000_000.0).round();
    // `u64::MAX as f64` rounds up to 2^64, so `<` keeps the cast exact.
    (micros < u64::MAX as f64).then_some(micros as u64)
}

/// The model a call names as served, normalized as the answers normalize
/// labels; `None` when it names none.
fn served_label(row: &RoutedExchange) -> Option<String> {
    row.served_model
        .as_deref()
        .and_then(crate::insights::session_pricing::normalize_model_label)
}

/// The model a call names as served, for NEAR AI credit by model:
/// lowercased as [`served_label`] lowercases, but whole, so NEAR AI's
/// `org/model` ids (which `served_label` reads as none) keep their name.
/// `None` when it names none, or names something that is not a plain id.
/// Never the requested model.
fn served_model_key(row: &RoutedExchange) -> Option<String> {
    let label = row.served_model.as_deref()?.trim().to_ascii_lowercase();
    let plain = !label.is_empty()
        && label.len() <= MAX_MODEL_KEY_BYTES
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/".contains(&byte));
    plain.then_some(label)
}

/// The longest served model name kept as a group's key.
const MAX_MODEL_KEY_BYTES: usize = 128;

/// A session's NEAR AI charge for the Models answers: `None` when it had
/// no call through the NEAR AI route. Its `usd_micros` is `None` when the
/// charge is unknown (a call without one, or an overflow).
#[must_use]
pub(crate) fn session_charge(
    tally: &RouteTally,
) -> Option<crate::insights::model_answers::SessionCharge> {
    tally
        .charge
        .as_ref()
        .map(|charge| crate::insights::model_answers::SessionCharge {
            usd_micros: charge.known(),
            served_model: charge.served_model.clone(),
        })
}

/// One session's tally. Counts, one time and the NEAR AI charge only: never
/// an id, a digest of one, a backend, a list price or a body reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouteTally {
    /// Calls per bucket. The category is drawn from these, never from
    /// tokens, so a call whose counters are unknown still counts where it
    /// went.
    pub calls: RouteBuckets,
    /// Tokens per bucket, from calls whose counters are all known:
    /// uncached input, cache reads, cache writes and output, normalized per
    /// facade as feed L does.
    pub tokens: RouteBuckets,
    /// Calls counted in `calls` whose tokens are unknown, per bucket, and so
    /// in no token figure. Per bucket so the answer can mark exactly the
    /// token figures they leave unknown, and never read one as zero.
    pub calls_without_counts: RouteBuckets,
    /// The latest call folded, for ageing the tally out.
    pub last_call_at: DateTime<Utc>,
    /// The NEAR AI charge of the calls through the NEAR AI route, one per
    /// session. `None` when no such call was folded, and then left out of
    /// the file, so a store written before charges loads with none and a
    /// tally with none is written as it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charge: Option<NearAiCharge>,
}

/// Where a session's calls went, checked in this order, first match wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteCategory {
    /// No tally. Never "outside".
    Unobserved,
    /// Calls, none with a proof label.
    Unrecorded,
    /// Every labelled call `outside`.
    Outside,
    /// Labelled calls both outside and through the route.
    Mixed,
    /// All labelled calls through the route, at least one `failed`.
    CheckFailed,
    /// All labelled calls `verified`.
    RoutedVerified,
    /// All labelled calls through the route, at least one not proven.
    RoutedUnverified,
}

impl RouteCategory {
    /// The wire spelling. None of them says "private".
    #[must_use]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unobserved => "unobserved",
            Self::Unrecorded => "unrecorded",
            Self::Outside => "outside",
            Self::Mixed => "mixed",
            Self::CheckFailed => "check_failed",
            Self::RoutedVerified => "routed_verified",
            Self::RoutedUnverified => "routed_unverified",
        }
    }
}

/// Why a category needs a qualifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteReason {
    /// Some calls carry a proof label and some do not; the category is the
    /// labelled calls'.
    SomeCallsUnrecorded,
}

impl RouteReason {
    #[must_use]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SomeCallsUnrecorded => "some_calls_unrecorded",
        }
    }
}

/// A session's category, and its reason when some calls are unrecorded.
#[must_use]
pub(crate) fn route_category(tally: Option<&RouteTally>) -> (RouteCategory, Option<RouteReason>) {
    let Some(calls) = tally.map(|tally| tally.calls) else {
        return (RouteCategory::Unobserved, None);
    };
    if calls.total() == 0 {
        return (RouteCategory::Unobserved, None);
    }
    if calls.labelled() == 0 {
        return (RouteCategory::Unrecorded, None);
    }
    let reason = (calls.unrecorded > 0).then_some(RouteReason::SomeCallsUnrecorded);
    let category = if calls.routed() == 0 {
        RouteCategory::Outside
    } else if calls.outside > 0 {
        RouteCategory::Mixed
    } else if calls.check_failed > 0 {
        RouteCategory::CheckFailed
    } else if calls.routed_unverified == 0 {
        RouteCategory::RoutedVerified
    } else {
        RouteCategory::RoutedUnverified
    };
    (category, reason)
}

/// One session's `routing` as it crosses the socket, in `insights_week`'s
/// `rollup.sessions[]`: the category, its reasons, tokens per bucket, calls
/// and calls without counts. Counts and fixed labels only: never an id, a
/// digest, a backend, a model, a price or a time.
///
/// - No tally (or one with no calls) is `unobserved` with every figure
///   `null`: no proxy record is not zero calls.
/// - A bucket's token figure is `null` while any call in it has unknown
///   counters, so a figure is never the zero or the part its counted calls
///   happen to sum to. A bucket with no calls is a true 0.
/// - `calls` and `calls_without_counts` are totals over every bucket.
#[must_use]
pub(crate) fn routing_value(tally: Option<&RouteTally>) -> serde_json::Value {
    let (category, reason) = route_category(tally);
    let reasons: Vec<&str> = reason.map(RouteReason::as_str).into_iter().collect();
    let Some(tally) = tally.filter(|_| category != RouteCategory::Unobserved) else {
        return serde_json::json!({
            "category": category.as_str(),
            "reasons": reasons,
            "tokens": null,
            "calls": null,
            "calls_without_counts": null,
        });
    };
    let figure = |tokens: u64, uncounted: u64| (uncounted == 0).then_some(tokens);
    let (tokens, uncounted) = (&tally.tokens, &tally.calls_without_counts);
    serde_json::json!({
        "category": category.as_str(),
        "reasons": reasons,
        "tokens": {
            "verified": figure(tokens.verified, uncounted.verified),
            "routed_unverified": figure(tokens.routed_unverified, uncounted.routed_unverified),
            "check_failed": figure(tokens.check_failed, uncounted.check_failed),
            "outside": figure(tokens.outside, uncounted.outside),
            "unrecorded": figure(tokens.unrecorded, uncounted.unrecorded),
        },
        "calls": tally.calls.total(),
        "calls_without_counts": uncounted.total(),
    })
}

/// Where the folds have reached in the proxy's ledger: the highest row id
/// passed, and the latest `started_at` of any row taken. Both persist with
/// the tallies, and move together.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FoldCursor {
    pub through: Option<i64>,
    pub started: Option<DateTime<Utc>>,
}

/// The rows a fold takes, oldest id first, and where its cursor ends.
#[derive(Debug)]
pub(crate) struct FoldPlan<'a> {
    pub rows: Vec<&'a RoutedExchange>,
    pub next: FoldCursor,
}

impl FoldPlan<'_> {
    /// Whether attributing the rows needs the connected tools: only when one
    /// carries a session id at all.
    pub(crate) fn needs_speakers(&self) -> bool {
        self.rows.iter().any(|row| session_of(row).is_some())
    }
}

/// What one fold did. Counts only.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct FoldSummary {
    pub folded: usize,
    pub skipped: usize,
}

/// The rows of `window` not yet folded, and the cursor after them.
///
/// - A row without an id is never taken: nothing could say it was folded
///   once only.
/// - A row is taken when its id is above the cursor's, or when it started
///   after every row taken so far. IronWire assigns ids in the order rows
///   are recorded, at the end of a call, and `started_at` follows neither
///   (`ironwire_ledger`'s `page` says so), so the two together are needed:
///   - A long call can hold the highest id and still be the first to age
///     out of the window. The window's highest id then falls below the
///     cursor with no restart at all, and every row left in it was taken
///     already, each starting no later than the latest start taken. None is
///     taken again, so nothing counts twice.
///   - A ledger that started over holds only calls made since, each
///     starting after every row taken from the old one. They are taken
///     whether their ids are below the cursor or have climbed past it, so
///     no call is lost to the restart either. `take_added_rows` instead
///     re-baselines and hands out nothing, which suits a pulse; for a tally
///     a dropped `outside` call would read as a stronger claim than the
///     calls support.
/// - The cursor's id becomes the window's highest, so it falls with a
///   restarted ledger. An empty window leaves the cursor where it is.
/// - Residual edge: a proxy whose clock stepped back across a restart can
///   start its first calls before the latest start taken; those rows are
///   not taken. A store from before the start mark (an id and no start)
///   takes by id only, so it cannot count a row twice.
#[must_use]
pub(crate) fn fold_plan(window: &[RoutedExchange], cursor: FoldCursor) -> FoldPlan<'_> {
    let rows: Vec<&RoutedExchange> = window.iter().collect();
    fold_plan_refs(&rows, cursor)
}

/// [`fold_plan`] over rows held elsewhere, such as one managed route's
/// ledger in the daemon's managed-history snapshot. Each managed ledger
/// numbers its rows on its own, so each is planned with its own cursor.
#[must_use]
pub(crate) fn fold_plan_refs<'a>(
    window: &[&'a RoutedExchange],
    cursor: FoldCursor,
) -> FoldPlan<'a> {
    let highest = window.iter().filter_map(|row| row.id).max();
    let mut rows: Vec<&RoutedExchange> = window
        .iter()
        .copied()
        .filter(|row| {
            row.id.is_some_and(|id| {
                cursor.through.is_none_or(|through| id > through)
                    || cursor
                        .started
                        .is_some_and(|started| row.started_at > started)
            })
        })
        .collect();
    rows.sort_by_key(|row| row.id);
    let started = rows.iter().map(|row| row.started_at).max();
    FoldPlan {
        next: FoldCursor {
            through: highest.or(cursor.through),
            started: cursor.started.max(started),
        },
        rows,
    }
}

/// Fold `plan`'s rows into `routing`, keyed by the harness-session digest
/// under `key`.
///
/// A row is skipped when its session id is missing or empty (the guard
/// `routing::enriched` applies before any join), or when no single tool can
/// be named for it, or the tool is not one feed T counts. The tool is never
/// guessed.
///
/// `ledger_money` is `LEDGER_MONEY_IN_INSIGHTS` (owner decision D5): held,
/// no charge is taken, so no money and no served model reach the store.
///
/// A skipped call's NEAR AI charge is not lost: it goes to `loose`, the
/// spend no session can be named for (an opencode session through a
/// managed route, a call with no session ID), which `insights_week`'s
/// `near_ai_credit` counts in every sum and in each unknown group
/// (product owner, 2026-10-28: the money is spent whoever spent it).
pub(crate) fn fold_into(
    routing: &mut BTreeMap<KeyedDigest, RouteTally>,
    loose: &mut Option<NearAiCharge>,
    key: &DigestKey,
    plan: &FoldPlan<'_>,
    speakers: &Speakers,
    ledger_money: bool,
) -> FoldSummary {
    let mut summary = FoldSummary::default();
    for row in &plan.rows {
        let Some((session, tool)) = named(row, speakers) else {
            add_loose(loose, row, ledger_money);
            summary.skipped += 1;
            continue;
        };
        let digest = harness_session_digest(key, tool, session);
        let tally = routing.entry(digest).or_insert_with(|| RouteTally {
            calls: RouteBuckets::default(),
            tokens: RouteBuckets::default(),
            calls_without_counts: RouteBuckets::default(),
            last_call_at: row.started_at,
            charge: None,
        });
        let calls = tally.calls.slot(row.proof);
        *calls = calls.saturating_add(1);
        match call_tokens(row) {
            Some(tokens) => {
                let bucket = tally.tokens.slot(row.proof);
                *bucket = bucket.saturating_add(tokens);
            }
            None => {
                let bucket = tally.calls_without_counts.slot(row.proof);
                *bucket = bucket.saturating_add(1);
            }
        }
        if ledger_money && charged_by_near_ai(row) {
            let last_call_at = tally.last_call_at;
            tally.charge.get_or_insert_with(NearAiCharge::default).add(
                call_charge(row),
                (served_label(row), served_model_key(row)),
                row.started_at,
                last_call_at,
            );
        }
        tally.last_call_at = tally.last_call_at.max(row.started_at);
        summary.folded += 1;
    }
    summary
}

/// Drop tallies no stored row matches once their last call is older than
/// `cutoff`, and drop `gone` (sessions whose rows were removed for a Never
/// folder) at once; then, past `max`, the oldest by last call.
pub(crate) fn prune(
    routing: &mut BTreeMap<KeyedDigest, RouteTally>,
    matched: &BTreeSet<KeyedDigest>,
    gone: &BTreeSet<KeyedDigest>,
    cutoff: DateTime<Utc>,
    max: usize,
) {
    routing.retain(|digest, tally| {
        !gone.contains(digest) && (matched.contains(digest) || tally.last_call_at >= cutoff)
    });
    if routing.len() <= max {
        return;
    }
    let mut oldest: Vec<(DateTime<Utc>, KeyedDigest)> = routing
        .iter()
        .map(|(digest, tally)| (tally.last_call_at, *digest))
        .collect();
    oldest.sort();
    let excess = routing.len() - max;
    for (_, digest) in oldest.into_iter().take(excess) {
        routing.remove(&digest);
    }
}

/// Whether a call went through the NEAR AI route: the buckets
/// `RouteBuckets::routed` sums.
fn through_near_ai(proof: Option<ProofStatus>) -> bool {
    matches!(
        proof,
        Some(
            ProofStatus::Verified
                | ProofStatus::GatewayOnly
                | ProofStatus::Unattested
                | ProofStatus::Pending
                | ProofStatus::Unavailable
                | ProofStatus::Failed
        )
    )
}

/// The session id a row carries; an empty one is none.
/// The session and tool a call is folded under, or `None` when it is
/// skipped: no session ID, or no tool feed T counts can be named for it.
/// The tool is never guessed.
fn named<'r>(row: &'r RoutedExchange, speakers: &Speakers) -> Option<(&'r str, &'static str)> {
    let session = session_of(row)?;
    let tool = attribute(row, speakers);
    (tool == SOURCE_CLAUDE_CODE || tool == SOURCE_CODEX).then_some((session, tool))
}

/// Add a skipped call's NEAR AI charge to `loose`.
fn add_loose(loose: &mut Option<NearAiCharge>, row: &RoutedExchange, ledger_money: bool) {
    if !ledger_money || !charged_by_near_ai(row) {
        return;
    }
    let charge = loose.get_or_insert_with(NearAiCharge::default);
    // `hours` is all a loose charge is read by, so the last call's time
    // is only a fallback for an empty charge, never used once a call is in.
    charge.add(
        call_charge(row),
        (None, served_model_key(row)),
        row.started_at,
        row.started_at,
    );
}

/// [`fold_into`]'s `loose` only, for rows a fold made before skipped
/// calls' charges were kept (`CounterStore::loose_epoch`): the rows it
/// would skip add their charge; the rows it would fold add nothing, since
/// they were folded then.
pub(crate) fn fold_loose_only(
    loose: &mut Option<NearAiCharge>,
    rows: &[&RoutedExchange],
    speakers: &Speakers,
    ledger_money: bool,
) {
    for row in rows {
        if named(row, speakers).is_none() {
            add_loose(loose, row, ledger_money);
        }
    }
}

/// Whether a call's charge is NEAR AI's: through the NEAR AI route by its
/// proof label, or, with no label recorded, on the `nearai` backend (a
/// managed route's ledger names it so). `outside` never is.
fn charged_by_near_ai(row: &RoutedExchange) -> bool {
    through_near_ai(row.proof) || (row.proof.is_none() && row.backend == "nearai")
}

/// The amount a call adds. A call the provider refused (an HTTP 4xx or
/// 5xx status) with no recorded charge was not billed, and adds a known
/// $0 (owner question, 2026-10-28). Any other call with no charge (a
/// completed one, or one with no status) is unknown, never zero.
fn call_charge(row: &RoutedExchange) -> (Option<f64>, Option<u64>) {
    let refused = (400..600).contains(&row.status);
    let usd = match row.cost_usd {
        Some(usd) => Some(usd),
        None if refused => Some(0.0),
        None => None,
    };
    // Tokens as feed L counts them; a refused call with none recorded used
    // none.
    let tokens = call_tokens(row).or(refused.then_some(0));
    (usd, tokens)
}

fn session_of(row: &RoutedExchange) -> Option<&str> {
    row.client_session_id
        .as_deref()
        .filter(|session| !session.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use trace_commons_protocol::insights_usage_series::{DigestKeyStore, InMemoryDigestKeyStore};

    /// The convention `inference_map`'s tests use for an id that must never
    /// cross.
    const SESSION: &str = "SESSION-SECRET";

    fn key() -> DigestKey {
        InMemoryDigestKeyStore::with_seed([3; 32])
            .load_or_create()
            .unwrap()
    }

    fn at(minute: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 14, 9, 0, 0).unwrap() + Duration::minutes(minute)
    }

    /// A Codex call on its own endpoint, so the tool is named without any
    /// connected speaker: 100 input of which 40 cached, 10 output, no cache
    /// write.
    fn codex(id: i64, proof: Option<ProofStatus>) -> RoutedExchange {
        RoutedExchange {
            id: Some(id),
            started_at: at(id),
            client_session_id: Some(SESSION.to_string()),
            facade: "openai".to_string(),
            path: Some("/v1/responses".to_string()),
            backend: "SECRET-BACKEND".to_string(),
            input_tokens: Some(100),
            cache_read_tokens: Some(40),
            cache_write_tokens: Some(0),
            output_tokens: Some(10),
            cost_usd: Some(1.0),
            proof,
            ..Default::default()
        }
    }

    fn fold(
        routing: &mut BTreeMap<KeyedDigest, RouteTally>,
        cursor: &mut FoldCursor,
        window: &[RoutedExchange],
    ) -> FoldSummary {
        let plan = fold_plan(window, *cursor);
        let summary = fold_into(routing, &mut None, &key(), &plan, &Vec::new(), true);
        *cursor = plan.next;
        summary
    }

    fn only(routing: &BTreeMap<KeyedDigest, RouteTally>) -> &RouteTally {
        assert_eq!(routing.len(), 1);
        routing
            .get(&harness_session_digest(&key(), SOURCE_CODEX, SESSION))
            .expect("keyed by the harness-session digest")
    }

    #[test]
    fn every_proof_label_lands_in_its_bucket() {
        let cases: [(Option<ProofStatus>, fn(&RouteBuckets) -> u64); 8] = [
            (Some(ProofStatus::Verified), |b| b.verified),
            (Some(ProofStatus::GatewayOnly), |b| b.routed_unverified),
            (Some(ProofStatus::Unattested), |b| b.routed_unverified),
            (Some(ProofStatus::Pending), |b| b.routed_unverified),
            (Some(ProofStatus::Unavailable), |b| b.routed_unverified),
            (Some(ProofStatus::Failed), |b| b.check_failed),
            (Some(ProofStatus::Outside), |b| b.outside),
            (None, |b| b.unrecorded),
        ];
        for (proof, bucket) in cases {
            let mut routing = BTreeMap::new();
            let summary = fold(&mut routing, &mut FoldCursor::default(), &[codex(1, proof)]);
            assert_eq!(
                summary,
                FoldSummary {
                    folded: 1,
                    skipped: 0
                },
                "{proof:?}"
            );
            let tally = only(&routing);
            assert_eq!(bucket(&tally.calls), 1, "{proof:?}");
            assert_eq!(tally.calls.total(), 1, "{proof:?}");
            // OpenAI's cached input is inside input: 60 + 40 + 0 + 10.
            assert_eq!(bucket(&tally.tokens), 110, "{proof:?}");
            assert_eq!(tally.tokens.total(), 110, "{proof:?}");
            assert_eq!(tally.calls_without_counts.total(), 0);
            assert_eq!(tally.last_call_at, at(1));
        }
    }

    #[test]
    fn anthropic_counters_add_as_recorded() {
        let row = RoutedExchange {
            facade: "anthropic".to_string(),
            path: Some("/v1/messages".to_string()),
            input_tokens: Some(3),
            cache_read_tokens: Some(10),
            cache_write_tokens: Some(5),
            output_tokens: Some(2),
            ..codex(1, Some(ProofStatus::Verified))
        };
        let mut routing = BTreeMap::new();
        fold(&mut routing, &mut FoldCursor::default(), &[row]);
        let tally = routing
            .get(&harness_session_digest(&key(), SOURCE_CLAUDE_CODE, SESSION))
            .expect("Claude Code's digest");
        assert_eq!(tally.tokens.verified, 20);
    }

    #[test]
    fn a_call_with_unknown_counters_counts_where_it_went_and_adds_no_tokens() {
        let mut unknown = codex(2, Some(ProofStatus::Verified));
        unknown.output_tokens = None;
        let mut routing = BTreeMap::new();
        fold(
            &mut routing,
            &mut FoldCursor::default(),
            &[codex(1, Some(ProofStatus::Outside)), unknown],
        );
        let tally = only(&routing);
        assert_eq!(tally.calls.verified, 1);
        assert_eq!(tally.tokens.verified, 0);
        assert_eq!(tally.calls_without_counts.verified, 1);
        assert_eq!(tally.calls_without_counts.total(), 1);
        assert_eq!(route_category(Some(tally)).0, RouteCategory::Mixed);
    }

    #[test]
    fn rows_that_cannot_be_joined_or_named_are_skipped() {
        let mut empty = codex(1, Some(ProofStatus::Verified));
        empty.client_session_id = Some(String::new());
        let mut missing = codex(2, Some(ProofStatus::Verified));
        missing.client_session_id = None;
        // An endpoint no connection writes: the tool is unknown.
        let mut unnamed = codex(3, Some(ProofStatus::Verified));
        unnamed.path = Some("/v1/chat/completions".to_string());
        // A facade no tool speaks, with no endpoint recorded.
        let mut unspoken = codex(4, Some(ProofStatus::Verified));
        unspoken.path = None;
        let mut no_id = codex(5, Some(ProofStatus::Verified));
        no_id.id = None;
        let mut routing = BTreeMap::new();
        let mut cursor = FoldCursor::default();
        let summary = fold(
            &mut routing,
            &mut cursor,
            &[empty, missing, unnamed, unspoken, no_id],
        );
        // The id-less row is not even a candidate.
        assert_eq!(
            summary,
            FoldSummary {
                folded: 0,
                skipped: 4
            }
        );
        assert!(routing.is_empty());
        assert_eq!(cursor.through, Some(4));
    }

    #[test]
    fn an_old_proxy_row_is_named_by_the_one_connected_speaker() {
        let mut row = codex(1, Some(ProofStatus::Verified));
        row.path = None;
        let plan = fold_plan(std::slice::from_ref(&row), FoldCursor::default());
        let mut routing = BTreeMap::new();
        let summary = fold_into(
            &mut routing,
            &mut None,
            &key(),
            &plan,
            &vec![(SOURCE_CODEX, "openai")],
            true,
        );
        assert_eq!(summary.folded, 1);
        only(&routing);
    }

    #[test]
    fn folding_the_same_window_twice_counts_each_call_once() {
        let window: Vec<RoutedExchange> = (1..=3)
            .map(|id| codex(id, Some(ProofStatus::Verified)))
            .collect();
        let mut routing = BTreeMap::new();
        let mut cursor = FoldCursor::default();
        assert_eq!(fold(&mut routing, &mut cursor, &window).folded, 3);
        assert_eq!(fold(&mut routing, &mut cursor, &window).folded, 0);
        assert_eq!(only(&routing).calls.verified, 3);
        assert_eq!(cursor.through, Some(3));

        // A later window that grew by one, and lost its oldest row to age.
        let mut grown = window[1..].to_vec();
        grown.push(codex(4, Some(ProofStatus::Verified)));
        assert_eq!(fold(&mut routing, &mut cursor, &grown).folded, 1);
        assert_eq!(only(&routing).calls.verified, 4);
        assert_eq!(cursor.through, Some(4));

        // An empty window keeps the cursor.
        assert_eq!(fold(&mut routing, &mut cursor, &[]).folded, 0);
        assert_eq!(cursor.through, Some(4));
    }

    /// A call on a ledger that started over, `minutes` after the old
    /// ledger's last call began.
    fn after_restart(id: i64, minutes: i64) -> RoutedExchange {
        let mut row = codex(id, Some(ProofStatus::Outside));
        row.started_at = at(5 + minutes);
        row
    }

    #[test]
    fn a_ledger_that_started_over_is_folded_from_its_start() {
        let mut routing = BTreeMap::new();
        let mut cursor = FoldCursor::default();
        let window: Vec<RoutedExchange> = (1..=5)
            .map(|id| codex(id, Some(ProofStatus::Verified)))
            .collect();
        fold(&mut routing, &mut cursor, &window);
        assert_eq!(cursor.through, Some(5));
        // The proxy's ledger started over: ids 1 and 2 again, new calls,
        // the old ledger's rows gone with it.
        let restarted = [after_restart(1, 1), after_restart(2, 2)];
        assert_eq!(fold(&mut routing, &mut cursor, &restarted).folded, 2);
        assert_eq!(cursor.through, Some(2));
        let tally = only(&routing);
        assert_eq!((tally.calls.verified, tally.calls.outside), (5, 2));
        // And from there it grows as any ledger does.
        assert_eq!(fold(&mut routing, &mut cursor, &restarted).folded, 0);
        let mut grown = restarted.to_vec();
        grown.push(after_restart(3, 3));
        assert_eq!(fold(&mut routing, &mut cursor, &grown).folded, 1);
        assert_eq!(only(&routing).calls.outside, 3);
    }

    /// A ledger that started over and climbed past the cursor between two
    /// folds: its rows at or below the old cursor are new calls too.
    #[test]
    fn a_ledger_that_started_over_and_climbed_past_the_cursor_loses_nothing() {
        let mut routing = BTreeMap::new();
        let mut cursor = FoldCursor::default();
        let window: Vec<RoutedExchange> = (1..=2)
            .map(|id| codex(id, Some(ProofStatus::Verified)))
            .collect();
        fold(&mut routing, &mut cursor, &window);
        let restarted: Vec<RoutedExchange> = (1..=4).map(|id| after_restart(id, id)).collect();
        assert_eq!(fold(&mut routing, &mut cursor, &restarted).folded, 4);
        assert_eq!(cursor.through, Some(4));
        let tally = only(&routing);
        assert_eq!((tally.calls.verified, tally.calls.outside), (2, 4));
        assert_eq!(fold(&mut routing, &mut cursor, &restarted).folded, 0);
    }

    /// IronWire records a row when its body ends, so ids follow completion
    /// and `started_at` does not (`ironwire_ledger::page`'s doc). A long
    /// call that started first and ended last holds the highest id, and
    /// ages out of the window first: the window's highest id then falls
    /// below the cursor with no restart at all, and its rows were all
    /// folded.
    #[test]
    fn a_window_whose_newest_id_aged_out_is_not_folded_again() {
        let short = codex(10, Some(ProofStatus::Verified));
        let mut long = codex(11, Some(ProofStatus::Verified));
        long.started_at = at(0);
        let mut routing = BTreeMap::new();
        let mut cursor = FoldCursor::default();
        assert_eq!(
            fold(&mut routing, &mut cursor, &[long, short.clone()]).folded,
            2
        );
        assert_eq!(cursor.through, Some(11));
        // A day later the long call is out of the window, the short one in.
        assert_eq!(fold(&mut routing, &mut cursor, &[short]).folded, 0);
        assert_eq!(only(&routing).calls.verified, 2);
        // A new call after that still folds once.
        assert_eq!(
            fold(
                &mut routing,
                &mut cursor,
                &[codex(12, Some(ProofStatus::Verified))]
            )
            .folded,
            1
        );
        assert_eq!(only(&routing).calls.verified, 3);
    }

    fn tally(calls: RouteBuckets) -> RouteTally {
        RouteTally {
            calls,
            tokens: RouteBuckets::default(),
            calls_without_counts: RouteBuckets::default(),
            last_call_at: at(0),
            charge: None,
        }
    }

    fn buckets(v: u64, u: u64, f: u64, o: u64, n: u64) -> RouteBuckets {
        RouteBuckets {
            verified: v,
            routed_unverified: u,
            check_failed: f,
            outside: o,
            unrecorded: n,
        }
    }

    /// The NEAR AI charge of the one session: `None` with no NEAR AI call,
    /// `Some(None)` when unknown.
    fn charge(routing: &BTreeMap<KeyedDigest, RouteTally>) -> Option<Option<u64>> {
        session_charge(only(routing)).map(|charge| charge.usd_micros)
    }

    fn served(routing: &BTreeMap<KeyedDigest, RouteTally>) -> Option<String> {
        session_charge(only(routing)).and_then(|charge| charge.served_model)
    }

    #[test]
    fn a_fold_captures_the_near_ai_charge_and_a_later_fold_adds_to_it() {
        let mut first = codex(1, Some(ProofStatus::Verified));
        first.served_model = Some("GLM-4.6".to_string());
        first.cost_usd = Some(0.0125);
        let mut routing = BTreeMap::new();
        let mut cursor = FoldCursor::default();
        fold(&mut routing, &mut cursor, std::slice::from_ref(&first));
        assert_eq!(charge(&routing), Some(Some(12_500)));
        let hours = BTreeMap::from([(
            unix_hour(at(1)),
            HourCharge {
                calls: 1,
                usd_micros: 12_500,
                tokens: Some(110),
                tokens_known: 110,
                calls_with_tokens: 1,
                ..HourCharge::default()
            },
        )]);
        assert_eq!(
            only(&routing).charge,
            Some(NearAiCharge {
                usd_micros: 12_500,
                charged_calls: 1,
                calls_without_charge: 0,
                overflowed: false,
                served_model: Some("glm-4.6".to_string()),
                by_hour: hours.clone(),
                by_model: vec![ModelCharge {
                    model: Some("glm-4.6".to_string()),
                    by_hour: hours,
                }],
            })
        );

        // A later fold adds the new calls only, into the one charge.
        let mut second = codex(2, Some(ProofStatus::GatewayOnly));
        second.served_model = Some("GLM-4.6".to_string());
        second.cost_usd = Some(0.5);
        fold(&mut routing, &mut cursor, &[first, second.clone()]);
        assert_eq!(charge(&routing), Some(Some(512_500)));
        assert_eq!(served(&routing).as_deref(), Some("glm-4.6"));
        assert_eq!(only(&routing).charge.as_ref().unwrap().charged_calls, 2);
    }

    /// A tally keeps one charge, and a served model only while every call
    /// named the same one: a call that names another, or none (the
    /// requested model is not who served), leaves it unnamed for good.
    #[test]
    fn a_charge_names_a_served_model_only_while_every_call_names_the_same_one() {
        let named = |id: i64, model: Option<&str>| {
            let mut row = codex(id, Some(ProofStatus::Verified));
            row.served_model = model.map(str::to_owned);
            row.requested_model = Some("Requested-Model".to_string());
            row.cost_usd = Some(0.25);
            row
        };
        for (calls, expected) in [
            (vec![named(1, Some("A")), named(2, Some("a"))], Some("a")),
            (vec![named(1, Some("A")), named(2, Some("B"))], None),
            (vec![named(1, Some("A")), named(2, None)], None),
            (vec![named(1, None), named(2, Some("A"))], None),
            (
                vec![
                    named(1, Some("A")),
                    named(2, Some("B")),
                    named(3, Some("A")),
                ],
                None,
            ),
        ] {
            let mut routing = BTreeMap::new();
            fold(&mut routing, &mut FoldCursor::default(), &calls);
            assert_eq!(served(&routing).as_deref(), expected, "{calls:?}");
            assert_eq!(charge(&routing), Some(Some(250_000 * calls.len() as u64)));
        }
    }

    /// NEAR AI credit by model (product owner, 2026-10-29): each call's
    /// charge goes to the model that served it, so a session that used two
    /// models (a managed Claude session's main model and its Haiku alias)
    /// splits between them. A call that named no model is the unknown
    /// group; a call with no charge makes only its own model's figure
    /// partial.
    #[test]
    fn a_charge_splits_by_the_model_each_call_was_served_by() {
        let call = |id: i64, model: Option<&str>, usd: Option<f64>| {
            let mut row = codex(id, Some(ProofStatus::Verified));
            row.served_model = model.map(str::to_owned);
            row.requested_model = Some("Requested-Model".to_string());
            row.cost_usd = usd;
            row
        };
        let mut routing = BTreeMap::new();
        fold(
            &mut routing,
            &mut FoldCursor::default(),
            &[
                call(1, Some("GLM-4.6"), Some(0.5)),
                call(2, Some("glm-4.6"), Some(0.25)),
                call(3, Some("claude-haiku-4-5"), Some(0.125)),
                call(4, None, Some(1.0)),
                call(5, Some("claude-haiku-4-5"), None),
            ],
        );
        let tally = only(&routing);
        let charge = tally.charge.as_ref().unwrap();
        assert_eq!(charge.served_model, None, "the session mixed models");
        let models: BTreeMap<Option<&str>, HourCharge> = charge
            .within_by_model(tally.last_call_at, at(-600), at(600))
            .into_iter()
            .collect();
        assert_eq!(models.len(), 3, "{models:?}");
        let glm = models[&Some("glm-4.6")];
        assert_eq!((glm.calls, glm.known()), (2, Some(750_000)));
        let haiku = models[&Some("claude-haiku-4-5")];
        assert_eq!((haiku.calls, haiku.known()), (2, None));
        assert_eq!(
            haiku.known_part().map(|part| (part.sum, part.partial)),
            Some((125_000, true))
        );
        let unnamed = models[&None];
        assert_eq!((unnamed.calls, unnamed.known()), (1, Some(1_000_000)));
        // The groups add up to the whole.
        let whole = charge
            .within(tally.last_call_at, at(-600), at(600))
            .unwrap();
        let mut sum = HourCharge::default();
        for hour in models.values() {
            sum.absorb(hour);
        }
        assert_eq!(sum, whole);
        assert!(
            charge
                .within_by_model(tally.last_call_at, at(600), at(660))
                .is_empty()
        );
    }

    /// NEAR AI names its models `org/model`: by model keeps the whole name,
    /// lowercased, and a name that is not a plain id is the unknown group.
    #[test]
    fn by_model_keeps_near_ais_org_and_model_name() {
        let call = |id: i64, model: &str| {
            let mut row = codex(id, Some(ProofStatus::Verified));
            row.served_model = Some(model.to_string());
            row
        };
        let mut routing = BTreeMap::new();
        fold(
            &mut routing,
            &mut FoldCursor::default(),
            &[
                call(1, "Qwen/Qwen3.6-27B-FP8"),
                call(2, "qwen/qwen3.6-27b-fp8"),
                call(3, "<synthetic>"),
            ],
        );
        let tally = only(&routing);
        let models: BTreeMap<Option<&str>, u64> = tally
            .charge
            .as_ref()
            .unwrap()
            .within_by_model(tally.last_call_at, at(-600), at(600))
            .into_iter()
            .map(|(model, hour)| (model, hour.calls))
            .collect();
        assert_eq!(
            models,
            BTreeMap::from([(Some("qwen/qwen3.6-27b-fp8"), 2), (None, 1)])
        );
    }

    /// A charge kept before models were is read as one group, the served
    /// model it kept (`None` when its calls mixed models), and a later fold
    /// keeps that group and adds the new calls by their own model.
    #[test]
    fn a_charge_from_before_models_is_its_served_models() {
        let named = |id: i64, model: &str| {
            let mut row = codex(id, Some(ProofStatus::Verified));
            row.served_model = Some(model.to_string());
            row
        };
        let mut routing = BTreeMap::new();
        let mut cursor = FoldCursor::default();
        fold(&mut routing, &mut cursor, &[named(1, "GLM-4.6")]);
        let tally = routing.values_mut().next().unwrap();
        tally.charge.as_mut().unwrap().by_model.clear();
        let legacy = tally.charge.clone().unwrap();
        assert_eq!(
            legacy
                .within_by_model(tally.last_call_at, at(-600), at(600))
                .into_iter()
                .map(|(model, hour)| (model.map(str::to_owned), hour.known()))
                .collect::<Vec<_>>(),
            vec![(Some("glm-4.6".to_string()), Some(1_000_000))]
        );
        fold(&mut routing, &mut cursor, &[named(2, "claude-haiku-4-5")]);
        let tally = only(&routing);
        let charge = tally.charge.as_ref().unwrap();
        let models: BTreeMap<Option<&str>, Option<u64>> = charge
            .within_by_model(tally.last_call_at, at(-600), at(600))
            .into_iter()
            .map(|(model, hour)| (model, hour.known()))
            .collect();
        assert_eq!(
            models,
            BTreeMap::from([
                (Some("claude-haiku-4-5"), Some(1_000_000)),
                (Some("glm-4.6"), Some(1_000_000)),
            ])
        );
    }

    /// `LEDGER_MONEY_IN_INSIGHTS` held: the fold counts calls and tokens
    /// as before and takes no charge, so no money or served model is kept.
    #[test]
    fn held_ledger_money_takes_no_charge() {
        let mut row = codex(1, Some(ProofStatus::Verified));
        row.served_model = Some("GLM-4.6".to_string());
        let window = [row];
        let plan = fold_plan(&window, FoldCursor::default());
        let mut held = BTreeMap::new();
        let mut shown = BTreeMap::new();
        fold_into(&mut held, &mut None, &key(), &plan, &Vec::new(), false);
        fold_into(&mut shown, &mut None, &key(), &plan, &Vec::new(), true);
        assert!(only(&shown).charge.is_some());
        assert_eq!(only(&held).charge, None);
        assert_eq!(session_charge(only(&held)), None);
        assert_eq!(only(&held).calls, only(&shown).calls);
        assert_eq!(only(&held).tokens, only(&shown).tokens);
        let text = serde_json::to_string(&held).unwrap().to_lowercase();
        assert!(
            !text.contains("usd_micros") && !text.contains("glm"),
            "{text}"
        );
    }

    #[test]
    fn only_calls_through_the_near_ai_route_are_charged() {
        let mut routing = BTreeMap::new();
        fold(
            &mut routing,
            &mut FoldCursor::default(),
            &[codex(1, Some(ProofStatus::Outside)), codex(2, None)],
        );
        assert_eq!(only(&routing).charge, None);
        assert_eq!(charge(&routing), None, "no NEAR AI traffic, no charge");
    }

    #[test]
    fn a_call_without_a_charge_makes_the_sessions_charge_unknown_never_zero() {
        let mut missing = codex(2, Some(ProofStatus::Verified));
        missing.cost_usd = None;
        let mut routing = BTreeMap::new();
        let mut cursor = FoldCursor::default();
        fold(
            &mut routing,
            &mut cursor,
            &[codex(1, Some(ProofStatus::Verified)), missing],
        );
        let entry = only(&routing).charge.clone().unwrap();
        assert_eq!((entry.charged_calls, entry.calls_without_charge), (1, 1));
        assert_eq!(charge(&routing), Some(None));
        // A later priced call does not make it known again.
        fold(
            &mut routing,
            &mut cursor,
            &[codex(3, Some(ProofStatus::Verified))],
        );
        assert_eq!(charge(&routing), Some(None));

        // A charge that is not a finite, non-negative amount is unknown too.
        for bad in [f64::NAN, f64::INFINITY, -0.01, 1e20] {
            let mut row = codex(1, Some(ProofStatus::Verified));
            row.cost_usd = Some(bad);
            let mut routing = BTreeMap::new();
            fold(&mut routing, &mut FoldCursor::default(), &[row]);
            assert_eq!(charge(&routing), Some(None), "{bad}");
        }
    }

    #[test]
    fn a_charge_that_overflows_is_unknown() {
        let mut routing = BTreeMap::new();
        let mut cursor = FoldCursor::default();
        fold(
            &mut routing,
            &mut cursor,
            &[codex(1, Some(ProofStatus::Verified))],
        );
        routing.values_mut().next().unwrap().charge = Some(NearAiCharge {
            usd_micros: u64::MAX,
            charged_calls: 1,
            ..NearAiCharge::default()
        });
        fold(
            &mut routing,
            &mut cursor,
            &[codex(2, Some(ProofStatus::Verified))],
        );
        assert!(only(&routing).charge.as_ref().unwrap().overflowed);
        assert_eq!(charge(&routing), Some(None));
    }

    /// The ledger keeps a call for 24 hours; the charge stays in the tally
    /// after the call has left the window.
    #[test]
    fn the_charge_outlives_the_ledger_window() {
        let mut routing = BTreeMap::new();
        let mut cursor = FoldCursor::default();
        let window: Vec<RoutedExchange> = (1..=2)
            .map(|id| codex(id, Some(ProofStatus::Verified)))
            .collect();
        fold(&mut routing, &mut cursor, &window);
        assert_eq!(charge(&routing), Some(Some(2_000_000)));
        // Every row aged out of the ledger.
        assert_eq!(fold(&mut routing, &mut cursor, &[]).folded, 0);
        assert_eq!(charge(&routing), Some(Some(2_000_000)));
    }

    #[test]
    fn a_tally_without_a_charge_loads_and_writes_as_before() {
        let old = serde_json::json!({
            "calls": buckets(1, 0, 0, 0, 0),
            "tokens": buckets(110, 0, 0, 0, 0),
            "calls_without_counts": RouteBuckets::default(),
            "last_call_at": at(0),
        });
        let tally: RouteTally = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(tally.charge, None);
        assert_eq!(session_charge(&tally), None);
        assert_eq!(serde_json::to_value(&tally).unwrap(), old);
    }

    #[test]
    fn categories_follow_the_precedence() {
        use RouteCategory::*;
        let unrecorded = Some(RouteReason::SomeCallsUnrecorded);
        let cases = [
            (buckets(0, 0, 0, 0, 0), (Unobserved, None)),
            (buckets(0, 0, 0, 0, 3), (Unrecorded, None)),
            (buckets(0, 0, 0, 2, 0), (Outside, None)),
            (buckets(0, 0, 0, 2, 1), (Outside, unrecorded)),
            (buckets(1, 0, 0, 2, 0), (Mixed, None)),
            (buckets(0, 0, 1, 1, 0), (Mixed, None)),
            (buckets(1, 1, 1, 0, 0), (CheckFailed, None)),
            (buckets(0, 0, 1, 0, 1), (CheckFailed, unrecorded)),
            (buckets(4, 0, 0, 0, 0), (RoutedVerified, None)),
            (buckets(4, 0, 0, 0, 1), (RoutedVerified, unrecorded)),
            (buckets(4, 1, 0, 0, 0), (RoutedUnverified, None)),
            (buckets(0, 1, 0, 0, 2), (RoutedUnverified, unrecorded)),
        ];
        for (calls, expected) in cases {
            assert_eq!(route_category(Some(&tally(calls))), expected, "{calls:?}");
        }
        assert_eq!(route_category(None), (Unobserved, None));
    }

    #[test]
    fn gateway_only_alone_is_never_verified() {
        let mut routing = BTreeMap::new();
        fold(
            &mut routing,
            &mut FoldCursor::default(),
            &[codex(1, Some(ProofStatus::GatewayOnly))],
        );
        assert_eq!(
            route_category(Some(only(&routing))),
            (RouteCategory::RoutedUnverified, None)
        );
    }

    #[test]
    fn wire_spellings_never_say_private() {
        use RouteCategory::*;
        for category in [
            Unobserved,
            Unrecorded,
            Outside,
            Mixed,
            CheckFailed,
            RoutedVerified,
            RoutedUnverified,
        ] {
            assert!(!category.as_str().to_lowercase().contains("private"));
        }
        assert_eq!(
            RouteReason::SomeCallsUnrecorded.as_str(),
            "some_calls_unrecorded"
        );
    }

    /// The tally holds no id, no backend and no list price. The one money
    /// figure is the NEAR AI charge, as integer USD micros.
    #[test]
    fn a_tally_holds_no_id_backend_or_list_price() {
        let mut routing = BTreeMap::new();
        fold(
            &mut routing,
            &mut FoldCursor::default(),
            &[codex(1, Some(ProofStatus::Verified))],
        );
        let text = serde_json::to_string(only(&routing)).unwrap();
        for leak in [SESSION, "SECRET-BACKEND", "cost", "price", "1.0"] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
        assert!(text.contains("\"usd_micros\":1000000"), "{text}");
    }

    #[test]
    fn no_tally_crosses_as_unobserved_with_no_figure() {
        let empty = tally(RouteBuckets::default());
        for tally in [None, Some(&empty)] {
            assert_eq!(
                routing_value(tally),
                serde_json::json!({
                    "category": "unobserved",
                    "reasons": [],
                    "tokens": null,
                    "calls": null,
                    "calls_without_counts": null,
                })
            );
        }
    }

    /// A bucket holding a call whose counters are unknown has an unknown
    /// token figure: never the zero its counted calls happen to sum to.
    #[test]
    fn a_bucket_with_an_uncounted_call_reads_unknown_never_zero() {
        let mut uncounted = codex(2, Some(ProofStatus::Verified));
        uncounted.output_tokens = None;
        let mut routing = BTreeMap::new();
        fold(
            &mut routing,
            &mut FoldCursor::default(),
            &[codex(1, Some(ProofStatus::Outside)), uncounted],
        );
        assert_eq!(
            routing_value(Some(only(&routing))),
            serde_json::json!({
                "category": "mixed",
                "reasons": [],
                "tokens": {
                    "verified": null,
                    "routed_unverified": 0,
                    "check_failed": 0,
                    "outside": 110,
                    "unrecorded": 0,
                },
                "calls": 2,
                "calls_without_counts": 1,
            })
        );
    }

    #[test]
    fn a_session_with_some_unrecorded_calls_says_so() {
        let mut routing = BTreeMap::new();
        fold(
            &mut routing,
            &mut FoldCursor::default(),
            &[codex(1, Some(ProofStatus::Verified)), codex(2, None)],
        );
        let value = routing_value(Some(only(&routing)));
        assert_eq!(value["category"], "routed_verified");
        assert_eq!(
            value["reasons"],
            serde_json::json!(["some_calls_unrecorded"])
        );
        assert_eq!(value["tokens"]["verified"], 110);
        assert_eq!(value["tokens"]["unrecorded"], 110);
        assert_eq!(value["calls"], 2);
        assert_eq!(value["calls_without_counts"], 0);
        let text = value.to_string();
        for leak in [SESSION, "SECRET-BACKEND", "cost", "last_call_at", "private"] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
    }

    #[test]
    fn prune_ages_out_unmatched_tallies_drops_gone_ones_and_caps() {
        let digest = |n: u8| KeyedDigest([n; 32]);
        let mut routing: BTreeMap<KeyedDigest, RouteTally> = (1..=4)
            .map(|n| {
                let mut t = tally(buckets(1, 0, 0, 0, 0));
                t.last_call_at = at(i64::from(n));
                (digest(n), t)
            })
            .collect();
        let cutoff = at(3);
        // 1 is old and matched: kept. 2 is old and unmatched: dropped.
        // 4 is gone: dropped whatever its age.
        let matched = BTreeSet::from([digest(1)]);
        let gone = BTreeSet::from([digest(4)]);
        prune(&mut routing, &matched, &gone, cutoff, 10);
        assert_eq!(
            routing.keys().copied().collect::<Vec<_>>(),
            vec![digest(1), digest(3)]
        );
        // Past the cap the oldest goes, matched or not.
        prune(&mut routing, &matched, &BTreeSet::new(), cutoff, 1);
        assert_eq!(routing.keys().copied().collect::<Vec<_>>(), vec![digest(3)]);
    }

    /// Product owner, 2026-10-28: a window takes only the calls made in
    /// it. Calls in two hours land in two hours; a call with no charge
    /// makes its hour unknown and leaves the other known.
    #[test]
    fn a_charge_is_kept_by_the_hour_its_calls_started_in() {
        let mut early = codex(1, Some(ProofStatus::Verified));
        early.cost_usd = Some(0.25);
        let mut late = codex(2, Some(ProofStatus::Verified));
        late.started_at = at(90);
        late.cost_usd = None;
        let mut routing = BTreeMap::new();
        fold(&mut routing, &mut FoldCursor::default(), &[early, late]);
        let tally = only(&routing);
        let charge = tally.charge.as_ref().unwrap();
        assert_eq!(charge.known(), None, "one call had no charge");
        let first = charge
            .within(tally.last_call_at, at(0), at(60))
            .expect("the first hour had a call");
        assert_eq!((first.calls, first.known()), (1, Some(250_000)));
        let second = charge.within(tally.last_call_at, at(60), at(120)).unwrap();
        assert_eq!((second.calls, second.known()), (1, None));
        let both = charge.within(tally.last_call_at, at(0), at(120)).unwrap();
        assert_eq!((both.calls, both.known()), (2, None));
        assert_eq!(charge.within(tally.last_call_at, at(120), at(180)), None);
    }

    /// A charge kept before hours were is read as one hour, its last
    /// call's, and a later fold keeps the hours summing to the whole.
    #[test]
    fn a_charge_from_before_hours_is_its_last_calls_hour() {
        let mut routing = BTreeMap::new();
        let mut cursor = FoldCursor::default();
        fold(
            &mut routing,
            &mut cursor,
            &[codex(1, Some(ProofStatus::Verified))],
        );
        let tally = routing.values_mut().next().unwrap();
        tally.charge.as_mut().unwrap().by_hour.clear();
        let legacy = tally.charge.clone().unwrap();
        assert_eq!(
            legacy
                .within(tally.last_call_at, at(0), at(60))
                .and_then(|hour| hour.known()),
            Some(1_000_000)
        );
        let mut later = codex(2, Some(ProofStatus::Verified));
        later.started_at = at(120);
        fold(&mut routing, &mut cursor, &[later]);
        let tally = only(&routing);
        let charge = tally.charge.as_ref().unwrap();
        assert_eq!(charge.by_hour.len(), 2);
        let all = charge
            .within(tally.last_call_at, at(-600), at(600))
            .unwrap();
        assert_eq!(all.known(), charge.known());
        assert_eq!(all.calls, 2);
    }
}
