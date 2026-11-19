//! Section navigation for the existing Settings controls. No daemon writes.

use super::super::style::{self, space};
use adw::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    Account,
    Watching,
    Tools,
    Projects,
    Uses,
    Profile,
    Changes,
    PrivateAI,
    General,
    Startup,
    Witness,
}

impl Section {
    pub const ALL: [Self; 11] = [
        Self::Account,
        Self::Watching,
        Self::Tools,
        Self::Projects,
        Self::Uses,
        Self::Profile,
        Self::Changes,
        Self::PrivateAI,
        Self::General,
        Self::Startup,
        Self::Witness,
    ];

    pub fn title(self) -> &'static str {
        let nav = trace_commons_contributor::preview_copy::monitor_screens_copy().settings_nav;
        match self {
            Self::Account => nav.account,
            Self::Watching => nav.watching,
            Self::Tools => nav.tools,
            Self::Projects => nav.projects,
            Self::Uses => nav.uses,
            Self::Profile => nav.profile,
            Self::Changes => nav.log,
            Self::PrivateAI => nav.private_ai,
            Self::General => nav.general,
            Self::Startup => nav.startup,
            Self::Witness => nav.witness,
        }
    }

    fn starts_group(self) -> bool {
        matches!(
            self,
            Self::Watching | Self::Projects | Self::PrivateAI | Self::General | Self::Witness
        )
    }
}

pub(super) struct Sections {
    bodies: Vec<(Section, gtk::Box)>,
}

impl Sections {
    pub fn new() -> Self {
        Self {
            bodies: Section::ALL
                .into_iter()
                .map(|section| {
                    let body = gtk::Box::new(gtk::Orientation::Vertical, space::M);
                    // Public profile owns its state-dependent native/brand heading.
                    if section != Section::Profile {
                        body.append(&style::section(section.title()));
                    }
                    (section, body)
                })
                .collect(),
        }
    }

    pub fn body(&self, section: Section) -> &gtk::Box {
        &self
            .bodies
            .iter()
            .find(|(id, _)| *id == section)
            .expect("known section")
            .1
    }
}

fn scroll_target(y: f64, upper: f64, page_size: f64) -> f64 {
    y.clamp(0.0, (upper - page_size).max(0.0))
}

pub(super) struct Navigation {
    pub root: gtk::Box,
    /// Selects a section and scrolls to it, now or, before the window has
    /// been laid out, on its first frame that can scroll.
    pub select: std::rc::Rc<dyn Fn(Section)>,
    // Keep these for the real-widget regression test; production ownership
    // belongs to the GTK tree, not a second persistent view model.
    #[cfg(test)]
    sidebar: gtk::ScrolledWindow,
    #[cfg(test)]
    compact: gtk::MenuButton,
    #[cfg(test)]
    compact_choices: Vec<gtk::Button>,
    #[cfg(test)]
    list: gtk::ListBox,
    #[cfg(test)]
    scroller: gtk::ScrolledWindow,
}

