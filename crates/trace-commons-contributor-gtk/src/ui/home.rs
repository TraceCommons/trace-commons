//! An overview of current facts, linking to the existing full screens.
use super::{App, balance, private_inference, style::space};
use crate::{copy, model};
use adw::prelude::*;
use std::rc::Rc;
use trace_commons_contributor::preview_copy::monitor_screens_copy;

pub struct HomeView {
    pub root: gtk::Box,
    pub balance: balance::BalanceSection,
    waiting: gtk::Label,
    contributed: gtk::Label,
    inference: gtk::Box,
    links: Vec<(gtk::Button, &'static str)>,
}

impl HomeView {
    pub fn new(missions: bool) -> Self {
        let words = monitor_screens_copy();
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
        // The shared Monitor subtitle names macOS. Insights' own intro is
        // device-neutral and describes the local action this link opens.
        let insights_words = trace_commons_contributor::insights::service::ui_copy();
        let insights = shortcut(words.home_history.insights_action, &insights_words["intro"]);
        content.append(&insights);
        links.push((insights, "insights"));
        if missions {
            let missions = shortcut(
                words.home_history.missions_action,
                words.home_history.missions_subtitle,
            );
            content.append(&missions);
            links.push((missions, "mission-drafts"));
        }
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
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&scroll);
        let view = Self {
            root,
            balance,
            waiting,
            contributed,
            inference: inference_status,
            links,
        };
        view.render_inference(None);
        view
    }

    pub fn bind(&self, app: &Rc<App>) {
        for (button, target) in &self.links {
            let weak = Rc::downgrade(app);
            let target = *target;
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.stack.set_visible_child_name(target);
                }
            });
        }
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

fn shortcut(title: &str, subtitle: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Vertical, space::S);
    let label = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .wrap(true)
        .build();
    label.add_css_class("heading");
    content.append(&label);
    let subtitle = gtk::Label::builder()
        .label(subtitle)
        .xalign(0.0)
        .wrap(true)
        .build();
    subtitle.add_css_class("caption");
    content.append(&subtitle);
    let button = gtk::Button::builder().child(&content).build();
    button.add_css_class("tc-insights-card");
    button
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
        let insights = view
            .links
            .iter()
            .find(|(_, target)| *target == "insights")
            .unwrap();
        let subtitle = insights
            .0
            .child()
            .unwrap()
            .last_child()
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap();
        assert_eq!(
            subtitle.text(),
            trace_commons_contributor::insights::service::ui_copy()["intro"]
        );
        assert!(!subtitle.text().contains("Mac"));
        for (_, target) in &view.links {
            assert!(
                super::super::screens_with(false)
                    .iter()
                    .any(|(name, _, _)| name == target)
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
}
