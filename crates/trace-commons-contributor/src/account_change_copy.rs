//! Words for changing the NEAR AI login an account uses (F10), shared by
//! every shell (`tc_account_change_copy_json`).
//!
//! Spec: `docs/superpowers/specs/2026-11-03-f10-change-account-design.md`,
//! section 2. Every string here was approved by the owner on 2026-11-03
//! ("go with your recs on F10 and build it"), except the mismatch body,
//! approved 2026-11-02 with #1836, and each is marked with its date. Both
//! notices the change adds were approved with them (spec 2.6): the mismatch
//! block's "Use a different account" button and the payouts line for a
//! wallet-origin account. Nothing here says "Trace Commons" (owner,
//! 2026-11-02).
//!
//! Templates carry `{provider}`, `{old_provider}` or `{new_provider}`: a
//! login's title ([`crate::private_inference_copy::credential_provider_title`]:
//! NEAR wallet, GitHub or Google), filled by [`render`]. A body whose old
//! login is not known uses [`ACCOUNT_CHANGE_BODY_OLD_UNKNOWN`], as the spec
//! says.

use serde::Serialize;

// ---- The mismatch block (spec 2.1, entry point 1) ----

/// Owner-approved 2026-11-02 (#1836). The account section after a NEAR AI
/// sign-in the account refused as another account.
pub const ACCOUNT_LOGIN_MISMATCH_BODY: &str =
    "This app is connected to a different NEAR AI account. Sign in with that account again.";
/// Owner-approved 2026-11-03. The mismatch block's sign-in button.
pub const ACCOUNT_LOGIN_MISMATCH_SIGN_IN: &str = "Sign in";
/// Owner-approved 2026-11-03 (a notice, spec 2.6 item 1). The mismatch
/// block's button that starts a change with the refused login held aside.
pub const ACCOUNT_LOGIN_MISMATCH_CHANGE: &str = "Use a different account";

// ---- The account section (spec 2.1, entry point 2) ----

/// Owner-approved 2026-11-03. Beside the connected line: starts a change.
pub const ACCOUNT_LOGIN_CHANGE: &str = "Use a different account";

// ---- Payouts, for a wallet-origin account (spec 2.1) ----

/// Owner-approved 2026-11-03 (a notice, spec 2.6 item 2). Where the payout
/// link status would be, on an account that joined with a NEAR wallet.
pub const PAYOUTS_NEEDS_LOGIN_BODY: &str =
    "Payouts need a NEAR AI login. Add one to this account to set up payouts.";
/// Owner-approved 2026-11-03. Beside it: starts a change.
pub const PAYOUTS_NEEDS_LOGIN_ADD: &str = "Add a NEAR AI login";

// ---- The change sheet (spec 2.2) ----

/// Owner-approved 2026-11-03. The sheet's title.
pub const ACCOUNT_CHANGE_TITLE: &str = "Use a different NEAR AI account";
/// Owner-approved 2026-11-03. Step 3, for a NEAR AI account.
pub const ACCOUNT_CHANGE_BODY: &str = "Your traces, credit and payout settings stay. From now on, sign in with {new_provider}. Your {old_provider} login will no longer connect to this app.";
/// Owner-approved 2026-11-03. Step 3, for a NEAR AI account whose old login
/// is not known (spec 2.2).
pub const ACCOUNT_CHANGE_BODY_OLD_UNKNOWN: &str = "Your traces, credit and payout settings stay. From now on, sign in with {new_provider}. Your old login will no longer connect to this app.";
/// Owner-approved 2026-11-03. Step 3, for an account that joined with a
/// NEAR wallet.
pub const ACCOUNT_CHANGE_BODY_WALLET: &str = "Your traces and credit stay. From now on, sign in with {new_provider}. Your NEAR wallet will no longer sign in to this app.";
/// Owner-approved 2026-11-03. Step 3, beside the body.
pub const ACCOUNT_CHANGE_PRIVATE_AI: &str =
    "Private AI will use your {new_provider} account. Keys made in the old account are removed.";