impl Navigation {
    pub fn new(sections: Sections) -> Self {
        let words = trace_commons_contributor::preview_copy::monitor_screens_copy();
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(space::XL)
            .margin_top(space::L)
            .margin_bottom(space::XXL)
            .margin_start(space::XL)
            .margin_end(space::XL)
            .build();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        list.add_css_class("navigation-sidebar");
        list.update_property(&[gtk::accessible::Property::Label(words.settings_sections)]);
        let mut anchors = Vec::new();
        for (section, body) in sections.bodies {
            let row = gtk::ListBoxRow::new();
            let label = gtk::Label::builder()
                .label(section.title())
                .xalign(0.0)
                .wrap(true)
                .margin_start(space::M)
                .margin_end(space::M)
                .margin_top(space::S)
                .margin_bottom(space::S)
                .build();
            row.set_child(Some(&label));
            if section.starts_group() {
                let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
                separator.set_margin_top(space::S);
                separator.set_margin_bottom(space::S);
                row.set_header(Some(&separator));
            }
            list.append(&row);
            content.append(&body);
            anchors.push(body.downgrade());
        }
        let compact = gtk::MenuButton::builder()
            .label(Section::Account.title())
            .build();
        let choices = gtk::Box::new(gtk::Orientation::Vertical, space::XS);
        let compact_choices: Vec<_> = Section::ALL
            .iter()
            .map(|section| {
                let button = gtk::Button::with_label(section.title());
                button.add_css_class("flat");
                choices.append(&button);
                button
            })
            .collect();
        let popover = gtk::Popover::builder().child(&choices).build();
        compact.set_popover(Some(&popover));
        compact.set_margin_start(space::XL);
        compact.set_margin_end(space::XL);
        compact.set_margin_top(space::S);
        compact.set_margin_bottom(space::S);
        compact.update_property(&[gtk::accessible::Property::Label(words.settings_sections)]);
        let sidebar = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(190)
            .child(&list)
            .build();
        let clamp = adw::Clamp::builder()
            .maximum_size(720)
            .tightening_threshold(540)
            .child(&content)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .vexpand(true)
            .child(&clamp)
            .build();
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.append(&sidebar);
        body.append(&scroller);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("tc-root");
        root.append(&compact);
        root.append(&body);
        body.set_vexpand(true);
        let navigate = std::rc::Rc::new({
            let compact = compact.downgrade();
            let content = content.downgrade();
            let scroller = scroller.downgrade();
            move |index: usize| {
                if let Some(compact) = compact.upgrade() {
                    compact.set_label(Section::ALL[index].title());
                }
                let (Some(anchor), Some(content), Some(scroller)) = (
                    anchors.get(index).and_then(|a| a.upgrade()),
                    content.upgrade(),
                    scroller.upgrade(),
                ) else {
                    return;
                };
                if let Some(point) =
                    anchor.compute_point(&content, &gtk::graphene::Point::new(0.0, 0.0))
                {
                    let adjustment = scroller.vadjustment();
                    adjustment.set_value(scroll_target(
                        f64::from(point.y()),
                        adjustment.upper(),
                        adjustment.page_size(),
                    ));
                }
            }
        });
        // A section choice is an action, including the current section.
        // DropDown's selected-notify cannot represent reselecting the same
        // value after manual scrolling, so each compact row is a button.
        for (index, button) in compact_choices.iter().enumerate() {
            let list = list.downgrade();
            let popover = popover.downgrade();
            let navigate = navigate.clone();
            button.connect_clicked(move |_| {
                if let Some(list) = list.upgrade() {
                    list.select_row(list.row_at_index(index as i32).as_ref());
                }
                navigate(index);
                if let Some(popover) = popover.upgrade() {
                    popover.popdown();
                }
            });
        }
        list.connect_row_selected({
            let navigate = navigate.clone();
            move |_, row| {
                if let Some(row) = row {
                    navigate(row.index() as usize);
                }
            }
        });
        // Activating the already-selected section still returns to it after
        // manual scrolling; selection-changed alone would leave a dead row.
        list.connect_row_activated({
            let navigate = navigate.clone();
            move |_, row| navigate(row.index() as usize)
        });
        // A section asked for from outside, scrolled to on the first frame
        // the content is laid out in: before that there is nowhere to
        // scroll, and the window may not have been shown yet.
        let pending: std::rc::Rc<std::cell::Cell<Option<usize>>> = std::rc::Rc::default();
        let select: std::rc::Rc<dyn Fn(Section)> = std::rc::Rc::new({
            let list = list.downgrade();
            let pending = pending.clone();
            move |section: Section| {
                let index = Section::ALL.iter().position(|s| *s == section).unwrap_or(0);
                if let Some(list) = list.upgrade() {
                    list.select_row(list.row_at_index(index as i32).as_ref());
                }
                pending.set(Some(index));
            }
        });
        // Written when the libadwaita floor was 1.2, which has no
        // Breakpoint; the floor is 1.4 since 2026-11-16 and this has not
        // been moved to one. Allocation is checked at frame time and only
        // visibility changes; fields are never rebuilt. Minimised or
        // suspended, the check comes off the clock, and goes back on when
        // the window is shown.
        let frame: std::rc::Rc<dyn Fn(&gtk::Box)> = std::rc::Rc::new({
            let sidebar = sidebar.downgrade();
            let compact = compact.downgrade();
            let scroller = scroller.downgrade();
            let navigate = navigate.clone();
            move |root: &gtk::Box| {
                if let (Some(sidebar), Some(compact)) = (sidebar.upgrade(), compact.upgrade()) {
                    let wide = root.width() >= 760;
                    sidebar.set_visible(wide);
                    compact.set_visible(!wide);
                }
                if let Some(index) = pending.get()
                    && let Some(scroller) = scroller.upgrade()
                    && scroller.is_mapped()
                    && scroller.vadjustment().page_size() > 0.0
                {
                    pending.set(None);
                    navigate(index);
                }
            }
        });
        let tick: std::rc::Rc<std::cell::RefCell<Option<gtk::TickCallbackId>>> =
            std::rc::Rc::default();
        let start: std::rc::Rc<dyn Fn(&gtk::Box)> = std::rc::Rc::new({
            let tick = tick.clone();
            move |root: &gtk::Box| {
                if tick.borrow().is_some() || crate::ui::glass_motion::window_hidden(root) {
                    return;
                }
                let frame = frame.clone();
                let slot = tick.clone();
                let id = root.add_tick_callback(move |root, _| {
                    if crate::ui::glass_motion::window_hidden(root) {
                        // Ended by returning Break; shown again, it restarts.
                        drop(slot.borrow_mut().take());
                        return gtk::glib::ControlFlow::Break;
                    }
                    frame(root);
                    gtk::glib::ControlFlow::Continue
                });
                *tick.borrow_mut() = Some(id);
            }
        });
        start(&root);
        crate::ui::glass_motion::on_window_state(&root, {
            let tick = tick.clone();
            move |widget| {
                let Some(root) = widget.downcast_ref::<gtk::Box>() else {
                    return;
                };
                if crate::ui::glass_motion::window_hidden(root) {
                    if let Some(id) = tick.borrow_mut().take() {
                        id.remove();
                    }
                } else {
                    start(root);
                }
            }
        });
        list.select_row(list.row_at_index(0).as_ref());
        Self {
            root,
            select,
            #[cfg(test)]
            sidebar,
            #[cfg(test)]
            compact,
            #[cfg(test)]
            compact_choices,
            #[cfg(test)]
            list,
            #[cfg(test)]
            scroller,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_follow_the_supported_macos_groups_and_shared_names() {
        let nav = trace_commons_contributor::preview_copy::monitor_screens_copy().settings_nav;
        assert_eq!(
            Section::ALL.map(Section::title),
            [
                nav.account,
                nav.watching,
                nav.tools,
                nav.projects,
                nav.uses,
                nav.profile,
                nav.log,
                nav.private_ai,
                nav.general,
                nav.startup,
                nav.witness
            ]
        );
        // The Mac's App group (General, Startup) starts at General.
        assert_eq!(
            Section::ALL.map(Section::starts_group),
            [
                false, true, false, true, false, false, false, true, true, false, true
            ]
        );
    }

    #[test]
    fn section_scroll_stays_inside_the_viewport_range() {
        assert_eq!(scroll_target(250.0, 1000.0, 400.0), 250.0);
        assert_eq!(scroll_target(900.0, 1000.0, 400.0), 600.0);
        assert_eq!(scroll_target(-20.0, 1000.0, 400.0), 0.0);
        assert_eq!(scroll_target(150.0, 200.0, 400.0), 0.0);
    }

    #[test]
    #[ignore = "requires a GTK display; run separately with --ignored --test-threads=1"]
    fn gtk_section_choices_scroll_and_resize_preserves_fields() {
        adw::init().expect("GTK display unavailable");
        let sections = Sections::new();
        let field = gtk::Entry::new();
        sections.body(Section::Account).append(&field);
        for section in Section::ALL {
            sections.body(section).set_size_request(-1, 240);
        }
        let navigation = Navigation::new(sections);
        let window = gtk::Window::builder()
            .default_width(1000)
            .default_height(600)
            .child(&navigation.root)
            .build();
        window.present();
        wait_for_layout(&navigation, true);
        assert!(navigation.sidebar.is_visible());
        assert!(!navigation.compact.is_visible());
        field.set_text("unsaved input");
        navigation
            .list
            .select_row(navigation.list.row_at_index(3).as_ref());
        settle();
        assert_eq!(
            navigation.compact.label().as_deref(),
            Some(Section::Projects.title())
        );
        assert!(navigation.scroller.vadjustment().value() > 0.0);
        let before = navigation.scroller.vadjustment().value();
        navigation.scroller.vadjustment().set_value(0.0);
        let row = navigation.list.row_at_index(3).unwrap();
        navigation.list.emit_by_name::<()>("row-activated", &[&row]);
        settle();
        assert_eq!(navigation.scroller.vadjustment().value(), before);
        navigation.compact_choices[0].emit_clicked();
        settle();
        assert_eq!(navigation.list.selected_row().unwrap().index(), 0);
        assert!(navigation.scroller.vadjustment().value() < before);
        window.set_default_size(550, 600);
        wait_for_layout(&navigation, false);
        assert!(!navigation.sidebar.is_visible());
        assert!(navigation.compact.is_visible());
        let witness = Section::ALL
            .iter()
            .position(|section| *section == Section::Witness)
            .unwrap();
        navigation.compact_choices[witness].emit_clicked();
        settle();
        assert_eq!(
            navigation.list.selected_row().unwrap().index(),
            witness as i32
        );
        assert!(navigation.scroller.vadjustment().value() > 0.0);
        let witness_scroll = navigation.scroller.vadjustment().value();
        navigation.scroller.vadjustment().set_value(0.0);
        settle();
        // The label still names Witness, but choosing it again is an action.
        assert_eq!(
            navigation.compact.label().as_deref(),
            Some(Section::Witness.title())
        );
        navigation.compact_choices[witness].emit_clicked();
        settle();
        assert_eq!(navigation.scroller.vadjustment().value(), witness_scroll);
        window.set_default_size(1000, 600);
        wait_for_layout(&navigation, true);
        assert!(navigation.sidebar.is_visible());
        assert_eq!(
            navigation.compact.label().as_deref(),
            Some(Section::Witness.title())
        );
        assert_eq!(field.text(), "unsaved input");
        window.close();
    }

    /// Settings opened at a section from outside (the edge rail's and the
    /// popover's buttons) scrolls there even when asked before the window
    /// has ever been laid out.
    #[test]
    #[ignore = "requires a GTK display; run separately with --ignored --test-threads=1"]
    fn mounted_settings_opened_at_a_section_scrolls_there_on_first_show() {
        adw::init().expect("GTK display unavailable");
        let sections = Sections::new();
        for section in Section::ALL {
            sections.body(section).set_size_request(-1, 240);
        }
        let navigation = Navigation::new(sections);
        (navigation.select)(Section::Startup);
        let window = gtk::Window::builder()
            .default_width(1000)
            .default_height(600)
            .child(&navigation.root)
            .build();
        window.present();
        wait_for_layout(&navigation, true);
        settle();
        let startup = Section::ALL
            .iter()
            .position(|section| *section == Section::Startup)
            .expect("Startup");
        assert_eq!(
            navigation.list.selected_row().unwrap().index() as usize,
            startup
        );
        assert!(
            navigation.scroller.vadjustment().value() > 0.0,
            "left at the top"
        );
        // Asked again once shown, from somewhere else.
        navigation.scroller.vadjustment().set_value(0.0);
        (navigation.select)(Section::PrivateAI);
        settle();
        assert!(navigation.scroller.vadjustment().value() > 0.0);
        window.close();
    }

    // Wayland configure/allocate and the responsive tick are asynchronous.
    // A default size is a request, not evidence a frame has that size. The
    // hosted failure occurred before that old fixed 180ms wait observed a
    // wide layout; renderer startup also reported unavailable GPU drivers.
    // Require the actual allocated layout, keeping wide/narrow assertions
    // equally strict and reporting enough state to distinguish no configure
    // from a responsive control that failed to follow its allocation.
    fn wait_for_layout(navigation: &Navigation, wide: bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            while gtk::glib::MainContext::default().pending() {
                gtk::glib::MainContext::default().iteration(false);
            }
            let width = navigation.root.width();
            let ready = navigation.root.is_mapped()
                && width > 0
                && (width >= 760) == wide
                && navigation.sidebar.is_mapped() == wide
                && navigation.compact.is_mapped() != wide
                && if wide {
                    navigation.sidebar.width() > 0
                } else {
                    navigation.compact.width() > 0
                }
                && navigation.scroller.vadjustment().page_size() > 0.0;
            if ready {
                eprintln!("Settings layout ready: wide={wide}, allocated width={width}");
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Settings layout timed out: requested wide={wide}, root mapped={}, allocated={}x{}, sidebar visible={} mapped={} width={}, compact visible={} mapped={} width={}, scroll page={}",
                navigation.root.is_mapped(),
                width,
                navigation.root.height(),
                navigation.sidebar.is_visible(),
                navigation.sidebar.is_mapped(),
                navigation.sidebar.width(),
                navigation.compact.is_visible(),
                navigation.compact.is_mapped(),
                navigation.compact.width(),
                navigation.scroller.vadjustment().page_size()
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn settle() {
        let until = std::time::Instant::now() + std::time::Duration::from_millis(180);
        while std::time::Instant::now() < until {
            while gtk::glib::MainContext::default().pending() {
                gtk::glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}
