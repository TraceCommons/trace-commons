//! Which catalog publishers Skills Home shows as recently active.
//!
//! Everything here is read from the server's signed catalog document, and
//! from nothing else: public repository signals the server measured
//! anonymously. Nothing local takes part. The inventory, usage, follows and
//! traces are not arguments, so two installs holding the same document rank
//! the same publishers in the same order.
//!
//! The signals describe a whole repository, not its skills: a repository
//! that holds skills beside other work counts all of its commits.

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use trace_commons_protocol::skills_catalog::{
    CatalogDocument, CatalogPublisher, CatalogSkill, SourceKind,
};

/// Publishers shown, and skills shown for each.
pub(crate) const MAX_PUBLISHERS: usize = 3;
pub(crate) const MAX_SKILLS: usize = 3;

/// Below this a repository is not ranked. Commits are cheap to make and the
/// place is on every Home, so a repository needs some standing first.
const MIN_STARS: u64 = 10;
/// Commits past this count no further, for the same reason: among busy
/// repositories the order is by stars.
const COMMITS_CAP: u32 = 20;
/// `commits_7d` is the week before the signals were measured, not before
/// now, and a document may be held for weeks. Older signals are not recent.
const MAX_SIGNAL_AGE_DAYS: i64 = 14;

fn eligible(publisher: &CatalogPublisher, now: DateTime<Utc>) -> Option<(u32, u64)> {
    let signals = &publisher.signals;
    // A publisher known only from an app's report is left out: its place in
    // the catalog came from what someone has installed.
    if publisher.source_kind == SourceKind::Suggested || signals.archived == Some(true) {
        return None;
    }
    let age = now.signed_duration_since(signals.checked_at);
    if age < Duration::zero() || age > Duration::days(MAX_SIGNAL_AGE_DAYS) {
        return None;
    }
    // Unknown is not zero: a missing count ranks nowhere.
    let stars = signals.stars.filter(|stars| *stars >= MIN_STARS)?;
    let commits = signals.commits_7d.filter(|commits| *commits >= 1)?;
    Some((commits.min(COMMITS_CAP), stars))
}

/// Every eligible publisher with a published skill, most active first:
/// commits in the measured week (capped), then stars, then id.
pub(crate) fn ranked(document: &CatalogDocument, now: DateTime<Utc>) -> Vec<&CatalogPublisher> {
    let mut found: Vec<_> = document
        .publishers
        .iter()
        .filter(|publisher| {
            document
                .skills
                .iter()
                .any(|skill| skill.publisher_id == publisher.id)
        })
        .filter_map(|publisher| eligible(publisher, now).map(|key| (key, publisher)))
        .collect();
    found.sort_by(|a, b| {
        (b.0.0, b.0.1)
            .cmp(&(a.0.0, a.0.1))
            .then_with(|| a.1.id.cmp(&b.1.id))
    });
    found.into_iter().map(|(_, publisher)| publisher).collect()
}

