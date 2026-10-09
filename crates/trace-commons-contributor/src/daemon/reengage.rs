//! The attention arbiter, wired into the digest tick.
//!
//! [`super::attention`] is pure and decides; this module gathers what it
//! decides about and carries the decision out. Once per digest tick,
//! [`tick`]:
//!
//! 1. builds the candidates from one snapshot: idle sessions (N1) not yet
//!    named by an announcement, and verdict news (N2) not yet announced;
//! 2. decides whether a digest that is due by its schedule may post -- the
//!    master switch and the digest switch on, and not inside the gap after
//!    a standalone notification;
//! 3. runs [`attention::arbitrate`];
//! 4. publishes a standalone item as [`ipc::EVENT_REENGAGE_DUE`], or hands
//!    the caller the sentence to fold into the digest, and records either in
//!    the attention log.
//!
//! Every word comes from [`crate::nudge_render`]. What it logs is kind and
//! reason labels only.

use chrono::{DateTime, Duration, TimeZone, Utc};

use super::attention::{
    self, AttentionEntry, Candidate, DigestPlan, Fact, Kind, NotifySettings, Outcome, Route,
};
use super::ipc::{self, DaemonShared};
use super::settings::DigestSchedule;
use crate::nudge_render::{self, Batch, NotificationText, Verdicts};

/// The digest's own decision inputs, before the arbiter has a say.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DigestInputs {
    /// `notify::digest_due_for_schedule`'s answer.
    pub due_by_schedule: bool,
    pub pending: usize,
    pub contributed: usize,
    pub last_digest_at: Option<DateTime<Utc>>,
    pub schedule: DigestSchedule,
    pub interval_secs: u64,
}

/// What one tick decided about the digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TickResult {
    /// The digest posts this tick.
    pub digest_posts: bool,
    /// The sentence folded into it, and the kind it announces.
    pub fold: Option<(Kind, String)>,
    /// The standalone notification published this tick, if any.
    pub standalone: Option<Kind>,
}

/// Idle sessions an announcement would name: only those no earlier
/// announcement named.
struct IdleAnnouncement {
    ids: Vec<uuid::Uuid>,
    batch: Batch,
}

/// Everything one tick reads, gathered under short locks.
struct Prepared {
    candidates: Vec<Candidate>,
    idle: Option<IdleAnnouncement>,
    verdicts: Option<Verdicts>,
    settings: NotifySettings,
    log: Vec<AttentionEntry>,
}

impl Prepared {
    fn fold_text(&self, kind: Kind) -> Option<String> {
        match kind {
            Kind::IdleSessions => self
                .idle
                .as_ref()
                .map(|i| nudge_render::digest_idle_sentence(&i.batch)),
            Kind::VerdictsLanded => self
                .verdicts
                .as_ref()
                .map(nudge_render::digest_verdict_sentence),
            Kind::InsightsTip | Kind::WeeklyRecap => None,
        }
    }

    fn notification(&self, kind: Kind) -> Option<NotificationText> {
        match kind {
            Kind::IdleSessions => self
                .idle
                .as_ref()
                .map(|i| nudge_render::idle_notification(&i.batch)),
            Kind::VerdictsLanded => self
                .verdicts
                .as_ref()
                .map(nudge_render::verdicts_notification),
            Kind::InsightsTip | Kind::WeeklyRecap => None,
        }
    }
}

