//! The inspector with nothing selected: the GTK port of the Mac's
//! `SummaryInspector` (`macos/.../Views/Monitor/SummaryInspector.swift`).
//!
//! What it draws, in the Mac's order: the header ("Summary" over the tools
//! watched, folders and sessions waiting), the shared and kept legend
//! cells, the Decisions card (waiting, contributed, pending credit), the
//! Statistics card (top projects and tools), and the safeguards card.
//!
//! Every word is the core's and every count the core's; a count the core
//! did not give, or gave from a read that failed, is a dash, never zero
//! (`SummaryFacts`). The pure rules are here, tested against the Mac's own
//! `SummaryInspectorTests` cases; [`view`] draws them.
//!
//! Not drawn here, unlike the Mac: the certificate list and the
//! no-longer-waiting disclosure (the Queue screen still draws both on the
//! same tab, until G12 replaces it), the rollup-failure notice (a notice
//! this shell does not carry), the statistics' tool tiles, and the
//! safeguards' unreadable-rows line (this shell's status model does not
//! read `routing.unreadable_rows`).

use adw::prelude::*;
use trace_commons_contributor::history_copy::{count_figure, credit_figure};
use trace_commons_contributor::preview_copy::{
    MonitorCountsCopy, MonitorSafeguardsCopy, monitor_screens_copy, monitor_traces_copy,
};

use super::graph_footer::{Series, legend_cell};
use super::traces_tree::SourceKind;
use crate::model::{HistoryRecord, Project, QueueEntry, RoutingStatus, Status};
use crate::ui::style::brand_tokens::glass::space;

/// What a count the core did not give reads as.
pub const DASH: &str = "\u{2014}";

/// The history statuses counted as contributed (`SummaryFacts
/// .contributedStatuses`): in the commons, or waiting to be scored. Held
/// for privacy review is its own figure; withdrawn and expired are not
/// counted. Not `graph_model::CONTRIBUTED_STATUSES`, which is the graph's.
pub const CONTRIBUTED_STATUSES: [&str; 3] = ["accepted", "submitted", "processing"];

/// A count, grouped by the core's rule, or a dash when not given.
pub fn count(value: Option<u64>) -> String {
    value.map_or_else(|| DASH.to_owned(), count_figure)
}

/// Decisions owed, the badge's count; never `queue_depth`.
pub fn waiting(status: Option<&Status>) -> String {
    count(status.and_then(|status| status.decisions_owed))
}

/// Sessions waiting in the tree; a dash before the tree is read.
pub fn kept(loaded: bool, sessions: usize) -> String {
    if loaded {
        count(Some(sessions as u64))
    } else {
        DASH.to_owned()
    }
}

/// The core's Contributed figure from a rollup read that worked, read and
/// never re-added; a dash when unread, failed, or from an older daemon.
pub fn contributed(rollup: Option<&crate::home::Rollup>) -> String {
    count(
        rollup
            .and_then(|rollup| rollup.all_time.as_ref())
            .and_then(|all| all.contributed)
            .map(u64::from),
    )
}

/// The commons' statement of what pending credit waits on
/// (`HomeFormat.pendingCondition`): only from a known posture, and only
/// when it says something.
pub fn pending_condition(summary: &serde_json::Value) -> Option<String> {
    if summary
        .get("posture_state")
        .and_then(|value| value.as_str())
        != Some("known")
    {
        return None;
    }
    summary
        .get("commons_settlement_explanation")
        .and_then(|value| value.as_str())
        .filter(|explanation| !explanation.is_empty())
        .map(str::to_owned)
}

/// Pending credit (D6): a figure only beside the commons' condition, a
/// dash without it or without a rollup.
pub fn pending(rollup: Option<&crate::home::Rollup>, condition: Option<&str>) -> String {
    match (condition, rollup.and_then(|rollup| rollup.credit_pending)) {
        (Some(_), Some(figure)) => credit_figure(f64::from(figure)),
        _ => DASH.to_owned(),
    }
}

fn fill(template: &str, count: &str) -> String {
    template.replace("{count}", count)
}