/// Owner-approved 2026-11-03. Step 3's button.
pub const ACCOUNT_CHANGE_CONFIRM: &str = "Switch";
/// Owner-approved 2026-11-03. Closes the sheet and changes nothing.
pub const ACCOUNT_CHANGE_CANCEL: &str = "Cancel";
/// Owner-approved 2026-11-03. Step 1's heading.
pub const ACCOUNT_CHANGE_STEP_CURRENT: &str = "Confirm it's you";
/// Owner-approved 2026-11-03. Step 1, for an account that joined with a
/// NEAR wallet.
pub const ACCOUNT_CHANGE_STEP_CURRENT_WALLET: &str = "Approve the request in your NEAR wallet.";
/// Owner-approved 2026-11-03. Step 2's heading (its button is
/// [`ACCOUNT_CHANGE_STEP_NEW_BUTTON`] since 2026-11-04).
pub const ACCOUNT_CHANGE_STEP_NEW: &str = "Sign in with the new account";
/// Step 1's and step 2's buttons, apart from their headings (copy-guide
/// rule 4: a title identifies the step, a button performs it). The
/// headings stay [`ACCOUNT_CHANGE_STEP_CURRENT`] and
/// [`ACCOUNT_CHANGE_STEP_NEW`]; each button's accessible name stays the
/// longer one below. DRAFT, NEEDS APPROVAL (2026-11-04).
pub const ACCOUNT_CHANGE_STEP_CURRENT_BUTTON: &str = "Confirm";
pub const ACCOUNT_CHANGE_STEP_NEW_BUTTON: &str = "Sign in";
/// Owner-approved 2026-11-03. Step 1's sign-in was another login.
pub const ACCOUNT_CHANGE_ERROR_CURRENT_LOGIN: &str =
    "Sign in with {old_provider} first to confirm it's you.";
/// Owner-approved 2026-11-03. The wallet did not confirm.
pub const ACCOUNT_CHANGE_ERROR_WALLET: &str = "Your NEAR wallet didn't confirm. Try again.";
/// Owner-approved 2026-11-03. The new login already holds an account.
pub const ACCOUNT_CHANGE_ERROR_IN_USE: &str =
    "That NEAR AI account is already in use. Choose a different account.";
/// Owner-approved 2026-11-03. Within the seven-day cooldown (decision 9).
pub const ACCOUNT_CHANGE_ERROR_TOO_SOON: &str =
    "This account changed its login recently. Try again later.";
/// Owner-approved 2026-11-03. Anything else.
pub const ACCOUNT_CHANGE_ERROR_UNAVAILABLE: &str = "Couldn't switch accounts. Try again.";
/// Under "Add a NEAR AI login" when it could not start: it adds a login,
/// so it does not say "switch accounts" (copy-guide rule 7). DRAFT, NEEDS
/// APPROVAL (2026-11-04).
pub const PAYOUTS_NEEDS_LOGIN_ADD_FAILED: &str = "Couldn't add a NEAR AI login. Try again.";
/// Owner-approved 2026-11-03. An ordinary sign-in with a login a change
/// retired, not in the sheet.
pub const ACCOUNT_CHANGE_ERROR_RETIRED: &str =
    "This app no longer uses that NEAR AI account. Sign in with {provider}.";

// Accessibility names: each step's primary button names its step (spec
// 2.2, copy-guide rule 5).

/// Owner-approved 2026-11-03. Step 1's button, for a NEAR AI account.
pub const ACCOUNT_CHANGE_STEP_CURRENT_ACCESSIBILITY_NAME: &str =
    "Confirm it's you with {old_provider}";
/// Owner-approved 2026-11-03. Step 2's button.
pub const ACCOUNT_CHANGE_STEP_NEW_ACCESSIBILITY_NAME: &str = "Sign in with the new account";
/// Owner-approved 2026-11-03. Step 3's button.
pub const ACCOUNT_CHANGE_CONFIRM_ACCESSIBILITY_NAME: &str = "Switch to {new_provider}";

/// Every change string, in one payload, for the shells.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct AccountChangeCopy {
    pub mismatch_body: &'static str,
    pub mismatch_sign_in: &'static str,
    pub mismatch_change: &'static str,
    pub change: &'static str,
    pub payouts_needs_login_body: &'static str,
    pub payouts_needs_login_add: &'static str,
    pub payouts_needs_login_add_failed: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    pub body_old_unknown: &'static str,
    pub body_wallet: &'static str,
    pub private_ai: &'static str,
    pub confirm: &'static str,
    pub cancel: &'static str,
    pub step_current: &'static str,
    pub step_current_wallet: &'static str,
    pub step_new: &'static str,
    pub step_current_button: &'static str,
    pub step_new_button: &'static str,
    pub error_current_login: &'static str,
    pub error_wallet: &'static str,
    pub error_in_use: &'static str,
    pub error_too_soon: &'static str,
    pub error_unavailable: &'static str,
    pub error_retired: &'static str,
    pub step_current_accessibility_name: &'static str,
    pub step_new_accessibility_name: &'static str,
    pub confirm_accessibility_name: &'static str,
    /// The login titles the templates take, by tag.
    pub provider_near: &'static str,
    pub provider_github: &'static str,
    pub provider_google: &'static str,
}