fn prepare(shared: &DaemonShared, now: DateTime<Utc>) -> Prepared {
    // Each in its own short lock, none nested: the renderer count, then the
    // settings, then the state, then (inside the idle section) the policy
    // and the queue in the order every other reader takes them.
    let renderer = shared.has_renderer(ipc::EVENT_REENGAGE_DUE);
    let (settings, queue_ttl_days, history_poll_secs) = {
        let s = shared.settings.lock().expect("settings lock");
        (
            s.notify_snapshot(renderer),
            s.queue_ttl_days,
            s.history_poll_secs,
        )
    };
    let (log, announced, verdicts_pending, last_history_poll_at, ledger) = {
        let state = shared.state.lock().expect("state lock");
        (
            state.attention_log.clone(),
            state.idle_announced.clone(),
            state.verdicts_pending.clone(),
            state.last_history_poll_at,
            state.nudges.clone(),
        )
    };
    let closed = shared.is_paused(now)
        || !shared.logged_in()
        || crate::config::consent_hold(shared.store.load_config().ok().flatten().as_ref())
            .is_some();
    let mut prepared = Prepared {
        candidates: Vec::new(),
        idle: None,
        verdicts: None,
        settings,
        log,
    };
    if closed {
        return prepared;
    }

    // N1: idle candidates nobody has named yet, unless the shared in-app
    // "Not now" is in force -- a person who said not now to the card has
    // said it to the notification too.
    let window = super::nudge::idle_window(queue_ttl_days);
    let silenced = super::nudge::shared_cooldown_until(queue_ttl_days, &ledger, now).is_some();
    if let (Some(window), false) = (window, silenced) {
        let join = super::mission_matching::MissionJoin::read(
            shared,
            now,
            super::mission_matching::Probe::CacheOnly,
        );
        let policy = shared.policy.lock().expect("policy lock");
        let queue = shared.queue.lock().expect("queue lock");
        let fresh: Vec<&super::queue::QueueEntry> =
            super::queue::idle_candidates(&queue, &policy, now, window.idle_days)
                .into_iter()
                .filter(|e| !announced.contains(&e.entry_id))
                .collect();
        if let Some(since) = fresh
            .iter()
            .map(|e| e.last_modified_at.unwrap_or(e.discovered_at) + window.idle())
            .min()
        {
            let mission_fit = join
                .as_ref()
                .map(|j| fresh.iter().filter(|e| j.fit(&policy, e) > 0).count() as u64);
            prepared.idle = Some(IdleAnnouncement {
                ids: fresh.iter().map(|e| e.entry_id).collect(),
                batch: Batch {
                    count: fresh.len() as u64,
                    tools: ipc::batch_tools(&fresh),
                    idle_days: window.idle_days as u32,
                    mission_fit,
                    estimate: None,
                },
            });
            prepared.candidates.push(Candidate {
                kind: Kind::IdleSessions,
                foldable: true,
                resolves: Fact::QueueDecided,
                since,
                ttl: None,
                caps: Kind::IdleSessions.default_caps(window.repeat()),
            });
        }
    }

    // N2: verdict news from a fresh history poll that no announcement has
    // covered. An announcement at or after the news' newest verdict covered
    // all of it; news that grew since is a candidate again, under its kind
    // cap.
    let fresh_poll = super::nudge::history_is_fresh(last_history_poll_at, history_poll_secs, now);
    if let Some(news) = verdicts_pending.filter(|d| fresh_poll && d.total() > 0) {
        let covered = prepared
            .log
            .iter()
            .any(|e| e.kind == Kind::VerdictsLanded && e.at >= news.newest_at);
        if !covered {
            prepared.verdicts = Some(ipc::render_verdicts(
                news.newly_accepted,
                news.newly_held,
                news.credit_final_tenths(),
                news.since,
            ));
            prepared.candidates.push(Candidate {
                kind: Kind::VerdictsLanded,
                foldable: true,
                resolves: Fact::NewsAcknowledged,
                since: news.newest_at,
                ttl: Some(attention::N2_NEWS_TTL),
                caps: Kind::VerdictsLanded.default_caps(Duration::zero()),
            });
        }
    }
    prepared
}

/// When the digest is next due by its schedule, as the arbiter reads it.
fn next_due_at<Tz: TimeZone>(
    digest: &DigestInputs,
    now: DateTime<Utc>,
    tz: &Tz,
) -> Option<DateTime<Utc>> {
    Some(match digest.schedule {
        DigestSchedule::Interval => digest
            .last_digest_at
            .map(|l| l + Duration::seconds(digest.interval_secs as i64))
            .unwrap_or(now),
        DigestSchedule::Evening { hour } => {
            super::notify::next_evening_at(digest.last_digest_at, now, hour, tz)
        }
    })
}

/// Run the arbiter for this tick and carry out its decision. See the module
/// doc. The caller posts the digest when `digest_posts` is set, with the
/// fold sentence as its third sentence, and stamps `last_digest_at`.
pub(crate) fn tick<Tz: TimeZone>(
    shared: &DaemonShared,
    now: DateTime<Utc>,
    tz: &Tz,
    digest: DigestInputs,
) -> TickResult {
    let prepared = prepare(shared, now);
    let digest_posts = digest.due_by_schedule
        && prepared.settings.master
        && prepared.settings.digest
        && !attention::digest_held_by_gap(&prepared.log, now);
    let plan = DigestPlan {
        fires_this_tick: digest_posts,
        pending: digest.pending,
        contributed: digest.contributed,
        last_digest_at: digest.last_digest_at,
        next_due_at: next_due_at(&digest, now, tz),
        schedule: digest.schedule,
    };
    let outcome = attention::arbitrate(
        &prepared.candidates,
        &plan,
        &prepared.log,
        &prepared.settings,
        now,
        tz,
    );
    apply(shared, &prepared, &outcome, digest_posts, now)
}

