//! An overview of current facts, linking to the existing full screens.
//!
//! Under the tiles, the Mac's three hero cards (`HomeOverview` in
//! `macos/Sources/TraceCommonsApp/Views/Monitor/HomeViews.swift`): the NEAR
//! AI campaign, Missions and Insights, each a `hero_card::HeroCard` with the
//! Mac's ground and graphic, shown and closed under the Mac's conditions
//! (`crate::home_cards`). When NEAR AI credit arrives, gold coins burst from
//! the Balance card and its figure counts up (`BalanceArrivalFlourish`).
use super::hero_art::{self, Ground, arrival, bars::Glance};
use super::hero_card::{self, HeroCard};
use super::{App, balance, private_inference, style::space};
use crate::home_cards::{self, Campaign, HomePrefs};
use crate::{copy, model};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use trace_commons_contributor::preview_copy::monitor_screens_copy;

/// The coins under way: when they began (frame-clock microseconds), and
/// whether they are the still ones of reduced motion.
#[derive(Default)]
struct Flourish {
    start: Cell<Option<i64>>,
    still: Cell<bool>,
    /// The clock that moves the coins, and the timer that ends them.
    tick: RefCell<Option<gtk::TickCallbackId>>,
    end: RefCell<Option<glib::SourceId>>,
    /// The count under way, kept apart from the reads so a refresh does
    /// not cancel it.
    count: RefCell<Option<glib::SourceId>>,
}

