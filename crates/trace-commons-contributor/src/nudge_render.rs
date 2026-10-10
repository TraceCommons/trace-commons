//! Filling the nudge copy: every sentence a re-engagement surface shows,
//! composed here from [`crate::nudge_copy`] so no shell composes one of its
//! own.
//!
//! Pure: counts, tool display names, a date and an estimate sum in; finished
//! text out. It reads no state and logs nothing. Which template a count gets
//! is [`nudge_copy::pick`]'s, and the idle threshold is always
//! [`nudge_copy::days_phrase`], so "1 sessions" and "1 days" cannot be
//! composed here.
//!
//! The words are the copy table's and share its status: DRAFT, NEEDS
//! APPROVAL.

use chrono::NaiveDate;
use serde::Serialize;

use crate::nudge_copy as copy;

/// One button a surface draws: a stable id the shell acts on, and its label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Action {
    pub id: &'static str,
    pub label: String,
}

/// Opens Traces (or History, for verdicts) at the nudge's subjects.
pub const ACTION_REVIEW: &str = "review";
/// Opens History.
pub const ACTION_SEE_HISTORY: &str = "see_history";
/// The in-app "Not now": `nudge_decline`.
pub const ACTION_NOT_NOW: &str = "not_now";

/// The text of an in-app card and its menu-bar panel row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CardText {
    pub title: String,
    /// Empty when the title says everything.
    pub body: String,
    pub actions: Vec<Action>,
    pub panel_row: String,
}

/// The text of one standalone notification (`reengage_due`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotificationText {
    pub title: String,
    pub body: String,
    pub actions: Vec<Action>,
}

/// The menu-bar mark's words: an accessibility sentence and the Windows
/// tooltip clause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MarkText {
    pub accessibility: String,
    pub tooltip: String,
}

/// A local credit estimate summed over a batch, as `status` carries it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EstimateSum {
    pub low: f64,
    pub high: f64,
    /// How many of the batch have an estimate.
    pub known: u64,
    /// Whether the estimate is drawn at all (`ipc::estimate_is_drawn`).
    pub drawn: bool,
}

/// A batch of waiting sessions: the idle candidates (U4) or the backlog (U1).
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    pub count: u64,
    /// Tool display names, deduplicated, in a stable order. Empty when the
    /// words should name no tool: some source in the batch has no display
    /// name, and naming only the others would credit them with its
    /// sessions.
    pub tools: Vec<String>,
    /// The idle threshold in days. Unused by the backlog.
    pub idle_days: u32,
    /// How many fit at least one matched mission, or `None` while no
    /// catalogue is live.
    pub mission_fit: Option<u64>,
    pub estimate: Option<EstimateSum>,
}

/// Verdict news (U2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdicts {
    pub accepted: u32,
    pub held: u32,
    /// The final credit in tenths, or `None` when it rounds to zero.
    pub credit_final_tenths: Option<u64>,
    /// The local date the news began.
    pub since: NaiveDate,
}

impl Verdicts {
    /// Credit becoming final is the only news.
    fn finals_only(&self) -> bool {
        self.accepted == 0 && self.held == 0
    }
}

/// The longest tool name put into notification text, in characters (spec
/// section 4.2).
pub const TOOL_NAME_MAX_CHARS: usize = 64;

/// `template` with each `{name}` replaced. Every placeholder in `template`
/// must be given: a placeholder left in the output is a bug, and tests over
/// every renderer below check for one.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (name, value) in values {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

fn cap_tool(name: &str) -> String {
    name.chars().take(TOOL_NAME_MAX_CHARS).collect()
}

/// `{tool}`: one name, two joined, or "{k} tools". Empty for no tools,
/// which no renderer places: a batch with no tool to name takes the
/// `_NO_TOOL` templates instead (see [`names_tools`]).
#[must_use]
pub fn tool_phrase(tools: &[String]) -> String {
    match tools {
        [] => String::new(),
        [one] => cap_tool(one),
        [a, b] => fill(
            copy::TOOL_LIST_TWO,
            &[("a", &cap_tool(a)), ("b", &cap_tool(b))],
        ),
        many => fill(copy::TOOL_LIST_MANY, &[("k", &many.len().to_string())]),
    }
}

