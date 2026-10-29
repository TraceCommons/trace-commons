//! The words for referral codes
//! (docs/superpowers/specs/2026-10-29-referral-rewards-design.md, sections 5
//! and 6): the Settings "Referrals" section, the optional code field on the
//! NEAR AI sign-in step, and the own-traces vs referrals split on the
//! Insights NEAR AI credit card.
//!
//! Every sentence here is DRAFT, NEEDS APPROVAL (owner decision R6: all app
//! copy waits for the owner). None is a notice: there is no banner, card,
//! notification or one-time message among them (CLAUDE.md, "No new notices
//! without explicit approval"). Each is a heading, a field label, a button,
//! the state of something on screen, or the line that answers the person's
//! own action.
//!
//! Templates carry only the holes in [`REFERRAL_COPY_PLACEHOLDERS`]: `{code}`
//! (a code as shown, `TC-XXXXX-XXXXX`), `{n}` (a count), `{date}` (a date
//! the shell formats) and `{amount}` (money the shell formats from
//! nano-USD). A shell fills them with [`render`] and adds nothing else.

use serde::Serialize;

/// Every `{name}` hole the referral templates use.
pub const REFERRAL_COPY_PLACEHOLDERS: &[&str] = &["code", "n", "date", "amount"];

/// DRAFT, NEEDS APPROVAL. The Settings section's heading and its name in
/// the section list (rule 6: a short noun phrase, the same in both places).
pub const REFERRAL_TITLE: &str = "Referrals";
/// DRAFT, NEEDS APPROVAL. What a referral earns, beside the codes: the
/// consequence the person needs before sharing one (rule 3).
pub const REFERRAL_BODY: &str = "When someone joins with your code, you get 10% of the NEAR AI credit they earn in their first 3 months. Their own credit is not reduced.";
/// DRAFT, NEEDS APPROVAL. Makes a new code.
pub const REFERRAL_CREATE_BUTTON: &str = "Create code";
/// DRAFT, NEEDS APPROVAL. The create button's accessibility name.
pub const REFERRAL_CREATE_ACCESSIBILITY_NAME: &str = "Create code to share";
/// DRAFT, NEEDS APPROVAL. In place of the create button once the account
/// has the most live codes it may have. `{n}` is that most.
pub const REFERRAL_LIMIT_REACHED: &str = "You can have {n} codes on at once.";
/// DRAFT, NEEDS APPROVAL. The commons does not run referrals yet, or no
/// longer does (`referral_unavailable`), answering a create.
pub const REFERRAL_UNAVAILABLE: &str = "Referral codes aren't open yet.";
/// DRAFT, NEEDS APPROVAL. Copies a code.
pub const REFERRAL_COPY_BUTTON: &str = "Copy";
/// DRAFT, NEEDS APPROVAL. The copy button's accessibility name.
pub const REFERRAL_COPY_ACCESSIBILITY_NAME: &str = "Copy code {code}";
/// DRAFT, NEEDS APPROVAL. The copy button's label for a moment after it was
/// pressed, as the Devices section's "Copied" (`ACCOUNT_DEVICES_COPIED`).
pub const REFERRAL_COPIED: &str = "Copied";
/// DRAFT, NEEDS APPROVAL. Turns a code off.
pub const REFERRAL_DISABLE_BUTTON: &str = "Turn off";
/// DRAFT, NEEDS APPROVAL. The turn-off button's accessibility name.
pub const REFERRAL_DISABLE_ACCESSIBILITY_NAME: &str = "Turn off code {code}";
/// DRAFT, NEEDS APPROVAL. Beside the codes: what turning one off does and
/// does not do (rule 3), shown while a code can be turned off.
pub const REFERRAL_DISABLE_SCOPE: &str =
    "A code you turn off takes no new people. People who already joined with it still count.";
