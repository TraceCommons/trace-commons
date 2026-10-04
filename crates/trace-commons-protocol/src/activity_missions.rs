//! Read-only trace-activity missions. Separate from skill-evaluation rewards.
//!
//! Response envelopes and progress details ignore additive fields, which are
//! discarded rather than forwarded to clients. Policy and rule schemas remain
//! strict: every field affects interpretation or the canonical policy digest,
//! so their evolution requires an explicitly supported schema version.
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
    pub fn parse(bytes: &[u8]) -> Result<Self, InvalidActivityPolicy> {
        if bytes.len() > MAX_POLICY_BYTES {
            return Err(InvalidActivityPolicy);
        }
        let policy: Self = serde_json::from_slice(bytes).map_err(|_| InvalidActivityPolicy)?;
        policy.validate()?;
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
}