/// The header's sub line: tools watched, folders, sessions waiting. A part
/// the core has not said is left out, never a dash.
pub fn subline(
    watched: Option<(usize, usize)>,
    loaded: bool,
    folders: usize,
    sessions: usize,
) -> String {
    let words = monitor_traces_copy();
    let mut parts = Vec::new();
    if let Some((watched, total)) = watched {
        parts.push(
            words
                .summary_panel
                .tools_watched
                .replace("{count}", &watched.to_string())
                .replace("{total}", &total.to_string()),
        );
    }
    if loaded {
        let counts = &words.counts;
        parts.push(if folders == 1 {
            counts.project_count_one.to_owned()
        } else {
            fill(counts.project_count, &folders.to_string())
        });
        parts.push(if sessions == 1 {
            counts.sessions_waiting_one.to_owned()
        } else {
            fill(counts.sessions_waiting, &sessions.to_string())
        });
    }
    parts.join(" \u{b7} ")
}

/// The history page the statistics count: none when unread, or when the
/// daemon filled the page (`list_history`'s limit), which is not a whole
/// count (`SummaryFacts.wholeHistory`).
pub fn whole_history(history: Option<&[HistoryRecord]>) -> Option<&[HistoryRecord]> {
    history.filter(|rows| rows.len() < crate::ui::history::HISTORY_LIMIT)
}

/// A tool's name (`InferenceTabView.toolName`): the known tools by name,
/// anything else as the core gave it.
pub fn tool_name(id: &str) -> String {
    match SourceKind::from_wire(id) {
        Some(kind) => kind.display_name().to_owned(),
        None if id == "unknown" => DASH.to_owned(),
        None => id.to_owned(),
    }
}

/// A statistic's line: a project or a tool, and its counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stat {
    pub id: String,
    pub title: String,
    pub sub: String,
}

fn contributed_row(row: &HistoryRecord) -> bool {
    CONTRIBUTED_STATUSES.contains(&row.status.as_str())
}

/// The two folders with the most waiting plus contributed, with the
/// folder's mode word. Contributed is a dash when history is not whole,
/// and the ranking is then on waiting alone.
pub fn top_projects(
    folders: &[crate::queue_folders::Folder],
    projects: &[Project],
    history: Option<&[HistoryRecord]>,
    counts: &MonitorCountsCopy,
) -> Vec<Stat> {
    let history = whole_history(history);
    let mut ranked: Vec<(&crate::queue_folders::Folder, Option<usize>)> = folders
        .iter()
        .map(|folder| {
            let contributed = history.map(|rows| {
                rows.iter()
                    .filter(|row| row.project_id == folder.project_id && contributed_row(row))
                    .count()
            });
            (folder, contributed)
        })
        .filter(|(folder, contributed)| folder.members.len() + contributed.unwrap_or(0) > 0)
        .collect();
    ranked.sort_by(|(a, a_count), (b, b_count)| {
        let left = a.members.len() + a_count.unwrap_or(0);
        let right = b.members.len() + b_count.unwrap_or(0);
        right
            .cmp(&left)
            .then_with(|| a.project_label.cmp(&b.project_label))
    });
    let modes = trace_commons_contributor::project_copy::contribution_mode_copy();
    ranked
        .into_iter()
        .take(2)
        .map(|(folder, contributed)| {
            let mut parts = vec![
                fill(
                    counts.waiting_count,
                    &count(Some(folder.members.len() as u64)),
                ),
                fill(
                    counts.contributed_count,
                    &count(contributed.map(|n| n as u64)),
                ),
            ];
            let mode = projects
                .iter()
                .find(|project| project.project_id == folder.project_id)
                .and_then(|project| {
                    modes
                        .choices
                        .iter()
                        .find(|choice| choice.mode == project.mode)
                })
                .map(|choice| choice.label);
            if let Some(word) = mode.filter(|word| !word.is_empty()) {
                parts.push(word.to_owned());
            }
            Stat {
                id: folder.project_id.clone(),
                title: folder.project_label.clone(),
                sub: parts.join(" \u{b7} "),
            }
        })
        .collect()
}

