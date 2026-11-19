//! Home reads current answers; missing facts are never zero.
use crate::model::Status;

/// Keep field presence for Home even when an older daemon omits this count.
/// The existing History model defaults legacy counters for its own surface.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct Rollup {
    #[serde(default)]
    pub all_time: Option<ContributionCount>,
    /// Credit not yet settled, drawn only beside the commons' condition.
    #[serde(default)]
    pub credit_pending: Option<f32>,
    /// Credit decided (History's Final): Home's Credit earned.
    #[serde(default)]
    pub credit_final: Option<f32>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ContributionCount {
    #[serde(default)]
    pub contributed: Option<u32>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Counts {
    pub waiting: Option<u64>,
    pub contributed: Option<u32>,
}

impl Counts {
    pub fn from_answers(status: Option<&Status>, rollup: Option<&Rollup>) -> Self {
        Self {
            waiting: status.and_then(|s| s.decisions_owed),
            contributed: rollup
                .and_then(|r| r.all_time.as_ref())
                .and_then(|c| c.contributed),
        }
    }
}

/// What waits on this computer, not contributed (`HomeUncontributed`): the
/// queue's traces that have not left, their size, the credit the core
/// estimates for them, and the daemon's settlement line.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Uncontributed {
    pub count: usize,
    pub bytes: u64,
    /// The sum of the estimates the core says to draw; none with none.
    pub low: Option<f64>,
    pub high: Option<f64>,
    /// The core's finished dollar line (its call to share, else its plain
    /// line), or none to draw.
    pub dollars: Option<String>,
}

impl Uncontributed {
    /// States that have left, or are leaving, this computer.
    pub const GONE: [&'static str; 2] = ["uploading", "uploaded"];

    pub fn of(
        entries: &[crate::model::QueueEntry],
        settlement: Option<&crate::model::SettlementEstimate>,
    ) -> Self {
        let waiting: Vec<&crate::model::QueueEntry> = entries
            .iter()
            .filter(|entry| !Self::GONE.contains(&entry.state.as_str()))
            .collect();
        let drawn: Vec<&crate::model::CreditEstimate> = waiting
            .iter()
            .filter_map(|entry| entry.credit_estimate.as_ref())
            .filter(|estimate| estimate.drawn == Some(true))
            .collect();
        let sum = |pick: fn(&crate::model::CreditEstimate) -> f64| {
            (!drawn.is_empty()).then(|| drawn.iter().map(|e| pick(e)).sum())
        };
        let dollars = settlement
            .map(|s| s.share_text.clone().unwrap_or_else(|| s.text.clone()))
            .filter(|line| !line.trim().is_empty());
        Self {
            count: waiting.len(),
            bytes: waiting
                .iter()
                .fold(0u64, |sum, entry| sum.saturating_add(entry.size_bytes)),
            low: sum(|e| e.low),
            high: sum(|e| e.high),
            dollars,
        }
    }

    /// The size beside the count, in the core's words; none with nothing
    /// waiting.
    pub fn size(&self) -> Option<String> {
        (self.count > 0).then(|| {
            trace_commons_contributor::preview_copy::monitor_screens_copy()
                .home_history
                .uncontributed_size
                .replace("{size}", &crate::model::file_bytes(self.bytes))
        })
    }

    /// The line under the count (`HomeUncontributed.detail`): the dollar
    /// line, which covers every trace here and so needs no count, else the
    /// estimate's range in the core's words; none without either. With one
    /// the tile spans two columns, as the Mac's.
    pub fn detail(&self) -> Option<String> {
        use trace_commons_contributor::history_copy::credit_figure;
        if let Some(dollars) = &self.dollars {
            return Some(dollars.clone());
        }
        let (low, high) = (self.low?, self.high?);
        (self.count > 0).then(|| {
            trace_commons_contributor::preview_copy::monitor_screens_copy()
                .home_history
                .uncontributed_estimate
                .replace("{min}", &credit_figure(low))
                .replace("{max}", &credit_figure(high))
        })
    }
}

/// What Home reads from `commons_credit_summary`: the account's lifetime
/// points while the commons knows them, and whether they wait on an
/// account session (`CommonsCreditSummary.accountSessionRequired`).
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct CreditSummary {
    #[serde(default)]
    pub points_state: String,
    #[serde(default)]
    pub points_unknown_reason: Option<String>,
    #[serde(default)]
    pub commons_points_lifetime_earned: Option<i64>,
}

impl CreditSummary {
    pub fn points_known(&self) -> bool {
        self.points_state == "known"
    }

