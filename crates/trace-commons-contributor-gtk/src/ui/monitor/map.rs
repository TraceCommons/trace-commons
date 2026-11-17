//! Pending queue relationships, never a claim about collection or transfer.
use super::{App, inspector::Selection};
use crate::model::QueueEntry;
use adw::prelude::*;
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Scene {
    folders: Vec<Folder>,
    tools: Vec<(String, String, usize)>,
    pending: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Folder {
    id: String,
    label: String,
    count: usize,
    tools: Vec<String>,
}

impl Scene {
    fn new(entries: &[QueueEntry]) -> Self {
        let pending: Vec<_> = entries.iter().filter(|e| e.state == "pending").collect();
        let mut scene = Self {
            pending: pending.len(),
            ..Self::default()
        };
        for entry in &pending {
            if let Some(tool) = scene.tools.iter_mut().find(|t| t.0 == entry.source) {
                tool.2 += 1;
            } else {
                scene
                    .tools
                    .push((entry.source.clone(), entry.agent_label().into(), 1));
            }
        }
        for folder in crate::queue_folders::group(&pending) {
            let mut tools = Vec::new();
            for (_, entry) in &folder.members {
                if !tools.contains(&entry.source) {
                    tools.push(entry.source.clone());
                }
            }
            scene.folders.push(Folder {
                id: folder.project_id,
                label: folder.project_label,
                count: folder.members.len(),
                tools,
            });
        }
        scene
    }

    fn selected_folder<'a>(
        &'a self,
        selection: &Selection,
        entries: &[QueueEntry],
    ) -> Option<&'a Folder> {
        let id = match selection {
            Selection::Folder(id) => id,
            Selection::Session(id) => {
                &entries
                    .iter()
                    .find(|e| e.entry_id == *id && e.state == "pending")?
                    .project_id
            }
            Selection::None => return None,
        };
        self.folders.iter().find(|f| f.id == *id)
    }
}

type Nodes = Rc<RefCell<Vec<(gtk::Widget, gtk::Widget)>>>;

pub(super) struct TraceMap {
    pub root: gtk::Box,
    pub body: gtk::Box,
    folders: gtk::Box,
    tools: gtk::Box,
    computer: gtk::Box,
    lines: Nodes,
    drawing: gtk::DrawingArea,
    scene: RefCell<Scene>,
    folder_buttons: RefCell<Vec<(String, gtk::Button)>>,
    tool_labels: RefCell<Vec<(String, gtk::Label)>>,
}

