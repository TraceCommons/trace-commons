//! Read-only trace-activity missions. Separate from skill-evaluation rewards.
//!
//! Response envelopes and progress details ignore additive fields, which are
//! discarded rather than forwarded to clients. Policy and rule schemas remain
//! strict: every field affects interpretation or the canonical policy digest,
//! so their evolution requires an explicitly supported schema version.
//!
//! The one versioned extension point inside that strictness is a mission's
//! optional [`MissionPredicate`]: what local work a mission asks for, for a
//! client to match on its own machine. A version-1 block is read strictly; a
//! block of a later version is carried verbatim, still digest-covered, and
//! treated as absent, never half-read. Every predicate block serializes as
//! key-sorted JSON (see [`crate::canonical_json`]), and a later version may
//! hold no float and no integer outside i64/u64 (a block that does is
//! malformed), so a reader that does not know a version re-serializes it to
//! the same bytes and the policy digest still agrees. The server never evaluates a predicate and accepts no
//! matching result.
use crate::trace_contribution::{TaskSuccess, TraceFailureMode};
use chrono::{DateTime, Datelike, Days, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_POLICY_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActivityPolicy {
    pub schema_version: u32,
    pub policy_id: String,
    pub starts_on: NaiveDate,
    /// Exclusive UTC day boundary.
    pub ends_before: NaiveDate,
    pub qualification: Qualification,
    pub missions: Vec<ActivityMission>,
    pub daily: Option<DailyRule>,
    pub levels: Option<Vec<Threshold>>,
    pub badges: Option<Vec<BadgeRule>>,
    /// Model labels a version-2 predicate's `"frontier"` stands for. Absent
    /// on a policy that names none, and then not serialized, so no digest
    /// from before this field moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontier_models: Option<Vec<String>>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Qualification {
    Accepted,
    ReceivedOrAccepted,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActivityMission {
    pub id: String,
    pub title: String,
    pub required_contributions: u32,
    /// What local work the mission asks for. Absent on a count-only
    /// mission, and then not serialized, so a policy without predicates
    /// keeps the digest it had before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicate: Option<MissionPredicate>,
}

impl ActivityMission {
    /// The predicate a client may match on: a version-1 block. `None` for
    /// a mission without one and for a version this build does not read,
    /// which counts as having no predicate.
    pub fn supported_predicate(&self) -> Option<&MissionPredicateV1> {
        match self.predicate.as_ref()? {
            MissionPredicate::V1(predicate) => Some(predicate),
            MissionPredicate::V2(_) | MissionPredicate::Unsupported(_) => None,
        }
    }
}

/// The first predicate version this build reads.
pub const MISSION_PREDICATE_VERSION: u32 = 1;
/// The outcome-aware predicate version this build also reads.
pub const MISSION_PREDICATE_VERSION_V2: u32 = 2;
/// The most values one predicate list may carry. Equal to the client
/// catalogue's per-criterion bound.
pub const MAX_PREDICATE_VALUES: usize = 32;
/// The most bytes one predicate value may have.
pub const MAX_PREDICATE_LABEL_BYTES: usize = 64;
/// The most readable sessions a predicate may ask for.
pub const MAX_PREDICATE_MIN_SESSIONS: u32 = 1000;
/// The most characters the title of a mission carrying a predicate may have:
/// the client catalogue's title bound, tighter than the policy's own.
pub const MAX_PREDICATE_TITLE_CHARS: usize = 200;

/// A mission's versioned predicate block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissionPredicate {
    /// Read and validated.
    V1(MissionPredicateV1),
    /// Read and validated. Outcome-aware; a version-1 client does not match
    /// on it (see [`ActivityMission::supported_predicate`]).
    V2(MissionPredicateV2),
    /// A later version, kept key-sorted so the digest still verifies, and
    /// never interpreted. It may hold no float and no integer outside
    /// i64/u64, which would not re-serialize to the bytes the server hashed.
    /// An operator policy refuses it.
    Unsupported(serde_json::Value),
}

/// Version 1: every list is "any of" and an empty list does not restrict;
/// the lists combine with AND per session. A mission fits when at least
/// `min_sessions` readable sessions satisfy every non-empty list. At least
/// one list is non-empty, so a predicate never fits everything.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionPredicateV1 {
    pub version: u32,
    /// Session sources, e.g. `claude-code`, `codex`, `gemini-cli`.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Tool families, e.g. `anthropic`, `openai`, `google`.
    #[serde(default)]
    pub tool_families: Vec<String>,
    /// Languages a folder is worked in, e.g. `rust`, `python`.
    #[serde(default)]
    pub languages: Vec<String>,
    pub min_sessions: u32,
}

impl MissionPredicateV1 {
    fn validate(&self) -> Result<(), InvalidActivityPolicy> {
        let lists = [&self.tools, &self.tool_families, &self.languages];
        if self.version != MISSION_PREDICATE_VERSION
            || !(1..=MAX_PREDICATE_MIN_SESSIONS).contains(&self.min_sessions)
            || lists.iter().all(|list| list.is_empty())
        {
            return Err(InvalidActivityPolicy);
        }
        for list in lists {
            if list.len() > MAX_PREDICATE_VALUES
                || list.iter().collect::<BTreeSet<_>>().len() != list.len()
                || !list.iter().all(|value| predicate_label(value))
            {
                return Err(InvalidActivityPolicy);
            }
        }
        Ok(())
    }
}

/// The lowest evidence tier a version-2 predicate counts. E3 is
/// server-side, so a client predicate cannot ask for it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PredicateMinTier {
    E0,
    E1,
    E2,
}

/// Which models a version-2 predicate asks for: the policy's
/// `frontier_models` (the string `"frontier"`) or an explicit label list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PredicateModels {
    Frontier,
    List(Vec<String>),
}

impl Serialize for PredicateModels {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            PredicateModels::Frontier => serializer.serialize_str("frontier"),
            PredicateModels::List(list) => list.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for PredicateModels {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Keyword(String),
            List(Vec<String>),
        }
        match Repr::deserialize(deserializer)? {
            Repr::Keyword(word) if word == "frontier" => Ok(PredicateModels::Frontier),
            Repr::Keyword(_) => Err(serde::de::Error::custom("unknown models keyword")),
            Repr::List(list) => Ok(PredicateModels::List(list)),
        }
    }
}

