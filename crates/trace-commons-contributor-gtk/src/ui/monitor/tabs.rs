//! The Monitor window's tabs, sub-pages and what each one allows.
//!
//! A port of the pure rules in
//! `macos/Sources/TraceCommonsApp/Views/MonitorWindowView.swift` (`Tab`,
//! `shownTab`, `inspectorAvailable`, `graphAvailable`, `breadcrumb`,
//! `mapTab(for:)`, `land`) and of `MonitorNotices.place`. No widget is built
//! here, so every rule is tested without a display.
//!
//! Deliberate platform differences from the Swift:
//!
//! - Before onboarding the Mac shows Inference alone. This shell shows
//!   Inference and Insights: GTK's Insights is account-free, it is what a
//!   plain launch opens, and the Weston gate reads it on a device that was
//!   never enrolled (`scripts/weston-portal-verify-inner.sh`).
//! - The person drags every tab but Home into their own order, as on the
//!   Mac (`Tab.movable`, `ordered`, `stored`, `moving`, owner 2026-10-30),
//!   kept in `monitor-window.json` under the Mac's comma-separated form;
//!   the accelerators and the tray follow it (`ui::screens`).
//! - There is no Missions catalogue page; this shell's only mission screen
//!   is the local drafts inbox.

use trace_commons_contributor::preview_copy::monitor_screens_copy;

/// The window's tabs, in the Mac's default order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    Home,
    Inference,
    Traces,
    Insights,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Home, Tab::Inference, Tab::Traces, Tab::Insights];

    /// The tab's identity: the name the tray, the accelerators, the
    /// `--start-page` driver and every in-window link have always opened it
    /// by. `private-inference` is `ui::PRIVATE_INFERENCE_SCREEN`.
    pub const fn name(self) -> &'static str {
        match self {
            Tab::Home => "home",
            Tab::Inference => "private-inference",
            Tab::Traces => "queue",
            Tab::Insights => "insights",
        }
    }

    /// The word shown, from the core's shell table. Insights takes the
    /// heading the Mac tab takes (`HomeTabView.insightsHeading`): the
    /// Insights service's own title.
    pub fn title(self) -> &'static str {
        let shell = monitor_screens_copy().shell;
        match self {
            Tab::Home => shell.tab_home,
            Tab::Inference => shell.tab_inference,
            Tab::Traces => shell.tab_traces,
            Tab::Insights => insights_title(),
        }
    }

    /// The tabs the strip shows. `None` is "the core has not said yet":
    /// every tab, as this window has always opened.
    pub fn shown(requires_onboarding: Option<bool>) -> Vec<Tab> {
        if requires_onboarding == Some(true) {
            vec![Tab::Inference, Tab::Insights]
        } else {
            current_order()
        }
    }
}

impl Tab {
    /// The Mac's raw value, the stored order's word for the tab.
    pub const fn raw(self) -> &'static str {
        match self {
            Tab::Home => "Home",
            Tab::Inference => "Inference",
            Tab::Traces => "Traces",
            Tab::Insights => "Insights",
        }
    }

    /// `Tab.movable`: every tab but Home, which always opens first.
    pub fn movable(self) -> bool {
        self != Tab::Home
    }
}

/// `Tab.ordered(_:)`: Home, then the movable tabs as stored, then any the
/// stored order does not name in their default order. An unknown or
/// repeated name is skipped.
pub fn ordered(stored: &str) -> Vec<Tab> {
    let mut movable: Vec<Tab> = Vec::new();
    for name in stored.split(',') {
        if let Some(tab) = Tab::ALL.into_iter().find(|tab| tab.raw() == name)
            && tab.movable()
            && !movable.contains(&tab)
        {
            movable.push(tab);
        }
    }
    let unnamed: Vec<Tab> = Tab::ALL
        .into_iter()
        .filter(|tab| tab.movable() && !movable.contains(tab))
        .collect();
    movable.extend(unnamed);
    std::iter::once(Tab::Home).chain(movable).collect()
}