/// DRAFT, NEEDS APPROVAL. A code's state, when it is not live.
pub const REFERRAL_STATE_DISABLED: &str = "Off";
/// DRAFT, NEEDS APPROVAL. A code the abuse check paused.
pub const REFERRAL_STATE_SUSPENDED: &str = "Paused";
/// DRAFT, NEEDS APPROVAL. A code an operator closed.
pub const REFERRAL_STATE_VOIDED: &str = "Closed";
/// DRAFT, NEEDS APPROVAL. A code the commons can no longer show (its key
/// was rotated away); it still works for anyone who has it.
pub const REFERRAL_CODE_HIDDEN: &str = "Code not shown";
/// DRAFT, NEEDS APPROVAL. People who joined with one code: one.
pub const REFERRAL_JOINED_ONE: &str = "{n} person joined";
/// DRAFT, NEEDS APPROVAL. People who joined with one code: not one.
pub const REFERRAL_JOINED_MANY: &str = "{n} people joined";
/// DRAFT, NEEDS APPROVAL. The total over every code, as a label.
pub const REFERRAL_PEOPLE_JOINED: &str = "People joined";
/// DRAFT, NEEDS APPROVAL. The bonus earned from referrals, as a label.
pub const REFERRAL_CREDIT: &str = "Credit from referrals";
/// DRAFT, NEEDS APPROVAL. Beside that bonus: the part NEAR AI confirmed
/// sending.
pub const REFERRAL_CREDIT_SENT: &str = "{amount} sent";
/// DRAFT, NEEDS APPROVAL. The heading over the field for a code someone
/// shared, shown only while one may still be entered.
pub const REFERRAL_ENTER_TITLE: &str = "Enter a code";
/// DRAFT, NEEDS APPROVAL. The field's label, there and on the NEAR AI
/// sign-in step.
pub const REFERRAL_FIELD_LABEL: &str = "Referral code";
/// DRAFT, NEEDS APPROVAL. The field's placeholder: the shape of a code.
pub const REFERRAL_FIELD_PLACEHOLDER: &str = "TC-XXXXX-XXXXX";
/// DRAFT, NEEDS APPROVAL. Applies the code typed.
pub const REFERRAL_APPLY_BUTTON: &str = "Apply";
/// DRAFT, NEEDS APPROVAL. The apply button's accessibility name.
pub const REFERRAL_APPLY_ACCESSIBILITY_NAME: &str = "Apply referral code";
/// DRAFT, NEEDS APPROVAL. Beside the field: until when, and that it is
/// once (rule 3: immutability before the decision). `{date}` is
/// `apply_until`.
pub const REFERRAL_ENTER_SCOPE: &str =
    "You can enter one code until {date}. It can't be changed after.";
/// DRAFT, NEEDS APPROVAL. The account's state once a code is applied: a
/// status, which never says whose code.
pub const REFERRAL_APPLIED_STATUS: &str = "A referral code is applied to your account.";
/// DRAFT, NEEDS APPROVAL. The commons refused the code
/// (`referral_not_applied`, whatever the reason).
pub const REFERRAL_NOT_APPLIED: &str = "That code couldn't be applied.";
/// DRAFT, NEEDS APPROVAL. What was typed is not a code
/// (`referral-code-invalid`); checked before anything is sent.
pub const REFERRAL_INVALID_CODE: &str = "That isn't a referral code.";
/// DRAFT, NEEDS APPROVAL. The field on the NEAR AI sign-in step, which is
/// optional.
pub const REFERRAL_SIGN_UP_LABEL: &str = "Referral code (optional)";
/// DRAFT, NEEDS APPROVAL. Any other refusal of a create, turn off or apply.
pub const REFERRAL_FAILED: &str = "Nothing changed. Try again.";
/// DRAFT, NEEDS APPROVAL. In place of the section when this device holds
/// no usable account session, as the payout section says it
/// (`PAYOUT_LINK_SESSION_ENDED`).
pub const REFERRAL_SESSION_ENDED: &str = "Sign in again";
/// DRAFT, NEEDS APPROVAL. Insights' NEAR AI card: credit earned for the
/// person's own traces (the `earned` stream), as a row label.
pub const REFERRAL_INSIGHTS_OWN: &str = "From your traces";
/// DRAFT, NEEDS APPROVAL. Insights' NEAR AI card: credit earned from
/// referrals (the `referral` stream), as a row label.
pub const REFERRAL_INSIGHTS_REFERRALS: &str = "From referrals";
/// DRAFT, NEEDS APPROVAL. A stream's figure while a hold stops it being
/// sent. `{amount}` is what it owes.
pub const REFERRAL_INSIGHTS_ON_HOLD: &str = "{amount}, on hold";

