//! The Insights tabs over saved snapshots (feed S): Overview ("This week"),
//! Patterns ("Where tokens went") and Sessions (the drill-in), beside
//! Analyze, which keeps the whole earlier screen. The same tabs, terms and
//! states as the macOS window, read through the same service operations
//! (`week_overview`, `card_inputs`, `patterns`, `pattern_sessions`,
//! `session_drill`), drawn with simple bars.
//!
//! Every word is the core's `ui_copy()` analytics table (DRAFT, NEEDS
//! APPROVAL, owner decision D17, open) and every figure is the core's. This
//! module fills `{name}` holes and composes no sentence. An unknown figure is
//! the core's dash, never zero, and a failed read clears the figures rather
//! than keep stale ones.
//!
//! The rulings this follows, all open: bars, never a ring, and no run of
//! weeks is counted (owner decision D1); no advice, what-if or tip is drawn
//! (owner decision D2); no money (owner decision D5); By project is not
//! available for analyzed files (owner decision D7); a file is a letter and
//! an extension, never a name (owner decision D8); "Edit, failed command,
//! edit" is labelled inferred (owner decision D9); a Codex session has no
//! turn series (owner decision D11). Feed S compares no weeks, so every
//! change reads as the dash; this shell shows feed S only and says so.
use super::label;
use adw::prelude::*;
use serde::Serialize;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    path::PathBuf,
    rc::Rc,
};
use trace_commons_contributor::insights::{
    markers::MarkerKind,
    patterns::PatternKind,
    service::{
        self, LocalInsightsOperation as Op, LocalInsightsRequest, LocalInsightsResponse as Response,
    },
    session_drill::{DrillMarker, MarkerBasis, SeriesUnavailable, SessionDrill},
    week_glance::{CardInputs, OverviewCard, OverviewSource, ShareFigure, WeekOverview},
    week_patterns::{PatternBasis, PatternCard, PatternSessions, WeekPatterns},
    week_rollup::{AnalyticsSource, CoverageReason, CoverageState, Feed, WeekCoverage},
};

fn table() -> &'static BTreeMap<String, String> {
    static COPY: std::sync::OnceLock<BTreeMap<String, String>> = std::sync::OnceLock::new();
    COPY.get_or_init(service::ui_copy)
}

/// The core's word for `key`; the dash when the core has none, so an
/// unknown wire label never shows as a key or as nothing.
fn text(key: &str) -> String {
    table().get(key).cloned().unwrap_or_else(dash)
}