/// `Tab.stored(_:)`: an order's movable tabs, by raw value.
pub fn stored(order: &[Tab]) -> String {
    order
        .iter()
        .filter(|tab| tab.movable())
        .map(|tab| tab.raw())
        .collect::<Vec<_>>()
        .join(",")
}

/// `Tab.moving(_:to:in:)`: the order after dragging `tab` onto `target`'s
/// place. Home never moves and nothing takes its place.
pub fn moving(tab: Tab, target: Tab, order: &[Tab]) -> Vec<Tab> {
    let (Some(from), Some(to)) = (
        order.iter().position(|t| *t == tab),
        order.iter().position(|t| *t == target),
    ) else {
        return order.to_vec();
    };
    if !tab.movable() || !target.movable() || tab == target {
        return order.to_vec();
    }
    let mut moved = order.to_vec();
    moved.remove(from);
    moved.insert(to, tab);
    moved
}

/// `GlassRuleTabs.landing`: where a tab dragged from slot `from` by
/// `offset` lands: the slot whose middle it has passed, kept to `allowed`.
pub fn landing(from: usize, offset: f64, tab_width: f64, allowed: (usize, usize)) -> usize {
    if tab_width <= 0.0 {
        return from;
    }
    // Swift's `.rounded()` is half away from zero, as Rust's `round`.
    let slot = from as i64 + (offset / tab_width).round() as i64;
    slot.clamp(allowed.0 as i64, allowed.1 as i64) as usize
}

/// `GlassRuleTabs.dragThreshold`: how far a lifted tab travels before
/// letting go no longer opens it.
pub const DRAG_THRESHOLD: f64 = 6.0;

/// `GlassRuleTabs.hasMoved`.
pub fn has_moved(translation: f64) -> bool {
    translation.abs() >= DRAG_THRESHOLD
}

/// The person's tab order as stored, for every reader: the strip and the
/// accelerators on the main thread, the tray's rows on its D-Bus thread.
#[cfg(not(test))]
static ORDER: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

#[cfg(test)]
thread_local! {
    /// Per test thread, so tests that set an order never see each other's.
    static ORDER: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// The tabs in the person's order now.
pub fn current_order() -> Vec<Tab> {
    #[cfg(not(test))]
    {
        ordered(
            &ORDER
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
    #[cfg(test)]
    {
        ORDER.with(|order| ordered(&order.borrow()))
    }
}

/// Records the person's order (`stored` form).
pub fn set_current_order(order: &str) {
    #[cfg(not(test))]
    {
        *ORDER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = order.to_string();
    }
    #[cfg(test)]
    {
        ORDER.with(|kept| *kept.borrow_mut() = order.to_string());
    }
}

fn insights_title() -> &'static str {
    static TITLE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TITLE.get_or_init(|| {
        trace_commons_contributor::insights::service::ui_copy()
            .get("title")
            .cloned()
            .unwrap_or_default()
    })
}

/// The crumb for the hosted mission-drafts page: the drafts service's own
/// title, which the screen itself is headed with.
pub fn mission_drafts_title() -> &'static str {
    static TITLE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TITLE.get_or_init(|| {
        trace_commons_contributor::mission_draft_service::ui_copy()
            .get("title")
            .cloned()
            .unwrap_or_default()
    })
}

/// The tab drawn for the selected one: while onboarding is required, a tab
/// the strip does not show falls back to Inference, where Private AI
/// sign-in is.
pub fn shown_tab(tab: Tab, requires_onboarding: Option<bool>) -> Tab {
    if Tab::shown(requires_onboarding).contains(&tab) {
        tab
    } else {
        Tab::Inference
    }
}

/// Home's page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HomePage {
    #[default]
    Overview,
    /// Kept for parity with the Mac's restored state. Nothing in this shell
    /// links here: History opens on Traces (owner, 2026-10-12).
    History,
    MissionDrafts,
}