/// The two tools with the most waiting plus contributed, each session
/// under its own tool. Contributed is a dash when history is not whole.
pub fn top_tools(
    sessions: &[&QueueEntry],
    history: Option<&[HistoryRecord]>,
    counts: &MonitorCountsCopy,
) -> Vec<Stat> {
    use std::collections::BTreeMap;
    let history = whole_history(history);
    let mut waiting: BTreeMap<String, usize> = BTreeMap::new();
    for session in sessions {
        let tool = session
            .declared_source
            .clone()
            .unwrap_or_else(|| session.source.clone());
        *waiting.entry(tool).or_default() += 1;
    }
    let mut contributed: BTreeMap<String, usize> = BTreeMap::new();
    for row in history.unwrap_or_default() {
        if let Some(source) = row.source.as_ref().filter(|_| contributed_row(row)) {
            *contributed.entry(source.clone()).or_default() += 1;
        }
    }
    let mut tools: Vec<(String, usize)> = waiting
        .keys()
        .chain(contributed.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|tool| {
            let total = waiting.get(tool).copied().unwrap_or(0)
                + contributed.get(tool).copied().unwrap_or(0);
            (tool.clone(), total)
        })
        .filter(|(_, total)| *total > 0)
        .collect();
    tools.sort_by(|(a, a_total), (b, b_total)| {
        b_total
            .cmp(a_total)
            .then_with(|| tool_name(a).cmp(&tool_name(b)))
    });
    tools
        .into_iter()
        .take(2)
        .map(|(tool, _)| Stat {
            title: tool_name(&tool),
            sub: [
                fill(
                    counts.waiting_count,
                    &waiting.get(&tool).copied().unwrap_or(0).to_string(),
                ),
                fill(
                    counts.contributed_count,
                    &if history.is_none() {
                        DASH.to_owned()
                    } else {
                        count(Some(contributed.get(&tool).copied().unwrap_or(0) as u64))
                    },
                ),
            ]
            .join(" \u{b7} "),
            id: tool,
        })
        .collect()
}

/// The routing cell's short state (`MonitorSafeguardsCopy.routingLabel`).
pub fn routing_label(state: &str, copy: &MonitorSafeguardsCopy) -> &'static str {
    match state {
        "not_declared" => copy.routing_not_declared,
        "awaiting_rows" => copy.routing_awaiting_rows,
        "rows_seen" => copy.routing_rows_seen,
        "token_unreadable" => copy.routing_token_unreadable,
        _ => copy.routing_unknown,
    }
}

/// The safeguards' rows: inference routing in the core's state word, and
/// that the daemon owns it when it derived it.
pub fn safeguard_rows(
    routing: &RoutingStatus,
    copy: &MonitorSafeguardsCopy,
) -> Vec<(&'static str, String)> {
    let state = routing_label(&routing.state, copy);
    let value = if routing.derived {
        format!("{state} \u{b7} {}", copy.daemon_owned)
    } else {
        state.to_owned()
    };
    vec![(copy.inference_routing, value)]
}

/// A decision line's glyph and its ink (`SummaryDecisionLine`): the Mac's
/// SF Symbols as stock symbolic icons (Adwaita 46 on, and Breeze).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    /// `rectangle.split.2x1` in the ask tone: waiting for you.
    Waiting,
    /// `checkmark` in the on tone: contributed.
    Contributed,
    /// `hourglass`, tertiary: pending credit.
    Pending,
}

impl Line {
    pub fn icon(self) -> &'static str {
        match self {
            Line::Waiting => "view-dual-symbolic",
            Line::Contributed => "object-select-symbolic",
            Line::Pending => "document-open-recent-symbolic",
        }
    }

    pub fn css(self) -> &'static str {
        match self {
            Line::Waiting => "tc-summary-ask",
            Line::Contributed => "tc-summary-on",
            Line::Pending => "tc-summary-tertiary",
        }
    }
}

/// Everything the summary draws from, read from the window's last answers.
#[derive(Default)]
pub struct Inputs {
    pub status: Option<Status>,
    /// Whether `list_pending` answered.
    pub loaded: bool,
    pub pending: Vec<QueueEntry>,
    pub projects: Vec<Project>,
    /// The last `history_rollup` read that worked; `None` unread or failed.
    pub rollup: Option<crate::home::Rollup>,
    /// The commons' pending condition, from `commons_credit_summary`.
    pub condition: Option<String>,
    pub history: Option<Vec<HistoryRecord>>,
    /// `(watched, total)` from `tool_destinations`, when it was read.
    pub watched: Option<(usize, usize)>,
}

