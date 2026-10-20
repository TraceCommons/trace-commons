//! The event bus: telling the user what routing just did.
//!
//! IronWire has no UI channel into a coding agent. It cannot put a line in
//! Claude Code's transcript, and it should not try — the only writable channel
//! is the response stream itself, and injecting text there would put words in
//! the model's mouth and corrupt the conversation the agent stores. So the
//! answer to "how does the user find out" is a **side channel**, not an
//! in-band one: `ironwire watch` in a second terminal, the menu bar app, an
//! optional desktop notification, and the daemon's own output.
//!
//! One rule governs this module, and it is the same rule as the observation tee
//! (`docs/PROTOCOL.md` §2): **nothing here may stall or fail the forward
//! path.** Publishing is non-blocking and lossy by construction. A user who
//! misses an event sees a slightly less complete log; a user whose request
//! blocked on a notification sees a hung agent.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use ironwire_core::policy::Rung;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// How many events a slow subscriber may fall behind before it starts missing
/// them.
///
/// Small on purpose. These are for a human watching a terminal, and a
/// subscriber that is 256 events behind is not showing anyone anything useful —
/// dropping is more honest than buffering megabytes on their behalf.
const CAPACITY: usize = 256;

/// Something worth telling the user about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A spend cap the user set has been reached, and IronWire has stopped
    /// routing to that backend.
    ///
    /// Published once per backend per window. This is the moment the user most
    /// needs to know and the moment they are least likely to be looking at
    /// `status` — and a per-request event would be the thing overrunning a bus
    /// that drops on lag.
    CapReached {
        /// When.
        at: DateTime<Utc>,
        /// Which backend stopped being routed to.
        backend: String,
        /// Spent against the cap in this window.
        spent_usd: f64,
        /// The cap itself.
        cap_usd: f64,
    },
    /// A conversation moved to a different backend.
    Routed {
        /// When.
        at: DateTime<Utc>,
        /// Opaque conversation key, so a user can tell two sessions apart
        /// without us naming anything about either.
        conversation: String,
        /// Where it was, if it had been somewhere.
        from: Option<String>,
        /// Where it is now.
        to: String,
        /// How far down the ladder.
        rung: Rung,
        /// Whether this crossed an API family and therefore needs translation.
        translated: bool,
        /// Short human-readable reason.
        reason: String,
    },
    /// A backend's circuit opened or closed.
    Health {
        /// When.
        at: DateTime<Utc>,
        /// Which backend.
        backend: String,
        /// `open`, `half_open`, or `closed`.
        circuit: String,
    },
    /// A request could not be served at all.
    Failed {
        /// When.
        at: DateTime<Utc>,
        /// Opaque conversation key.
        conversation: String,
        /// What went wrong, already phrased for a person.
        detail: String,
    },
}

impl Event {
    /// Whether this is worth interrupting the user for.
    ///
    /// Deliberately narrow. Rungs 0–2 change nothing the user can observe, so
    /// announcing them would train people to ignore the channel — and then the
    /// one announcement that matters gets ignored too (`docs/DESIGN.md` §3).
    #[must_use]
    pub fn is_user_visible(&self) -> bool {
        match self {
            Self::Routed { rung, .. } => rung.is_user_visible(),
            Self::Failed { .. } => true,
            // The user set this cap and then stopped being able to see it
            // working. Reaching it is exactly the moment worth announcing.
            Self::CapReached { .. } => true,
            // Circuit changes are useful in a live view and noise as an alert:
            // the whole point of the breaker is that the user's request was
            // still served.
            Self::Health { .. } => false,
        }
    }

    /// When this happened.
    #[must_use]
    pub fn at(&self) -> DateTime<Utc> {
        match self {
            Self::Routed { at, .. }
            | Self::Health { at, .. }
            | Self::Failed { at, .. }
            | Self::CapReached { at, .. } => *at,
        }
    }
}

/// Publishes events to whoever is listening, and to nobody at no cost.
#[derive(Debug, Clone)]
pub struct EventBus {
    sender: Arc<broadcast::Sender<Event>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    /// A bus with no subscribers.
    #[must_use]
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(CAPACITY);
        Self {
            sender: Arc::new(sender),
        }
    }

    /// Publish. Never blocks, never fails, never returns an error to the
    /// caller — the datapath has nothing useful to do with one.
    pub fn publish(&self, event: Event) {
        // `send` errors only when there are no receivers, which is the normal
        // state: most users never run `ironwire watch`.
        let _ = self.sender.send(event);
    }

    /// Subscribe. A subscriber that falls behind misses events rather than
    /// slowing anyone down.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }

    /// How many subscribers are listening. Used to skip work nobody will see.
    #[must_use]
    pub fn subscribers(&self) -> usize {
        self.sender.receiver_count()
    }
}