impl HomePage {
    /// A mission page falls back to the overview while missions are hidden.
    pub fn shown(self, missions: bool) -> Self {
        if self == HomePage::MissionDrafts && !missions {
            HomePage::Overview
        } else {
            self
        }
    }
}

/// The Traces tab's page: the queue, or History.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TracesPage {
    #[default]
    Tree,
    History,
}

/// The map pane's view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MapView {
    #[default]
    Traces,
    PrivateAi,
}

/// What is on screen in the main pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location {
    pub tab: Tab,
    pub home_page: HomePage,
    pub traces_page: TracesPage,
}

impl Default for Location {
    fn default() -> Self {
        Self {
            tab: Tab::Home,
            home_page: HomePage::Overview,
            traces_page: TracesPage::Tree,
        }
    }
}

/// Where a request to open something lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// A tab, at a page.
    Show(Location),
    /// Settings is its own window; the tabs stay as they are.
    Settings,
}

/// Where a screen name lands, from wherever the window is now. The names
/// are the ones the old stack carried, so the tray, the accelerators, the
/// notification action, `--start-page` and every in-window link keep
/// working; `None` for a name nothing carries.
///
/// As `MonitorWindowView.land`: History lives on Traces.
pub fn route(name: &str, from: Location, missions: bool) -> Option<Route> {
    let mut to = from;
    match name {
        "home" => {
            to.tab = Tab::Home;
            to.home_page = HomePage::Overview;
        }
        "home-history" => {
            to.tab = Tab::Home;
            to.home_page = HomePage::History;
        }
        "mission-drafts" => {
            to.tab = Tab::Home;
            to.home_page = HomePage::MissionDrafts.shown(missions);
        }
        "private-inference" | "inference" => to.tab = Tab::Inference,
        "queue" | "traces" => {
            to.tab = Tab::Traces;
            to.traces_page = TracesPage::Tree;
        }
        "history" => {
            to.tab = Tab::Traces;
            to.traces_page = TracesPage::History;
        }
        "insights" => to.tab = Tab::Insights,
        "settings" => return Some(Route::Settings),
        _ => return None,
    }
    Some(Route::Show(to))
}

/// The content page a location draws, by the name the content stack
/// carries it under.
pub fn content_page(at: Location, missions: bool) -> &'static str {
    match at.tab {
        Tab::Home => match at.home_page.shown(missions) {
            HomePage::Overview => "home",
            HomePage::History => "history",
            HomePage::MissionDrafts => "mission-drafts",
        },
        Tab::Inference => "private-inference",
        Tab::Traces => match at.traces_page {
            TracesPage::Tree => "queue",
            TracesPage::History => "history",
        },
        Tab::Insights => "insights",
    }
}

/// Whether History is the page on screen: on Traces, or on Home.
pub fn shows_history(at: Location) -> bool {
    (at.tab == Tab::Traces && at.traces_page == TracesPage::History)
        || (at.tab == Tab::Home && at.home_page == HomePage::History)
}

/// Whether the inspector may open on what is shown: on Traces, on Home's
/// History, and on Insights while it hands the inspector a detail. Never on
/// Home's other pages or on Inference.
///
/// This shell's Insights hands the inspector nothing yet, so its callers
/// pass `false`.
pub fn inspector_available(at: Location, insights_detail: bool) -> bool {
    match at.tab {
        Tab::Traces => true,
        Tab::Home => at.home_page == HomePage::History,
        Tab::Insights => insights_detail,
        Tab::Inference => false,
    }
}

/// Whether the toolbar's Graph toggle works: only on Traces, the tab the
/// graph is drawn under.
pub fn graph_available(tab: Tab) -> bool {
    tab == Tab::Traces
}

/// Whether the transfer cards sit at the Traces map's foot: while any tab
/// but Traces is shown (owner-approved, 2026-11-16), as on the Mac, where
/// Traces shows its moving trace itself. Traces' History page is still
/// Traces.
pub fn transfer_cards_shown(tab: Tab) -> bool {
    tab != Tab::Traces
}