/// A publisher's skills, latest change first, then by id.
pub(crate) fn skills<'a>(
    document: &'a CatalogDocument,
    publisher_id: &str,
) -> Vec<&'a CatalogSkill> {
    let mut found: Vec<_> = document
        .skills
        .iter()
        .filter(|skill| skill.publisher_id == publisher_id)
        .collect();
    found.sort_by(|a, b| {
        b.last_changed_at
            .cmp(&a.last_changed_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    found
}

/// The snapshot's rows: the first [`MAX_PUBLISHERS`] ranked publishers that
/// have a skill the snapshot carries, each with up to [`MAX_SKILLS`] of
/// those. `emitted` answers the id of the catalog row sent for a skill, or
/// `None` when the snapshot's size cap cut it: a row never names a skill the
/// app was not also given.
pub(crate) fn rows(
    document: &CatalogDocument,
    now: DateTime<Utc>,
    emitted: impl Fn(&CatalogSkill) -> Option<String>,
) -> Vec<Value> {
    ranked(document, now)
        .into_iter()
        .filter_map(|publisher| {
            let ids: Vec<String> = skills(document, &publisher.id)
                .into_iter()
                .filter_map(&emitted)
                .take(MAX_SKILLS)
                .collect();
            (!ids.is_empty()).then(|| json!({"publisher_id": publisher.id, "skill_ids": ids}))
        })
        .take(MAX_PUBLISHERS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const REV: &str = "0123456789abcdef0123456789abcdef01234567";

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn publisher(id: &str, kind: &str, signals: Value) -> Value {
        json!({"id": id, "name": id, "repository": format!("{id}/skills"),
               "url": format!("https://github.com/{id}/skills"),
               "instructions_url": format!("https://github.com/{id}/skills"),
               "source_kind": kind, "signals": signals})
    }

    fn signals(stars: u64, week: u32) -> Value {
        json!({"stars": stars, "commits_7d": week, "commits_28d": week * 3, "checked_at": now()})
    }

    fn skill(publisher: &str, name: &str, changed_days_ago: i64) -> Value {
        let path = format!("{name}/SKILL.md");
        json!({"id": format!("{publisher}:{path}"), "publisher_id": publisher, "name": name,
               "description": "", "repository": format!("{publisher}/skills"), "path": path,
               "revision": REV, "sha256": "a".repeat(64),
               "source_url": format!("https://github.com/{publisher}/skills/blob/{REV}/{name}/SKILL.md"),
               "license": {"status": "unknown"},
               "first_seen_at": now() - Duration::days(60),
               "last_changed_at": now() - Duration::days(changed_days_ago),
               "checks": {"frontmatter": "valid", "document_bytes": 10, "directory_bytes": 10,
                          "directory_files": 1, "bundled": {"scripts": 0, "executables": 0, "binaries": 0},
                          "patterns": [], "inactive": false}})
    }

    fn document(publishers: Vec<Value>, skills: Vec<Value>) -> CatalogDocument {
        serde_json::from_value(json!({
            "schema_version": 1, "checks_version": 2, "environment": "testing",
            "revision": now().timestamp(), "issued_at": now(),
            "expires_at": now() + Duration::days(7),
            "publishers": publishers, "skills": skills,
        }))
        .unwrap()
    }

    fn ids(document: &CatalogDocument) -> Vec<&str> {
        ranked(document, now())
            .into_iter()
            .map(|p| p.id.as_str())
            .collect()
    }

    #[test]
    fn publishers_rank_by_capped_commits_then_stars_then_id() {
        let document = document(
            vec![
                publisher("quiet", "list", signals(50_000, 1)),
                publisher("busy", "marketplace", signals(40, 9)),
                // Past the cap, more commits buy nothing: stars decide.
                publisher("spam", "list", signals(12, 4_000)),
                publisher("known", "seed", signals(9_000, 20)),
                publisher("tie-b", "list", signals(40, 9)),
            ],
            ["quiet", "busy", "spam", "known", "tie-b"]
                .iter()
                .map(|id| skill(id, "one", 1))
                .collect(),
        );
        assert_eq!(ids(&document), ["known", "spam", "busy", "tie-b", "quiet"]);
    }

    #[test]
    fn unknown_weak_stale_suggested_and_empty_publishers_are_not_ranked() {
        let old = json!({"stars": 500, "commits_7d": 9, "commits_28d": 30,
                         "checked_at": now() - Duration::days(15)});
        let future = json!({"stars": 500, "commits_7d": 9, "commits_28d": 30,
                            "checked_at": now() + Duration::hours(1)});
        let edge = json!({"stars": 10, "commits_7d": 1, "commits_28d": 1,
                          "checked_at": now() - Duration::days(14)});
        let document = document(
            vec![
                publisher("fine", "list", signals(500, 3)),
                publisher("edge", "list", edge),
                publisher(
                    "no-stars",
                    "list",
                    json!({"commits_7d": 9, "commits_28d": 9, "checked_at": now()}),
                ),
                publisher(
                    "no-commits",
                    "list",
                    json!({"stars": 500, "checked_at": now()}),
                ),
                publisher("idle", "list", signals(500, 0)),
                publisher("few-stars", "list", signals(9, 9)),
                publisher("stale", "list", old),
                publisher("ahead", "list", future),
                publisher("reported", "suggested", signals(500, 9)),
                publisher(
                    "archived",
                    "list",
                    json!({"stars": 500, "commits_7d": 9, "commits_28d": 9,
                                                     "archived": true, "checked_at": now()}),
                ),
                publisher("no-skills", "list", signals(500, 9)),
            ],
            [
                "fine",
                "edge",
                "no-stars",
                "no-commits",
                "idle",
                "few-stars",
                "stale",
                "ahead",
                "reported",
                "archived",
            ]
            .iter()
            .map(|id| skill(id, "one", 1))
            .collect(),
        );
        assert_eq!(ids(&document), ["fine", "edge"]);
        // The same document a day later: the edge's signals are too old,
        // and the ones measured an hour ahead of this clock are now behind it.
        let later: Vec<_> = ranked(&document, now() + Duration::days(1))
            .into_iter()
            .map(|p| p.id.as_str())
            .collect();
        assert_eq!(later, ["ahead", "fine"]);
    }

    #[test]
    fn rows_carry_three_publishers_and_their_three_latest_emitted_skills() {
        let catalog = document(
            vec![
                publisher("a", "list", signals(100, 9)),
                publisher("b", "list", signals(100, 8)),
                publisher("c", "list", signals(100, 7)),
                publisher("d", "list", signals(100, 6)),
                publisher("e", "list", signals(100, 5)),
            ],
            vec![
                skill("a", "old", 9),
                skill("a", "newest", 1),
                skill("a", "newer", 2),
                skill("a", "new", 3),
                skill("a", "same-day-z", 3),
                skill("b", "cut", 1),
                skill("c", "kept", 5),
                skill("c", "cut", 1),
                skill("d", "one", 1),
                skill("e", "one", 1),
            ],
        );
        let all = rows(&catalog, now(), |skill| Some(skill.id.clone()));
        assert_eq!(
            all,
            vec![
                json!({"publisher_id": "a", "skill_ids": ["a:newest/SKILL.md", "a:newer/SKILL.md", "a:new/SKILL.md"]}),
                json!({"publisher_id": "b", "skill_ids": ["b:cut/SKILL.md"]}),
                json!({"publisher_id": "c", "skill_ids": ["c:cut/SKILL.md", "c:kept/SKILL.md"]}),
            ]
        );
        // The snapshot's size cap cut every row named "cut": a publisher
        // left with no skill gives its place to the next, and a skill id is
        // the id of the row that was sent, which for a seed is not the
        // document's.
        let bounded = rows(&catalog, now(), |skill| {
            (skill.name != "cut").then(|| format!("sent:{}", skill.name))
        });
        assert_eq!(
            bounded,
            vec![
                json!({"publisher_id": "a", "skill_ids": ["sent:newest", "sent:newer", "sent:new"]}),
                json!({"publisher_id": "c", "skill_ids": ["sent:kept"]}),
                json!({"publisher_id": "d", "skill_ids": ["sent:one"]}),
            ]
        );
        assert!(rows(&catalog, now(), |_| None).is_empty());
        assert!(
            rows(&document(vec![], vec![]), now(), |skill| Some(
                skill.id.clone()
            ))
            .is_empty()
        );
    }
}