/// Version 2: version 1's lists and `min_sessions`, plus outcome fields.
/// Every list is "any of", fields combine with AND, and at least one field
/// restricts (a non-empty list, `models`, `min_tier`, or
/// `with_correction: true`). Matching stays on the device.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionPredicateV2 {
    pub version: u32,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub tool_families: Vec<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    /// Task outcomes, in snake_case.
    #[serde(default)]
    pub outcomes: Vec<TaskSuccess>,
    /// Failure modes, in snake_case. The free-form `other` mode is refused.
    #[serde(default)]
    pub failure_modes: Vec<TraceFailureMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<PredicateModels>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_tier: Option<PredicateMinTier>,
    #[serde(default)]
    pub with_correction: bool,
    pub min_sessions: u32,
}

impl MissionPredicateV2 {
    fn validate(&self, frontier_models: Option<&Vec<String>>) -> Result<(), InvalidActivityPolicy> {
        if self.version != MISSION_PREDICATE_VERSION_V2
            || !(1..=MAX_PREDICATE_MIN_SESSIONS).contains(&self.min_sessions)
        {
            return Err(InvalidActivityPolicy);
        }
        for list in [&self.tools, &self.tool_families, &self.languages] {
            label_list(list)?;
        }
        // Each enum list: bounded and without duplicates (the enums are
        // closed, so every value is already a known label).
        if self.outcomes.len() > MAX_PREDICATE_VALUES
            || self
                .outcomes
                .iter()
                .enumerate()
                .any(|(i, o)| self.outcomes[..i].contains(o))
        {
            return Err(InvalidActivityPolicy);
        }
        if self.failure_modes.len() > MAX_PREDICATE_VALUES
            || self
                .failure_modes
                .iter()
                .any(|m| matches!(m, TraceFailureMode::Other(_)))
            || self.failure_modes.iter().collect::<BTreeSet<_>>().len() != self.failure_modes.len()
        {
            return Err(InvalidActivityPolicy);
        }
        match &self.models {
            Some(PredicateModels::List(list)) => label_list(list)?,
            Some(PredicateModels::Frontier) if frontier_models.is_none() => {
                return Err(InvalidActivityPolicy);
            }
            _ => (),
        }
        let restricts = !self.tools.is_empty()
            || !self.tool_families.is_empty()
            || !self.languages.is_empty()
            || !self.outcomes.is_empty()
            || !self.failure_modes.is_empty()
            || matches!(&self.models, Some(PredicateModels::Frontier))
            || matches!(&self.models, Some(PredicateModels::List(l)) if !l.is_empty())
            || self.min_tier.is_some()
            || self.with_correction;
        if restricts {
            Ok(())
        } else {
            Err(InvalidActivityPolicy)
        }
    }
}

/// A predicate label list: at most [`MAX_PREDICATE_VALUES`] values, each a
/// valid label, none repeated.
fn label_list(list: &[String]) -> Result<(), InvalidActivityPolicy> {
    if list.len() > MAX_PREDICATE_VALUES
        || list.iter().collect::<BTreeSet<_>>().len() != list.len()
        || !list.iter().all(|value| predicate_label(value))
    {
        return Err(InvalidActivityPolicy);
    }
    Ok(())
}

fn predicate_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PREDICATE_LABEL_BYTES
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

impl Serialize for MissionPredicate {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Both variants through the same canonical form, so the digest is
        // taken over key-sorted bytes whatever map backs serde_json here.
        let value = match self {
            MissionPredicate::V1(predicate) => {
                serde_json::to_value(predicate).map_err(serde::ser::Error::custom)?
            }
            MissionPredicate::V2(predicate) => {
                serde_json::to_value(predicate).map_err(serde::ser::Error::custom)?
            }
            MissionPredicate::Unsupported(value) => value.clone(),
        };
        crate::canonical_json::canonical_value(&value).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for MissionPredicate {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = serde_json::Value::deserialize(deserializer)?;
        // The version decides how the rest is read; without a positive
        // integer version the block is malformed, never unsupported.
        let version = value
            .as_object()
            .and_then(|object| object.get("version"))
            .and_then(serde_json::Value::as_u64)
            .filter(|version| *version >= 1)
            .ok_or_else(|| D::Error::custom("activity_missions_predicate_invalid"))?;
        if version == u64::from(MISSION_PREDICATE_VERSION) {
            serde_json::from_value(value)
                .map(MissionPredicate::V1)
                .map_err(|_| D::Error::custom("activity_missions_predicate_invalid"))
        } else if version == u64::from(MISSION_PREDICATE_VERSION_V2) {
            serde_json::from_value(value)
                .map(MissionPredicate::V2)
                .map_err(|_| D::Error::custom("activity_missions_predicate_invalid"))
        } else if exact_json(&value) {
            Ok(MissionPredicate::Unsupported(
                crate::canonical_json::canonical_value(&value),
            ))
        } else {
            Err(D::Error::custom("activity_missions_predicate_invalid"))
        }
    }
}