/// The map view a tab picks; `None` keeps the current one.
pub fn map_view_for(picked: Tab) -> Option<MapView> {
    match picked {
        Tab::Inference => Some(MapView::PrivateAi),
        Tab::Traces => Some(MapView::Traces),
        Tab::Home | Tab::Insights => None,
    }
}

/// Where a crumb goes back to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Back {
    Home,
    Traces,
}

/// One breadcrumb entry: its word, and where clicking it goes, if anywhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    pub title: &'static str,
    pub back: Option<Back>,
}

/// The breadcrumb under the tabs: History on Traces or Home, and Home's
/// mission drafts. `None` elsewhere.
pub fn breadcrumb(at: Location, missions: bool) -> Option<Vec<Crumb>> {
    let history = monitor_screens_copy().history;
    if at.tab == Tab::Traces && at.traces_page == TracesPage::History {
        return Some(vec![
            Crumb {
                title: Tab::Traces.title(),
                back: Some(Back::Traces),
            },
            Crumb {
                title: history,
                back: None,
            },
        ]);
    }
    if at.tab != Tab::Home {
        return None;
    }
    let home = Crumb {
        title: Tab::Home.title(),
        back: Some(Back::Home),
    };
    match at.home_page.shown(missions) {
        HomePage::Overview => None,
        HomePage::History => Some(vec![
            home,
            Crumb {
                title: history,
                back: None,
            },
        ]),
        HomePage::MissionDrafts => {
            let mut crumbs = vec![home];
            // Without the core's heading the crumb ends at Home, never a
            // word of this shell's own.
            if !mission_drafts_title().is_empty() {
                crumbs.push(Crumb {
                    title: mission_drafts_title(),
                    back: None,
                });
            }
            Some(crumbs)
        }
    }
}

/// The breadcrumb's icon-only back button's name: the core's "Back to
/// Home" for Home's trail, the first crumb's word for Traces'.
pub fn back_label(crumbs: &[Crumb]) -> &'static str {
    match crumbs.first() {
        Some(first) if first.back == Some(Back::Home) => monitor_screens_copy().shell.back_to_home,
        Some(first) => first.title,
        None => "",
    }
}

/// Where a tab's popover notices belong (`MonitorNotices.Place`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Home,
    PrivateAi,
    Traces,
    History,
    Insights,
    HomeHistory,
}