/// Every referral string, in one payload, for the shells
/// (`tc_referral_copy_json`).
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct ReferralCopy {
    pub title: &'static str,
    pub body: &'static str,
    pub create_button: &'static str,
    pub create_accessibility_name: &'static str,
    pub limit_reached: &'static str,
    pub unavailable: &'static str,
    pub copy_button: &'static str,
    pub copy_accessibility_name: &'static str,
    pub copied: &'static str,
    pub disable_button: &'static str,
    pub disable_accessibility_name: &'static str,
    pub disable_scope: &'static str,
    pub state_disabled: &'static str,
    pub state_suspended: &'static str,
    pub state_voided: &'static str,
    pub code_hidden: &'static str,
    pub joined_one: &'static str,
    pub joined_many: &'static str,
    pub people_joined: &'static str,
    pub credit: &'static str,
    pub credit_sent: &'static str,
    pub enter_title: &'static str,
    pub field_label: &'static str,
    pub field_placeholder: &'static str,
    pub apply_button: &'static str,
    pub apply_accessibility_name: &'static str,
    pub enter_scope: &'static str,
    pub applied_status: &'static str,
    pub not_applied: &'static str,
    pub invalid_code: &'static str,
    pub sign_up_label: &'static str,
    pub failed: &'static str,
    pub session_ended: &'static str,
    pub insights_own: &'static str,
    pub insights_referrals: &'static str,
    pub insights_on_hold: &'static str,
}

/// The referral words.
#[must_use]
pub fn referral_copy() -> ReferralCopy {
    ReferralCopy {
        title: REFERRAL_TITLE,
        body: REFERRAL_BODY,
        create_button: REFERRAL_CREATE_BUTTON,
        create_accessibility_name: REFERRAL_CREATE_ACCESSIBILITY_NAME,
        limit_reached: REFERRAL_LIMIT_REACHED,
        unavailable: REFERRAL_UNAVAILABLE,
        copy_button: REFERRAL_COPY_BUTTON,
        copy_accessibility_name: REFERRAL_COPY_ACCESSIBILITY_NAME,
        copied: REFERRAL_COPIED,
        disable_button: REFERRAL_DISABLE_BUTTON,
        disable_accessibility_name: REFERRAL_DISABLE_ACCESSIBILITY_NAME,
        disable_scope: REFERRAL_DISABLE_SCOPE,
        state_disabled: REFERRAL_STATE_DISABLED,
        state_suspended: REFERRAL_STATE_SUSPENDED,
        state_voided: REFERRAL_STATE_VOIDED,
        code_hidden: REFERRAL_CODE_HIDDEN,
        joined_one: REFERRAL_JOINED_ONE,
        joined_many: REFERRAL_JOINED_MANY,
        people_joined: REFERRAL_PEOPLE_JOINED,
        credit: REFERRAL_CREDIT,
        credit_sent: REFERRAL_CREDIT_SENT,
        enter_title: REFERRAL_ENTER_TITLE,
        field_label: REFERRAL_FIELD_LABEL,
        field_placeholder: REFERRAL_FIELD_PLACEHOLDER,
        apply_button: REFERRAL_APPLY_BUTTON,
        apply_accessibility_name: REFERRAL_APPLY_ACCESSIBILITY_NAME,
        enter_scope: REFERRAL_ENTER_SCOPE,
        applied_status: REFERRAL_APPLIED_STATUS,
        not_applied: REFERRAL_NOT_APPLIED,
        invalid_code: REFERRAL_INVALID_CODE,
        sign_up_label: REFERRAL_SIGN_UP_LABEL,
        failed: REFERRAL_FAILED,
        session_ended: REFERRAL_SESSION_ENDED,
        insights_own: REFERRAL_INSIGHTS_OWN,
        insights_referrals: REFERRAL_INSIGHTS_REFERRALS,
        insights_on_hold: REFERRAL_INSIGHTS_ON_HOLD,
    }
}

