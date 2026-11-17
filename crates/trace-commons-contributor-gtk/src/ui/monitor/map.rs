//! The window's map pane: the Traces map.
//!
//! A native port of `FlowMapView` and `MapToolPile` in
//! `macos/Sources/TraceCommonsApp/Views/Monitor/`, drawing `map_scene`'s
//! scene. The arcs and the nodes' drawings are painted by one custom
//! widget; each node is also a real child button, focusable, named by the
//! node's label and described by its card, so a keyboard and a screen
//! reader inspect what a pointer can. A node card shows on hover or
//! keyboard focus; a folder click selects the folder in the tree, switches
//! to Traces and opens the inspector. The legend and the zoom sit along the
//! bottom, the Traces / Private AI switch in the top corner. The map is
//! drawn whatever is pending, nothing included (spec, "No hiding").
//!
//! Inputs are the daemon reads the Mac's map uses: `list_pending`,
//! `list_projects`, `get_settings`, `status`, `tool_destinations` and
//! `list_history`, through the window's worker.
//!
//! Deliberate platform differences, each until a later slice of the
//! 2026-11-16 spec:
//! - Nothing moves yet. A flowing arc is drawn lit and still, as the Mac
//!   draws it under Reduce Motion; the moving dashes, the transfers and the
//!   frame clock are the transfers slice.
//! - The pile opens at once under the pointer, where the Mac stages it
//!   (taller, then wider, then the names).
//! - The library is a disc and the coins carry the tools' initials: the
//!   station art, the tools' logos and tints are the look slice. Colours
//!   are the shell's status tokens and the theme's ink.
//! - The Private AI view is drawn empty: its scene is the Private AI slice.
//! - The legend's memory item and the coins' lock say "this computer"
//!   (`legend_memory_computer`) where the Mac's say "this Mac".
//! - The core's line when the map is not current, and the other devices
//!   on the account, are not drawn: GTK reads neither yet.
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use gtk::subclass::prelude::*;
use trace_commons_contributor::preview_copy::monitor_screens_copy;

use super::map_scene::{
    self, ArcStyle, CommonsGate, Contributions, Lamp, MapGeometry, NodeKind, Point, Scene,
    ScreenState, Size, ToolStack,
};
use super::tabs::MapView;
use super::traces_tree::{ProjectMode, SourceKind, TracesTree};
use crate::model::{GateStatus, HistoryRecord, Project, QueueEntry, Settings, ToolDestinations};
use crate::ui::App;

/// What the scene is built from, apart from the pane's size.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneInputs {
    pub tree: TracesTree,
    pub gate: CommonsGate,
    pub contributed: Contributions,
    /// The selection's tool: ringed, the others faded.
    pub selected_tool: Option<String>,
    /// The binoculars' tool: the camera centres on it.
    pub focus_tool: Option<String>,
}

impl Default for SceneInputs {
    fn default() -> Self {
        Self {
            tree: TracesTree::default(),
            gate: CommonsGate::with_state(ScreenState::Loading),
            contributed: Contributions::unread(),
            selected_tool: None,
            focus_tool: None,
        }
    }
}