    pub fn account_session_required(&self) -> bool {
        !self.points_known()
            && self.points_unknown_reason.as_deref() == Some("account-session-required")
    }

    /// The Credit earned tile's line for the whole account
    /// (`HomeFormat.creditAccountTotal`): only while the commons knows its
    /// points; unknown is no line, never a zero.
    pub fn account_total(&self) -> Option<String> {
        let lifetime = self
            .commons_points_lifetime_earned
            .filter(|_| self.points_known())?;
        let template = trace_commons_contributor::preview_copy::monitor_screens_copy()
            .home_history
            .credit_account_total;
        (!template.is_empty()).then(|| {
            template.replace(
                "{amount}",
                &trace_commons_contributor::history_copy::credit_figure(lifetime as f64),
            )
        })
    }
}

/// Whether the Account heading says "No data - Sign in"
/// (`HomeFormat.accountNeedsSignIn`): the commons' points wait on an account
/// session, or the balance would offer a sign-in (the core's balance table
/// offers one on exactly the signed-out and expired states).
pub fn account_needs_sign_in(
    credit: Option<&CreditSummary>,
    balance: Option<&crate::model::NearAiBalance>,
) -> bool {
    credit.is_some_and(CreditSummary::account_session_required)
        || balance.is_some_and(|balance| {
            crate::balance::view(balance).action == crate::copy::CredentialAction::Obtain
        })
}

/// Where the Account heading's Sign in goes (`HomeFormat.accountSignInRoute`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountSignInRoute {
    /// The account session the Credit tiles wait on: `account_sign_in`, as
    /// the Payouts section's sign-in and the Mac's `signInToAccount` call.
    Account,
    /// NEAR AI's sign-in, as the Private AI tab's balance row starts it.
    NearAi,
}

impl AccountSignInRoute {
    /// The daemon call the route starts with.
    pub fn method(self) -> &'static str {
        match self {
            Self::Account => "account_sign_in",
            Self::NearAi => "near_ai_credential_start",
        }
    }
}

/// The account session when the commons' points wait on one, else NEAR AI.
pub fn account_sign_in_route(credit: Option<&CreditSummary>) -> AccountSignInRoute {
    if credit.is_some_and(CreditSummary::account_session_required) {
        AccountSignInRoute::Account
    } else {
        AccountSignInRoute::NearAi
    }
}

/// The NEAR AI balance tile's figure and the line under it
/// (`HomeFormat.balance`): the amount left when the core reads one, else a
/// dash; the core's sentence for an account with no ceiling goes under the
/// dash. No sentence for why there is none: a sign-in it waits on is the
/// Account heading's.
pub fn balance_tile(balance: Option<&crate::model::NearAiBalance>) -> (String, Option<String>) {
    let dash = || ("—".to_owned(), None);
    let Some(balance) = balance else {
        return dash();
    };
    let view = crate::balance::view(balance);
    if !view.figures_carry_the_row() {
        return dash();
    }
    if let (Some(nanos), Some(scale)) = (balance.remaining_nanos, balance.scale) {
        let amount = trace_commons_contributor::private_inference_copy::format_amount(nanos, scale);
        if !amount.is_empty() {
            return (amount, None);
        }
    }
    let sentence = view.remaining_line;
    let unknown = [
        trace_commons_contributor::private_inference_copy::BALANCE_UNKNOWN,
        trace_commons_contributor::private_inference_copy::BALANCE_UNREPORTED,
    ];
    if sentence.is_empty() || unknown.contains(&sentence.as_str()) {
        return dash();
    }
    ("—".to_owned(), Some(sentence))
}