/// A wire label, as the core spells it, for building a copy key.
fn wire(value: impl Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The core's figure for "not shown": unknown, or not computed under this
/// feed.
pub(super) fn dash() -> String {
    table()
        .get("analytics_unavailable")
        .cloned()
        .unwrap_or_default()
}

/// Fill a core template's `{name}` holes. Nothing else is added.
pub(super) fn fill(template: &str, holes: &[(&str, &str)]) -> String {
    holes
        .iter()
        .fold(template.to_owned(), |filled, (name, value)| {
            filled.replace(&format!("{{{name}}}"), value)
        })
}

/// A count with digit grouping, or the dash for unknown. A measured zero
/// stays zero.
pub(super) fn figure(value: Option<u64>) -> String {
    let Some(value) = value else { return dash() };
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

fn count(value: Option<u32>) -> String {
    figure(value.map(u64::from))
}

/// Whole percent from the core's per mille, rounded half up; the dash for
/// unknown.
pub(super) fn share(share: Option<&ShareFigure>) -> String {
    share.map_or_else(dash, |share| ((share.permille + 5) / 10).to_string())
}

pub(super) fn coverage_line(coverage: &WeekCoverage) -> String {
    let sessions = coverage.known + coverage.partial + coverage.unknown;
    fill(
        &text("analytics_coverage_line"),
        &[
            ("k", &coverage.known.to_string()),
            ("n", &sessions.to_string()),
            ("p", &coverage.partial.to_string()),
            ("u", &coverage.unknown.to_string()),
        ],
    )
}

/// Which feed is showing. Feed S also says that weeks are not compared.
pub(super) fn feed_lines(feed: Feed) -> Vec<String> {
    match feed {
        Feed::Saved => vec![
            text("analytics_feed_saved"),
            text("analytics_feed_comparisons_need_counter_pass"),
        ],
        Feed::CounterPass => vec![text("analytics_feed_counter_pass")],
        Feed::Ledger => vec![text("analytics_feed_ledger")],
    }
}

pub(super) fn reason(reason: CoverageReason) -> String {
    text(&format!("analytics_reason_{}", wire(reason)))
}

fn reasons(list: &[CoverageReason]) -> String {
    list.iter()
        .map(|item| reason(*item))
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

pub(super) fn state(state: CoverageState) -> String {
    text(&format!("analytics_state_{}", wire(state)))
}

/// The harness's name, or the dash when the session's source is unknown.
pub(super) fn harness(source: Option<AnalyticsSource>) -> String {
    source.map_or_else(dash, |source| text(&wire(source)))
}

pub(super) fn source_line(source: AnalyticsSource) -> String {
    text(&format!("analytics_source_{}", wire(source)))
}

/// "vs last week" and "Your best week". The core sends a figure only under
/// feed T; an overview source carries only the reason there is none, so
/// both read as the dash.
pub(super) fn change(_source: &OverviewSource) -> String {
    dash()
}

pub(super) fn cache_share_line(source: &OverviewSource) -> String {
    fill(
        &text("analytics_cache_share_line"),
        &[
            ("source", &harness(Some(source.source))),
            ("p", &share(source.cache_share.as_ref())),
        ],
    )
}

pub(super) fn largest_line(source: &OverviewSource) -> Option<String> {
    let largest = source.largest_session.as_ref()?;
    Some(format!(
        "{} \u{b7} {}",
        harness(Some(source.source)),
        fill(
            &text("analytics_largest"),
            &[("t", &figure(Some(largest.tokens)))]
        )
    ))
}

/// By model, in the core's fixed order. Never sorted by value here.
pub(super) fn model_rows(overview: &WeekOverview) -> Vec<(String, String)> {
    overview
        .by_model
        .iter()
        .map(|row| {
            (
                row.label
                    .clone()
                    .unwrap_or_else(|| text("analytics_unknown_label")),
                figure(Some(row.tokens)),
            )
        })
        .collect()
}

/// Monday to Sunday, as ISO dates.
pub(super) fn week_range(start: chrono::NaiveDate) -> String {
    let end = start + chrono::Days::new(6);
    format!("{start} \u{2013} {end}")
}

/// The week picker's weeks: the core's dated weeks, newest first, with the
/// week on screen first when it holds no saved session.
pub(super) fn week_choices(
    weeks: &[chrono::NaiveDate],
    shown: chrono::NaiveDate,
) -> Vec<chrono::NaiveDate> {
    let mut choices = weeks.to_vec();
    if !choices.contains(&shown) {
        choices.insert(0, shown);
    }
    choices
}

pub(super) fn pattern_title(card: &PatternCard) -> String {
    pattern_name(card.kind)
}

fn pattern_name(kind: PatternKind) -> String {
    text(&format!("analytics_pattern_{}", wire(kind)))
}

/// The line under the headline: the core's count sentence, or the
/// long-context threshold line.
pub(super) fn count_line(card: &PatternCard, threshold: u64) -> String {
    match card.kind {
        PatternKind::RepeatedReads => fill(
            &text("analytics_pattern_repeated_reads_count"),
            &[("r", &count(card.count)), ("f", &count(card.files))],
        ),
        PatternKind::RetriedCalls => fill(
            &text("analytics_pattern_retried_calls_count"),
            &[("c", &count(card.count))],
        ),
        PatternKind::EditFailEdit => fill(
            &text("analytics_pattern_edit_fail_edit_count"),
            &[("l", &count(card.count))],
        ),
        PatternKind::LongContext => fill(
            &text("analytics_pattern_long_context_line"),
            &[("threshold", &figure(Some(threshold)))],
        ),
    }
}

/// How the figure was arrived at: the inferred label first, then the basis.
pub(super) fn basis_lines(card: &PatternCard) -> Vec<String> {
    let mut lines = Vec::new();
    if card.inferred {
        lines.push(text("analytics_inferred_from_order"));
    }
    lines.push(text(match card.basis {
        PatternBasis::EstimateFromResultSize => "analytics_estimate_from_result_size",
        PatternBasis::FromCounters => "analytics_from_counters",
    }));
    lines
}

/// "vs last week". Only feed T sends a figure; anything else is the dash.
pub(super) fn pattern_change(card: &PatternCard) -> String {
    let Some(permille) = card.change else {
        return dash();
    };
    let percent = ((permille.unsigned_abs() + 5) / 10).to_string();
    let key = if permille < 0 {
        "analytics_change_down"
    } else {
        "analytics_change_up"
    };
    fill(&text(key), &[("p", &percent)])
}

/// Every week the card covers, oldest first. `None` is a gap: no bar is
/// drawn for it, never a zero bar.
pub(super) fn weekly_bars(card: &PatternCard) -> Vec<Option<u64>> {
    card.weeks.iter().map(|week| week.tokens).collect()
}

pub(super) fn see_sessions(card: &PatternCard) -> Option<String> {
    (card.sessions > 0).then(|| {
        fill(
            &text("analytics_see_sessions"),
            &[("n", &card.sessions.to_string())],
        )
    })
}

/// "File B · .rs", or "File B" with no extension.
pub(super) fn file_label(letter: &str, ext: Option<&str>) -> String {
    match ext {
        Some(ext) => fill(
            &text("analytics_file_label"),
            &[("letter", letter), ("ext", ext)],
        ),
        None => fill(&text("analytics_file_label_no_ext"), &[("letter", letter)]),
    }
}

/// "Claude Code sessions only: {k} of {n}." when another harness is in the
/// week.
pub(super) fn claude_only_line(patterns: &WeekPatterns) -> Option<String> {
    patterns.claude_only.then(|| {
        fill(
            &text("analytics_claude_sessions_only"),
            &[
                ("k", &patterns.claude_sessions.to_string()),
                ("n", &patterns.sessions.to_string()),
            ],
        )
    })
}

/// Hours and minutes between the first and last event, as `h:mm`.
fn span(seconds: Option<u64>) -> String {
    seconds.map_or_else(dash, |seconds| {
        format!("{}:{:02}", seconds / 3600, (seconds % 3600) / 60)
    })
}

pub(super) fn session_header(drill: &SessionDrill) -> String {
    fill(
        &text("analytics_session_header"),
        &[
            (
                "date",
                &drill.date.map_or_else(dash, |date| date.to_string()),
            ),
            ("harness", &harness(drill.source)),
            ("n", &count(drill.turns)),
            ("t", &figure(drill.tokens)),
            ("span", &span(drill.span_secs)),
        ],
    )
}

/// Why there is no chart; `None` when there is one.
pub(super) fn series_unavailable_line(drill: &SessionDrill) -> Option<String> {
    Some(match drill.series_unavailable? {
        SeriesUnavailable::NotRecorded => text("analytics_codex_not_recorded"),
        other => text(&format!("analytics_reason_{}", wire(other))),
    })
}

/// One turn of "Input sent each turn": its label, the lettered markers at
/// it, and cache read, uncached and cache write, or `None` when any counter
/// of the turn is unknown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TurnRow {
    pub label: String,
    pub letters: String,
    pub cells: Option<[u64; 3]>,
}

pub(super) fn turn_rows(drill: &SessionDrill) -> Vec<TurnRow> {
    drill
        .series
        .iter()
        .flatten()
        .map(|turn| TurnRow {
            label: fill(&text("analytics_turn"), &[("n", &turn.ordinal.to_string())]),
            letters: drill
                .markers
                .iter()
                .filter(|marker| marker.turn_ordinal == turn.ordinal)
                .map(|marker| marker.letter.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            cells: match (turn.cache_read, turn.uncached, turn.cache_write) {
                (Some(read), Some(uncached), Some(write)) => {
                    Some([u64::from(read), u64::from(uncached), write])
                }
                _ => None,
            },
        })
        .collect()
}

/// A marker card: its title, its detail, a re-read's file label, and last
/// the derivation label.
pub(super) fn marker_lines(marker: &DrillMarker, threshold: u64) -> Vec<String> {
    let turn = marker.turn_ordinal.to_string();
    let mut lines = match marker.kind {
        MarkerKind::CacheWrittenAgain => vec![
            fill(
                &text("analytics_marker_cache_rewrite"),
                &[("m", &count(marker.pause_minutes))],
            ),
            fill(
                &text("analytics_marker_cache_rewrite_detail"),
                &[("t", &turn), ("x", &figure(marker.cache_write))],
            ),
        ],
        MarkerKind::ContextShrank => vec![fill(&text("analytics_marker_shrank"), &[("t", &turn)])],
        MarkerKind::CrossedLongContext => vec![
            fill(
                &text("analytics_marker_crossed"),
                &[("threshold", &figure(Some(threshold)))],
            ),
            text("analytics_marker_crossed_detail"),
        ],
        MarkerKind::ReRead => {
            let letter = marker.file_letter.clone().unwrap_or_else(dash);
            vec![
                fill(&text("analytics_marker_reread"), &[("letter", &letter)]),
                text("analytics_marker_reread_detail"),
                file_label(&letter, marker.file_ext.as_deref()),
            ]
        }
    };
    lines.push(text(match marker.basis {
        MarkerBasis::InferredFromCounters => "analytics_marker_inferred",
        MarkerBasis::FromCounters => "analytics_from_counters",
        MarkerBasis::FromToolCalls => "analytics_marker_from_tool_calls",
    }));
    lines
}

/// The tabs, in the macOS order. Spend is not a tab: it is shown disabled
/// beside them with its Later chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tab {
    Overview,
    Patterns,
    Sessions,
    Analyze,
}

impl Tab {
    pub(super) const ALL: [Tab; 4] = [Tab::Overview, Tab::Patterns, Tab::Sessions, Tab::Analyze];

    /// The page name in the tab stack.
    pub(super) fn name(self) -> &'static str {
        match self {
            Tab::Overview => "overview",
            Tab::Patterns => "patterns",
            Tab::Sessions => "sessions",
            Tab::Analyze => "analyze",
        }
    }

    /// The core's copy key for the tab's title.
    pub(super) fn title_key(self) -> &'static str {
        match self {
            Tab::Overview => "analytics_tab_overview",
            Tab::Patterns => "analytics_tab_patterns",
            Tab::Sessions => "analytics_tab_sessions",
            Tab::Analyze => "analytics_tab_analyze",
        }
    }

    fn from_name(name: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|tab| tab.name() == name)
    }
}

