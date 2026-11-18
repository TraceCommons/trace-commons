//! What the panel's and the rail's status words say, from the core's words.
//!
//! Ports of the Mac's `MenuPanelStatus.privateAI*` (Private AI's one word
//! and its dot) and `WatchingMenuButton` (Watching's state and its menu).
//! Every word is the core's: `private_inference_copy::menu_state` for the
//! listener, `MonitorScreensCopy` for the rest.

use trace_commons_contributor::preview_copy::monitor_screens_copy;
use trace_commons_contributor::private_inference_copy::{self, PrivateInferenceTone};

/// Private AI's state, as the panel shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivateAi {
    /// The switch was not read.
    Unknown,
    Off,
    /// The switch is on: the core's one word for the listener's state and
    /// the tone of its dot. Only a clear tone is drawn green.
    Listener {
        word: &'static str,
        tone: PrivateInferenceTone,
    },
}

/// Off while the switch is off; otherwise the core's word and tone for the
/// listener's reported state, so a switch left on over a dead listener
/// never reads On and an On that is not answering is not drawn green. A
/// switch the core did not report is unknown, never Off.
pub fn private_ai(on: Option<bool>, reported: Option<&str>) -> PrivateAi {
    let Some(on) = on else {
        return PrivateAi::Unknown;
    };
    if !on {
        return PrivateAi::Off;
    }
    let Some(label) = reported else {
        return PrivateAi::Unknown;
    };
    let (word, tone) = private_inference_copy::menu_state(label);
    if word.is_empty() {
        return PrivateAi::Unknown;
    }
    PrivateAi::Listener { word, tone }
}

/// The one word: the listener's, Off, or a dash for unknown (never Off).
pub fn private_ai_word(state: PrivateAi) -> &'static str {
    match state {
        PrivateAi::Listener { word, .. } => word,
        PrivateAi::Off => monitor_screens_copy().off,
        PrivateAi::Unknown => "\u{2014}",
    }
}

/// The dot's tone: the listener's, or neutral grey.
pub fn private_ai_tone(state: PrivateAi) -> PrivateInferenceTone {
    match state {
        PrivateAi::Listener { tone, .. } => tone,
        PrivateAi::Off | PrivateAi::Unknown => PrivateInferenceTone::Neutral,
    }
}

/// Watching's state (the stack-wide ScreenState rule): a core that has not
/// answered is unknown and never reads as On.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Watching {
    Unknown,
    On,
    Paused,
}

pub fn watching(paused: Option<bool>) -> Watching {
    match paused {
        None => Watching::Unknown,
        Some(true) => Watching::Paused,
        Some(false) => Watching::On,
    }
}

/// Watching's value, for its accessible description.
pub fn watching_value(state: Watching) -> &'static str {
    let words = monitor_screens_copy();
    match state {
        Watching::On => words.on,
        Watching::Paused => words.paused,
        Watching::Unknown => "\u{2014}",
    }
}

/// One choice in Watching's menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchChoice {
    PauseHour,
    PauseMorning,
    PauseUntilResumed,
    Resume,
}

/// Watching's menu: the three pause lengths while watching, Resume while
/// paused, nothing while the core has not answered.
pub fn watch_choices(state: Watching) -> Vec<(WatchChoice, &'static str)> {
    let shell = monitor_screens_copy().shell;
    match state {
        Watching::On => vec![
            (WatchChoice::PauseHour, shell.pause_hour),
            (WatchChoice::PauseMorning, shell.pause_morning),
            (WatchChoice::PauseUntilResumed, shell.pause_until_resumed),
        ],
        Watching::Paused => vec![(WatchChoice::Resume, shell.resume_watcher)],
        Watching::Unknown => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unread_switch_is_unknown_never_off() {
        assert_eq!(private_ai(None, Some("running")), PrivateAi::Unknown);
        assert_eq!(private_ai_word(PrivateAi::Unknown), "\u{2014}");
        assert_ne!(
            private_ai_word(PrivateAi::Unknown),
            monitor_screens_copy().off
        );
    }

    #[test]
    fn a_switch_that_is_off_reads_off_whatever_the_listener_said() {
        assert_eq!(private_ai(Some(false), Some("running")), PrivateAi::Off);
        assert_eq!(private_ai_word(PrivateAi::Off), monitor_screens_copy().off);
        assert_eq!(
            private_ai_tone(PrivateAi::Off),
            PrivateInferenceTone::Neutral
        );
    }

    #[test]
    fn a_switch_left_on_takes_the_listeners_word_and_tone() {
        let state = private_ai(Some(true), Some("running"));
        assert_eq!(
            state,
            PrivateAi::Listener {
                word: private_inference_copy::MENU_WORD_ON,
                tone: PrivateInferenceTone::Clear
            }
        );
        // A dead listener under an on switch never reads On.
        let crashed = private_ai(Some(true), Some("crashed"));
        assert_ne!(
            private_ai_word(crashed),
            private_inference_copy::MENU_WORD_ON
        );
        assert_eq!(private_ai_tone(crashed), PrivateInferenceTone::Refused);
        // An unreported listener is unknown.
        assert_eq!(private_ai(Some(true), None), PrivateAi::Unknown);
    }

    #[test]
    fn watching_offers_the_three_pauses_or_resume() {
        let shell = monitor_screens_copy().shell;
        let words = |state| -> Vec<&str> {
            watch_choices(state)
                .into_iter()
                .map(|(_, word)| word)
                .collect()
        };
        assert_eq!(
            words(watching(Some(false))),
            [
                shell.pause_hour,
                shell.pause_morning,
                shell.pause_until_resumed
            ]
        );
        assert_eq!(words(watching(Some(true))), [shell.resume_watcher]);
        assert!(words(watching(None)).is_empty());
    }

    #[test]
    fn watching_never_reads_on_before_the_core_answers() {
        assert_eq!(watching_value(watching(None)), "\u{2014}");
        assert_eq!(
            watching_value(watching(Some(false))),
            monitor_screens_copy().on
        );
        assert_eq!(
            watching_value(watching(Some(true))),
            monitor_screens_copy().paused
        );
    }
}