/// A credit figure to one decimal: `{x}`.
#[must_use]
pub fn credit_text(tenths: u64) -> String {
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// An estimate band end: whole numbers without a decimal ("1"), halves
/// with one ("1.5"). Band ends are multiples of 0.5.
#[must_use]
pub fn band_text(value: f64) -> String {
    let halves = (value * 2.0).round() as i64;
    if halves % 2 == 0 {
        format!("{}", halves / 2)
    } else {
        format!("{}.5", halves / 2)
    }
}

/// `{date}`: "8 October".
#[must_use]
pub fn date_text(date: NaiveDate) -> String {
    date.format("%-d %B").to_string()
}

/// The clauses a batch card's body carries after its fixed sentence: which
/// missions it fits, then the estimate, each only when there is something
/// true to say.
fn batch_clauses(batch: &Batch) -> Vec<String> {
    let mut clauses = Vec::new();
    if let Some(m) = batch.mission_fit.filter(|m| *m > 0) {
        clauses.push(fill(
            copy::pick(
                m,
                copy::NUDGE_MISSION_FIT_CLAUSE,
                copy::NUDGE_MISSION_FIT_CLAUSE_ONE,
            ),
            &[("m", &m.to_string())],
        ));
    }
    if let Some(e) = batch.estimate.filter(|e| e.drawn && e.known > 0) {
        let (low, high) = (band_text(e.low), band_text(e.high));
        let clause = if e.known >= batch.count {
            fill(
                copy::NUDGE_ESTIMATE_CLAUSE,
                &[("low", &low), ("high", &high)],
            )
        } else {
            fill(
                copy::NUDGE_ESTIMATE_CLAUSE_PARTIAL,
                &[
                    ("known", &e.known.to_string()),
                    ("low", &low),
                    ("high", &high),
                ],
            )
        };
        clauses.push(clause);
    }
    clauses
}

fn join(first: String, rest: Vec<String>) -> String {
    std::iter::once(first)
        .chain(rest)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn action(id: &'static str, label: &str) -> Action {
    Action {
        id,
        label: label.to_string(),
    }
}

/// Whether a batch's words name its tools. A batch with none to name
/// (`tools` empty: some source in it has no display name) reads without
/// one, never "from 0 tools".
fn names_tools(batch: &Batch) -> bool {
    !batch.tools.is_empty()
}

/// `with_tool` when the batch names its tools, else `without`.
fn by_tool(batch: &Batch, with_tool: &'static str, without: &'static str) -> &'static str {
    if names_tools(batch) {
        with_tool
    } else {
        without
    }
}

/// The idle-sessions card (U4) and its panel row.
#[must_use]
pub fn idle_card(batch: &Batch) -> CardText {
    let (n, days, tool) = (
        batch.count.to_string(),
        copy::days_phrase(batch.idle_days),
        tool_phrase(&batch.tools),
    );
    let values = [("n", n.as_str()), ("days", days.as_str()), ("tool", &tool)];
    let one = |many, single| copy::pick(batch.count, many, single);
    CardText {
        title: fill(
            one(
                by_tool(
                    batch,
                    copy::NUDGE_IDLE_TITLE,
                    copy::NUDGE_IDLE_TITLE_NO_TOOL,
                ),
                by_tool(
                    batch,
                    copy::NUDGE_IDLE_TITLE_ONE,
                    copy::NUDGE_IDLE_TITLE_NO_TOOL_ONE,
                ),
            ),
            &values,
        ),
        body: join(
            one(copy::NUDGE_IDLE_BODY, copy::NUDGE_IDLE_BODY_ONE).to_string(),
            batch_clauses(batch),
        ),
        actions: vec![
            action(ACTION_REVIEW, copy::NUDGE_IDLE_REVIEW),
            action(ACTION_NOT_NOW, copy::NUDGE_NOT_NOW),
        ],
        panel_row: fill(
            one(copy::NUDGE_PANEL_IDLE, copy::NUDGE_PANEL_IDLE_ONE),
            &values,
        ),
    }
}

/// The review-backlog card (U1) and its panel row.
#[must_use]
pub fn backlog_card(batch: &Batch) -> CardText {
    let n = batch.count.to_string();
    let values = [("n", n.as_str())];
    let one = |many, single| copy::pick(batch.count, many, single);
    CardText {
        title: fill(
            one(copy::NUDGE_BACKLOG_TITLE, copy::NUDGE_BACKLOG_TITLE_ONE),
            &values,
        ),
        body: join(
            one(copy::NUDGE_BACKLOG_BODY, copy::NUDGE_BACKLOG_BODY_ONE).to_string(),
            batch_clauses(batch),
        ),
        actions: vec![
            action(
                ACTION_REVIEW,
                &fill(
                    one(
                        copy::NUDGE_BACKLOG_REVIEW_IN_TRACES,
                        copy::NUDGE_BACKLOG_REVIEW_IN_TRACES_ONE,
                    ),
                    &values,
                ),
            ),
            action(ACTION_NOT_NOW, copy::NUDGE_NOT_NOW),
        ],
        panel_row: one(copy::NUDGE_PANEL_BACKLOG, copy::NUDGE_PANEL_BACKLOG_ONE).to_string(),
    }
}

/// The parts of [`copy::NUDGE_VERDICTS_TITLE`] dropped when their count is
/// zero ("A zero clause is dropped"). Pinned by a test to occur in it.
const VERDICTS_ACCEPTED_CLAUSE: &str = "{a} accepted, ";
const VERDICTS_HELD_CLAUSE: &str = ", {h} held for privacy review";

fn verdict_values(v: &Verdicts) -> [(&'static str, String); 4] {
    [
        ("a", v.accepted.to_string()),
        ("h", v.held.to_string()),
        ("date", date_text(v.since)),
        (
            "x",
            v.credit_final_tenths.map(credit_text).unwrap_or_default(),
        ),
    ]
}

fn fill_owned(template: &str, values: &[(&'static str, String)]) -> String {
    let borrowed: Vec<(&str, &str)> = values.iter().map(|(k, v)| (*k, v.as_str())).collect();
    fill(template, &borrowed)
}

/// The final clause, or nothing when no credit became final.
fn final_clause(v: &Verdicts) -> String {
    v.credit_final_tenths
        .map(|t| fill(copy::NUDGE_VERDICTS_FINAL_CLAUSE, &[("x", &credit_text(t))]))
        .unwrap_or_default()
}

/// The verdicts card (U2) and its panel row. It has no "Not now": it is
/// news, and opening it is what acknowledges it.
#[must_use]
pub fn verdicts_card(v: &Verdicts) -> CardText {
    let values = verdict_values(v);
    let (title, body, panel_row) = if v.finals_only() {
        (
            fill_owned(copy::NUDGE_VERDICTS_TITLE_FINAL_ONLY, &values),
            String::new(),
            fill_owned(copy::NUDGE_PANEL_VERDICTS_FINAL_ONLY, &values),
        )
    } else {
        let mut template = copy::NUDGE_VERDICTS_TITLE.to_string();
        if v.accepted == 0 {
            template = template.replace(VERDICTS_ACCEPTED_CLAUSE, "");
        } else if v.held == 0 {
            template = template.replace(VERDICTS_HELD_CLAUSE, "");
        }
        (
            fill_owned(&template, &values),
            final_clause(v),
            fill_owned(
                by_zero_clause(
                    v,
                    copy::NUDGE_PANEL_VERDICTS,
                    copy::NUDGE_PANEL_VERDICTS_ACCEPTED_ONLY,
                    copy::NUDGE_PANEL_VERDICTS_HELD_ONLY,
                ),
                &values,
            ),
        )
    };
    CardText {
        title,
        body,
        actions: vec![action(ACTION_SEE_HISTORY, copy::NUDGE_VERDICTS_SEE)],
        panel_row,
    }
}

/// `both` when some were accepted and some held, else the template that
/// leaves out the zero count ("A zero clause is dropped"). Not for
/// finals-only news, which has templates of its own.
fn by_zero_clause(
    v: &Verdicts,
    both: &'static str,
    accepted_only: &'static str,
    held_only: &'static str,
) -> &'static str {
    if v.held == 0 {
        accepted_only
    } else if v.accepted == 0 {
        held_only
    } else {
        both
    }
}

/// The verdict sentence shared by the N2 notification and the digest fold:
/// the counts, then the final clause; or the final clause alone. A zero
/// count is left out, never said.
fn verdict_sentence(v: &Verdicts) -> String {
    if v.finals_only() {
        return final_clause(v);
    }
    let template = if v.held == 0 {
        copy::pick(
            u64::from(v.accepted),
            copy::NOTIFY_VERDICTS_BODY_ACCEPTED_ONLY,
            copy::NOTIFY_VERDICTS_BODY_ACCEPTED_ONLY_ONE,
        )
    } else if v.accepted == 0 {
        copy::pick(
            u64::from(v.held),
            copy::NOTIFY_VERDICTS_BODY_HELD_ONLY,
            copy::NOTIFY_VERDICTS_BODY_HELD_ONLY_ONE,
        )
    } else {
        copy::pick(
            u64::from(v.accepted),
            copy::NOTIFY_VERDICTS_BODY,
            copy::NOTIFY_VERDICTS_BODY_ONE,
        )
    };
    let body = fill_owned(template, &verdict_values(v));
    join(body, vec![final_clause(v)])
}

/// N1: the idle-sessions notification.
#[must_use]
pub fn idle_notification(batch: &Batch) -> NotificationText {
    let (n, days, tool) = (
        batch.count.to_string(),
        copy::days_phrase(batch.idle_days),
        tool_phrase(&batch.tools),
    );
    NotificationText {
        title: copy::NOTIFY_TITLE.to_string(),
        body: fill(
            copy::pick(
                batch.count,
                by_tool(
                    batch,
                    copy::NOTIFY_IDLE_BODY,
                    copy::NOTIFY_IDLE_BODY_NO_TOOL,
                ),
                by_tool(
                    batch,
                    copy::NOTIFY_IDLE_BODY_ONE,
                    copy::NOTIFY_IDLE_BODY_NO_TOOL_ONE,
                ),
            ),
            &[("n", &n), ("days", &days), ("tool", &tool)],
        ),
        actions: vec![
            action(ACTION_REVIEW, copy::NOTIFY_ACTION_REVIEW_IDLE),
            action(ACTION_NOT_NOW, copy::NOTIFY_ACTION_NOT_NOW),
        ],
    }
}

/// N2: the verdicts notification.
#[must_use]
pub fn verdicts_notification(v: &Verdicts) -> NotificationText {
    NotificationText {
        title: copy::NOTIFY_TITLE.to_string(),
        body: verdict_sentence(v),
        actions: vec![action(ACTION_SEE_HISTORY, copy::NOTIFY_ACTION_SEE_HISTORY)],
    }
}

/// The idle sentence folded into a due digest, with the mission clause when
/// some of the batch fit one.
#[must_use]
pub fn digest_idle_sentence(batch: &Batch) -> String {
    let (n, days, tool) = (
        batch.count.to_string(),
        copy::days_phrase(batch.idle_days),
        tool_phrase(&batch.tools),
    );
    let sentence = fill(
        copy::pick(
            batch.count,
            by_tool(
                batch,
                copy::DIGEST_IDLE_SENTENCE,
                copy::DIGEST_IDLE_SENTENCE_NO_TOOL,
            ),
            by_tool(
                batch,
                copy::DIGEST_IDLE_SENTENCE_ONE,
                copy::DIGEST_IDLE_SENTENCE_NO_TOOL_ONE,
            ),
        ),
        &[("n", &n), ("days", &days), ("tool", &tool)],
    );
    let mission = Batch {
        estimate: None,
        ..batch.clone()
    };
    join(sentence, batch_clauses(&mission))
}

/// The verdict sentence folded into a due digest.
#[must_use]
pub fn digest_verdict_sentence(v: &Verdicts) -> String {
    verdict_sentence(v)
}

/// The mark's words while it shows verdict news.
#[must_use]
pub fn mark_news_text(v: &Verdicts) -> MarkText {
    let accessibility = if v.finals_only() {
        fill_owned(copy::MARK_A11Y_VERDICTS_FINAL_ONLY, &verdict_values(v))
    } else {
        fill_owned(
            by_zero_clause(
                v,
                copy::MARK_A11Y_VERDICTS,
                copy::MARK_A11Y_VERDICTS_ACCEPTED_ONLY,
                copy::MARK_A11Y_VERDICTS_HELD_ONLY,
            ),
            &verdict_values(v),
        )
    };
    MarkText {
        accessibility,
        tooltip: copy::MARK_TOOLTIP_VERDICTS.to_string(),
    }
}

/// The mark's words while it shows the idle halo.
#[must_use]
pub fn mark_ready_text(batch: &Batch) -> MarkText {
    let (n, days) = (batch.count.to_string(), copy::days_phrase(batch.idle_days));
    let values = [("n", n.as_str()), ("days", days.as_str())];
    MarkText {
        accessibility: fill(
            copy::pick(batch.count, copy::MARK_A11Y_IDLE, copy::MARK_A11Y_IDLE_ONE),
            &values,
        ),
        tooltip: fill(copy::MARK_TOOLTIP_IDLE, &values),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()
    }

    fn batch(count: u64, tools: &[&str], days: u32) -> Batch {
        Batch {
            count,
            tools: tools.iter().map(|t| t.to_string()).collect(),
            idle_days: days,
            mission_fit: None,
            estimate: None,
        }
    }

    fn verdicts(a: u32, h: u32, tenths: Option<u64>) -> Verdicts {
        Verdicts {
            accepted: a,
            held: h,
            credit_final_tenths: tenths,
            since: date(),
        }
    }

    fn assert_filled(text: &str) {
        assert!(
            !text.contains('{') && !text.contains('}'),
            "placeholder left: {text}"
        );
    }

    fn card_filled(card: &CardText) {
        assert_filled(&card.title);
        assert_filled(&card.body);
        assert_filled(&card.panel_row);
        for a in &card.actions {
            assert_filled(&a.label);
        }
    }

    #[test]
    fn idle_card_reads_plural_and_singular() {
        let many = idle_card(&batch(3, &["Claude Code"], 3));
        assert_eq!(
            many.title,
            "3 traces from Claude Code have been idle for 3 days or more"
        );
        assert_eq!(
            many.panel_row,
            "Some traces have been idle for 3 days or more"
        );
        assert_eq!(
            many.actions,
            vec![
                action(ACTION_REVIEW, "Review"),
                action(ACTION_NOT_NOW, "Not now")
            ]
        );
        let one = idle_card(&batch(1, &["Codex"], 1));
        assert_eq!(
            one.title,
            "1 trace from Codex has been idle for 1 day or more"
        );
        assert_eq!(one.panel_row, "A trace has been idle for 1 day or more");
        assert!(one.body.starts_with("It looks finished."), "{}", one.body);
        card_filled(&many);
        card_filled(&one);
    }

    /// A batch whose words can name no tool (every source unnamed, or some
    /// of them) takes the tool-free templates: never "from 0 tools", and
    /// never one named tool credited with the others' sessions.
    #[test]
    fn a_batch_with_no_named_tool_reads_without_one() {
        let many = batch(2, &[], 3);
        let one = batch(1, &[], 1);
        assert_eq!(
            idle_card(&many).title,
            "2 traces have been idle for 3 days or more"
        );
        assert_eq!(
            idle_card(&one).title,
            "1 trace has been idle for 1 day or more"
        );
        assert_eq!(
            idle_notification(&many).body,
            "2 traces have been idle for 3 days or more. Review them to send or keep."
        );
        assert_eq!(
            idle_notification(&one).body,
            "1 trace has been idle for 1 day or more. Review it to send or keep."
        );
        assert_eq!(
            digest_idle_sentence(&many),
            "2 of them have been idle for 3 days or more."
        );
        assert_eq!(
            digest_idle_sentence(&one),
            "1 of them has been idle for 1 day or more."
        );
        for b in [many, one] {
            for text in [
                idle_card(&b).title,
                idle_card(&b).panel_row,
                idle_notification(&b).body,
                digest_idle_sentence(&b),
            ] {
                assert!(!text.contains("tools") && !text.contains("from"), "{text}");
                assert_filled(&text);
            }
        }
    }

    #[test]
    fn tools_are_named_one_two_or_counted() {
        assert_eq!(tool_phrase(&["Codex".into()]), "Codex");
        assert_eq!(
            tool_phrase(&["Codex".into(), "Claude Code".into()]),
            "Codex and Claude Code"
        );
        assert_eq!(
            tool_phrase(&["a".into(), "b".into(), "c".into()]),
            "3 tools"
        );
        assert_eq!(tool_phrase(&[]), "", "never \"0 tools\"");
        let long = "x".repeat(200);
        assert_eq!(tool_phrase(&[long]).chars().count(), TOOL_NAME_MAX_CHARS);
    }

    /// The mission clause appears only above zero, and the estimate clause
    /// only when drawn: the built-in band says nothing about this batch.
    #[test]
    fn body_clauses_say_only_what_is_true() {
        let mut b = batch(4, &["Codex"], 3);
        assert_eq!(idle_card(&b).body, copy::NUDGE_IDLE_BODY);
        b.mission_fit = Some(0);
        assert_eq!(idle_card(&b).body, copy::NUDGE_IDLE_BODY);
        b.mission_fit = Some(1);
        assert!(idle_card(&b).body.ends_with("1 of them fits a mission."));
        b.mission_fit = Some(2);
        assert!(idle_card(&b).body.ends_with("2 of them fit a mission."));
        b.estimate = Some(EstimateSum {
            low: 4.0,
            high: 12.0,
            known: 4,
            drawn: false,
        });
        assert!(!idle_card(&b).body.contains("Estimated"), "not drawn");
        b.estimate = Some(EstimateSum {
            low: 4.0,
            high: 12.5,
            known: 4,
            drawn: true,
        });
        assert!(
            idle_card(&b)
                .body
                .ends_with("Estimated credit before scoring: about 4 to 12.5."),
            "{}",
            idle_card(&b).body
        );
        b.estimate = Some(EstimateSum {
            low: 2.0,
            high: 6.0,
            known: 2,
            drawn: true,
        });
        assert!(
            idle_card(&b)
                .body
                .ends_with("Estimated credit for 2 of them, before scoring: about 2 to 6.")
        );
        card_filled(&idle_card(&b));
    }

    #[test]
    fn backlog_card_uses_the_plain_title_and_reads_singular() {
        let many = backlog_card(&batch(6, &[], 0));
        assert_eq!(many.title, "6 traces to review");
        assert_eq!(many.actions[0].label, "Review the 6 in Traces");
        let one = backlog_card(&batch(1, &[], 0));
        assert_eq!(one.title, "1 trace to review");
        assert_eq!(one.actions[0].label, "Review it in Traces");
        assert_eq!(one.panel_row, copy::NUDGE_PANEL_BACKLOG_ONE);
        card_filled(&many);
        card_filled(&one);
    }

    #[test]
    fn verdict_titles_drop_zero_clauses_and_never_go_empty() {
        assert!(copy::NUDGE_VERDICTS_TITLE.contains(VERDICTS_ACCEPTED_CLAUSE));
        assert!(copy::NUDGE_VERDICTS_TITLE.contains(VERDICTS_HELD_CLAUSE));
        assert_eq!(
            verdicts_card(&verdicts(2, 1, None)).title,
            "Since 8 October: 2 accepted, 1 held for privacy review"
        );
        assert_eq!(
            verdicts_card(&verdicts(2, 0, None)).title,
            "Since 8 October: 2 accepted"
        );
        assert_eq!(
            verdicts_card(&verdicts(0, 3, None)).title,
            "Since 8 October: 3 held for privacy review"
        );
        let finals = verdicts_card(&verdicts(0, 0, Some(25)));
        assert_eq!(finals.title, "Since 8 October: 2.5 credit is now final");
        assert_eq!(finals.body, "");
        assert_eq!(finals.panel_row, "2.5 credit is now final");
        let with_credit = verdicts_card(&verdicts(1, 0, Some(10)));
        assert_eq!(with_credit.body, "1.0 credit is now final.");
        assert_eq!(
            with_credit.actions,
            vec![action(ACTION_SEE_HISTORY, "See history")],
            "news has no Not now"
        );
        for v in [
            verdicts(2, 1, Some(3)),
            verdicts(0, 0, Some(3)),
            verdicts(1, 1, None),
        ] {
            card_filled(&verdicts_card(&v));
        }
    }

    #[test]
    fn notifications_and_digest_sentences_read_singular() {
        let n1 = idle_notification(&batch(1, &["Codex"], 3));
        assert_eq!(
            n1.body,
            "1 trace from Codex has been idle for 3 days or more. Review it to send or keep."
        );
        assert_eq!(n1.title, copy::NOTIFY_TITLE);
        let n2 = verdicts_notification(&verdicts(1, 2, Some(15)));
        assert_eq!(
            n2.body,
            "1 trace accepted and 2 held for privacy review. 1.5 credit is now final."
        );
        assert_eq!(
            verdicts_notification(&verdicts(0, 0, Some(15))).body,
            "1.5 credit is now final."
        );
        assert_eq!(
            digest_verdict_sentence(&verdicts(3, 0, None)),
            "3 traces accepted."
        );
        let mut b = batch(2, &["Codex", "Claude Code"], 2);
        b.mission_fit = Some(1);
        b.estimate = Some(EstimateSum {
            low: 2.0,
            high: 6.0,
            known: 2,
            drawn: true,
        });
        assert_eq!(
            digest_idle_sentence(&b),
            "2 of them, from Codex and Claude Code, have been idle for 2 days or more. \
             1 of them fits a mission."
        );
        for text in [n1.body, digest_idle_sentence(&b)] {
            assert_filled(&text);
        }
    }

    /// A zero count is never said: accepted-only news names no held, and
    /// held-only news names no accepted, in every renderer that reports
    /// verdicts (the notification, the digest fold, the mark and the panel
    /// row), each reading singular at one.
    #[test]
    fn verdict_renderers_drop_zero_clauses() {
        let cases = [
            ((3, 0, None), "3 traces accepted."),
            ((1, 0, None), "1 trace accepted."),
            ((0, 2, None), "2 traces held for privacy review."),
            ((0, 1, None), "1 trace held for privacy review."),
            (
                (1, 0, Some(15)),
                "1 trace accepted. 1.5 credit is now final.",
            ),
            (
                (0, 2, Some(15)),
                "2 traces held for privacy review. 1.5 credit is now final.",
            ),
            (
                (2, 1, None),
                "2 traces accepted and 1 held for privacy review.",
            ),
        ];
        for ((a, h, x), want) in cases {
            let v = verdicts(a, h, x);
            assert_eq!(verdicts_notification(&v).body, want, "N2 {a}/{h}");
            assert_eq!(digest_verdict_sentence(&v), want, "fold {a}/{h}");
        }
        assert_eq!(
            mark_news_text(&verdicts(2, 0, None)).accessibility,
            "New: 2 accepted."
        );
        assert_eq!(
            mark_news_text(&verdicts(0, 3, None)).accessibility,
            "New: 3 held for privacy review."
        );
        assert_eq!(
            mark_news_text(&verdicts(2, 1, None)).accessibility,
            "New: 2 accepted and 1 held for privacy review."
        );
        assert_eq!(
            verdicts_card(&verdicts(2, 0, None)).panel_row,
            "2 accepted since 8 October"
        );
        assert_eq!(
            verdicts_card(&verdicts(0, 3, None)).panel_row,
            "3 held since 8 October"
        );
        assert_eq!(
            verdicts_card(&verdicts(2, 1, None)).panel_row,
            "2 accepted, 1 held since 8 October"
        );
        for (a, h) in [(4, 0), (0, 4), (1, 0), (0, 1)] {
            let v = verdicts(a, h, Some(5));
            for text in [
                verdicts_notification(&v).body,
                digest_verdict_sentence(&v),
                mark_news_text(&v).accessibility,
                verdicts_card(&v).panel_row,
                verdicts_card(&v).title,
            ] {
                assert!(!text.contains(" 0 ") && !text.starts_with("0 "), "{text}");
                assert_filled(&text);
            }
        }
    }

    #[test]
    fn mark_text_fills_every_placeholder() {
        let ready = mark_ready_text(&batch(1, &[], 1));
        assert_eq!(
            ready.accessibility,
            "1 of them has been idle for 1 day or more."
        );
        assert_eq!(ready.tooltip, "1 idle for 1 day or more.");
        let news = mark_news_text(&verdicts(2, 0, None));
        assert_eq!(news.accessibility, "New: 2 accepted.");
        let finals = mark_news_text(&verdicts(0, 0, Some(40)));
        assert_eq!(finals.accessibility, "New: 4.0 credit is now final.");
    }

    #[test]
    fn figures_and_dates_are_formatted_once() {
        assert_eq!(credit_text(0), "0.0");
        assert_eq!(credit_text(25), "2.5");
        assert_eq!(credit_text(1234), "123.4");
        assert_eq!(band_text(1.0), "1");
        assert_eq!(band_text(2.5), "2.5");
        assert_eq!(band_text(12.0), "12");
        assert_eq!(date_text(date()), "8 October");
    }
}