pub struct HomeView {
    pub root: gtk::Box,
    pub balance: balance::BalanceSection,
    waiting: gtk::Label,
    contributed: gtk::Label,
    inference: gtk::Box,
    links: Vec<(gtk::Button, &'static str)>,
    prefs: RefCell<HomePrefs>,
    campaign: Option<(Campaign, HeroCard)>,
    missions: Option<(HeroCard, gtk::Label)>,
    insights: Option<(HeroCard, gtk::Label)>,
    /// Over everything on Home, so the coins fall over the cards beside
    /// and below the Balance card.
    overlay: gtk::Overlay,
    burst: gtk::DrawingArea,
    flourish: Rc<Flourish>,
    /// The latest Balance read, checked when the card is shown.
    latest: Rc<RefCell<Option<model::NearAiBalance>>>,
}

impl HomeView {
    pub fn new(missions: bool) -> Self {
        let words = monitor_screens_copy();
        let prefs = HomePrefs::load();
        let content = gtk::Box::new(gtk::Orientation::Vertical, space::L);
        // Reuse the generated flat palette and type roles already used by Insights.
        content.add_css_class("tc-insights");
        content.set_margin_top(space::L);
        content.set_margin_bottom(space::L);
        content.set_margin_start(space::M);
        content.set_margin_end(space::M);
        let title = gtk::Label::builder()
            .label(words.shell.tab_home)
            .xalign(0.0)
            .build();
        title.add_css_class("title-2");
        content.append(&title);
        let mut links = Vec::new();
        content.append(&heading(words.home_history.section_private_ai));
        let inference = gtk::Box::new(gtk::Orientation::Vertical, space::S);
        inference.add_css_class("tc-insights-card");
        let inference_status = gtk::Box::new(gtk::Orientation::Vertical, space::XS);
        inference.append(&inference_status);
        let open = gtk::Button::with_label(words.shell.tab_inference);
        open.set_halign(gtk::Align::Start);
        inference.append(&open);
        links.push((open, super::PRIVATE_INFERENCE_SCREEN));
        content.append(&inference);

        content.append(&heading(words.home_history.section_traces));
        let tiles = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .min_children_per_line(1)
            .max_children_per_line(2)
            .column_spacing(space::M as u32)
            .row_spacing(space::M as u32)
            .homogeneous(true)
            .build();
        tiles.add_css_class("tc-insights-summary-grid");
        let (waiting_button, waiting) = tile(words.waiting);
        let (contributed_button, contributed) = tile(words.contributed);
        tiles.insert(&waiting_button, -1);
        tiles.insert(&contributed_button, -1);
        links.push((waiting_button, "queue"));
        links.push((contributed_button, "history"));
        content.append(&tiles);
        content.append(&heading(words.home_history.section_account));
        let balance = balance::BalanceSection::new();
        balance.use_flat_style();
        content.append(&balance.root);

        // The NEAR AI campaign, under the tiles; the X closes it as it is
        // now, and it comes back when what it shows changes.
        let campaign = Campaign::compiled().map(|campaign| {
            let hero = HeroCard::new(
                Ground::Campaign,
                &campaign.title,
                Some(&campaign.body),
                Some(&campaign.action_title),
                Some(words.close),
            );
            hero.set_symbol(Some(&campaign.symbol));
            let url = campaign.action_url.clone();
            hero.button.connect_clicked(move |_| {
                let _ = gtk::gio::AppInfo::launch_default_for_uri(
                    &url,
                    None::<&gtk::gio::AppLaunchContext>,
                );
            });
            hero.root.set_visible(home_cards::shows_campaign(
                &campaign.fingerprint(),
                &prefs.closed_campaign,
                prefs.shows_promotions,
            ));
            content.append(&hero.root);
            (campaign, hero)
        });

        // Missions, while missions are on: the drafts' line, and the way
        // into them.
        let missions = missions.then(|| {
            let hero = HeroCard::new(
                Ground::Missions,
                words.missions,
                Some(words.home_history.missions_subtitle),
                Some(words.home_history.missions_action),
                None,
            );
            hero.set_symbol(Some(MISSIONS_SYMBOL));
            let line = hero_line(&home_cards::drafts_line(None, ""));
            line.set_wrap(false);
            line.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            hero.content.append(&line);
            content.append(&hero.root);
            links.push((hero.button.clone(), "mission-drafts"));
            (hero, line)
        });

        // Insights, once the core's heading has loaded. The Mac's line
        // under it names the Mac; this shell keeps Insights' own intro,
        // which is device-neutral.
        let insights_words = trace_commons_contributor::insights::service::ui_copy();
        let insights = insights_words
            .get("title")
            .filter(|title| !title.is_empty())
            .map(|title| {
                let hero = HeroCard::new(
                    Ground::Insights,
                    title,
                    Some(&insights_words["intro"]),
                    Some(words.home_history.insights_action),
                    None,
                );
                let line = hero_line("");
                hero.content.append(&line);
                content.append(&hero.root);
                links.push((hero.button.clone(), "insights"));
                (hero, line)
            });

        let clamp = adw::Clamp::builder()
            .maximum_size(super::COLUMN_MAX)
            .tightening_threshold(super::COLUMN_TIGHTEN)
            .child(&content)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build();
        let burst = gtk::DrawingArea::builder()
            .can_target(false)
            .hexpand(true)
            .vexpand(true)
            .visible(false)
            .build();
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&scroll));
        overlay.add_overlay(&burst);
        overlay.set_overflow(gtk::Overflow::Hidden);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&overlay);
        let view = Self {
            root,
            balance,
            waiting,
            contributed,
            inference: inference_status,
            links,
            prefs: RefCell::new(prefs),
            campaign,
            missions,
            insights,
            overlay,
            burst,
            flourish: Rc::new(Flourish::default()),
            latest: Rc::new(RefCell::new(None)),
        };
        view.render_inference(None);
        view.render_glance(None);
        view.connect_burst();
        view
    }

    pub fn bind(&self, app: &Rc<App>) {
        for (button, target) in &self.links {
            let weak = Rc::downgrade(app);
            let target = *target;
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.open_screen(target);
                }
            });
        }
        let weak = Rc::downgrade(app);
        self.balance.card.connect_map(move |_| {
            if let Some(app) = weak.upgrade() {
                app.home.check_balance();
            }
        });
        let weak = Rc::downgrade(app);
        self.balance.card.connect_unmap(move |_| {
            if let Some(app) = weak.upgrade() {
                app.home.stop_flourish();
            }
        });
        if let Some((_, hero)) = &self.campaign
            && let Some(close) = &hero.close
        {
            let weak = Rc::downgrade(app);
            close.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.home.close_campaign();
                }
            });
        }
    }

    /// Draws the coins round the Balance card's centre, wherever it is on
    /// Home now.
    fn connect_burst(&self) {
        let flourish = Rc::downgrade(&self.flourish);
        let overlay = self.overlay.downgrade();
        let card = self.balance.card.downgrade();
        self.burst.set_draw_func(move |_, cr, _, _| {
            let (Some(flourish), Some(overlay), Some(card)) =
                (flourish.upgrade(), overlay.upgrade(), card.upgrade())
            else {
                return;
            };
            let Some(start) = flourish.start.get() else {
                return;
            };
            let t = if flourish.still.get() {
                hero_art::coins::STILL_AT
            } else {
                (overlay
                    .frame_clock()
                    .map_or(start, |clock| clock.frame_time())
                    - start) as f64
                    / 1_000_000.0
            };
            let Some(centre) = card.compute_point(
                &overlay,
                &gtk::graphene::Point::new(card.width() as f32 / 2.0, card.height() as f32 / 2.0),
            ) else {
                return;
            };
            let room = card.width() as f64 / 2.0 + hero_art::coins::EDGE_ROOM;
            hero_art::coins::draw(
                cr,
                (centre.x() as f64, centre.y() as f64),
                t,
                !flourish.still.get(),
                room,
            );
        });
    }

    pub fn render_status(&self, status: Option<&model::Status>) {
        let counts = crate::home::Counts::from_answers(status, None);
        self.waiting.set_text(&crate::home::figure(counts.waiting));
    }

    pub fn render_rollup(&self, rollup: Option<&crate::home::Rollup>) {
        let counts = crate::home::Counts::from_answers(None, rollup);
        self.contributed
            .set_text(&crate::home::figure(counts.contributed));
    }

    pub fn render_inference(&self, settings: Option<&model::Settings>) {
        while let Some(child) = self.inference.first_child() {
            self.inference.remove(&child);
        }
        let state = settings
            .and_then(|s| s.private_inference_state.as_ref())
            .map(|s| s.state.as_str())
            .unwrap_or_default();
        self.inference.append(&private_inference::tone_row(
            copy::private_inference_state_line(state),
            private_inference::indicator_tone(copy::private_inference_state_tone(state)),
        ));
    }

    /// The Insights hero from today's glance (`HomeInsightsVisual`,
    /// `HomeInsightsGraphic`): with figures, today's total in its words and
    /// the tools' bars as its graphic; without, the teaser and the chart.
    /// `None` is a glance that did not come back: a stale day is not drawn
    /// as today's.
    pub fn render_glance(&self, glance: Option<&Glance>) {
        let Some((hero, line)) = &self.insights else {
            return;
        };
        let words = &monitor_screens_copy().home_history;
        let bars = hero_art::bars::bars(glance);
        if bars.is_empty() {
            line.set_text(words.insights_teaser);
            line.remove_css_class("tc-type-heading");
            line.remove_css_class("tc-hero-figure");
            line.add_css_class("tc-type-label");
            line.add_css_class("tc-hero-line");
            hero.set_symbol(Some(INSIGHTS_SYMBOL));
            hero.set_graphic(None::<&gtk::Widget>);
        } else {
            let total = bars.iter().map(|(_, tokens)| *tokens).sum::<u64>();
            line.set_text(
                &words
                    .insights_today
                    .replace("{total}", &hero_art::compact(total)),
            );
            line.remove_css_class("tc-type-label");
            line.remove_css_class("tc-hero-line");
            line.add_css_class("tc-type-heading");
            line.add_css_class("tc-hero-figure");
            hero.set_symbol(None);
            let graphic = hero_card::insights_bars(&bars, tool_name);
            hero.set_graphic(Some(&graphic));
        }
    }

    /// The Missions hero's line from the drafts inbox; `None` before it has
    /// answered.
    pub fn render_drafts(&self, ids: Option<&[String]>) {
        if let Some((_, line)) = &self.missions {
            line.set_text(&home_cards::drafts_line(
                ids,
                monitor_screens_copy().home_history.no_mission_drafts,
            ));
        }
    }

    /// Settings' "Show promotions".
    pub fn set_promotions(&self, on: bool) {
        self.prefs.borrow_mut().shows_promotions = on;
        self.prefs.borrow().save();
        self.show_campaign();
    }

    /// The campaign's X: closed as it is now.
    pub fn close_campaign(&self) {
        if let Some((campaign, _)) = &self.campaign {
            self.prefs.borrow_mut().closed_campaign = campaign.fingerprint();
            self.prefs.borrow().save();
        }
        self.show_campaign();
    }

    fn show_campaign(&self) {
        if let Some((campaign, hero)) = &self.campaign {
            let prefs = self.prefs.borrow();
            hero.root.set_visible(home_cards::shows_campaign(
                &campaign.fingerprint(),
                &prefs.closed_campaign,
                prefs.shows_promotions,
            ));
        }
    }

    /// A Balance read that answered. It is checked for NEAR AI credit
    /// arriving only while the Balance card is on screen, as the Mac checks
    /// when its tile is drawn; otherwise it waits for the card to be shown
    /// (`check_balance`), so credit is never marked where nobody sees it.
    pub fn balance_read(&self, balance: &model::NearAiBalance) {
        *self.latest.borrow_mut() = Some(balance.clone());
        if self.balance.card.is_mapped() {
            self.check_balance();
        }
    }

    /// Marks credit that arrived since the figure last shown
    /// (`BalanceArrivalFlourish`): the coins burst as the count starts;
    /// under reduced motion they are drawn still for a moment and the
    /// figure does not count.
    pub fn check_balance(&self) {
        let Some(balance) = self.latest.borrow().clone() else {
            return;
        };
        let now = arrival::Reading {
            received_nanos: balance.received_total_nanos,
            remaining_nanos: balance.remaining_nanos,
        };
        let reduce = !hero_card::animations_enabled();
        let shown = self.prefs.borrow().balance_shown;
        if shown == Some(now) {
            return;
        }
        let effect = arrival::effect(shown, now, reduce);
        if now.received_nanos.is_some() {
            self.prefs.borrow_mut().balance_shown = Some(now);
            self.prefs.borrow().save();
        }
        match effect {
            arrival::Effect::None => {}
            arrival::Effect::Highlight => self.burst(reduce),
            arrival::Effect::CountUp { from, to } => {
                self.burst(false);
                self.count_up(&balance, from, to);
            }
        }
    }

    /// The card left the screen: the coins and the count stop, and the row
    /// shows the latest read.
    pub fn stop_flourish(&self) {
        let flourish = &self.flourish;
        for timer in [&flourish.end, &flourish.count] {
            if let Some(id) = timer.borrow_mut().take() {
                id.remove();
            }
        }
        if let Some(id) = flourish.tick.borrow_mut().take() {
            id.remove();
        }
        flourish.start.set(None);
        self.burst.set_visible(false);
        self.restore_head();
    }

    fn restore_head(&self) {
        if let Some(latest) = self.latest.borrow().as_ref() {
            let text = balance::head_text(latest);
            if !text.is_empty() {
                (self.balance.head_writer())(&text);
            }
        }
    }

    /// Starts the coins: the frame clock moves them for the burst's length,
    /// or they stand still for the hold under reduced motion.
    fn burst(&self, still: bool) {
        let flourish = &self.flourish;
        if let Some(id) = flourish.tick.borrow_mut().take() {
            id.remove();
        }
        if let Some(id) = flourish.end.borrow_mut().take() {
            id.remove();
        }
        let start = self
            .overlay
            .frame_clock()
            .map_or_else(glib::monotonic_time, |clock| clock.frame_time());
        flourish.start.set(Some(start));
        flourish.still.set(still);
        self.burst.set_visible(true);
        self.burst.queue_draw();
        if !still {
            let tick = self.burst.add_tick_callback(|area, _| {
                area.queue_draw();
                glib::ControlFlow::Continue
            });
            *flourish.tick.borrow_mut() = Some(tick);
        }
        let stay = if still {
            std::time::Duration::from_millis(arrival::HOLD_MS)
        } else {
            std::time::Duration::from_secs_f64(hero_art::coins::DURATION)
        };
        let weak = Rc::downgrade(flourish);
        let area = self.burst.downgrade();
        let end = glib::timeout_add_local_once(stay, move || {
            let Some(flourish) = weak.upgrade() else {
                return;
            };
            flourish.end.borrow_mut().take();
            if let Some(id) = flourish.tick.borrow_mut().take() {
                id.remove();
            }
            flourish.start.set(None);
            if let Some(area) = area.upgrade() {
                area.set_visible(false);
            }
        });
        *flourish.end.borrow_mut() = Some(end);
    }

    /// Counts the Balance figure from the one last shown to the new one,
    /// one step every 50 ms, in the figure's own words.
    fn count_up(&self, balance: &model::NearAiBalance, from: i64, to: i64) {
        if let Some(id) = self.flourish.count.borrow_mut().take() {
            id.remove();
        }
        let steps = arrival::count_steps(from, to, arrival::FRAMES);
        let template = balance.clone();
        let section = self.balance.head_writer();
        let flourish = Rc::downgrade(&self.flourish);
        let latest = Rc::downgrade(&self.latest);
        let mut next = 0;
        let id = glib::timeout_add_local(
            std::time::Duration::from_millis(arrival::FRAME_MS),
            move || {
                let Some(step) = steps.get(next) else {
                    if let Some(flourish) = flourish.upgrade() {
                        flourish.count.borrow_mut().take();
                    }
                    // A read that came in while counting is what stays.
                    if let Some(text) = latest
                        .upgrade()
                        .and_then(|latest| latest.borrow().as_ref().map(balance::head_text))
                        .filter(|text| !text.is_empty())
                    {
                        section(&text);
                    }
                    return glib::ControlFlow::Break;
                };
                next += 1;
                let reading = model::NearAiBalance {
                    remaining_nanos: Some(*step),
                    ..template.clone()
                };
                let line = crate::balance::view(&reading).remaining_line;
                if !line.is_empty() {
                    section(&line);
                }
                glib::ControlFlow::Continue
            },
        );
        *self.flourish.count.borrow_mut() = Some(id);
    }

    /// Whether coins are on screen: for a test.
    pub fn bursting(&self) -> bool {
        self.flourish.start.get().is_some()
    }

    pub fn campaign_shown(&self) -> Option<bool> {
        self.campaign
            .as_ref()
            .map(|(_, hero)| hero.root.is_visible())
    }
}

