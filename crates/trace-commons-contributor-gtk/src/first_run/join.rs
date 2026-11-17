//! Join's decisions, as the Mac makes them (`JoinScreenLayout`): near.ai is
//! the one way in (owner, 2026-10-12), an invite card is drawn only while an
//! invite is held, and Start sharing waits for a NEAR AI account.
use super::{Account, State};
use trace_commons_contributor::first_run_copy::JoinCopy;

/// A NEAR AI account: near.ai chosen, or the enrollment near.ai made for
/// this run, not signed out of. An invite alone, or an earlier run's
/// enrollment whose account is not known here, is not one.
pub fn has_near_ai_account(state: &State) -> bool {
    match state.account {
        Account::NearAi => true,
        Account::None | Account::WatchOnly | Account::Enrolled => {
            state.near_ai_enrolled && !state.signed_out_of_enrollment
        }
    }
}

/// A NEAR AI account signed in.
pub fn can_forward(state: &State) -> bool {
    has_near_ai_account(state) && (state.account != Account::NearAi || state.signed_in)
}

/// The state Start commits: a held near.ai enrollment with no account
/// answered starts as that enrollment.
pub fn ready_to_start(state: &mut State) {
    if can_forward(state) && state.account != Account::NearAi {
        state.account = Account::Enrolled;
    }
}

/// Sign in with near.ai: a signed-in near.ai is not replaced.
pub fn can_toggle_near_ai(state: &State) -> bool {
    match state.account {
        Account::None | Account::WatchOnly | Account::Enrolled => true,
        Account::NearAi => !state.signed_in,
    }
}

/// Choose near.ai; the sign-in commit runs at once.
pub fn choose_near_ai(state: &mut State) {
    if can_toggle_near_ai(state) {
        state.account = Account::NearAi;
    }
}

/// The invite card is drawn only while an invite is held.
pub fn shows_invite_card(state: &State) -> bool {
    !state.invite.trim().is_empty() || state.enrolled_invite.is_some()
}

/// Start sharing: a NEAR AI account signed in, the required use known, an
/// enrollment to finish under, the sharing line for the path chosen
/// present (`UsesScreenLayout.canStart`: the fallback line says Starting is
/// disabled), and nothing committing.
pub fn can_start(
    state: &State,
    required_known: bool,
    committing: bool,
    sharing_available: bool,
) -> bool {
    if committing || !sharing_available || !can_forward(state) {
        return false;
    }
    let mut ready = state.clone();
    ready_to_start(&mut ready);
    ready.step = super::Step::Join;
    ready.can_continue(required_known)
}

/// Whether Join shows an account as signed into (`hasSignedInAccount`):
/// near.ai answered on this run, a known invite joined, or a held near.ai
/// enrollment, and nothing signed out of.
pub fn has_signed_in_account(state: &State) -> bool {
    if state.signed_out_of_enrollment {
        return false;
    }
    match state.account {
        Account::NearAi => true,
        Account::None | Account::WatchOnly | Account::Enrolled => {
            state
                .enrolled_invite
                .as_deref()
                .is_some_and(|invite| !invite.is_empty())
                || state.near_ai_enrolled
        }
    }
}

/// The invite's credit range in the core's words, or the core's `unknown`
/// when the daemon gave none or a unit this build cannot word
/// (`JoinScreenLayout.payRange`).
pub fn pay_range(lookup: Option<&serde_json::Value>, copy: &JoinCopy) -> String {
    let Some(range) = lookup
        .map(|v| &v["credit_range"])
        .filter(|r| r["unit"].as_str() == Some(POINTS_PER_ACCEPTED_TRACE))
    else {
        return copy.unknown.to_owned();
    };
    let (Some(min), Some(max)) = (range["min"].as_u64(), range["max"].as_u64()) else {
        return copy.unknown.to_owned();
    };
    if min == max {
        return copy.pay_range_points_one.replace("{min}", &min.to_string());
    }
    copy.pay_range_points
        .replace("{min}", &min.to_string())
        .replace("{max}", &max.to_string())
}

