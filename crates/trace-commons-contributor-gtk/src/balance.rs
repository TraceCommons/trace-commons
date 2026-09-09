//! What is left in the Private AI account this computer answers calls with.
//!
//! The daemon has read this since #745 and no shell rendered it: a
//! contributor could sign in, spend, and never be told what remained. This
//! module turns one `near_ai_balance` answer into the handful of lines
//! `ui::balance` draws, and decides nothing of its own.
//!
//! **Nothing here judges an amount.** The tone says the read succeeded, not
//! that the balance is healthy -- `balance_state_tone` answers `Clear` for
//! `known` and for nothing else, whatever the figure is. A shell that
//! painted a low number red would be inventing a threshold nobody set, on an
//! account whose ceiling may not exist at all.
//!
//! # The rule this module exists to hold
//!
//! **An empty amount is never `$0.00`.** These figures are SIGNED -- an
//! overdrawn account is negative -- so absence cannot be folded onto an
//! out-of-range integer the way every other money field on this surface
//! folds it. `Option<i64>` carries it here and
//! `private_inference_copy::balance_*_line` decides what each absence looks
//! like: `null` remaining is the no-limit sentence, `null` limit and `null`
//! spent are the empty string, and a real zero is `$0.00` in all three.
//! Nothing in this shell formats a dollar.
//!
//! # `scale` is read, never assumed
//!
//! [`crate::model::NearAiBalance::scale`] is the wire's own field, handed
//! straight to the shared functions. A shell dividing by a nine of its own
//! would be wrong by a factor of a thousand the day the daemon changes it,
//! and would be wrong silently. A daemon that sends no scale at all gets no
//! figures rather than figures under a guessed one.

use crate::copy;
use crate::model::NearAiBalance;

/// One balance answer, as the lines a view draws.
///
/// Built as a unit from a single state label so the sentence, the colour and
/// the control cannot answer differently -- the shape [`crate::attestation`]
/// settled on, with the field it deliberately lacks: this row HAS an action,
/// on exactly two states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceView {
    /// The state sentence, or the EMPTY STRING for `known`.
    ///
    /// Empty is not an oversight and not a missing case: on a read that
    /// succeeded the figures are the content, and a sentence above them
    /// announcing the read succeeded is this app narrating itself. The view
    /// draws [`Self::remaining_line`] as the row's own statement there.
    pub state_line: &'static str,
    /// How firmly the row reads. `Clear` means READ, not healthy.
    pub tone: copy::PrivateInferenceTone,
    /// The one control this row may offer: the sign-in row's own `Obtain`,
    /// on `no_session` and `session_expired` and nothing else.
    pub action: copy::CredentialAction,
    /// What is left, as a finished sentence, or the sentence for an account
    /// with no ceiling. NEVER `$0.00` for an absent figure.
    pub remaining_line: String,
    /// The configured ceiling, or the empty string -- which the view draws
    /// as no line at all. The remaining line has already said the part that
    /// matters about an uncapped account.
    pub limit_line: String,
    /// What the whole account has spent, or the empty string. A zero is not
    /// that: an account that has spent nothing renders `$0.00`, which is
    /// true.
    pub spent_line: String,
    /// How long ago THIS COMPUTER asked, or the empty string.
    pub observed_line: String,
}

impl BalanceView {
    /// Whether the figures carry this row rather than a sentence.
    ///
    /// **Read off the shared table's own answer, not off a state label.**
    /// The empty state line IS the statement that this state's row is
    /// figures; matching on `known` here would be a second copy of that
    /// decision, in the one shell that could disagree with the other two
    /// without anything noticing.
    #[must_use]
    pub fn figures_carry_the_row(&self) -> bool {
        self.state_line.is_empty()
    }
}

/// The balance surface for one answer. Never absent.
#[must_use]
pub fn view(balance: &NearAiBalance) -> BalanceView {
    view_through(balance, &SHARED)
}

/// The lookups one row is built from, behind function pointers.
///
/// **A seam for the tests, not a policy choice**, the reason
/// `AttestationTable` is one: a test can hand [`view_through`] a table
/// answering the INVERSE of the real one and require the output to follow
/// it. Code that decided anything for itself would keep agreeing with the
/// real table and could not agree with the inverse by coincidence.
struct BalanceTable {
    state_line: fn(&str) -> &'static str,
    state_tone: fn(&str) -> copy::PrivateInferenceTone,
    action: fn(&str) -> copy::CredentialAction,
    remaining_line: fn(Option<i64>, u8) -> String,
    limit_line: fn(Option<i64>, u8) -> String,
    spent_line: fn(Option<i64>, u8) -> String,
    observed_line: fn(Option<u64>) -> String,
}