/// Home's Credit earned and Credit pending tiles (`HomeFormat.creditTiles`):
/// earned is History's Final, 0 once History answered with pending credit
/// and nothing decided; pending is a figure only beside the commons'
/// statement of what it waits on (D6). A dash for what is not known.
pub fn credit_tiles(rollup: Option<&Rollup>, condition: Option<&str>) -> (String, String) {
    use trace_commons_contributor::history_copy::credit_figure;
    let dash = || "—".to_owned();
    let earned =
        rollup.and_then(|rollup| rollup.credit_final.or(rollup.credit_pending.map(|_| 0.0)));
    let pending = match (condition, rollup.and_then(|rollup| rollup.credit_pending)) {
        (Some(_), Some(figure)) => Some(figure),
        _ => None,
    };
    (
        earned.map_or_else(dash, |v| credit_figure(f64::from(v))),
        pending.map_or_else(dash, |v| credit_figure(f64::from(v))),
    )
}

pub fn figure(value: Option<impl std::fmt::Display>) -> String {
    value
        .map(|n| n.to_string())
        .unwrap_or_else(|| "—".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(state: &str, bytes: u64) -> crate::model::QueueEntry {
        crate::model::QueueEntry {
            state: state.into(),
            size_bytes: bytes,
            ..crate::model::QueueEntry::default()
        }
    }

    /// The Mac's Not contributed tile: every trace not uploading or
    /// uploaded, and their size beside the count in the core's words.
    #[test]
    fn not_contributed_counts_what_has_not_left_and_its_size() {
        let entries = [
            queued("pending", 100 * 1024),
            queued("held", 45 * 1024),
            queued("uploading", 900 * 1024),
            queued("uploaded", 900 * 1024),
        ];
        let waiting = Uncontributed::of(&entries, None);
        assert_eq!((waiting.count, waiting.bytes), (2, 145 * 1024));
        assert_eq!(waiting.detail(), None, "no estimate, no dollar line");
        let words = trace_commons_contributor::preview_copy::monitor_screens_copy();
        assert_eq!(
            waiting.size().as_deref(),
            Some(
                words
                    .home_history
                    .uncontributed_size
                    // 148,480 bytes, as the Mac's `countStyle: .file`
                    // (decimal) draws it, not 145 binary kilobytes.
                    .replace("{size}", "148 KB")
                    .as_str()
            )
        );
        assert_eq!(Uncontributed::of(&[], None).size(), None);
    }

    /// The line under Not contributed (`HomeUncontributed.detail`): the
    /// core's call to share (else its plain line) when the daemon reports
    /// a settlement estimate; else the range of the estimates the core
    /// says to draw, in its words; else none. An estimate not drawn is
    /// never added, and a trace that left is not counted.
    #[test]
    fn not_contributed_says_the_dollar_line_else_the_drawn_estimate() {
        let estimate = |state: &str, low, high, drawn| crate::model::QueueEntry {
            credit_estimate: Some(crate::model::CreditEstimate {
                low,
                high,
                drawn: Some(drawn),
            }),
            ..queued(state, 1024)
        };
        let entries = [
            estimate("pending", 1.25, 4.0, true),
            estimate("held", 0.5, 2.0, true),
            estimate("pending", 100.0, 900.0, false),
            estimate("uploaded", 100.0, 900.0, true),
        ];
        let words = trace_commons_contributor::preview_copy::monitor_screens_copy();
        let points = Uncontributed::of(&entries, None);
        assert_eq!(points.count, 3);
        assert_eq!(
            points.detail().as_deref(),
            Some(
                words
                    .home_history
                    .uncontributed_estimate
                    .replace("{min}", "1.8")
                    .replace("{max}", "6")
                    .as_str()
            )
        );
        let settlement = crate::model::SettlementEstimate {
            text: "Up to $4.10 from 3 traces".into(),
            share_text: Some("Share them to earn up to $4.10".into()),
        };
        let dollars = Uncontributed::of(&entries, Some(&settlement));
        assert_eq!(
            dollars.detail().as_deref(),
            Some("Share them to earn up to $4.10")
        );
        let plain = crate::model::SettlementEstimate {
            share_text: None,
            ..settlement.clone()
        };
        assert_eq!(
            Uncontributed::of(&entries, Some(&plain))
                .detail()
                .as_deref(),
            Some("Up to $4.10 from 3 traces")
        );
        let blank = crate::model::SettlementEstimate {
            text: " ".into(),
            share_text: None,
        };
        assert_eq!(
            Uncontributed::of(&entries[2..3], Some(&blank)).detail(),
            None,
            "an undrawn estimate and a blank line draw nothing"
        );
    }

    /// The Credit earned tile's account line: the commons' lifetime
    /// points in the core's words, only while it knows them; and the
    /// Account heading's sign-in, while the points wait on an account
    /// session or the balance is signed out or expired.
    #[test]
    fn the_account_line_and_its_sign_in_are_the_macs() {
        let summary = |value| serde_json::from_value::<CreditSummary>(value).unwrap();
        let known = summary(serde_json::json!({
            "points_state": "known", "commons_points_lifetime_earned": 12345
        }));
        let words = trace_commons_contributor::preview_copy::monitor_screens_copy();
        assert_eq!(
            known.account_total().as_deref(),
            Some(
                words
                    .home_history
                    .credit_account_total
                    .replace("{amount}", "12,345")
                    .as_str()
            )
        );
        let waits = summary(serde_json::json!({
            "points_state": "unknown",
            "points_unknown_reason": "account-session-required",
            "commons_points_lifetime_earned": 12345
        }));
        assert_eq!(waits.account_total(), None, "unknown points draw no line");
        assert!(account_needs_sign_in(Some(&waits), None));
        assert!(!account_needs_sign_in(Some(&known), None));
        assert!(!account_needs_sign_in(None, None));
        let balance = |state: &str| crate::model::NearAiBalance {
            state: state.into(),
            ..Default::default()
        };
        assert!(account_needs_sign_in(None, Some(&balance("no_session"))));
        assert!(account_needs_sign_in(
            None,
            Some(&balance("session_expired"))
        ));
        assert!(!account_needs_sign_in(None, Some(&balance("known"))));
    }

    /// The Account heading's Sign in signs in to what is missing, as the
    /// Mac's `HomeFormat.accountSignInRoute` routes it: the account session
    /// when the Credit tiles wait on one, else NEAR AI.
    #[test]
    fn the_account_sign_in_routes_as_the_macs() {
        let summary = |value| serde_json::from_value::<CreditSummary>(value).unwrap();
        let waits = summary(serde_json::json!({
            "points_state": "unknown",
            "points_unknown_reason": "account-session-required"
        }));
        let other_unknown = summary(serde_json::json!({
            "points_state": "unknown",
            "points_unknown_reason": "commons-unreachable"
        }));
        let known = summary(serde_json::json!({
            "points_state": "known", "commons_points_lifetime_earned": 1
        }));
        assert_eq!(
            account_sign_in_route(Some(&waits)),
            AccountSignInRoute::Account
        );
        assert_eq!(
            account_sign_in_route(Some(&other_unknown)),
            AccountSignInRoute::NearAi
        );
        assert_eq!(
            account_sign_in_route(Some(&known)),
            AccountSignInRoute::NearAi
        );
        assert_eq!(account_sign_in_route(None), AccountSignInRoute::NearAi);
        assert_eq!(AccountSignInRoute::Account.method(), "account_sign_in");
        assert_eq!(
            AccountSignInRoute::NearAi.method(),
            "near_ai_credential_start"
        );
    }

    /// The NEAR AI balance tile: what is left as the core formats it, a
    /// dash with the core's no-ceiling sentence under it, and a dash for
    /// any reading that is not one; never $0.00 for an absent figure.
    #[test]
    fn the_balance_tile_is_the_figure_or_a_dash() {
        let known = |remaining| crate::model::NearAiBalance {
            state: "known".into(),
            scale: Some(9),
            remaining_nanos: remaining,
            ..Default::default()
        };
        assert_eq!(
            balance_tile(Some(&known(Some(1_234_560_000_000)))),
            ("$1234.56".to_owned(), None)
        );
        let (dash, line) = balance_tile(Some(&known(None)));
        assert_eq!(dash, "—");
        assert_eq!(
            line.as_deref(),
            Some(trace_commons_contributor::private_inference_copy::BALANCE_NO_REMAINING)
        );
        assert_eq!(balance_tile(None), ("—".to_owned(), None));
        let signed_out = crate::model::NearAiBalance {
            state: "no_session".into(),
            ..Default::default()
        };
        assert_eq!(balance_tile(Some(&signed_out)), ("—".to_owned(), None));
    }

    /// Credit earned is History's Final, 0 once History answered with only
    /// pending credit; Credit pending is a figure only beside the commons'
    /// condition (D6). An unread rollup is a dash in both, never 0.
    #[test]
    fn the_credit_tiles_are_the_macs() {
        let rollup = |value| serde_json::from_value::<Rollup>(value).unwrap();
        assert_eq!(credit_tiles(None, None), ("—".into(), "—".into()));
        let both = rollup(serde_json::json!({"credit_final": 10.0, "credit_pending": 2.5}));
        assert_eq!(credit_tiles(Some(&both), None), ("10".into(), "—".into()));
        assert_eq!(
            credit_tiles(Some(&both), Some("Paid weekly")),
            ("10".into(), "2.5".into())
        );
        let pending_only = rollup(serde_json::json!({"credit_pending": 1.0}));
        assert_eq!(credit_tiles(Some(&pending_only), None).0, "0");
        assert_eq!(
            credit_tiles(Some(&rollup(serde_json::json!({}))), None).0,
            "—"
        );
    }

    #[test]
    fn unavailable_answers_are_not_zero_or_queue_occupancy() {
        let status: Status =
            serde_json::from_value(serde_json::json!({"queue_depth": 42})).unwrap();
        assert_eq!(
            Counts::from_answers(Some(&status), None),
            Counts {
                waiting: None,
                contributed: None
            }
        );
        assert_eq!(figure(None::<u64>), "—");
        assert_eq!(figure(Some(0)), "0");
    }

    #[test]
    fn older_daemon_missing_contributed_is_unknown() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"all_time": {"accepted": 3, "submitted": 5}}),
        ] {
            let rollup: Rollup = serde_json::from_value(value).unwrap();
            assert_eq!(Counts::from_answers(None, Some(&rollup)).contributed, None);
        }
    }

    #[test]
    fn current_answers_use_core_contribution_and_decision_counts() {
        let status: Status =
            serde_json::from_value(serde_json::json!({"decisions_owed": 0, "queue_depth": 42}))
                .unwrap();
        let rollup: Rollup = serde_json::from_value(
            serde_json::json!({"all_time": {"accepted": 3, "submitted": 5, "contributed": 8}}),
        )
        .unwrap();
        assert_eq!(
            Counts::from_answers(Some(&status), Some(&rollup)),
            Counts {
                waiting: Some(0),
                contributed: Some(8)
            }
        );
        // A failed response is None, not the earlier successful snapshot.
        assert_eq!(
            Counts::from_answers(None, None),
            Counts {
                waiting: None,
                contributed: None
            }
        );
    }
}