/// The one credit-range unit the daemon accepts (`CREDIT_RANGE_UNIT_POINTS`).
pub const POINTS_PER_ACCEPTED_TRACE: &str = "points_per_accepted_trace";

/// The referral code sent with the NEAR AI enrollment: the field trimmed,
/// none when empty (`FirstRunRunner.sentReferralCode`).
pub fn sent_referral_code(typed: &str) -> Option<String> {
    let trimmed = typed.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// The line under the referral field: the core's invalid-code line for
/// text that is not a code; nothing for an empty field
/// (`ReferralSurface.fieldProblem`).
pub fn referral_problem(typed: &str) -> Option<&'static str> {
    let trimmed = typed.trim();
    if trimmed.is_empty() {
        return None;
    }
    let shown = trace_commons_protocol::referral::normalize_referral_code(trimmed)
        .and_then(|code| trace_commons_protocol::referral::format_referral_code(&code));
    shown
        .is_none()
        .then(|| trace_commons_contributor::referral_copy::referral_copy().invalid_code)
}

/// Join stopped holding a signed-in NEAR AI account: the single-organization
/// check and its automatic Start may run again (`JoinScreen`'s
/// `onChange(of: canForward)`).
pub fn forward_lost(before: bool, now: bool) -> bool {
    before && !now
}

/// How the near.ai browser login stands (`NearAILoginPoll.verdict`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginPoll {
    SignedIn,
    Waiting,
    /// Ended by the person: not a failure, nothing said.
    Cancelled,
    Ended,
}

/// A `near_ai_credential_status` answer as a verdict.
pub fn login_verdict(status: &serde_json::Value) -> LoginPoll {
    if status["session_state"].as_str()
        == Some(trace_commons_contributor::daemon::nearai_credential::LABEL_CREDENTIAL_PRESENT)
    {
        return LoginPoll::SignedIn;
    }
    match status["attempt_status"].as_str() {
        None | Some("waiting_for_browser") => LoginPoll::Waiting,
        Some("cancelled") => LoginPoll::Cancelled,
        Some(_) => LoginPoll::Ended,
    }
}