/// The change words.
#[must_use]
pub fn account_change_copy() -> AccountChangeCopy {
    use crate::private_inference_copy as login;
    AccountChangeCopy {
        mismatch_body: ACCOUNT_LOGIN_MISMATCH_BODY,
        mismatch_sign_in: ACCOUNT_LOGIN_MISMATCH_SIGN_IN,
        mismatch_change: ACCOUNT_LOGIN_MISMATCH_CHANGE,
        change: ACCOUNT_LOGIN_CHANGE,
        payouts_needs_login_body: PAYOUTS_NEEDS_LOGIN_BODY,
        payouts_needs_login_add: PAYOUTS_NEEDS_LOGIN_ADD,
        payouts_needs_login_add_failed: PAYOUTS_NEEDS_LOGIN_ADD_FAILED,
        title: ACCOUNT_CHANGE_TITLE,
        body: ACCOUNT_CHANGE_BODY,
        body_old_unknown: ACCOUNT_CHANGE_BODY_OLD_UNKNOWN,
        body_wallet: ACCOUNT_CHANGE_BODY_WALLET,
        private_ai: ACCOUNT_CHANGE_PRIVATE_AI,
        confirm: ACCOUNT_CHANGE_CONFIRM,
        cancel: ACCOUNT_CHANGE_CANCEL,
        step_current: ACCOUNT_CHANGE_STEP_CURRENT,
        step_current_wallet: ACCOUNT_CHANGE_STEP_CURRENT_WALLET,
        step_new: ACCOUNT_CHANGE_STEP_NEW,
        step_current_button: ACCOUNT_CHANGE_STEP_CURRENT_BUTTON,
        step_new_button: ACCOUNT_CHANGE_STEP_NEW_BUTTON,
        error_current_login: ACCOUNT_CHANGE_ERROR_CURRENT_LOGIN,
        error_wallet: ACCOUNT_CHANGE_ERROR_WALLET,
        error_in_use: ACCOUNT_CHANGE_ERROR_IN_USE,
        error_too_soon: ACCOUNT_CHANGE_ERROR_TOO_SOON,
        error_unavailable: ACCOUNT_CHANGE_ERROR_UNAVAILABLE,
        error_retired: ACCOUNT_CHANGE_ERROR_RETIRED,
        step_current_accessibility_name: ACCOUNT_CHANGE_STEP_CURRENT_ACCESSIBILITY_NAME,
        step_new_accessibility_name: ACCOUNT_CHANGE_STEP_NEW_ACCESSIBILITY_NAME,
        confirm_accessibility_name: ACCOUNT_CHANGE_CONFIRM_ACCESSIBILITY_NAME,
        provider_near: login::CREDENTIAL_PROVIDER_NEAR,
        provider_github: login::CREDENTIAL_PROVIDER_GITHUB,
        provider_google: login::CREDENTIAL_PROVIDER_GOOGLE,
    }
}

/// `template` with each of `{provider}`, `{old_provider}` and
/// `{new_provider}` replaced by the matching login's title. `None` when the
/// template names a login that is not given or not known: a shell then
/// shows the fallback the spec names, never a raw tag or a placeholder.
#[must_use]
pub fn render(
    template: &str,
    provider: Option<&str>,
    old_provider: Option<&str>,
    new_provider: Option<&str>,
) -> Option<String> {
    let title =
        |tag: Option<&str>| tag.and_then(crate::private_inference_copy::credential_provider_title);
    let mut text = template.to_owned();
    for (placeholder, tag) in [
        ("{provider}", provider),
        ("{old_provider}", old_provider),
        ("{new_provider}", new_provider),
    ] {
        if text.contains(placeholder) {
            text = text.replace(placeholder, title(tag)?);
        }
    }
    Some(text)
}