fn apply(
    shared: &DaemonShared,
    prepared: &Prepared,
    outcome: &Outcome,
    digest_posts: bool,
    now: DateTime<Utc>,
) -> TickResult {
    for (kind, reason) in &outcome.deferred {
        tracing::debug!(
            kind = kind.label(),
            reason = reason.label(),
            "re-engagement held"
        );
    }
    let mut announced: Vec<(Kind, Route)> = Vec::new();
    let fold = outcome
        .fold
        .filter(|_| digest_posts)
        .and_then(|kind| prepared.fold_text(kind).map(|text| (kind, text)));
    if let Some((kind, _)) = &fold {
        announced.push((*kind, Route::Folded));
    }
    let standalone = outcome.standalone.and_then(|kind| {
        let text = prepared.notification(kind)?;
        shared.publish(
            ipc::EVENT_REENGAGE_DUE,
            serde_json::json!({
                "kind": kind.label(),
                "title": text.title,
                "body": text.body,
                "actions": text.actions,
            }),
        );
        announced.push((kind, Route::Standalone));
        Some(kind)
    });

    // What is still pending, for pruning the announced set: read under the
    // queue lock and released before the state lock.
    let pending: std::collections::BTreeSet<uuid::Uuid> = {
        let queue = shared.queue.lock().expect("queue lock");
        queue.pending().iter().map(|e| e.entry_id).collect()
    };
    let mut state = shared.state.lock().expect("state lock");
    let mut changed = super::nudge::prune_idle_announced(&mut state.idle_announced, &pending);
    for (kind, route) in &announced {
        state.record_attention(*kind, *route, now);
        if *kind == Kind::IdleSessions {
            if let Some(idle) = &prepared.idle {
                state.idle_announced.extend(idle.ids.iter().copied());
            }
        }
        tracing::info!(kind = kind.label(), route = ?route, "re-engagement announced");
        changed = true;
    }
    if changed {
        let _ = state.save(&shared.store);
    }
    TickResult {
        digest_posts,
        fold,
        standalone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noon() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-07T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    const ASK: &str = "/tmp/reengage-ask";

    fn live() -> DaemonShared {
        let (dir, store) = crate::config::tests_support::temp_store();
        std::mem::forget(dir);
        let s = DaemonShared::load(store).unwrap();
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.consent_scopes_chosen = Some(true);
        s.store.save_config(&cfg).unwrap();
        crate::identity::DeviceIdentity::load_or_generate(&s.store).unwrap();
        assert!(s.logged_in());
        s.state.lock().unwrap().last_history_poll_at = Some(noon());
        s
    }

    fn seed_idle(s: &DaemonShared, days: i64) -> uuid::Uuid {
        let entry_id = uuid::Uuid::new_v4();
        let at = noon() - Duration::days(days);
        s.queue
            .lock()
            .unwrap()
            .upsert(
                super::super::queue::QueueEntry {
                    entry_id,
                    session_hash: format!("sha256:{entry_id}"),
                    source: crate::source::SOURCE_CODEX.to_string(),
                    project_key: ASK.to_string(),
                    project_label: super::super::policy::project_label_for(ASK),
                    path: std::path::PathBuf::from(format!("/tmp/re-{entry_id}.jsonl")),
                    size_bytes: 1,
                    discovered_at: at,
                    last_modified_at: Some(at),
                    ..Default::default()
                },
                500,
            )
            .unwrap();
        entry_id
    }

    fn digest(due: bool, pending: usize) -> DigestInputs {
        DigestInputs {
            due_by_schedule: due,
            pending,
            contributed: 0,
            last_digest_at: None,
            schedule: DigestSchedule::Interval,
            interval_secs: 3 * 3600,
        }
    }

    fn digest_off(s: &DaemonShared) {
        s.settings.lock().unwrap().notify.digest = false;
    }

    fn renderer(s: &DaemonShared) -> ipc::RendererDeclaration {
        s.declare_renderer(&serde_json::json!(["reengage_due"]))
            .unwrap()
    }

    fn drain(rx: &mut tokio::sync::broadcast::Receiver<ipc::Event>) -> Vec<ipc::Event> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    fn reengage_frames(
        rx: &mut tokio::sync::broadcast::Receiver<ipc::Event>,
    ) -> Vec<serde_json::Value> {
        drain(rx)
            .into_iter()
            .filter(|e| e.event == ipc::EVENT_REENGAGE_DUE)
            .map(|e| e.data)
            .collect()
    }

    /// With nobody able to draw it, nothing posts and no budget is spent.
    #[test]
    fn nothing_posts_without_a_renderer() {
        let s = live();
        digest_off(&s);
        seed_idle(&s, 5);
        let mut rx = s.events.subscribe();
        let got = tick(&s, noon(), &Utc, digest(false, 1));
        assert_eq!(got.standalone, None);
        assert!(reengage_frames(&mut rx).is_empty());
        assert!(s.state.lock().unwrap().attention_log.is_empty());
    }

    /// N1 posts on its own when the digest is off and a renderer is
    /// declared, with core's words; the sessions it named are not named
    /// again.
    #[test]
    fn idle_sessions_post_once_with_core_words() {
        let s = live();
        digest_off(&s);
        let id = seed_idle(&s, 5);
        let _r = renderer(&s);
        let mut rx = s.events.subscribe();
        let got = tick(&s, noon(), &Utc, digest(false, 1));
        assert_eq!(got.standalone, Some(Kind::IdleSessions));
        let frames = reengage_frames(&mut rx);
        assert_eq!(frames.len(), 1, "{frames:?}");
        assert_eq!(frames[0]["kind"], "idle_sessions");
        assert_eq!(
            frames[0]["body"],
            "1 session from Codex has been idle for 3 days or more. Review it to send or keep."
        );
        assert_eq!(frames[0]["actions"][0]["id"], nudge_render::ACTION_REVIEW);
        {
            let state = s.state.lock().unwrap();
            assert!(state.idle_announced.contains(&id));
            assert_eq!(state.attention_log.len(), 1);
            assert_eq!(state.attention_log[0].route, Route::Standalone);
        }
        // Days later, past every cap: the same session is not named again.
        let later = noon() + Duration::days(8);
        s.state.lock().unwrap().last_history_poll_at = Some(later);
        let got = tick(&s, later, &Utc, digest(false, 1));
        assert_eq!(got.standalone, None);
        assert!(reengage_frames(&mut rx).is_empty());
        // A new idle session is.
        seed_idle(&s, 9);
        let got = tick(&s, later, &Utc, digest(false, 2));
        assert_eq!(got.standalone, Some(Kind::IdleSessions));
    }

    /// With the digest on and sessions pending, idle sessions fold into the
    /// digest as its third sentence rather than posting on their own.
    #[test]
    fn idle_sessions_fold_into_a_due_digest() {
        let s = live();
        seed_idle(&s, 5);
        seed_idle(&s, 6);
        let _r = renderer(&s);
        let mut rx = s.events.subscribe();
        let got = tick(&s, noon(), &Utc, digest(true, 2));
        assert!(got.digest_posts);
        assert_eq!(
            got.fold,
            Some((
                Kind::IdleSessions,
                "2 of them, from Codex, have been idle for 3 days or more.".to_string()
            ))
        );
        assert_eq!(got.standalone, None);
        assert!(reengage_frames(&mut rx).is_empty());
        let state = s.state.lock().unwrap();
        assert_eq!(state.attention_log[0].route, Route::Folded);
        assert_eq!(state.idle_announced.len(), 2);
    }

    /// The master switch silences the digest too, and the digest switch
    /// silences the digest alone.
    #[test]
    fn the_switches_hold_the_digest() {
        let s = live();
        s.settings.lock().unwrap().notifications_enabled = false;
        assert!(!tick(&s, noon(), &Utc, digest(true, 1)).digest_posts);
        s.settings.lock().unwrap().notifications_enabled = true;
        assert!(tick(&s, noon(), &Utc, digest(true, 1)).digest_posts);
        digest_off(&s);
        assert!(!tick(&s, noon(), &Utc, digest(true, 1)).digest_posts);
    }

    /// Verdict news posts once; the same news is not announced again, and
    /// news that grew waits out its kind's interval.
    #[test]
    fn verdict_news_posts_once_and_waits_its_interval() {
        let s = live();
        let _r = renderer(&s);
        let landed = noon() - Duration::hours(1);
        s.state.lock().unwrap().verdicts_pending = Some(super::super::nudge::VerdictDelta {
            newly_accepted: 2,
            newly_held: 1,
            newly_final: 1,
            credit_final_delta: 2.5,
            since: landed,
            newest_at: landed,
        });
        let mut rx = s.events.subscribe();
        let got = tick(&s, noon(), &Utc, digest(false, 0));
        assert_eq!(got.standalone, Some(Kind::VerdictsLanded));
        let frames = reengage_frames(&mut rx);
        assert_eq!(
            frames[0]["body"],
            "2 sessions accepted and 1 held for privacy review. 2.5 credit is now final."
        );
        let two_hours = noon() + Duration::hours(2);
        assert_eq!(tick(&s, two_hours, &Utc, digest(false, 0)).standalone, None);
        // More news a little later: covered by nothing, held by N2's 24h.
        s.state
            .lock()
            .unwrap()
            .verdicts_pending
            .as_mut()
            .unwrap()
            .newest_at = two_hours;
        let three_hours = noon() + Duration::hours(3);
        assert_eq!(
            tick(&s, three_hours, &Utc, digest(false, 0)).standalone,
            None
        );
        let next_day = noon() + Duration::hours(26);
        s.state.lock().unwrap().last_history_poll_at = Some(next_day);
        assert_eq!(
            tick(&s, next_day, &Utc, digest(false, 0)).standalone,
            Some(Kind::VerdictsLanded)
        );
    }

    /// The same news, unchanged, is never announced twice, even once its
    /// kind's interval has passed: only news that grew is news again.
    #[test]
    fn unchanged_news_is_not_announced_again_after_its_interval() {
        let s = live();
        let _r = renderer(&s);
        let landed = noon() - Duration::hours(1);
        s.state.lock().unwrap().verdicts_pending = Some(super::super::nudge::VerdictDelta {
            newly_accepted: 1,
            newly_held: 0,
            newly_final: 0,
            credit_final_delta: 0.0,
            since: landed,
            newest_at: landed,
        });
        assert_eq!(
            tick(&s, noon(), &Utc, digest(false, 0)).standalone,
            Some(Kind::VerdictsLanded)
        );
        let next_day = noon() + Duration::hours(26);
        s.state.lock().unwrap().last_history_poll_at = Some(next_day);
        assert_eq!(tick(&s, next_day, &Utc, digest(false, 0)).standalone, None);
    }

    /// Paused, nothing is a candidate; the digest's own decision is
    /// unchanged.
    #[test]
    fn a_paused_daemon_announces_nothing() {
        let s = live();
        digest_off(&s);
        seed_idle(&s, 5);
        let _r = renderer(&s);
        s.paused.store(true, std::sync::atomic::Ordering::Relaxed);
        s.state.lock().unwrap().paused = true;
        let got = tick(&s, noon(), &Utc, digest(false, 1));
        assert_eq!(got.standalone, None);
        assert!(s.state.lock().unwrap().attention_log.is_empty());
    }

    /// A "Not now" on the card holds the notification too.
    #[test]
    fn a_not_now_holds_the_idle_notification() {
        let s = live();
        digest_off(&s);
        seed_idle(&s, 5);
        let _r = renderer(&s);
        s.state
            .lock()
            .unwrap()
            .nudges
            .entry(super::super::nudge::ledger_key(
                super::super::nudge::NudgeKind::IdleSessions,
                None,
            ))
            .or_default()
            .declined_at = Some(noon() - Duration::hours(1));
        assert_eq!(tick(&s, noon(), &Utc, digest(false, 1)).standalone, None);
    }

    /// Quiet hours hold a standalone notification.
    #[test]
    fn quiet_hours_hold_a_standalone() {
        let s = live();
        digest_off(&s);
        seed_idle(&s, 5);
        let _r = renderer(&s);
        let late = DateTime::parse_from_rfc3339("2026-10-07T23:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        s.state.lock().unwrap().last_history_poll_at = Some(late);
        assert_eq!(tick(&s, late, &Utc, digest(false, 1)).standalone, None);
    }
}