fn text(words: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(words)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .build();
    for class in classes {
        label.add_css_class(class);
    }
    label
}

/// Ron's inspector `Section`: a quiet card with an expander heading over
/// its content, open by default.
fn section(title: &str, content: &impl IsA<gtk::Widget>) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("tc-glass-card-quiet");
    card.add_css_class("tc-summary-section");
    let expander = gtk::Expander::builder().expanded(true).build();
    let label = text(title, &["tc-type-label", "tc-summary-section-title"]);
    expander.set_label_widget(Some(&label));
    let inner = content.as_ref();
    inner.set_margin_top(space::S3);
    inner.set_margin_start(space::S2);
    inner.set_margin_end(space::S2);
    expander.set_child(Some(inner));
    card.append(&expander);
    card
}

fn decision_line(line: Line, words: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, space::S5);
    let icon = gtk::Image::from_icon_name(line.icon());
    icon.set_pixel_size(12);
    icon.add_css_class(line.css());
    icon.set_size_request(20, -1);
    icon.set_accessible_role(gtk::AccessibleRole::Presentation);
    row.append(&icon);
    row.append(&text(words, &["tc-type-body", "tc-summary-ink"]));
    row
}

/// An eyebrow: the type role is uppercase on the Mac.
fn eyebrow(words: &str) -> gtk::Label {
    text(
        &words.to_uppercase(),
        &["tc-type-eyebrow", "tc-summary-tertiary"],
    )
}