/// One in-flight read's slot. A later read in the same slot, or the view
/// going away, discards an earlier result.
#[derive(Clone, Copy)]
enum Slot {
    Overview,
    Inputs,
    Patterns,
    PatternSessions,
    Sessions,
}

/// The shell's current UTC offset, which buckets the week's days.
fn local_offset() -> i32 {
    chrono::Local::now().offset().local_minus_utc()
}

fn small(text: &str) -> gtk::Label {
    let line = label(text);
    line.add_css_class("dim-label");
    line.add_css_class("caption");
    line
}

fn styled(text: &str, class: &str) -> gtk::Label {
    let line = label(text);
    line.add_css_class(class);
    line
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

/// A plain bar for `fraction` of its track; `None` leaves the slot empty, a
/// gap rather than a zero bar. Never a ring (owner decision D1, open).
fn bar(fraction: Option<f64>, vertical: bool, figure_text: &str) -> gtk::Widget {
    let widget: gtk::Widget = match fraction {
        None => gtk::Box::new(gtk::Orientation::Horizontal, 0).upcast(),
        Some(fraction) => {
            let level = gtk::LevelBar::for_interval(0.0, 1.0);
            for offset in ["low", "high", "full"] {
                level.remove_offset_value(Some(offset));
            }
            level.set_value(fraction.clamp(0.0, 1.0));
            if vertical {
                level.set_orientation(gtk::Orientation::Vertical);
                level.set_inverted(true);
            }
            level.upcast()
        }
    };
    if vertical {
        widget.set_size_request(14, 48);
    } else {
        widget.set_hexpand(true);
        widget.set_size_request(80, 6);
        widget.set_valign(gtk::Align::Center);
    }
    widget.set_tooltip_text(Some(figure_text));
    widget
}

fn fraction(value: u64, most: u64) -> f64 {
    if most == 0 {
        0.0
    } else {
        value as f64 / most as f64
    }
}

/// A grid of text rows under a header row of core words.
fn table_grid(header: &[String], rows: Vec<Vec<String>>) -> gtk::Grid {
    let grid = gtk::Grid::builder()
        .column_spacing(16)
        .row_spacing(6)
        .build();
    for (column, heading) in header.iter().enumerate() {
        grid.attach(&styled(heading, "heading"), column as i32, 0, 1, 1);
    }
    for (row, cells) in rows.into_iter().enumerate() {
        for (column, cell) in cells.into_iter().enumerate() {
            grid.attach(&label(&cell), column as i32, row as i32 + 1, 1, 1);
        }
    }
    grid
}

fn card_box() -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.add_css_class("card");
    for setter in [
        gtk::prelude::WidgetExt::set_margin_top,
        gtk::prelude::WidgetExt::set_margin_bottom,
        gtk::prelude::WidgetExt::set_margin_start,
        gtk::prelude::WidgetExt::set_margin_end,
    ] {
        setter(&card, 4);
    }
    card.set_hexpand(true);
    card
}

/// The three analytics tabs. Analyze is the caller's.
pub(super) struct AnalyticsTabs {
    pub overview: gtk::ScrolledWindow,
    pub patterns: gtk::ScrolledWindow,
    pub sessions: gtk::ScrolledWindow,
    overview_body: gtk::Box,
    inputs_body: RefCell<Option<gtk::Box>>,
    patterns_body: gtk::Box,
    pattern_sessions_body: RefCell<Option<gtk::Box>>,
    sessions_body: gtk::Box,
    overview_week: Cell<Option<chrono::NaiveDate>>,
    patterns_week: Cell<Option<chrono::NaiveDate>>,
    /// The drill-down open under the Overview cards.
    open_inputs: Cell<Option<OverviewCard>>,
    /// The Patterns card whose sessions are listed.
    open_pattern: Cell<Option<PatternKind>>,
    /// Saved snapshots, newest first: ID and the saved list's label.
    snapshots: RefCell<Vec<(String, String)>>,
    selected: RefCell<Option<String>>,
    /// Set by the first saved list, which reads the tab on screen even when
    /// the list is empty.
    synced: Cell<bool>,
    shown: Cell<Tab>,
    generations: [Cell<u64>; 5],
    store_dir: Option<PathBuf>,
}

fn scrolled(body: &gtk::Box) -> gtk::ScrolledWindow {
    for setter in [
        gtk::prelude::WidgetExt::set_margin_top,
        gtk::prelude::WidgetExt::set_margin_bottom,
        gtk::prelude::WidgetExt::set_margin_start,
        gtk::prelude::WidgetExt::set_margin_end,
    ] {
        setter(body, 8);
    }
    gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(body)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build()
}

impl AnalyticsTabs {
    pub(super) fn new(store_dir: Option<PathBuf>) -> Rc<Self> {
        let overview_body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let patterns_body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let sessions_body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        Rc::new(Self {
            overview: scrolled(&overview_body),
            patterns: scrolled(&patterns_body),
            sessions: scrolled(&sessions_body),
            overview_body,
            inputs_body: RefCell::new(None),
            patterns_body,
            pattern_sessions_body: RefCell::new(None),
            sessions_body,
            overview_week: Cell::new(None),
            patterns_week: Cell::new(None),
            open_inputs: Cell::new(None),
            open_pattern: Cell::new(None),
            snapshots: RefCell::new(Vec::new()),
            selected: RefCell::new(None),
            synced: Cell::new(false),
            shown: Cell::new(Tab::Overview),
            generations: Default::default(),
            store_dir,
        })
    }

    fn generation(&self, slot: Slot) -> &Cell<u64> {
        &self.generations[slot as usize]
    }