type FolderCallback = Box<dyn Fn(String)>;
type Card = (gtk::Box, gtk::Label, gtk::Label);

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MapCanvas {
        pub inputs: RefCell<SceneInputs>,
        pub revealed: RefCell<BTreeMap<String, usize>>,
        pub scene: RefCell<Scene>,
        pub zoom: Cell<f64>,
        /// The size the scene was last laid out for.
        pub size: Cell<(i32, i32)>,
        pub hovered: RefCell<Option<String>>,
        pub focused: RefCell<Option<String>>,
        /// Whether the tool pile is open: the pointer is over its card.
        pub pile_open: Cell<bool>,
        /// Each node's button, by node id, in keyboard order.
        pub nodes: RefCell<Vec<(String, gtk::Button)>>,
        pub pill: RefCell<Option<gtk::Button>>,
        pub card: RefCell<Option<Card>>,
        pub on_select_folder: RefCell<Option<FolderCallback>>,
        /// The legend's memory item, shown while the tool group is drawn.
        pub memory_legend: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MapCanvas {
        const NAME: &'static str = "TcTraceMapCanvas";
        type Type = super::MapCanvas;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }
    }

    impl ObjectImpl for MapCanvas {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            self.zoom.set(1.0);
            obj.set_overflow(gtk::Overflow::Hidden);
            obj.set_hexpand(true);
            obj.set_vexpand(true);

            let pill = gtk::Button::new();
            pill.add_css_class("tc-map-pill");
            pill.set_visible(false);
            pill.set_parent(&*obj);
            let weak = obj.downgrade();
            pill.connect_clicked(move |_| {
                if let Some(obj) = weak.upgrade() {
                    obj.reveal();
                }
            });
            self.pill.replace(Some(pill));

            let card = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(4)
                .accessible_role(gtk::AccessibleRole::Presentation)
                .can_target(false)
                .visible(false)
                .build();
            card.add_css_class("tc-map-card");
            let title = gtk::Label::new(None);
            title.add_css_class("heading");
            title.set_xalign(0.0);
            title.set_wrap(true);
            let detail = gtk::Label::new(None);
            detail.set_xalign(0.0);
            detail.set_wrap(true);
            card.append(&title);
            card.append(&detail);
            card.set_parent(&*obj);
            self.card.replace(Some((card, title, detail)));

            // The pile opens only while the pointer is over its card.
            let motion = gtk::EventControllerMotion::new();
            let weak = obj.downgrade();
            motion.connect_motion(move |_, x, y| {
                if let Some(obj) = weak.upgrade() {
                    obj.imp().point_at(Some((x, y)));
                }
            });
            let weak = obj.downgrade();
            motion.connect_leave(move |_| {
                if let Some(obj) = weak.upgrade() {
                    obj.imp().point_at(None);
                }
            });
            obj.add_controller(motion);

            let weak = obj.downgrade();
            adw::StyleManager::default().connect_dark_notify(move |_| {
                if let Some(obj) = weak.upgrade() {
                    obj.queue_draw();
                }
            });
        }

        fn dispose(&self) {
            for (_, button) in self.nodes.take() {
                button.unparent();
            }
            if let Some(pill) = self.pill.take() {
                pill.unparent();
            }
            if let Some((card, _, _)) = self.card.take() {
                card.unparent();
            }
        }
    }

    impl WidgetImpl for MapCanvas {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            // Every child is measured, so each may be allocated; the map
            // itself asks for nothing and takes what the pane gives.
            for child in self.children() {
                child.measure(orientation, -1);
            }
            let natural = match orientation {
                gtk::Orientation::Horizontal => map_scene::SIZE.width / 2.0,
                _ => map_scene::SIZE.height / 2.0,
            };
            (0, natural as i32, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            if self.size.replace((width, height)) != (width, height) {
                // The scene takes the pane's shape; its nodes are the same.
                self.layout_scene();
            }
            let fit = self.geometry();
            let scene = self.scene.borrow();
            let open = self.pile_open.get();
            for (id, button) in self.nodes.borrow().iter() {
                let Some(node) = scene.nodes.iter().find(|n| n.id == *id) else {
                    button.set_child_visible(false);
                    continue;
                };
                button.set_child_visible(true);
                let at = coin_index(&scene, id)
                    .and_then(|i| Some(scene.stack.as_ref()?.coin_center(i, open)))
                    .unwrap_or(node.at);
                let centre = fit.point(at);
                let side = 24.0_f64.max((node.radius * 2.0 + 8.0) * fit.scale);
                button.size_allocate(
                    &gtk::Allocation::new(
                        (centre.x - side / 2.0) as i32,
                        (centre.y - side / 2.0) as i32,
                        side as i32,
                        side as i32,
                    ),
                    -1,
                );
            }
            if let Some(pill) = self.pill.borrow().as_ref() {
                match scene.reveals.first() {
                    Some(reveal) => {
                        let (_, w, _, _) = pill.measure(gtk::Orientation::Horizontal, -1);
                        let (_, h, _, _) = pill.measure(gtk::Orientation::Vertical, w);
                        // Its trailing edge where the folder names end, on
                        // the line under the last one.
                        let unit = fit.scale.max(map_scene::LABEL_FLOOR);
                        let column = fit.point(reveal.at);
                        let end = column.x - map_scene::FOLDER_RADIUS * fit.scale - 6.0 * unit;
                        pill.size_allocate(
                            &gtk::Allocation::new(
                                (end - f64::from(w)) as i32,
                                (column.y - f64::from(h) / 2.0) as i32,
                                w,
                                h,
                            ),
                            -1,
                        );
                    }
                    None => pill.size_allocate(&gtk::Allocation::new(0, 0, 0, 0), -1),
                }
            }
            if let Some((card, _, _)) = self.card.borrow().as_ref() {
                let shown = self
                    .selection()
                    .and_then(|id| scene.nodes.iter().find(|n| n.id == id).cloned());
                match shown {
                    Some(node) => {
                        let field = f64::from(width);
                        let card_width = map_scene::NODE_CARD_WIDTH
                            .min(field - 2.0 * map_scene::CARD_MARGIN)
                            .max(1.0);
                        let (_, h, _, _) =
                            card.measure(gtk::Orientation::Vertical, card_width as i32);
                        let centre = fit.point(node.at);
                        let half =
                            map_scene::drawn_half_width(&node, scene.stack.is_some()) * fit.scale;
                        let x = map_scene::card_x(centre.x, half, card_width, field);
                        let y = map_scene::card_y(centre.y, f64::from(height));
                        card.size_allocate(
                            &gtk::Allocation::new(
                                (x - card_width / 2.0) as i32,
                                (y - f64::from(h) / 2.0) as i32,
                                card_width as i32,
                                h,
                            ),
                            -1,
                        );
                    }
                    None => card.size_allocate(&gtk::Allocation::new(0, 0, 0, 0), -1),
                }
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let (width, height) = (obj.width(), obj.height());
            if width <= 0 || height <= 0 {
                return;
            }
            let fit = self.geometry();
            let scene = self.scene.borrow();
            let palette = Palette::current(&*obj);
            let bounds = gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
            let cr = snapshot.append_cairo(&bounds);
            let healthy = self.inputs.borrow().gate.state.is_healthy();
            draw_arcs(&cr, &scene, &fit, &palette, healthy);
            let selection = self.selection();
            for node in &scene.nodes {
                let selected = selection.as_deref() == Some(node.id.as_str());
                draw_node(&cr, &scene, node, &fit, &palette, selected);
            }
            if let Some(stack) = scene.stack.as_ref() {
                draw_pile(&cr, &scene, stack, &fit, &palette, self.pile_open.get());
            }
            drop(cr);
            self.draw_words(snapshot, &scene, &fit, &palette);
            for child in self.children() {
                obj.snapshot_child(&child, snapshot);
            }
        }
    }

    impl MapCanvas {
        /// The buttons, then the pill, then the card: the order they paint.
        pub fn children(&self) -> Vec<gtk::Widget> {
            let mut children: Vec<gtk::Widget> = self
                .nodes
                .borrow()
                .iter()
                .map(|(_, b)| b.clone().upcast())
                .collect();
            if let Some(pill) = self.pill.borrow().as_ref() {
                children.push(pill.clone().upcast());
            }
            if let Some((card, _, _)) = self.card.borrow().as_ref() {
                children.push(card.clone().upcast());
            }
            children
        }

        /// The node whose ring and card are drawn: the hovered one, then
        /// the one with keyboard focus. A click keeps no card open.
        pub fn selection(&self) -> Option<String> {
            self.hovered
                .borrow()
                .clone()
                .or_else(|| self.focused.borrow().clone())
        }

        pub fn geometry(&self) -> MapGeometry {
            let (width, height) = self.size.get();
            let scene = self.scene.borrow();
            let inputs = self.inputs.borrow();
            MapGeometry::new(
                Size::new(f64::from(width.max(1)), f64::from(height.max(1))),
                self.zoom.get(),
                scene.focus_point(inputs.focus_tool.as_deref()),
                scene.space,
            )
        }

        /// The scene for the current inputs, in the pane's shape.
        pub fn layout_scene(&self) {
            let (width, height) = self.size.get();
            let space = Scene::space_for(Size::new(f64::from(width), f64::from(height)));
            let inputs = self.inputs.borrow();
            let scene = Scene::traces(
                &inputs.tree,
                &inputs.gate,
                &inputs.contributed,
                inputs.selected_tool.as_deref(),
                &self.revealed.borrow(),
                space,
            );
            // The lock's legend item goes with the coins that carry it, as
            // the Mac draws `legend_memory` only with its pile. Here, where
            // every layout passes, not only a change of inputs.
            let has_pile = scene.stack.is_some();
            self.scene.replace(scene);
            if let Some(item) = self
                .memory_legend
                .borrow()
                .as_ref()
                .and_then(|item| item.upgrade())
                && item.get_visible() != has_pile
            {
                // Not inside this layout pass: a layout runs from the
                // canvas's own allocation.
                let item = item.downgrade();
                glib::idle_add_local_once(move || {
                    if let Some(item) = item.upgrade() {
                        item.set_visible(has_pile);
                    }
                });
            }
        }

        fn point_at(&self, pointer: Option<(f64, f64)>) {
            let open = self.pile_open.get();
            let inside = {
                let scene = self.scene.borrow();
                let Some(stack) = scene.stack.as_ref() else {
                    return;
                };
                let (x, y, w, h) = stack.card(&self.geometry(), open, open);
                pointer.is_some_and(|(px, py)| px >= x && px <= x + w && py >= y && py <= y + h)
            };
            if inside != open {
                self.pile_open.set(inside);
                let obj = self.obj();
                obj.queue_allocate();
                obj.queue_draw();
            }
        }

        fn layout(&self, text: &str, size: f64, bold: bool) -> gtk::pango::Layout {
            let layout = self.obj().create_pango_layout(Some(text));
            let mut font = gtk::pango::FontDescription::new();
            font.set_absolute_size(size * f64::from(gtk::pango::SCALE));
            if bold {
                font.set_weight(gtk::pango::Weight::Semibold);
            }
            layout.set_font_description(Some(&font));
            layout
        }

        /// One line of text, vertically centred on `at`. `anchor` is where
        /// `at` sits along it: `End` its trailing edge, `Middle` its middle,
        /// `Start` its leading edge.
        #[allow(clippy::too_many_arguments)]
        fn text(
            &self,
            snapshot: &gtk::Snapshot,
            text: &str,
            at: Point,
            size: f64,
            bold: bool,
            anchor: Anchor,
            colour: &gtk::gdk::RGBA,
        ) {
            let layout = self.layout(text, size, bold);
            let (w, h) = layout.pixel_size();
            let x = match anchor {
                Anchor::End => at.x - f64::from(w),
                Anchor::Middle => at.x - f64::from(w) / 2.0,
                Anchor::Start => at.x,
            };
            snapshot.save();
            snapshot.translate(&gtk::graphene::Point::new(
                x as f32,
                (at.y - f64::from(h) / 2.0) as f32,
            ));
            snapshot.append_layout(&layout, colour);
            snapshot.restore();
        }

        fn draw_words(
            &self,
            snapshot: &gtk::Snapshot,
            scene: &Scene,
            fit: &MapGeometry,
            palette: &Palette,
        ) {
            let words = map_scene::words();
            let (label_size, sublabel_size, label_offset, sublabel_offset) =
                map_scene::label_type(fit.scale);
            let traces = scene.stack.is_some();
            for node in &scene.nodes {
                // On the Traces map the coins carry their own names, and
                // the library none: its card says it.
                if traces && matches!(node.kind, NodeKind::Tool { .. } | NodeKind::Library { .. }) {
                    continue;
                }
                let centre = fit.point(node.at);
                let radius = node.radius * fit.scale;
                let ink = palette.ink(node.dim);
                if node.label_beside() {
                    // A folder's name sits left of its dot, on its line, so
                    // a long list stays one line a folder.
                    let text = match node.kind {
                        NodeKind::Folder(_) => map_scene::short_label(&node.label),
                        _ => node.label.clone(),
                    };
                    let gap = 6.0 * fit.scale.max(map_scene::LABEL_FLOOR);
                    let at = Point::new(centre.x - radius - gap, centre.y);
                    self.text(snapshot, &text, at, label_size, true, Anchor::End, &ink);
                } else {
                    let at = Point::new(centre.x, centre.y + radius + label_offset);
                    self.text(
                        snapshot,
                        &node.label,
                        at,
                        label_size,
                        true,
                        Anchor::Middle,
                        &ink,
                    );
                }
                if let Some(sublabel) = node.sublabel.as_deref() {
                    let at = Point::new(centre.x, centre.y + radius + sublabel_offset);
                    let colour = palette.ink(0.6 * node.dim);
                    self.text(
                        snapshot,
                        sublabel,
                        at,
                        sublabel_size,
                        false,
                        Anchor::Middle,
                        &colour,
                    );
                }
            }
            for overflow in &scene.overflows {
                let text = format!("+{}", overflow.count);
                let colour = palette.ink(0.6);
                self.text(
                    snapshot,
                    &text,
                    fit.point(overflow.at),
                    10.0,
                    false,
                    Anchor::Middle,
                    &colour,
                );
            }
            let Some(stack) = scene.stack.as_ref() else {
                return;
            };
            let s = fit.scale;
            let open = self.pile_open.get();
            let left = fit.point(stack.port_left());
            let height = stack.height(open) * s;
            self.text(
                snapshot,
                &words.tools_heading.to_uppercase(),
                Point::new(left.x + 6.0 * s, left.y - height / 2.0 - 12.0 * s),
                (10.0 * s).max(9.0),
                true,
                Anchor::Start,
                &palette.ink(0.6),
            );
            for (index, id) in stack.tools.iter().enumerate() {
                let Some(node) = scene.nodes.iter().find(|n| n.id == *id) else {
                    continue;
                };
                let centre = fit.point(stack.coin_center(index, open));
                let shown = if node.dashed { 0.55 } else { node.dim };
                if let NodeKind::Tool { kind, .. } = node.kind {
                    let initials = map_scene::initials(kind.display_name());
                    let colour = palette.ink(shown);
                    self.text(
                        snapshot,
                        &initials,
                        centre,
                        12.0 * s,
                        true,
                        Anchor::Middle,
                        &colour,
                    );
                }
                if !open {
                    continue;
                }
                // A coin's name and what waits from it, beside it once the
                // card is open.
                let line = if node.dashed {
                    Some(words.legend_not_watched.to_string())
                } else {
                    node.waiting.map(|n| map_scene::coin_waiting(&words, n))
                };
                let x = centre.x + 32.0 * s;
                let name_y = if line.is_some() {
                    centre.y - 6.0 * s
                } else {
                    centre.y
                };
                let colour = palette.ink(shown);
                let size = (11.0 * s).max(9.5);
                self.text(
                    snapshot,
                    &node.label,
                    Point::new(x, name_y),
                    size,
                    true,
                    Anchor::Start,
                    &colour,
                );
                if let Some(line) = line {
                    let colour = palette.ink(0.6 * shown);
                    let at = Point::new(x, centre.y + 7.0 * s);
                    self.text(
                        snapshot,
                        &line,
                        at,
                        (10.0 * s).max(9.0),
                        false,
                        Anchor::Start,
                        &colour,
                    );
                }
            }
        }
    }

    #[derive(Clone, Copy)]
    pub enum Anchor {
        Start,
        Middle,
        End,
    }
}