impl TraceMap {
    pub fn new() -> Self {
        let words = trace_commons_contributor::preview_copy::monitor_screens_copy();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let toggle = gtk::ToggleButton::with_label(words.shell.show_map);
        toggle.set_halign(gtk::Align::Start);
        let reveal = gtk::Revealer::new();
        reveal.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        let settings = gtk::Settings::default();
        if let Some(settings) = settings {
            reveal.set_transition_duration(if settings.is_gtk_enable_animations() {
                180
            } else {
                0
            });
            let weak = reveal.downgrade();
            settings.connect_gtk_enable_animations_notify(move |settings| {
                if let Some(reveal) = weak.upgrade() {
                    reveal.set_transition_duration(if settings.is_gtk_enable_animations() {
                        180
                    } else {
                        0
                    });
                }
            });
        } else {
            reveal.set_transition_duration(0);
        }
        let weak = reveal.downgrade();
        toggle.connect_toggled(move |button| {
            let words = trace_commons_contributor::preview_copy::monitor_screens_copy();
            button.set_label(if button.is_active() {
                words.shell.hide_map
            } else {
                words.shell.show_map
            });
            if let Some(reveal) = weak.upgrade() {
                reveal.set_reveal_child(button.is_active());
            }
        });
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 48);
        body.set_margin_top(16);
        body.set_margin_bottom(16);
        body.set_margin_start(16);
        body.set_margin_end(16);
        let folders = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let tools = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let computer = gtk::Box::new(gtk::Orientation::Vertical, 12);
        for column in [&folders, &tools, &computer] {
            column.set_hexpand(true);
            column.set_valign(gtk::Align::Center);
            body.append(column);
        }
        let overlay = gtk::Overlay::new();
        let drawing = gtk::DrawingArea::new();
        drawing.set_hexpand(true);
        drawing.set_vexpand(true);
        drawing.set_can_target(false);
        drawing.set_accessible_role(gtk::AccessibleRole::Presentation);
        overlay.set_child(Some(&drawing));
        overlay.add_overlay(&body);
        overlay.set_measure_overlay(&body, true);
        let lines: Nodes = Rc::new(RefCell::new(Vec::new()));
        let edges = lines.clone();
        let weak_body = body.downgrade();
        drawing.set_draw_func(move |drawing, context, _, _| {
            let Some(body) = weak_body.upgrade() else {
                return;
            };
            let color = body.style_context().color();
            context.set_source_rgba(
                color.red().into(),
                color.green().into(),
                color.blue().into(),
                0.25,
            );
            context.set_line_width(1.0);
            for (from, to) in edges.borrow().iter() {
                let (Some(a), Some(b)) = (from.compute_bounds(drawing), to.compute_bounds(drawing))
                else {
                    continue;
                };
                if body.orientation() == gtk::Orientation::Horizontal {
                    context.move_to(
                        (a.x() + a.width()).into(),
                        (a.y() + a.height() / 2.0).into(),
                    );
                    context.line_to(b.x().into(), (b.y() + b.height() / 2.0).into());
                } else {
                    // Route beside the columns, never across intervening
                    // folder buttons or tool labels in the stacked layout.
                    let gutter = 4.0;
                    let ay = f64::from(a.y() + a.height() / 2.0);
                    let by = f64::from(b.y() + b.height() / 2.0);
                    context.move_to(a.x().into(), ay);
                    context.line_to(gutter, ay);
                    context.line_to(gutter, by);
                    context.line_to(b.x().into(), by);
                }
                let _ = context.stroke();
            }
        });
        let scroller = gtk::ScrolledWindow::builder()
            .child(&overlay)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(300)
            .propagate_natural_height(true)
            .build();
        scroller.update_property(&[gtk::accessible::Property::Label(words.flow_map.map_label)]);
        reveal.set_child(Some(&scroller));
        root.append(&toggle);
        root.append(&reveal);
        Self {
            root,
            body,
            folders,
            tools,
            computer,
            lines,
            drawing,
            scene: RefCell::new(Scene::default()),
            folder_buttons: RefCell::new(Vec::new()),
            tool_labels: RefCell::new(Vec::new()),
        }
    }

    pub fn orient(body: &gtk::Box, width: i32) {
        let orientation = if width < 600 {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        };
        if body.orientation() != orientation {
            body.set_orientation(orientation);
        }
    }

    pub fn render(&self, app: &Rc<App>) {
        let entries = app.entries.borrow();
        let scene = Scene::new(&entries);
        if *self.scene.borrow() != scene {
            let weak = Rc::downgrade(app);
            self.populate(
                &scene,
                Rc::new(move |id| {
                    if let Some(app) = weak.upgrade() {
                        app.queue.inspector.select(&app, Selection::Folder(id));
                    }
                }),
            );
            *self.scene.borrow_mut() = scene;
        }
        self.highlight(&app.queue.inspector.selection(), &entries);
    }

    fn populate(&self, scene: &Scene, select: Rc<dyn Fn(String)>) {
        self.lines.borrow_mut().clear();
        self.folder_buttons.borrow_mut().clear();
        self.tool_labels.borrow_mut().clear();
        for column in [&self.folders, &self.tools, &self.computer] {
            while let Some(child) = column.first_child() {
                column.remove(&child);
            }
        }
        self.root.set_visible(scene.pending != 0);
        let words = trace_commons_contributor::preview_copy::monitor_screens_copy();
        let hub = gtk::Label::new(Some(&format!(
            "{}\n{}",
            words.computer,
            words
                .flow_map
                .coin_waiting
                .replace("{count}", &scene.pending.to_string())
        )));
        hub.set_wrap(true);
        self.computer.append(&hub);
        for (id, label, count) in &scene.tools {
            let node = gtk::Label::new(Some(&format!(
                "{label}\n{}",
                words
                    .flow_map
                    .coin_waiting
                    .replace("{count}", &count.to_string())
            )));
            node.set_wrap(true);
            self.tools.append(&node);
            self.lines
                .borrow_mut()
                .push((node.clone().upcast(), hub.clone().upcast()));
            self.tool_labels.borrow_mut().push((id.clone(), node));
        }
        for folder in &scene.folders {
            let label = format!(
                "{}\n{}",
                folder.label,
                words
                    .flow_map
                    .coin_waiting
                    .replace("{count}", &folder.count.to_string())
            );
            let node = gtk::Button::new();
            let text = gtk::Label::new(Some(&label));
            text.set_wrap(true);
            node.set_child(Some(&text));
            node.update_property(&[gtk::accessible::Property::Label(&label)]);
            let id = folder.id.clone();
            let select = select.clone();
            node.connect_clicked(move |_| select(id.clone()));
            self.folders.append(&node);
            for (id, tool) in self.tool_labels.borrow().iter() {
                if folder.tools.contains(id) {
                    self.lines
                        .borrow_mut()
                        .push((node.clone().upcast(), tool.clone().upcast()));
                }
            }
            self.folder_buttons
                .borrow_mut()
                .push((folder.id.clone(), node));
        }
        self.drawing.queue_draw();
    }

    pub fn highlight(&self, selection: &Selection, entries: &[QueueEntry]) {
        let scene = self.scene.borrow();
        let folder = scene.selected_folder(selection, entries);
        for (id, node) in self.folder_buttons.borrow().iter() {
            if folder.is_some_and(|f| f.id == *id) {
                node.add_css_class("suggested-action");
            } else {
                node.remove_css_class("suggested-action");
            }
        }
        let selected_source = match selection {
            Selection::Session(id) => entries
                .iter()
                .find(|e| e.entry_id == *id && e.state == "pending")
                .map(|e| &e.source),
            _ => None,
        };
        for (id, node) in self.tool_labels.borrow().iter() {
            let selected = selected_source.map_or_else(
                || folder.is_some_and(|f| f.tools.contains(id)),
                |source| source == id,
            );
            if selected {
                node.add_css_class("accent");
            } else {
                node.remove_css_class("accent");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(id: &str, project: &str, source: &str, state: &str) -> QueueEntry {
        QueueEntry {
            entry_id: id.into(),
            project_id: project.into(),
            source: source.into(),
            state: state.into(),
            project_label: "same label".into(),
            ..Default::default()
        }
    }
    #[test]
    fn pending_relationships_use_ids_and_count_each_entry_once() {
        let entries = vec![
            entry("a", "p1", "codex", "pending"),
            entry("b", "p1", "claude-code", "pending"),
            entry("c", "p2", "codex", "pending"),
            entry("d", "p3", "codex", "approved"),
        ];
        let scene = Scene::new(&entries);
        assert_eq!(scene.pending, 3);
        assert_eq!(scene.folders.len(), 2);
        assert_eq!(scene.folders[0].tools, ["codex", "claude-code"]);
        assert_eq!(scene.tools[0].2, 2);
        assert_eq!(
            scene
                .selected_folder(&Selection::Session("c".into()), &entries)
                .unwrap()
                .id,
            "p2"
        );
        assert!(
            scene
                .selected_folder(&Selection::Session("d".into()), &entries)
                .is_none()
        );
        assert!(
            scene
                .selected_folder(&Selection::Folder("p3".into()), &entries)
                .is_none()
        );
    }
    #[test]
    fn selection_follows_identity_after_reordering_and_removal() {
        let mut entries = vec![
            entry("a", "p1", "codex", "pending"),
            entry("b", "p2", "codex", "pending"),
        ];
        entries.reverse();
        assert_eq!(
            Scene::new(&entries)
                .selected_folder(&Selection::Session("a".into()), &entries)
                .unwrap()
                .id,
            "p1"
        );
        entries.pop();
        assert!(
            Scene::new(&entries)
                .selected_folder(&Selection::Session("a".into()), &entries)
                .is_none()
        );
        assert_eq!(Scene::new(&[]), Scene::default());
    }
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_map_preserves_identity_accessibility_and_reduced_motion() {
        gtk::init().expect("GTK display");
        let map = TraceMap::new();
        let entries: Vec<_> = (0..40)
            .map(|i| entry(&format!("e{i}"), &format!("p{i}"), "codex", "pending"))
            .collect();
        let scene = Scene::new(&entries);
        let selected = Rc::new(RefCell::new(String::new()));
        let result = selected.clone();
        map.populate(&scene, Rc::new(move |id| *result.borrow_mut() = id));
        *map.scene.borrow_mut() = scene;
        let window = gtk::Window::builder()
            .default_width(480)
            .default_height(600)
            .child(&map.root)
            .build();
        window.present();
        let context = glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }
        TraceMap::orient(&map.body, 480);
        assert_eq!(map.body.orientation(), gtk::Orientation::Vertical);
        TraceMap::orient(&map.body, 800);
        assert_eq!(map.body.orientation(), gtk::Orientation::Horizontal);
        TraceMap::orient(&map.body, 480);
        let settings = gtk::Settings::default().expect("settings");
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(false);
        let toggle = map
            .root
            .first_child()
            .unwrap()
            .downcast::<gtk::ToggleButton>()
            .unwrap();
        toggle.set_active(true);
        let reveal = toggle
            .next_sibling()
            .unwrap()
            .downcast::<gtk::Revealer>()
            .unwrap();
        assert_eq!(reveal.transition_duration(), 0);
        assert!(reveal.reveals_child());
        assert_eq!(map.folder_buttons.borrow().len(), 40);
        let node = map.folder_buttons.borrow()[17].1.clone();
        assert!(node.is_focusable());
        assert_eq!(node.accessible_role(), gtk::AccessibleRole::Button);
        assert!(node.grab_focus());
        assert!(node.activate());
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            if !selected.borrow().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(node.width() > 0 && node.height() > 0);
        let scroller = reveal
            .child()
            .unwrap()
            .downcast::<gtk::ScrolledWindow>()
            .unwrap();
        let scroll = scroller.vadjustment();
        assert!(scroll.upper() > scroll.page_size());
        scroll.set_value(scroll.upper() - scroll.page_size());
        assert!(scroll.value() > 0.0);
        assert_eq!(*selected.borrow(), "p17");
        map.highlight(&Selection::Session("e17".into()), &entries);
        assert!(node.has_css_class("suggested-action"));
        map.highlight(&Selection::None, &entries);
        assert!(!node.has_css_class("suggested-action"));
        window.close();
        settings.set_gtk_enable_animations(previous);
    }
}