    /// Discard whatever read is in flight in `slot`.
    fn bump(&self, slot: Slot) {
        self.generation(slot).set(self.generation(slot).get() + 1);
    }

    /// Read the tab that is now on screen.
    pub(super) fn show(self: &Rc<Self>, name: &str) {
        let Some(tab) = Tab::from_name(name) else {
            return;
        };
        self.shown.set(tab);
        self.load(tab);
    }

    /// Keep the session selection inside the saved list, newest first, and
    /// re-read the tab on screen when the list changed.
    pub(super) fn sync(self: &Rc<Self>, snapshots: Vec<(String, String)>) {
        if self.synced.get() && *self.snapshots.borrow() == snapshots {
            return;
        }
        self.synced.set(true);
        let keep = self
            .selected
            .borrow()
            .as_ref()
            .is_some_and(|id| snapshots.iter().any(|(saved, _)| saved == id));
        if !keep {
            *self.selected.borrow_mut() = snapshots.first().map(|(id, _)| id.clone());
        }
        *self.snapshots.borrow_mut() = snapshots;
        self.load(self.shown.get());
    }

    fn load(self: &Rc<Self>, tab: Tab) {
        let tz = local_offset();
        match tab {
            Tab::Overview => {
                // A drill-down still in flight belongs to the old cards.
                self.open_inputs.set(None);
                self.bump(Slot::Inputs);
                let operation = Op::WeekOverview {
                    week_start: self.overview_week.get(),
                    tz,
                };
                self.read(
                    Slot::Overview,
                    &self.overview_body,
                    operation,
                    |tabs, response| {
                        let Response::WeekOverview { overview } = response else {
                            return false;
                        };
                        tabs.present_overview(&overview);
                        true
                    },
                );
            }
            Tab::Patterns => {
                self.open_pattern.set(None);
                self.bump(Slot::PatternSessions);
                let operation = Op::Patterns {
                    week_start: self.patterns_week.get(),
                    weeks: None,
                    tz,
                };
                self.read(
                    Slot::Patterns,
                    &self.patterns_body,
                    operation,
                    |tabs, response| {
                        let Response::Patterns { patterns } = response else {
                            return false;
                        };
                        tabs.present_patterns(&patterns);
                        true
                    },
                );
            }
            Tab::Sessions => {
                let Some(snapshot_id) = self.selected.borrow().clone() else {
                    // Nothing saved: no session to read, and nothing to show.
                    self.bump(Slot::Sessions);
                    clear(&self.sessions_body);
                    self.sessions_body.append(&styled(&dash(), "title-2"));
                    return;
                };
                let operation = Op::SessionDrill {
                    snapshot_id: snapshot_id.clone(),
                    tz,
                };
                self.read(
                    Slot::Sessions,
                    &self.sessions_body,
                    operation,
                    move |tabs, response| {
                        let Response::SessionDrill { session } = response else {
                            return false;
                        };
                        if session.session_ref != snapshot_id {
                            return false;
                        }
                        tabs.present_session(&session);
                        true
                    },
                );
            }
            Tab::Analyze => {}
        }
    }