/// How long the first run waits on the browser (`NearAILoginPoll.limit`).
pub const LOGIN_LIMIT_SECS: u64 = 600;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_earlier_enrollment_alone_cannot_start() {
        let state = State {
            account: Account::Enrolled,
            enrollment_held: true,
            ..Default::default()
        };
        assert!(!has_near_ai_account(&state));
        assert!(!can_forward(&state));
        assert!(!can_start(&state, true, false, true));
        assert!(
            can_toggle_near_ai(&state),
            "near.ai can be chosen beside it"
        );
    }

    #[test]
    fn near_ai_chosen_waits_for_its_sign_in() {
        let mut state = State::default();
        choose_near_ai(&mut state);
        assert_eq!(state.account, Account::NearAi);
        assert!(has_near_ai_account(&state));
        assert!(!can_forward(&state));
        state.signed_in = true;
        assert!(can_forward(&state));
        assert!(
            !can_toggle_near_ai(&state),
            "a signed-in near.ai is not replaced"
        );
        // Signed in, but no enrollment the daemon holds yet.
        assert!(!can_start(&state, true, false, true));
        state.near_ai_enrolled = true;
        assert!(can_start(&state, true, false, true));
        assert!(
            !can_start(&state, false, false, true),
            "required use unknown"
        );
        assert!(!can_start(&state, true, true, true), "committing");
    }

    #[test]
    fn a_held_near_ai_enrollment_starts_as_that_enrollment() {
        let mut state = State {
            near_ai_enrolled: true,
            signed_in: true,
            ..Default::default()
        };
        assert!(can_forward(&state));
        ready_to_start(&mut state);
        assert_eq!(state.account, Account::Enrolled);
        state.signed_out_of_enrollment = true;
        assert!(!has_near_ai_account(&state));
    }

    #[test]
    fn the_invite_card_shows_only_while_an_invite_is_held() {
        let mut state = State::default();
        assert!(!shows_invite_card(&state));
        state.invite = "  ".into();
        assert!(!shows_invite_card(&state));
        state.invite = "https://x/#code".into();
        assert!(shows_invite_card(&state));
        let enrolled = State {
            enrolled_invite: Some(String::new()),
            ..Default::default()
        };
        assert!(shows_invite_card(&enrolled));
    }

    #[test]
    fn start_waits_for_the_sharing_line() {
        let state = State {
            account: Account::NearAi,
            signed_in: true,
            near_ai_enrolled: true,
            ..Default::default()
        };
        assert!(can_start(&state, true, false, true));
        assert!(
            !can_start(&state, true, false, false),
            "the fallback line disables Start"
        );
    }

    #[test]
    fn a_signed_in_account_is_shown_until_signed_out() {
        let mut state = State::default();
        assert!(!has_signed_in_account(&state));
        state.enrolled_invite = Some(String::new());
        assert!(
            !has_signed_in_account(&state),
            "an enrollment with no invite to show"
        );
        state.enrolled_invite = Some("https://x/#c".into());
        assert!(has_signed_in_account(&state));
        state.signed_out_of_enrollment = true;
        assert!(!has_signed_in_account(&state));
        let near = State {
            account: Account::NearAi,
            ..Default::default()
        };
        assert!(has_signed_in_account(&near));
        let enrolled = State {
            near_ai_enrolled: true,
            ..Default::default()
        };
        assert!(has_signed_in_account(&enrolled));
    }

    #[test]
    fn the_pay_range_is_the_daemons_in_the_cores_words() {
        let copy = trace_commons_contributor::first_run_copy::first_run_copy().join;
        let range = |min: u64, max: u64, unit: &str| serde_json::json!({"valid": true, "credit_range": {"min": min, "max": max, "unit": unit}});
        assert_eq!(pay_range(None, &copy), copy.unknown);
        assert_eq!(
            pay_range(Some(&serde_json::json!({"valid": true})), &copy),
            copy.unknown
        );
        assert_eq!(
            pay_range(Some(&range(1, 5, "points")), &copy),
            copy.unknown,
            "a unit this build cannot word"
        );
        assert_eq!(
            pay_range(Some(&range(1, 5, POINTS_PER_ACCEPTED_TRACE)), &copy),
            copy.pay_range_points
                .replace("{min}", "1")
                .replace("{max}", "5")
        );
        assert_eq!(
            pay_range(Some(&range(3, 3, POINTS_PER_ACCEPTED_TRACE)), &copy),
            copy.pay_range_points_one.replace("{min}", "3")
        );
    }

    #[test]
    fn a_referral_code_is_sent_trimmed_and_checked_before_sending() {
        assert_eq!(sent_referral_code("  "), None);
        assert_eq!(
            sent_referral_code(" TC-ABCDE-12345 ").as_deref(),
            Some("TC-ABCDE-12345")
        );
        let words = trace_commons_contributor::referral_copy::referral_copy();
        assert_eq!(referral_problem(""), None);
        assert_eq!(referral_problem("not a code"), Some(words.invalid_code));
        let shown = trace_commons_protocol::referral::normalize_referral_code("abcde12345")
            .and_then(|c| trace_commons_protocol::referral::format_referral_code(&c));
        if let Some(code) = shown {
            assert_eq!(referral_problem(&code), None);
        }
    }

    #[test]
    fn losing_the_signed_in_account_resets_the_organization_check() {
        assert!(forward_lost(true, false));
        assert!(!forward_lost(false, false));
        assert!(!forward_lost(true, true));
        assert!(!forward_lost(false, true));
    }
}
