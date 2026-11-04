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
use super::settlement_estimate::{SettlementCandidacy, SettlementShown, settlement_candidacy};
use super::state::SettlementNotified;
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
    /// The submissions whose newly final credit the folded sentence states,
    /// from the same snapshot as the sentence. Empty without a verdicts fold,
    /// and when the fold states no credit (its finals add to under 0.05).
    pub fold_covers: std::collections::BTreeSet<String>,
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
    /// The submissions behind the final credit `verdicts` states: empty when
    /// it states none. Taken from the same snapshot as `verdicts`.
    verdicts_final_ids: std::collections::BTreeSet<String>,
    /// The settlement estimate an announcement would carry, and whether its
    /// wording is the pricing one.
    settlement: Option<(SettlementShown, bool)>,
    /// The first drawable estimate, to be recorded as the baseline without
    /// a notification.
    settlement_seed: Option<SettlementNotified>,
    /// A pricing change too small to announce: the `pricing_version` to
    /// write onto the last announcement record.
    settlement_adopt_pricing: Option<Option<String>>,
    /// NEAR AI credit an announcement would carry: the amount since the last
    /// one, and the received total it brings the baseline to.
    credit: Option<super::nearai_credential::arrivals::PendingCredit>,
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
            Kind::InsightsTip
            | Kind::WeeklyRecap
            | Kind::SettlementEstimate
            | Kind::NearAiCredit => None,
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
            Kind::SettlementEstimate => self.settlement.as_ref().map(|(shown, pricing)| {
                nudge_render::settlement_notification(&shown.total, *pricing)
            }),
            Kind::NearAiCredit => self
                .credit
                .as_ref()
                .map(|credit| nudge_render::near_ai_credit_notification(credit.amount)),
            Kind::InsightsTip | Kind::WeeklyRecap => None,
        }
    }
}

/// What [`prepare`] read from disk on this thread: the config (for the
/// consent hold) and the mission join (the history cache, via
/// `read_gates`). The arbiter runs on every poll tick, so a test holds it
/// to reading only when something could be announced.
#[cfg(test)]
mod reads {
    use std::cell::Cell;
    thread_local! {
        pub(super) static CONFIG: Cell<u32> = const { Cell::new(0) };
        pub(super) static JOIN: Cell<u32> = const { Cell::new(0) };
    }
    pub(super) fn take() -> (u32, u32) {
        (CONFIG.with(|c| c.replace(0)), JOIN.with(|c| c.replace(0)))
    }
}

fn note_config_read() {
    #[cfg(test)]
    reads::CONFIG.with(|c| c.set(c.get() + 1));
}

fn note_join_read() {
    #[cfg(test)]
    reads::JOIN.with(|c| c.set(c.get() + 1));
}