/// The one table this shell ships: the shared crate's, which macOS and
/// Windows reach across the C ABI as `tc_near_ai_balance_*`.
const SHARED: BalanceTable = BalanceTable {
    state_line: copy::balance_state_line,
    state_tone: copy::balance_state_tone,
    action: copy::balance_action,
    remaining_line: copy::balance_remaining_line,
    limit_line: copy::balance_limit_line,
    spent_line: copy::balance_spent_line,
    observed_line: copy::balance_observed_line,
};

/// Read one answer through a table.
///
/// Every field is a lookup. There is no `match` on a state label anywhere in
/// this shell, and nothing here that would still be true if the table said
/// something else.
///
/// # The figures are gated, and on the table's answer
///
/// The nanos keys are present and `null` in every state that is not
/// `known`, so handing them to the shared functions unconditionally would
/// print the no-limit sentence -- a claim about an account -- on a machine
/// that holds no sign-in at all. The gate is the state line being empty,
/// which is the shared table's own way of saying this state's row is
/// figures. A daemon that sends no `scale` gets no figures either: the
/// alternative is a figure divided by a constant this shell made up.
fn view_through(balance: &NearAiBalance, table: &BalanceTable) -> BalanceView {
    let state_line = (table.state_line)(&balance.state);
    let figures = state_line.is_empty();
    // `scale` off the wire or nothing at all. Never a nine of this shell's.
    let scale = balance.scale.filter(|_| figures);
    BalanceView {
        state_line,
        tone: (table.state_tone)(&balance.state),
        action: (table.action)(&balance.state),
        remaining_line: scale.map_or_else(String::new, |scale| {
            (table.remaining_line)(balance.remaining_nanos, scale)
        }),
        limit_line: scale.map_or_else(String::new, |scale| {
            (table.limit_line)(balance.spend_limit_nanos, scale)
        }),
        spent_line: scale.map_or_else(String::new, |scale| {
            (table.spent_line)(balance.total_spent_nanos, scale)
        }),
        observed_line: if figures {
            (table.observed_line)(seconds_since(balance.observed_at.as_deref()))
        } else {
            String::new()
        },
    }
}