/// Whether `value` re-serializes to the bytes it was parsed from, given
/// key-sorted output: true unless it holds a number the default parser does
/// not keep exactly -- a float, or an integer outside i64/u64, which it reads
/// as a float. A later-version predicate block is held to this, so the
/// digest a reader recomputes over it is the one the server took.
fn exact_json(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Number(number) => number.is_i64() || number.is_u64(),
        serde_json::Value::Array(items) => items.iter().all(exact_json),
        serde_json::Value::Object(map) => map.values().all(exact_json),
        _ => true,
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DailyRule {
    Fixed { mission_id: String },
    Rotation { mission_ids: Vec<String> },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Threshold {
    pub id: String,
    pub required: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BadgeRule {
    pub id: String,
    pub metric: BadgeMetric,
    pub required: u32,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BadgeMetric {
    MonthlyContributions,
    CompletedDays,
    CurrentStreak,
}

/// Aggregated, authoritative input; never accepted from an HTTP caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityDay {
    pub day: NaiveDate,
    pub contributions: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DailyProgress {
    pub day: NaiveDate,
    pub mission_id: String,
    pub contributions: u64,
    pub required_contributions: u32,
    pub complete: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BadgeProgress {
    pub id: String,
    pub achieved: Option<bool>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivityProgress {
    pub schema_version: u32,
    pub kind: String,
    pub policy_sha256: String,
    pub observed_at: DateTime<Utc>,
    pub source: String,
    pub qualification: Qualification,
    pub coverage_starts_on: NaiveDate,
    pub month_starts_on: NaiveDate,
    pub monthly_contributions: u64,
    pub daily: Option<DailyProgress>,
    pub completed_days: Option<u32>,
    pub current_streak: Option<u32>,
    pub level: Option<String>,
    pub levels_configured: bool,
    pub badges: Option<Vec<BadgeProgress>>,
    pub rewards_enabled: bool,
    pub credit_points_pending: Option<u64>,
    pub credit_condition: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("activity_missions_invalid")]
pub struct InvalidActivityPolicy;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivityCatalogue {
    pub schema_version: u32,
    pub kind: String,
    pub state: String,
    pub policy_sha256: Option<String>,
    pub policy: Option<ActivityPolicy>,
    pub rewards_enabled: bool,
    pub credit_points_pending: Option<u64>,
    pub credit_condition: String,
}
impl ActivityCatalogue {
    pub fn new(policy: Option<ActivityPolicy>) -> Result<Self, InvalidActivityPolicy> {
        let digest = policy.as_ref().map(ActivityPolicy::digest).transpose()?;
        Ok(Self {
            schema_version: 1,
            kind: "trace_activity".into(),
            state: if policy.is_some() {
                "configured"
            } else {
                "unconfigured"
            }
            .into(),
            policy_sha256: digest,
            policy,
            rewards_enabled: false,
            credit_points_pending: None,
            credit_condition: "mission_credit_ledger_unavailable".into(),
        })
    }
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
impl ActivityPolicy {
    /// Parse an operator policy. Stricter than [`Self::validate`]: a
    /// predicate of a version this build does not read is refused, so a
    /// server never publishes a block it cannot validate.
    pub fn parse(bytes: &[u8]) -> Result<Self, InvalidActivityPolicy> {
        if bytes.len() > MAX_POLICY_BYTES {
            return Err(InvalidActivityPolicy);
        }
        let policy: Self = serde_json::from_slice(bytes).map_err(|_| InvalidActivityPolicy)?;
        policy.validate()?;
        if policy
            .missions
            .iter()
            .any(|m| matches!(m.predicate, Some(MissionPredicate::Unsupported(_))))
        {
            return Err(InvalidActivityPolicy);
        }
        Ok(policy)
    }
    pub fn validate(&self) -> Result<(), InvalidActivityPolicy> {
        let duration = (self.ends_before - self.starts_on).num_days();
        if self.schema_version != 1
            || !identifier(&self.policy_id)
            || !(1..=366).contains(&duration)
            || self.missions.is_empty()
            || self.missions.len() > 128
            || serde_json::to_vec(self)
                .map_err(|_| InvalidActivityPolicy)?
                .len()
                > MAX_POLICY_BYTES
        {
            return Err(InvalidActivityPolicy);
        }
        if let Some(models) = &self.frontier_models {
            if models.is_empty() {
                return Err(InvalidActivityPolicy);
            }
            label_list(models)?;
        }
        let mut ids = BTreeSet::new();
        for mission in &self.missions {
            if !identifier(&mission.id)
                || !ids.insert(&mission.id)
                || mission.required_contributions == 0
                || mission.title.trim().is_empty()
                || mission.title.chars().count() > 240
                || mission.title.chars().any(char::is_control)
            {
                return Err(InvalidActivityPolicy);
            }
            // A published version-1 or -2 block must fit what the client
            // catalogue accepts. A later version is the reader's to ignore;
            // its size is bounded by the policy's.
            match &mission.predicate {
                Some(MissionPredicate::V1(predicate)) => predicate.validate()?,
                Some(MissionPredicate::V2(predicate)) => {
                    predicate.validate(self.frontier_models.as_ref())?
                }
                _ => continue,
            }
            if mission.title.chars().count() > MAX_PREDICATE_TITLE_CHARS {
                return Err(InvalidActivityPolicy);
            }
        }
        match &self.daily {
            Some(DailyRule::Fixed { mission_id }) if !ids.contains(mission_id) => {
                return Err(InvalidActivityPolicy);
            }
            Some(DailyRule::Rotation { mission_ids })
                if mission_ids.is_empty()
                    || mission_ids.len() > 128
                    || mission_ids.iter().any(|id| !ids.contains(id))
                    || mission_ids.iter().collect::<BTreeSet<_>>().len() != mission_ids.len() =>
            {
                return Err(InvalidActivityPolicy);
            }
            _ => (),
        }
        if let Some(levels) = &self.levels {
            let mut previous = 0;
            let mut ids = BTreeSet::new();
            if levels.is_empty() || levels.len() > 128 {
                return Err(InvalidActivityPolicy);
            }
            for level in levels {
                if !identifier(&level.id) || !ids.insert(&level.id) || level.required <= previous {
                    return Err(InvalidActivityPolicy);
                }
                previous = level.required;
            }
        }
        if let Some(badges) = &self.badges {
            let mut ids = BTreeSet::new();
            if badges.is_empty() || badges.len() > 128 {
                return Err(InvalidActivityPolicy);
            }
            for badge in badges {
                if !identifier(&badge.id) || !ids.insert(&badge.id) || badge.required == 0 {
                    return Err(InvalidActivityPolicy);
                }
            }
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<String, InvalidActivityPolicy> {
        self.validate()?;
        Ok(hex::encode(Sha256::digest(
            serde_json::to_vec(self).map_err(|_| InvalidActivityPolicy)?,
        )))
    }
    fn assignment(&self, day: NaiveDate) -> Option<&ActivityMission> {
        let id = match self.daily.as_ref()? {
            DailyRule::Fixed { mission_id } => mission_id,
            DailyRule::Rotation { mission_ids } => {
                &mission_ids[((day - self.starts_on).num_days() as usize) % mission_ids.len()]
            }
        };
        self.missions.iter().find(|m| &m.id == id)
    }
    pub fn evaluate(
        &self,
        days: &[ActivityDay],
        now: DateTime<Utc>,
    ) -> Result<ActivityProgress, InvalidActivityPolicy> {
        self.validate()?;
        let today = now.date_naive();
        if today < self.starts_on || today >= self.ends_before || days.len() > 366 {
            return Err(InvalidActivityPolicy);
        }
        let mut facts = BTreeMap::new();
        for fact in days {
            if fact.day < self.starts_on
                || fact.day > today
                || facts.insert(fact.day, fact.contributions).is_some()
            {
                return Err(InvalidActivityPolicy);
            }
        }
        let month = today.with_day(1).ok_or(InvalidActivityPolicy)?;
        let monthly_contributions = facts
            .range(month..)
            .try_fold(0u64, |acc, (_, count)| acc.checked_add(*count))
            .ok_or(InvalidActivityPolicy)?;
        let mut completed = BTreeSet::new();
        let mut cursor = self.starts_on;
        while cursor <= today {
            if let Some(mission) = self.assignment(cursor) {
                if facts.get(&cursor).copied().unwrap_or(0)
                    >= u64::from(mission.required_contributions)
                {
                    completed.insert(cursor);
                }
            }
            cursor = cursor
                .checked_add_days(Days::new(1))
                .ok_or(InvalidActivityPolicy)?;
        }
        let mut streak = 0;
        let mut cursor = if completed.contains(&today) {
            Some(today)
        } else {
            today.pred_opt()
        };
        while let Some(day) = cursor {
            if !completed.contains(&day) {
                break;
            }
            streak += 1;
            cursor = day.pred_opt();
        }
        let current_streak = self.daily.as_ref().map(|_| streak);
        let completed_days = self.daily.as_ref().map(|_| completed.len() as u32);
        let daily = self.assignment(today).map(|mission| DailyProgress {
            day: today,
            mission_id: mission.id.clone(),
            contributions: facts.get(&today).copied().unwrap_or(0),
            required_contributions: mission.required_contributions,
            complete: completed.contains(&today),
        });
        let level = self
            .levels
            .as_ref()
            .and_then(|levels| {
                levels
                    .iter()
                    .rev()
                    .find(|l| monthly_contributions >= u64::from(l.required))
            })
            .map(|l| l.id.clone());
        let badges = self.badges.as_ref().map(|badges| {
            badges
                .iter()
                .map(|badge| {
                    let metric = match badge.metric {
                        BadgeMetric::MonthlyContributions => Some(monthly_contributions),
                        BadgeMetric::CompletedDays => completed_days.map(u64::from),
                        BadgeMetric::CurrentStreak => current_streak.map(u64::from),
                    };
                    BadgeProgress {
                        id: badge.id.clone(),
                        achieved: metric.map(|value| value >= u64::from(badge.required)),
                    }
                })
                .collect()
        });
        Ok(ActivityProgress {
            schema_version: 1,
            kind: "trace_activity".into(),
            policy_sha256: self.digest()?,
            observed_at: now,
            source: "account_contributed_submissions".into(),
            qualification: self.qualification,
            coverage_starts_on: self.starts_on,
            month_starts_on: month,
            monthly_contributions,
            daily,
            completed_days,
            current_streak,
            level,
            levels_configured: self.levels.is_some(),
            badges,
            rewards_enabled: false,
            credit_points_pending: None,
            credit_condition: "mission_credit_ledger_unavailable".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> ActivityPolicy {
        ActivityPolicy::parse(br#"{"schema_version":1,"policy_id":"test-policy","starts_on":"2026-09-01","ends_before":"2026-11-01","qualification":"accepted","missions":[{"id":"one","title":"One contribution","required_contributions":1},{"id":"two","title":"Two contributions","required_contributions":2}],"daily":{"kind":"rotation","mission_ids":["one","two"]},"levels":[{"id":"starter","required":2}],"badges":[{"id":"consistent","metric":"current_streak","required":2}]}"#).unwrap()
    }
    fn day(s: &str, contributions: u64) -> ActivityDay {
        ActivityDay {
            day: s.parse().unwrap(),
            contributions,
        }
    }
    #[test]
    fn activity_missions_response_extensions_are_ignored() {
        let catalogue = ActivityCatalogue::new(Some(policy())).unwrap();
        let mut value = serde_json::to_value(&catalogue).unwrap();
        value["display_hint"] = serde_json::json!({"future": "ignored"});
        assert_eq!(
            serde_json::from_value::<ActivityCatalogue>(value).unwrap(),
            catalogue
        );

        let progress = policy()
            .evaluate(&[], "2026-10-02T12:00:00Z".parse().unwrap())
            .unwrap();
        let mut value = serde_json::to_value(&progress).unwrap();
        value["display_hint"] = serde_json::json!({"future": "ignored"});
        value["daily"]["display_hint"] = true.into();
        value["badges"][0]["display_hint"] = true.into();
        assert_eq!(
            serde_json::from_value::<ActivityProgress>(value).unwrap(),
            progress
        );
    }

    #[test]
    fn activity_missions_policy_extensions_remain_fail_closed() {
        let catalogue = ActivityCatalogue::new(Some(policy())).unwrap();
        let original = serde_json::to_value(catalogue).unwrap();
        for path in [
            "/policy",
            "/policy/missions/0",
            "/policy/daily",
            "/policy/levels/0",
            "/policy/badges/0",
        ] {
            let mut value = original.clone();
            value.pointer_mut(path).unwrap()["future_rule"] = true.into();
            assert!(
                serde_json::from_value::<ActivityCatalogue>(value).is_err(),
                "{path}"
            );
        }
        // The predicate is the one versioned extension point inside that
        // strictness: an unknown field in a version-1 block still fails,
        // while a block of a later version is carried and ignored.
        let mut value = original.clone();
        value["policy"]["missions"][0]["predicate"] =
            serde_json::json!({"version":1,"tools":["codex"],"min_sessions":1,"future_rule":true});
        assert!(serde_json::from_value::<ActivityCatalogue>(value).is_err());
        let mut value = original.clone();
        value["policy"]["missions"][0]["predicate"] =
            serde_json::json!({"version":3,"future_rule":true});
        assert!(serde_json::from_value::<ActivityCatalogue>(value).is_ok());
    }

    #[test]
    fn activity_missions_month_reset_and_global_rotation_use_real_counts() {
        let p = policy();
        let r = p
            .evaluate(
                &[day("2026-09-30", 2), day("2026-10-01", 1)],
                "2026-10-02T12:00:00Z".parse().unwrap(),
            )
            .unwrap();
        assert_eq!(r.monthly_contributions, 1);
        assert_eq!(r.daily.unwrap().mission_id, "two");
        assert_eq!(r.current_streak, Some(2));
        assert_eq!(r.level, None);
        assert_eq!(r.badges.unwrap()[0].achieved, Some(true));
        assert!(!r.rewards_enabled);
        assert_eq!(r.credit_points_pending, None);
    }
    #[test]
    fn activity_missions_no_rules_does_not_invent_daily_level_or_badge() {
        let mut p = policy();
        p.daily = None;
        p.levels = None;
        p.badges = None;
        let r = p
            .evaluate(&[], "2026-10-02T00:00:00Z".parse().unwrap())
            .unwrap();
        assert_eq!(r.daily, None);
        assert_eq!(r.current_streak, None);
        assert_eq!(r.completed_days, None);
        assert_eq!(r.badges, None);
        assert!(!r.levels_configured);
    }
    #[test]
    fn activity_missions_reject_reward_activation_and_duplicate_facts() {
        let p = policy();
        let mut value = serde_json::to_value(&p).unwrap();
        value["rewards_enabled"] = true.into();
        assert!(ActivityPolicy::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(
            p.evaluate(
                &[day("2026-10-01", 1), day("2026-10-01", 1)],
                "2026-10-02T00:00:00Z".parse().unwrap()
            )
            .is_err()
        );
    }
    #[test]
    fn activity_missions_withdrawal_recomputation_removes_completion_and_badge() {
        let p = policy();
        let now = "2026-10-02T12:00:00Z".parse().unwrap();
        let full = p
            .evaluate(
                &[
                    day("2026-09-30", 2),
                    day("2026-10-01", 1),
                    day("2026-10-02", 2),
                ],
                now,
            )
            .unwrap();
        assert_eq!(full.current_streak, Some(3));
        assert!(full.daily.unwrap().complete);
        let retracted = p
            .evaluate(&[day("2026-09-30", 2), day("2026-10-02", 1)], now)
            .unwrap();
        assert_eq!(retracted.current_streak, Some(0));
        assert!(!retracted.daily.unwrap().complete);
        assert_eq!(retracted.badges.unwrap()[0].achieved, Some(false));
        assert_eq!(retracted.credit_points_pending, None);
    }
    #[test]
    fn activity_missions_reject_invalid_policy_references_bounds_and_thresholds() {
        let p = policy();
        let mut q = p.clone();
        q.ends_before = q.starts_on;
        assert!(q.validate().is_err());
        let mut q = p.clone();
        q.ends_before = q.starts_on + chrono::Duration::days(367);
        assert!(q.validate().is_err());
        let mut q = p.clone();
        q.daily = Some(DailyRule::Fixed {
            mission_id: "absent".into(),
        });
        assert!(q.validate().is_err());
        let mut q = p.clone();
        q.missions.push(q.missions[0].clone());
        assert!(q.validate().is_err());
        let mut q = p.clone();
        q.levels = Some(vec![
            Threshold {
                id: "a".into(),
                required: 2,
            },
            Threshold {
                id: "b".into(),
                required: 1,
            },
        ]);
        assert!(q.validate().is_err());
        let mut q = p.clone();
        q.missions[0].required_contributions = 0;
        assert!(q.validate().is_err());
        let mut q = p.clone();
        q.missions[0].title = "untrusted\nline".into();
        assert!(q.validate().is_err());
        assert!(ActivityPolicy::parse(&vec![b' '; MAX_POLICY_BYTES + 1]).is_err());
        let mut value = serde_json::to_value(p).unwrap();
        value["activity_profile"] = serde_json::json!({});
        assert!(ActivityPolicy::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    #[test]
    fn activity_missions_utc_end_exclusive_and_future_or_overflow_facts_refused() {
        let p = policy();
        assert!(
            p.evaluate(&[], "2026-11-01T00:00:00Z".parse().unwrap())
                .is_err()
        );
        assert!(
            p.evaluate(&[], "2026-08-31T23:59:59Z".parse().unwrap())
                .is_err()
        );
        let now = "2026-10-02T00:00:00Z".parse().unwrap();
        assert!(p.evaluate(&[day("2026-10-03", 1)], now).is_err());
        assert!(
            p.evaluate(&[day("2026-10-01", u64::MAX), day("2026-10-02", 1)], now)
                .is_err()
        );
        assert!(p.evaluate(&[day("2026-08-31", 1)], now).is_err());
    }
    #[test]
    fn activity_missions_digest_covers_selection_and_catalogue_has_no_economic_value() {
        let p = policy();
        let digest = p.digest().unwrap();
        let mut changed = p.clone();
        changed.missions[0].required_contributions = 3;
        assert_ne!(digest, changed.digest().unwrap());
        let whitespace = serde_json::to_vec_pretty(&p).unwrap();
        assert_eq!(
            digest,
            ActivityPolicy::parse(&whitespace)
                .unwrap()
                .digest()
                .unwrap()
        );
        for catalog in [
            ActivityCatalogue::new(None).unwrap(),
            ActivityCatalogue::new(Some(p)).unwrap(),
        ] {
            assert!(!catalog.rewards_enabled);
            assert_eq!(catalog.credit_points_pending, None);
            assert_eq!(
                catalog.credit_condition,
                "mission_credit_ledger_unavailable"
            );
        }
    }

    /// The policy every other test uses, with a predicate on its first
    /// mission, as operator JSON.
    fn predicate_policy_json(predicate: serde_json::Value) -> serde_json::Value {
        let mut value = serde_json::to_value(policy()).unwrap();
        value["missions"][0]["predicate"] = predicate;
        value
    }
    fn parse_value(value: &serde_json::Value) -> Result<ActivityPolicy, InvalidActivityPolicy> {
        ActivityPolicy::parse(&serde_json::to_vec(value).unwrap())
    }
    fn v1() -> serde_json::Value {
        serde_json::json!({"version":1,"tools":["claude-code"],"tool_families":[],"languages":["rust"],"min_sessions":2})
    }

    /// Adding the optional predicate moved no deployed digest: a policy
    /// without one serializes byte for byte as it did before it existed.
    #[test]
    fn activity_missions_predicate_free_digest_is_unchanged() {
        assert_eq!(
            policy().digest().unwrap(),
            "58a46687243041bd87fff7de28953b25c618e96809d725f73759cc29e75b1886"
        );
        let text = serde_json::to_string(&policy()).unwrap();
        assert!(!text.contains("predicate"), "{text}");
    }

    /// A version-1 predicate is read whole, is covered by the digest, and
    /// the digest does not depend on the order its keys arrived in.
    #[test]
    fn activity_missions_v1_predicate_is_read_and_digest_covered() {
        let parsed = parse_value(&predicate_policy_json(v1())).unwrap();
        let read = parsed.missions[0].supported_predicate().unwrap();
        assert_eq!(read.tools, vec!["claude-code"]);
        assert!(read.tool_families.is_empty());
        assert_eq!(read.languages, vec!["rust"]);
        assert_eq!(read.min_sessions, 2);
        assert_eq!(parsed.missions[1].supported_predicate(), None);
        assert_ne!(parsed.digest().unwrap(), policy().digest().unwrap());

        let mut wider = v1();
        wider["min_sessions"] = 3.into();
        assert_ne!(
            parse_value(&predicate_policy_json(wider))
                .unwrap()
                .digest()
                .unwrap(),
            parsed.digest().unwrap(),
            "every predicate field is digest-covered"
        );

        // The same predicate written with its keys in reverse order.
        // Key-sorted text first, so the replacement below finds the block
        // whatever map backs serde_json in this build.
        let text =
            crate::canonical_json::to_canonical_string(&predicate_policy_json(v1())).unwrap();
        let reversed = text.replace(
            r#"{"languages":["rust"],"min_sessions":2,"tool_families":[],"tools":["claude-code"],"version":1}"#,
            r#"{"version":1,"tools":["claude-code"],"tool_families":[],"min_sessions":2,"languages":["rust"]}"#,
        );
        assert_ne!(text, reversed, "the replacement must have happened");
        assert_eq!(
            ActivityPolicy::parse(reversed.as_bytes())
                .unwrap()
                .digest()
                .unwrap(),
            parsed.digest().unwrap()
        );
        // Emitted key-sorted, whatever map backs serde_json in this build.
        let emitted = serde_json::to_string(&parsed).unwrap();
        assert!(emitted.contains(
            r#""predicate":{"languages":["rust"],"min_sessions":2,"tool_families":[],"tools":["claude-code"],"version":1}"#
        ), "{emitted}");
    }

    /// Version 1 is strict and bounded to what the client catalogue
    /// accepts, so a policy the server publishes can never make the client
    /// refuse its whole mission catalogue.
    #[test]
    fn activity_missions_v1_predicate_bounds_are_enforced() {
        let refused = |mutate: &dyn Fn(&mut serde_json::Value)| {
            let mut predicate = v1();
            mutate(&mut predicate);
            parse_value(&predicate_policy_json(predicate)).is_err()
        };
        assert!(!refused(&|_| {}));
        assert!(refused(&|p| p["future_rule"] = true.into()), "v1 is strict");
        assert!(refused(&|p| p["min_sessions"] = 0.into()));
        assert!(refused(&|p| {
            p["min_sessions"] = (MAX_PREDICATE_MIN_SESSIONS + 1).into()
        }));
        assert!(refused(&|p| p
            .as_object_mut()
            .unwrap()
            .remove("min_sessions")
            .map(drop)
            .unwrap_or(())));
        assert!(
            refused(&|p| {
                p["tools"] = serde_json::json!([]);
                p["languages"] = serde_json::json!([]);
            }),
            "a predicate that restricts nothing would fit every session"
        );
        let many: Vec<String> = (0..=MAX_PREDICATE_VALUES)
            .map(|i| format!("t{i}"))
            .collect();
        assert!(refused(&|p| p["tools"] = serde_json::json!(many)));
        assert!(refused(
            &|p| p["languages"] = serde_json::json!(["rust", "rust"])
        ));
        for bad in [
            "",
            "Rust Lang",
            "rust\n",
            "ünicode",
            &"x".repeat(MAX_PREDICATE_LABEL_BYTES + 1),
        ] {
            assert!(
                refused(&|p| p["tool_families"] = serde_json::json!([bad])),
                "{bad:?}"
            );
        }
        for version in [
            serde_json::json!(0),
            serde_json::json!("1"),
            serde_json::json!(-1),
            serde_json::Value::Null,
        ] {
            assert!(refused(&|p| p["version"] = version.clone()), "{version}");
        }
        assert!(refused(&|p| p
            .as_object_mut()
            .unwrap()
            .remove("version")
            .map(drop)
            .unwrap_or(())));
        assert!(parse_value(&predicate_policy_json(serde_json::json!([1]))).is_err());
        assert!(parse_value(&predicate_policy_json(serde_json::json!("v1"))).is_err());

        // A predicate-bearing mission's title must fit the client's bound.
        let mut value = predicate_policy_json(v1());
        value["missions"][0]["title"] = "t".repeat(MAX_PREDICATE_TITLE_CHARS + 1).into();
        assert!(parse_value(&value).is_err());
        value["missions"][0]["title"] = "t".repeat(MAX_PREDICATE_TITLE_CHARS).into();
        assert!(parse_value(&value).is_ok());
        // ...and a mission without one keeps the wider title bound.
        let mut value = serde_json::to_value(policy()).unwrap();
        value["missions"][0]["title"] = "t".repeat(MAX_PREDICATE_TITLE_CHARS + 1).into();
        assert!(parse_value(&value).is_ok());
    }

    /// A predicate version this build does not know is never half-read: a
    /// reader keeps it verbatim for the digest and treats the mission as
    /// having no predicate. Only an operator policy refuses it, so a server
    /// never publishes a version it cannot validate.
    #[test]
    fn activity_missions_unknown_predicate_version_is_ignored_not_half_read() {
        let v3 = serde_json::json!({"version":3,"tools":["claude-code"],"min_sessions":1,"repo_size":"large"});
        let operator = predicate_policy_json(v3.clone());
        assert!(parse_value(&operator).is_err(), "the server refuses it");

        // What a later server would publish: a v1 and a v2 mission.
        let mut published = predicate_policy_json(v3);
        published["missions"][1]["predicate"] = v1();
        let reader: ActivityPolicy = serde_json::from_value(published.clone()).unwrap();
        assert!(reader.validate().is_ok());
        assert_eq!(reader.missions[0].supported_predicate(), None);
        assert_eq!(
            reader.missions[1]
                .supported_predicate()
                .map(|p| p.min_sessions),
            Some(2)
        );
        // Its digest is taken over the same canonical bytes either way the
        // unknown block's keys arrive.
        let text = crate::canonical_json::to_canonical_string(&published).unwrap();
        let reordered = text.replace(
            r#"{"min_sessions":1,"repo_size":"large","tools":["claude-code"],"version":3}"#,
            r#"{"version":3,"repo_size":"large","tools":["claude-code"],"min_sessions":1}"#,
        );
        assert_ne!(text, reordered, "the replacement must have happened");
        let again: ActivityPolicy = serde_json::from_str(&reordered).unwrap();
        assert_eq!(again.digest().unwrap(), reader.digest().unwrap());
        assert_ne!(reader.digest().unwrap(), policy().digest().unwrap());

        let catalogue = ActivityCatalogue {
            policy_sha256: Some(reader.digest().unwrap()),
            policy: Some(reader.clone()),
            ..ActivityCatalogue::new(None).unwrap()
        };
        let round: ActivityCatalogue =
            serde_json::from_slice(&serde_json::to_vec(&catalogue).unwrap()).unwrap();
        assert_eq!(round, catalogue);
    }

    /// A later-version block may carry only JSON a reader without
    /// `float_roundtrip` or `arbitrary_precision` re-serializes to the same
    /// bytes: no float and no integer outside i64/u64. Either one, anywhere
    /// in the block, makes it malformed, so the digest a reader recomputes
    /// is never taken over bytes that differ from what the server hashed.
    #[test]
    fn activity_missions_unknown_predicate_version_refuses_numbers_that_do_not_round_trip() {
        for number in [
            "1.331993865736232e-34",
            "0.5",
            "1.0",
            "18446744073709551616",
            "-9223372036854775809",
        ] {
            for block in [
                format!(r#"{{"version":3,"weight":{number}}}"#),
                format!(r#"{{"version":3,"nested":{{"list":[1,{number}]}}}}"#),
            ] {
                let mut published = serde_json::to_string(&predicate_policy_json(
                    serde_json::json!({"version": 3}),
                ))
                .unwrap();
                published = published.replace(r#"{"version":3}"#, &block);
                assert!(published.contains(number), "{published}");
                assert!(
                    serde_json::from_str::<ActivityPolicy>(&published).is_err(),
                    "{block}"
                );
            }
        }
        // Integers inside i64/u64, strings, booleans, null, arrays and
        // objects are carried.
        let carried = serde_json::json!({"version":3,"max":u64::MAX,"min":i64::MIN,"on":true,"off":null,"nested":{"list":["a",1]}});
        let reader: ActivityPolicy =
            serde_json::from_value(predicate_policy_json(carried)).unwrap();
        assert_eq!(reader.missions[0].supported_predicate(), None);
    }

    fn v2() -> serde_json::Value {
        serde_json::json!({"version":2,"tools":["claude-code","codex"],"outcomes":["failure","partial"],
            "failure_modes":["looping_or_repetition","missing_verification"],
            "models":["claude-opus-4"],"min_tier":"e2","with_correction":true,"min_sessions":1})
    }
    fn frontier_policy_json(predicate: serde_json::Value) -> serde_json::Value {
        let mut value = predicate_policy_json(predicate);
        value["frontier_models"] = serde_json::json!(["claude-opus-4", "gpt-5"]);
        value
    }

    /// Version 2 is read whole, digest-covered, and published; the
    /// version-1 accessor (`supported_predicate`) does not return it.
    #[test]
    fn activity_missions_v2_predicate_is_read_and_digest_covered() {
        let parsed = parse_value(&predicate_policy_json(v2())).unwrap();
        let Some(MissionPredicate::V2(read)) = &parsed.missions[0].predicate else {
            panic!("v2 not read");
        };
        assert_eq!(read.tools, vec!["claude-code", "codex"]);
        assert_eq!(
            read.outcomes,
            vec![TaskSuccess::Failure, TaskSuccess::Partial]
        );
        assert_eq!(
            read.failure_modes,
            vec![
                TraceFailureMode::LoopingOrRepetition,
                TraceFailureMode::MissingVerification
            ]
        );
        assert_eq!(
            read.models,
            Some(PredicateModels::List(vec!["claude-opus-4".into()]))
        );
        assert_eq!(read.min_tier, Some(PredicateMinTier::E2));
        assert!(read.with_correction);
        assert_eq!(parsed.missions[0].supported_predicate(), None);
        let emitted = serde_json::to_string(&parsed).unwrap();
        assert!(emitted.contains(r#""min_tier":"e2""#), "{emitted}");
        assert!(
            emitted.contains(r#""outcomes":["failure","partial"]"#),
            "{emitted}"
        );
        let round: ActivityPolicy = serde_json::from_str(&emitted).unwrap();
        assert_eq!(round.digest().unwrap(), parsed.digest().unwrap());
        for (key, value) in [
            ("min_tier", serde_json::json!("e1")),
            ("with_correction", serde_json::json!(false)),
            ("outcomes", serde_json::json!(["failure"])),
            ("failure_modes", serde_json::json!(["context_loss"])),
            ("models", serde_json::json!(["gpt-5"])),
        ] {
            let mut changed = v2();
            changed[key] = value;
            assert_ne!(
                parse_value(&predicate_policy_json(changed))
                    .unwrap()
                    .digest()
                    .unwrap(),
                parsed.digest().unwrap(),
                "{key} is digest-covered"
            );
        }
    }

    #[test]
    fn activity_missions_v2_models_frontier_needs_policy_frontier_models() {
        let mut p = v2();
        p["models"] = "frontier".into();
        assert!(parse_value(&predicate_policy_json(p.clone())).is_err());
        let ok = parse_value(&frontier_policy_json(p.clone())).unwrap();
        assert_eq!(ok.frontier_models.as_ref().unwrap().len(), 2);
        let Some(MissionPredicate::V2(read)) = &ok.missions[0].predicate else {
            panic!()
        };
        assert_eq!(read.models, Some(PredicateModels::Frontier));
        // An explicit list does not need the policy list, and a policy list
        // without a frontier mission is fine.
        assert!(parse_value(&predicate_policy_json(v2())).is_ok());
        assert!(parse_value(&frontier_policy_json(v2())).is_ok());
        // Any other string is not the keyword.
        p["models"] = "Frontier".into();
        assert!(parse_value(&frontier_policy_json(p.clone())).is_err());
        p["models"] = "other".into();
        assert!(parse_value(&frontier_policy_json(p)).is_err());
    }

    #[test]
    fn activity_missions_frontier_models_is_bounded_and_digest_neutral_when_absent() {
        let refused = |list: serde_json::Value| {
            let mut value = serde_json::to_value(policy()).unwrap();
            value["frontier_models"] = list;
            parse_value(&value).is_err()
        };
        assert!(!refused(serde_json::json!(["gpt-5"])));
        assert!(refused(serde_json::json!([])));
        assert!(refused(serde_json::json!(["gpt-5", "gpt-5"])));
        assert!(refused(serde_json::json!(["GPT 5"])));
        assert!(refused(serde_json::json!([""])));
        assert!(refused(serde_json::json!([
            "x".repeat(MAX_PREDICATE_LABEL_BYTES + 1)
        ])));
        let many: Vec<String> = (0..=MAX_PREDICATE_VALUES)
            .map(|i| format!("m{i}"))
            .collect();
        assert!(refused(serde_json::json!(many)));
        assert!(refused(serde_json::json!("frontier")));
        assert!(
            !serde_json::to_string(&policy())
                .unwrap()
                .contains("frontier_models")
        );
        let mut value = serde_json::to_value(policy()).unwrap();
        value["frontier_models"] = serde_json::json!(["gpt-5"]);
        assert_ne!(
            parse_value(&value).unwrap().digest().unwrap(),
            policy().digest().unwrap()
        );
    }

    /// Version 2 carries V1's rules and bounds its own fields.
    #[test]
    fn activity_missions_v2_predicate_bounds_are_enforced() {
        let refused = |mutate: &dyn Fn(&mut serde_json::Value)| {
            let mut predicate = v2();
            mutate(&mut predicate);
            parse_value(&frontier_policy_json(predicate)).is_err()
        };
        assert!(!refused(&|_| {}));
        assert!(refused(&|p| p["future_rule"] = true.into()), "v2 is strict");
        assert!(refused(&|p| p["min_sessions"] = 0.into()));
        assert!(refused(&|p| {
            p["min_sessions"] = (MAX_PREDICATE_MIN_SESSIONS + 1).into()
        }));
        assert!(refused(&|p| p
            .as_object_mut()
            .unwrap()
            .remove("min_sessions")
            .map(drop)
            .unwrap_or(())));
        assert!(refused(
            &|p| p["tools"] = serde_json::json!(["codex", "codex"])
        ));
        assert!(refused(&|p| p["tools"] = serde_json::json!(["Codex"])));
        assert!(refused(&|p| p["tool_families"] = serde_json::json!([""])));
        let many: Vec<String> = (0..=MAX_PREDICATE_VALUES)
            .map(|i| format!("t{i}"))
            .collect();
        assert!(refused(&|p| p["languages"] = serde_json::json!(many)));
        // Enum-valued fields.
        assert!(refused(
            &|p| p["outcomes"] = serde_json::json!(["failure", "failure"])
        ));
        assert!(refused(&|p| p["outcomes"] = serde_json::json!(["Failure"])));
        assert!(refused(&|p| p["outcomes"] = serde_json::json!(["win"])));
        assert!(!refused(
            &|p| p["outcomes"] = serde_json::json!(["success", "unknown"])
        ));
        assert!(refused(
            &|p| p["failure_modes"] = serde_json::json!(["context_loss", "context_loss"])
        ));
        assert!(refused(
            &|p| p["failure_modes"] = serde_json::json!(["made_up"])
        ));
        assert!(
            refused(&|p| p["failure_modes"] = serde_json::json!([{"other": "x"}])),
            "no free-form mode"
        );
        let modes = vec!["tool_selection_error"; MAX_PREDICATE_VALUES + 1];
        assert!(refused(&|p| p["failure_modes"] = serde_json::json!(modes)));
        // min_tier: e0, e1, e2 only.
        for tier in ["e0", "e1", "e2"] {
            assert!(!refused(&|p| p["min_tier"] = tier.into()), "{tier}");
        }
        for tier in ["e3", "E2", "", "2"] {
            assert!(refused(&|p| p["min_tier"] = tier.into()), "{tier}");
        }
        assert!(refused(&|p| p["min_tier"] = 2.into()));
        // models.
        assert!(refused(&|p| p["models"] = serde_json::json!(["a", "a"])));
        assert!(refused(&|p| p["models"] = serde_json::json!(["Bad Model"])));
        assert!(refused(&|p| p["models"] = serde_json::json!(many)));
        assert!(refused(&|p| p["models"] = serde_json::json!(true)));
        assert!(refused(&|p| p["with_correction"] = "yes".into()));
        // A predicate that restricts nothing would fit every session.
        let empty = serde_json::json!({"version":2,"tools":[],"outcomes":[],"failure_modes":[],
            "models":[],"with_correction":false,"min_sessions":1});
        assert!(parse_value(&frontier_policy_json(empty)).is_err());
        // Each new field restricts on its own.
        for only in [
            serde_json::json!({"version":2,"outcomes":["failure"],"min_sessions":1}),
            serde_json::json!({"version":2,"failure_modes":["context_loss"],"min_sessions":1}),
            serde_json::json!({"version":2,"models":["gpt-5"],"min_sessions":1}),
            serde_json::json!({"version":2,"models":"frontier","min_sessions":1}),
            serde_json::json!({"version":2,"min_tier":"e1","min_sessions":1}),
            serde_json::json!({"version":2,"with_correction":true,"min_sessions":1}),
        ] {
            assert!(
                parse_value(&frontier_policy_json(only.clone())).is_ok(),
                "{only}"
            );
        }
        // The title bound applies.
        let mut value = frontier_policy_json(v2());
        value["missions"][0]["title"] = "t".repeat(MAX_PREDICATE_TITLE_CHARS + 1).into();
        assert!(parse_value(&value).is_err());
    }

    /// V1 and V2 missions coexist; a V1 reader's accessor does not move.
    #[test]
    fn activity_missions_v1_and_v2_missions_coexist() {
        let mut value = predicate_policy_json(v2());
        value["missions"][1]["predicate"] = v1();
        let parsed = parse_value(&value).unwrap();
        assert_eq!(parsed.missions[0].supported_predicate(), None);
        assert_eq!(
            parsed.missions[1]
                .supported_predicate()
                .unwrap()
                .min_sessions,
            2
        );
    }
}