glib::wrapper! {
    pub struct MapCanvas(ObjectSubclass<imp::MapCanvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for MapCanvas {
    fn default() -> Self {
        Self::new()
    }
}

impl MapCanvas {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The scene's inputs changed: lay it out again, and keep a button for
    /// each node it now draws.
    pub fn set_inputs(&self, inputs: SceneInputs) {
        let imp = self.imp();
        if *imp.inputs.borrow() == inputs {
            return;
        }
        imp.inputs.replace(inputs);
        self.relayout();
    }

    fn relayout(&self) {
        self.imp().layout_scene();
        self.sync_children();
        self.queue_allocate();
        self.queue_draw();
    }

    pub fn scene(&self) -> Scene {
        self.imp().scene.borrow().clone()
    }

    pub fn zoom(&self) -> f64 {
        self.imp().zoom.get()
    }

    pub fn set_zoom(&self, zoom: f64) {
        self.imp()
            .zoom
            .set(zoom.clamp(map_scene::ZOOM_MIN, map_scene::ZOOM_MAX));
        self.queue_allocate();
        self.queue_draw();
    }

    pub fn connect_select_folder(&self, callback: impl Fn(String) + 'static) {
        self.imp()
            .on_select_folder
            .replace(Some(Box::new(callback)));
    }

    /// Each node's button, by node id, in keyboard order.
    pub fn node_buttons(&self) -> Vec<(String, gtk::Button)> {
        self.imp().nodes.borrow().clone()
    }

    /// The hovered or focused node: the one whose card shows.
    pub fn card_node(&self) -> Option<String> {
        self.imp().selection()
    }

    /// The "+N" pill, while the scene has one.
    pub fn pill(&self) -> Option<gtk::Button> {
        self.imp()
            .pill
            .borrow()
            .clone()
            .filter(|pill| pill.is_visible())
    }

    /// A click on the pill: the next few folders, or all of them folded
    /// back.
    fn reveal(&self) {
        let imp = self.imp();
        let Some(reveal) = imp.scene.borrow().reveals.first().cloned() else {
            return;
        };
        let revealed = Scene::revealing(&reveal, &imp.revealed.borrow());
        imp.revealed.replace(revealed);
        self.relayout();
    }

    /// One button per node, each named by the node's label and described
    /// by its card. Buttons for nodes the scene still draws are kept, so
    /// keyboard focus stays where it was.
    fn sync_children(&self) {
        let imp = self.imp();
        let scene = imp.scene.borrow().clone();
        let (stay, gone): (Vec<_>, Vec<_>) = imp
            .nodes
            .take()
            .into_iter()
            .partition(|(id, _)| scene.nodes.iter().any(|n| n.id == *id));
        for (_, button) in gone {
            button.unparent();
        }
        let mut stay: BTreeMap<String, gtk::Button> = stay.into_iter().collect();
        // Keyboard order follows the drawing, left to right: the folders,
        // then the pile, then this computer and the library.
        let mut order: Vec<&map_scene::Node> = scene.nodes.iter().collect();
        order.sort_by_key(|node| match node.kind {
            NodeKind::Folder(_) => 0,
            NodeKind::Tool { .. } => 1,
            _ => 2,
        });
        let pill = imp.pill.borrow().clone();
        let mut kept = Vec::new();
        for node in order {
            let button = stay
                .remove(&node.id)
                .unwrap_or_else(|| self.node_button(&node.id));
            button.update_property(&[
                gtk::accessible::Property::Label(&node.label),
                gtk::accessible::Property::Description(&node.detail),
            ]);
            button.set_tooltip_text(Some(&node.label));
            // Siblings paint in order: every node under the pill and the
            // card.
            if let Some(pill) = pill.as_ref() {
                button.insert_before(self, Some(pill));
            }
            kept.push((node.id.clone(), button));
        }
        imp.nodes.replace(kept);
        for id in [&imp.hovered, &imp.focused] {
            let mut id = id.borrow_mut();
            if id
                .as_ref()
                .is_some_and(|id| !scene.nodes.iter().any(|n| n.id == *id))
            {
                *id = None;
            }
        }

        if let Some(pill) = pill.as_ref() {
            let words = map_scene::words();
            match scene.reveals.first() {
                Some(reveal) => {
                    let folds = reveal.hidden == 0;
                    let text = if folds {
                        words.fold_folders.to_string()
                    } else {
                        format!("+{}", reveal.hidden)
                    };
                    let name = if folds {
                        words.fold_folders_label.to_string()
                    } else {
                        map_scene::reveal_folders(&words, reveal.next())
                    };
                    pill.set_label(&text);
                    pill.set_tooltip_text(Some(&name));
                    pill.update_property(&[gtk::accessible::Property::Label(&name)]);
                    pill.set_opacity(reveal.dim);
                    pill.set_visible(true);
                }
                None => pill.set_visible(false),
            }
        }
        self.refresh_card();
    }

    fn node_button(&self, id: &str) -> gtk::Button {
        let button = gtk::Button::new();
        button.add_css_class("tc-map-node");
        button.set_parent(self);

        let weak = self.downgrade();
        let clicked = id.to_string();
        button.connect_clicked(move |_| {
            let Some(canvas) = weak.upgrade() else { return };
            if let Some(project) = map_scene::folder_id(&clicked)
                && let Some(callback) = canvas.imp().on_select_folder.borrow().as_ref()
            {
                callback(project.to_string());
            }
        });

        let hover = gtk::EventControllerMotion::new();
        let weak = self.downgrade();
        let entered = id.to_string();
        hover.connect_enter(move |_, _, _| {
            if let Some(canvas) = weak.upgrade() {
                canvas.imp().hovered.replace(Some(entered.clone()));
                canvas.refresh_card();
            }
        });
        let weak = self.downgrade();
        let left = id.to_string();
        hover.connect_leave(move |_| {
            let Some(canvas) = weak.upgrade() else { return };
            let was = canvas.imp().hovered.borrow().clone();
            if was.as_deref() == Some(left.as_str()) {
                canvas.imp().hovered.replace(None);
                canvas.refresh_card();
            }
        });
        button.add_controller(hover);

        // Keyboard focus selects the node as a hover does, so a keyboard
        // user sees the same ring and card a pointer gets.
        let focus = gtk::EventControllerFocus::new();
        let weak = self.downgrade();
        let entered = id.to_string();
        focus.connect_enter(move |_| {
            if let Some(canvas) = weak.upgrade() {
                canvas.imp().focused.replace(Some(entered.clone()));
                canvas.refresh_card();
            }
        });
        let weak = self.downgrade();
        let left = id.to_string();
        focus.connect_leave(move |_| {
            let Some(canvas) = weak.upgrade() else { return };
            let was = canvas.imp().focused.borrow().clone();
            if was.as_deref() == Some(left.as_str()) {
                canvas.imp().focused.replace(None);
                canvas.refresh_card();
            }
        });
        button.add_controller(focus);
        button
    }

    /// The card for the hovered or focused node, or none.
    fn refresh_card(&self) {
        let imp = self.imp();
        let node = {
            let scene = imp.scene.borrow();
            imp.selection()
                .and_then(|id| scene.nodes.iter().find(|n| n.id == id).cloned())
        };
        if let Some((card, title, detail)) = imp.card.borrow().as_ref() {
            match node {
                Some(node) => {
                    title.set_label(node.card_title());
                    detail.set_label(&node.detail);
                    card.set_visible(true);
                }
                None => card.set_visible(false),
            }
        }
        self.queue_allocate();
        self.queue_draw();
    }
}

/// The index of a tool's coin in the pile, for a tool node's id.
fn coin_index(scene: &Scene, id: &str) -> Option<usize> {
    scene
        .stack
        .as_ref()?
        .tools
        .iter()
        .position(|tool| tool == id)
}

// --- Drawing -----------------------------------------------------------------

/// The colours the map draws with: the theme's ink and background, and the
/// shell's accent and status tokens for the current scheme.
struct Palette {
    ink: gtk::gdk::RGBA,
    background: gtk::gdk::RGBA,
    accent: gtk::gdk::RGBA,
    on: gtk::gdk::RGBA,
    ask: gtk::gdk::RGBA,
    outside: gtk::gdk::RGBA,
}

impl Palette {
    #[allow(deprecated)]
    fn current(widget: &impl IsA<gtk::Widget>) -> Self {
        use crate::ui::style::brand_tokens::{dark, light};
        let parse = |hex: &str| gtk::gdk::RGBA::parse(hex).unwrap_or(gtk::gdk::RGBA::BLACK);
        let dark_scheme = adw::StyleManager::default().is_dark();
        let (accent, on, ask, outside) = if dark_scheme {
            (
                dark::ACCENT,
                dark::STATUS_ON,
                dark::STATUS_ASK,
                dark::STATUS_OUTSIDE,
            )
        } else {
            (
                light::ACCENT,
                light::STATUS_ON,
                light::STATUS_ASK,
                light::STATUS_OUTSIDE,
            )
        };
        let context = widget.as_ref().style_context();
        let fallback = if dark_scheme {
            gtk::gdk::RGBA::BLACK
        } else {
            gtk::gdk::RGBA::WHITE
        };
        Self {
            ink: context.color(),
            background: context.lookup_color("window_bg_color").unwrap_or(fallback),
            accent: parse(accent),
            on: parse(on),
            ask: parse(ask),
            outside: parse(outside),
        }
    }

    fn ink(&self, alpha: f64) -> gtk::gdk::RGBA {
        self.ink.with_alpha(self.ink.alpha() * alpha as f32)
    }

    /// A folder's dot by rule: its fill and its ring.
    fn rule(&self, mode: Option<ProjectMode>) -> (gtk::gdk::RGBA, gtk::gdk::RGBA) {
        let stroke = match mode {
            Some(ProjectMode::AutoUpload) => self.on,
            Some(ProjectMode::Ask) => self.ask,
            Some(ProjectMode::Ignore) => self.outside,
            None => self.ink(0.45),
        };
        (stroke.with_alpha(stroke.alpha() * 0.25), stroke)
    }
}

fn source(cr: &gtk::cairo::Context, colour: &gtk::gdk::RGBA, alpha: f64) {
    cr.set_source_rgba(
        f64::from(colour.red()),
        f64::from(colour.green()),
        f64::from(colour.blue()),
        f64::from(colour.alpha()) * alpha,
    );
}

fn circle(cr: &gtk::cairo::Context, centre: Point, radius: f64) {
    cr.new_sub_path();
    cr.arc(
        centre.x,
        centre.y,
        radius.max(0.0),
        0.0,
        std::f64::consts::TAU,
    );
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

/// The arcs, under everything else. `healthy`: only a current screen draws
/// a flowing arc lit; otherwise it is still and dashed, never live.
fn draw_arcs(
    cr: &gtk::cairo::Context,
    scene: &Scene,
    fit: &MapGeometry,
    palette: &Palette,
    healthy: bool,
) {
    let s = fit.scale;
    for arc in &scene.arcs {
        let a = fit.point(arc.from);
        let c1 = fit.point(arc.control1);
        let c2 = fit.point(arc.control2);
        let b = fit.point(arc.to);
        cr.new_path();
        cr.move_to(a.x, a.y);
        cr.curve_to(c1.x, c1.y, c2.x, c2.y, b.x, b.y);
        cr.set_line_width(1.4 * s);
        cr.set_line_cap(gtk::cairo::LineCap::Butt);
        cr.set_dash(&[], 0.0);
        match arc.style {
            ArcStyle::Quiet => source(cr, &palette.ink, 0.2 * arc.dim),
            ArcStyle::Dashed => {
                cr.set_dash(&[5.0 * s, 5.0 * s], 0.0);
                source(cr, &palette.ink, 0.18 * arc.dim);
            }
            ArcStyle::Shared => source(cr, &palette.on, 0.35 * arc.dim),
            ArcStyle::Unknown => {
                cr.set_line_cap(gtk::cairo::LineCap::Round);
                cr.set_dash(&[0.5 * s, 4.0 * s], 0.0);
                source(cr, &palette.ink, 0.18 * arc.dim);
            }
            ArcStyle::Flowing if !healthy => {
                // Paused, unknown or stale: still and dashed, not live.
                cr.set_dash(&[5.0 * s, 5.0 * s], 0.0);
                source(cr, &palette.ink, 0.25 * arc.dim);
            }
            ArcStyle::Flowing => {
                source(cr, &palette.on, 0.35 * arc.dim);
                let _ = cr.stroke_preserve();
                // The dashes, still: they move in the transfers slice.
                cr.set_line_width(1.4 * s * 1.3);
                cr.set_line_cap(gtk::cairo::LineCap::Round);
                cr.set_dash(&[4.0 * s, 8.0 * s], 0.0);
                source(cr, &palette.on, arc.dim);
            }
        }
        let _ = cr.stroke();
    }
    cr.set_dash(&[], 0.0);
}

fn ring(
    cr: &gtk::cairo::Context,
    centre: Point,
    radius: f64,
    scale: f64,
    colour: &gtk::gdk::RGBA,
    alpha: f64,
) {
    let (offset, width) = map_scene::ring_metrics(scale);
    circle(cr, centre, radius + offset);
    cr.set_line_width(width);
    source(cr, colour, alpha);
    let _ = cr.stroke();
}

fn cross(cr: &gtk::cairo::Context, centre: Point, half: f64, s: f64, palette: &Palette, dim: f64) {
    cr.move_to(centre.x - half, centre.y - half);
    cr.line_to(centre.x + half, centre.y + half);
    cr.set_line_width(2.0 * s);
    source(cr, &palette.ink, dim);
    let _ = cr.stroke();
}

fn draw_node(
    cr: &gtk::cairo::Context,
    scene: &Scene,
    node: &map_scene::Node,
    fit: &MapGeometry,
    palette: &Palette,
    selected: bool,
) {
    let traces = scene.stack.is_some();
    // On the Traces map the tools are the pile's coins (`draw_pile`).
    if traces && matches!(node.kind, NodeKind::Tool { .. }) {
        return;
    }
    let s = fit.scale;
    let centre = fit.point(node.at);
    let radius = node.radius * s;
    let dim = node.dim;
    match node.kind {
        NodeKind::Hub if traces => draw_pad(cr, centre, s, palette, dim),
        NodeKind::Hub => {
            circle(cr, centre, radius);
            source(cr, &palette.ink, 0.45 * dim);
            let _ = cr.fill();
            ring(cr, centre, radius, s, &palette.ink, 0.25 * dim);
        }
        NodeKind::Library { active } => {
            circle(cr, centre, radius);
            if active {
                source(cr, &palette.accent, dim);
            } else {
                source(cr, &palette.ink, 0.2 * dim);
            }
            let _ = cr.fill();
            if active {
                ring(cr, centre, radius, s, &palette.accent, 0.35 * dim);
            }
        }
        NodeKind::Tool { off, .. } => {
            circle(cr, centre, radius);
            source(cr, &palette.ink, 0.12 * dim);
            let _ = cr.fill();
            if node.dashed {
                circle(cr, centre, radius);
                cr.set_line_width(1.5 * s);
                cr.set_dash(&[2.0 * s, 2.0 * s], 0.0);
                source(cr, &palette.ink, 0.45 * dim);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
            }
            if node.ringed {
                ring(cr, centre, radius, s, &palette.ink, 0.35 * dim);
            }
            if off {
                cross(cr, centre, 9.0 * s, s, palette, dim);
            }
        }
        NodeKind::Folder(mode) => {
            let (fill, stroke) = palette.rule(mode);
            circle(cr, centre, radius);
            source(cr, &fill, dim);
            let _ = cr.fill_preserve();
            cr.set_line_width(2.0 * s);
            source(cr, &stroke, dim);
            let _ = cr.stroke();
        }
        NodeKind::Harness { .. } => {
            circle(cr, centre, radius);
            source(cr, &palette.ink, 0.12 * dim);
            let _ = cr.fill();
        }
        NodeKind::Device => {
            let deck = Point::new(centre.x, centre.y + 4.0 * s);
            draw_laptop(cr, deck, s * 0.45, palette, dim);
        }
        NodeKind::Destination(lamp) => draw_destination(cr, centre, radius, s, lamp, palette, dim),
    }
    let drawn_elsewhere = traces && matches!(node.kind, NodeKind::Hub | NodeKind::Library { .. });
    if selected && !drawn_elsewhere {
        circle(cr, centre, radius);
        cr.set_line_width(2.0 * s);
        source(cr, &palette.accent, 1.0);
        let _ = cr.stroke();
    }
}

/// The Private AI destination: lit while running, ringed while answering,
/// a dotted rim while unknown (never drawn as off), and the credential's
/// key on it.
fn draw_destination(
    cr: &gtk::cairo::Context,
    centre: Point,
    radius: f64,
    s: f64,
    lamp: Lamp,
    palette: &Palette,
    dim: f64,
) {
    circle(cr, centre, radius);
    if matches!(lamp, Lamp::Answering | Lamp::Running) {
        source(cr, &palette.on, dim);
    } else {
        source(cr, &palette.ink, 0.2 * dim);
    }
    let _ = cr.fill();
    if lamp == Lamp::Answering {
        ring(cr, centre, radius, s, &palette.on, 0.35 * dim);
    }
    if lamp == Lamp::Unknown {
        circle(cr, centre, radius);
        cr.set_line_width(1.5 * s);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.set_dash(&[0.5 * s, 4.0 * s], 0.0);
        source(cr, &palette.ink, 0.5 * dim);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
    }
    let ((bow, bow_radius), segments) = map_scene::key_glyph(centre, s);
    circle(cr, bow, bow_radius);
    for (a, b) in segments {
        cr.move_to(a.x, a.y);
        cr.line_to(b.x, b.y);
    }
    cr.set_line_width(2.0 * s);
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    source(cr, &palette.ink, dim);
    let _ = cr.stroke();
}

/// This computer as a launch pad: a slab on a base, with a service tower
/// beside it; the line out lands on its left edge.
fn draw_pad(cr: &gtk::cairo::Context, centre: Point, s: f64, palette: &Palette, dim: f64) {
    let half = map_scene::PAD_HALF_WIDTH * s;
    rounded(
        cr,
        centre.x - half,
        centre.y - 5.0 * s,
        half * 2.0,
        10.0 * s,
        4.0 * s,
    );
    source(cr, &palette.ink, 0.45 * dim);
    let _ = cr.fill();
    rounded(
        cr,
        centre.x - 24.0 * s,
        centre.y + 5.0 * s,
        48.0 * s,
        6.0 * s,
        3.0 * s,
    );
    source(cr, &palette.ink, 0.2 * dim);
    let _ = cr.fill();
    for (x, y, w, h) in [
        (28.0, -55.0, 4.0, 50.0),
        (14.0, -43.0, 16.0, 3.0),
        (18.0, -27.0, 12.0, 3.0),
    ] {
        rounded(
            cr,
            centre.x + x * s,
            centre.y + y * s,
            w * s,
            h * s,
            1.5 * s,
        );
        source(cr, &palette.ink, 0.32 * dim);
        let _ = cr.fill();
    }
}

/// A laptop: its deck as wide as the pad, its screen open above it.
fn draw_laptop(cr: &gtk::cairo::Context, centre: Point, s: f64, palette: &Palette, dim: f64) {
    let half = map_scene::PAD_HALF_WIDTH * s;
    rounded(
        cr,
        centre.x - half,
        centre.y - 3.0 * s,
        half * 2.0,
        6.0 * s,
        3.0 * s,
    );
    source(cr, &palette.ink, 0.45 * dim);
    let _ = cr.fill();
    let top = centre.y - 36.0 * s;
    rounded(cr, centre.x - 25.0 * s, top, 50.0 * s, 32.0 * s, 3.5 * s);
    source(cr, &palette.ink, 0.38 * dim);
    let _ = cr.fill();
    rounded(
        cr,
        centre.x - 22.5 * s,
        top + 2.5 * s,
        45.0 * s,
        27.0 * s,
        2.0 * s,
    );
    source(cr, &palette.accent, 0.9 * dim);
    let _ = cr.fill();
}

/// The tool group: a card, and the coins on it, the front coin first.
fn draw_pile(
    cr: &gtk::cairo::Context,
    scene: &Scene,
    stack: &ToolStack,
    fit: &MapGeometry,
    palette: &Palette,
    open: bool,
) {
    let s = fit.scale;
    let (x, y, w, h) = stack.card(fit, open, open);
    rounded(cr, x, y, w, h, 26.0 * s);
    if open {
        // An open card is opaque: nothing under it shows through.
        source(cr, &palette.background, 1.0);
    } else {
        source(cr, &palette.ink, 0.06);
    }
    let _ = cr.fill();
    // The back of the pile first, so the front coin is drawn over the rest.
    for (index, id) in stack.tools.iter().enumerate().rev() {
        let Some(node) = scene.nodes.iter().find(|n| n.id == *id) else {
            continue;
        };
        let NodeKind::Tool { off, .. } = node.kind else {
            continue;
        };
        let centre = fit.point(stack.coin_center(index, open));
        let radius = ToolStack::COIN / 2.0 * s;
        let alpha = if node.dashed { 0.55 } else { node.dim };
        // The coin's edge below it, then an opaque face and its rim.
        circle(cr, Point::new(centre.x, centre.y + 3.0 * s), radius);
        source(cr, &palette.ink, 0.25 * alpha);
        let _ = cr.fill();
        circle(cr, centre, radius);
        source(cr, &palette.background, 1.0);
        let _ = cr.fill_preserve();
        source(cr, &palette.ink, 0.12 * alpha);
        let _ = cr.fill();
        circle(cr, centre, radius - s);
        cr.set_line_width(2.0 * s);
        if node.dashed {
            cr.set_dash(&[3.0 * s, 3.0 * s], 0.0);
        }
        source(cr, &palette.ink, 0.6 * alpha);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        if node.ringed {
            ring(cr, centre, radius, s, &palette.ink, 0.35 * alpha);
        }
        if off {
            cross(cr, centre, radius * 0.5, s, palette, alpha);
        }
        if coin_has_lock(node.dashed, open, index) {
            // The tool's memory stays on this computer: a pip at the coin's
            // right edge, on the pane's ground.
            let pip = Point::new(centre.x + radius + s, centre.y);
            circle(cr, pip, 7.5 * s);
            source(cr, &palette.background, 1.0);
            let _ = cr.fill();
            draw_lock(cr, pip, s, &palette.ink, 0.7);
        }
    }
}

/// Whether a coin carries the lock, as `MapToolPile` draws it: every
/// watched coin of the open pile, and only the front one of the closed
/// pile, so the pips do not stack.
fn coin_has_lock(dashed: bool, open: bool, index: usize) -> bool {
    !dashed && (open || index == 0)
}

/// A small closed padlock centred on `centre`, `s` the map's scale: a body
/// and its shackle.
fn draw_lock(cr: &gtk::cairo::Context, centre: Point, s: f64, ink: &gtk::gdk::RGBA, alpha: f64) {
    let (w, h) = (6.0 * s, 4.5 * s);
    let top = centre.y - h / 2.0 + 1.2 * s;
    source(cr, ink, alpha);
    cr.rectangle(centre.x - w / 2.0, top, w, h);
    let _ = cr.fill();
    cr.new_sub_path();
    cr.arc(
        centre.x,
        top,
        1.9 * s,
        std::f64::consts::PI,
        std::f64::consts::TAU,
    );
    cr.set_line_width(1.2 * s);
    let _ = cr.stroke();
}

/// The legend's word for the lock: memory stays on this computer. The
/// Mac's says "this Mac".
fn memory_legend_word() -> &'static str {
    monitor_screens_copy().flow_map.legend_memory_computer
}

/// The legend's memory item: the coins' lock, and its word.
fn memory_legend_item() -> gtk::Box {
    let item = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let lock = gtk::DrawingArea::new();
    lock.set_content_width(10);
    lock.set_content_height(10);
    lock.set_valign(gtk::Align::Center);
    lock.set_draw_func(|area, cr, w, h| {
        let palette = Palette::current(area);
        let centre = Point::new(f64::from(w) / 2.0, f64::from(h) / 2.0);
        draw_lock(cr, centre, 1.0, &palette.ink, 0.6);
    });
    let label = gtk::Label::new(Some(memory_legend_word()));
    label.add_css_class("caption");
    label.add_css_class("dim-label");
    item.append(&lock);
    item.append(&label);
    item
}

// --- The pane ----------------------------------------------------------------

/// What the map reads besides the queue the window already holds. Each is
/// `None` until its read answers, and again after a failed read: an unread
/// signal never draws a flow.
#[derive(Default)]
struct Reads {
    settings: Option<Settings>,
    status: Option<GateStatus>,
    destinations: Option<ToolDestinations>,
    /// `None` before `list_pending` first answers; `Some(false)` after a
    /// failed read.
    pending: Option<bool>,
    /// The same for `list_projects`: the Mac's Traces store fails its whole
    /// load when either read fails, since a folder's rule comes from it.
    projects: Option<bool>,
}

pub struct TraceMap {
    pub root: gtk::Box,
    pub canvas: MapCanvas,
    /// The Traces / Private AI switch.
    pub traces_view: gtk::ToggleButton,
    pub private_view: gtk::ToggleButton,
    pub zoom_out: gtk::Button,
    pub zoom_in: gtk::Button,
    stage: gtk::Stack,
    foot: gtk::Box,
    spinner: gtk::Spinner,
    reads: RefCell<Reads>,
}

impl Default for TraceMap {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceMap {
    pub fn new() -> Self {
        let words = monitor_screens_copy();
        let flow = &words.flow_map;
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let overlay = gtk::Overlay::new();
        overlay.set_vexpand(true);
        overlay.set_hexpand(true);

        let canvas = MapCanvas::new();
        canvas.update_property(&[gtk::accessible::Property::Label(flow.map_label)]);
        let stage = gtk::Stack::new();
        stage.set_vexpand(true);
        stage.add_named(&canvas, Some("traces"));
        // The Private AI view is a later slice: an empty field, as the Mac
        // draws one with no scene.
        let empty = gtk::Box::new(gtk::Orientation::Vertical, 0);
        stage.add_named(&empty, Some("private-ai"));
        overlay.set_child(Some(&stage));

        // Before the first reply the map is empty only because it is
        // unread: a spinner says so.
        let spinner = gtk::Spinner::new();
        spinner.set_halign(gtk::Align::Center);
        spinner.set_valign(gtk::Align::Center);
        spinner.set_can_target(false);
        overlay.add_overlay(&spinner);

        // The view switch, floating at the upper trailing edge. It changes
        // the map's view only, never the tab, consent or routing.
        let switch = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        switch.add_css_class("linked");
        switch.set_halign(gtk::Align::End);
        switch.set_valign(gtk::Align::Start);
        switch.set_margin_top(14);
        switch.set_margin_end(14);
        switch.set_accessible_role(gtk::AccessibleRole::Group);
        switch.update_property(&[gtk::accessible::Property::Label(
            words.shell.map_views_label,
        )]);
        let traces_view = gtk::ToggleButton::with_label(words.shell.tab_traces);
        let private_view = gtk::ToggleButton::with_label(
            trace_commons_contributor::private_inference_copy::private_inference_copy().destination,
        );
        private_view.set_group(Some(&traces_view));
        traces_view.set_active(true);
        switch.append(&traces_view);
        switch.append(&private_view);
        overlay.add_overlay(&switch);

        // The map's foot, fixed whatever its zoom or focus: the legend,
        // then the zoom, on one row; the legend wraps before it would run
        // under the zoom.
        let foot = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        foot.set_valign(gtk::Align::End);
        foot.set_margin_start(14);
        foot.set_margin_end(14);
        foot.set_margin_bottom(14);
        // The legend's items flow onto further lines as the pane narrows,
        // as the Mac's `HistoryFilterFlow` does (`style::wrap_row`).
        let legend = crate::ui::style::wrap_row(10);
        legend.set_hexpand(true);
        legend.set_valign(gtk::Align::End);
        legend.set_can_target(false);
        for mode in [
            ProjectMode::AutoUpload,
            ProjectMode::Ask,
            ProjectMode::Ignore,
        ] {
            legend.append(&legend_item(mode.label(), Some(mode)));
        }
        legend.append(&legend_item(flow.legend_not_watched, None));
        let memory = memory_legend_item();
        memory.set_visible(false);
        legend.append(&memory);
        canvas
            .imp()
            .memory_legend
            .replace(Some(memory.clone().upcast::<gtk::Widget>().downgrade()));
        foot.append(&legend);
        let zoom = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        zoom.add_css_class("linked");
        zoom.set_valign(gtk::Align::End);
        zoom.set_accessible_role(gtk::AccessibleRole::Group);
        zoom.update_property(&[gtk::accessible::Property::Label(flow.zoom_label)]);
        let zoom_out = gtk::Button::from_icon_name("zoom-out-symbolic");
        zoom_out.set_tooltip_text(Some(words.reduce));
        zoom_out.update_property(&[gtk::accessible::Property::Label(words.reduce)]);
        let zoom_in = gtk::Button::from_icon_name("zoom-in-symbolic");
        zoom_in.set_tooltip_text(Some(words.enlarge));
        zoom_in.update_property(&[gtk::accessible::Property::Label(words.enlarge)]);
        zoom.append(&zoom_out);
        zoom.append(&zoom_in);
        foot.append(&zoom);
        overlay.add_overlay(&foot);
        // The foot along the pane's bottom, as tall as its legend is at the
        // pane's width: measured for the width it is given, so a legend that
        // wraps onto a second line gets that line's height on every GTK.
        let placed = foot.downgrade();
        overlay.connect_get_child_position(move |overlay, child| {
            let foot = placed.upgrade()?;
            if child != foot.upcast_ref::<gtk::Widget>() {
                return None;
            }
            let (width, height) = (overlay.width(), overlay.height());
            if width <= 0 || height <= 0 {
                return None;
            }
            let (least, _, _, _) = foot.measure(gtk::Orientation::Horizontal, -1);
            let width = width.max(least);
            let (_, natural, _, _) = foot.measure(gtk::Orientation::Vertical, width);
            let natural = natural.min(height);
            Some(gtk::gdk::Rectangle::new(
                0,
                height - natural,
                width,
                natural,
            ))
        });

        root.append(&overlay);
        let map = Self {
            root,
            canvas,
            traces_view,
            private_view,
            zoom_out,
            zoom_in,
            stage,
            foot,
            spinner,
            reads: RefCell::new(Reads::default()),
        };
        map.connect_zoom();
        map
    }

    fn connect_zoom(&self) {
        for (button, enlarge) in [(&self.zoom_out, false), (&self.zoom_in, true)] {
            let canvas = self.canvas.downgrade();
            let out = self.zoom_out.downgrade();
            let into = self.zoom_in.downgrade();
            button.connect_clicked(move |_| {
                let Some(canvas) = canvas.upgrade() else {
                    return;
                };
                canvas.set_zoom(map_scene::zoomed(canvas.zoom(), enlarge));
                if let (Some(out), Some(into)) = (out.upgrade(), into.upgrade()) {
                    out.set_sensitive(canvas.zoom() > map_scene::ZOOM_MIN);
                    into.set_sensitive(canvas.zoom() < map_scene::ZOOM_MAX);
                }
            });
        }
    }

    /// Shows the view the window chose.
    pub fn show_view(&self, view: MapView) {
        let traces = view == MapView::Traces;
        if traces && !self.traces_view.is_active() {
            self.traces_view.set_active(true);
        } else if !traces && !self.private_view.is_active() {
            self.private_view.set_active(true);
        }
        self.stage
            .set_visible_child_name(if traces { "traces" } else { "private-ai" });
        self.foot.set_visible(traces);
    }

    // --- The reads ---------------------------------------------------------

    /// `get_settings`, or `None` after a failed read: every tool is then
    /// unknown, never unset.
    pub fn set_settings(&self, settings: Option<Settings>) {
        self.reads.borrow_mut().settings = settings;
    }

    /// The gate's fields of a `status` reply, or `None` after a failed read.
    pub fn set_status(&self, status: Option<&serde_json::Value>) {
        self.reads.borrow_mut().status =
            status.and_then(|value| serde_json::from_value(value.clone()).ok());
    }

    /// `tool_destinations`, or `None` after a failed or unreadable read.
    pub fn set_destinations(&self, destinations: Option<&serde_json::Value>) {
        self.reads.borrow_mut().destinations =
            destinations.and_then(|value| serde_json::from_value(value.clone()).ok());
    }

    /// Whether the last `list_pending` read answered.
    pub fn set_pending_read(&self, answered: bool) {
        self.reads.borrow_mut().pending = Some(answered);
    }

    /// Whether the last `list_projects` read answered. A failed one leaves
    /// the window's last rows in place, so the map is then not current and
    /// a stale Automatic rule draws no flow.
    pub fn set_projects_read(&self, answered: bool) {
        self.reads.borrow_mut().projects = Some(answered);
    }

    /// The map's screen state, by the Mac's precedence.
    fn state(&self) -> ScreenState {
        let reads = self.reads.borrow();
        ScreenState::resolve(
            reads.pending == Some(false) || reads.projects == Some(false),
            reads.pending.is_some() && reads.projects.is_some(),
            reads.status.as_ref().and_then(|status| status.paused),
            reads.status.is_some(),
        )
    }

    /// The scene's inputs from what the window holds.
    pub fn inputs(
        &self,
        entries: &[QueueEntry],
        projects: &[Project],
        history: Option<&[HistoryRecord]>,
        selected_tool: Option<String>,
        focus_tool: Option<String>,
    ) -> SceneInputs {
        let state = self.state();
        let reads = self.reads.borrow();
        SceneInputs {
            tree: TracesTree::build(entries, projects, reads.settings.as_ref()),
            gate: CommonsGate {
                state,
                status: reads.status.clone(),
                destinations: reads.destinations.clone(),
            },
            contributed: Contributions::from_history(history),
            selected_tool: selected_tool.filter(|tool| SourceKind::from_wire(tool).is_some()),
            focus_tool,
        }
    }

    pub fn render(&self, app: &Rc<App>) {
        let inputs = self.inputs(
            &app.entries.borrow(),
            &app.projects.borrow(),
            app.history_records.borrow().as_deref(),
            app.shell.selected_tool_id(app),
            app.shell.map_focus_tool(app),
        );
        let loading = inputs.gate.state == ScreenState::Loading;
        self.spinner.set_spinning(loading);
        self.spinner.set_visible(loading);
        if self.canvas.imp().on_select_folder.borrow().is_none() {
            let weak = Rc::downgrade(app);
            self.canvas.connect_select_folder(move |project| {
                if let Some(app) = weak.upgrade() {
                    // A folder clicked on the map is the selection and the
                    // inspector's subject, on Traces.
                    app.shell.select_from_map(&app, project);
                }
            });
        }
        self.canvas.set_inputs(inputs);
    }
}

/// One legend item: a rule's dot, or the dashed ring of a tool that is not
/// watched, and its word.
fn legend_item(word: &str, mode: Option<ProjectMode>) -> gtk::Box {
    let item = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let dot = gtk::DrawingArea::new();
    dot.set_content_width(10);
    dot.set_content_height(10);
    dot.set_valign(gtk::Align::Center);
    dot.set_draw_func(move |area, cr, w, h| {
        let palette = Palette::current(area);
        let centre = Point::new(f64::from(w) / 2.0, f64::from(h) / 2.0);
        circle(cr, centre, 4.0);
        cr.set_line_width(1.5);
        match mode {
            Some(_) => {
                let (fill, stroke) = palette.rule(mode);
                source(cr, &fill, 1.0);
                let _ = cr.fill_preserve();
                source(cr, &stroke, 1.0);
            }
            None => {
                cr.set_dash(&[2.0, 2.0], 0.0);
                source(cr, &palette.ink, 0.45);
            }
        }
        let _ = cr.stroke();
    });
    let label = gtk::Label::new(Some(word));
    label.add_css_class("caption");
    label.add_css_class("dim-label");
    item.append(&dot);
    item.append(&label);
    item
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The legend's memory item is the shared copy's word for this
    /// computer, never the Mac's, and only while the tool group is drawn,
    /// as the Mac shows `legend_memory` only with its pile.
    #[test]
    fn the_memory_legend_says_this_computer() {
        let flow = monitor_screens_copy().flow_map;
        assert_eq!(memory_legend_word(), flow.legend_memory_computer);
        assert_ne!(memory_legend_word(), flow.legend_memory);
        let body = include_str!("map.rs")
            .split("\n#[cfg(test)]")
            .next()
            .unwrap();
        assert!(!body.contains("flow.legend_memory,"));
        assert!(!body.contains("words.legend_memory)"));
    }

    /// The lock on a coin, as `MapToolPile` draws it: on every watched coin
    /// of the open pile, and on the front coin alone of the closed one.
    #[test]
    fn the_lock_is_on_watched_coins_the_front_one_while_closed() {
        assert!(coin_has_lock(false, false, 0));
        assert!(!coin_has_lock(false, false, 1));
        assert!(coin_has_lock(false, true, 3));
        assert!(!coin_has_lock(true, true, 0));
        assert!(!coin_has_lock(true, false, 0));
    }

    fn entry(id: &str, project: &str, source: &str) -> QueueEntry {
        QueueEntry {
            entry_id: id.into(),
            project_id: project.into(),
            project_label: project.into(),
            source: source.into(),
            state: "pending".into(),
            ..Default::default()
        }
    }

    fn settle() {
        let context = glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }
    }

    /// Iterates the main loop until `done` holds, or half a second passes:
    /// allocation and a keyboard activation's `clicked` both land a few
    /// iterations after the call that causes them.
    fn settle_until(done: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while !done() && std::time::Instant::now() < deadline {
            settle();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        settle();
    }

    /// Whether anything under `widget` waits on a transition.
    fn moving(widget: &gtk::Widget) -> bool {
        let mut child = widget.first_child();
        while let Some(next) = child {
            let stack_slides = next.downcast_ref::<gtk::Stack>().is_some_and(|stack| {
                stack.transition_type() != gtk::StackTransitionType::None
                    && stack.transition_duration() > 0
            });
            if next.is::<gtk::Revealer>() || stack_slides || moving(&next) {
                return true;
            }
            child = next.next_sibling();
        }
        false
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    // Reduced motion: the map's slide-down revealer went with its Show/Hide
    // button, and this pane has nothing that moves until the transfers
    // slice. Under reduced motion it is drawn whole at once, and nothing in
    // it waits on a transition. The graph footer's half is
    // `graph_footer::tests::mounted_footer_opens_at_once_under_reduced_motion`.
    fn mounted_map_preserves_identity_and_accessibility() {
        gtk::init().expect("GTK display");
        let settings = gtk::Settings::default().expect("settings");
        let animations = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(false);
        let map = TraceMap::new();
        map.set_pending_read(true);
        map.set_projects_read(true);
        let entries: Vec<_> = (0..40)
            .map(|i| entry(&format!("e{i}"), &format!("p{i}"), "codex"))
            .collect();
        let selected = Rc::new(RefCell::new(String::new()));
        let result = selected.clone();
        map.canvas
            .connect_select_folder(move |id| *result.borrow_mut() = id);
        map.canvas
            .set_inputs(map.inputs(&entries, &[], None, None, None));
        let window = gtk::Window::builder()
            .default_width(480)
            .default_height(700)
            .child(&map.root)
            .build();
        window.present();
        let canvas = map.canvas.clone();
        settle_until(move || canvas.width() > 0);
        assert!(!moving(map.root.upcast_ref()), "something in the map moves");

        // Ten folders, then the pill; every tool a coin; this computer and
        // the library.
        let buttons = map.canvas.node_buttons();
        let folders: Vec<_> = buttons
            .iter()
            .filter(|(id, _)| id.starts_with("folder:"))
            .cloned()
            .collect();
        assert_eq!(folders.len(), map_scene::FOLDERS_SHOWN);
        assert_eq!(
            buttons
                .iter()
                .filter(|(id, _)| id.starts_with("tool:"))
                .count(),
            SourceKind::ALL.len()
        );
        assert!(buttons.iter().any(|(id, _)| id == "hub"));
        assert!(buttons.iter().any(|(id, _)| id == "library"));
        for (id, button) in &buttons {
            assert!(button.is_focusable(), "{id}");
            assert_eq!(
                button.accessible_role(),
                gtk::AccessibleRole::Button,
                "{id}"
            );
            assert!(button.width() > 0 && button.height() > 0, "{id}");
        }

        // Keyboard focus shows the node's card, as a hover does; a click
        // selects the folder by its id.
        let (id, node) = folders[3].clone();
        assert!(node.grab_focus());
        settle();
        assert_eq!(map.canvas.card_node().as_deref(), Some(id.as_str()));
        assert!(node.activate());
        let clicked = selected.clone();
        settle_until(move || !clicked.borrow().is_empty());
        assert_eq!(Some(selected.borrow().as_str()), map_scene::folder_id(&id));

        // The pill draws six more, and the focused folder keeps its button.
        let pill = map.canvas.pill().expect("a pill past ten folders");
        pill.emit_clicked();
        settle();
        let after = map.canvas.node_buttons();
        assert_eq!(
            after
                .iter()
                .filter(|(id, _)| id.starts_with("folder:"))
                .count(),
            map_scene::FOLDERS_SHOWN + map_scene::FOLDERS_REVEALED
        );
        assert!(after.iter().any(|(i, b)| *i == id && *b == node));

        // Zoom steps and stops at its limits.
        for _ in 0..10 {
            map.zoom_in.emit_clicked();
        }
        assert_eq!(map.canvas.zoom(), map_scene::ZOOM_MAX);
        assert!(!map.zoom_in.is_sensitive());
        assert!(map.zoom_out.is_sensitive());

        // The legend names the coins' lock in this computer's words, as
        // the Mac's does with its pile drawn.
        let flow = monitor_screens_copy().flow_map;
        let mut labels = Vec::new();
        let mut stack = vec![map.foot.clone().upcast::<gtk::Widget>()];
        while let Some(widget) = stack.pop() {
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                labels.push((label.text().to_string(), label.is_visible()));
            }
            let mut child = widget.first_child();
            while let Some(next) = child {
                child = next.next_sibling();
                stack.push(next);
            }
        }
        assert!(map.canvas.scene().stack.is_some(), "no tool group is drawn");
        assert!(
            labels.contains(&(flow.legend_memory_computer.to_string(), true)),
            "the legend's memory item is not shown: {labels:?}"
        );
        assert!(!labels.iter().any(|(text, _)| text == flow.legend_memory));
        // Every legend word is drawn whole inside the pane, at 480 and
        // narrowed to 360: the legend wraps and the foot grows to hold it.
        for narrower in [0, 120] {
            map.root.set_margin_end(narrower);
            settle();
            let mut words = Vec::new();
            let mut stack = vec![map.foot.clone().upcast::<gtk::Widget>()];
            while let Some(widget) = stack.pop() {
                if widget.is::<gtk::Label>() && widget.is_visible() {
                    words.push(widget.clone());
                }
                let mut child = widget.first_child();
                while let Some(next) = child {
                    child = next.next_sibling();
                    stack.push(next);
                }
            }
            let (w, h) = (map.root.width() as f32, map.root.height() as f32);
            for word in &words {
                let text = word.downcast_ref::<gtk::Label>().unwrap().text();
                let bounds = word.compute_bounds(&map.root).expect("placed");
                assert!(
                    word.is_mapped()
                        && bounds.x() >= -0.5
                        && bounds.y() >= -0.5
                        && bounds.x() + bounds.width() <= w + 0.5
                        && bounds.y() + bounds.height() <= h + 0.5,
                    "{text:?} at {bounds:?} is not inside the {w} x {h} pane"
                );
            }
        }
        map.root.set_margin_end(0);

        // The map is drawn with nothing pending too (spec, "No hiding").
        map.canvas
            .set_inputs(map.inputs(&[], &[], None, None, None));
        settle();
        assert!(map.root.is_visible());
        assert!(map.canvas.node_buttons().iter().any(|(id, _)| id == "hub"));

        // The Private AI view is a later slice: the switch shows its empty
        // field and hides the legend and the zoom.
        map.show_view(MapView::PrivateAi);
        assert!(map.private_view.is_active());
        assert!(!map.foot.is_visible());
        map.show_view(MapView::Traces);
        assert!(map.traces_view.is_active());
        assert!(map.foot.is_visible());
        settings.set_gtk_enable_animations(animations);
        window.close();
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_map_focuses_the_camera_on_the_binoculars_tool() {
        gtk::init().expect("GTK display");
        let map = TraceMap::new();
        map.set_pending_read(true);
        map.set_projects_read(true);
        let entries = vec![entry("a", "p", "codex")];
        let window = gtk::Window::builder()
            .default_width(480)
            .default_height(700)
            .child(&map.root)
            .build();
        window.present();
        map.canvas
            .set_inputs(map.inputs(&entries, &[], None, Some("codex".into()), None));
        let canvas = map.canvas.clone();
        settle_until(move || canvas.width() > 0);
        let whole = map.canvas.imp().geometry();
        map.canvas.set_inputs(map.inputs(
            &entries,
            &[],
            None,
            Some("codex".into()),
            Some("codex".into()),
        ));
        settle();
        let focused = map.canvas.imp().geometry();
        assert!(
            focused.scale > whole.scale,
            "the camera closes in on the tool"
        );
        let ringed = map.canvas.scene();
        assert!(
            ringed
                .nodes
                .iter()
                .any(|n| n.id == "tool:codex" && n.ringed)
        );
        window.close();
    }
}