/// How long ago the daemon asked, from the timestamp it stamped.
///
/// `observed_at` is the DAEMON'S clock at the moment the service answered,
/// not the service's own `updated_at`, so the only true thing to say with it
/// is when the question was put -- which is what the shared sentence says.
///
/// An absent or unparseable time is `None`, which the shared function draws
/// as no line at all rather than as "just now" invented under a figure.
fn seconds_since(at: Option<&str>) -> Option<u64> {
    let at = chrono::DateTime::parse_from_rfc3339(at?).ok()?;
    let elapsed = chrono::Utc::now()
        .signed_duration_since(at.with_timezone(&chrono::Utc))
        .num_seconds();
    u64::try_from(elapsed.max(0)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_commons_contributor::private_inference_copy::{
        BALANCE_NO_REMAINING, LABEL_BALANCE_KNOWN, LABEL_BALANCE_NO_ORGANIZATION,
        LABEL_BALANCE_NO_SESSION, LABEL_BALANCE_SESSION_EXPIRED, LABEL_BALANCE_UNAVAILABLE,
        private_inference_copy,
    };

    /// This module's source, read at compile time so a sweep cannot pass
    /// over a file that moved.
    const SOURCE: &str = include_str!("balance.rs");

    /// Every state a daemon can report, plus the two ways of reporting none.
    fn every_label() -> Vec<&'static str> {
        vec![
            LABEL_BALANCE_KNOWN,
            LABEL_BALANCE_NO_SESSION,
            LABEL_BALANCE_SESSION_EXPIRED,
            LABEL_BALANCE_NO_ORGANIZATION,
            LABEL_BALANCE_UNAVAILABLE,
            "",
            "a_balance_state_from_a_later_daemon",
        ]
    }

    /// The module's own code, without the tests that quote it.
    fn code() -> &'static str {
        SOURCE
            .split("\n#[cfg(test)]")
            .next()
            .expect("the module has a body before its tests")
    }

    /// One answer off the wire, as the daemon actually shapes it.
    fn decode(body: &str) -> NearAiBalance {
        serde_json::from_str(body).expect("the daemon's own body decodes")
    }

    /// A `known` body with the numeric fields spelled by the caller.
    fn known(fields: &str) -> NearAiBalance {
        decode(&format!(
            r#"{{"state":"known","currency":"USD","scale":9{fields}}}"#
        ))
    }

    /// THE RULE THIS ROW EXISTS TO HOLD: a null figure is not a zero one.
    ///
    /// `remaining_nanos` is nullable even on a `known` read -- it is the
    /// ordinary shape of an account nobody has capped -- and the answer a
    /// shell reaches for when it formats its own money is `$0.00`. That
    /// would tell a contributor with an uncapped account that they are out
    /// of money, which is both false and the most alarming thing this
    /// surface could say.
    #[test]
    fn a_null_remaining_figure_is_never_a_zero_balance() {
        let absent = view(&known(r#","remaining_nanos":null"#));
        assert_eq!(absent.remaining_line, BALANCE_NO_REMAINING);
        assert!(
            !absent.remaining_line.contains("$0.00"),
            "an absent figure rendered as a zero one: {}",
            absent.remaining_line
        );
        assert!(
            !absent.remaining_line.contains('$'),
            "an absent figure rendered a figure: {}",
            absent.remaining_line
        );

        // A real zero IS a figure, and says the money is gone.
        let zero = view(&known(r#","remaining_nanos":0"#));
        assert!(
            zero.remaining_line.contains("$0.00"),
            "a zero balance did not render as one: {}",
            zero.remaining_line
        );
        assert_ne!(zero.remaining_line, absent.remaining_line);
    }

    /// The other two figures fold absence the other way, and a zero is
    /// still a zero in both.
    #[test]
    fn an_absent_limit_or_spend_draws_no_line_and_a_zero_draws_one() {
        let absent = view(&known(
            r#","spend_limit_nanos":null,"total_spent_nanos":null"#,
        ));
        assert_eq!(absent.limit_line, "");
        assert_eq!(absent.spent_line, "");

        let zero = view(&known(r#","spend_limit_nanos":0,"total_spent_nanos":0"#));
        assert!(zero.limit_line.contains("$0.00"), "{}", zero.limit_line);
        assert!(zero.spent_line.contains("$0.00"), "{}", zero.spent_line);
    }

    /// The wire's `scale` decides the figure, and nothing in this shell
    /// does.
    ///
    /// The same integer under two scales is two different amounts, and a
    /// shell holding a nine of its own would render one of them wrong by a
    /// factor of a thousand -- silently, in the direction that reads as
    /// plenty.
    #[test]
    fn the_scale_comes_off_the_wire() {
        let at = |scale: u8| {
            view(&decode(&format!(
                r#"{{"state":"known","scale":{scale},"remaining_nanos":8500000000}}"#
            )))
            .remaining_line
        };
        assert!(at(9).contains("$8.50"), "{}", at(9));
        assert!(at(6).contains("$8500.00"), "{}", at(6));
        assert_ne!(at(9), at(6));
        // And no constant of this shell's own anywhere in the module.
        assert!(
            !code().contains("1_000_000_000") && !code().contains("1000000000"),
            "this shell holds a scale constant of its own"
        );
    }

    /// A daemon that reports no scale gets no figures, rather than figures
    /// under a scale this shell made up.
    #[test]
    fn a_missing_scale_draws_no_figure_at_all() {
        let bare = view(&decode(
            r#"{"state":"known","remaining_nanos":8500000000,"total_spent_nanos":1}"#,
        ));
        assert_eq!(bare.remaining_line, "");
        assert_eq!(bare.limit_line, "");
        assert_eq!(bare.spent_line, "");
    }

    /// Only the state whose row IS figures gets figures.
    ///
    /// The nanos keys are present and `null` in every other state, so a view
    /// that handed them on unconditionally would print the no-limit sentence
    /// -- a claim about an account -- on a machine holding no sign-in at
    /// all.
    #[test]
    fn only_a_read_balance_carries_figures() {
        for label in every_label() {
            // The daemon's own non-known body: every numeric key null.
            let answer = decode(&format!(
                r#"{{"state":"{label}","currency":"USD","scale":9,
                     "remaining_nanos":null,"spend_limit_nanos":null,
                     "total_spent_nanos":null,"observed_at":null}}"#
            ));
            let row = view(&answer);
            if label == LABEL_BALANCE_KNOWN {
                assert!(row.figures_carry_the_row(), "{label:?}");
                assert_eq!(row.remaining_line, BALANCE_NO_REMAINING, "{label:?}");
                continue;
            }
            assert!(!row.figures_carry_the_row(), "{label:?}");
            for line in [&row.remaining_line, &row.limit_line, &row.spent_line] {
                assert_eq!(line, "", "{label:?} drew a figure it does not have");
            }
            assert_eq!(row.observed_line, "", "{label:?}");
        }
    }

    /// A state this build cannot read borrows nobody's sentence, is not
    /// painted as working, and offers nothing.
    #[test]
    fn an_unread_balance_state_borrows_no_sentence_and_offers_nothing() {
        let shared = private_inference_copy();
        let unknown = view(&decode(
            r#"{"state":"a_balance_state_from_a_later_daemon"}"#,
        ));
        let unreported = view(&decode(r#"{}"#));
        assert_eq!(unknown.state_line, shared.balance_unknown);
        assert_eq!(unreported.state_line, shared.balance_unreported);
        // Two ways of having no state, two different facts, two sentences.
        assert_ne!(unknown.state_line, unreported.state_line);
        for row in [&unknown, &unreported] {
            for borrowed in [
                shared.balance_no_session,
                shared.balance_session_expired,
                shared.balance_no_organization,
                shared.balance_unavailable,
            ] {
                assert_ne!(row.state_line, borrowed, "a known sentence was borrowed");
            }
            assert_ne!(row.tone, copy::PrivateInferenceTone::Clear);
            assert_eq!(row.action, copy::CredentialAction::None);
        }
    }

    /// Every state reaches its own sentence.
    #[test]
    fn each_state_reaches_its_own_sentence() {
        let shared = private_inference_copy();
        let line = |label: &str| view(&decode(&format!(r#"{{"state":"{label}"}}"#))).state_line;
        assert_eq!(line(LABEL_BALANCE_NO_SESSION), shared.balance_no_session);
        assert_eq!(
            line(LABEL_BALANCE_SESSION_EXPIRED),
            shared.balance_session_expired
        );
        assert_eq!(
            line(LABEL_BALANCE_NO_ORGANIZATION),
            shared.balance_no_organization
        );
        assert_eq!(line(LABEL_BALANCE_UNAVAILABLE), shared.balance_unavailable);
        assert_eq!(line(LABEL_BALANCE_KNOWN), "");
        // Six labels, six distinct answers.
        let mut seen = std::collections::BTreeSet::new();
        for label in every_label() {
            assert!(seen.insert(line(label)), "{label:?} shares a sentence");
        }
    }

    /// The tone says the read succeeded and never that the money is fine.
    #[test]
    fn only_a_read_balance_reads_as_settled_whatever_the_figure() {
        for label in every_label() {
            let tone = view(&decode(&format!(r#"{{"state":"{label}"}}"#))).tone;
            if label == LABEL_BALANCE_KNOWN {
                assert_eq!(tone, copy::PrivateInferenceTone::Clear, "{label:?}");
            } else {
                assert_ne!(tone, copy::PrivateInferenceTone::Clear, "{label:?}");
            }
        }
        // Nothing judges an amount: an empty account, an overdrawn one and a
        // full one are painted identically, because all three were READ.
        for nanos in ["0", "-5000000000", "8500000000"] {
            assert_eq!(
                view(&known(&format!(r#","remaining_nanos":{nanos}"#))).tone,
                copy::PrivateInferenceTone::Clear,
                "{nanos} was judged"
            );
        }
    }

    /// One action, on exactly two states.
    ///
    /// A refused session answers `Obtain` WITHOUT a forget first: the
    /// ceremony overwrites both records, and forgetting would throw away a
    /// working key to fix an unrelated sign-in.
    #[test]
    fn obtain_appears_on_exactly_the_two_states_that_answer_it() {
        let action = |label: &str| view(&decode(&format!(r#"{{"state":"{label}"}}"#))).action;
        assert_eq!(
            action(LABEL_BALANCE_NO_SESSION),
            copy::CredentialAction::Obtain
        );
        assert_eq!(
            action(LABEL_BALANCE_SESSION_EXPIRED),
            copy::CredentialAction::Obtain
        );
        for label in every_label() {
            if label == LABEL_BALANCE_NO_SESSION || label == LABEL_BALANCE_SESSION_EXPIRED {
                continue;
            }
            assert_eq!(
                action(label),
                copy::CredentialAction::None,
                "{label:?} offered a control"
            );
        }
        // Never Forget, on any state. It is not this row's action.
        for label in every_label() {
            assert_ne!(action(label), copy::CredentialAction::Forget, "{label:?}");
            assert_ne!(action(label), copy::CredentialAction::Cancel, "{label:?}");
        }
    }

    /// The age line is the daemon's clock, and an absent one draws nothing.
    #[test]
    fn an_absent_observation_time_draws_no_age() {
        assert_eq!(view(&known(r#","observed_at":null"#)).observed_line, "");
        assert_eq!(
            view(&known(r#","observed_at":"not a timestamp""#)).observed_line,
            ""
        );
        let now = chrono::Utc::now().to_rfc3339();
        let fresh = view(&known(&format!(r#","observed_at":"{now}""#)));
        assert!(!fresh.observed_line.is_empty(), "a fresh read drew no age");
    }

    /// Nothing here decides anything: a table answering the inverse of the
    /// real one is followed exactly.
    ///
    /// A view that matched on a state label of its own would keep agreeing
    /// with the real table and could not agree with this one by accident.
    #[test]
    fn the_view_follows_the_table_it_is_given() {
        fn inverted_line(label: &str) -> &'static str {
            if label == LABEL_BALANCE_KNOWN {
                "x"
            } else {
                ""
            }
        }
        fn inverted_tone(_: &str) -> copy::PrivateInferenceTone {
            copy::PrivateInferenceTone::Refused
        }
        fn inverted_action(_: &str) -> copy::CredentialAction {
            copy::CredentialAction::Forget
        }
        fn amount(nanos: Option<i64>, _: u8) -> String {
            nanos.map_or_else(|| "absent".to_string(), |n| n.to_string())
        }
        fn age(_: Option<u64>) -> String {
            "age".to_string()
        }
        let table = BalanceTable {
            state_line: inverted_line,
            state_tone: inverted_tone,
            action: inverted_action,
            remaining_line: amount,
            limit_line: amount,
            spent_line: amount,
            observed_line: age,
        };
        // `known` now has a sentence, so its row is NOT figures.
        let row = view_through(&known(r#","remaining_nanos":7"#), &table);
        assert_eq!(row.state_line, "x");
        assert_eq!(row.tone, copy::PrivateInferenceTone::Refused);
        assert_eq!(row.action, copy::CredentialAction::Forget);
        assert_eq!(row.remaining_line, "");
        assert!(!row.figures_carry_the_row());
        // And `no_session` now answers the empty string, so its row IS.
        let flipped = view_through(
            &decode(r#"{"state":"no_session","scale":9,"remaining_nanos":7}"#),
            &table,
        );
        assert!(flipped.figures_carry_the_row());
        assert_eq!(flipped.remaining_line, "7");
        assert_eq!(flipped.observed_line, "age");
    }

    /// No state label is spelled in this shell's own code.
    #[test]
    fn the_state_is_not_branched_on_in_this_shell() {
        let code = code();
        for spelled in [
            LABEL_BALANCE_KNOWN,
            LABEL_BALANCE_NO_SESSION,
            LABEL_BALANCE_SESSION_EXPIRED,
            LABEL_BALANCE_NO_ORGANIZATION,
            LABEL_BALANCE_UNAVAILABLE,
        ] {
            assert!(
                !code.contains(&format!("\"{spelled}\"")),
                "the state is branched on in this shell: {spelled}"
            );
        }
        for read in [
            "copy::balance_state_line",
            "copy::balance_state_tone",
            "copy::balance_action",
            "copy::balance_remaining_line",
            "copy::balance_limit_line",
            "copy::balance_spent_line",
            "copy::balance_observed_line",
        ] {
            assert!(code.contains(read), "{read} stopped coming from the table");
        }
    }
}
