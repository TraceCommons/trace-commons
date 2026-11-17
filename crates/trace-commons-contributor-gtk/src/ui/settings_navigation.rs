//! Section navigation for the existing Settings controls. No daemon writes.

use super::super::style::{self, space};
use adw::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Section {
    Account,
    Watching,
    Tools,
    Projects,
    Profile,
    Changes,
    PrivateAI,
    Startup,
    Witness,
}

impl Section {
    pub const ALL: [Self; 9] = [
        Self::Account,
        Self::Watching,
        Self::Tools,
        Self::Projects,
        Self::Profile,
        Self::Changes,
        Self::PrivateAI,
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
            Self::Profile => nav.profile,
            Self::Changes => nav.log,
            Self::PrivateAI => nav.private_ai,
            Self::Startup => nav.startup,
            Self::Witness => nav.witness,
        }
    }

    fn starts_group(self) -> bool {
        matches!(
            self,
            Self::Watching | Self::Projects | Self::PrivateAI | Self::Startup | Self::Witness
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
        list.connect_row_activated(move |_, row| navigate(row.index() as usize));
        // GTK 4 / libadwaita 1.2 has no Breakpoint. Allocation is checked at
        // frame time and only visibility changes; fields are never rebuilt.
        root.add_tick_callback({
            let sidebar = sidebar.downgrade();
            let compact = compact.downgrade();
            move |root, _| {
                if let (Some(sidebar), Some(compact)) = (sidebar.upgrade(), compact.upgrade()) {
                    let wide = root.width() >= 760;
                    sidebar.set_visible(wide);
                    compact.set_visible(!wide);
                }
                gtk::glib::ControlFlow::Continue
            }
        });
        list.select_row(list.row_at_index(0).as_ref());
        Self {
            root,
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
                nav.profile,
                nav.log,
                nav.private_ai,
                nav.startup,
                nav.witness
            ]
        );
        assert_eq!(
            Section::ALL.map(Section::starts_group),
            [false, true, false, true, false, false, true, true, true]
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
        navigation.compact_choices[8].emit_clicked();
        settle();
        assert_eq!(navigation.list.selected_row().unwrap().index(), 8);
        assert!(navigation.scroller.vadjustment().value() > 0.0);
        let witness_scroll = navigation.scroller.vadjustment().value();
        navigation.scroller.vadjustment().set_value(0.0);
        settle();
        // The label still names Witness, but choosing it again is an action.
        assert_eq!(
            navigation.compact.label().as_deref(),
            Some(Section::Witness.title())
        );
        navigation.compact_choices[8].emit_clicked();
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
