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
//! What moves (the transfers slice): the flowing arcs' dashes, and the
//! traces the window's transfer player plays (`transfers`,
//! `map_transfers`), on the widget's frame clock. The clock runs only
//! while something moves: at 30 frames a second for the dashes, 60 while a
//! run plays, and not at all under reduced motion (`gtk-enable-animations`
//! off), where the map draws no run and no moving dash and only the cards
//! at its foot say what is happening. The nodes, their names and the pad,
//! and at rest the pile and the rocket on the pad too, are a still layer,
//! kept as a render node and drawn again only when what it is drawn from
//! changes (`StillKey`), as the Mac keys its still layer: at rest a tick
//! redraws only the arcs and the library.
//!
//! The map's style is the Mac's Settings switch (`TracesMapStyle`, kept in
//! `PanePrefs`): on the rocket map, the default, this computer is a launch
//! pad and the library the NEAR AI space station (`station`); on the
//! standard map they are the laptop and the turning orb (`orb`). The
//! library is drawn between the two halves of the still layer on the map's
//! resting timeline (20 frames a second, 30 with moving dashes, never the
//! run's 60), kept between frames (`LibraryArt`), and drawn once at rest
//! with animations off. Under everything is the Mac's field (`field`).
//! The coins carry each tool's logo, or its initials, in its tint
//! (`tool_art`). Colours are resolved from the inks (`MapColours::resolve`):
//! in the standard look the Mac's own map tokens in the flat theme, the
//! words in its text colours, with the theme's ink where the Mac uses its
//! ink; in the Omarchy skin its palette's roles. Every word colour
//! (`word_colour`) is opaque and raised, where it falls short, to 4.5:1 on
//! the map's grounds.
//!
//! The pile opens in the Mac's stages, at its times and on its springs
//! (taller, then wider, then the names fading in), and its coins re-lay,
//! come to the front and grow on theirs (`map_motion::PileMotion`, drawn
//! as a `PileFrame`). The field's shimmer and the light under the pointer
//! move on the same clock (`field`). Those run at the display's rate while
//! they move; the dashes keep their 30 frames a second, the arcs drawn
//! again only when the dashes have moved (`ArcsKey`).
//!
//! Deliberate platform differences, each until a later slice of the
//! 2026-11-16 spec:
//! - A spring turned back while it moves starts again from rest where it
//!   is; SwiftUI carries its velocity (`map_motion`).
//! - The map style switch sits in Settings' Startup section: this shell
//!   has no General section, where the Mac puts it.
//! - A grown coin's glow is a soft radial halo; cairo has no blur filter.
//! - The legend's memory item and the coins' lock say "this computer"
//!   (`legend_memory_computer`) where the Mac's say "this Mac".
//! - The core's line when the map is not current, and the other devices
//!   on the account, are not drawn: GTK reads neither yet.
//!
//! The Private AI view is a second canvas drawing `Scene::private_ai` from
//! `harness_list`, `tool_destinations` and `status`, with no legend, zoom
//! or spinner, as the Mac's `PrivateAIFlowMap` does.
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use gtk::subclass::prelude::*;
use trace_commons_contributor::preview_copy::monitor_screens_copy;

use super::field;
use super::map_motion::{self, Ease, PileMotion, Tween};
use super::map_scene::{
    self, ArcStyle, CommonsGate, Contributions, HarnessRow, Lamp, MapGeometry, NodeKind, PileStage,
    Point, Scene, ScreenState, Size, ToolStack,
};
use super::map_transfers;
use super::orb;
use super::station;
use super::tabs::MapView;
use super::tool_art::{self, Tint};
use super::traces_tree::{ProjectMode, SourceKind, TracesTree};
use super::transfer_card;
use super::transfers::{self, Action, Feed, MapStyle, MapTransfer, Player, Run};
use crate::model::{
    GateStatus, HarnessList, HistoryRecord, Project, QueueEntry, Settings, ToolDestinations,
};
use crate::ui::App;
use crate::ui::omarchy;

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
    /// Set, the canvas draws the Private AI map from these instead of the
    /// Traces map.
    pub private_ai: Option<PrivateAiInputs>,
}

impl Default for SceneInputs {
    fn default() -> Self {
        Self {
            tree: TracesTree::default(),
            gate: CommonsGate::with_state(ScreenState::Loading),
            contributed: Contributions::unread(),
            selected_tool: None,
            focus_tool: None,
            private_ai: None,
        }
    }
}

/// What the Private AI map is built from (`PrivateAIFlowMap`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrivateAiInputs {
    /// `harness_list`'s rows; `None` while unread or after a failed read.
    pub harnesses: Option<Vec<HarnessRow>>,
    /// The core's Private AI state label (`map_scene::private_ai_label`).
    pub private_ai: Option<String>,
    /// `tool_destinations`, for which tools are installed and the managed
    /// routes; `None` after a failed read.
    pub destinations: Option<ToolDestinations>,
}

impl SceneInputs {
    /// The Private AI map's inputs. The Mac draws that map in the default
    /// `ready` state: the Traces map's gate never holds its arcs still.
    pub fn private_ai(inputs: PrivateAiInputs) -> Self {
        Self {
            gate: CommonsGate::with_state(ScreenState::Ready),
            private_ai: Some(inputs),
            ..Self::default()
        }
    }

    /// The scene these inputs draw, laid out in `space`.
    pub fn scene(&self, revealed: &BTreeMap<String, usize>, space: Size) -> Scene {
        match self.private_ai.as_ref() {
            Some(private_ai) => Scene::private_ai(
                private_ai.harnesses.as_deref(),
                private_ai.private_ai.as_deref(),
                map_scene::row_sentence,
                |row| map_scene::harness_state(&row.state),
                private_ai.destinations.as_ref(),
                space,
            )
            .adding_managed_routes(private_ai.destinations.as_ref()),
            None => Scene::traces(
                &self.tree,
                &self.gate,
                &self.contributed,
                self.selected_tool.as_deref(),
                revealed,
                space,
            ),
        }
    }
}

type FolderCallback = Box<dyn Fn(String)>;
type Card = (gtk::Box, gtk::Label, gtk::Label);

/// Whether the desktop asks for reduced motion: animations switched off.
pub fn reduce_motion() -> bool {
    gtk::Settings::default().is_some_and(|settings| !settings.is_gtk_enable_animations())
}

/// What the still layer is drawn from: the nodes, their names and the
/// pad. Equal keys draw the same picture, so a tick of the clock that
/// changes none of them draws none of it again.
#[derive(Clone, Debug, PartialEq)]
pub struct StillKey {
    pub scene: Scene,
    pub fit: MapGeometry,
    pub selection: Option<String>,
    pub palette: Palette,
    /// At rest, how far the pile is open: the pile, its words and the
    /// rocket on the pad are then part of the layer. `None` during a run,
    /// when they move and are drawn frame by frame.
    pub resting: Option<PileStage>,
    /// The map's style: the pad or the laptop, and the rocket on the pad.
    pub style: MapStyle,
}

impl StillKey {
    /// Whether this key is the one for these, without building another.
    pub fn is(
        &self,
        scene: &Scene,
        fit: &MapGeometry,
        selection: Option<&str>,
        palette: &Palette,
        resting: Option<PileStage>,
        style: MapStyle,
    ) -> bool {
        self.scene == *scene
            && self.fit == *fit
            && self.selection.as_deref() == selection
            && self.palette == *palette
            && self.resting == resting
            && self.style == style
    }
}

/// The still layer's two render nodes: the nodes and their names, under
/// the station; and, at rest, the rocket and the pile, over it.
#[derive(Clone, Default)]
pub struct StillLayer {
    pub under: Option<gtk::gsk::RenderNode>,
    pub over: Option<gtk::gsk::RenderNode>,
}

/// What the arcs were last drawn from: a frame drawn for the pile, the
/// shimmer or the pointer redraws them only when their dashes have moved.
#[derive(Clone, PartialEq)]
pub struct ArcsKey {
    pub phase: f64,
    pub healthy: bool,
    pub scene: Scene,
    pub fit: MapGeometry,
    pub palette: Palette,
}

/// The pile as drawn at one moment (`MapToolPile`): how far its card is
/// tall and wide, how opaque its fill and its names, how far its coins are
/// laid open, and each coin's place and growth.
#[derive(Clone, Debug, PartialEq)]
pub struct PileFrame {
    pub tall: f64,
    pub wide: f64,
    pub opaque: f64,
    pub names: f64,
    pub lay: f64,
    /// The pile laid open, as its coins head: every watched coin then
    /// carries the lock.
    pub open: bool,
    /// Front first, in the order the coins head for.
    pub coins: Vec<CoinFrame>,
}

/// One coin of a `PileFrame`.
#[derive(Clone, Debug, PartialEq)]
pub struct CoinFrame {
    pub id: String,
    /// Its place in the order it heads for.
    pub slot: usize,
    /// Where it is now, an index between two places while it moves.
    pub at: f64,
    /// How far grown: 0 at its size, 1 at `map_motion::GROWN`.
    pub grow: f64,
    /// A trace is in it: drawn over the rest.
    pub grown: bool,
}

impl PileFrame {
    /// The pile at rest at `stage`, its coins in `order`, those in
    /// `grown` grown.
    pub fn at_rest(order: &[String], grown: &[String], stage: PileStage) -> Self {
        let one = |on: bool| if on { 1.0 } else { 0.0 };
        Self {
            tall: one(stage.tall),
            wide: one(stage.wide),
            opaque: one(stage.open()),
            names: one(stage.tall && stage.names),
            lay: one(stage.tall),
            open: stage.tall,
            coins: order
                .iter()
                .enumerate()
                .map(|(slot, id)| CoinFrame {
                    id: id.clone(),
                    slot,
                    at: slot as f64,
                    grow: one(grown.contains(id)),
                    grown: grown.contains(id),
                })
                .collect(),
        }
    }

    /// The pile as `motion` has it at `now`, heading for `stage`. While a
    /// trace moves (`moving`) the card keeps its closed size and shows no
    /// names, at once: on the Mac that flip is in no animation.
    pub fn moving(
        motion: &PileMotion,
        stage: PileStage,
        moving: bool,
        order: &[String],
        grown: &[String],
        now: f64,
    ) -> Self {
        let card = |tween: &Tween| if moving { 0.0 } else { tween.value(now) };
        let wide = card(&motion.wide);
        Self {
            tall: card(&motion.tall),
            wide,
            // An open card is opaque: its fill turns with its width.
            opaque: wide.clamp(0.0, 1.0),
            names: card(&motion.names).clamp(0.0, 1.0),
            lay: motion.lay.value(now),
            open: stage.tall && !moving,
            coins: order
                .iter()
                .enumerate()
                .map(|(slot, id)| {
                    let coin = motion.coin(id);
                    CoinFrame {
                        id: id.clone(),
                        slot,
                        at: coin.map_or(slot as f64, |c| c.index.value(now)),
                        grow: coin.map_or(0.0, |c| c.grow.value(now)),
                        grown: grown.contains(id),
                    }
                })
                .collect(),
        }
    }