/// The Missions hero's graphic (`HomeOverview.missionsHeroSymbol`) and the
/// Insights hero's without figures.
const MISSIONS_SYMBOL: &str = "scope";
const INSIGHTS_SYMBOL: &str = "chart.bar.xaxis";

/// A tool's name under its bar: the shell's name for it, the dash for the
/// daemon's "unknown" (`InferenceTabView.toolName`).
fn tool_name(tool: &str) -> String {
    if tool == "unknown" {
        "—".to_owned()
    } else {
        super::monitor::transfers::tool_name(tool)
    }
}

/// A line in the hero's ink.
fn hero_line(text: &str) -> gtk::Label {
    let line = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .build();
    line.add_css_class("tc-type-label");
    line.add_css_class("tc-hero-line");
    line
}

/// Reads what Home's heroes show that no other screen reads: today's
/// glance, and the mission drafts inbox while missions are shown.
pub fn refresh(app: &Rc<App>) {
    let tz = chrono::Local::now().offset().local_minus_utc();
    app.call(
        "insights_glance",
        serde_json::json!({ "tz": tz }),
        |app, result| {
            let glance = result
                .ok()
                .and_then(|value| serde_json::from_value::<Glance>(value).ok());
            app.home.render_glance(glance.as_ref());
        },
    );
    if app.home.missions.is_none() {
        return;
    }
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        use trace_commons_contributor::mission_draft_service::{
            self as service, MissionDraftOperation, MissionDraftRequest, MissionDraftResponse,
        };
        let ids = std::panic::catch_unwind(|| {
            match service::execute(MissionDraftRequest {
                store_dir: None,
                operation: MissionDraftOperation::List {},
            }) {
                Ok(MissionDraftResponse::List { drafts }) => {
                    Some(drafts.into_iter().map(|d| d.id).collect::<Vec<_>>())
                }
                _ => None,
            }
        })
        .ok()
        .flatten();
        let _ = tx.send_blocking(ids);
    });
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let Ok(ids) = rx.recv().await else { return };
        let Some(app) = weak.upgrade() else { return };
        // A failed read leaves the last answer, as the Mac's does.
        if let Some(ids) = ids {
            app.home.render_drafts(Some(&ids));
        }
    });
}