/// One answer of `account_login_change`, as a shell reads it: the
/// ceremony's step, whose login is being proved and which the account moves
/// to, and the step's error. Unknown fields are ignored.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct ChangeState {
    pub change_id: String,
    /// `current`, `new`, `confirm` or `done`.
    pub step: String,
    /// `wallet` or `near_ai_login`.
    pub current_kind: String,
    #[serde(default)]
    pub old_provider: Option<String>,
    #[serde(default)]
    pub new_provider: Option<String>,
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub browser_url: Option<String>,
}

impl ChangeState {
    pub fn is_wallet(&self) -> bool {
        self.current_kind == "wallet"
    }
}

/// What a step's primary button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageAction {
    /// Step 1: prove the current identity (a browser leg).
    ProveCurrent,
    /// Step 2: sign in with the new login (a browser leg).
    SignInNew,
    /// Step 3: the person confirms. Never the default button: it moves the
    /// account.
    Confirm,
    /// Done: close the sheet. There is no confirmation of its own (spec 2.2).
    Close,
}

/// What the change sheet shows for one state, every sentence the core's.
/// The macOS shell maps the same states in TCShellCore
/// (`AccountLoginChangeSurface`); this is the same mapping for the shells
/// that link this crate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChangePage {
    pub title: &'static str,
    pub heading: Option<&'static str>,
    pub lines: Vec<String>,
    pub primary: &'static str,
    pub primary_accessibility_name: String,
    pub action: PageAction,
    /// Step 1 of a wallet-origin account: the NEAR account name field.
    pub asks_wallet_account: bool,
    pub error: Option<String>,
    pub busy: bool,
    pub cancel: &'static str,
}

/// The sheet's page for `state`. `None` only when step 3's words cannot be
/// drawn (a new login nobody can name): the shell closes rather than show a
/// placeholder.
#[must_use]
pub fn change_page(state: &ChangeState) -> Option<ChangePage> {
    let old = state.old_provider.as_deref();
    let new = state.new_provider.as_deref();
    let error = change_error_line(state);
    let page = |heading, lines, primary, name: String, action, asks| ChangePage {
        title: ACCOUNT_CHANGE_TITLE,
        heading,
        lines,
        primary,
        primary_accessibility_name: name,
        action,
        asks_wallet_account: asks,
        error: error.clone(),
        busy: state.busy,
        cancel: ACCOUNT_CHANGE_CANCEL,
    };
    match state.step.as_str() {
        "current" => {
            let lines = if state.is_wallet() {
                vec![ACCOUNT_CHANGE_STEP_CURRENT_WALLET.to_owned()]
            } else {
                Vec::new()
            };
            let name = if state.is_wallet() {
                ACCOUNT_CHANGE_STEP_CURRENT.to_owned()
            } else {
                render(
                    ACCOUNT_CHANGE_STEP_CURRENT_ACCESSIBILITY_NAME,
                    None,
                    old,
                    None,
                )
                .unwrap_or_else(|| ACCOUNT_CHANGE_STEP_CURRENT.to_owned())
            };
            Some(page(
                Some(ACCOUNT_CHANGE_STEP_CURRENT),
                lines,
                ACCOUNT_CHANGE_STEP_CURRENT,
                name,
                PageAction::ProveCurrent,
                state.is_wallet(),
            ))
        }
        "new" => Some(page(
            Some(ACCOUNT_CHANGE_STEP_NEW),
            Vec::new(),
            ACCOUNT_CHANGE_STEP_NEW,
            ACCOUNT_CHANGE_STEP_NEW_ACCESSIBILITY_NAME.to_owned(),
            PageAction::SignInNew,
            false,
        )),
        "confirm" => {
            let body = if state.is_wallet() {
                render(ACCOUNT_CHANGE_BODY_WALLET, None, None, new)?
            } else {
                render(ACCOUNT_CHANGE_BODY, None, old, new)
                    .or_else(|| render(ACCOUNT_CHANGE_BODY_OLD_UNKNOWN, None, None, new))?
            };
            let private_ai = render(ACCOUNT_CHANGE_PRIVATE_AI, None, None, new)?;
            let name = render(ACCOUNT_CHANGE_CONFIRM_ACCESSIBILITY_NAME, None, None, new)?;
            Some(page(
                None,
                vec![body, private_ai],
                ACCOUNT_CHANGE_CONFIRM,
                name,
                PageAction::Confirm,
                false,
            ))
        }
        // Done, and a step this build does not know: close.
        _ => Some(ChangePage {
            title: ACCOUNT_CHANGE_TITLE,
            heading: None,
            lines: Vec::new(),
            primary: ACCOUNT_CHANGE_CANCEL,
            primary_accessibility_name: ACCOUNT_CHANGE_CANCEL.to_owned(),
            action: PageAction::Close,
            asks_wallet_account: false,
            error: None,
            busy: false,
            cancel: ACCOUNT_CHANGE_CANCEL,
        }),
    }
}