fn prepare(shared: &DaemonShared, now: DateTime<Utc>) -> Prepared {
    // Each in its own short lock, none nested: the renderer count, then the
    // settings, then the state, then (inside the idle section) the policy
    // and the queue in the order every other reader takes them.
    let renderer = shared.has_renderer(ipc::EVENT_REENGAGE_DUE);
    let (settings, queue_ttl_days, history_poll_secs, estimate_on, suggestions_on) = {
        let s = shared.settings.lock().expect("settings lock");
        (
            s.notify_snapshot(renderer),
            s.queue_ttl_days,
            s.history_poll_secs,
            s.settlement_estimate_enabled,
            s.suggestions_enabled,
        )
    };
    // The estimate on status, read from its own leaf lock with nothing else
    // held. Absent when the switch is off or nothing is drawable, which also
    // keeps this tick from reading anything when nothing can be announced.
    let shown_estimate = if estimate_on {
        let st = shared.settlement.lock().expect("settlement lock");
        st.table
            .as_ref()
            .filter(|table| table.drawable_at(now))
            .and(st.shown.clone())
            .filter(|shown| shown.total.known > 0)
    } else {
        None
    };
    let (log, announced, verdicts_pending, last_history_poll_at, ledger, notified) = {
        let state = shared.state.lock().expect("state lock");
        (
            state.attention_log.clone(),
            state.idle_announced.clone(),
            state.verdicts_pending.clone(),
            state.last_history_poll_at,
            state.nudges.clone(),
            state.settlement_notified.clone(),
        )
    };
    // Only the kinds this tick could announce are gathered. With the master
    // switch off or both kinds off nothing can be, so the tick reads
    // nothing from disk: the arbiter runs on every poll tick.
    let wants_idle = settings.master && settings.idle_sessions;
    let wants_verdicts = settings.master && settings.verdicts_landed;
    let wants_settlement = settings.master
        && settings.settlement_estimate
        && suggestions_on
        && shown_estimate.is_some();
    // N6: read from memory, and only while it could be announced.
    let credit = (settings.master && settings.near_ai_credit)
        .then(|| super::nearai_credential::arrivals::pending_notification(shared))
        .flatten();
    let wants_credit = credit.is_some();
    let closed = !(wants_idle || wants_verdicts || wants_settlement || wants_credit)
        || shared.is_paused(now)
        || !shared.logged_in()
        || {
            note_config_read();
            crate::config::consent_hold(shared.store.load_config().ok().flatten().as_ref())
                .is_some()
        };
    let mut prepared = Prepared {
        candidates: Vec::new(),
        idle: None,
        verdicts: None,
        verdicts_final_ids: Default::default(),
        settlement: None,
        settlement_seed: None,
        settlement_adopt_pricing: None,
        credit: None,
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
    if let (Some(window), true, false) = (window, wants_idle, silenced) {
        // The fresh candidates first, under the policy and queue locks,
        // cloned out and released: the mission join below takes those
        // locks itself, and reads the history cache, so it is built only
        // once something is fresh.
        let fresh: Vec<super::queue::QueueEntry> = {
            let policy = shared.policy.lock().expect("policy lock");
            let queue = shared.queue.lock().expect("queue lock");
            super::queue::idle_candidates(&queue, &policy, now, window.idle_days)
                .into_iter()
                .filter(|e| !announced.contains(&e.entry_id))
                .cloned()
                .collect()
        };
        if let Some(since) = fresh
            .iter()
            .map(|e| e.last_modified_at.unwrap_or(e.discovered_at) + window.idle())
            .min()
        {
            note_join_read();
            let join = super::mission_matching::MissionJoin::read(
                shared,
                now,
                super::mission_matching::Probe::CacheOnly,
            );
            let mission_fit = join.as_ref().map(|j| {
                let policy = shared.policy.lock().expect("policy lock");
                fresh.iter().filter(|e| j.fit(&policy, e) > 0).count() as u64
            });
            let refs: Vec<&super::queue::QueueEntry> = fresh.iter().collect();
            prepared.idle = Some(IdleAnnouncement {
                ids: fresh.iter().map(|e| e.entry_id).collect(),
                batch: Batch {
                    count: fresh.len() as u64,
                    tools: ipc::batch_text_tools(&refs),
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
    if let Some(news) = verdicts_pending.filter(|d| wants_verdicts && fresh_poll && d.total() > 0) {
        let covered = prepared
            .log
            .iter()
            .any(|e| e.kind == Kind::VerdictsLanded && e.at >= news.newest_at);
        if !covered {
            if news.credit_final_tenths().is_some() {
                prepared.verdicts_final_ids = news
                    .submissions
                    .iter()
                    .filter(|(_, v)| v.final_credit)
                    .map(|(id, _)| id.clone())
                    .collect();
            }
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

    // N5: the settlement estimate, only while the person's own health gate
    // is open (the same source as the lead's `healthy`).
    if let (true, Some(shown)) = (wants_settlement, shown_estimate) {
        let healthy = shared
            .health
            .lock()
            .expect("health lock")
            .last_error_label
            .is_none();
        let candidacy = settlement_candidacy(&shown, notified.as_ref());
        // Bookkeeping, not an announcement: adopted whatever the health
        // gate, so a pricing change seen while it is shut is not taken for
        // one later.
        if candidacy == SettlementCandidacy::AdoptPricing {
            prepared.settlement_adopt_pricing = Some(shown.pricing_version.clone());
        } else if healthy {
            match candidacy {
                SettlementCandidacy::SeedBaseline => {
                    prepared.settlement_seed = Some(notified_from(&shown, now));
                }
                SettlementCandidacy::Candidate { pricing_changed } => {
                    prepared.candidates.push(Candidate {
                        kind: Kind::SettlementEstimate,
                        foldable: false,
                        resolves: Fact::EstimateAnnounced,
                        since: shown.estimated_at,
                        ttl: None,
                        caps: Kind::SettlementEstimate.default_caps(Duration::zero()),
                    });
                    prepared.settlement = Some((shown, pricing_changed));
                }
                SettlementCandidacy::AdoptPricing | SettlementCandidacy::Quiet => {}
            }
        }
    }

    // N6: NEAR AI credit received, past the 25%-and-$1 rule. Not held by the
    // settlement estimate's switches or health gate: it is about money that
    // already arrived, not an estimate.
    if let Some(credit) = credit {
        prepared.candidates.push(Candidate {
            kind: Kind::NearAiCredit,
            foldable: false,
            resolves: Fact::CreditAnnounced,
            since: now,
            ttl: None,
            caps: Kind::NearAiCredit.default_caps(Duration::zero()),
        });
        prepared.credit = Some(credit);
    }
    prepared
}

/// The figures an announcement (or the silent baseline) records.
fn notified_from(shown: &SettlementShown, now: DateTime<Utc>) -> SettlementNotified {
    SettlementNotified {
        low_usd_micros: shown.total.low_usd_micros,
        mid_usd_micros: shown.total.mid_usd_micros(),
        pricing_version: shown.pricing_version.clone(),
        at: now,
    }
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
        let publish = || {
            shared.publish(
                ipc::EVENT_REENGAGE_DUE,
                serde_json::json!({
                    "kind": kind.label(),
                    "title": text.title,
                    "body": text.body,
                    "actions": text.actions,
                }),
            );
        };
        if kind == Kind::NearAiCredit {
            let credit = prepared.credit.as_ref()?;
            if !super::nearai_credential::arrivals::publish_notification(shared, credit, publish) {
                return None;
            }
        } else {
            publish();
        }
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
        if *kind == Kind::SettlementEstimate {
            if let Some((shown, _)) = &prepared.settlement {
                state.settlement_notified = Some(notified_from(shown, now));
            }
        }
        tracing::info!(kind = kind.label(), route = ?route, "re-engagement announced");
        changed = true;
    }
    if let Some(seed) = &prepared.settlement_seed {
        if state.settlement_notified.is_none() {
            state.settlement_notified = Some(seed.clone());
            changed = true;
        }
    }
    if let Some(pricing_version) = &prepared.settlement_adopt_pricing {
        if let Some(notified) = state.settlement_notified.as_mut() {
            if notified.pricing_version != *pricing_version {
                notified.pricing_version = pricing_version.clone();
                changed = true;
            }
        }
    }
    if changed {
        let _ = state.save(&shared.store);
    }
    let fold_covers = match &fold {
        Some((Kind::VerdictsLanded, _)) => prepared.verdicts_final_ids.clone(),
        _ => Default::default(),
    };
    TickResult {
        digest_posts,
        fold,
        fold_covers,
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
        seed_idle_from(s, crate::source::SOURCE_CODEX, days)
    }

    fn seed_idle_from(s: &DaemonShared, source: &str, days: i64) -> uuid::Uuid {
        let entry_id = uuid::Uuid::new_v4();
        let at = noon() - Duration::days(days);
        s.queue
            .lock()
            .unwrap()
            .upsert(
                super::super::queue::QueueEntry {
                    entry_id,
                    session_hash: format!("sha256:{entry_id}"),
                    source: source.to_string(),
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

    /// The arbiter runs on every poll tick, so it reads nothing from disk
    /// when nothing could be announced: not the config with the master
    /// switch off or every kind off, and not the mission join (which loads
    /// the history cache) until some idle session is fresh.
    #[test]
    fn the_arbiter_reads_nothing_when_nothing_can_be_announced() {
        let s = live();
        digest_off(&s);
        let _r = renderer(&s);
        seed_idle(&s, 5);
        reads::take();

        s.settings.lock().unwrap().notifications_enabled = false;
        tick(&s, noon(), &Utc, digest(false, 1));
        assert_eq!(reads::take(), (0, 0), "master off");

        {
            let mut settings = s.settings.lock().unwrap();
            settings.notifications_enabled = true;
            settings.notify.idle_sessions = false;
            settings.notify.verdicts_landed = false;
        }
        tick(&s, noon(), &Utc, digest(false, 1));
        assert_eq!(reads::take(), (0, 0), "every kind off");

        // Only verdicts on: the consent hold is read, but idle sessions
        // are not gathered, so a fresh one builds no join.
        s.settings.lock().unwrap().notify.verdicts_landed = true;
        let got = tick(&s, noon(), &Utc, digest(false, 1));
        assert_eq!(got.standalone, None);
        assert_eq!(reads::take(), (1, 0), "idle off, no join");
        s.settings.lock().unwrap().notify.verdicts_landed = false;

        s.settings.lock().unwrap().notify.idle_sessions = true;
        let got = tick(&s, noon(), &Utc, digest(false, 1));
        assert_eq!(got.standalone, Some(Kind::IdleSessions));
        assert_eq!(reads::take(), (1, 1), "a fresh idle session reads both");

        // Its session is now announced: the window is open, nothing is
        // fresh, so the join is not built.
        let got = tick(&s, noon(), &Utc, digest(false, 1));
        assert_eq!(got.standalone, None);
        assert_eq!(reads::take(), (1, 0), "nothing fresh, no join");
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
            "1 trace from Codex has been idle for 3 days or more. Review it to send or keep."
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

    /// The digest's idle sentence carries the mission clause only while
    /// missions are shown: hidden (the default), a live catalogue the
    /// session fits adds no mission text, and the standalone idle
    /// notification never names one either way.
    #[test]
    fn the_idle_digest_names_a_mission_only_while_missions_are_shown() {
        fn texts(shown: bool, due: bool) -> String {
            let s = live();
            if !due {
                digest_off(&s);
            }
            s.settings.lock().unwrap().missions_enabled = shown;
            s.settings.lock().unwrap().claude_source =
                Some(super::super::settings::SourceDeclaration::Watch {
                    path: std::path::PathBuf::from("/tmp/reengage-claude-root"),
                });
            let id = seed_idle_from(&s, crate::source::SOURCE_CLAUDE_CODE, 5);
            let path = format!("/tmp/re-{id}.jsonl");
            s.state.lock().unwrap().cwd_cache.insert(
                path,
                super::super::state::CwdCacheEntry {
                    size_bytes: 1,
                    modified_at: noon(),
                    cwd: Some(ASK.to_string()),
                    project_key: Some(ASK.to_string()),
                    tool: Some(crate::source::SOURCE_CLAUDE_CODE.to_string()),
                    adapter: Some(crate::source::SOURCE_CLAUDE_CODE.to_string()),
                },
            );
            super::super::mission_matching::receive_catalogue(
                &s.mission_catalogue,
                &serde_json::json!({"schema_version": 1, "missions": [{"mission_id": "any"}]}),
                noon(),
            )
            .unwrap();
            let _r = renderer(&s);
            let mut rx = s.events.subscribe();
            let got = tick(&s, noon(), &Utc, digest(due, 1));
            if due {
                let (kind, text) = got.fold.expect("folded into the digest");
                assert_eq!(kind, Kind::IdleSessions);
                text
            } else {
                assert_eq!(got.standalone, Some(Kind::IdleSessions));
                let frames = reengage_frames(&mut rx);
                assert_eq!(frames.len(), 1, "{frames:?}");
                frames[0].to_string()
            }
        }
        let hidden = texts(false, true);
        assert!(!hidden.contains("mission"), "{hidden}");
        let hidden = texts(false, false);
        assert!(!hidden.contains("mission"), "{hidden}");
        let shown = texts(true, true);
        assert!(shown.contains("1 of them fits a mission."), "{shown}");
    }

    /// A batch holding a source the tool table cannot name (trajectory)
    /// names no tool in the N1 notification or the digest fold: never
    /// "from 0 tools", and never Codex credited with the trajectory
    /// session (Kristi's #1300 review, finding 5).
    #[test]
    fn an_unnamed_source_names_no_tool_in_the_notification_or_the_fold() {
        let s = live();
        digest_off(&s);
        seed_idle_from(&s, crate::source::SOURCE_CODEX, 5);
        seed_idle_from(&s, crate::source::SOURCE_TRAJECTORY, 6);
        let _r = renderer(&s);
        let mut rx = s.events.subscribe();
        let got = tick(&s, noon(), &Utc, digest(false, 2));
        assert_eq!(got.standalone, Some(Kind::IdleSessions));
        let frames = reengage_frames(&mut rx);
        assert_eq!(
            frames[0]["body"],
            "2 traces have been idle for 3 days or more. Review them to send or keep."
        );

        let s = live();
        seed_idle_from(&s, crate::source::SOURCE_TRAJECTORY, 5);
        let _r = renderer(&s);
        let got = tick(&s, noon(), &Utc, digest(true, 1));
        assert!(got.digest_posts);
        assert_eq!(
            got.fold,
            Some((
                Kind::IdleSessions,
                "1 of them has been idle for 3 days or more.".to_string()
            ))
        );
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
            submissions: Default::default(),
        });
        let mut rx = s.events.subscribe();
        let got = tick(&s, noon(), &Utc, digest(false, 0));
        assert_eq!(got.standalone, Some(Kind::VerdictsLanded));
        let frames = reengage_frames(&mut rx);
        assert_eq!(
            frames[0]["body"],
            "2 traces accepted and 1 held for privacy review. 2.5 credit is now final."
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
            submissions: Default::default(),
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

    // ---- N6: NEAR AI credit received ----

    const DOLLAR: i64 = 1_000_000_000;

    fn with_near_ai_session(s: &DaemonShared) {
        s.settings.lock().unwrap().near_ai_session = Some(crate::daemon::settings::NearAiSession {
            refresh_token: "rt_synthetic".into(),
            refresh_token_expires_at: None,
            stored_at: noon(),
            user_agent: "Mozilla/5.0 Test".into(),
        });
    }

    fn received(s: &DaemonShared, total: i64, notified: Option<i64>) {
        use super::super::nearai_credential::arrivals::{ArrivalsRecord, set_for_test};
        set_for_test(
            &s.store,
            ArrivalsRecord {
                organization_id: Some("org-1".into()),
                scanned: true,
                received_total_nanos: total,
                notified_received_total_nanos: notified,
                ..Default::default()
            },
        );
    }

    fn notified_total(s: &DaemonShared) -> Option<i64> {
        super::super::nearai_credential::arrivals::snapshot(&s.store).notified_received_total_nanos
    }

    fn credit_frames(
        rx: &mut tokio::sync::broadcast::Receiver<ipc::Event>,
    ) -> Vec<serde_json::Value> {
        reengage_frames(rx)
            .into_iter()
            .filter(|f| f["kind"] == "near_ai_credit")
            .collect()
    }

    /// The first credit posts; more credit waits for 25% and $1 of what was
    /// announced, then posts and moves the baseline.
    #[test]
    fn near_ai_credit_posts_the_first_arrival_then_only_past_25_percent_and_a_dollar() {
        let s = live();
        digest_off(&s);
        with_near_ai_session(&s);
        let _r = renderer(&s);
        let mut rx = s.events.subscribe();

        received(&s, 2 * DOLLAR, None);
        let got = tick(&s, noon(), &Utc, digest(false, 0));
        assert_eq!(got.standalone, Some(Kind::NearAiCredit));
        let frames = credit_frames(&mut rx);
        assert_eq!(frames.len(), 1, "{frames:?}");
        assert_eq!(
            frames[0]["title"],
            crate::nudge_copy::NOTIFY_NEAR_AI_CREDIT_TITLE
        );
        assert_eq!(
            frames[0]["body"],
            "$2.00 of NEAR AI credit arrived in your account."
        );
        assert_eq!(frames[0]["actions"][0]["id"], nudge_render::ACTION_REVIEW);
        assert_eq!(frames[0]["actions"][1]["id"], nudge_render::ACTION_NOT_NOW);
        assert_eq!(notified_total(&s), Some(2 * DOLLAR));

        // Days later, $0.40 more: 20%, under both bars. Nothing.
        let later = noon() + Duration::days(3);
        s.state.lock().unwrap().last_history_poll_at = Some(later);
        received(&s, 2 * DOLLAR + 400_000_000, Some(2 * DOLLAR));
        assert_eq!(tick(&s, later, &Utc, digest(false, 0)).standalone, None);
        assert!(credit_frames(&mut rx).is_empty());

        // $1 more in all: 50% and a dollar. Announced, and the baseline moves.
        let later = later + Duration::days(3);
        s.state.lock().unwrap().last_history_poll_at = Some(later);
        received(&s, 3 * DOLLAR, Some(2 * DOLLAR));
        assert_eq!(
            tick(&s, later, &Utc, digest(false, 0)).standalone,
            Some(Kind::NearAiCredit)
        );
        let frames = credit_frames(&mut rx);
        assert_eq!(
            frames[0]["body"],
            "$1.00 of NEAR AI credit arrived in your account."
        );
        assert_eq!(notified_total(&s), Some(3 * DOLLAR));
        // And nothing again at once.
        let later = later + Duration::days(3);
        s.state.lock().unwrap().last_history_poll_at = Some(later);
        assert_eq!(tick(&s, later, &Utc, digest(false, 0)).standalone, None);
    }

    /// The master switch and the kind's own switch each hold it, and the
    /// baseline does not move while held.
    #[test]
    fn muted_near_ai_credit_posts_nothing_and_keeps_its_baseline() {
        for mute in ["master", "kind"] {
            let s = live();
            digest_off(&s);
            with_near_ai_session(&s);
            let _r = renderer(&s);
            let mut rx = s.events.subscribe();
            match mute {
                "master" => s.settings.lock().unwrap().notifications_enabled = false,
                _ => s.settings.lock().unwrap().notify.near_ai_credit = false,
            }
            received(&s, 2 * DOLLAR, None);
            assert_ne!(
                tick(&s, noon(), &Utc, digest(false, 0)).standalone,
                Some(Kind::NearAiCredit),
                "{mute}"
            );
            assert!(credit_frames(&mut rx).is_empty(), "{mute}");
            assert_eq!(notified_total(&s), None, "{mute}");
        }
    }

    // ---- N5: the settlement estimate ----

    fn settlement_table() -> trace_commons_protocol::settlement_estimate::SettlementEstimateTable {
        trace_commons_protocol::settlement_estimate::SettlementEstimateTable::from_value(
            &serde_json::json!({
                "schema_version": 2,
                "features_version": "lef1",
                "version": "s1",
                "pricing_version": "p2",
                "instrument_id": "trace_credit",
                "atomic_units_per_credit": 1_000_000,
                "usd_micros_per_credit": 1_000_000,
                "published_at": "2026-10-07T00:00:00Z",
                "bytes_per_token": 4,
                "chunk_target_tokens": 2048,
                "chunk_cap": 16,
                "weights": [{"term": "ln_content_bytes", "weight": 1.0}],
                "cut_offs": [5.0],
                "tiers": [
                    {"mean_atomic_units": 1_000_000, "withheld_share_micros": 0},
                    {"mean_atomic_units": 1_010_000, "withheld_share_micros": 0}
                ],
                "aggregate_band": {"low_ratio_micros": 600_000, "high_ratio_micros": 1_400_000},
                "fit_rows": 40
            }),
        )
        .expect("synthetic table parses")
    }

    /// Show an estimate whose rounded band is `low` to `high` dollar micros.
    fn show_estimate(s: &DaemonShared, low: u64, high: u64, pricing: &str) {
        let mut st = s.settlement.lock().unwrap();
        st.table = Some(settlement_table());
        st.shown = Some(SettlementShown {
            total: trace_commons_protocol::settlement_estimate::SettlementTotal {
                low_usd_micros: low,
                high_usd_micros: high,
                known: 3,
                unknown: 0,
            },
            table_version: "s1".to_string(),
            pricing_version: Some(pricing.to_string()),
            estimated_at: noon(),
        });
    }

    fn settlement_frames(
        rx: &mut tokio::sync::broadcast::Receiver<ipc::Event>,
    ) -> Vec<serde_json::Value> {
        reengage_frames(rx)
            .into_iter()
            .filter(|f| f["kind"] == "settlement_estimate")
            .collect()
    }

    /// The first estimate seeds the baseline silently; a grown one then
    /// posts once with core's words and records the figures it announced.
    #[test]
    fn a_settlement_estimate_posts_once_with_core_words() {
        let s = live();
        digest_off(&s);
        let _r = renderer(&s);
        let mut rx = s.events.subscribe();
        show_estimate(&s, 4_000_000, 6_000_000, "p2");
        assert_eq!(tick(&s, noon(), &Utc, digest(false, 0)).standalone, None);
        assert!(settlement_frames(&mut rx).is_empty(), "the first is silent");
        assert_eq!(
            s.state
                .lock()
                .unwrap()
                .settlement_notified
                .as_ref()
                .map(|n| n.low_usd_micros),
            Some(4_000_000)
        );
        assert!(s.state.lock().unwrap().attention_log.is_empty());

        // Days later (past the minimum gap), the estimate grew.
        let later = noon() + Duration::days(4);
        s.state.lock().unwrap().last_history_poll_at = Some(later);
        show_estimate(&s, 6_000_000, 9_000_000, "p2");
        s.settlement
            .lock()
            .unwrap()
            .table
            .as_mut()
            .unwrap()
            .published_at = later;
        let got = tick(&s, later, &Utc, digest(false, 0));
        assert_eq!(got.standalone, Some(Kind::SettlementEstimate));
        let frames = settlement_frames(&mut rx);
        assert_eq!(frames.len(), 1, "{frames:?}");
        assert_eq!(
            frames[0]["title"],
            crate::nudge_copy::NOTIFY_SETTLEMENT_TITLE
        );
        assert_eq!(frames[0]["actions"][0]["id"], nudge_render::ACTION_REVIEW);
        let state = s.state.lock().unwrap();
        assert_eq!(state.attention_log.len(), 1);
        assert_eq!(state.attention_log[0].kind, Kind::SettlementEstimate);
        assert_eq!(state.attention_log[0].route, Route::Standalone);
        let notified = state.settlement_notified.as_ref().unwrap();
        assert_eq!(notified.low_usd_micros, 6_000_000);
        assert_eq!(notified.mid_usd_micros, 7_500_000);
    }

    /// A pricing change too small to announce is adopted silently, so a
    /// later drop under the same pricing is not posted as "Pricing changed".
    #[test]
    fn a_quiet_pricing_change_is_adopted_and_a_later_drop_stays_quiet() {
        let s = live();
        digest_off(&s);
        let _r = renderer(&s);
        let mut rx = s.events.subscribe();
        s.state.lock().unwrap().settlement_notified = Some(SettlementNotified {
            low_usd_micros: 8_000_000,
            mid_usd_micros: 10_000_000,
            pricing_version: Some("p1".to_string()),
            at: noon(),
        });
        let later = noon() + Duration::days(4);
        s.state.lock().unwrap().last_history_poll_at = Some(later);
        // New pricing, mid 10_500_000: under 25%, quiet.
        show_estimate(&s, 8_500_000, 12_500_000, "p2");
        s.settlement
            .lock()
            .unwrap()
            .table
            .as_mut()
            .unwrap()
            .published_at = later;
        assert_eq!(tick(&s, later, &Utc, digest(false, 0)).standalone, None);
        {
            let state = s.state.lock().unwrap();
            let notified = state.settlement_notified.as_ref().unwrap();
            assert_eq!(notified.pricing_version.as_deref(), Some("p2"));
            assert_eq!(notified.low_usd_micros, 8_000_000);
            assert_eq!(notified.mid_usd_micros, 10_000_000);
        }
        // Later, same pricing, mid fell to 7_000_000: no pricing change now.
        let after = later + Duration::days(4);
        s.state.lock().unwrap().last_history_poll_at = Some(after);
        show_estimate(&s, 5_000_000, 9_000_000, "p2");
        s.settlement
            .lock()
            .unwrap()
            .table
            .as_mut()
            .unwrap()
            .published_at = after;
        assert_eq!(tick(&s, after, &Utc, digest(false, 0)).standalone, None);
        assert!(settlement_frames(&mut rx).is_empty());
    }

    /// Every existing gate holds the settlement notification, one case each.
    #[test]
    fn each_gate_holds_the_settlement_notification() {
        type Hold = (&'static str, fn(&DaemonShared, DateTime<Utc>));
        let holds: [Hold; 11] = [
            ("paused", |s, _now| {
                s.paused.store(true, std::sync::atomic::Ordering::Relaxed);
                s.state.lock().unwrap().paused = true;
            }),
            ("consent hold", |s, _now| {
                let mut cfg = s.store.load_config().unwrap().unwrap();
                cfg.consent_scopes_chosen = Some(false);
                s.store.save_config(&cfg).unwrap();
            }),
            ("signed out", |s, _now| {
                std::fs::remove_file(s.store.device_key_path()).unwrap();
                assert!(!s.logged_in());
            }),
            ("unhealthy", |s, now| {
                s.health
                    .lock()
                    .unwrap()
                    .fail(crate::daemon::health::LABEL_QUEUE_FULL, now);
            }),
            ("suggestions off", |s, _now| {
                s.settings.lock().unwrap().suggestions_enabled = false;
            }),
            ("estimate switch off", |s, _now| {
                s.settings.lock().unwrap().settlement_estimate_enabled = false;
            }),
            ("kind switch off", |s, _now| {
                s.settings.lock().unwrap().notify.settlement_estimate = false;
            }),
            ("master off", |s, _now| {
                s.settings.lock().unwrap().notifications_enabled = false;
            }),
            ("quiet hours", |s, now| {
                // Checked below with a late clock.
                let _ = (s, now);
            }),
            ("daily cap", |s, now| {
                s.state.lock().unwrap().record_attention(
                    Kind::IdleSessions,
                    Route::Standalone,
                    now - Duration::hours(3),
                );
            }),
            ("min gap", |s, now| {
                s.state.lock().unwrap().record_attention(
                    Kind::IdleSessions,
                    Route::Standalone,
                    now - Duration::minutes(10),
                );
            }),
        ];
        for (name, hold) in holds {
            let s = live();
            digest_off(&s);
            let _r = renderer(&s);
            let mut rx = s.events.subscribe();
            let now = if name == "quiet hours" {
                DateTime::parse_from_rfc3339("2026-10-11T23:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc)
            } else {
                noon() + Duration::days(4)
            };
            s.state.lock().unwrap().last_history_poll_at = Some(now);
            show_estimate(&s, 6_000_000, 9_000_000, "p2");
            s.settlement
                .lock()
                .unwrap()
                .table
                .as_mut()
                .unwrap()
                .published_at = now;
            s.state.lock().unwrap().settlement_notified = Some(SettlementNotified {
                low_usd_micros: 4_000_000,
                mid_usd_micros: 6_000_000,
                pricing_version: Some("p2".to_string()),
                at: noon(),
            });
            hold(&s, now);
            let got = tick(&s, now, &Utc, digest(false, 0));
            assert_ne!(got.standalone, Some(Kind::SettlementEstimate), "{name}");
            assert!(settlement_frames(&mut rx).is_empty(), "{name}");
        }
    }

    /// "Not now" on the notification only dismisses: the daemon refuses
    /// kind `settlement_estimate` on `nudge_decline`, writes no cooldown, and
    /// the next grown estimate still posts after its kind interval.
    #[test]
    fn not_now_only_dismisses() {
        let s = live();
        digest_off(&s);
        let _r = renderer(&s);
        show_estimate(&s, 4_000_000, 6_000_000, "p2");
        tick(&s, noon(), &Utc, digest(false, 0));
        let later = noon() + Duration::days(4);
        s.state.lock().unwrap().last_history_poll_at = Some(later);
        show_estimate(&s, 6_000_000, 9_000_000, "p2");
        s.settlement
            .lock()
            .unwrap()
            .table
            .as_mut()
            .unwrap()
            .published_at = later;
        assert_eq!(
            tick(&s, later, &Utc, digest(false, 0)).standalone,
            Some(Kind::SettlementEstimate)
        );

        let before = s.state.lock().unwrap().nudges.clone();
        let response = ipc::handle_request(
            &s,
            &ipc::Request {
                id: 1,
                method: "nudge_decline".to_string(),
                params: serde_json::json!({"kind": "settlement_estimate"}),
            },
        );
        assert!(response.error.is_some(), "refused as an unknown kind");
        assert_eq!(
            s.state.lock().unwrap().nudges,
            before,
            "no cooldown written"
        );

        // Grown again after the 72 hour interval and a new week: posts.
        let much_later = later + Duration::days(8);
        s.state.lock().unwrap().last_history_poll_at = Some(much_later);
        show_estimate(&s, 12_000_000, 16_000_000, "p2");
        s.settlement
            .lock()
            .unwrap()
            .table
            .as_mut()
            .unwrap()
            .published_at = much_later;
        assert_eq!(
            tick(&s, much_later, &Utc, digest(false, 0)).standalone,
            Some(Kind::SettlementEstimate)
        );
    }
    // ---- the digest's credit sentences with a verdicts fold, end to end ----

    fn history_record(
        n: u8,
        label: &str,
        final_credit: Option<f32>,
    ) -> super::super::history::HistoryRecord {
        serde_json::from_value(serde_json::json!({
            "submission_id": uuid::Uuid::from_bytes([n; 16]),
            "submitted_at": noon() - Duration::hours(2),
            "project_id": "p",
            "project_label": label,
            "source": "claude-code",
            "session_hash": format!("sha256:{n}"),
            "status": "accepted",
            "consent_scopes": [],
            "credit_points_pending": 0.0,
            "credit_points_final": final_credit,
            "explanations": [],
            "last_refreshed_at": noon(),
        }))
        .unwrap()
    }

    fn folded_final(n: u8, credit: f32) -> (String, super::super::nudge::PendingVerdict) {
        (
            uuid::Uuid::from_bytes([n; 16]).to_string(),
            super::super::nudge::PendingVerdict {
                accepted: true,
                held: false,
                final_credit: true,
                credit_final: credit,
                since: noon() - Duration::hours(1),
                newest_at: noon() - Duration::hours(1),
            },
        )
    }

    /// Runs `expire_and_digest` with a verdicts fold over `news` and the
    /// history cache `records`, and returns the one published `digest_due`.
    fn digest_with_fold(
        records: &[super::super::history::HistoryRecord],
        news: Vec<(String, super::super::nudge::PendingVerdict)>,
    ) -> serde_json::Value {
        let s = std::sync::Arc::new(live());
        s.settings.lock().unwrap().local_notifications = false;
        super::super::history::HistoryCache::save(&s.store, records).unwrap();
        let submissions: std::collections::BTreeMap<_, _> = news.into_iter().collect();
        s.state.lock().unwrap().verdicts_pending =
            super::super::nudge::VerdictDelta::from_submissions(submissions);
        let mut rx = s.events.subscribe();
        super::super::expire_and_digest(&s, noon());
        let due: Vec<_> = drain(&mut rx)
            .into_iter()
            .filter(|e| e.event == ipc::EVENT_DIGEST_DUE)
            .collect();
        assert_eq!(due.len(), 1, "one digest posts");
        due[0].data.clone()
    }

    /// The traces the fold states are left out of `text`'s contribution
    /// sentence; the rest are not. `contribution` and `credit_final` carry
    /// everything, for shells that do not show the fold.
    #[test]
    fn the_text_leaves_out_only_the_credit_the_fold_states() {
        let data = digest_with_fold(
            &[
                history_record(1, "api", Some(2.0)),
                history_record(2, "api", Some(1.5)),
            ],
            vec![folded_final(1, 2.0)],
        );
        assert_eq!(
            data["contribution"],
            "2 traces contributed from api. 3.5 credit earned."
        );
        assert_eq!(data["credit_final"], 3.5);
        let text = data["text"].as_str().unwrap();
        assert!(
            text.contains("2 traces contributed from api. 1.5 credit earned."),
            "{text}"
        );
        assert_eq!(
            data["fold"]["text"],
            "1 trace accepted. 2.0 credit is now final."
        );
        assert!(text.ends_with("2.0 credit is now final."), "{text}");
    }

    /// A fold whose finals add to under 0.05 states no credit, so it owns
    /// none: the text must still state the earned credit.
    #[test]
    fn a_fold_that_states_no_credit_owns_none() {
        let data = digest_with_fold(
            &[
                history_record(1, "api", Some(0.04)),
                history_record(2, "api", Some(0.03)),
            ],
            vec![folded_final(1, 0.04)],
        );
        let text = data["text"].as_str().unwrap();
        assert!(text.contains("0.1 credit earned."), "{text}");
        assert!(!text.contains("credit is now final"), "{text}");
        assert_eq!(
            data["contribution"],
            "2 traces contributed from api. 0.1 credit earned."
        );
    }

    /// Each part rounds on its own: two 0.25 finals, one folded and one
    /// not, read "0.3 credit is now final" in the fold and "0.3 credit
    /// earned" in the text (0.6 together), while the shells that show no
    /// fold say "0.5 credit earned". Pinned so the difference is a known,
    /// tested one. The fold states a folded trace's figure from when it
    /// went final; a later revision of it is not stated again.
    #[test]
    fn each_part_rounds_on_its_own() {
        let data = digest_with_fold(
            &[
                history_record(1, "api", Some(0.25)),
                history_record(2, "api", Some(0.25)),
            ],
            vec![folded_final(1, 0.25)],
        );
        let text = data["text"].as_str().unwrap();
        assert!(text.contains("0.3 credit earned."), "{text}");
        assert!(
            data["fold"]["text"]
                .as_str()
                .unwrap()
                .contains("0.3 credit is now final")
        );
        assert_eq!(
            data["contribution"],
            "2 traces contributed from api. 0.5 credit earned."
        );
    }

    /// The set of traces a fold covers comes from the same snapshot as its
    /// sentence, so it is on the tick's result.
    #[test]
    fn the_tick_reports_the_traces_its_fold_covers() {
        let s = live();
        let _r = renderer(&s);
        let landed = noon() - Duration::hours(1);
        let (id, pending) = folded_final(7, 2.0);
        s.state.lock().unwrap().verdicts_pending =
            super::super::nudge::VerdictDelta::from_submissions([(id.clone(), pending)].into());
        let got = tick(&s, noon(), &Utc, digest(true, 0));
        assert_eq!(got.fold.as_ref().map(|f| f.0), Some(Kind::VerdictsLanded));
        assert_eq!(got.fold_covers, [id].into());
        let _ = landed;
        // No credit stated, none covered.
        let s = live();
        let (id, pending) = folded_final(8, 0.04);
        s.state.lock().unwrap().verdicts_pending =
            super::super::nudge::VerdictDelta::from_submissions([(id, pending)].into());
        let got = tick(&s, noon(), &Utc, digest(true, 0));
        assert!(got.fold_covers.is_empty());
    }
}
