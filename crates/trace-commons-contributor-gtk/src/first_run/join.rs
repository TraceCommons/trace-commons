//! Join's decisions, as the Mac makes them (`JoinScreenLayout`): near.ai is
//! the one way in (owner, 2026-10-12), an invite card is drawn only while an
//! invite is held, and Start sharing waits for a NEAR AI account.
use super::{Account, State};

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
/// enrollment to finish under, and nothing committing.
pub fn can_start(state: &State, required_known: bool, committing: bool) -> bool {
    if committing || !can_forward(state) {
        return false;
    }
    let mut ready = state.clone();
    ready_to_start(&mut ready);
    ready.step = super::Step::Join;
    ready.can_continue(required_known)
}

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
        assert!(!can_start(&state, true, false));
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
        assert!(!can_start(&state, true, false));
        state.near_ai_enrolled = true;
        assert!(can_start(&state, true, false));
        assert!(!can_start(&state, false, false), "required use unknown");
        assert!(!can_start(&state, true, true), "committing");
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
}