    /// Run one bounded read on an OS thread and present it in `body`. A
    /// failed or mismatched read leaves the dash, never an earlier figure.
    fn read(
        self: &Rc<Self>,
        slot: Slot,
        body: &gtk::Box,
        operation: Op,
        present: impl FnOnce(&Rc<Self>, Response) -> bool + 'static,
    ) {
        let generation = self.generation(slot).get() + 1;
        self.generation(slot).set(generation);
        clear(body);
        let spinner = gtk::Spinner::new();
        spinner.start();
        body.append(&spinner);
        let (tx, rx) = async_channel::bounded(1);
        let store_dir = self.store_dir.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                service::execute(LocalInsightsRequest {
                    store_dir,
                    operation,
                })
            })
            .ok()
            .and_then(Result::ok);
            let _ = tx.send_blocking(result);
        });
        let weak = Rc::downgrade(self);
        let body = body.downgrade();
        gtk::glib::spawn_future_local(async move {
            let result = rx.recv().await.ok().flatten();
            let (Some(tabs), Some(body)) = (weak.upgrade(), body.upgrade()) else {
                return;
            };
            if tabs.generation(slot).get() != generation {
                return;
            }
            clear(&body);
            let shown = result.is_some_and(|response| present(&tabs, response));
            if !shown {
                clear(&body);
                body.append(&styled(&dash(), "title-2"));
            }
        });
    }

    fn week_picker(
        self: &Rc<Self>,
        weeks: &[chrono::NaiveDate],
        shown: chrono::NaiveDate,
        tab: Tab,
    ) -> gtk::DropDown {
        let choices = week_choices(weeks, shown);
        let labels: Vec<String> = choices.iter().map(|week| week_range(*week)).collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let picker = gtk::DropDown::from_strings(&labels);
        picker.set_tooltip_text(Some(&text("analytics_this_week")));
        let index = choices.iter().position(|week| *week == shown).unwrap_or(0);
        picker.set_selected(index as u32);
        let weak = Rc::downgrade(self);
        picker.connect_selected_notify(move |picker| {
            let (Some(tabs), Some(week)) = (
                weak.upgrade(),
                choices.get(picker.selected() as usize).copied(),
            ) else {
                return;
            };
            match tab {
                Tab::Overview => tabs.overview_week.set(Some(week)),
                _ => tabs.patterns_week.set(Some(week)),
            }
            tabs.load(tab);
        });
        picker
    }

    fn present_overview(self: &Rc<Self>, overview: &WeekOverview) {
        let body = &self.overview_body;
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        heading.append(&styled(&text("analytics_this_week"), "title-2"));
        heading.append(&self.week_picker(&overview.weeks, overview.week_start, Tab::Overview));
        body.append(&heading);
        body.append(&label(&coverage_line(&overview.coverage)));
        if overview.undated_sessions > 0 {
            body.append(&small(&fill(
                &text("analytics_coverage_undated"),
                &[("d", &overview.undated_sessions.to_string())],
            )));
        }
        if let Some(overlap) = overview
            .coverage
            .reasons
            .get(&CoverageReason::ReimportOverlap)
            .filter(|overlap| **overlap > 0)
        {
            body.append(&small(&fill(
                &text("analytics_coverage_overlap"),
                &[("r", &overlap.to_string())],
            )));
        }
        for line in feed_lines(overview.feed) {
            body.append(&small(&line));
        }

        let cards = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        cards.set_homogeneous(true);
        // Tokens: one line per harness, side by side; never summed.
        let tokens = card_box();
        if overview.sources.is_empty() {
            tokens.append(&styled(&dash(), "title-3"));
        }
        for source in &overview.sources {
            tokens.append(&styled(&figure(source.tokens), "title-3"));
            tokens.append(&small(&source_line(source.source)));
            tokens.append(&small(&change(source)));
        }
        cards.append(&self.card_button("analytics_card_tokens", OverviewCard::Tokens, tokens));
        // Cache share: a bar per harness, not a ring.
        let cache = card_box();
        if overview.sources.is_empty() {
            cache.append(&styled(&dash(), "title-3"));
        }
        for source in &overview.sources {
            let permille = source.cache_share.map(|share| share.permille);
            cache.append(&bar(
                Some(permille.map_or(0.0, |permille| permille as f64 / 1000.0)),
                false,
                &share(source.cache_share.as_ref()),
            ));
            cache.append(&small(&cache_share_line(source)));
            cache.append(&small(&change(source)));
        }
        cards.append(&self.card_button(
            "analytics_card_cache_share",
            OverviewCard::CacheShare,
            cache,
        ));
        let sessions = card_box();
        sessions.append(&styled(&overview.sessions.to_string(), "title-3"));
        for line in overview.sources.iter().filter_map(largest_line) {
            sessions.append(&small(&line));
        }
        cards.append(&self.card_button(
            "analytics_card_sessions",
            OverviewCard::Sessions,
            sessions,
        ));
        body.append(&cards);
        let inputs = gtk::Box::new(gtk::Orientation::Vertical, 6);
        body.append(&inputs);
        *self.inputs_body.borrow_mut() = Some(inputs);

        body.append(&self.by_day(overview));
        body.append(&breakdown(overview));
    }

    /// A card that opens, or on a second press closes, its drill-down.
    fn card_button(
        self: &Rc<Self>,
        name_key: &str,
        card: OverviewCard,
        content: gtk::Box,
    ) -> gtk::Button {
        content.prepend(&styled(&text(name_key), "heading"));
        let button = gtk::Button::builder().child(&content).build();
        button.add_css_class("flat");
        button.set_tooltip_text(Some(&text("analytics_drill_title")));
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(tabs) = weak.upgrade() {
                tabs.toggle_inputs(card);
            }
        });
        button
    }

    fn toggle_inputs(self: &Rc<Self>, card: OverviewCard) {
        let Some(body) = self.inputs_body.borrow().clone() else {
            return;
        };
        if self.open_inputs.get() == Some(card) {
            self.open_inputs.set(None);
            self.bump(Slot::Inputs);
            clear(&body);
            return;
        }
        self.open_inputs.set(Some(card));
        let operation = Op::CardInputs {
            card,
            week_start: self.overview_week.get(),
            tz: local_offset(),
        };
        self.read(Slot::Inputs, &body, operation, move |tabs, response| {
            let Response::CardInputs { inputs } = response else {
                return false;
            };
            if inputs.card != card {
                return false;
            }
            if let Some(body) = tabs.inputs_body.borrow().as_ref() {
                body.append(&card_inputs(&inputs));
            }
            true
        });
    }

    fn by_day(&self, overview: &WeekOverview) -> gtk::Box {
        let card = card_box();
        card.append(&styled(&text("analytics_card_tokens_by_day"), "heading"));
        match &overview.by_day {
            Some(days) => {
                let series = [
                    "analytics_series_uncached",
                    "analytics_series_cache_read",
                    "analytics_series_cache_write",
                    "analytics_series_output",
                ];
                let most = days
                    .iter()
                    .flat_map(|day| [day.uncached, day.cache_read, day.cache_write, day.output])
                    .max()
                    .unwrap_or(0);
                let grid = gtk::Grid::builder()
                    .column_spacing(12)
                    .row_spacing(4)
                    .build();
                for (column, key) in series.iter().enumerate() {
                    grid.attach(&small(&text(key)), column as i32 + 1, 0, 1, 1);
                }
                for (row, day) in days.iter().enumerate() {
                    let row = row as i32 + 1;
                    grid.attach(&label(&day.date.to_string()), 0, row, 1, 1);
                    let values = [day.uncached, day.cache_read, day.cache_write, day.output];
                    for (column, value) in values.into_iter().enumerate() {
                        grid.attach(
                            &bar(Some(fraction(value, most)), false, &figure(Some(value))),
                            column as i32 + 1,
                            row,
                            1,
                            1,
                        );
                    }
                }
                card.append(&grid);
            }
            None => card.append(&label(&dash())),
        }
        if overview.codex_interval_tokens.is_some()
            || overview
                .sources
                .iter()
                .any(|source| source.source == AnalyticsSource::Codex)
        {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.append(&small(&text("analytics_codex_interval")));
            row.append(&label(&figure(overview.codex_interval_tokens)));
            card.append(&row);
        }
        card
    }

    fn present_patterns(self: &Rc<Self>, patterns: &WeekPatterns) {
        let body = &self.patterns_body;
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        heading.append(&styled(&text("analytics_where_tokens_went"), "title-2"));
        heading.append(&self.week_picker(&patterns.weeks, patterns.week_start, Tab::Patterns));
        body.append(&heading);
        body.append(&label(&text("analytics_patterns_intro")));
        body.append(&label(&text("analytics_patterns_overlap")));
        body.append(&small(&coverage_line(&patterns.coverage)));
        for line in feed_lines(patterns.feed) {
            body.append(&small(&line));
        }
        let cards = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        cards.set_homogeneous(true);
        for card in &patterns.cards {
            cards.append(&self.pattern_card(card, patterns.long_context_threshold));
        }
        body.append(&cards);
        if let Some(line) = claude_only_line(patterns) {
            body.append(&small(&line));
        }
        let listed = gtk::Box::new(gtk::Orientation::Vertical, 6);
        body.append(&listed);
        *self.pattern_sessions_body.borrow_mut() = Some(listed);

        let table = card_box();
        table.append(&styled(&text("analytics_reread_title"), "heading"));
        if patterns.reread_files.is_empty() {
            table.append(&label(&dash()));
        } else {
            table.append(&table_grid(
                &[
                    text("analytics_reread_file"),
                    text("analytics_reread_reads"),
                    text("analytics_reread_after_shrink"),
                    text("analytics_reread_tokens"),
                ],
                patterns
                    .reread_files
                    .iter()
                    .map(|row| {
                        vec![
                            file_label(&row.letter, row.ext.as_deref()),
                            row.reads.to_string(),
                            row.after_shrink.to_string(),
                            figure(row.tokens),
                        ]
                    })
                    .collect(),
            ));
        }
        body.append(&table);
    }

    fn pattern_card(self: &Rc<Self>, card: &PatternCard, threshold: u64) -> gtk::Box {
        let content = card_box();
        content.append(&styled(&pattern_title(card), "heading"));
        // The token figure is the headline; the count sits beneath it.
        content.append(&styled(&figure(card.tokens), "title-3"));
        content.append(&label(&count_line(card, threshold)));
        // Six weekly bars on a fixed axis of every week, so an absent week
        // is a visible gap.
        let bars = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let weeks = weekly_bars(card);
        let most = weeks.iter().flatten().copied().max().unwrap_or(0);
        for week in weeks {
            bars.append(&bar(
                week.map(|tokens| fraction(tokens, most)),
                true,
                &figure(week),
            ));
        }
        content.append(&bars);
        content.append(&small(&pattern_change(card)));
        for line in basis_lines(card) {
            content.append(&small(&line));
        }
        if let Some(words) = see_sessions(card) {
            let button = gtk::Button::with_label(&words);
            button.add_css_class("flat");
            let weak = Rc::downgrade(self);
            let kind = card.kind;
            button.connect_clicked(move |_| {
                if let Some(tabs) = weak.upgrade() {
                    tabs.toggle_pattern_sessions(kind);
                }
            });
            content.append(&button);
        }
        content
    }

    fn toggle_pattern_sessions(self: &Rc<Self>, pattern: PatternKind) {
        let Some(body) = self.pattern_sessions_body.borrow().clone() else {
            return;
        };
        if self.open_pattern.get() == Some(pattern) {
            self.open_pattern.set(None);
            self.bump(Slot::PatternSessions);
            clear(&body);
            return;
        }
        self.open_pattern.set(Some(pattern));
        let operation = Op::PatternSessions {
            pattern,
            week_start: self.patterns_week.get(),
            tz: local_offset(),
        };
        self.read(
            Slot::PatternSessions,
            &body,
            operation,
            move |tabs, response| {
                let Response::PatternSessions { pattern_sessions } = response else {
                    return false;
                };
                if pattern_sessions.pattern != pattern {
                    return false;
                }
                if let Some(body) = tabs.pattern_sessions_body.borrow().as_ref() {
                    body.append(&pattern_sessions_table(&pattern_sessions));
                }
                true
            },
        );
    }

    fn present_session(self: &Rc<Self>, drill: &SessionDrill) {
        let body = &self.sessions_body;
        let snapshots = self.snapshots.borrow().clone();
        if !snapshots.is_empty() {
            let labels: Vec<&str> = snapshots.iter().map(|(_, label)| label.as_str()).collect();
            let picker = gtk::DropDown::from_strings(&labels);
            picker.set_tooltip_text(Some(&text("analytics_drill_session")));
            let index = snapshots
                .iter()
                .position(|(id, _)| *id == drill.session_ref)
                .unwrap_or(0);
            picker.set_selected(index as u32);
            let weak = Rc::downgrade(self);
            picker.connect_selected_notify(move |picker| {
                let (Some(tabs), Some((id, _))) =
                    (weak.upgrade(), snapshots.get(picker.selected() as usize))
                else {
                    return;
                };
                *tabs.selected.borrow_mut() = Some(id.clone());
                tabs.load(Tab::Sessions);
            });
            body.append(&picker);
        }
        body.append(&styled(&session_header(drill), "title-3"));
        body.append(&small(&text("analytics_feed_saved")));
        if !drill.reasons.is_empty() {
            body.append(&small(&reasons(&drill.reasons)));
        }
        if let Some(line) = series_unavailable_line(drill) {
            body.append(&label(&line));
            return;
        }
        let chart = card_box();
        chart.append(&styled(&text("analytics_input_each_turn"), "heading"));
        let legend = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        for key in [
            "analytics_series_cache_read",
            "analytics_series_uncached",
            "analytics_series_cache_write",
        ] {
            legend.append(&small(&text(key)));
        }
        chart.append(&legend);
        // The long-context threshold, where the macOS chart draws its dashed
        // line.
        chart.append(&small(&format!(
            "{} \u{b7} {}",
            text("analytics_pattern_long_context"),
            figure(Some(drill.long_context_threshold))
        )));
        let rows = turn_rows(drill);
        let most = rows
            .iter()
            .filter_map(|row| row.cells)
            .flatten()
            .max()
            .unwrap_or(0);
        let grid = gtk::Grid::builder()
            .column_spacing(12)
            .row_spacing(4)
            .build();
        for (index, row) in rows.iter().enumerate() {
            let line = index as i32;
            grid.attach(&label(&row.label), 0, line, 1, 1);
            grid.attach(&styled(&row.letters, "heading"), 1, line, 1, 1);
            match row.cells {
                Some(cells) => {
                    for (column, value) in cells.into_iter().enumerate() {
                        grid.attach(
                            &bar(Some(fraction(value, most)), false, &figure(Some(value))),
                            column as i32 + 2,
                            line,
                            1,
                            1,
                        );
                    }
                }
                // A turn with an unknown counter draws nothing.
                None => grid.attach(&small(&dash()), 2, line, 3, 1),
            }
        }
        chart.append(&grid);
        body.append(&chart);
        for marker in &drill.markers {
            let card = card_box();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.append(&styled(&marker.letter, "title-3"));
            let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
            for (index, line) in marker_lines(marker, drill.long_context_threshold)
                .into_iter()
                .enumerate()
            {
                lines.append(&if index == 0 {
                    styled(&line, "heading")
                } else {
                    label(&line)
                });
            }
            row.append(&lines);
            card.append(&row);
            body.append(&card);
        }
    }
}