/// The summary, drawn from `inputs`.
pub fn view(inputs: &Inputs) -> gtk::Box {
    let screens = monitor_screens_copy();
    let words = monitor_traces_copy();
    let pending: Vec<&QueueEntry> = inputs.pending.iter().collect();
    let folders = crate::queue_folders::group(&pending);
    let root = gtk::Box::new(gtk::Orientation::Vertical, space::CARD_GAP);
    root.add_css_class("tc-summary");

    // InspectorHeader: the title, and the sub line when it says anything.
    let header = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let title = text(screens.summary, &["tc-type-heading", "tc-summary-title"]);
    title.set_wrap(false);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_accessible_role(gtk::AccessibleRole::Heading);
    header.append(&title);
    let sub = subline(inputs.watched, inputs.loaded, folders.len(), pending.len());
    if !sub.is_empty() {
        let sub = text(&sub, &["tc-type-caption", "tc-summary-tertiary"]);
        sub.set_wrap(false);
        sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        header.append(&sub);
    }
    root.append(&header);

    // The legend: shared is the core's contributed, kept is the tree's.
    let legend = gtk::Box::new(gtk::Orientation::Horizontal, space::S3);
    legend.set_homogeneous(true);
    let (shared_cell, shared_value) = legend_cell(screens.shared, Series::Shared);
    shared_value.set_text(&contributed(inputs.rollup.as_ref()));
    let (kept_cell, kept_value) = legend_cell(screens.kept, Series::Kept);
    kept_value.set_text(&kept(inputs.loaded, pending.len()));
    legend.append(&shared_cell);
    legend.append(&kept_cell);
    root.append(&legend);

    // Decisions: waiting, contributed, and pending credit, in Ron's order.
    let decisions = gtk::Box::new(gtk::Orientation::Vertical, space::S4);
    decisions.append(&decision_line(
        Line::Waiting,
        &fill(
            words.summary_panel.waiting_for_you,
            &waiting(inputs.status.as_ref()),
        ),
    ));
    decisions.append(&decision_line(
        Line::Contributed,
        &fill(
            words.counts.contributed_count,
            &contributed(inputs.rollup.as_ref()),
        ),
    ));
    decisions.append(&decision_line(
        Line::Pending,
        &format!(
            "{} {}",
            screens.pending,
            self::pending(inputs.rollup.as_ref(), inputs.condition.as_deref())
        ),
    ));
    root.append(&section(words.inspector.decisions, &decisions));

    // Statistics, once the tree is read and there is anything to rank.
    if inputs.loaded {
        let history = inputs.history.as_deref();
        let projects = top_projects(&folders, &inputs.projects, history, &words.counts);
        let tools = top_tools(&pending, history, &words.counts);
        if !projects.is_empty() || !tools.is_empty() {
            let groups = gtk::Box::new(gtk::Orientation::Vertical, space::S8);
            for (label, lines) in [
                (words.summary_panel.top_projects, &projects),
                (words.summary_panel.top_tools, &tools),
            ] {
                if lines.is_empty() {
                    continue;
                }
                let group = gtk::Box::new(gtk::Orientation::Vertical, space::S3);
                group.append(&eyebrow(label));
                let list = gtk::Box::new(gtk::Orientation::Vertical, space::S5);
                for line in lines {
                    let item = gtk::Box::new(gtk::Orientation::Vertical, space::S1);
                    let name = text(&line.title, &["tc-type-label", "tc-summary-stat-title"]);
                    name.set_wrap(false);
                    name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                    item.append(&name);
                    item.append(&text(
                        &line.sub,
                        &["tc-type-caption", "tc-summary-secondary"],
                    ));
                    list.append(&item);
                }
                group.append(&list);
                groups.append(&group);
            }
            root.append(&section(words.summary_panel.statistics, &groups));
        }
    }

    // The safeguards: inference routing, once status has answered.
    if let Some(status) = inputs.status.as_ref() {
        let copy = &screens.safeguards;
        let card = gtk::Box::new(gtk::Orientation::Vertical, space::S4);
        card.add_css_class("tc-glass-card");
        card.add_css_class("tc-summary-card");
        card.append(&eyebrow(copy.eyebrow));
        let body = gtk::Box::new(gtk::Orientation::Vertical, space::S3);
        body.append(&text(
            copy.heading,
            &["tc-type-body-strong", "tc-summary-ink"],
        ));
        let grid = gtk::Box::new(gtk::Orientation::Horizontal, space::S4);
        grid.set_homogeneous(true);
        grid.set_margin_top(space::S2);
        grid.set_margin_bottom(space::S2);
        for (label, value) in safeguard_rows(&status.routing, copy) {
            let cell = gtk::Box::new(gtk::Orientation::Vertical, space::S1);
            cell.append(&text(label, &["tc-type-body-strong", "tc-summary-ink"]));
            cell.append(&text(&value, &["tc-type-caption", "tc-summary-secondary"]));
            grid.append(&cell);
        }
        body.append(&grid);
        card.append(&body);
        root.append(&card);
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rollup(value: serde_json::Value) -> crate::home::Rollup {
        serde_json::from_value(value).expect("a rollup")
    }

    /// `test_pendingCreditFollowsD6`: pending credit is a figure only beside
    /// the commons' statement of what it waits on; a dash otherwise, and a
    /// dash with a statement but no rollup.
    #[test]
    fn pending_credit_follows_d6() {
        let with_credit = rollup(serde_json::json!({"credit_pending": 12.5}));
        let known = serde_json::json!({
            "posture_state": "known",
            "commons_settlement": "pending_review",
            "commons_settlement_explanation": "Scored when the commons reviews it.",
        });
        let condition = pending_condition(&known);
        assert_eq!(
            condition.as_deref(),
            Some("Scored when the commons reviews it.")
        );
        assert_eq!(
            pending(Some(&with_credit), condition.as_deref()),
            credit_figure(12.5)
        );
        let unknown = serde_json::json!({
            "posture_state": "unknown",
            "commons_settlement_explanation": "Scored when the commons reviews it.",
        });
        assert_eq!(pending_condition(&unknown), None);
        assert_eq!(pending(Some(&with_credit), None), DASH);
        assert_eq!(pending(None, condition.as_deref()), DASH);
        let silent =
            serde_json::json!({"posture_state": "known", "commons_settlement_explanation": ""});
        assert_eq!(pending_condition(&silent), None);
    }

    /// `test_aMissingCountIsADash`: a count the core did not give is a
    /// dash, never zero and never derived from something else.
    #[test]
    fn a_missing_count_is_a_dash() {
        let status: Status =
            serde_json::from_value(serde_json::json!({"decisions_owed": 3, "queue_depth": 9}))
                .expect("a status");
        assert_eq!(waiting(Some(&status)), "3");
        let unsaid: Status =
            serde_json::from_value(serde_json::json!({"queue_depth": 9})).expect("a status");
        assert_eq!(waiting(Some(&unsaid)), DASH, "never queue_depth");
        assert_eq!(waiting(None), DASH);

        // Before the tree is read nothing waiting is known.
        assert_eq!(kept(false, 1), DASH);
        assert_eq!(kept(true, 2), "2");

        // Contributed is the core's figure, never summed here.
        assert_eq!(contributed(None), DASH);
        let older = rollup(serde_json::json!({"all_time": {"accepted": 2, "submitted": 1}}));
        assert_eq!(
            contributed(Some(&older)),
            DASH,
            "an older daemon's rollup is not summed"
        );
        let mixed = rollup(serde_json::json!({"all_time": {
            "submitted": 1, "accepted": 56, "quarantined": 3, "contributed": 57
        }}));
        assert_eq!(contributed(Some(&mixed)), "57");
        assert_eq!(count(Some(1234)), "1,234");

        // The tool count only when the core said which tools it reads.
        let words = monitor_traces_copy();
        let unread = subline(None, true, 2, 1);
        assert!(!unread.contains(DASH) && !unread.contains('{'), "{unread}");
        assert!(
            unread.ends_with(words.counts.sessions_waiting_one),
            "{unread}"
        );
        let read = subline(Some((2, 5)), true, 2, 1);
        let watched = words
            .summary_panel
            .tools_watched
            .replace("{count}", "2")
            .replace("{total}", "5");
        assert!(read.starts_with(&watched), "{read}");
        // An unread tree counts nothing.
        assert_eq!(subline(None, false, 0, 0), "");
    }

    fn records(count: usize, project: &str, source: &str, status: &str) -> Vec<HistoryRecord> {
        (0..count)
            .map(|index| {
                serde_json::from_value(serde_json::json!({
                    "submission_id": format!("row-{index}"),
                    "project_id": project,
                    "project_label": "api",
                    "source": source,
                    "status": status,
                }))
                .expect("a history row")
            })
            .collect()
    }

    fn session(project: &str, source: &str) -> QueueEntry {
        QueueEntry {
            entry_id: "s1".into(),
            project_id: project.into(),
            project_label: "api".into(),
            source: source.into(),
            declared_source: Some(source.into()),
            state: "pending".into(),
            ..Default::default()
        }
    }

    /// `test_aCappedHistoryCountsNoContributed`: a page the daemon filled
    /// is not a whole count, so contributed is a dash and the ranking falls
    /// back to waiting.
    #[test]
    fn a_capped_history_counts_no_contributed() {
        let counts = monitor_traces_copy().counts;
        let limit = crate::ui::history::HISTORY_LIMIT;
        let entry = session("p1", "claude-code");
        let pending = vec![&entry];
        let folders = crate::queue_folders::group(&pending);
        let contributed = |n: &str| fill(counts.contributed_count, n);

        let whole = records(limit - 1, "p1", "claude-code", "accepted");
        let project = &top_projects(&folders, &[], Some(&whole), &counts)[0];
        assert!(
            project.sub.contains(&contributed(&(limit - 1).to_string())),
            "{}",
            project.sub
        );
        let tool = &top_tools(&pending, Some(&whole), &counts)[0];
        assert!(
            tool.sub.contains(&contributed(&(limit - 1).to_string())),
            "{}",
            tool.sub
        );
        assert_eq!(tool.title, "Claude Code");

        let capped = records(limit, "p1", "claude-code", "accepted");
        let project = &top_projects(&folders, &[], Some(&capped), &counts)[0];
        assert!(project.sub.contains(&contributed(DASH)), "{}", project.sub);
        let tool = &top_tools(&pending, Some(&capped), &counts)[0];
        assert!(tool.sub.contains(&contributed(DASH)), "{}", tool.sub);

        // A tool with only capped history and nothing waiting is not ranked.
        let elsewhere = records(limit, "p2", "codex", "accepted");
        let ranked = top_tools(&pending, Some(&elsewhere), &counts);
        assert_eq!(
            ranked
                .iter()
                .map(|stat| stat.id.as_str())
                .collect::<Vec<_>>(),
            ["claude-code"]
        );
    }

    /// Contributed counts the core's statuses: processing is waiting to be
    /// scored and counts; held for review and withdrawn do not.
    #[test]
    fn contributed_counts_accepted_submitted_and_processing() {
        let counts = monitor_traces_copy().counts;
        let mut history = records(2, "p1", "codex", "processing");
        history.extend(records(1, "p1", "codex", "quarantined"));
        history.extend(records(1, "p1", "codex", "withdrawn"));
        history.extend(records(1, "p1", "codex", "accepted"));
        let ranked = top_tools(&[], Some(&history), &counts);
        assert_eq!(ranked.len(), 1);
        assert!(
            ranked[0].sub.contains(&fill(counts.contributed_count, "3")),
            "{}",
            ranked[0].sub
        );
    }

    /// The routing cell's word (`routingLabel`), and the daemon-owned tail
    /// when the daemon derived the state.
    #[test]
    fn the_safeguards_say_routing_in_the_cores_words() {
        let copy = monitor_screens_copy().safeguards;
        for (state, word) in [
            ("not_declared", copy.routing_not_declared),
            ("awaiting_rows", copy.routing_awaiting_rows),
            ("rows_seen", copy.routing_rows_seen),
            ("token_unreadable", copy.routing_token_unreadable),
            ("something_new", copy.routing_unknown),
        ] {
            let routing = RoutingStatus {
                state: state.into(),
                ..Default::default()
            };
            assert_eq!(
                safeguard_rows(&routing, &copy),
                [(copy.inference_routing, word.to_owned())]
            );
        }
        let derived = RoutingStatus {
            state: "rows_seen".into(),
            derived: true,
            ..Default::default()
        };
        assert_eq!(
            safeguard_rows(&derived, &copy)[0].1,
            format!("{} \u{b7} {}", copy.routing_rows_seen, copy.daemon_owned)
        );
    }

    /// The decision lines' glyphs, the Mac's SF Symbols as stock icons of
    /// the same meaning: two panes, a check mark, a clock.
    #[test]
    fn the_decision_glyphs_are_stock_icons() {
        assert_eq!(Line::Waiting.icon(), "view-dual-symbolic");
        assert_eq!(Line::Contributed.icon(), "object-select-symbolic");
        assert_eq!(Line::Pending.icon(), "document-open-recent-symbolic");
    }

    /// Mounted: the summary's cards, in the Mac's order, its legend dashes
    /// before anything is read, and its cards on the glass tiers that dim
    /// with the pane.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_summary_is_the_macs_cards() {
        adw::init().expect("GTK display");
        let screens = monitor_screens_copy();
        let words = monitor_traces_copy();
        let unread = view(&Inputs::default());
        let mut labels = Vec::new();
        let mut child = unread.first_child();
        let mut order = Vec::new();
        while let Some(widget) = child {
            order.push(widget.clone());
            child = widget.next_sibling();
        }
        // Header, legend, Decisions; no statistics or safeguards unread.
        assert_eq!(order.len(), 3);
        assert!(order[2].has_css_class("tc-glass-card-quiet"));
        fn collect(widget: &gtk::Widget, labels: &mut Vec<String>) {
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                labels.push(label.text().to_string());
            }
            let mut child = widget.first_child();
            while let Some(next) = child {
                collect(&next, labels);
                child = next.next_sibling();
            }
        }
        collect(unread.upcast_ref(), &mut labels);
        assert_eq!(labels.first().map(String::as_str), Some(screens.summary));
        assert!(
            labels.iter().filter(|label| *label == DASH).count() >= 2,
            "{labels:?}"
        );
        assert!(labels.contains(&fill(words.summary_panel.waiting_for_you, DASH)));

        let status: Status =
            serde_json::from_value(serde_json::json!({"decisions_owed": 2})).expect("a status");
        let entry = session("p1", "codex");
        let read = view(&Inputs {
            status: Some(status),
            loaded: true,
            pending: vec![entry],
            ..Inputs::default()
        });
        let mut count = 0;
        let mut child = read.first_child();
        while let Some(widget) = child {
            count += 1;
            child = widget.next_sibling();
        }
        // With the tree and status read: statistics and safeguards too.
        assert_eq!(count, 5);
        assert!(
            read.last_child()
                .expect("safeguards")
                .has_css_class("tc-glass-card")
        );
    }
}
