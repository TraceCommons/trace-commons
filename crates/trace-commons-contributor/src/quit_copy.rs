//! What quitting says, in one place, for every shell.
//!
//! Quitting asks first only when it would cut off something live: compute
//! work this app runs, or Private AI answering model calls through this app.
//! Anything else quits at once, without a prompt. When the prompt does
//! appear it gives the one reason in one short sentence and nothing more
//! (owner decision 2026-11-12, replacing the earlier "Quit is explicit about
//! consequences" prompt shown on every quit).
//!
//! Each shell reads its own two facts (whether a compute worker is still
//! running, and `private_inference_copy::quit_needs_notice`) and quits
//! directly when both are false. Only when one is true does it ask here for
//! the words. A shell that then cannot read them keeps its existing
//! refusal, so a failed read never turns into a silent quit while something
//! is running.

use serde::Serialize;

/// The prompt's heading.
pub const QUIT_TITLE: &str = concat!("Quit ", crate::app_name!(), "?");

/// Compute work is running and quitting stops it.
// Approved by the owner 2026-11-12.
pub const QUIT_STOPS_COMPUTE: &str = "Quitting stops compute on this computer.";

/// Private AI is answering model calls through this app and quitting stops it.
// Approved by the owner 2026-11-12.
pub const QUIT_STOPS_PRIVATE_AI: &str = "Quitting stops Private AI on this computer.";

/// Both of the above at once.
// Approved by the owner 2026-11-12.
pub const QUIT_STOPS_COMPUTE_AND_PRIVATE_AI: &str =
    "Quitting stops compute and Private AI on this computer.";

pub const QUIT_CONFIRM: &str = "Quit";
pub const QUIT_CANCEL: &str = "Cancel";

/// Every word of one quit prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuitPrompt {
    pub title: &'static str,
    pub body: &'static str,
    pub confirm: &'static str,
    pub cancel: &'static str,
}

/// The quit prompt, or `None` when quitting interrupts nothing and the shell
/// should quit without asking.
///
/// `stops_compute`: a compute worker is still running (the compute
/// snapshot's `worker_stopped` is false). `stops_private_ai`:
/// `private_inference_copy::quit_needs_notice` is true.
pub fn quit_prompt(stops_compute: bool, stops_private_ai: bool) -> Option<QuitPrompt> {
    let body = match (stops_compute, stops_private_ai) {
        (false, false) => return None,
        (true, false) => QUIT_STOPS_COMPUTE,
        (false, true) => QUIT_STOPS_PRIVATE_AI,
        (true, true) => QUIT_STOPS_COMPUTE_AND_PRIVATE_AI,
    };
    Some(QuitPrompt {
        title: QUIT_TITLE,
        body,
        confirm: QUIT_CONFIRM,
        cancel: QUIT_CANCEL,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_live_means_no_prompt() {
        assert_eq!(quit_prompt(false, false), None);
    }

    #[test]
    fn each_live_combination_gets_its_own_sentence() {
        for (compute, private_ai, body) in [
            (true, false, QUIT_STOPS_COMPUTE),
            (false, true, QUIT_STOPS_PRIVATE_AI),
            (true, true, QUIT_STOPS_COMPUTE_AND_PRIVATE_AI),
        ] {
            let prompt = quit_prompt(compute, private_ai).expect("something live asks");
            assert_eq!(prompt.body, body);
            assert_eq!(prompt.title, concat!("Quit ", crate::app_name!(), "?"));
            assert_eq!(prompt.confirm, "Quit");
            assert_eq!(prompt.cancel, "Cancel");
        }
    }

    /// The owner's rule: one short sentence saying why, and nothing more.
    #[test]
    fn every_body_is_one_short_sentence() {
        for body in [
            QUIT_STOPS_COMPUTE,
            QUIT_STOPS_PRIVATE_AI,
            QUIT_STOPS_COMPUTE_AND_PRIVATE_AI,
        ] {
            assert!(body.starts_with("Quitting stops "), "{body}");
            assert!(body.ends_with('.'), "{body}");
            assert_eq!(body.matches('.').count(), 1, "one sentence: {body}");
            assert!(!body.contains('\n'), "{body}");
            assert!(body.len() <= 60, "short: {body}");
        }
    }

    #[test]
    fn prompt_serializes_without_a_role() {
        let value = serde_json::to_value(quit_prompt(true, false)).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "title": QUIT_TITLE,
                "body": QUIT_STOPS_COMPUTE,
                "confirm": "Quit",
                "cancel": "Cancel",
            })
        );
    }
}