/// By model | By project | By tool.
fn breakdown(overview: &WeekOverview) -> gtk::Box {
    let card = card_box();
    let stack = gtk::Stack::new();
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&stack));
    card.append(&switcher);
    card.append(&stack);

    let models = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let most = overview
        .by_model
        .iter()
        .map(|row| row.tokens)
        .max()
        .unwrap_or(0);
    for ((name, tokens), row) in model_rows(overview).into_iter().zip(&overview.by_model) {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        line.append(&label(&name));
        line.append(&bar(Some(fraction(row.tokens, most)), false, &tokens));
        line.append(&label(&tokens));
        models.append(&line);
    }
    models.append(&small(&text("analytics_model_labels_note")));
    stack.add_titled(&models, Some("model"), &text("analytics_card_by_model"));

    // Owner decision D7, open: analyzed files carry no project.
    let project = gtk::Box::new(gtk::Orientation::Vertical, 4);
    project.append(&styled(&dash(), "heading"));
    project.append(&small(&text("analytics_by_project_unavailable")));
    stack.add_titled(
        &project,
        Some("project"),
        &text("analytics_card_by_project"),
    );

    let tools = gtk::Box::new(gtk::Orientation::Vertical, 4);
    for tool in &overview.by_tool {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        line.append(&label(&harness(Some(tool.source))));
        line.append(&label(&figure(tool.tokens)));
        tools.append(&line);
    }
    stack.add_titled(&tools, Some("tool"), &text("analytics_card_by_tool"));
    card
}