/// `template` with each `{name}` in `values` filled. Holes not named stay
/// as they are.
#[must_use]
pub fn render(template: &str, values: &[(&str, &str)]) -> String {
    values
        .iter()
        .fold(template.to_owned(), |text, (name, value)| {
            text.replace(&format!("{{{name}}}"), value)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `REFERRAL_*` string constants as declared in this file, each with
    /// the doc comment above it.
    fn declared() -> Vec<(String, String)> {
        let source = include_str!("referral_copy.rs");
        let mut out = Vec::new();
        let mut docs = String::new();
        for line in source.lines() {
            let trimmed = line.trim_start();
            if let Some(doc) = trimmed.strip_prefix("///") {
                docs.push_str(doc);
                docs.push('\n');
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("pub const REFERRAL_")
                && let Some(name) = rest.split(':').next()
                && rest.contains(": &str")
            {
                out.push((format!("REFERRAL_{name}"), docs.clone()));
            }
            docs.clear();
        }
        out
    }

    fn every_string(copy: &ReferralCopy) -> Vec<String> {
        serde_json::to_value(copy)
            .unwrap()
            .as_object()
            .unwrap()
            .values()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn every_referral_string_is_marked_draft_and_in_the_payload() {
        let declared = declared();
        let copy = referral_copy();
        let fields = serde_json::to_value(copy).unwrap();
        assert_eq!(
            declared.len(),
            fields.as_object().unwrap().len(),
            "every constant is in the payload, and only they are: {declared:?}"
        );
        for (name, docs) in &declared {
            assert!(
                docs.contains("DRAFT, NEEDS APPROVAL"),
                "{name} is not marked for owner review"
            );
        }
    }

    #[test]
    fn every_string_follows_the_copy_guide() {
        let copy = referral_copy();
        for text in every_string(&copy) {
            assert!(!text.trim().is_empty());
            assert_eq!(text, text.trim(), "{text:?}");
            assert!(text.is_ascii(), "no emoji or typographic oddities: {text}");
            // Only the declared holes, and no currency of our own: money is
            // the shell's `{amount}`.
            for (index, _) in text.match_indices('{') {
                assert!(
                    REFERRAL_COPY_PLACEHOLDERS
                        .iter()
                        .any(|name| text[index..].starts_with(&format!("{{{name}}}"))),
                    "{text}"
                );
            }
            assert!(!text.contains('$'), "{text}");
            // No notice wording sneaks in.
            let lower = text.to_lowercase();
            for word in [
                "new!",
                "congratulations",
                "don't miss",
                "invite your friends",
            ] {
                assert!(!lower.contains(word), "{text}");
            }
        }
        // Rule 1: buttons name an action in one or two words.
        for button in [
            copy.create_button,
            copy.copy_button,
            copy.disable_button,
            copy.apply_button,
        ] {
            assert!(button.split_whitespace().count() <= 2, "{button}");
            assert!(!button.contains('{'), "{button}");
        }
        // Rule 4: titles and buttons are separate words.
        assert_ne!(copy.title, copy.create_button);
        assert_ne!(copy.enter_title, copy.apply_button);
        // Rule 5: an accessibility name includes the visible action, and the
        // repeated per-code buttons name their code.
        assert!(
            copy.create_accessibility_name
                .starts_with(copy.create_button)
        );
        assert!(copy.apply_accessibility_name.starts_with(copy.apply_button));
        assert!(copy.copy_accessibility_name.starts_with(copy.copy_button));
        assert!(copy.copy_accessibility_name.contains("{code}"));
        assert!(
            copy.disable_accessibility_name
                .starts_with(copy.disable_button)
        );
        assert!(copy.disable_accessibility_name.contains("{code}"));
        // Rule 3: the scope beside each decision.
        assert!(copy.enter_scope.contains("{date}"));
        assert!(copy.enter_scope.contains("can't be changed"));
        assert!(copy.disable_scope.contains("still count"));
        // The states are distinct.
        let states = [copy.state_disabled, copy.state_suspended, copy.state_voided];
        let distinct: std::collections::BTreeSet<_> = states.iter().collect();
        assert_eq!(distinct.len(), states.len());
        // The two Insights rows are distinct labels.
        assert_ne!(copy.insights_own, copy.insights_referrals);
        assert_eq!(copy.session_ended, "Sign in again");
    }

    /// The body states the spec's terms as they are today: 10%, the first
    /// 3 months, and that the referee's own credit is untouched (R1, R2).
    #[test]
    fn the_body_says_what_a_referral_earns_and_takes_nothing() {
        let body = referral_copy().body;
        assert!(body.contains("10%"));
        assert!(body.contains("first 3 months"));
        assert!(body.contains("not reduced"));
    }

    #[test]
    fn render_fills_only_the_named_holes() {
        assert_eq!(
            render(
                REFERRAL_COPY_ACCESSIBILITY_NAME,
                &[("code", "TC-7K3M9-QX2RD")]
            ),
            "Copy code TC-7K3M9-QX2RD"
        );
        assert_eq!(
            render(REFERRAL_JOINED_MANY, &[("n", "3")]),
            "3 people joined"
        );
        assert_eq!(
            render(REFERRAL_ENTER_SCOPE, &[("n", "3")]),
            REFERRAL_ENTER_SCOPE,
            "a hole not named stays"
        );
    }
}