fn tile(title: &str) -> (gtk::Button, gtk::Label) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, space::S);
    let label = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .wrap(true)
        .build();
    let value = gtk::Label::builder().label("—").xalign(0.0).build();
    value.add_css_class("title-3");
    content.append(&label);
    content.append(&value);
    let button = gtk::Button::builder().child(&content).hexpand(true).build();
    button.add_css_class("tc-insights-summary");
    (button, value)
}

fn heading(title: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .wrap(true)
        .build();
    label.add_css_class("heading");
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn home_links_and_failed_reads_preserve_unknowns() {
        gtk::init().unwrap();
        let view = HomeView::new(false);
        assert!(
            view.links
                .iter()
                .all(|(_, target)| *target != "mission-drafts")
        );
        assert!(view.missions.is_none());
        let (insights, _) = view.insights.as_ref().expect("the Insights hero");
        assert_eq!(
            insights.subtitle.text(),
            trace_commons_contributor::insights::service::ui_copy()["intro"]
        );
        assert!(!insights.subtitle.text().contains("Mac"));
        assert!(
            view.links
                .iter()
                .any(|(button, target)| *target == "insights" && button == &insights.button)
        );
        for (_, target) in &view.links {
            // Every link names something the shell can open.
            assert!(
                super::super::monitor::tabs::route(
                    target,
                    super::super::monitor::tabs::Location::default(),
                    false
                )
                .is_some(),
                "{target} opens nothing"
            );
        }
        let status = serde_json::from_value(serde_json::json!({"decisions_owed": 4})).unwrap();
        view.render_status(Some(&status));
        assert_eq!(view.waiting.text(), "4");
        view.render_status(None);
        assert_eq!(view.waiting.text(), "—");
        let rollup =
            serde_json::from_value(serde_json::json!({"all_time": {"contributed": 12}})).unwrap();
        view.render_rollup(Some(&rollup));
        assert_eq!(view.contributed.text(), "12");
        view.render_rollup(None);
        assert_eq!(view.contributed.text(), "—");
        let enabled = HomeView::new(true);
        assert_eq!(
            enabled
                .links
                .iter()
                .filter(|(_, target)| *target == "mission-drafts")
                .count(),
            1
        );
    }

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn the_heroes_show_and_close_as_the_macs_do() {
        gtk::init().unwrap();
        let state = std::env::temp_dir().join(format!("tc-home-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&state);
        crate::home_cards::set_test_file(Some(state.clone()));
        let words = monitor_screens_copy();
        let view = HomeView::new(true);
        let (campaign, hero) = view.campaign.as_ref().expect("the compiled campaign");
        assert_eq!(hero.title.text(), campaign.title);
        assert_eq!(view.campaign_shown(), Some(true));
        // The X closes it as it is now, and it stays closed.
        view.close_campaign();
        assert_eq!(view.campaign_shown(), Some(false));
        assert_eq!(HomeView::new(true).campaign_shown(), Some(false));
        // Promotions off hides it.
        std::fs::write(&state, b"{}").unwrap();
        let fresh = HomeView::new(true);
        assert_eq!(fresh.campaign_shown(), Some(true));
        fresh.set_promotions(false);
        assert_eq!(fresh.campaign_shown(), Some(false));
        fresh.set_promotions(true);
        assert_eq!(fresh.campaign_shown(), Some(true));

        // Missions: a dash until the inbox answers, then its line.
        let (missions, line) = fresh.missions.as_ref().unwrap();
        assert_eq!(missions.title.text(), words.missions);
        assert_eq!(line.text(), "—");
        fresh.render_drafts(Some(&[]));
        assert_eq!(line.text(), words.home_history.no_mission_drafts);
        fresh.render_drafts(Some(&["m-1".into(), "m-2".into()]));
        assert_eq!(line.text(), "m-1 · +1");

        // Insights: the teaser without a glance, today's total with one.
        let (_, insights_line) = fresh.insights.as_ref().unwrap();
        assert_eq!(insights_line.text(), words.home_history.insights_teaser);
        let glance: Glance = serde_json::from_value(serde_json::json!({
            "enabled": true, "readable": true,
            "tools": [{"tool": "codex", "tokens": 1200}, {"tool": "claude-code", "tokens": 34}]
        }))
        .unwrap();
        fresh.render_glance(Some(&glance));
        assert_eq!(
            insights_line.text(),
            words.home_history.insights_today.replace("{total}", "1.2K")
        );
        fresh.render_glance(None);
        assert_eq!(insights_line.text(), words.home_history.insights_teaser);

        // Credit arriving: coins only when the received total grew.
        let read = |received: i64, remaining: i64| model::NearAiBalance {
            state: "known".into(),
            scale: Some(9),
            remaining_nanos: Some(remaining),
            received_total_nanos: Some(received),
            ..Default::default()
        };
        // Read while Home is not on screen: nothing is checked yet.
        fresh.balance_read(&read(10, 100));
        assert!(!fresh.bursting());
        let window = gtk::Window::builder()
            .default_width(700)
            .default_height(900)
            .child(&fresh.root)
            .build();
        window.present();
        let context = glib::MainContext::default();
        while !fresh.balance.card.is_mapped() {
            context.iteration(true);
        }
        // Shown: the first reading is remembered and marks nothing.
        fresh.check_balance();
        assert!(!fresh.bursting());
        fresh.balance_read(&read(10, 90));
        // Spending moves the figure, not the received total: nothing.
        assert!(!fresh.bursting());
        fresh.balance_read(&read(20, 190));
        assert!(fresh.bursting());
        // Off screen, the coins stop.
        window.set_child(None::<&gtk::Widget>);
        while fresh.balance.card.is_mapped() {
            context.iteration(true);
        }
        fresh.stop_flourish();
        assert!(!fresh.bursting());
        window.close();
        let _ = std::fs::remove_file(&state);
        crate::home_cards::set_test_file(None);
    }
}