/// "What makes up this number": each session's own figure, coverage state
/// and reasons, in the core's order.
fn card_inputs(inputs: &CardInputs) -> gtk::Box {
    let card = card_box();
    card.append(&styled(&text("analytics_drill_title"), "heading"));
    let parts = |share: Option<&ShareFigure>| {
        share.map_or_else(dash, |share| {
            format!(
                "{} / {}",
                figure(Some(share.numerator)),
                figure(Some(share.denominator))
            )
        })
    };
    let cache = inputs.card == OverviewCard::CacheShare;
    if cache {
        for source in &inputs.sources {
            card.append(&label(&format!(
                "{} \u{b7} {}",
                harness(Some(source.source)),
                parts(source.cache_share.as_ref())
            )));
        }
    }
    card.append(&table_grid(
        &[
            text("analytics_drill_session"),
            text("analytics_drill_tokens"),
            text("analytics_drill_coverage"),
            text("analytics_drill_reason"),
        ],
        inputs
            .sessions
            .iter()
            .map(|session| {
                vec![
                    format!("{}\n{}", harness(session.source), session.session_ref),
                    if cache {
                        parts(session.cache_share.as_ref())
                    } else {
                        figure(session.tokens)
                    },
                    state(session.state),
                    reasons(&session.reasons),
                ]
            })
            .collect(),
    ));
    card
}