/// History is the Traces tab's other page.
pub fn place(at: Location) -> Place {
    match at.tab {
        Tab::Home if at.home_page == HomePage::History => Place::HomeHistory,
        Tab::Home => Place::Home,
        Tab::Inference => Place::PrivateAi,
        Tab::Traces if at.traces_page == TracesPage::History => Place::History,
        Tab::Traces => Place::Traces,
        Tab::Insights => Place::Insights,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(tab: Tab) -> Location {
        Location {
            tab,
            ..Location::default()
        }
    }

    fn swift(path: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/../../macos/Sources/{path}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    /// `MonitorWindowView.Tab`'s order rules, case for case with the
    /// Swift (`ordered`, `stored`, `moving`), and `GlassRuleTabs`'
    /// `landing` and 6-point threshold.
    #[test]
    fn the_person_orders_every_tab_but_home_as_on_the_mac() {
        let window = swift("TraceCommonsApp/Views/MonitorWindowView.swift");
        assert!(window.contains("var movable: Bool { self != .home }"));
        assert!(window.contains("case home = \"Home\""));
        let navigation = swift("TCDesign/Components/Navigation.swift");
        assert!(navigation.contains("static var dragThreshold: CGFloat { 6 }"));
        assert!(navigation.contains("let slot = from + Int((offset / tabWidth).rounded())"));

        use Tab::*;
        assert_eq!(ordered(""), vec![Home, Inference, Traces, Insights]);
        assert_eq!(
            ordered("Insights,Traces"),
            vec![Home, Insights, Traces, Inference]
        );
        // Home stored, unknown, repeated: skipped.
        assert_eq!(
            ordered("Home,Bogus,Traces,Traces,Inference"),
            vec![Home, Traces, Inference, Insights]
        );
        assert_eq!(
            stored(&[Home, Traces, Insights, Inference]),
            "Traces,Insights,Inference"
        );
        let order = ordered("");
        assert_eq!(
            moving(Insights, Inference, &order),
            vec![Home, Insights, Inference, Traces]
        );
        assert_eq!(
            moving(Inference, Insights, &order),
            vec![Home, Traces, Insights, Inference]
        );
        assert_eq!(moving(Home, Traces, &order), order);
        assert_eq!(moving(Traces, Home, &order), order);
        assert_eq!(landing(1, 0.0, 100.0, (1, 3)), 1);
        assert_eq!(landing(1, 49.0, 100.0, (1, 3)), 1);
        assert_eq!(landing(1, 50.0, 100.0, (1, 3)), 2);
        assert_eq!(landing(1, -50.0, 100.0, (1, 3)), 1);
        assert_eq!(landing(2, -50.0, 100.0, (1, 3)), 1);
        assert_eq!(landing(3, 400.0, 100.0, (1, 3)), 3);
        assert_eq!(landing(2, 10.0, 0.0, (1, 3)), 2);
        assert!(!has_moved(5.9) && has_moved(6.0) && has_moved(-6.0));
    }

    /// The strip, the accelerators and the tray read the stored order.
    #[test]
    fn the_shown_tabs_follow_the_order() {
        set_current_order("Insights,Inference");
        assert_eq!(
            Tab::shown(None),
            vec![Tab::Home, Tab::Insights, Tab::Inference, Tab::Traces]
        );
        assert_eq!(Tab::shown(Some(true)), vec![Tab::Inference, Tab::Insights]);
        set_current_order("");
        assert_eq!(Tab::shown(None), Tab::ALL.to_vec());
    }

    #[test]
    fn the_tabs_are_home_inference_traces_insights_in_the_cores_words() {
        assert_eq!(
            Tab::ALL,
            [Tab::Home, Tab::Inference, Tab::Traces, Tab::Insights]
        );
        let shell = monitor_screens_copy().shell;
        assert_eq!(Tab::Home.title(), shell.tab_home);
        assert_eq!(Tab::Inference.title(), shell.tab_inference);
        assert_eq!(Tab::Traces.title(), shell.tab_traces);
        assert_eq!(
            Tab::Insights.title(),
            trace_commons_contributor::insights::service::ui_copy()["title"]
        );
        for tab in Tab::ALL {
            assert!(!tab.title().is_empty(), "{tab:?} has no word");
        }
    }

    #[test]
    fn the_inference_tab_keeps_the_name_every_shortcut_spells() {
        assert_eq!(Tab::Inference.name(), crate::ui::PRIVATE_INFERENCE_SCREEN);
        let mut names: Vec<_> = Tab::ALL.iter().map(|tab| tab.name()).collect();
        names.dedup();
        assert_eq!(names.len(), Tab::ALL.len());
    }

    #[test]
    fn before_onboarding_home_and_traces_are_not_shown() {
        let shown = Tab::shown(Some(true));
        assert!(!shown.contains(&Tab::Home));
        assert!(!shown.contains(&Tab::Traces));
        assert_eq!(shown.first(), Some(&Tab::Inference));
        // The account-free Insights stays (see the module's note).
        assert_eq!(shown, vec![Tab::Inference, Tab::Insights]);
        assert_eq!(Tab::shown(Some(false)), Tab::ALL.to_vec());
        assert_eq!(Tab::shown(None), Tab::ALL.to_vec());
    }

    #[test]
    fn a_tab_that_is_not_shown_is_drawn_as_inference() {
        assert_eq!(shown_tab(Tab::Home, Some(true)), Tab::Inference);
        assert_eq!(shown_tab(Tab::Traces, Some(true)), Tab::Inference);
        assert_eq!(shown_tab(Tab::Inference, Some(true)), Tab::Inference);
        assert_eq!(shown_tab(Tab::Insights, Some(true)), Tab::Insights);
        for tab in Tab::ALL {
            assert_eq!(shown_tab(tab, Some(false)), tab);
            assert_eq!(shown_tab(tab, None), tab);
        }
    }

    #[test]
    fn the_inspector_opens_only_where_it_has_something_to_show() {
        assert!(inspector_available(at(Tab::Traces), false));
        let mut history = at(Tab::Traces);
        history.traces_page = TracesPage::History;
        assert!(inspector_available(history, false));
        assert!(!inspector_available(at(Tab::Home), false));
        let mut home_history = at(Tab::Home);
        home_history.home_page = HomePage::History;
        assert!(inspector_available(home_history, false));
        let mut drafts = at(Tab::Home);
        drafts.home_page = HomePage::MissionDrafts;
        assert!(!inspector_available(drafts, false));
        assert!(!inspector_available(at(Tab::Insights), false));
        assert!(inspector_available(at(Tab::Insights), true));
        assert!(!inspector_available(at(Tab::Inference), false));
        assert!(!inspector_available(at(Tab::Inference), true));
    }

    #[test]
    fn the_graph_is_the_traces_tabs() {
        assert!(graph_available(Tab::Traces));
        for tab in [Tab::Home, Tab::Inference, Tab::Insights] {
            assert!(!graph_available(tab), "the Graph toggle works on {tab:?}");
        }
    }

    #[test]
    fn inference_and_traces_pick_the_maps_view_and_the_others_keep_it() {
        assert_eq!(map_view_for(Tab::Inference), Some(MapView::PrivateAi));
        assert_eq!(map_view_for(Tab::Traces), Some(MapView::Traces));
        assert_eq!(map_view_for(Tab::Home), None);
        assert_eq!(map_view_for(Tab::Insights), None);
    }

    #[test]
    fn every_old_screen_name_still_lands_somewhere() {
        let from = Location::default();
        let show = |name: &str, missions: bool| match route(name, from, missions) {
            Some(Route::Show(to)) => to,
            other => panic!("{name} did not land on a tab: {other:?}"),
        };
        assert_eq!(show("home", false), at(Tab::Home));
        assert_eq!(show("insights", false).tab, Tab::Insights);
        assert_eq!(show("queue", false).tab, Tab::Traces);
        assert_eq!(show("queue", false).traces_page, TracesPage::Tree);
        assert_eq!(
            show(crate::ui::PRIVATE_INFERENCE_SCREEN, false).tab,
            Tab::Inference
        );
        // History lives on Traces, behind the breadcrumb.
        let history = show("history", false);
        assert_eq!(history.tab, Tab::Traces);
        assert_eq!(history.traces_page, TracesPage::History);
        // Mission drafts is Home's sub-page while missions are shown.
        let drafts = show("mission-drafts", true);
        assert_eq!(drafts.tab, Tab::Home);
        assert_eq!(drafts.home_page, HomePage::MissionDrafts);
        assert_eq!(show("mission-drafts", false), at(Tab::Home));
        // Settings is its own window and moves no tab.
        assert_eq!(route("settings", from, false), Some(Route::Settings));
        assert_eq!(route("no-such-screen", from, true), None);
        for tab in Tab::ALL {
            assert_eq!(show(tab.name(), true).tab, tab);
        }
    }

    #[test]
    fn opening_the_queue_leaves_history_and_opening_home_leaves_its_sub_page() {
        let mut from = at(Tab::Traces);
        from.traces_page = TracesPage::History;
        from.home_page = HomePage::MissionDrafts;
        assert_eq!(
            route("queue", from, true),
            Some(Route::Show(Location {
                tab: Tab::Traces,
                home_page: HomePage::MissionDrafts,
                traces_page: TracesPage::Tree,
            }))
        );
        match route("home", from, true) {
            Some(Route::Show(to)) => assert_eq!(to.home_page, HomePage::Overview),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn each_location_draws_one_of_the_existing_pages() {
        assert_eq!(content_page(at(Tab::Home), false), "home");
        assert_eq!(content_page(at(Tab::Inference), false), "private-inference");
        assert_eq!(content_page(at(Tab::Traces), false), "queue");
        assert_eq!(content_page(at(Tab::Insights), false), "insights");
        let mut history = at(Tab::Traces);
        history.traces_page = TracesPage::History;
        assert_eq!(content_page(history, false), "history");
        let mut drafts = at(Tab::Home);
        drafts.home_page = HomePage::MissionDrafts;
        assert_eq!(content_page(drafts, true), "mission-drafts");
        // Hidden missions cannot hold a mission page open.
        assert_eq!(content_page(drafts, false), "home");
    }

    #[test]
    fn the_breadcrumb_is_drawn_only_on_the_sub_pages() {
        for tab in Tab::ALL {
            assert_eq!(breadcrumb(at(tab), true), None, "{tab:?}");
        }
        let words = monitor_screens_copy();
        let mut history = at(Tab::Traces);
        history.traces_page = TracesPage::History;
        let crumbs = breadcrumb(history, false).expect("Traces' History has a trail");
        assert_eq!(crumbs[0].title, words.shell.tab_traces);
        assert_eq!(crumbs[0].back, Some(Back::Traces));
        assert_eq!(crumbs[1].title, words.history);
        assert_eq!(crumbs[1].back, None);
        assert_eq!(back_label(&crumbs), words.shell.tab_traces);

        let mut home_history = at(Tab::Home);
        home_history.home_page = HomePage::History;
        let crumbs = breadcrumb(home_history, false).expect("Home's History has a trail");
        assert_eq!(crumbs[0].back, Some(Back::Home));
        assert_eq!(crumbs[1].title, words.history);
        assert_eq!(back_label(&crumbs), words.shell.back_to_home);

        let mut drafts = at(Tab::Home);
        drafts.home_page = HomePage::MissionDrafts;
        let crumbs = breadcrumb(drafts, true).expect("mission drafts has a trail");
        assert_eq!(crumbs[0].title, words.shell.tab_home);
        assert_eq!(
            crumbs.last().map(|crumb| crumb.title),
            Some(trace_commons_contributor::mission_draft_service::ui_copy()["title"].as_str())
        );
        assert_eq!(breadcrumb(drafts, false), None, "missions are hidden");
    }

    #[test]
    fn history_is_on_screen_on_either_tab() {
        let mut traces = at(Tab::Traces);
        assert!(!shows_history(traces));
        traces.traces_page = TracesPage::History;
        assert!(shows_history(traces));
        let mut home = at(Tab::Home);
        home.traces_page = TracesPage::History;
        assert!(!shows_history(home), "Traces' page is not Home's");
        home.home_page = HomePage::History;
        assert!(shows_history(home));
    }

    #[test]
    fn a_tabs_notices_go_where_the_mac_puts_them() {
        assert_eq!(place(at(Tab::Home)), Place::Home);
        assert_eq!(place(at(Tab::Inference)), Place::PrivateAi);
        assert_eq!(place(at(Tab::Traces)), Place::Traces);
        assert_eq!(place(at(Tab::Insights)), Place::Insights);
        let mut history = at(Tab::Traces);
        history.traces_page = TracesPage::History;
        assert_eq!(place(history), Place::History);
        let mut home_history = at(Tab::Home);
        home_history.home_page = HomePage::History;
        assert_eq!(place(home_history), Place::HomeHistory);
    }

    #[test]
    fn transfer_cards_sit_at_the_maps_foot_on_every_tab_but_traces() {
        assert!(!transfer_cards_shown(Tab::Traces));
        for tab in [Tab::Home, Tab::Inference, Tab::Insights] {
            assert!(transfer_cards_shown(tab), "{tab:?}");
        }
    }
}