/// Render an event as one line, for `ironwire watch` and the daemon's output.
#[must_use]
pub fn line(event: &Event) -> String {
    match event {
        Event::CapReached {
            at,
            backend,
            spent_usd,
            cap_usd,
        } => format!(
            "{}  spend cap reached on {backend}: ${spent_usd:.2} of ${cap_usd:.2} — \
             not routing there until tomorrow ([limits] in config.toml)",
            at.format("%H:%M:%S")
        ),
        Event::Routed {
            at,
            conversation,
            from,
            to,
            // The rung is in the payload for the menu bar app; the line says
            // "different model family" instead, because that is the part a
            // person can act on.
            rung: _,
            translated,
            reason,
        } => {
            let arrow = match from {
                Some(from) if from != to => format!("{from} → {to}"),
                _ => to.clone(),
            };
            let note = if *translated {
                // The one case the user genuinely needs to know about: a
                // different model family answering, with reasoning state
                // dropped and prompt cache cold.
                "  [different model family — translated]"
            } else {
                ""
            };
            format!(
                "{} {} {arrow}  ({reason}){note}",
                at.format("%H:%M:%S"),
                short(conversation),
            )
        }
        Event::Health {
            at,
            backend,
            circuit,
        } => format!(
            "{} {} {backend} circuit {circuit}",
            at.format("%H:%M:%S"),
            short("health"),
        ),
        Event::Failed {
            at,
            conversation,
            detail,
        } => format!(
            "{} {} failed: {detail}",
            at.format("%H:%M:%S"),
            short(conversation),
        ),
    }
}

/// A short, stable stand-in for a conversation key, so a user watching two
/// sessions can tell them apart without the log carrying anything about either.
fn short(conversation: &str) -> String {
    let trimmed: String = conversation.chars().take(6).collect();
    format!("[{trimmed:>6}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routed(rung: Rung, translated: bool) -> Event {
        Event::Routed {
            at: Utc::now(),
            conversation: "1234567890".to_string(),
            from: Some("claude-sub".to_string()),
            to: "nearai".to_string(),
            rung,
            translated,
            reason: "no same-family capacity available".to_string(),
        }
    }

    #[test]
    fn publishing_with_nobody_listening_is_fine() {
        // The normal case: most users never run `ironwire watch`, and the
        // datapath must not care.
        let bus = EventBus::new();
        assert_eq!(bus.subscribers(), 0);
        bus.publish(routed(Rung::Preferred, false));
    }

    #[tokio::test]
    async fn a_subscriber_receives_what_is_published() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        bus.publish(routed(Rung::CrossFamily, true));
        let event = rx.try_recv().expect("delivered");
        assert!(event.is_user_visible());
    }

    #[tokio::test]
    async fn a_slow_subscriber_misses_events_rather_than_blocking_the_datapath() {
        // The property that matters. A user who wandered off must never be
        // able to stall someone's coding session.
        let bus = EventBus::new();
        let _rx = bus.subscribe();
        for _ in 0..(CAPACITY * 4) {
            bus.publish(routed(Rung::Preferred, false));
        }
        // Publishing returned; that is the whole assertion.
    }

    #[test]
    fn only_a_family_change_is_worth_interrupting_someone_for() {
        // Announcing rungs 0-2 would train people to ignore the channel, and
        // then the announcement that matters gets ignored too.
        assert!(!routed(Rung::Preferred, false).is_user_visible());
        assert!(!routed(Rung::SmallerModel, false).is_user_visible());
        assert!(!routed(Rung::AlternateCredential, false).is_user_visible());
        assert!(routed(Rung::CrossFamily, true).is_user_visible());
    }

    #[test]
    fn a_failure_is_always_worth_surfacing() {
        let event = Event::Failed {
            at: Utc::now(),
            conversation: "abc".to_string(),
            detail: "every backend is rate limited".to_string(),
        };
        assert!(event.is_user_visible());
    }

    #[test]
    fn a_translated_route_says_so_in_the_line() {
        // This is the sentence the whole feature exists to produce.
        let rendered = line(&routed(Rung::CrossFamily, true));
        assert!(rendered.contains("claude-sub → nearai"));
        assert!(rendered.contains("different model family"));
    }

    #[test]
    fn a_same_backend_route_does_not_draw_a_pointless_arrow() {
        let event = Event::Routed {
            at: Utc::now(),
            conversation: "abc".to_string(),
            from: Some("claude-sub".to_string()),
            to: "claude-sub".to_string(),
            rung: Rung::Preferred,
            translated: false,
            reason: "sticky affinity".to_string(),
        };
        assert!(!line(&event).contains('→'));
    }

    #[test]
    fn the_line_never_carries_the_full_conversation_key() {
        // The key is derived from the user's system prompt. It is opaque, but
        // there is no reason to print all of it.
        let rendered = line(&routed(Rung::CrossFamily, true));
        assert!(!rendered.contains("1234567890"));
        assert!(rendered.contains("123456"));
    }
}