/// The sessions behind one Patterns card, in the core's order.
fn pattern_sessions_table(found: &PatternSessions) -> gtk::Box {
    let card = card_box();
    card.append(&styled(&pattern_name(found.pattern), "heading"));
    card.append(&table_grid(
        &[
            text("analytics_drill_session"),
            text("analytics_drill_tokens"),
            text("analytics_drill_coverage"),
            text("analytics_drill_reason"),
        ],
        found
            .sessions
            .iter()
            .map(|session| {
                vec![
                    session.session_ref.clone(),
                    figure(session.tokens),
                    state(session.state),
                    reasons(&session.reasons),
                ]
            })
            .collect(),
    ));
    card
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_commons_contributor::insights::service::{LocalInsightsResponse, ui_copy};
    use trace_commons_contributor::insights::week_glance::CardInputs;
    use trace_commons_contributor::insights::week_patterns::PatternSessions;

    fn response(name: &str) -> LocalInsightsResponse {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/insights-analytics")
            .join(name);
        serde_json::from_slice(&std::fs::read(path).expect("shared fixture")).expect("wire shape")
    }
    fn overview(name: &str) -> WeekOverview {
        match response(name) {
            LocalInsightsResponse::WeekOverview { overview } => *overview,
            _ => panic!("week_overview"),
        }
    }
    fn patterns(name: &str) -> WeekPatterns {
        match response(name) {
            LocalInsightsResponse::Patterns { patterns } => *patterns,
            _ => panic!("patterns"),
        }
    }
    fn drill(name: &str) -> SessionDrill {
        match response(name) {
            LocalInsightsResponse::SessionDrill { session } => *session,
            _ => panic!("session_drill"),
        }
    }
    fn words(key: &str) -> String {
        ui_copy()[key].clone()
    }

    #[test]
    fn unknown_is_the_core_dash_and_a_measured_zero_stays_zero() {
        assert_eq!(dash(), words("analytics_unavailable"));
        assert_eq!(figure(None), dash());
        assert_eq!(figure(Some(0)), "0");
        assert_eq!(figure(Some(37_235)), "37,235");
        assert_eq!(figure(Some(1_234_567)), "1,234,567");
        assert_eq!(share(None), dash());
        let half = ShareFigure {
            numerator: 1,
            denominator: 2,
            permille: 665,
        };
        // Whole percent from the core's per mille, rounded half up.
        assert_eq!(share(Some(&half)), "67");
        assert_eq!(
            fill("{a} of {b}, {a}", &[("a", "1"), ("b", "2")]),
            "1 of 2, 1"
        );
    }

    #[test]
    fn the_overview_reads_every_word_from_the_core_and_never_sums_harnesses() {
        let week = overview("week_overview.json");
        assert_eq!(
            coverage_line(&week.coverage),
            fill(
                &words("analytics_coverage_line"),
                &[("k", "1"), ("n", "2"), ("p", "1"), ("u", "0")]
            )
        );
        // Feed S says which feed it is and that weeks are not compared.
        assert_eq!(
            feed_lines(week.feed),
            [
                words("analytics_feed_saved"),
                words("analytics_feed_comparisons_need_counter_pass")
            ]
        );
        assert_eq!(
            feed_lines(Feed::CounterPass),
            [words("analytics_feed_counter_pass")]
        );
        assert_eq!(feed_lines(Feed::Ledger), [words("analytics_feed_ledger")]);
        let claude = &week.sources[0];
        let codex = &week.sources[1];
        assert_eq!(
            source_line(claude.source),
            words("analytics_source_claude_code")
        );
        assert_eq!(source_line(codex.source), words("analytics_source_codex"));
        // Under feed S nothing is compared with another week.
        assert_eq!(change(claude), dash());
        assert_eq!(
            cache_share_line(claude),
            fill(
                &words("analytics_cache_share_line"),
                &[("source", &words("claude_code")), ("p", "66")]
            )
        );
        // Codex has no cache share: the dash, never 0%.
        assert_eq!(
            cache_share_line(codex),
            fill(
                &words("analytics_cache_share_line"),
                &[("source", &words("codex")), ("p", &dash())]
            )
        );
        assert_eq!(
            largest_line(claude).unwrap(),
            format!(
                "{} \u{b7} {}",
                words("claude_code"),
                fill(&words("analytics_largest"), &[("t", "37,235")])
            )
        );
        assert_eq!(
            model_rows(&week),
            [("claude-fixture-model".to_string(), "37,235".to_string())]
        );
        assert_eq!(harness(None), dash());
        assert_eq!(
            reason(CoverageReason::SomeTurnsUnknown),
            words("analytics_reason_some_turns_unknown")
        );
        assert_eq!(
            state(CoverageState::Partial),
            words("analytics_state_partial")
        );
        let start = chrono::NaiveDate::from_ymd_opt(2026, 9, 14).unwrap();
        assert_eq!(week_range(start), "2026-09-14 \u{2013} 2026-09-20");
        let shown = chrono::NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        assert_eq!(week_choices(&week.weeks, start), [start]);
        assert_eq!(week_choices(&week.weeks, shown), [shown, start]);

        let inputs: CardInputs = match response("card_inputs_tokens.json") {
            LocalInsightsResponse::CardInputs { inputs } => *inputs,
            _ => panic!("card_inputs"),
        };
        assert_eq!(inputs.sessions.len(), 2);
    }

    #[test]
    fn an_empty_week_is_unknown_not_zero() {
        let week = overview("week_overview_empty.json");
        assert!(week.sources.is_empty());
        assert_eq!(
            coverage_line(&week.coverage),
            fill(
                &words("analytics_coverage_line"),
                &[("k", "0"), ("n", "0"), ("p", "0"), ("u", "0")]
            )
        );
        let empty = patterns("patterns_empty.json");
        for card in &empty.cards {
            assert_eq!(figure(card.tokens), dash());
            // The week on screen, the last bar, holds no saved session: a
            // gap, never a zero bar. The saved week of 2026-09-14 keeps its
            // own bar.
            let bars = weekly_bars(card);
            assert_eq!(bars.len(), 6);
            assert_eq!(bars[5], None);
            assert_eq!(see_sessions(card), None);
        }
    }

    #[test]
    fn patterns_lead_with_tokens_label_inference_and_leave_gaps_for_absent_weeks() {
        let week = patterns("patterns.json");
        let titles: Vec<String> = week.cards.iter().map(pattern_title).collect();
        assert_eq!(
            titles,
            [
                words("analytics_pattern_repeated_reads"),
                words("analytics_pattern_retried_calls"),
                words("analytics_pattern_edit_fail_edit"),
                words("analytics_pattern_long_context"),
            ]
        );
        let [reads, retried, loops, long] = &week.cards[..] else {
            panic!("four cards")
        };
        assert_eq!(
            count_line(reads, week.long_context_threshold),
            fill(
                &words("analytics_pattern_repeated_reads_count"),
                &[("r", "0"), ("f", "0")]
            )
        );
        assert_eq!(
            count_line(retried, week.long_context_threshold),
            fill(
                &words("analytics_pattern_retried_calls_count"),
                &[("c", "0")]
            )
        );
        assert_eq!(
            count_line(loops, week.long_context_threshold),
            fill(
                &words("analytics_pattern_edit_fail_edit_count"),
                &[("l", "0")]
            )
        );
        assert_eq!(
            count_line(long, week.long_context_threshold),
            fill(
                &words("analytics_pattern_long_context_line"),
                &[("threshold", "200,000")]
            )
        );
        // Edit, failed command, edit is labelled inferred (owner decision
        // D9, open), before its basis.
        assert_eq!(
            basis_lines(loops),
            [
                words("analytics_inferred_from_order"),
                words("analytics_estimate_from_result_size")
            ]
        );
        assert_eq!(basis_lines(long), [words("analytics_from_counters")]);
        assert_eq!(pattern_change(reads), dash());
        // Six weekly marks, oldest first; an absent week is a gap.
        assert_eq!(weekly_bars(reads), [None, None, None, None, None, Some(0)]);
        assert_eq!(weekly_bars(long), [None; 6]);
        assert_eq!(figure(long.tokens), dash());
        assert_eq!(see_sessions(reads), None);
        assert_eq!(
            claude_only_line(&week).unwrap(),
            fill(
                &words("analytics_claude_sessions_only"),
                &[("k", "1"), ("n", "2")]
            )
        );
        // Files are a letter and an extension, never a name (owner decision
        // D8, open).
        let labels: Vec<String> = week
            .reread_files
            .iter()
            .map(|row| file_label(&row.letter, row.ext.as_deref()))
            .collect();
        assert_eq!(
            labels,
            [
                fill(
                    &words("analytics_file_label"),
                    &[("letter", "A"), ("ext", ".rs")]
                ),
                fill(&words("analytics_file_label_no_ext"), &[("letter", "B")]),
            ]
        );
        let found: PatternSessions = match response("pattern_sessions.json") {
            LocalInsightsResponse::PatternSessions { pattern_sessions } => *pattern_sessions,
            _ => panic!("pattern_sessions"),
        };
        assert_eq!(figure(found.sessions[0].tokens), dash());
    }

    #[test]
    fn a_week_with_sessions_offers_them_and_a_change_is_worded_by_the_core() {
        let mut card = patterns("patterns.json").cards[0].clone();
        card.sessions = 3;
        assert_eq!(
            see_sessions(&card).unwrap(),
            fill(&words("analytics_see_sessions"), &[("n", "3")])
        );
        card.change = Some(-125);
        assert_eq!(
            pattern_change(&card),
            fill(&words("analytics_change_down"), &[("p", "13")])
        );
        card.change = Some(40);
        assert_eq!(
            pattern_change(&card),
            fill(&words("analytics_change_up"), &[("p", "4")])
        );
    }

    #[test]
    fn the_drill_in_heads_with_the_core_header_and_draws_no_unknown_turn() {
        let session = drill("session_drill_claude.json");
        assert_eq!(
            session_header(&session),
            fill(
                &words("analytics_session_header"),
                &[
                    ("date", "2026-09-14"),
                    ("harness", &words("claude_code")),
                    ("n", "6"),
                    ("t", "49,869"),
                    ("span", "0:08"),
                ]
            )
        );
        assert_eq!(series_unavailable_line(&session), None);
        let rows = turn_rows(&session);
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0].label, fill(&words("analytics_turn"), &[("n", "0")]));
        // Cache read, uncached, cache write.
        assert_eq!(rows[1].cells, Some([12_000, 5, 500]));
        // A turn with an unknown counter is a gap, never a zero bar.
        assert_eq!(rows[3].cells, None);
        assert_eq!(rows[2].letters, "A");
        assert_eq!(rows[5].letters, "D E");
        let threshold = session.long_context_threshold;
        let lines: Vec<Vec<String>> = session
            .markers
            .iter()
            .map(|marker| marker_lines(marker, threshold))
            .collect();
        assert_eq!(
            lines[0],
            [
                fill(&words("analytics_marker_cache_rewrite"), &[("m", "6")]),
                fill(
                    &words("analytics_marker_cache_rewrite_detail"),
                    &[("t", "2"), ("x", "12,600")]
                ),
                words("analytics_marker_inferred"),
            ]
        );
        assert_eq!(
            lines[1],
            [
                fill(&words("analytics_marker_shrank"), &[("t", "3")]),
                words("analytics_marker_inferred"),
            ]
        );
        assert_eq!(
            lines[2],
            [
                fill(
                    &words("analytics_marker_crossed"),
                    &[("threshold", "200,000")]
                ),
                words("analytics_marker_crossed_detail"),
                words("analytics_from_counters"),
            ]
        );
        assert_eq!(
            lines[3],
            [
                fill(&words("analytics_marker_reread"), &[("letter", "B")]),
                words("analytics_marker_reread_detail"),
                fill(
                    &words("analytics_file_label"),
                    &[("letter", "B"), ("ext", ".rs")]
                ),
                words("analytics_marker_from_tool_calls"),
            ]
        );
        assert_eq!(
            lines[4][2],
            fill(&words("analytics_file_label_no_ext"), &[("letter", "C")])
        );
    }

    #[test]
    fn a_codex_drill_in_says_turns_are_not_recorded_and_draws_nothing() {
        let session = drill("session_drill_codex.json");
        assert_eq!(
            series_unavailable_line(&session).unwrap(),
            words("analytics_codex_not_recorded")
        );
        assert!(turn_rows(&session).is_empty());
        assert!(session_header(&session).contains(&words("codex")));
    }
}