/// The daemon's step error as the approved sentence. `current_login` with an
/// old login nobody can name has no approved words naming it and says the
/// generic line.
fn change_error_line(state: &ChangeState) -> Option<String> {
    let error = state.error.as_deref()?;
    Some(match error {
        "current_login" => render(
            ACCOUNT_CHANGE_ERROR_CURRENT_LOGIN,
            None,
            state.old_provider.as_deref(),
            None,
        )
        .unwrap_or_else(|| ACCOUNT_CHANGE_ERROR_UNAVAILABLE.to_owned()),
        "wallet" => ACCOUNT_CHANGE_ERROR_WALLET.to_owned(),
        "in_use" => ACCOUNT_CHANGE_ERROR_IN_USE.to_owned(),
        "too_soon" => ACCOUNT_CHANGE_ERROR_TOO_SOON.to_owned(),
        _ => ACCOUNT_CHANGE_ERROR_UNAVAILABLE.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every string is marked with the date it was approved.
    #[test]
    fn every_string_is_marked_approved() {
        let source = include_str!("account_change_copy.rs");
        let lines = source.lines().collect::<Vec<_>>();
        let mut declared = 0;
        for (index, line) in lines.iter().enumerate() {
            if !line.starts_with("pub const ") {
                continue;
            }
            declared += 1;
            let doc = lines[..index]
                .iter()
                .rev()
                .take_while(|line| line.starts_with("///"))
                .any(|line| {
                    line.contains("Owner-approved 2026-11-03")
                        || line.contains("Owner-approved 2026-11-02")
                });
            assert!(doc, "not marked approved: {line}");
        }
        assert_eq!(declared, 25);
    }

    /// The spec's exact words (section 2), so a later edit is a decision.
    #[test]
    fn the_words_are_the_approved_ones() {
        assert_eq!(
            ACCOUNT_LOGIN_MISMATCH_BODY,
            "This app is connected to a different NEAR AI account. Sign in with that account again."
        );
        assert_eq!(ACCOUNT_LOGIN_MISMATCH_CHANGE, "Use a different account");
        assert_eq!(ACCOUNT_LOGIN_CHANGE, "Use a different account");
        assert_eq!(ACCOUNT_CHANGE_TITLE, "Use a different NEAR AI account");
        assert_eq!(ACCOUNT_CHANGE_CONFIRM, "Switch");
        assert_eq!(PAYOUTS_NEEDS_LOGIN_ADD, "Add a NEAR AI login");
    }

    #[test]
    fn templates_render_login_titles_and_never_a_tag() {
        assert_eq!(
            render(ACCOUNT_CHANGE_BODY, None, Some("near"), Some("github")).as_deref(),
            Some(
                "Your traces, credit and payout settings stay. From now on, sign in with GitHub. Your NEAR wallet login will no longer connect to this app."
            )
        );
        assert_eq!(
            render(
                ACCOUNT_CHANGE_CONFIRM_ACCESSIBILITY_NAME,
                None,
                None,
                Some("google")
            )
            .as_deref(),
            Some("Switch to Google")
        );
        // A login the template needs that is missing or unknown: no text.
        assert_eq!(
            render(ACCOUNT_CHANGE_BODY, None, None, Some("github")),
            None
        );
        assert_eq!(
            render(ACCOUNT_CHANGE_BODY, None, Some("myspace"), Some("github")),
            None
        );
        // The fallback body needs only the new login.
        assert!(render(ACCOUNT_CHANGE_BODY_OLD_UNKNOWN, None, None, Some("github")).is_some());
        // A template with no placeholder renders as is.
        assert_eq!(
            render(ACCOUNT_CHANGE_TITLE, None, None, None).as_deref(),
            Some(ACCOUNT_CHANGE_TITLE)
        );
    }

    /// Title and button are separate keys (copy-guide rule 4), even where
    /// the words match.
    #[test]
    fn title_and_buttons_are_separate_keys() {
        let copy = account_change_copy();
        assert_ne!(copy.title, copy.change);
        assert_eq!(copy.change, copy.mismatch_change);
        assert_ne!(copy.confirm, copy.title);
    }

    fn state(step: &str, kind: &str, old: Option<&str>, new: Option<&str>) -> ChangeState {
        ChangeState {
            change_id: "c1".into(),
            step: step.into(),
            current_kind: kind.into(),
            old_provider: old.map(str::to_owned),
            new_provider: new.map(str::to_owned),
            busy: false,
            error: None,
            browser_url: None,
        }
    }

    #[test]
    fn each_step_is_its_page_in_the_approved_words() {
        let current =
            change_page(&state("current", "near_ai_login", Some("github"), None)).unwrap();
        assert_eq!(current.action, PageAction::ProveCurrent);
        assert_eq!(current.heading, Some(ACCOUNT_CHANGE_STEP_CURRENT));
        assert_eq!(
            current.primary_accessibility_name,
            "Confirm it's you with GitHub"
        );
        assert!(!current.asks_wallet_account);

        let wallet = change_page(&state("current", "wallet", None, None)).unwrap();
        assert!(wallet.asks_wallet_account);
        assert_eq!(
            wallet.lines,
            vec![ACCOUNT_CHANGE_STEP_CURRENT_WALLET.to_owned()]
        );

        let new = change_page(&state("new", "near_ai_login", Some("github"), None)).unwrap();
        assert_eq!(new.action, PageAction::SignInNew);
        assert_eq!(new.primary, ACCOUNT_CHANGE_STEP_NEW);

        let confirm = change_page(&state(
            "confirm",
            "near_ai_login",
            Some("near"),
            Some("google"),
        ))
        .unwrap();
        assert_eq!(confirm.action, PageAction::Confirm);
        assert_eq!(confirm.primary_accessibility_name, "Switch to Google");
        assert_eq!(
            confirm.lines,
            vec![
                "Your traces, credit and payout settings stay. From now on, sign in with Google. Your NEAR wallet login will no longer connect to this app.".to_owned(),
                "Private AI will use your Google account. Keys made in the old account are removed.".to_owned(),
            ]
        );
        let unknown_old =
            change_page(&state("confirm", "near_ai_login", None, Some("google"))).unwrap();
        assert!(unknown_old.lines[0].contains("Your old login"));
        let wallet_confirm =
            change_page(&state("confirm", "wallet", None, Some("github"))).unwrap();
        assert!(wallet_confirm.lines[0].contains("Your NEAR wallet will no longer sign in"));
        assert!(change_page(&state("confirm", "near_ai_login", None, None)).is_none());

        let done = change_page(&state("done", "near_ai_login", None, None)).unwrap();
        assert_eq!(done.action, PageAction::Close);
    }

    #[test]
    fn each_error_is_its_approved_sentence() {
        let line = |error: &str, old: Option<&str>| {
            let mut s = state("current", "near_ai_login", old, None);
            s.error = Some(error.into());
            change_page(&s).unwrap().error.unwrap()
        };
        assert_eq!(
            line("current_login", Some("github")),
            "Sign in with GitHub first to confirm it's you."
        );
        assert_eq!(
            line("current_login", None),
            ACCOUNT_CHANGE_ERROR_UNAVAILABLE
        );
        assert_eq!(line("wallet", None), ACCOUNT_CHANGE_ERROR_WALLET);
        assert_eq!(line("in_use", None), ACCOUNT_CHANGE_ERROR_IN_USE);
        assert_eq!(line("too_soon", None), ACCOUNT_CHANGE_ERROR_TOO_SOON);
        assert_eq!(
            line("something_new", None),
            ACCOUNT_CHANGE_ERROR_UNAVAILABLE
        );
    }

    #[test]
    fn a_daemon_answer_decodes_as_a_state() {
        let state: ChangeState = serde_json::from_value(serde_json::json!({
            "change_id": "c9", "step": "confirm", "current_kind": "wallet",
            "old_provider": null, "new_provider": "github", "busy": false,
            "error": null, "browser_url": "http://127.0.0.1:1/x", "future": 1
        }))
        .unwrap();
        assert!(state.is_wallet());
        assert_eq!(state.browser_url.as_deref(), Some("http://127.0.0.1:1/x"));
    }
}