    /// The coin at `index` of the order, where it is now.
    pub fn coin(&self, id: &str) -> Option<&CoinFrame> {
        self.coins.iter().find(|coin| coin.id == id)
    }
}

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
        /// How far the tool pile is open (`MapToolPile`'s stages), and
        /// whether the pointer is over its card.
        pub pile_stage: Cell<PileStage>,
        pub pile_inside: Cell<bool>,
        /// Counts the pointer's comings and goings over the pile: a stage
        /// still to come from an earlier one is dropped (a fired timeout
        /// cannot be removed again, so stale ones are ignored instead).
        pub pile_turn: Cell<u64>,
        /// Each node's button, by node id, in keyboard order.
        pub nodes: RefCell<Vec<(String, gtk::Button)>>,
        pub pill: RefCell<Option<gtk::Button>>,
        pub card: RefCell<Option<Card>>,
        pub on_select_folder: RefCell<Option<FolderCallback>>,
        /// The run the window's player is playing, if any.
        pub run: RefCell<Option<Run>>,
        /// The frame clock's callback, while something moves.
        pub tick: RefCell<Option<gtk::TickCallbackId>>,
        /// The clock's last drawn frame and its rate, in seconds.
        pub last_frame: Cell<f64>,
        pub interval: Cell<f64>,
        /// The still layer and what it was drawn from: what lies under the
        /// station, and, at rest, the rocket and the pile over it.
        pub still: RefCell<Option<(StillKey, StillLayer)>>,
        /// The pile's coin order and openness the buttons were last
        /// allocated for.
        pub pile_at: RefCell<(Vec<String>, PileStage)>,
        /// The legend's memory item, shown while the tool group is drawn.
        pub memory_legend: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
        /// The Traces map's style (Settings): the rocket and station, or
        /// the laptop and the orb.
        pub style: Cell<MapStyle>,
        /// The library's station or orb, kept between resting frames.
        pub library: LibraryArt,
        /// The map's field under everything, kept until its size or
        /// scheme changes.
        pub field: field::Field,
        /// The pile as it moves toward `pile_stage` and the run's order.
        pub pile_motion: RefCell<PileMotion>,
        /// The light under the pointer: the last point it was at, and how
        /// far it is faded in.
        pub light: RefCell<(Option<(f64, f64)>, Tween)>,
        /// When the shimmer's pass now crossing began, and the timer for
        /// the next.
        pub shimmer_start: Cell<Option<f64>>,
        pub shimmer_timer: RefCell<Option<glib::SourceId>>,
        /// The clock's time the dashes were last moved to, and their rate.
        pub dash_time: Cell<f64>,
        pub dash_interval: Cell<f64>,
        /// The arcs, kept while their dashes have not moved.
        pub arcs: RefCell<Option<(ArcsKey, gtk::gsk::RenderNode)>>,
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
            pill.add_css_class("tc-glass-control");
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
            card.add_css_class("tc-glass-node-card");
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

            // The clock runs only while the map is mapped. The shimmer's
            // first pass comes a second after the map is shown
            // (`GlassMapShimmerHost`), and none comes while it is not.
            obj.connect_map(|obj| {
                obj.schedule_shimmer(map_motion::SHIMMER_FIRST);
                obj.update_clock();
            });
            obj.connect_unmap(|obj| {
                obj.stop_shimmer();
                obj.update_clock();
            });
            // Minimised or suspended, the clock and the shimmer hold, as
            // while unmapped; shown again, the shimmer starts over.
            // Only a change in hidden counts: the window's other state
            // changes (focus) leave the shimmer's pass where it is.
            let weak = obj.downgrade();
            let hidden = Cell::new(false);
            crate::ui::glass_motion::on_window_state(&*obj, move |_| {
                let Some(obj) = weak.upgrade() else { return };
                let now = crate::ui::glass_motion::window_hidden(&obj);
                match crate::ui::glass_motion::hidden_changed(&hidden, now) {
                    Some(true) => obj.stop_shimmer(),
                    Some(false) => obj.schedule_shimmer(map_motion::SHIMMER_FIRST),
                    None => {}
                }
                obj.update_clock();
            });

            // Reduced motion stops the clock at once and puts everything
            // that moves at its end, and lets it run again: the shimmer
            // starts over, as the Mac's task restarts on the setting.
            if let Some(settings) = gtk::Settings::default() {
                let weak = obj.downgrade();
                settings.connect_gtk_enable_animations_notify(move |settings| {
                    if let Some(obj) = weak.upgrade() {
                        if settings.is_gtk_enable_animations() {
                            obj.schedule_shimmer(map_motion::SHIMMER_FIRST);
                        } else {
                            let imp = obj.imp();
                            obj.stop_shimmer();
                            imp.pile_motion.borrow_mut().settle();
                            imp.light.borrow_mut().1.settle();
                        }
                        obj.update_clock();
                        obj.queue_allocate();
                        obj.queue_draw();
                    }
                });
            }
        }

        fn dispose(&self) {
            if let Some(tick) = self.tick.take() {
                tick.remove();
            }
            if let Some(timer) = self.shimmer_timer.take() {
                timer.remove();
            }
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
            self.pile_at.replace(self.pile(&scene));
            // Each coin's button where the coin is drawn now.
            let frame = self.pile_frame(&scene);
            for (id, button) in self.nodes.borrow().iter() {
                let Some(node) = scene.nodes.iter().find(|n| n.id == *id) else {
                    button.set_child_visible(false);
                    continue;
                };
                button.set_child_visible(true);
                let at = frame
                    .as_ref()
                    .and_then(|frame| {
                        let coin = frame.coin(id)?;
                        Some(scene.stack.as_ref()?.coin_center_at(coin.at, frame.lay))
                    })
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
                        let half = map_scene::drawn_half_width(
                            &node,
                            scene.stack.is_some(),
                            self.style.get(),
                        ) * fit.scale;
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
            let reduce = reduce_motion();
            let now = self.now();
            // The field under both maps (`GlassMapField`), its shimmer and
            // the light under the pointer over it.
            let dark = adw::StyleManager::default().is_dark();
            snapshot.append_node(self.field.node(width, height, dark, &palette.map));
            self.draw_field_light(snapshot, &bounds, now, dark);
            // The arcs, drawn again when their dashes have moved.
            let healthy = self.inputs.borrow().gate.state.is_healthy();
            let phase = transfers::dash_phase(self.dash_time.get(), reduce);
            snapshot.append_node(self.arcs_node(&scene, &fit, &palette, healthy, phase, &bounds));
            // The nodes, their names and the pad: drawn again only when
            // what they are drawn from changes.
            let motion = self.motion();
            // At rest the pile, its words and the rocket on the pad are
            // still too, so they join the still layer; during a run, or
            // while the pile moves, they are drawn on every frame.
            let frame = self.pile_frame(&scene);
            let pile_moves = self.pile_motion.borrow().moving(now);
            let resting = (motion.is_none() && !pile_moves).then(|| self.pile_stage.get());
            let still = self.still_layer(&scene, &fit, &palette, &bounds, resting);
            if let Some(under) = still.as_ref().and_then(|layer| layer.under.as_ref()) {
                snapshot.append_node(under);
            }
            // The library's station or orb turns on the map's resting
            // timeline, over the nodes and under the rocket and the pile,
            // as the Mac draws it: during a run too, it moves at the
            // resting rate, not the run's.
            let style = self.style.get();
            if let Some(frame) = library_frame(&scene, &fit, style) {
                let interval = transfers::resting_motion(
                    reduce,
                    scene.flows(),
                    healthy,
                    library_turns(&scene),
                )
                .interval;
                let seconds = library_seconds(self.now(), interval);
                snapshot.append_node(self.library.node(style, &frame, seconds, reduce));
            }
            if let Some(over) = still.as_ref().and_then(|layer| layer.over.as_ref()) {
                snapshot.append_node(over);
            }
            if let Some((run, t)) = motion.as_ref() {
                self.draw_moving(
                    snapshot,
                    &scene,
                    run,
                    *t,
                    frame.as_ref(),
                    &fit,
                    &palette,
                    &bounds,
                );
            } else if resting.is_none() {
                // The pile moves with no run: the rocket on the pad, and
                // the pile over it as it is now.
                let cr = snapshot.append_cairo(&bounds);
                map_transfers::resting_rocket(&cr, &scene, &fit, &palette, self.style.get());
                if let (Some(stack), Some(frame)) = (scene.stack.as_ref(), frame.as_ref()) {
                    draw_pile_frame(&cr, &scene, stack, frame, None, &fit, &palette);
                }
                drop(cr);
                if let Some(frame) = frame.as_ref() {
                    self.draw_pile_words(snapshot, &scene, frame, &fit, &palette);
                }
            }
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
            let scene = inputs.scene(&self.revealed.borrow(), space);
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

        pub(super) fn point_at(&self, pointer: Option<(f64, f64)>) {
            // The light follows the pointer as it is, and fades in as it
            // comes over the map and out where it left
            // (`GlassMapMarkGlow`, `GlassMapLight`).
            {
                let now = self.now();
                let ease = (!reduce_motion()).then(|| Ease::token(map_motion::FAST));
                let mut light = self.light.borrow_mut();
                match pointer {
                    Some(point) => {
                        light.0 = Some(point);
                        light.1.retarget(1.0, now, ease);
                    }
                    None => light.1.retarget(0.0, now, ease),
                }
            }
            self.obj().update_clock();
            self.obj().queue_draw();
            let inside = {
                let scene = self.scene.borrow();
                let Some(stack) = scene.stack.as_ref() else {
                    return;
                };
                // The card as drawn: while a trace moves it keeps its size.
                let drawn = self.pile(&scene).1;
                let (x, y, w, h) = stack.card(&self.geometry(), drawn.tall, drawn.wide);
                pointer.is_some_and(|(px, py)| px >= x && px <= x + w && py >= y && py <= y + h)
            };
            if inside == self.pile_inside.replace(inside) {
                return;
            }
            // A change of mind cancels the stages still to come
            // (`MapToolPile.hover`).
            let turn = self.pile_turn.get() + 1;
            self.pile_turn.set(turn);
            for (delay, step) in map_scene::pile_steps(inside, reduce_motion()) {
                if delay == 0 {
                    self.pile_stage.set(self.pile_stage.get().with(step));
                    self.pile_motion
                        .borrow_mut()
                        .step(step, self.now(), reduce_motion());
                    continue;
                }
                let weak = self.obj().downgrade();
                glib::timeout_add_local_once(
                    std::time::Duration::from_millis(u64::from(delay)),
                    move || {
                        if let Some(obj) = weak.upgrade() {
                            let imp = obj.imp();
                            if imp.pile_turn.get() != turn {
                                return;
                            }
                            imp.pile_stage.set(imp.pile_stage.get().with(step));
                            imp.pile_motion
                                .borrow_mut()
                                .step(step, imp.now(), reduce_motion());
                            obj.update_clock();
                            obj.queue_allocate();
                            obj.queue_draw();
                        }
                    },
                );
            }
            let obj = self.obj();
            obj.update_clock();
            obj.queue_allocate();
            obj.queue_draw();
        }

        /// The shimmer crossing the field and the light under the
        /// pointer, over the field: nothing while neither shows.
        fn draw_field_light(
            &self,
            snapshot: &gtk::Snapshot,
            bounds: &gtk::graphene::Rect,
            now: f64,
            dark: bool,
        ) {
            let obj = self.obj();
            let (width, height) = (obj.width(), obj.height());
            let progress = self
                .shimmer_start
                .get()
                .map_or(0.0, |start| map_motion::shimmer_progress(now - start));
            let light = {
                let light = self.light.borrow();
                light
                    .0
                    .map(|point| (point, light.1.value(now).clamp(0.0, 1.0)))
            }
            .filter(|(_, opacity)| *opacity > 0.0);
            if progress <= 0.0 && light.is_none() {
                return;
            }
            let cr = snapshot.append_cairo(bounds);
            if progress > 0.0
                && let Some(mask) = self.field.tile_mask(width, height, obj.scale_factor())
            {
                field::draw_shimmer(
                    &cr,
                    &mask,
                    f64::from(width),
                    f64::from(height),
                    progress,
                    dark,
                );
            }
            if let Some((point, opacity)) = light {
                field::draw_light(
                    &cr,
                    f64::from(width),
                    f64::from(height),
                    point,
                    opacity,
                    dark,
                );
            }
        }

        /// The arcs, kept while their dashes, the scene and the colours
        /// hold.
        fn arcs_node(
            &self,
            scene: &Scene,
            fit: &MapGeometry,
            palette: &Palette,
            healthy: bool,
            phase: f64,
            bounds: &gtk::graphene::Rect,
        ) -> gtk::gsk::RenderNode {
            if let Some((key, node)) = self.arcs.borrow().as_ref()
                && key.phase == phase
                && key.healthy == healthy
                && key.fit == *fit
                && key.scene == *scene
                && key.palette == *palette
            {
                return node.clone();
            }
            let arcs = gtk::Snapshot::new();
            let cr = arcs.append_cairo(bounds);
            draw_arcs(&cr, scene, fit, palette, healthy, phase);
            drop(cr);
            let node = arcs
                .to_node()
                .unwrap_or_else(|| gtk::gsk::ContainerNode::new(&[]).upcast());
            let key = ArcsKey {
                phase,
                healthy,
                scene: scene.clone(),
                fit: *fit,
                palette: palette.clone(),
            };
            self.arcs.replace(Some((key, node.clone())));
            node
        }

        /// The pile as drawn now, its coins following the run's order and
        /// growth; `None` with no pile.
        pub fn pile_frame(&self, scene: &Scene) -> Option<PileFrame> {
            let stack = scene.stack.as_ref()?;
            let now = self.now();
            let stage = self.pile_stage.get();
            let (order, moving, grown) = match self.motion() {
                Some((run, t)) => (
                    run.order(&stack.tools, t),
                    run.moving(t),
                    run.orbits(t).into_iter().map(|(id, _)| id).collect(),
                ),
                None => (stack.tools.clone(), false, Vec::new()),
            };
            let mut motion = self.pile_motion.borrow_mut();
            motion.follow(now, stage.tall && !moving, &order, &grown, reduce_motion());
            Some(PileFrame::moving(
                &motion, stage, moving, &order, &grown, now,
            ))
        }

        /// Whether anything drawn at the display's rate moves at `now`:
        /// the pile, the shimmer, the light fading.
        pub fn fine_motion(&self, now: f64) -> bool {
            {
                let scene = self.scene.borrow();
                // The coins' targets first: a run's step or end moves them.
                let _ = self.pile_frame(&scene);
            }
            self.pile_motion.borrow().moving(now)
                || self.light.borrow().1.moving(now)
                || self
                    .shimmer_start
                    .get()
                    .is_some_and(|start| now - start < map_motion::SHIMMER_DURATION)
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

        /// The seconds on the map's clock: the frame clock's, so a run and
        /// the dashes move with the frames drawn.
        pub fn now(&self) -> f64 {
            self.obj().frame_clock().map_or_else(
                || glib::monotonic_time() as f64 / 1e6,
                |clock| clock.frame_time() as f64 / 1e6,
            )
        }

        /// The run the map draws and the seconds into it: none under
        /// reduced motion.
        pub fn motion(&self) -> Option<(Run, f64)> {
            let run = self.run.borrow();
            let run = transfers::drawn_run(run.as_ref(), reduce_motion())?;
            Some((run.clone(), self.now() - run.start))
        }

        /// The pile's coin order, front first, and how far it is open:
        /// while a trace moves the pile keeps its size, and each trace's
        /// tool comes to the front as it sets off.
        pub fn pile(&self, scene: &Scene) -> (Vec<String>, PileStage) {
            let Some(stack) = scene.stack.as_ref() else {
                return (Vec::new(), PileStage::default());
            };
            let stage = self.pile_stage.get();
            match self.motion() {
                Some((run, t)) => (run.order(&stack.tools, t), stage.drawn(run.moving(t))),
                None => (stack.tools.clone(), stage),
            }
        }

        /// Whether the pile has moved since its buttons were allocated.
        pub fn pile_moved(&self) -> bool {
            let scene = self.scene.borrow();
            self.pile_motion.borrow().moving(self.now())
                || *self.pile_at.borrow() != self.pile(&scene)
        }

        /// The still layer: kept while its key holds, drawn again when not.
        fn still_layer(
            &self,
            scene: &Scene,
            fit: &MapGeometry,
            palette: &Palette,
            bounds: &gtk::graphene::Rect,
            resting: Option<PileStage>,
        ) -> Option<StillLayer> {
            let selection = self.selection();
            let kept = self
                .still
                .borrow()
                .as_ref()
                .filter(|(key, _)| {
                    key.is(
                        scene,
                        fit,
                        selection.as_deref(),
                        palette,
                        resting,
                        self.style.get(),
                    )
                })
                .map(|(_, layer)| layer.clone());
            if kept.is_some() {
                return kept;
            }
            let under = gtk::Snapshot::new();
            let cr = under.append_cairo(bounds);
            for node in &scene.nodes {
                let selected = selection.as_deref() == Some(node.id.as_str());
                draw_node(&cr, scene, node, fit, palette, selected, self.style.get());
            }
            drop(cr);
            self.draw_node_words(&under, scene, fit, palette);
            let over = resting.map(|open| {
                // The rocket on the pad, then the pile over it.
                let over = gtk::Snapshot::new();
                let cr = over.append_cairo(bounds);
                map_transfers::resting_rocket(&cr, scene, fit, palette, self.style.get());
                let frame = scene
                    .stack
                    .as_ref()
                    .map(|stack| PileFrame::at_rest(&stack.tools, &[], open));
                if let (Some(stack), Some(frame)) = (scene.stack.as_ref(), frame.as_ref()) {
                    draw_pile_frame(&cr, scene, stack, frame, None, fit, palette);
                }
                drop(cr);
                if let Some(frame) = frame.as_ref() {
                    self.draw_pile_words(&over, scene, frame, fit, palette);
                }
                over.to_node()
            });
            let layer = StillLayer {
                under: under.to_node(),
                over: over.flatten(),
            };
            let key = StillKey {
                scene: scene.clone(),
                fit: *fit,
                selection: selection.clone(),
                palette: palette.clone(),
                resting,
                style: self.style.get(),
            };
            self.still.replace(Some((key, layer.clone())));
            Some(layer)
        }

        /// What moves during a run, over the still layer and frame by frame:
        /// the traces, the rocket, the pile in the run's order with its
        /// grown coin, the rim light, and the run's words.
        #[allow(clippy::too_many_arguments)]
        fn draw_moving(
            &self,
            snapshot: &gtk::Snapshot,
            scene: &Scene,
            run: &Run,
            t: f64,
            frame: Option<&PileFrame>,
            fit: &MapGeometry,
            palette: &Palette,
            bounds: &gtk::graphene::Rect,
        ) {
            let cr = snapshot.append_cairo(bounds);
            let dock = map_transfers::draw(&cr, scene, run, t, fit, palette);
            // Between contributions the rocket stands on the pad.
            if run.action() != Action::Contribute {
                map_transfers::resting_rocket(&cr, scene, fit, palette, self.style.get());
            }
            if let (Some(stack), Some(frame)) = (scene.stack.as_ref(), frame) {
                let glow = map_transfers::glow(Some(run.action()), palette);
                draw_pile_frame(&cr, scene, stack, frame, Some(&glow), fit, palette);
            }
            drop(cr);
            if let Some(frame) = frame {
                self.draw_pile_words(snapshot, scene, frame, fit, palette);
            }
            // The light round a coin's rim, over the coins.
            if let Some(stack) = scene.stack.as_ref() {
                let cr = snapshot.append_cairo(bounds);
                map_transfers::rim_lights(&cr, stack, run, t, fit, palette);
                drop(cr);
            }
            self.draw_run_words(snapshot, scene, run, t, dock, fit, palette);
        }

        /// The run's words: the line under the pad while a contribution's
        /// traces gather, and the library's "+N" as the rocket docks.
        #[allow(clippy::too_many_arguments)]
        fn draw_run_words(
            &self,
            snapshot: &gtk::Snapshot,
            scene: &Scene,
            run: &Run,
            t: f64,
            dock: Option<map_transfers::DockLabel>,
            fit: &MapGeometry,
            palette: &Palette,
        ) {
            let words = map_scene::words();
            let s = fit.scale;
            if let Some(line) = transfers::line_under_pad(run, t, &words)
                && let Some(hub) = scene.nodes.iter().find(|n| n.id == "hub")
            {
                let (_, sublabel_size, _, sublabel_offset) = map_scene::label_type(s);
                let centre = fit.point(hub.at);
                self.text(
                    snapshot,
                    &line,
                    Point::new(centre.x, centre.y + 16.0 * s + sublabel_offset),
                    sublabel_size,
                    false,
                    Anchor::Middle,
                    &word_colour(palette, Word::PadLine),
                );
            }
            if let Some(dock) = dock {
                self.text(
                    snapshot,
                    &dock.text,
                    dock.at,
                    (10.0 * s).max(9.0),
                    true,
                    Anchor::Middle,
                    &word_colour(palette, Word::Dock),
                );
            }
        }

        fn draw_node_words(
            &self,
            snapshot: &gtk::Snapshot,
            scene: &Scene,
            fit: &MapGeometry,
            palette: &Palette,
        ) {
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
                let (ink, sublabel_colour) = node_word_colours(node, palette);
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
                    self.text(
                        snapshot,
                        sublabel,
                        at,
                        sublabel_size,
                        false,
                        Anchor::Middle,
                        &sublabel_colour,
                    );
                }
            }
            // The micro type: 10 bold, not scaled with the map.
            for overflow in &scene.overflows {
                let text = format!("+{}", overflow.count);
                self.text(
                    snapshot,
                    &text,
                    fit.point(overflow.at),
                    MarkFont::MICRO.size,
                    true,
                    Anchor::Middle,
                    &overflow_colour(palette),
                );
            }
        }

        /// The pile's heading and, open, each coin's name and what waits
        /// from it, in the pile's order. The coins' marks are drawn with
        /// the coins (`draw_pile`).
        fn draw_pile_words(
            &self,
            snapshot: &gtk::Snapshot,
            scene: &Scene,
            frame: &PileFrame,
            fit: &MapGeometry,
            palette: &Palette,
        ) {
            let words = map_scene::words();
            let Some(stack) = scene.stack.as_ref() else {
                return;
            };
            let s = fit.scale;
            let left = fit.point(stack.port_left());
            let height = stack.height_at(frame.tall) * s;
            self.text(
                snapshot,
                &words.tools_heading.to_uppercase(),
                Point::new(left.x + 30.0 * s, left.y - height / 2.0 - 12.0 * s),
                (10.0 * s).max(9.0),
                true,
                Anchor::Middle,
                &word_colour(palette, Word::PileHeading),
            );
            for coin in &frame.coins {
                let Some(node) = scene.nodes.iter().find(|n| n.id == coin.id) else {
                    continue;
                };
                let centre = fit.point(stack.coin_center_at(coin.at, frame.lay));
                // The names fade in at their own stage, after the card has
                // widened, and out first (`MapToolPile`'s `names`).
                let shown = if node.dashed { 0.55 } else { 1.0 } * frame.names;
                if shown <= 0.0 {
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
                let colour = faded_word(palette, Word::CoinName, shown);
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
                    let colour = faded_word(palette, Word::CoinLine, shown);
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

    /// Draws the Traces map in `style` (Settings): the pad, rocket and
    /// station, or the laptop and the orb. It changes at once.
    pub fn set_style(&self, style: MapStyle) {
        if self.imp().style.replace(style) == style {
            return;
        }
        self.queue_allocate();
        self.queue_draw();
    }

    pub fn style(&self) -> MapStyle {
        self.imp().style.get()
    }

    fn relayout(&self) {
        self.imp().layout_scene();
        self.sync_children();
        self.update_clock();
        self.queue_allocate();
        self.queue_draw();
    }

    /// The run the window's player is playing, or none.
    pub fn set_run(&self, run: Option<Run>) {
        let imp = self.imp();
        if *imp.run.borrow() == run {
            return;
        }
        imp.run.replace(run);
        self.update_clock();
        self.queue_allocate();
        self.queue_draw();
    }

    pub fn run(&self) -> Option<Run> {
        self.imp().run.borrow().clone()
    }

    /// Whether the map's clock is running: only while something moves.
    pub fn is_ticking(&self) -> bool {
        self.imp().tick.borrow().is_some()
    }

    /// Runs the frame clock while something moves and stops it when
    /// nothing does: the dashes at 30 frames a second, a run at 60, the
    /// pile, the shimmer and the light at the display's rate while they
    /// move, and nothing at all under reduced motion
    /// (`FlowMapView.restingMotion`).
    fn update_clock(&self) {
        let imp = self.imp();
        let Some(interval) = self.clock_wanted() else {
            if let Some(tick) = imp.tick.take() {
                tick.remove();
            }
            return;
        };
        imp.interval.set(interval);
        if imp.tick.borrow().is_some() {
            return;
        }
        let tick = self.add_tick_callback(|canvas, clock| {
            let imp = canvas.imp();
            let now = clock.frame_time() as f64 / 1e6;
            let Some(interval) = canvas.clock_wanted() else {
                // Nothing moves any more: one last frame at rest.
                imp.tick.take();
                canvas.queue_allocate();
                canvas.queue_draw();
                return glib::ControlFlow::Break;
            };
            imp.interval.set(interval);
            // A little under the interval, so a frame that lands just early
            // is not skipped.
            if now - imp.dash_time.get() >= imp.dash_interval.get() * 0.9 {
                imp.dash_time.set(now);
            }
            if now - imp.last_frame.get() >= interval * 0.9 {
                imp.last_frame.set(now);
                if imp.pile_moved() {
                    canvas.queue_allocate();
                }
                canvas.queue_draw();
            }
            glib::ControlFlow::Continue
        });
        imp.tick.replace(Some(tick));
    }

    /// The clock's rate while something moves; `None` when nothing does.
    /// Only while the map can be seen: unmapped (hidden, or its pane
    /// collapsed) it draws nothing, so its clock stops, and mapping it
    /// again starts it (`GlassAnimationTimeline`'s visibility gate).
    fn clock_wanted(&self) -> Option<f64> {
        let imp = self.imp();
        if !self.is_mapped() || crate::ui::glass_motion::window_hidden(self) {
            return None;
        }
        let reduce = reduce_motion();
        let flows = imp.scene.borrow().flows();
        let healthy = imp.inputs.borrow().gate.state.is_healthy();
        // The Traces map's station turns at its own rate while nothing
        // else moves.
        let library = library_turns(&imp.scene.borrow());
        let resting = transfers::resting_motion(reduce, flows, healthy, library);
        let running = transfers::drawn_run(imp.run.borrow().as_ref(), reduce).is_some();
        let dashes = if running {
            transfers::RUN_INTERVAL
        } else {
            resting.interval
        };
        imp.dash_interval.set(dashes);
        if !reduce && imp.fine_motion(imp.now()) {
            return Some(0.0);
        }
        (running || !resting.paused).then_some(dashes)
    }

    /// The shimmer's next pass `delay` seconds from now, and one every
    /// `SHIMMER_EVERY` after it; none while the map is not shown or under
    /// reduced motion.
    fn schedule_shimmer(&self, delay: f64) {
        let imp = self.imp();
        if let Some(timer) = imp.shimmer_timer.take() {
            timer.remove();
        }
        if !self.is_mapped() || reduce_motion() || crate::ui::glass_motion::window_hidden(self) {
            return;
        }
        let weak = self.downgrade();
        let timer =
            glib::timeout_add_local_once(std::time::Duration::from_secs_f64(delay), move || {
                let Some(canvas) = weak.upgrade() else { return };
                let imp = canvas.imp();
                // This timer has fired: forget it rather than remove it.
                imp.shimmer_timer.take();
                if !canvas.is_mapped()
                    || reduce_motion()
                    || crate::ui::glass_motion::window_hidden(&canvas)
                {
                    return;
                }
                imp.shimmer_start.set(Some(imp.now()));
                canvas.update_clock();
                canvas.queue_draw();
                canvas.schedule_shimmer(map_motion::SHIMMER_EVERY);
            });
        imp.shimmer_timer.replace(Some(timer));
    }

    /// No shimmer now, and none to come.
    fn stop_shimmer(&self) {
        let imp = self.imp();
        if let Some(timer) = imp.shimmer_timer.take() {
            timer.remove();
        }
        imp.shimmer_start.set(None);
    }

    /// Whether a shimmer pass is due: its timer is set.
    pub fn shimmer_scheduled(&self) -> bool {
        self.imp().shimmer_timer.borrow().is_some()
    }

    /// The clock's rate now: 0 for every frame.
    pub fn clock_interval(&self) -> f64 {
        self.imp().interval.get()
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

// --- Drawing -----------------------------------------------------------------

/// The colours the map draws with: the theme's ink and background, and
/// every map colour resolved from the inks (`MapColours::resolve`).
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub ink: gtk::gdk::RGBA,
    pub background: gtk::gdk::RGBA,
    /// What the map draws in: the Mac's map colours in the standard look,
    /// the Omarchy palette's roles in the skin, each word colour opaque and
    /// clear of 4.5:1 on the map's grounds.
    pub map: MapColours,
}

/// The Traces map's colours on the Mac, as its flat theme draws them in
/// one scheme (`brand_tokens::map`, generated from the glass tokens).
#[derive(Clone, Debug, PartialEq)]
pub struct MapColours {
    pub orb_shared: gtk::gdk::RGBA,
    pub orb_kept: gtk::gdk::RGBA,
    pub purple_soft: gtk::gdk::RGBA,
    pub blue: gtk::gdk::RGBA,
    pub status_off: gtk::gdk::RGBA,
    pub status_on: gtk::gdk::RGBA,
    pub status_on_text: gtk::gdk::RGBA,
    pub status_ask: gtk::gdk::RGBA,
    pub status_outside: gtk::gdk::RGBA,
    pub node_off: gtk::gdk::RGBA,
    /// The folder rules' fills: share automatically, ask, never, unset.
    pub rule_fills: [gtk::gdk::RGBA; 4],
    pub credential_on: gtk::gdk::RGBA,
    pub pane_opaque: gtk::gdk::RGBA,
    pub well_fill: gtk::gdk::RGBA,
    /// The map field's veil, at its centre and toward its edges.
    pub field_inner: gtk::gdk::RGBA,
    pub field_outer: gtk::gdk::RGBA,
    pub text_primary: gtk::gdk::RGBA,
    pub text_secondary: gtk::gdk::RGBA,
    pub text_tertiary: gtk::gdk::RGBA,
    /// A node's sublabel (`FlowMapView.drawNode`'s statusOff), kept apart
    /// from `status_off` so the rings keep the Mac's grey while the words
    /// are raised to the text floor.
    pub sublabel: gtk::gdk::RGBA,
    /// `GlassColor.accentText`: the purpleText token.
    pub accent_text: gtk::gdk::RGBA,
    /// Claude, Codex, Antigravity, OpenCode, Theia.
    pub tools: [gtk::gdk::RGBA; 5],
    pub hero_deep: gtk::gdk::RGBA,
    pub hero_bright: gtk::gdk::RGBA,
}

impl MapColours {
    /// The colours for one scheme, read from the tokens once a scheme and
    /// kept: the map asks for them on every frame it draws.
    pub fn scheme(dark: bool) -> Self {
        thread_local! {
            static KEPT: [std::cell::OnceCell<MapColours>; 2] = Default::default();
        }
        KEPT.with(|kept| {
            kept[usize::from(dark)]
                .get_or_init(|| Self::read(dark))
                .clone()
        })
    }

    fn read(dark: bool) -> Self {
        use crate::ui::style::brand_tokens::map::{Rgba, dark as d, light as l};
        let c = |(hex, alpha): Rgba| {
            let rgba = gtk::gdk::RGBA::parse(hex).unwrap_or(gtk::gdk::RGBA::BLACK);
            rgba.with_alpha(alpha as f32)
        };
        let pick = |light: Rgba, dark_value: Rgba| c(if dark { dark_value } else { light });
        Self {
            orb_shared: pick(l::MAP_ORB_SHARED, d::MAP_ORB_SHARED),
            orb_kept: pick(l::MAP_ORB_KEPT, d::MAP_ORB_KEPT),
            purple_soft: pick(l::PURPLE_SOFT, d::PURPLE_SOFT),
            blue: pick(l::BLUE, d::BLUE),
            status_off: pick(l::STATUS_OFF, d::STATUS_OFF),
            status_on: pick(l::STATUS_ON, d::STATUS_ON),
            status_on_text: pick(l::STATUS_ON_TEXT, d::STATUS_ON_TEXT),
            status_ask: pick(l::STATUS_ASK, d::STATUS_ASK),
            status_outside: pick(l::STATUS_OUTSIDE, d::STATUS_OUTSIDE),
            node_off: pick(l::MAP_NODE_OFF, d::MAP_NODE_OFF),
            rule_fills: [
                pick(l::MAP_RULE_AUTO_FILL, d::MAP_RULE_AUTO_FILL),
                pick(l::MAP_RULE_ASK_FILL, d::MAP_RULE_ASK_FILL),
                pick(l::MAP_RULE_IGNORE_FILL, d::MAP_RULE_IGNORE_FILL),
                pick(l::MAP_RULE_UNSET_FILL, d::MAP_RULE_UNSET_FILL),
            ],
            credential_on: pick(l::MAP_CREDENTIAL_ON, d::MAP_CREDENTIAL_ON),
            pane_opaque: pick(l::PANE_OPAQUE, d::PANE_OPAQUE),
            well_fill: pick(l::WELL_FILL, d::WELL_FILL),
            field_inner: pick(l::MAP_FIELD_INNER, d::MAP_FIELD_INNER),
            field_outer: pick(l::MAP_FIELD_OUTER, d::MAP_FIELD_OUTER),
            text_primary: pick(l::TEXT_PRIMARY, d::TEXT_PRIMARY),
            text_secondary: pick(l::TEXT_SECONDARY, d::TEXT_SECONDARY),
            text_tertiary: pick(l::TEXT_TERTIARY, d::TEXT_TERTIARY),
            sublabel: pick(l::STATUS_OFF, d::STATUS_OFF),
            accent_text: pick(l::PURPLE_TEXT, d::PURPLE_TEXT),
            tools: [
                pick(l::TOOL_CLAUDE, d::TOOL_CLAUDE),
                pick(l::TOOL_CODEX, d::TOOL_CODEX),
                pick(l::TOOL_ANTIGRAVITY, d::TOOL_ANTIGRAVITY),
                pick(l::TOOL_OPEN_CODE, d::TOOL_OPEN_CODE),
                pick(l::TOOL_THEIA, d::TOOL_THEIA),
            ],
            hero_deep: pick(l::HERO_CAMPAIGN_DEEP, d::HERO_CAMPAIGN_DEEP),
            hero_bright: pick(l::HERO_CAMPAIGN_BRIGHT, d::HERO_CAMPAIGN_BRIGHT),
        }
    }

    /// A tool mark's colour (`GlassTool.tint`).
    pub fn tint(&self, tint: Tint) -> gtk::gdk::RGBA {
        match tint {
            Tint::Claude => self.tools[0],
            Tint::Codex => self.tools[1],
            Tint::Antigravity => self.tools[2],
            Tint::OpenCode => self.tools[3],
            Tint::Theia => self.tools[4],
            Tint::Primary => self.text_primary,
        }
    }

    /// The colours the map draws in for `inks`: the Mac's for the scheme
    /// in the standard look; in the Omarchy skin, its palette's roles. In
    /// both, every word colour is then made opaque and raised to the 4.5:1
    /// text floor on the map's grounds (`raise_words`), the rule the rest of
    /// the GTK palette follows (`style::LIGHT_TOKENS`' tertiary twin): a
    /// colour that already clears is kept exactly.
    ///
    /// Kept for the last look asked for: the map asks on every frame.
    pub fn resolve(dark: bool, inks: &crate::ui::style::Inks) -> Self {
        type Kept = Option<((bool, Option<omarchy::Palette>), MapColours)>;
        thread_local! {
            static KEPT: RefCell<Kept> = const { RefCell::new(None) };
        }
        let key = (dark, inks.skin.clone());
        KEPT.with(|kept| {
            if let Some((k, colours)) = kept.borrow().as_ref()
                && *k == key
            {
                return colours.clone();
            }
            let colours = match &inks.skin {
                Some(p) => Self::skinned(p),
                None => Self::standard(dark),
            };
            kept.replace(Some((key, colours.clone())));
            colours
        })
    }

    /// The Mac's colours, words raised to the floor on the map pane and
    /// the window's scene, bare and under the field's veil.
    fn standard(dark: bool) -> Self {
        use crate::ui::style::brand_tokens::{GLASS_DARK_CSS, GLASS_LIGHT_CSS};
        let mut colours = Self::scheme(dark);
        let css = if dark {
            GLASS_DARK_CSS
        } else {
            GLASS_LIGHT_CSS
        };
        let pane = rgb(&colours.pane_opaque);
        let scene = css_colour(css, "tc_glass_scene").unwrap_or(pane);
        let toward = if dark {
            omarchy::Rgb::WHITE
        } else {
            omarchy::Rgb::BLACK
        };
        colours.raise_words(&[pane, scene], toward);
        colours
    }

    /// The Omarchy palette's roles in the Mac's places. The tools keep
    /// their own tints (they are the tools' marks, not the theme's), in
    /// the skin's scheme.
    fn skinned(p: &omarchy::Palette) -> Self {
        let dark = p.mode == omarchy::Mode::Dark;
        let mac = Self::scheme(dark);
        let c = |rgb: omarchy::Rgb| {
            let (r, g, b) = rgb.unit();
            gtk::gdk::RGBA::new(r as f32, g as f32, b as f32, 1.0)
        };
        let veil = |alpha: f32| c(p.scene).with_alpha(alpha);
        let mut colours = Self {
            orb_shared: c(p.accent),
            orb_kept: c(p.status_ask),
            purple_soft: c(p.accent),
            blue: c(p.held),
            status_off: c(p.tertiary),
            status_on: c(p.status_on),
            status_on_text: c(p.status_on_text),
            status_ask: c(p.status_ask),
            status_outside: c(p.status_outside),
            node_off: c(p.line),
            rule_fills: [
                c(p.bg.mix(p.status_on, 0.2)),
                c(p.bg.mix(p.status_ask, 0.2)),
                c(p.bg.mix(p.tertiary, 0.15)),
                c(p.inset),
            ],
            credential_on: c(p.status_on),
            pane_opaque: c(p.bg),
            well_fill: c(p.inset),
            field_inner: veil(mac.field_inner.alpha()),
            field_outer: veil(mac.field_outer.alpha()),
            text_primary: c(p.ink),
            text_secondary: c(p.muted),
            text_tertiary: c(p.tertiary),
            sublabel: c(p.tertiary),
            accent_text: c(p.accent_text),
            tools: mac.tools,
            hero_deep: c(p.accent),
            hero_bright: c(p.accent_line),
        };
        let toward = if dark {
            omarchy::Rgb::WHITE
        } else {
            omarchy::Rgb::BLACK
        };
        colours.raise_words(&p.grounds(), toward);
        colours
    }

    /// Every colour a word is drawn in (`word_colour`) made opaque over
    /// the first ground and moved toward `toward`, a step at a time, until
    /// it clears 4.5:1 on each of `grounds`, bare and under the field's
    /// veil at its edge and its centre. One that already clears is kept.
    fn raise_words(&mut self, grounds: &[omarchy::Rgb], toward: omarchy::Rgb) {
        let mut all = grounds.to_vec();
        for &g in grounds {
            let edge = over(&self.field_outer, g);
            all.push(edge);
            all.push(over(&self.field_inner, edge));
        }
        let first = grounds.first().copied().unwrap_or(toward);
        let raise = |colour: &mut gtk::gdk::RGBA| {
            let clears = |c: omarchy::Rgb| {
                all.iter()
                    .all(|g| omarchy::contrast(c, *g) >= omarchy::TEXT)
            };
            let opaque = over(colour, first);
            if colour.alpha() >= 1.0 && clears(opaque) {
                return;
            }
            let raised = (0..=255)
                .map(|step| opaque.mix(toward, f64::from(step) / 255.0))
                .find(|c| clears(*c))
                .unwrap_or(toward);
            let (r, g, b) = raised.unit();
            *colour = gtk::gdk::RGBA::new(r as f32, g as f32, b as f32, 1.0);
        };
        raise(&mut self.text_primary);
        raise(&mut self.text_tertiary);
        raise(&mut self.sublabel);
        raise(&mut self.status_on_text);
    }
}

/// `colour` composited onto the opaque `ground`.
fn over(colour: &gtk::gdk::RGBA, ground: omarchy::Rgb) -> omarchy::Rgb {
    ground.mix(rgb(colour), f64::from(colour.alpha()))
}

/// An RGBA's channels, its alpha dropped.
fn rgb(colour: &gtk::gdk::RGBA) -> omarchy::Rgb {
    let ch = |v: f32| (f64::from(v) * 255.0).round().clamp(0.0, 255.0) as u8;
    omarchy::Rgb(ch(colour.red()), ch(colour.green()), ch(colour.blue()))
}

/// A literal `@define-color` in a generated sheet.
fn css_colour(css: &str, name: &str) -> Option<omarchy::Rgb> {
    let prefix = format!("@define-color {name} ");
    css.lines()
        .find_map(|line| line.trim_start().strip_prefix(&prefix))
        .and_then(|rest| omarchy::Rgb::parse(rest.trim().trim_end_matches(';')))
}

/// Every kind of word the map draws. Each is set in one colour,
/// `word_colour`, and the map's text calls take a `Word`, never a colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Word {
    /// A node's name.
    Name,
    /// A node's sublabel.
    Sublabel,
    /// The line under the pad while a contribution's traces gather.
    PadLine,
    /// The library's "+N" as the rocket docks.
    Dock,
    /// A "+N" for the nodes that did not fit.
    Overflow,
    /// The pile's heading.
    PileHeading,
    /// A coin's name in the open pile.
    CoinName,
    /// What waits from a coin, under its name.
    CoinLine,
    /// The two letters of a tool with no artwork.
    Initials,
}

impl Word {
    pub const ALL: [Word; 9] = [
        Word::Name,
        Word::Sublabel,
        Word::PadLine,
        Word::Dock,
        Word::Overflow,
        Word::PileHeading,
        Word::CoinName,
        Word::CoinLine,
        Word::Initials,
    ];
}

/// The colour `word` is drawn in, before any fade (`FlowMapView.drawNode`,
/// `drawStill`, `MapToolPile`): names in the primary text colour,
/// sublabels in statusOff's text twin, the quiet lines in the tertiary
/// text colour, the dock's "+N" in statusOnText.
pub fn word_colour(palette: &Palette, word: Word) -> gtk::gdk::RGBA {
    let m = &palette.map;
    match word {
        Word::Name | Word::CoinName | Word::Initials => m.text_primary,
        Word::Sublabel => m.sublabel,
        Word::PadLine | Word::Overflow | Word::PileHeading | Word::CoinLine => m.text_tertiary,
        Word::Dock => m.status_on_text,
    }
}

/// `word`'s colour faded by `alpha`.
pub fn faded_word(palette: &Palette, word: Word, alpha: f64) -> gtk::gdk::RGBA {
    let c = word_colour(palette, word);
    c.with_alpha(c.alpha() * alpha as f32)
}

/// Whether the library turns on this map: only the Traces map's station
/// does (`FlowMapView.libraryOnMap`).
pub fn library_turns(scene: &Scene) -> bool {
    library_on_map(scene).is_some()
}

/// The library node the Traces map draws as the station; none on the
/// other maps, which draw it as a disc.
fn library_on_map(scene: &Scene) -> Option<&map_scene::Node> {
    scene.stack.as_ref()?;
    scene.nodes.iter().find(|node| node.id == "library")
}

/// The library's frame on the Traces map: its top-left, width and height,
/// centred on the library's point at the map's scale, and whether anything
/// has been contributed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LibraryFrame {
    pub origin: (f64, f64),
    pub width: f64,
    pub height: f64,
    pub lit: bool,
}

/// The library's frame for `style`: the station's on the rocket map, the
/// orb's on the standard one (`FlowMapView.libraryArt`). `None` on the
/// other maps.
pub fn library_frame(scene: &Scene, fit: &MapGeometry, style: MapStyle) -> Option<LibraryFrame> {
    let library = library_on_map(scene)?;
    let NodeKind::Library { active } = library.kind else {
        return None;
    };
    let centre = fit.point(library.at);
    let size = map_scene::library_size(style);
    let (width, height) = (size.width * fit.scale, size.height * fit.scale);
    Some(LibraryFrame {
        origin: (centre.x - width / 2.0, centre.y - height / 2.0),
        width,
        height,
        lit: active,
    })
}

/// The map's seconds held to the resting timeline's last tick, so the
/// library moves at the resting rate even while a run draws at 60.
pub fn library_seconds(now: f64, interval: f64) -> f64 {
    if interval > 0.0 {
        (now / interval).floor() * interval
    } else {
        now
    }
}

/// What the library was last drawn from.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LibraryKey {
    style: MapStyle,
    frame: LibraryFrame,
    angle: f64,
}

/// The library's drawing, kept until its angle, light, frame or style
/// changes: built at most once a resting frame.
#[derive(Default)]
pub struct LibraryArt {
    kept: RefCell<Option<(LibraryKey, gtk::gsk::RenderNode)>>,
    builds: Cell<usize>,
}

impl LibraryArt {
    /// The library at `seconds` on the map's clock (`library_seconds`),
    /// at rest under reduced motion.
    pub fn node(
        &self,
        style: MapStyle,
        frame: &LibraryFrame,
        seconds: f64,
        reduce_motion: bool,
    ) -> gtk::gsk::RenderNode {
        let angle = match style {
            MapStyle::Launch => station::angle(seconds, reduce_motion),
            MapStyle::Standard => orb::angle(seconds, reduce_motion),
        };
        let key = LibraryKey {
            style,
            frame: *frame,
            angle,
        };
        if let Some((kept, node)) = self.kept.borrow().as_ref()
            && *kept == key
        {
            return node.clone();
        }
        let LibraryFrame {
            origin,
            width,
            height,
            lit,
        } = *frame;
        let node = match style {
            MapStyle::Launch => station::node(origin, width, height, angle, lit),
            MapStyle::Standard => orb::node(origin, width, height, angle, lit),
        };
        self.builds.set(self.builds.get() + 1);
        self.kept.replace(Some((key, node.clone())));
        node
    }

    /// How many times the drawing has been built.
    pub fn builds(&self) -> usize {
        self.builds.get()
    }
}

impl Palette {
    /// A palette that needs no widget, for tests of what is drawn in
    /// which colour.
    #[cfg(test)]
    pub fn fixed() -> Self {
        Self::with_inks(
            gtk::gdk::RGBA::BLACK,
            gtk::gdk::RGBA::WHITE,
            &crate::ui::style::Inks::standard(false),
            false,
        )
    }

    #[allow(deprecated)]
    pub fn current(widget: &impl IsA<gtk::Widget>) -> Self {
        let dark_scheme = adw::StyleManager::default().is_dark();
        let context = widget.as_ref().style_context();
        let fallback = if dark_scheme {
            gtk::gdk::RGBA::BLACK
        } else {
            gtk::gdk::RGBA::WHITE
        };
        Self::with_inks(
            context.color(),
            context.lookup_color("window_bg_color").unwrap_or(fallback),
            &crate::ui::style::inks(),
            dark_scheme,
        )
    }

    /// The theme's ink and ground, and the map's colours resolved from
    /// `inks` (`MapColours::resolve`): the Mac's in the standard look, the
    /// Omarchy palette's in the skin. `dark` is the scheme.
    pub fn with_inks(
        ink: gtk::gdk::RGBA,
        background: gtk::gdk::RGBA,
        inks: &crate::ui::style::Inks,
        dark: bool,
    ) -> Self {
        Self {
            ink,
            background,
            map: MapColours::resolve(dark, inks),
        }
    }

    pub fn ink(&self, alpha: f64) -> gtk::gdk::RGBA {
        self.ink.with_alpha(self.ink.alpha() * alpha as f32)
    }

    /// A folder's dot by rule: its fill and its ring (`FlowMapView.rule`).
    fn rule(&self, mode: Option<ProjectMode>) -> (gtk::gdk::RGBA, gtk::gdk::RGBA) {
        let m = &self.map;
        match mode {
            Some(ProjectMode::AutoUpload) => (m.rule_fills[0], m.status_on),
            Some(ProjectMode::Ask) => (m.rule_fills[1], m.status_ask),
            Some(ProjectMode::Ignore) => (m.rule_fills[2], m.status_outside),
            None => (m.rule_fills[3], m.status_off),
        }
    }
}

/// The colours a node's name and sublabel are drawn in
/// (`FlowMapView.drawNode`): the primary text colour and statusOff, both
/// faded with the node.
pub fn node_word_colours(
    node: &map_scene::Node,
    palette: &Palette,
) -> (gtk::gdk::RGBA, gtk::gdk::RGBA) {
    (
        faded_word(palette, Word::Name, node.dim),
        faded_word(palette, Word::Sublabel, node.dim),
    )
}

/// A "+N"'s colour: the tertiary text colour (`drawStill`).
pub fn overflow_colour(palette: &Palette) -> gtk::gdk::RGBA {
    word_colour(palette, Word::Overflow)
}

pub fn source(cr: &gtk::cairo::Context, colour: &gtk::gdk::RGBA, alpha: f64) {
    cr.set_source_rgba(
        f64::from(colour.red()),
        f64::from(colour.green()),
        f64::from(colour.blue()),
        f64::from(colour.alpha()) * alpha,
    );
}

pub fn circle(cr: &gtk::cairo::Context, centre: Point, radius: f64) {
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
/// `phase` is how far a flowing arc's dashes have moved, in design units:
/// 0 under reduced motion, where they stand still.
fn draw_arcs(
    cr: &gtk::cairo::Context,
    scene: &Scene,
    fit: &MapGeometry,
    palette: &Palette,
    healthy: bool,
    phase: f64,
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
            ArcStyle::Shared => source(cr, &palette.map.status_on, 0.35 * arc.dim),
            ArcStyle::Unknown => {
                cr.set_line_cap(gtk::cairo::LineCap::Round);
                cr.set_dash(&[0.5 * s, 4.0 * s], 0.0);
                // The core could not say: dotted, in white as on the Mac.
                source(cr, &gtk::gdk::RGBA::WHITE, 0.18 * arc.dim);
            }
            ArcStyle::Flowing if !healthy => {
                // Paused, unknown or stale: still and dashed, not live.
                cr.set_dash(&[5.0 * s, 5.0 * s], 0.0);
                source(cr, &palette.ink, 0.25 * arc.dim);
            }
            ArcStyle::Flowing => {
                source(cr, &palette.map.status_on, 0.35 * arc.dim);
                let _ = cr.stroke_preserve();
                // The dashes, moving towards the pad on the map's clock.
                cr.set_line_width(1.4 * s * 1.3);
                cr.set_line_cap(gtk::cairo::LineCap::Round);
                cr.set_dash(&[4.0 * s, 8.0 * s], -phase * s);
                source(cr, &palette.map.status_on, arc.dim);
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

/// The slash across a tool that is off: from a quarter of its frame to
/// three quarters, in the primary text colour.
fn slash(
    cr: &gtk::cairo::Context,
    centre: Point,
    half: f64,
    s: f64,
    palette: &Palette,
    alpha: f64,
) {
    cr.move_to(centre.x - half, centre.y - half);
    cr.line_to(centre.x + half, centre.y + half);
    cr.set_line_width(2.0 * s);
    source(cr, &palette.map.text_primary, alpha);
    let _ = cr.stroke();
}

/// The face and size a tool's initials are set in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkFont {
    pub family: &'static str,
    pub size: f64,
}

impl MarkFont {
    /// On the map's nodes, the micro type: the system face, 10 bold, not
    /// scaled with the map (`FlowMapView.mark`, `TypeScale.micro`).
    pub const MICRO: Self = Self {
        family: "sans-serif",
        size: 10.0,
    };

    /// On a coin, bold and monospaced, 12 at the coin's scale `k`
    /// (`MapToolPile.coin`).
    pub fn coin(k: f64) -> Self {
        Self {
            family: "monospace",
            size: 12.0 * k,
        }
    }
}

/// A tool's mark, `side` across, centred on `centre`: its logo in its
/// tint, or its initials in its tint, in `font` (`FlowMapView.mark`,
/// `MapToolPile.coin`).
#[allow(clippy::too_many_arguments)]
fn draw_mark(
    cr: &gtk::cairo::Context,
    kind: SourceKind,
    centre: Point,
    side: f64,
    font: MarkFont,
    palette: &Palette,
    alpha: f64,
) {
    let art = tool_art::tool_art(kind);
    source(cr, &palette.map.tint(art.tint), alpha);
    match art.logo {
        Some(logo) => logo.fill(cr, (centre.x - side / 2.0, centre.y - side / 2.0), side),
        None => centred_text(cr, art.initials, centre, font.size, font.family),
    }
}

/// A short bold mark centred on `centre`, in the current source.
pub(super) fn centred_text(
    cr: &gtk::cairo::Context,
    text: &str,
    centre: Point,
    size: f64,
    family: &str,
) {
    cr.select_font_face(
        family,
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Bold,
    );
    cr.set_font_size(size);
    if let Ok(extents) = cr.text_extents(text) {
        cr.move_to(
            centre.x - extents.x_bearing() - extents.width() / 2.0,
            centre.y - extents.y_bearing() - extents.height() / 2.0,
        );
        let _ = cr.show_text(text);
    }
    cr.new_path();
}

fn draw_node(
    cr: &gtk::cairo::Context,
    scene: &Scene,
    node: &map_scene::Node,
    fit: &MapGeometry,
    palette: &Palette,
    selected: bool,
    style: MapStyle,
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
        NodeKind::Hub if traces => match style {
            MapStyle::Launch => draw_pad(cr, centre, s, palette, dim),
            MapStyle::Standard => draw_laptop(cr, centre, s, palette, dim),
        },
        NodeKind::Hub => {
            circle(cr, centre, radius);
            source(cr, &palette.map.status_off, dim);
            let _ = cr.fill();
            ring(cr, centre, radius, s, &palette.ink, 0.25 * dim);
        }
        // The station is drawn over the still layer, on the map's clock
        // (`MapCanvas::snapshot`); it has no name, even under the pointer
        // (owner, 2026-10-19): its card says it.
        NodeKind::Library { .. } if traces => return,
        NodeKind::Library { active } => {
            circle(cr, centre, radius);
            if active {
                source(cr, &palette.map.blue, dim);
            } else {
                source(cr, &palette.map.node_off, dim);
            }
            let _ = cr.fill();
            if active {
                ring(cr, centre, radius, s, &palette.map.blue, 0.35 * dim);
            }
        }
        NodeKind::Tool { kind, off } => {
            circle(cr, centre, radius);
            source(cr, &palette.ink, 0.12 * dim);
            let _ = cr.fill();
            draw_mark(cr, kind, centre, 16.0 * s, MarkFont::MICRO, palette, dim);
            if node.dashed {
                // The legend's "Not watched": a dashed rim.
                circle(cr, centre, radius);
                cr.set_line_width(1.5 * s);
                cr.set_dash(&[2.0 * s, 2.0 * s], 0.0);
                source(cr, &palette.map.status_off, dim);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
            }
            if node.ringed {
                ring(cr, centre, radius, s, &palette.ink, 0.35 * dim);
            }
            if off {
                slash(cr, centre, 9.0 * s, s, palette, dim);
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
        NodeKind::Harness { tool, .. } => {
            circle(cr, centre, radius);
            source(cr, &palette.ink, 0.12 * dim);
            let _ = cr.fill();
            match (tool, map_scene::harness_initials(node)) {
                (Some(kind), _) => {
                    draw_mark(cr, kind, centre, 16.0 * s, MarkFont::MICRO, palette, dim)
                }
                // A tool with no artwork carries two letters of its name in
                // the primary ink (`GlassTool.other`), at the map's micro
                // type.
                (None, Some(letters)) => {
                    source(cr, &word_colour(palette, Word::Initials), dim);
                    let font = MarkFont::MICRO;
                    centred_text(cr, &letters, centre, font.size, font.family);
                }
                (None, None) => {}
            }
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
        source(cr, &palette.map.accent_text, dim);
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
        source(cr, &palette.map.credential_on, dim);
    } else {
        source(cr, &palette.map.node_off, dim);
    }
    let _ = cr.fill();
    if lamp == Lamp::Answering {
        ring(cr, centre, radius, s, &palette.map.status_on, 0.35 * dim);
    }
    if lamp == Lamp::Unknown {
        // Unknown is not off: a dotted rim, in white as on the Mac.
        circle(cr, centre, radius);
        cr.set_line_width(1.5 * s);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.set_dash(&[0.5 * s, 4.0 * s], 0.0);
        source(cr, &gtk::gdk::RGBA::WHITE, 0.5 * dim);
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
    source(cr, &palette.map.text_primary, dim);
    let _ = cr.stroke();
}

/// This computer as a launch pad: a slab on a base, with a service tower
/// beside it; the line out lands on its left edge (`FlowMapView.drawPad`).
fn draw_pad(cr: &gtk::cairo::Context, centre: Point, s: f64, palette: &Palette, dim: f64) {
    let half = map_scene::PAD_HALF_WIDTH * s;
    let steel = palette.map.status_off;
    rounded(
        cr,
        centre.x - half,
        centre.y - 5.0 * s,
        half * 2.0,
        10.0 * s,
        4.0 * s,
    );
    let slab =
        gtk::cairo::LinearGradient::new(centre.x, centre.y - 5.0 * s, centre.x, centre.y + 5.0 * s);
    let stop = |at: f64, alpha: f64| {
        slab.add_color_stop_rgba(
            at,
            f64::from(steel.red()),
            f64::from(steel.green()),
            f64::from(steel.blue()),
            f64::from(steel.alpha()) * alpha * dim,
        );
    };
    stop(0.0, 0.75);
    stop(1.0, 1.0);
    let _ = cr.set_source(&slab);
    let _ = cr.fill();
    rounded(
        cr,
        centre.x - 24.0 * s,
        centre.y + 5.0 * s,
        48.0 * s,
        6.0 * s,
        3.0 * s,
    );
    source(cr, &palette.map.node_off, dim);
    let _ = cr.fill();
    for (x, y, w, h, r) in [
        (28.0, -55.0, 4.0, 50.0, 2.0),
        (14.0, -43.0, 16.0, 3.0, 1.5),
        (18.0, -27.0, 12.0, 3.0, 1.5),
    ] {
        rounded(cr, centre.x + x * s, centre.y + y * s, w * s, h * s, r * s);
        source(cr, &steel, 0.7 * dim);
        let _ = cr.fill();
    }
}

/// A laptop (`FlowMapView.drawLaptop`): its deck as wide as the pad, a
/// notch, and its screen open above it in the campaign's blues.
fn draw_laptop(cr: &gtk::cairo::Context, centre: Point, s: f64, palette: &Palette, dim: f64) {
    let half = map_scene::PAD_HALF_WIDTH * s;
    let steel = palette.map.status_off;
    rounded(
        cr,
        centre.x - half,
        centre.y - 3.0 * s,
        half * 2.0,
        6.0 * s,
        3.0 * s,
    );
    source(cr, &steel, dim);
    let _ = cr.fill();
    rounded(
        cr,
        centre.x - 7.0 * s,
        centre.y - 3.0 * s,
        14.0 * s,
        2.0 * s,
        1.0 * s,
    );
    source(cr, &palette.map.node_off, dim);
    let _ = cr.fill();
    let top = centre.y - LAPTOP_SCREEN_TOP * s;
    let (lid_x, lid_w, lid_h) = (centre.x - 25.0 * s, 50.0 * s, (LAPTOP_SCREEN_TOP - 4.0) * s);
    rounded(cr, lid_x, top, lid_w, lid_h, 3.5 * s);
    source(cr, &steel, 0.85 * dim);
    let _ = cr.fill();
    let inset = 2.5 * s;
    let (x, y, w, h) = (
        lid_x + inset,
        top + inset,
        lid_w - 2.0 * inset,
        lid_h - 2.0 * inset,
    );
    rounded(cr, x, y, w, h, 2.0 * s);
    let display = gtk::cairo::LinearGradient::new(x, y, x + w, y + h);
    for (at, colour, alpha) in [
        (0.0, palette.map.hero_bright, 0.9),
        (1.0, palette.map.hero_deep, 1.0),
    ] {
        display.add_color_stop_rgba(
            at,
            f64::from(colour.red()),
            f64::from(colour.green()),
            f64::from(colour.blue()),
            f64::from(colour.alpha()) * alpha * dim,
        );
    }
    let _ = cr.set_source(&display);
    let _ = cr.fill();
}

/// How far above this computer's point the laptop's screen rises.
pub const LAPTOP_SCREEN_TOP: f64 = 36.0;

/// The tool group: a card, and the coins on it, the front coin first.
/// The pile: its card, then its coins in `order`, front first. A coin a
/// trace is in (`grown`) is drawn larger, over the rest, in the run's
/// `glow`.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn draw_pile(
    cr: &gtk::cairo::Context,
    scene: &Scene,
    stack: &ToolStack,
    order: &[String],
    grown: &[String],
    glow: Option<&gtk::gdk::RGBA>,
    fit: &MapGeometry,
    palette: &Palette,
    stage: PileStage,
) {
    let frame = PileFrame::at_rest(order, grown, stage);
    draw_pile_frame(cr, scene, stack, &frame, glow, fit, palette);
}

/// The pile as `frame` has it: its card, then its coins, the back first.
fn draw_pile_frame(
    cr: &gtk::cairo::Context,
    scene: &Scene,
    stack: &ToolStack,
    frame: &PileFrame,
    glow: Option<&gtk::gdk::RGBA>,
    fit: &MapGeometry,
    palette: &Palette,
) {
    let s = fit.scale;
    let (x, y, w, h) = stack.card_at(fit, frame.tall, frame.wide);
    rounded(cr, x, y, w, h, 26.0 * s);
    // An open card is opaque: nothing under it shows through. Its fill
    // turns from the well's to the pane's as it widens.
    let (well, pane) = (palette.map.well_fill, palette.map.pane_opaque);
    let m = frame.opaque as f32;
    let mix = |a: f32, b: f32| a + (b - a) * m;
    let card = gtk::gdk::RGBA::new(
        mix(well.red(), pane.red()),
        mix(well.green(), pane.green()),
        mix(well.blue(), pane.blue()),
        mix(well.alpha(), pane.alpha()),
    );
    source(cr, &card, 1.0);
    let _ = cr.fill();
    // The back of the pile first, so the front coin is drawn over the rest,
    // and a grown coin over them all.
    let mut drawn: Vec<&CoinFrame> = frame.coins.iter().rev().collect();
    drawn.sort_by_key(|coin| coin.grown);
    for coin in drawn {
        let Some(node) = scene.nodes.iter().find(|n| n.id == coin.id) else {
            continue;
        };
        let NodeKind::Tool { kind, off } = node.kind else {
            continue;
        };
        let index = coin.slot;
        let open = frame.open;
        let centre = fit.point(stack.coin_center_at(coin.at, frame.lay));
        // A grown coin is the whole coin larger (`scaleEffect`): its rim,
        // its mark and its lock too, on its spring.
        let k = s * (1.0 + (map_motion::GROWN - 1.0) * coin.grow);
        let side = ToolStack::COIN * k;
        let radius = side / 2.0;
        let tint = palette.map.tint(tool_art::tool_art(kind).tint);
        let glowing = coin.grow.clamp(0.0, 1.0);
        if glowing > 0.0
            && let Some(glow) = glow
        {
            // The coin's glow (`shadow(radius: 9)`), as a soft halo.
            let halo = gtk::cairo::RadialGradient::new(
                centre.x,
                centre.y,
                radius * 0.8,
                centre.x,
                centre.y,
                radius + 9.0 * s,
            );
            let (r, g, b) = (
                f64::from(glow.red()),
                f64::from(glow.green()),
                f64::from(glow.blue()),
            );
            halo.add_color_stop_rgba(0.0, r, g, b, 0.45 * glowing);
            halo.add_color_stop_rgba(1.0, r, g, b, 0.0);
            circle(cr, centre, radius + 9.0 * s);
            let _ = cr.set_source(&halo);
            let _ = cr.fill();
        }
        // The coin is faded as a whole, as the Mac fades its view.
        cr.push_group();
        // The coin's edge below it, then an opaque face and its rim.
        circle(cr, Point::new(centre.x, centre.y + 3.0 * k), radius);
        source(cr, &tint, 0.45);
        let _ = cr.fill();
        circle(cr, centre, radius);
        source(cr, &palette.map.pane_opaque, 1.0);
        let _ = cr.fill_preserve();
        source(cr, &palette.ink, 0.12);
        let _ = cr.fill();
        circle(cr, centre, radius - k);
        cr.set_line_width(2.0 * k);
        if node.dashed {
            cr.set_dash(&[3.0 * k, 3.0 * k], 0.0);
        }
        source(cr, &tint, 1.0);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        draw_mark(cr, kind, centre, 20.0 * k, MarkFont::coin(k), palette, 1.0);
        if off {
            slash(cr, centre, side / 4.0, k, palette, 1.0);
        }
        if coin_has_lock(node.dashed, open, index) {
            // The tool's memory stays on this computer: a pip at the coin's
            // right edge, on the pane's ground.
            let pip = Point::new(centre.x + radius + k, centre.y);
            circle(cr, pip, 7.5 * k);
            source(cr, &palette.map.pane_opaque, 1.0);
            let _ = cr.fill();
            draw_lock(cr, pip, k, &palette.map.text_secondary, 1.0);
        }
        let _ = cr.pop_group_to_source();
        let _ = cr.paint_with_alpha(if node.dashed { 0.55 } else { node.dim });
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

/// The coins' lock as a small widget, for the legend's memory item and the
/// transfer card's memory well (the Mac's `Image(systemName: "lock")`).
pub(super) fn lock_icon() -> gtk::DrawingArea {
    let lock = gtk::DrawingArea::new();
    lock.set_content_width(10);
    lock.set_content_height(10);
    lock.set_valign(gtk::Align::Center);
    lock.set_draw_func(|area, cr, w, h| {
        let palette = Palette::current(area);
        let centre = Point::new(f64::from(w) / 2.0, f64::from(h) / 2.0);
        draw_lock(cr, centre, 1.0, &palette.ink, 0.6);
    });
    lock
}

/// The legend's memory item: the coins' lock, and its word.
fn memory_legend_item() -> gtk::Box {
    let item = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let lock = lock_icon();
    let label = gtk::Label::new(Some(memory_legend_word()));
    label.add_css_class("tc-map-legend");
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
    /// `harness_list`'s rows for the Private AI map; `None` before the
    /// first answer and after a failed read.
    harnesses: Option<Vec<HarnessRow>>,
}

/// The window's transfer player, its feed, and the cards at the map's
/// foot (`MapTransferPlayer` and `MapTransferFeed` on the Mac). The player
/// is the window's, so it keeps time whether or not the map is drawn: a
/// timer wakes it at its next deadline, a run's end or a card's expiry.
pub struct Transfers {
    player: RefCell<Player>,
    feed: RefCell<Feed>,
    timer: RefCell<Option<glib::SourceId>>,
    canvas: MapCanvas,
    /// The cards' column, above the legend and the zoom.
    pub cards: gtk::Box,
    /// Whether a tab other than Traces is shown: only then are the cards
    /// at the map's foot (Traces shows its traces itself).
    shown: Cell<bool>,
    /// The ids of the cards drawn now, front first.
    drawn: RefCell<Vec<String>>,
    /// The front card's id and its holder, kept while it stays in front.
    front: RefCell<Option<(String, crate::ui::popover_stack::Slide)>>,
    /// The plates peeking out above the front card: one for the stack's
    /// life, so a change in the count moves them on the stack's spring.
    plates: crate::ui::popover_stack::Plates,
    /// Where the front card sits, under the plates.
    place: gtk::Box,
}

impl Transfers {
    fn new(canvas: &MapCanvas) -> Rc<Self> {
        let cards = gtk::Box::new(gtk::Orientation::Vertical, 6);
        cards.set_visible(false);
        // The plates sit flush on the card, whatever the column's gap.
        let plates = crate::ui::popover_stack::Plates::hung_from_foot("tc-map-transfer-plate");
        let place = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let stack = gtk::Box::new(gtk::Orientation::Vertical, 0);
        stack.append(&plates);
        stack.append(&place);
        cards.append(&stack);
        Rc::new(Self {
            player: RefCell::new(Player::default()),
            feed: RefCell::new(Feed::default()),
            timer: RefCell::new(None),
            canvas: canvas.clone(),
            cards,
            shown: Cell::new(false),
            drawn: RefCell::new(Vec::new()),
            front: RefCell::new(None),
            plates,
            place,
        })
    }

    /// The player's clock: monotonic seconds, the frame clock's own base.
    fn now() -> f64 {
        glib::monotonic_time() as f64 / 1e6
    }

    /// Hands the player what the reads reported, then shows where it is.
    fn observe(
        self: &Rc<Self>,
        entries: &[QueueEntry],
        history: Option<&[HistoryRecord]>,
        last_kept: Option<&str>,
        last_dismissed: Option<&str>,
    ) {
        let now = Self::now();
        self.feed.borrow_mut().observe(
            &mut self.player.borrow_mut(),
            entries,
            history,
            last_kept,
            last_dismissed,
            now,
        );
        self.sync();
    }

    /// Advances the player to now, hands the map its run, draws the cards,
    /// and wakes again at the next deadline.
    fn sync(self: &Rc<Self>) {
        let now = Self::now();
        self.player.borrow_mut().tick(now);
        self.canvas.set_run(self.player.borrow().run().cloned());
        self.draw_cards();
        if let Some(timer) = self.timer.take() {
            timer.remove();
        }
        let Some(at) = self.player.borrow().next_deadline() else {
            return;
        };
        let delay = std::time::Duration::from_secs_f64((at - now).max(0.0) + 0.001);
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_local_once(delay, move || {
            if let Some(transfers) = weak.upgrade() {
                // This timer has fired: forget it rather than remove it.
                transfers.timer.take();
                transfers.sync();
            }
        });
        self.timer.replace(Some(timer));
    }

    fn set_shown(self: &Rc<Self>, shown: bool) {
        if self.shown.replace(shown) != shown {
            self.draw_cards();
        }
    }

    /// The cards drawn, as the Mac's popover stack hung from the map's foot
    /// (`GlassPopoverStack`, anchor bottom): the newest whole, up to two
    /// behind it peeking out above it; none while Traces is shown. A new
    /// card rises into place, one revealed by closing the card in front of
    /// it fades (`crate::ui::popover_stack`).
    fn draw_cards(self: &Rc<Self>) {
        let visible: Vec<MapTransfer> = if self.shown.get() {
            self.player
                .borrow()
                .visible()
                .into_iter()
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let ids: Vec<String> = visible.iter().map(|card| card.id.clone()).collect();
        if *self.drawn.borrow() == ids {
            return;
        }
        // The cards drawn before: the front one now was among them when a
        // close revealed it, and was not when it has just arrived.
        let before = self.drawn.replace(ids);
        // The card in front until now: kept as it is while it stays in
        // front, fading where it is beneath the new one once covered.
        let old = self.front.take();
        let kept: Option<gtk::Widget> = old
            .as_ref()
            .filter(|(id, _)| visible.first().is_some_and(|card| card.id == *id))
            .map(|(_, slide)| slide.clone().upcast());
        let mut child = self.place.first_child();
        while let Some(card) = child {
            child = card.next_sibling();
            if Some(&card) != kept.as_ref() {
                self.place.remove(&card);
            }
        }
        let scene = self.canvas.scene();
        if let Some(transfer) = visible.first() {
            let slide = match old {
                Some((id, slide)) if id == transfer.id => slide,
                old => {
                    let from_map = scene
                        .nodes
                        .iter()
                        .find(|node| node.id == format!("folder:{}", transfer.folder))
                        .map(|node| node.label.clone());
                    let weak = Rc::downgrade(self);
                    let id = transfer.id.clone();
                    let card = transfer_card::card(transfer, from_map.as_deref(), move || {
                        if let Some(transfers) = weak.upgrade() {
                            transfers.player.borrow_mut().close(&id);
                            transfers.sync();
                        }
                    });
                    let slide = crate::ui::popover_stack::Slide::new(&card);
                    // In place before it moves, so a window that is hidden
                    // lands it at once, as the Mac's does.
                    slide.set_valign(gtk::Align::End);
                    self.place.append(&slide);
                    if crate::ui::glass_motion::animations_enabled() {
                        // The card it covers fades where it is beneath it,
                        // on the stack's spring, drawn or not yet; it takes
                        // no pointer or focus.
                        slide.arrive_replacing(
                            !before.contains(&transfer.id),
                            true,
                            old.as_ref().map(|(_, slide)| slide),
                        );
                    }
                    slide
                }
            };
            // At the stack's foot (`GlassPopoverStack`'s ZStack, anchor
            // bottom).
            slide.set_valign(gtk::Align::End);
            if slide.parent().is_none() {
                self.place.append(&slide);
            }
            self.front.replace(Some((transfer.id.clone(), slide)));
        }
        // The stack itself comes and goes at once, as on the Mac, where
        // the cards change in no animation (`MapTransferPlayer`) and its
        // `.transition(.opacity)` has none to run in.
        // The plates move only while the stack stays shown: set while it
        // is hidden, they are in place at once.
        if visible.is_empty() {
            self.cards.set_visible(false);
            self.plates.set_behind(0);
        } else {
            self.plates
                .set_behind(visible.len().saturating_sub(1).min(2));
            self.cards.set_visible(true);
        }
    }

    /// The cards in the stack's front place now: only the front card, the
    /// one it covered fading as a picture inside it
    /// (`popover_stack::Slide::covered_opacity`).
    pub fn front_cards(&self) -> Vec<gtk::Widget> {
        let mut cards = Vec::new();
        let mut child = self.place.first_child();
        while let Some(card) = child {
            child = card.next_sibling();
            cards.push(card);
        }
        cards
    }
}

pub struct TraceMap {
    pub root: gtk::Box,
    pub canvas: MapCanvas,
    /// The Private AI view's map.
    pub private_canvas: MapCanvas,
    /// The Traces / Private AI switch.
    pub traces_view: gtk::ToggleButton,
    pub private_view: gtk::ToggleButton,
    pub zoom_out: gtk::Button,
    pub zoom_in: gtk::Button,
    stage: gtk::Stack,
    foot: gtk::Box,
    /// `GlassSpinner(size: FlowMapView.loadingSpinner)`, shown while
    /// `loading`.
    spinner: gtk::DrawingArea,
    loading: std::cell::Cell<bool>,
    reads: RefCell<Reads>,
    pub transfers: Rc<Transfers>,
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
        // The Private AI view: its own map, named as the switch names it.
        // It draws nothing while the tool list is unread, as the Mac's.
        let destination =
            trace_commons_contributor::private_inference_copy::private_inference_copy().destination;
        let private_canvas = MapCanvas::new();
        private_canvas.update_property(&[gtk::accessible::Property::Label(destination)]);
        // Never a Traces map, even before the first read lands.
        private_canvas.set_inputs(SceneInputs::private_ai(PrivateAiInputs::default()));
        stage.add_named(&private_canvas, Some("private-ai"));
        overlay.set_child(Some(&stage));

        // Before the first reply the map is empty only because it is
        // unread: a spinner says so.
        let spinner = super::super::glass_feedback::spinner_sized(LOADING_SPINNER);
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
        let private_view = gtk::ToggleButton::with_label(destination);
        private_view.set_group(Some(&traces_view));
        traces_view.set_active(true);
        switch.append(&traces_view);
        switch.append(&private_view);
        overlay.add_overlay(&switch);

        // The map's foot, fixed whatever its zoom or focus: the trace's
        // cards above, then the legend and the zoom on one row; the legend
        // wraps before it would run under the zoom.
        let transfers = Transfers::new(&canvas);
        let foot = gtk::Box::new(gtk::Orientation::Vertical, 8);
        foot.set_valign(gtk::Align::End);
        foot.set_margin_start(14);
        foot.set_margin_end(14);
        foot.set_margin_bottom(14);
        let clamp = adw::Clamp::builder()
            .maximum_size(TRANSFER_CARD_WIDTH)
            .child(&transfers.cards)
            .build();
        foot.append(&clamp);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        foot.append(&row);
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
        row.append(&legend);
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
        row.append(&zoom);
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
            private_canvas,
            traces_view,
            private_view,
            zoom_out,
            zoom_in,
            stage,
            foot,
            spinner,
            loading: std::cell::Cell::new(false),
            reads: RefCell::new(Reads::default()),
            transfers,
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
        // The legend, the zoom and the spinner are the Traces map's: the
        // Mac's Private AI map has none of them.
        self.foot.set_visible(traces);
        self.spinner.set_visible(traces && self.loading.get());
    }

    /// `harness_list`'s answer, or `None` after a failed or unreadable
    /// read: the Private AI map then draws no tools as current.
    pub fn set_harnesses(&self, list: Option<&HarnessList>) {
        self.reads.borrow_mut().harnesses =
            list.map(|list| list.harnesses.iter().map(HarnessRow::from_model).collect());
    }

    /// The Private AI map's inputs from the reads.
    pub fn private_inputs(&self) -> SceneInputs {
        let reads = self.reads.borrow();
        SceneInputs::private_ai(PrivateAiInputs {
            harnesses: reads.harnesses.clone(),
            private_ai: map_scene::private_ai_label(
                reads.destinations.as_ref(),
                reads.status.as_ref(),
            ),
            destinations: reads.destinations.clone(),
        })
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
    pub fn pending_read(&self) -> bool {
        self.reads.borrow().pending == Some(true)
    }

    /// `(watched, total)` tools from the last `tool_destinations` read
    /// (`ToolDestinations.watchedCount`), or `None` when it was not read.
    pub fn watched_tools(&self) -> Option<(usize, usize)> {
        self.reads
            .borrow()
            .destinations
            .as_ref()
            .map(|destinations| {
                let watched = destinations
                    .tools
                    .iter()
                    .filter(|tool| {
                        tool.sessions
                            .as_ref()
                            .is_some_and(|sessions| sessions.watch == "watched")
                    })
                    .count();
                (watched, destinations.tools.len())
            })
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
            private_ai: None,
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
        self.loading.set(loading);
        self.spinner
            .set_visible(loading && self.stage.visible_child_name().as_deref() == Some("traces"));
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
        self.private_canvas.set_inputs(self.private_inputs());
        self.observe_transfers(
            &app.entries.borrow(),
            app.history_records.borrow().as_deref(),
            app.last_kept.borrow().as_deref(),
            app.last_dismissed.borrow().as_deref(),
        );
    }

    /// Hands the window's transfer player what the reads now say: a row
    /// History gained plays as a contribution, the last keep as a keep, the
    /// last dismissal shows its card.
    pub fn observe_transfers(
        &self,
        entries: &[QueueEntry],
        history: Option<&[HistoryRecord]>,
        last_kept: Option<&str>,
        last_dismissed: Option<&str>,
    ) {
        self.transfers
            .observe(entries, history, last_kept, last_dismissed);
    }

    /// Whether the transfer cards are at the map's foot: while a tab other
    /// than Traces is shown (`tabs::transfer_cards_shown`).
    /// The Traces map's style (Settings): the map changes at once; a run
    /// already playing keeps its own, the next ones take this.
    pub fn set_style(&self, style: MapStyle) {
        self.canvas.set_style(style);
        self.transfers.player.borrow_mut().style = style;
    }

    pub fn set_cards_shown(&self, shown: bool) {
        self.transfers.set_shown(shown);
    }

    /// The cards drawn now, front first, by their trace's id.
    pub fn card_ids(&self) -> Vec<String> {
        self.transfers.drawn.borrow().clone()
    }
}

/// The transfer cards' widest, as the Mac's.
pub const TRANSFER_CARD_WIDTH: i32 = 460;

/// The map's loading spinner, `FlowMapView.loadingSpinner`.
const LOADING_SPINNER: f64 = 20.0;

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
    label.add_css_class("tc-map-legend");
    item.append(&dot);
    item.append(&label);
    item
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::ui::monitor::map_scene::PileStep;

    const OPEN_PILE: PileStage = PileStage {
        tall: true,
        wide: true,
        names: true,
    };
    use trace_commons_contributor::harness_state::HarnessState;

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

    /// The lock the legend and the transfer card share draws ink in its
    /// 10 by 10 box.
    #[test]
    fn the_lock_draws_ink_in_its_box() {
        let surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 10, 10).expect("surface");
        {
            let cr = gtk::cairo::Context::new(&surface).expect("context");
            let ink = gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 1.0);
            draw_lock(&cr, Point::new(5.0, 5.0), 1.0, &ink, 1.0);
        }
        surface.flush();
        let mut surface = surface;
        let inked = surface
            .data()
            .expect("pixels")
            .chunks(4)
            .filter(|px| px[3] != 0)
            .count();
        assert!(inked >= 20, "the lock drew {inked} pixels");
    }

    fn parse(hex: &str) -> gtk::gdk::RGBA {
        gtk::gdk::RGBA::parse(hex).unwrap()
    }

    use crate::ui::omarchy::{self, Rgb};

    /// An opaque colour's channels as `omarchy::Rgb`, for the WCAG maths.
    fn rgb_of(c: &gtk::gdk::RGBA) -> Rgb {
        let ch = |v: f32| (f64::from(v) * 255.0).round() as u8;
        Rgb(ch(c.red()), ch(c.green()), ch(c.blue()))
    }

    /// `over` composited onto the opaque `ground`.
    fn over_ground(over: &gtk::gdk::RGBA, ground: Rgb) -> Rgb {
        ground.mix(rgb_of(over), f64::from(over.alpha()))
    }

    /// A `@define-color` literal from a generated sheet.
    fn css_define(css: &str, name: &str) -> Rgb {
        let prefix = format!("@define-color {name} ");
        css.lines()
            .find_map(|line| line.strip_prefix(&prefix))
            .and_then(|rest| Rgb::parse(rest.trim_end_matches(';')))
            .unwrap_or_else(|| panic!("{name}"))
    }

    /// Every look the map is drawn in: the standard look in each scheme and
    /// every vendored Omarchy palette in its own mode, with the grounds the
    /// map's words sit on there. The grounds are the stylesheet's, not the
    /// map's: the map pane and the window's scene (`tc_glass_pane`,
    /// `tc_glass_scene`) in the standard look, and all six of the skin's
    /// grounds; each under the map's field veil as the map draws it, at its
    /// centre and its edge.
    fn every_look() -> Vec<(String, super::Palette, Option<omarchy::Palette>, Vec<Rgb>)> {
        use crate::ui::omarchy::fixtures::FIXTURES;
        use crate::ui::style::{Inks, brand_tokens};
        let (ink, ground) = (gtk::gdk::RGBA::BLACK, gtk::gdk::RGBA::WHITE);
        let veiled = |palette: &super::Palette, grounds: Vec<Rgb>| {
            let mut all = grounds.clone();
            for g in grounds {
                all.push(over_ground(&palette.map.field_outer, g));
                all.push(over_ground(
                    &palette.map.field_inner,
                    over_ground(&palette.map.field_outer, g),
                ));
            }
            all
        };
        let mut looks = Vec::new();
        for (dark, css) in [
            (false, brand_tokens::GLASS_LIGHT_CSS),
            (true, brand_tokens::GLASS_DARK_CSS),
        ] {
            let palette = super::Palette::with_inks(ink, ground, &Inks::standard(dark), dark);
            let grounds = veiled(
                &palette,
                vec![
                    css_define(css, "tc_glass_pane"),
                    css_define(css, "tc_glass_scene"),
                ],
            );
            looks.push((format!("standard dark={dark}"), palette, None, grounds));
        }
        for (name, text) in FIXTURES {
            let p = omarchy::parse(text, false).unwrap();
            let dark = p.mode == omarchy::Mode::Dark;
            let palette = super::Palette::with_inks(ink, ground, &Inks::omarchy(&p), dark);
            let grounds = veiled(&palette, p.grounds().to_vec());
            looks.push((name.to_string(), palette, Some(p), grounds));
        }
        looks
    }

    /// Every word the map draws (`Word::ALL`, the only colours its text
    /// calls take) is opaque and clears 4.5:1 on every ground it is drawn
    /// on, in the standard look light and dark and in every Omarchy
    /// palette. The Mac's statusOff sublabel (#8a8a90) measured 3.43:1 on
    /// white and 3.09:1 on the light pane, and its dark tertiary text is
    /// translucent (0.84).
    #[test]
    fn the_map_secondary_labels_clear_the_text_floor() {
        let mut failures = Vec::new();
        for (look, palette, _, grounds) in every_look() {
            for word in super::Word::ALL {
                let colour = super::word_colour(&palette, word);
                if colour.alpha() < 1.0 {
                    failures.push(format!("{look} {word:?}: alpha {:.2}", colour.alpha()));
                }
                for g in &grounds {
                    let ratio = omarchy::contrast(rgb_of(&colour), *g);
                    if ratio < omarchy::TEXT {
                        failures.push(format!(
                            "{look} {word:?} {} on {}: {ratio:.2}:1",
                            rgb_of(&colour).hex(),
                            g.hex()
                        ));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// In the standard look the map draws the Mac's map colours wherever
    /// they clear the floor, and raises only the words that do not, toward
    /// the scheme's ink just far enough: the sublabel ends within a step of
    /// 4.5:1. Every fill and ring is the Mac's.
    #[test]
    fn the_standard_map_is_the_macs_raised_only_to_the_floor() {
        use crate::ui::style::Inks;
        for dark in [false, true] {
            let mac = MapColours::scheme(dark);
            let drawn = MapColours::resolve(dark, &Inks::standard(dark));
            assert_eq!(drawn.text_primary, mac.text_primary, "dark={dark}");
            assert_eq!(drawn.status_off, mac.status_off, "dark={dark}");
            assert_eq!(drawn.rule_fills, mac.rule_fills, "dark={dark}");
            for (fill, mac_fill) in [
                (drawn.status_on, mac.status_on),
                (drawn.status_ask, mac.status_ask),
                (drawn.status_outside, mac.status_outside),
                (drawn.orb_shared, mac.orb_shared),
                (drawn.orb_kept, mac.orb_kept),
                (drawn.pane_opaque, mac.pane_opaque),
                (drawn.field_inner, mac.field_inner),
                (drawn.field_outer, mac.field_outer),
            ] {
                assert_eq!(fill, mac_fill, "dark={dark}");
            }
            assert_eq!(drawn.tools, mac.tools, "dark={dark}");
        }
        let light = MapColours::resolve(false, &Inks::standard(false));
        assert_ne!(
            light.sublabel,
            parse("#8a8a90"),
            "the sublabel was not raised"
        );
        let (_, _, _, grounds) = every_look().remove(0);
        let least = grounds
            .iter()
            .map(|g| omarchy::contrast(rgb_of(&light.sublabel), *g))
            .fold(f64::INFINITY, f64::min);
        assert!(
            (4.5..4.7).contains(&least),
            "raised past the floor: {least:.2}:1 on its darkest ground"
        );
        // The light tertiary already clears: kept.
        assert_eq!(light.text_tertiary, parse("#3d3d43"));
        let dark = MapColours::resolve(true, &Inks::standard(true));
        assert_eq!(dark.text_tertiary.alpha(), 1.0);
    }

    /// Under the Omarchy skin the map draws the skin's roles, not the
    /// Mac's: what a folder's dot is drawn in, by rule, is the skin's
    /// status colour (and no pixel is the Mac's), and its words, field and
    /// panes are the skin's.
    #[test]
    fn the_map_palette_takes_its_fills_from_the_inks() {
        let mut failures = Vec::new();
        for (look, palette, skin, _) in every_look() {
            let Some(p) = skin else { continue };
            let dark = p.mode == omarchy::Mode::Dark;
            let mac = MapColours::scheme(dark);
            let rgba = |c: Rgb| gtk::gdk::RGBA::parse(c.hex()).unwrap();
            for (mode, skin_ring, mac_ring) in [
                (ProjectMode::AutoUpload, p.status_on, mac.status_on),
                (ProjectMode::Ask, p.status_ask, mac.status_ask),
                (ProjectMode::Ignore, p.status_outside, mac.status_outside),
            ] {
                let scene = traces_scene(&[]);
                let fit = MapGeometry::new(Size::new(580.0, 760.0), 1.0, None, scene.space);
                let at = Point::new(scene.space.width / 2.0, scene.space.height / 2.0);
                let node = map_scene::Node::new(
                    "folder:p",
                    NodeKind::Folder(Some(mode)),
                    at,
                    20.0,
                    "p",
                    "",
                );
                let pixels = painted(580, 760, |cr| {
                    draw_node(cr, &scene, &node, &fit, &palette, false, MapStyle::Launch);
                });
                let skin_px = pixels
                    .iter()
                    .filter(|px| near_colour(**px, &rgba(skin_ring)))
                    .count();
                let mac_px = pixels
                    .iter()
                    .filter(|px| near_colour(**px, &mac_ring))
                    .count();
                if skin_px < 10 || (rgba(skin_ring) != mac_ring && mac_px > skin_px) {
                    failures.push(format!(
                        "{look} {mode:?}: {skin_px} skin pixels, {mac_px} Mac pixels"
                    ));
                }
            }
            if super::word_colour(&palette, super::Word::Name) != rgba(p.ink) {
                failures.push(format!("{look}: names are not the skin's ink"));
            }
            if palette.map.pane_opaque != rgba(p.bg) {
                failures.push(format!("{look}: the open pile is not the skin's pane"));
            }
            if rgb_of(&palette.map.field_outer) != p.scene {
                failures.push(format!("{look}: the field is not the skin's scene"));
            }
            if palette.map.orb_shared != rgba(p.accent) {
                failures.push(format!("{look}: the shared orb is not the skin's accent"));
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// The map's colours are the Mac's flat-theme tokens in each scheme.
    #[test]
    fn the_map_colours_follow_the_scheme() {
        let light = MapColours::scheme(false);
        let dark = MapColours::scheme(true);
        assert_eq!(light.orb_shared, parse("#056bdc"));
        assert_eq!(dark.orb_shared, parse("#0091fd"));
        assert_eq!(dark.orb_kept, parse("#d95926"));
        assert_eq!(light.tint(Tint::Claude), parse("#c2603e"));
        assert_eq!(dark.tint(Tint::Claude), parse("#e08a6a"));
        assert_eq!(dark.tint(Tint::OpenCode), parse("#4fd39a"));
        assert_eq!(light.tint(Tint::Primary), light.text_primary);
        assert_eq!(light.well_fill, parse("rgba(138,147,165,0.28)"));
        assert_eq!(dark.pane_opaque, parse("#0d1124"));
        assert_eq!(
            Palette::fixed().map,
            MapColours::resolve(false, &crate::ui::style::Inks::standard(false))
        );
    }

    fn traces_scene(entries: &[QueueEntry]) -> Scene {
        let tree = TracesTree::build(entries, &[], None);
        Scene::traces(
            &tree,
            &CommonsGate::with_state(ScreenState::Ready),
            &Contributions::unread(),
            None,
            &BTreeMap::new(),
            map_scene::SIZE,
        )
    }

    /// The pixels a drawing leaves with any alpha, unpremultiplied.
    fn painted(width: i32, height: i32, draw: impl Fn(&gtk::cairo::Context)) -> Vec<[u8; 4]> {
        let mut surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, width, height).unwrap();
        {
            let cr = gtk::cairo::Context::new(&surface).unwrap();
            draw(&cr);
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        let mut out = Vec::new();
        for y in 0..height as usize {
            for x in 0..width as usize {
                let i = y * stride + x * 4;
                if data[i + 3] > 0 {
                    // Unpremultiplied RGBA.
                    let a = f64::from(data[i + 3]) / 255.0;
                    let channel = |v: u8| (f64::from(v) / a).round().min(255.0) as u8;
                    out.push([
                        channel(data[i + 2]),
                        channel(data[i + 1]),
                        channel(data[i]),
                        data[i + 3],
                    ]);
                }
            }
        }
        out
    }

    fn near_colour(pixel: [u8; 4], colour: &gtk::gdk::RGBA) -> bool {
        let target = [colour.red(), colour.green(), colour.blue()];
        (0..3).all(|k| (f32::from(pixel[k]) - target[k] * 255.0).abs() <= 12.0) && pixel[3] > 200
    }

    /// On the Traces map the library is the station on the rocket map and
    /// the orb on the standard one, drawn over the still layer on the
    /// map's clock: the still layer draws no disc for it. Its frame is the
    /// Mac's: the station 150 by 130 (`FlowMapView.stationSize`), the orb
    /// 104 square (`MapTransferDrawing.orbFrame`), centred on the
    /// library's point at the map's scale.
    #[test]
    fn the_traces_library_is_the_station_or_the_orb_not_a_disc() {
        let scene = traces_scene(&[entry("a", "p", "codex")]);
        let fit = MapGeometry::new(Size::new(580.0, 760.0), 1.5, None, scene.space);
        let palette = Palette::fixed();
        let library = scene.nodes.iter().find(|n| n.id == "library").unwrap();
        let pixels = painted(580, 760, |cr| {
            draw_node(cr, &scene, library, &fit, &palette, false, MapStyle::Launch);
        });
        assert!(pixels.is_empty(), "the still layer drew the library");
        let centre = fit.point(library.at);
        for (style, (w, h)) in [
            (MapStyle::Launch, (150.0, 130.0)),
            (MapStyle::Standard, (104.0, 104.0)),
        ] {
            let frame = library_frame(&scene, &fit, style).expect("a library");
            assert_eq!((frame.width, frame.height), (w * fit.scale, h * fit.scale));
            assert_eq!(
                frame.origin,
                (
                    centre.x - w * fit.scale / 2.0,
                    centre.y - h * fit.scale / 2.0
                )
            );
            assert!(!frame.lit, "nothing contributed yet: unlit");
        }
        assert!(library_turns(&scene));
        let elsewhere = Scene::default();
        assert!(library_frame(&elsewhere, &fit, MapStyle::Launch).is_none());
        assert!(!library_turns(&elsewhere));
    }

    /// This computer on the Traces map is the launch pad on the rocket map
    /// and the laptop on the standard one (`FlowMapView.drawNode`): only
    /// the laptop's screen rises 20 above its point, inside the lid.
    #[test]
    fn this_computer_is_the_pad_or_the_laptop_by_the_maps_style() {
        let scene = traces_scene(&[entry("a", "p", "codex")]);
        let fit = MapGeometry::new(Size::new(580.0, 760.0), 1.0, None, scene.space);
        let palette = Palette::fixed();
        let hub = scene.nodes.iter().find(|n| n.id == "hub").unwrap();
        let centre = fit.point(hub.at);
        let probe = (centre.x as usize, (centre.y - 20.0) as usize);
        let at = |style| {
            let mut surface =
                gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 580, 760).unwrap();
            {
                let cr = gtk::cairo::Context::new(&surface).unwrap();
                draw_node(&cr, &scene, hub, &fit, &palette, false, style);
            }
            surface.flush();
            let stride = surface.stride() as usize;
            surface.data().unwrap()[probe.1 * stride + probe.0 * 4 + 3]
        };
        assert_eq!(at(MapStyle::Launch), 0, "the pad reaches up into the lid");
        assert!(at(MapStyle::Standard) > 200, "no laptop screen");
    }

    /// The library turns on the map's resting timeline, as on the Mac,
    /// where it is drawn on that timeline (20 frames a second, 30 while
    /// arcs carry dashes) and never on the run's 60: its drawing is built
    /// once a resting frame and kept between, however often the map draws.
    #[test]
    fn the_library_is_built_once_a_resting_frame_while_a_run_draws_at_sixty() {
        let art = LibraryArt::default();
        let frame = LibraryFrame {
            origin: (10.0, 20.0),
            width: 150.0,
            height: 130.0,
            lit: true,
        };
        // One second of a run, drawn at 60 frames a second.
        for i in 0..60 {
            let now = 100.01 + f64::from(i) / 60.0;
            art.node(
                MapStyle::Launch,
                &frame,
                library_seconds(now, 1.0 / 20.0),
                false,
            );
        }
        assert_eq!(art.builds(), 20, "20 station frames in a second");
        // With moving dashes the resting timeline is 30 a second.
        for i in 0..60 {
            let now = 200.01 + f64::from(i) / 60.0;
            art.node(
                MapStyle::Launch,
                &frame,
                library_seconds(now, 1.0 / 30.0),
                false,
            );
        }
        assert_eq!(art.builds(), 50);
        // Under reduced motion it rests: built once, whatever the clock.
        let still = LibraryArt::default();
        for i in 0..60 {
            let now = 300.0 + f64::from(i) / 7.0;
            still.node(MapStyle::Launch, &frame, now, true);
        }
        assert_eq!(still.builds(), 1);
        // A change of light, frame or style builds it again.
        still.node(
            MapStyle::Launch,
            &LibraryFrame {
                lit: false,
                ..frame
            },
            0.0,
            true,
        );
        still.node(
            MapStyle::Standard,
            &LibraryFrame {
                lit: false,
                ..frame
            },
            0.0,
            true,
        );
        assert_eq!(still.builds(), 3);
    }

    /// A coin is drawn as `MapToolPile` draws it: its edge and rim in the
    /// tool's tint and its logo inside, on an opaque face.
    #[test]
    fn a_coin_is_drawn_in_its_tools_tint() {
        let scene = traces_scene(&[entry("a", "p", "claude-code")]);
        let fit = MapGeometry::new(Size::new(580.0, 760.0), 1.0, None, scene.space);
        let palette = Palette::fixed();
        let stack = scene.stack.as_ref().unwrap();
        let pixels = painted(580, 760, |cr| {
            draw_pile(
                cr,
                &scene,
                stack,
                &stack.tools,
                &[],
                None,
                &fit,
                &palette,
                PileStage::default(),
            );
        });
        let tint = palette.map.tint(Tint::Claude);
        let tinted = pixels.iter().filter(|p| near_colour(**p, &tint)).count();
        assert!(tinted > 150, "only {tinted} pixels in the Claude tint");
    }

    /// The pile's card follows the Mac's stages: taller first, still the
    /// narrow card in the well's fill; only once wide is it the open,
    /// opaque card (`MapToolPile`: width from `wide`, the opaque fill
    /// from `open && wide`).
    #[test]
    fn the_pile_card_widens_only_at_its_wide_stage() {
        let scene = traces_scene(&[entry("a", "p", "claude-code"), entry("b", "p", "codex")]);
        let fit = MapGeometry::new(Size::new(580.0, 760.0), 1.0, None, scene.space);
        let palette = Palette::fixed();
        let stack = scene.stack.as_ref().unwrap();
        let (x, y, w, h) = stack.card(&fit, true, true);
        let narrow = stack.card(&fit, true, false).2;
        assert!(w > narrow + 20.0);
        // A point in the wide card, right of the narrow one.
        let probe = ((x + (narrow + w) / 2.0) as usize, (y + h / 2.0) as usize);
        let at = |stage: PileStage| {
            let mut surface =
                gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 580, 760).unwrap();
            {
                let cr = gtk::cairo::Context::new(&surface).unwrap();
                draw_pile(
                    &cr,
                    &scene,
                    stack,
                    &stack.tools,
                    &[],
                    None,
                    &fit,
                    &palette,
                    stage,
                );
            }
            surface.flush();
            let stride = surface.stride() as usize;
            let data = surface.data().unwrap();
            let i = probe.1 * stride + probe.0 * 4;
            [data[i + 2], data[i + 1], data[i], data[i + 3]]
        };
        let tall = PileStage {
            tall: true,
            ..PileStage::default()
        };
        assert_eq!(at(tall)[3], 0, "the card widened before its wide stage");
        let open = at(PileStage {
            tall: true,
            wide: true,
            names: false,
        });
        let pane = palette.map.pane_opaque;
        assert_eq!(open[3], 255, "the open card is opaque");
        assert!((f32::from(open[0]) - pane.red() * 255.0).abs() <= 2.0);
    }

    /// The card grows on the Mac's spring rather than in a step: 0.1 s
    /// after the pointer comes over it, it is 61% of the way to its open
    /// height (`spring(0.34, 0.84)`), drawn so; a moving trace closes it at
    /// once, and its fill turns opaque only with its width.
    #[test]
    fn the_pile_card_grows_on_its_spring() {
        let scene = traces_scene(&[
            entry("a", "p", "claude-code"),
            entry("b", "p", "codex"),
            entry("c", "p", "cursor"),
        ]);
        let fit = MapGeometry::new(Size::new(580.0, 760.0), 1.0, None, scene.space);
        let palette = Palette::fixed();
        let stack = scene.stack.as_ref().unwrap();
        let mut motion = PileMotion::default();
        motion.follow(0.0, false, &stack.tools, &[], false);
        let tall = PileStage {
            tall: true,
            ..PileStage::default()
        };
        motion.step(PileStep::Tall(true), 0.0, false);
        motion.follow(0.0, true, &stack.tools, &[], false);
        let frame = PileFrame::moving(&motion, tall, false, &stack.tools, &[], 0.1);
        assert!((frame.tall - 0.609_732_579).abs() < 1e-6, "{}", frame.tall);
        assert!((frame.lay - 0.582_676_783).abs() < 1e-6, "{}", frame.lay);
        assert_eq!((frame.wide, frame.opaque), (0.0, 0.0));
        // Drawn: filled to the spring's height, not to the open one.
        let (x, closed_top, _, _) = stack.card(&fit, false, false);
        let (_, open_top, _, _) = stack.card(&fit, true, false);
        let (_, now_top, _, _) = stack.card_at(&fit, frame.tall, 0.0);
        assert!(open_top + 4.0 < now_top && now_top + 4.0 < closed_top);
        let alpha_at = |y: f64| {
            let mut surface =
                gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 580, 760).unwrap();
            {
                let cr = gtk::cairo::Context::new(&surface).unwrap();
                draw_pile_frame(&cr, &scene, stack, &frame, None, &fit, &palette);
            }
            surface.flush();
            let stride = surface.stride() as usize;
            let data = surface.data().unwrap();
            data[y as usize * stride + (x + 30.0) as usize * 4 + 3]
        };
        assert!(alpha_at(now_top + 2.0) > 0, "inside the growing card");
        assert_eq!(alpha_at((open_top + now_top) / 2.0), 0, "past it");
        // A trace moving: the card closed at once, the coins on their way.
        let moving = PileFrame::moving(&motion, tall, true, &stack.tools, &[], 0.1);
        assert_eq!((moving.tall, moving.wide, moving.names), (0.0, 0.0, 0.0));
        assert!(!moving.open);
        // Opaque only as wide as it is.
        motion.step(PileStep::Wide(true), 0.26, false);
        let wide = PileFrame::moving(&motion, tall, false, &stack.tools, &[], 0.36);
        assert!(
            (wide.opaque - 0.676_634_630).abs() < 1e-6,
            "{}",
            wide.opaque
        );
    }

    /// Offscreen frames of what moves, for review beside the Mac's: the
    /// pile opening, the shimmer crossing the field and the light under
    /// the pointer, each at several times. Written only when
    /// `TC_MOTION_RENDER_DIR` names a directory.
    #[test]
    fn write_motion_renders() {
        let Ok(dir) = std::env::var("TC_MOTION_RENDER_DIR") else {
            return;
        };
        let (w, h) = (440, 600);
        let save = |name: &str, paint: &dyn Fn(&gtk::cairo::Context)| {
            let surface =
                gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w, h).unwrap();
            {
                let cr = gtk::cairo::Context::new(&surface).unwrap();
                paint(&cr);
            }
            surface.flush();
            let stride = surface.stride();
            let mut surface = surface;
            let data = surface.data().unwrap().to_vec();
            std::fs::write(format!("{dir}/{name}-{w}x{h}x{stride}.argb"), data).unwrap();
        };
        let scene = traces_scene(&[
            entry("a", "p", "claude-code"),
            entry("b", "p", "codex"),
            entry("c", "p", "cursor"),
        ]);
        let fit = MapGeometry::new(
            Size::new(f64::from(w), f64::from(h)),
            1.0,
            None,
            scene.space,
        );
        let palette = Palette::fixed();
        let colours = MapColours::scheme(true);
        let ground = |cr: &gtk::cairo::Context| {
            source(cr, &colours.pane_opaque, 1.0);
            let _ = cr.paint();
            field::draw(cr, f64::from(w), f64::from(h), &colours, true);
        };
        let stack = scene.stack.as_ref().unwrap();
        let mut motion = PileMotion::default();
        motion.follow(0.0, false, &stack.tools, &[], false);
        let stage = PileStage {
            tall: true,
            wide: true,
            names: true,
        };
        motion.step(PileStep::Tall(true), 0.0, false);
        motion.step(PileStep::Wide(true), 0.26, false);
        motion.step(PileStep::Names(true), 0.46, false);
        motion.follow(0.0, true, &stack.tools, &[], false);
        for t in [0.0, 0.05, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.8] {
            let frame = PileFrame::moving(&motion, stage, false, &stack.tools, &[], t);
            save(&format!("pile-open-t{:03}", (t * 100.0) as i32), &|cr| {
                ground(cr);
                draw_pile_frame(cr, &scene, stack, &frame, None, &fit, &palette);
            });
        }
        let mask = field::tile_mask(f64::from(w), f64::from(h), 1.0).unwrap();
        for t in [0.1, 0.25, 0.4, 0.55, 0.7, 0.9] {
            let progress = map_motion::shimmer_progress(t);
            save(&format!("shimmer-t{:03}", (t * 100.0) as i32), &|cr| {
                ground(cr);
                field::draw_shimmer(cr, &mask, f64::from(w), f64::from(h), progress, true);
            });
        }
        for opacity in [0.25, 0.5, 1.0] {
            save(&format!("light-o{:03}", (opacity * 100.0) as i32), &|cr| {
                ground(cr);
                field::draw_light(
                    cr,
                    f64::from(w),
                    f64::from(h),
                    (220.0, 300.0),
                    opacity,
                    true,
                );
            });
        }
    }

    fn private_rows() -> Vec<HarnessRow> {
        ["claude", "cursor"]
            .iter()
            .map(|id| HarnessRow {
                id: (*id).into(),
                name: if *id == "claude" {
                    "Claude Code"
                } else {
                    "Cursor"
                }
                .into(),
                installed: true,
                state: "answering".into(),
            })
            .collect()
    }

    /// Private AI inputs draw the Private AI map, in the Mac's default
    /// `ready` state, with the managed routes after it; the Traces inputs
    /// draw the Traces map.
    #[test]
    fn private_ai_inputs_draw_the_private_ai_map() {
        let destinations: ToolDestinations = serde_json::from_value(serde_json::json!({
            "private_ai": "running",
            "tools": [],
            "managed_copy": {"title": "Managed"},
            "managed_sessions": [{"tool": "opencode", "provider": "openai",
                                  "state": "running", "collection_state": "off"}]
        }))
        .unwrap();
        let inputs = SceneInputs::private_ai(PrivateAiInputs {
            harnesses: Some(private_rows()),
            private_ai: Some("running".into()),
            destinations: Some(destinations.clone()),
        });
        assert!(inputs.gate.state.is_healthy());
        let scene = inputs.scene(&BTreeMap::new(), map_scene::SIZE);
        // `PrivateAIFlowMap`: no pile, the credential lit and answering,
        // each tool in `harness_list` beside it by its own name, joined by
        // a flowing arc (its calls are answered), and the managed route
        // after them.
        assert!(scene.stack.is_none() && scene.flows());
        assert_eq!(scene.nodes[0].kind, NodeKind::Destination(Lamp::Answering));
        let harnesses: Vec<(&str, &str)> = scene
            .nodes
            .iter()
            .filter(|n| n.id.starts_with("harness:"))
            .map(|n| (n.id.as_str(), n.label.as_str()))
            .collect();
        assert_eq!(
            harnesses,
            [
                ("harness:claude", "Claude Code"),
                ("harness:cursor", "Cursor")
            ]
        );
        let flowing = scene
            .arcs
            .iter()
            .filter(|arc| arc.style == ArcStyle::Flowing)
            .count();
        assert_eq!(flowing, 2, "one flowing arc a tool");
        assert!(
            scene
                .nodes
                .iter()
                .any(|n| n.id == "managed:opencode:openai")
        );
        // Without the managed routes there is no managed node: they come
        // from `tool_destinations`.
        let unmanaged = SceneInputs::private_ai(PrivateAiInputs {
            harnesses: Some(private_rows()),
            private_ai: Some("running".into()),
            destinations: None,
        })
        .scene(&BTreeMap::new(), map_scene::SIZE);
        assert!(!unmanaged.nodes.iter().any(|n| n.id.starts_with("managed:")));
        // Unread, nothing is drawn.
        let unread = SceneInputs::private_ai(PrivateAiInputs::default());
        assert!(!unread.scene(&BTreeMap::new(), map_scene::SIZE).draws_map());
        // The Traces inputs are untouched by it.
        let traces = SceneInputs::default().scene(&BTreeMap::new(), map_scene::SIZE);
        assert!(traces.stack.is_some());
    }

    /// A node's words are in the Mac's text colours, not the theme's ink
    /// (`FlowMapView.drawNode`): its name in the primary text colour, its
    /// sublabel in statusOff's text twin (#8a8a90 raised to the 4.5:1
    /// floor, `the_standard_map_is_the_macs_raised_only_to_the_floor`),
    /// both faded with the node; a "+N" in the tertiary text colour.
    #[test]
    fn a_nodes_words_are_in_the_macs_text_colours() {
        let palette = Palette {
            ink: gtk::gdk::RGBA::new(1.0, 0.0, 0.0, 1.0),
            ..Palette::fixed()
        };
        let mut node = map_scene::Node::new(
            "folder:p",
            NodeKind::Folder(None),
            Point::new(0.0, 0.0),
            8.0,
            "p",
            "",
        );
        node.dim = 0.5;
        let (label, sublabel) = node_word_colours(&node, &palette);
        assert_eq!(label, parse("#1d1d1f").with_alpha(0.5));
        assert_eq!(sublabel, palette.map.sublabel.with_alpha(0.5));
        assert_ne!(palette.map.sublabel, parse("#8a8a90"));
        assert_eq!(overflow_colour(&palette), parse("#3d3d43"));
        let dark = Palette {
            map: MapColours::resolve(true, &crate::ui::style::Inks::standard(true)),
            ..palette
        };
        assert_eq!(overflow_colour(&dark), dark.map.text_tertiary);
    }

    /// The ring round the node under the pointer is in the accent text
    /// colour (`GlassColor.accentText`, the flat theme's purpleText,
    /// #023e83 in light), faded with the node.
    #[test]
    fn the_selection_ring_is_the_accent_text_colour() {
        let palette = Palette {
            ink: gtk::gdk::RGBA::new(1.0, 0.0, 0.0, 1.0),
            ..Palette::fixed()
        };
        let node = map_scene::Node::new(
            "harness:x",
            NodeKind::Harness {
                tool: None,
                installed: true,
            },
            Point::new(100.0, 100.0),
            16.0,
            "",
            "",
        );
        let space = Size::new(200.0, 200.0);
        let fit = MapGeometry::new(space, 2.0, None, space);
        let pixels = painted(200, 200, |cr| {
            draw_node(
                cr,
                &Scene::default(),
                &node,
                &fit,
                &palette,
                true,
                MapStyle::Launch,
            );
        });
        let accent = parse("#023e83");
        let ring = pixels.iter().filter(|p| near_colour(**p, &accent)).count();
        assert!(
            ring > 100,
            "only {ring} pixels of the ring in the accent text colour"
        );
    }

    /// The pixels of a tool's initials on a node drawn at `zoom`, as
    /// offsets from the node's centre: what is drawn with the disc's ink
    /// made clear, so only the letters' coverage counts, not their colour.
    fn initials_mask(
        kind: NodeKind,
        label: &str,
        zoom: f64,
        colour: impl Fn(&Palette) -> gtk::gdk::RGBA,
    ) -> Vec<(i32, i32)> {
        let palette = Palette {
            ink: gtk::gdk::RGBA::new(1.0, 0.0, 0.0, 0.0),
            ..Palette::fixed()
        };
        let _ = colour(&palette);
        let space = Size::new(200.0, 200.0);
        let fit = MapGeometry::new(space, zoom, None, space);
        let node =
            map_scene::Node::new("harness:x", kind, Point::new(100.0, 100.0), 16.0, label, "");
        let centre = fit.point(node.at);
        let mut surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 200, 200).unwrap();
        {
            let cr = gtk::cairo::Context::new(&surface).unwrap();
            draw_node(
                &cr,
                &Scene::default(),
                &node,
                &fit,
                &palette,
                false,
                MapStyle::Launch,
            );
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        // The disc's ink is clear here, so whatever is drawn is the
        // letters: their coverage, whatever their colour.
        let mut mask = Vec::new();
        for y in 0..200usize {
            for x in 0..200usize {
                if data[y * stride + x * 4 + 3] >= 128 {
                    mask.push((
                        x as i32 - centre.x.round() as i32,
                        y as i32 - centre.y.round() as i32,
                    ));
                }
            }
        }
        mask
    }

    /// On the map's nodes the Mac sets a tool's initials in the micro type
    /// (`FlowMapView.mark`: `TypeScale.micro`, the system face, 10 bold,
    /// not scaled with the map), the same for a tool with art of its own
    /// (Gemini CLI's "Gm") and one without (a harness named "Gmx", whose
    /// initials are also "Gm"): the two marks are the same letters in the
    /// same face, only their colour differs, and zooming does not grow them.
    #[test]
    fn a_map_nodes_initials_are_the_micro_type_whatever_the_tool() {
        let gemini = |zoom| {
            initials_mask(
                NodeKind::Harness {
                    tool: Some(SourceKind::GeminiCli),
                    installed: true,
                },
                "Gemini CLI",
                zoom,
                |p| p.map.tint(Tint::Antigravity),
            )
        };
        let other = initials_mask(
            NodeKind::Harness {
                tool: None,
                installed: true,
            },
            "Gmx",
            1.0,
            |p| p.map.text_primary,
        );
        let at_one = gemini(1.0);
        assert!(
            at_one.len() > 20,
            "Gemini's initials drew {} pixels",
            at_one.len()
        );
        assert_eq!(
            at_one, other,
            "Gemini CLI's initials are not set as another tool's"
        );
        assert_eq!(gemini(2.0), at_one, "the initials grew with the map");
    }

    /// A Private AI tool with no artwork is drawn with two letters of its
    /// name in the primary ink, never as a blank circle.
    #[test]
    fn a_harness_without_artwork_carries_its_initials() {
        let scene = Scene::private_ai(
            Some(&private_rows()),
            Some("running"),
            |_| None,
            |_| HarnessState::Answering,
            None,
            map_scene::SIZE,
        );
        let fit = MapGeometry::new(Size::new(580.0, 760.0), 2.0, None, scene.space);
        let palette = Palette::fixed();
        let cursor = scene
            .nodes
            .iter()
            .find(|n| n.id == "harness:cursor")
            .unwrap();
        let pixels = painted(580, 760, |cr| {
            draw_node(cr, &scene, cursor, &fit, &palette, false, MapStyle::Launch);
        });
        let ink = pixels
            .iter()
            .filter(|p| near_colour(**p, &palette.map.text_primary))
            .count();
        assert!(ink > 20, "only {ink} pixels of initials");
    }

    /// The looks the comparison renders are drawn in: the standard look
    /// light and dark, and two Omarchy palettes, one light and one dark.
    fn render_looks() -> Vec<(String, Palette, bool)> {
        use crate::ui::omarchy::fixtures::FIXTURES;
        use crate::ui::style::Inks;
        let mut looks = vec![
            ("light".to_string(), Palette::fixed(), false),
            (
                "dark".to_string(),
                Palette::with_inks(
                    gtk::gdk::RGBA::WHITE,
                    parse("#0d1124"),
                    &Inks::standard(true),
                    true,
                ),
                true,
            ),
        ];
        for (name, text) in FIXTURES {
            if !matches!(name, "catppuccin-latte" | "tokyo-night") {
                continue;
            }
            let p = omarchy::parse(text, false).unwrap();
            let dark = p.mode == omarchy::Mode::Dark;
            let (r, g, b) = p.ink.unit();
            let ink = gtk::gdk::RGBA::new(r as f32, g as f32, b as f32, 1.0);
            let (r, g, b) = p.bg.unit();
            let ground = gtk::gdk::RGBA::new(r as f32, g as f32, b as f32, 1.0);
            looks.push((
                format!("omarchy-{name}"),
                Palette::with_inks(ink, ground, &Inks::omarchy(&p), dark),
                dark,
            ));
        }
        looks
    }

    /// Renders the Private AI map from the Mac's recorded samples, as the
    /// Mac's `privateai-scene-<set>` renders were made: laid out in their
    /// 440 x 614 pane at scale 1 and drawn at 2x. Words are drawn with
    /// cairo's own text, since the widget's fonts need a display, so they
    /// are placed as the map places them but not set in its font. Writes
    /// raw premultiplied BGRA into `TC_MAP_RENDER_DIR`.
    #[test]
    #[ignore = "writes comparison renders; set TC_MAP_RENDER_DIR"]
    fn render_private_ai_map_for_comparison() {
        let Ok(dir) = std::env::var("TC_MAP_RENDER_DIR") else {
            return;
        };
        let samples = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../macos/Sources/TCShellCore/DataContract/RecordedSamples");
        let read = |set: &str, method: &str| -> serde_json::Value {
            let text =
                std::fs::read_to_string(samples.join(set).join(format!("{method}.json"))).unwrap();
            serde_json::from_str(&text).unwrap()
        };
        let (w, h) = (440, 614);
        let space = Size::new(f64::from(w), f64::from(h));
        for set in ["normalDay", "empty", "armedFolder"] {
            let list: HarnessList = serde_json::from_value(read(set, "harness_list")).unwrap();
            let destinations: ToolDestinations =
                serde_json::from_value(read(set, "tool_destinations")).unwrap();
            let status: GateStatus = serde_json::from_value(read(set, "status")).unwrap();
            let inputs = SceneInputs::private_ai(PrivateAiInputs {
                harnesses: Some(list.harnesses.iter().map(HarnessRow::from_model).collect()),
                private_ai: map_scene::private_ai_label(Some(&destinations), Some(&status)),
                destinations: Some(destinations),
            });
            let scene = inputs.scene(&BTreeMap::new(), space);
            let fit = MapGeometry::new(space, 1.0, None, scene.space);
            for (scheme, palette, dark) in render_looks() {
                let mut surface =
                    gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w * 2, h * 2)
                        .unwrap();
                {
                    let cr = gtk::cairo::Context::new(&surface).unwrap();
                    source(&cr, &palette.map.pane_opaque, 1.0);
                    let _ = cr.paint();
                    cr.scale(2.0, 2.0);
                    field::draw(&cr, f64::from(w), f64::from(h), &palette.map, dark);
                    draw_arcs(&cr, &scene, &fit, &palette, true, 0.0);
                    for node in &scene.nodes {
                        draw_node(&cr, &scene, node, &fit, &palette, false, MapStyle::Launch);
                    }
                    let (label, sublabel, label_offset, sublabel_offset) =
                        map_scene::label_type(fit.scale);
                    let text = |t: &str,
                                at: Point,
                                size: f64,
                                bold: bool,
                                anchor: f64,
                                c: gtk::gdk::RGBA| {
                        cr.select_font_face(
                            "sans-serif",
                            gtk::cairo::FontSlant::Normal,
                            if bold {
                                gtk::cairo::FontWeight::Bold
                            } else {
                                gtk::cairo::FontWeight::Normal
                            },
                        );
                        cr.set_font_size(size);
                        let e = cr.text_extents(t).unwrap();
                        source(&cr, &c, 1.0);
                        cr.move_to(
                            at.x - e.x_advance() * anchor,
                            at.y - e.y_bearing() - e.height() / 2.0,
                        );
                        let _ = cr.show_text(t);
                        cr.new_path();
                    };
                    for node in &scene.nodes {
                        let centre = fit.point(node.at);
                        let radius = node.radius * fit.scale;
                        let (ink, sublabel_colour) = node_word_colours(node, &palette);
                        if node.label_beside() {
                            let gap = 6.0 * fit.scale.max(map_scene::LABEL_FLOOR);
                            text(
                                &node.label,
                                Point::new(centre.x - radius - gap, centre.y),
                                label,
                                true,
                                1.0,
                                ink,
                            );
                        } else {
                            text(
                                &node.label,
                                Point::new(centre.x, centre.y + radius + label_offset),
                                label,
                                true,
                                0.5,
                                ink,
                            );
                        }
                        if let Some(line) = node.sublabel.as_deref() {
                            text(
                                line,
                                Point::new(centre.x, centre.y + radius + sublabel_offset),
                                sublabel,
                                false,
                                0.5,
                                sublabel_colour,
                            );
                        }
                    }
                    for overflow in &scene.overflows {
                        text(
                            &format!("+{}", overflow.count),
                            fit.point(overflow.at),
                            MarkFont::MICRO.size,
                            true,
                            0.5,
                            overflow_colour(&palette),
                        );
                    }
                }
                surface.flush();
                let name = format!(
                    "{dir}/privateai-scene-{set}-{scheme}-{}x{}.bgra",
                    w * 2,
                    h * 2
                );
                std::fs::write(name, &*surface.data().unwrap()).unwrap();
            }
        }
    }

    /// Renders the Traces map's drawing (no words: they need a widget's
    /// fonts) in each scheme, for comparison with the Mac's. Writes raw
    /// premultiplied BGRA into `TC_MAP_RENDER_DIR`.
    #[test]
    #[ignore = "writes comparison renders; set TC_MAP_RENDER_DIR"]
    fn render_traces_map_for_comparison() {
        let Ok(dir) = std::env::var("TC_MAP_RENDER_DIR") else {
            return;
        };
        let entries: Vec<QueueEntry> = ["claude-code", "codex", "gemini-cli", "cline", "opencode"]
            .iter()
            .enumerate()
            .map(|(i, source)| entry(&format!("e{i}"), &format!("p{}", i % 2), source))
            .collect();
        let scene = traces_scene(&entries);
        let (w, h) = (580, 760);
        let fit = MapGeometry::new(
            Size::new(f64::from(w), f64::from(h)),
            1.0,
            None,
            scene.space,
        );
        for (scheme, palette, dark) in render_looks() {
            for (style, open) in [
                (MapStyle::Launch, false),
                (MapStyle::Launch, true),
                (MapStyle::Standard, false),
            ] {
                let mut surface =
                    gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w * 2, h * 2)
                        .unwrap();
                {
                    let cr = gtk::cairo::Context::new(&surface).unwrap();
                    let ground = palette.map.pane_opaque;
                    source(&cr, &ground, 1.0);
                    let _ = cr.paint();
                    cr.scale(2.0, 2.0);
                    field::draw(&cr, f64::from(w), f64::from(h), &palette.map, dark);
                    draw_arcs(&cr, &scene, &fit, &palette, true, 0.0);
                    for node in &scene.nodes {
                        draw_node(&cr, &scene, node, &fit, &palette, false, style);
                    }
                    if let Some(frame) = library_frame(&scene, &fit, style) {
                        LibraryArt::default()
                            .node(style, &frame, 0.0, true)
                            .draw(&cr);
                    }
                    map_transfers::resting_rocket(&cr, &scene, &fit, &palette, style);
                    let stack = scene.stack.as_ref().unwrap();
                    draw_pile(
                        &cr,
                        &scene,
                        stack,
                        &stack.tools,
                        &[],
                        None,
                        &fit,
                        &palette,
                        if open {
                            OPEN_PILE
                        } else {
                            PileStage::default()
                        },
                    );
                }
                surface.flush();
                let name = format!(
                    "{dir}/map-{scheme}-{}{}-{}x{}.bgra",
                    if open { "open" } else { "closed" },
                    if style == MapStyle::Standard {
                        "-standard"
                    } else {
                        ""
                    },
                    w * 2,
                    h * 2
                );
                std::fs::write(name, &*surface.data().unwrap()).unwrap();
            }
        }
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

    /// The legend's four rule items on one line beside the zoom at the
    /// map pane's width in the Omarchy screenshot (358 px), where "Not
    /// watched" had wrapped: the Mac's legend type (`TypeScale.micro`,
    /// regular, 10) and textTertiary, not the theme's larger caption.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_the_legend_fits_one_line_at_the_narrow_map_width() {
        gtk::init().expect("GTK display");
        crate::ui::style::install();
        let map = TraceMap::new();
        map.set_pending_read(true);
        map.set_projects_read(true);
        let window = gtk::Window::builder()
            .default_width(358)
            .default_height(560)
            .child(&map.root)
            .build();
        window.present();
        let canvas = map.canvas.clone();
        settle_until(move || canvas.width() > 0);
        let mut items = Vec::new();
        let mut stack = vec![map.root.clone().upcast::<gtk::Widget>()];
        while let Some(widget) = stack.pop() {
            // Each rule item is its dot and its word. The lock's memory
            // item shares the class and the flow but is not one of the
            // four rules, so it is left out of the count.
            let memory = widget
                .downcast_ref::<gtk::Label>()
                .is_some_and(|label| label.text() == memory_legend_word());
            if widget.has_css_class("tc-map-legend") && !memory {
                let item = widget.parent().expect("an item");
                if item.is_visible() {
                    items.push(item);
                }
            }
            let mut child = widget.first_child();
            while let Some(next) = child {
                stack.push(next.clone());
                child = next.next_sibling();
            }
        }
        assert_eq!(items.len(), 4, "Automatic, Ask, Never and Not watched");
        let first = items[0].clone();
        settle_until(move || first.height() > 0);
        let tops: Vec<f32> = items
            .iter()
            .map(|item| item.compute_bounds(&window).expect("laid out").y())
            .collect();
        assert!(
            tops.iter().all(|top| (top - tops[0]).abs() < 1.0),
            "the legend wrapped: {tops:?}"
        );
        window.close();
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

        // The Private AI view: its own map, with no legend or zoom. Unread,
        // the tool list draws nothing; read, each tool and the credential.
        map.show_view(MapView::PrivateAi);
        assert!(map.private_view.is_active());
        assert!(!map.foot.is_visible());
        // Before any read it is the Private AI map, drawing nothing.
        settle();
        assert!(map.private_canvas.scene().stack.is_none());
        assert!(!map.private_canvas.scene().draws_map());
        map.private_canvas.set_inputs(map.private_inputs());
        settle();
        assert!(map.private_canvas.node_buttons().is_empty());
        let list: HarnessList = serde_json::from_value(serde_json::json!({
            "harnesses": [{"id": "claude", "name": "Claude Code", "installed": true,
                           "state": "answering"}]
        }))
        .unwrap();
        map.set_harnesses(Some(&list));
        map.private_canvas.set_inputs(map.private_inputs());
        settle();
        let ids: Vec<String> = map
            .private_canvas
            .node_buttons()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, ["destination", "harness:claude"]);
        assert!(map.private_canvas.scene().flows(), "an answered call flows");
        assert!(
            !map.private_canvas.is_ticking(),
            "with animations off it is drawn still"
        );
        map.set_harnesses(None);
        map.private_canvas.set_inputs(map.private_inputs());
        settle();
        assert!(map.private_canvas.node_buttons().is_empty());
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

    /// The still layer compares by what it is drawn from: the same scene,
    /// geometry, selection and colours are the same picture; a zoom, a
    /// resize, a new selection or a new scheme draws it again.
    #[test]
    fn the_still_layer_is_keyed_by_what_it_draws() {
        let entries = vec![entry("a", "p", "codex")];
        let tree = TracesTree::build(&entries, &[], None);
        let scene = Scene::traces(
            &tree,
            &CommonsGate::with_state(ScreenState::Ready),
            &Contributions::unread(),
            None,
            &BTreeMap::new(),
            map_scene::SIZE,
        );
        let size = Size::new(580.0, 760.0);
        let fit = MapGeometry::new(size, 1.0, None, scene.space);
        let palette = Palette::fixed();
        let key = StillKey {
            scene: scene.clone(),
            fit,
            selection: None,
            palette: palette.clone(),
            resting: Some(PileStage::default()),
            style: MapStyle::Launch,
        };
        assert!(key.is(
            &scene,
            &fit,
            None,
            &palette,
            Some(PileStage::default()),
            MapStyle::Launch
        ));
        assert!(
            !key.is(
                &scene,
                &fit,
                None,
                &palette,
                Some(OPEN_PILE),
                MapStyle::Launch
            ),
            "the pile opened"
        );
        assert!(
            !key.is(&scene, &fit, None, &palette, None, MapStyle::Launch),
            "a run began: the pile moves"
        );
        assert!(
            !key.is(
                &scene,
                &fit,
                None,
                &palette,
                Some(PileStage::default()),
                MapStyle::Standard
            ),
            "the map's style"
        );
        assert_eq!(
            MapGeometry::new(size, 1.0, None, scene.space),
            fit,
            "the same field is the same geometry"
        );
        let zoomed = MapGeometry::new(size, 1.25, None, scene.space);
        assert_ne!(zoomed, fit);
        assert!(
            !key.is(
                &scene,
                &zoomed,
                None,
                &palette,
                Some(PileStage::default()),
                MapStyle::Launch
            ),
            "a zoom"
        );
        let wider = MapGeometry::new(Size::new(600.0, 760.0), 1.0, None, scene.space);
        assert!(
            !key.is(
                &scene,
                &wider,
                None,
                &palette,
                Some(PileStage::default()),
                MapStyle::Launch
            ),
            "a resize"
        );
        assert!(
            !key.is(
                &scene,
                &fit,
                Some("hub"),
                &palette,
                Some(PileStage::default()),
                MapStyle::Launch
            ),
            "a selection"
        );
        let mut dark = palette.clone();
        dark.ink = gtk::gdk::RGBA::WHITE;
        assert!(
            !key.is(
                &scene,
                &fit,
                None,
                &dark,
                Some(PileStage::default()),
                MapStyle::Launch
            ),
            "a scheme"
        );
        let other = Scene::traces(
            &TracesTree::build(&[], &[], None),
            &CommonsGate::with_state(ScreenState::Ready),
            &Contributions::unread(),
            None,
            &BTreeMap::new(),
            map_scene::SIZE,
        );
        assert!(
            !key.is(
                &other,
                &fit,
                None,
                &palette,
                Some(PileStage::default()),
                MapStyle::Launch
            ),
            "a new scene"
        );
    }

    /// A dismissal's card sits at the map's foot while another tab than
    /// Traces is shown, and its X closes it; under reduced motion the map's
    /// clock stops, no run is drawn, and the card still shows.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_map_shows_transfer_cards_and_stops_under_reduced_motion() {
        gtk::init().expect("GTK display");
        let settings = gtk::Settings::default().expect("settings");
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(true);
        let map = TraceMap::new();
        map.set_pending_read(true);
        map.set_projects_read(true);
        let window = gtk::Window::builder()
            .default_width(480)
            .default_height(700)
            .child(&map.root)
            .build();
        window.present();
        let listed = vec![entry("a", "p", "codex"), entry("b", "p", "codex")];
        map.canvas
            .set_inputs(map.inputs(&listed, &[], None, None, None));
        let canvas = map.canvas.clone();
        settle_until(move || canvas.width() > 0);

        // On Traces the map's foot shows no card.
        map.set_cards_shown(false);
        map.observe_transfers(&listed, None, None, None);
        let left = vec![entry("b", "p", "codex")];
        map.observe_transfers(&left, None, None, Some("a"));
        assert!(map.card_ids().is_empty(), "Traces shows its own");
        // Elsewhere it does, with the shared words, and nothing moves.
        map.set_cards_shown(true);
        settle();
        assert_eq!(map.card_ids(), ["a"]);
        assert!(map.transfers.cards.is_visible());
        assert!(map.canvas.run().is_none(), "a dismissal moves nothing");
        let card = map.transfers.cards.first_child().expect("the card");
        let close = find_button(&card, monitor_screens_copy().close).expect("its X");
        close.emit_clicked();
        settle();
        assert!(map.card_ids().is_empty(), "its X closes it");
        assert!(!map.transfers.cards.is_visible());

        // A contribution plays on the map, and the clock runs while it does.
        let first = vec![history_row("s1", "p", "codex", 100)];
        let second = vec![
            history_row("s2", "p", "codex", 200),
            history_row("s1", "p", "codex", 100),
        ];
        map.observe_transfers(&left, Some(&first[..]), None, Some("a"));
        map.observe_transfers(&left, Some(&second[..]), None, Some("a"));
        assert!(map.canvas.run().is_some(), "the new row plays");
        assert!(map.canvas.is_ticking());
        assert_eq!(map.card_ids(), ["s2"]);

        // At rest the station still turns, but only while the map is
        // shown: hidden, the clock stops; shown again, it runs.
        map.canvas.set_run(None);
        settle();
        assert!(map.canvas.is_ticking(), "the station turns at rest");
        // Hidden as its pane is collapsed: the whole map, not the canvas
        // alone, which the view stack would answer by showing the Private
        // AI canvas in its place and keeping it there.
        map.root.set_visible(false);
        settle();
        assert!(!map.canvas.is_mapped());
        assert!(!map.canvas.is_ticking(), "a hidden map's clock stops");
        map.root.set_visible(true);
        let canvas = map.canvas.clone();
        settle_until(move || canvas.is_mapped());
        assert!(map.canvas.is_ticking(), "shown again, it runs");

        // Reduced motion: the clock stops and no run is drawn; the card
        // stays.
        settings.set_gtk_enable_animations(false);
        settle();
        assert!(!map.canvas.is_ticking(), "the clock stops");
        assert!(map.canvas.imp().motion().is_none(), "no run is drawn");
        assert_eq!(map.card_ids(), ["s2"], "the card still shows");
        settings.set_gtk_enable_animations(previous);
        window.close();
    }

    /// A new card covers the front one, which fades where it is beneath
    /// it on the stack's spring, as a picture that takes no pointer or
    /// focus; the plates above are one widget for the stack's life and
    /// move on the same spring when the count changes; a card staying in
    /// front is not drawn anew; under reduced motion the covered card is
    /// gone at once (`GlassPopoverStack`).
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_a_covered_transfer_card_fades_under_the_new_one() {
        use crate::ui::popover_stack::Slide;
        gtk::init().expect("GTK display");
        let settings = gtk::Settings::default().expect("settings");
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(true);
        let map = TraceMap::new();
        map.set_pending_read(true);
        map.set_projects_read(true);
        let window = gtk::Window::builder()
            .default_width(480)
            .default_height(700)
            .child(&map.root)
            .build();
        window.present();
        let listed = vec![
            entry("a", "p", "codex"),
            entry("b", "p", "codex"),
            entry("c", "p", "codex"),
        ];
        map.canvas
            .set_inputs(map.inputs(&listed, &[], None, None, None));
        let canvas = map.canvas.clone();
        settle_until(move || canvas.width() > 0);
        map.set_cards_shown(true);
        map.observe_transfers(&listed, None, None, None);
        map.observe_transfers(&listed[1..], None, None, Some("a"));
        settle();
        assert_eq!(map.card_ids(), ["a"]);
        let first = map.transfers.front_cards();
        assert_eq!(first.len(), 1);
        let plates = map.transfers.plates.clone();
        assert_eq!(plates.behind(), 0);
        map.observe_transfers(&listed[2..], None, None, Some("b"));
        assert_eq!(map.card_ids(), ["b", "a"]);
        let cards = map.transfers.front_cards();
        assert_eq!(cards.len(), 1, "the covered card is drawn by the new one");
        assert_ne!(cards[0], first[0]);
        assert!(
            first[0].parent().is_none(),
            "the covered card's widget is gone"
        );
        assert_eq!(map.transfers.plates, plates, "the plates were made anew");
        assert_eq!(plates.behind(), 1);
        assert!(
            plates.moving(),
            "the plates did not move to their new count"
        );
        let front = cards[0].clone().downcast::<Slide>().expect("a slide");
        let seen = Rc::new(Cell::new(false));
        {
            let seen = seen.clone();
            let front = front.clone();
            settle_within(400, move || {
                let o = front.covered_opacity();
                if o > 0.05 && o < 0.95 {
                    seen.set(true);
                }
                seen.get()
            });
        }
        assert!(seen.get(), "the covered card never faded part-way");
        {
            let front = front.clone();
            settle_within(1500, move || front.covered_opacity() == 0.0);
        }
        assert_eq!(front.covered_opacity(), 0.0, "then it goes");
        // The front card stays itself while a card behind changes.
        map.set_cards_shown(true);
        assert_eq!(
            map.transfers.front_cards()[0],
            front.clone().upcast::<gtk::Widget>()
        );
        // Reduced motion: a covered card is gone at once.
        settings.set_gtk_enable_animations(false);
        map.observe_transfers(&[], None, None, Some("c"));
        assert_eq!(map.card_ids()[0], "c");
        let cards = map.transfers.front_cards();
        assert_eq!(cards.len(), 1);
        let front = cards[0].clone().downcast::<Slide>().expect("a slide");
        assert_eq!(front.covered_opacity(), 0.0);
        assert!(!map.transfers.plates.moving());
        settings.set_gtk_enable_animations(previous);
        window.close();
    }

    /// A card covered before it was ever drawn, in the same turn of the
    /// main loop that showed it, still fades beneath the new one on the
    /// stack's spring and is drawn while it does, as the Mac's removal
    /// transition runs whether or not a frame of the old card was shown.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_a_card_covered_before_it_is_drawn_still_fades() {
        use crate::ui::popover_stack::Slide;
        gtk::init().expect("GTK display");
        let settings = gtk::Settings::default().expect("settings");
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(true);
        let map = TraceMap::new();
        map.set_pending_read(true);
        map.set_projects_read(true);
        let window = gtk::Window::builder()
            .default_width(480)
            .default_height(700)
            .child(&map.root)
            .build();
        window.present();
        let listed = vec![entry("a", "p", "codex"), entry("b", "p", "codex")];
        map.canvas
            .set_inputs(map.inputs(&listed, &[], None, None, None));
        let canvas = map.canvas.clone();
        settle_until(move || canvas.width() > 0);
        map.set_cards_shown(true);
        map.observe_transfers(&listed, None, None, None);
        // "a" arrives and "b" covers it in one turn: no frame of "a".
        map.observe_transfers(&listed[1..], None, None, Some("a"));
        map.observe_transfers(&[], None, None, Some("b"));
        assert_eq!(map.card_ids(), ["b", "a"]);
        let cards = map.transfers.front_cards();
        assert_eq!(cards.len(), 1, "the covered card is drawn by the new one");
        let front = cards[0].clone().downcast::<Slide>().expect("a slide");
        assert_eq!(
            front.covered_opacity(),
            1.0,
            "the covered card starts whole"
        );
        let seen = Rc::new(Cell::new(false));
        {
            let seen = seen.clone();
            let front = front.clone();
            settle_within(400, move || {
                let o = front.covered_opacity();
                if o > 0.05 && o < 0.95 && front.drew_covered() {
                    seen.set(true);
                }
                seen.get()
            });
        }
        assert!(
            seen.get(),
            "the never-drawn covered card did not fade part-way"
        );
        {
            let front = front.clone();
            settle_within(1500, move || front.covered_opacity() == 0.0);
        }
        assert_eq!(front.covered_opacity(), 0.0, "then it goes");
        front.queue_draw();
        settle_within(200, || false);
        assert!(!front.drew_covered(), "and is no longer drawn");
        settings.set_gtk_enable_animations(previous);
        window.close();
    }

    /// Iterates the main loop until `done` holds or `millis` pass.
    fn settle_within(millis: u64, done: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(millis);
        let context = glib::MainContext::default();
        while !done() && std::time::Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Over its card the pile opens in the Mac's stages (taller at once,
    /// then wider, then the names) and closes the other way, names first;
    /// the Settings switch's style reaches the map and the runs to come.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_the_pile_opens_in_stages_and_the_style_switches_at_once() {
        gtk::init().expect("GTK display");
        let settings = gtk::Settings::default().expect("settings");
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(true);
        let map = TraceMap::new();
        map.set_pending_read(true);
        map.set_projects_read(true);
        let window = gtk::Window::builder()
            .default_width(480)
            .default_height(700)
            .child(&map.root)
            .build();
        window.present();
        let listed = vec![entry("a", "p", "codex"), entry("b", "p", "claude-code")];
        map.canvas
            .set_inputs(map.inputs(&listed, &[], None, None, None));
        let canvas = map.canvas.clone();
        settle_until(move || canvas.width() > 0);
        let imp = map.canvas.imp();
        let scene = map.canvas.scene();
        let stack = scene.stack.as_ref().expect("the pile");
        let (x, y, w, h) = stack.card(&imp.geometry(), false, false);
        imp.point_at(Some((x + w / 2.0, y + h / 2.0)));
        let tall = PileStage {
            tall: true,
            ..PileStage::default()
        };
        assert_eq!(imp.pile_stage.get(), tall, "taller at once, no wider");
        settle_within(2000, || imp.pile_stage.get().names);
        assert_eq!(imp.pile_stage.get(), OPEN_PILE);
        imp.point_at(None);
        assert!(!imp.pile_stage.get().names, "the names go at once");
        assert!(imp.pile_stage.get().wide, "the width waits");
        settle_within(2000, || imp.pile_stage.get() == PileStage::default());
        assert_eq!(imp.pile_stage.get(), PileStage::default());

        assert_eq!(
            map.canvas.style(),
            MapStyle::Launch,
            "the rocket by default"
        );
        map.set_style(MapStyle::Standard);
        assert_eq!(map.canvas.style(), MapStyle::Standard);
        assert_eq!(map.transfers.player.borrow().style, MapStyle::Standard);
        settings.set_gtk_enable_animations(previous);
        window.close();
    }

    /// The pile, the light and the shimmer move on the frame clock at the
    /// display's rate while they move and stop it when done; under
    /// reduced motion each is at its end at once, followed live; the
    /// shimmer is due only while the map is shown with motion on.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_the_pile_light_and_shimmer_move_on_the_clock_and_stop() {
        gtk::init().expect("GTK display");
        let settings = gtk::Settings::default().expect("settings");
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(true);
        let map = TraceMap::new();
        map.set_pending_read(true);
        map.set_projects_read(true);
        let window = gtk::Window::builder()
            .default_width(480)
            .default_height(700)
            .child(&map.root)
            .build();
        window.present();
        let listed = vec![entry("a", "p", "codex"), entry("b", "p", "claude-code")];
        map.canvas
            .set_inputs(map.inputs(&listed, &[], None, None, None));
        let canvas = map.canvas.clone();
        settle_until(move || canvas.width() > 0);
        assert!(map.canvas.shimmer_scheduled(), "the first pass is due");
        let imp = map.canvas.imp();
        let scene = map.canvas.scene();
        let stack = scene.stack.as_ref().expect("the pile");
        let (x, y, w, h) = stack.card(&imp.geometry(), false, false);
        imp.point_at(Some((x + w / 2.0, y + h / 2.0)));
        assert!(map.canvas.is_ticking());
        assert_eq!(
            map.canvas.clock_interval(),
            0.0,
            "every frame while it moves"
        );
        let seen_between = Rc::new(Cell::new(false));
        {
            let seen = seen_between.clone();
            let canvas = map.canvas.clone();
            settle_within(400, move || {
                let tall = canvas
                    .imp()
                    .pile_motion
                    .borrow()
                    .tall
                    .value(canvas.imp().now());
                if tall > 0.05 && tall < 0.95 {
                    seen.set(true);
                }
                seen.get()
            });
        }
        assert!(seen_between.get(), "the card was never part-way open");
        assert!(imp.light.borrow().1.target() == 1.0, "the light is in");
        {
            let canvas = map.canvas.clone();
            settle_within(3000, move || {
                !canvas.imp().pile_motion.borrow().moving(canvas.imp().now())
            });
        }
        assert!(!imp.pile_motion.borrow().moving(imp.now()));
        // Reduced motion: the pile closes at once and no shimmer is due.
        settings.set_gtk_enable_animations(false);
        assert!(!map.canvas.shimmer_scheduled());
        imp.point_at(None);
        assert!(!imp.pile_motion.borrow().moving(imp.now()));
        assert_eq!(imp.light.borrow().1.value(imp.now()), 0.0);
        settle_within(1000, || imp.pile_stage.get() == PileStage::default());
        assert_eq!(imp.pile_motion.borrow().tall.value(imp.now()), 0.0);
        // Motion back on: the shimmer is due again; hidden, it is not.
        settings.set_gtk_enable_animations(true);
        assert!(map.canvas.shimmer_scheduled());
        map.root.set_visible(false);
        assert!(!map.canvas.shimmer_scheduled(), "none while hidden");
        assert!(!map.canvas.is_ticking());
        settings.set_gtk_enable_animations(previous);
        window.close();
    }

    fn history_row(id: &str, project: &str, source: &str, at: i64) -> HistoryRecord {
        HistoryRecord {
            submission_id: id.into(),
            project_id: project.into(),
            project_label: project.into(),
            source: Some(source.into()),
            submitted_at: chrono::DateTime::from_timestamp(at, 0),
            ..Default::default()
        }
    }

    /// The first button under `root` whose accessible name or tooltip is
    /// `name`.
    fn find_button(root: &gtk::Widget, name: &str) -> Option<gtk::Button> {
        let mut stack = vec![root.clone()];
        while let Some(widget) = stack.pop() {
            if let Some(button) = widget.downcast_ref::<gtk::Button>()
                && button.tooltip_text().as_deref() == Some(name)
            {
                return Some(button.clone());
            }
            let mut child = widget.first_child();
            while let Some(c) = child {
                child = c.next_sibling();
                stack.push(c);
            }
        }
        None
    }
}
