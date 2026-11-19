//! What the Traces map draws, as pure data.
//!
//! A native port of `FlowMapScene` in
//! `macos/Sources/TraceCommonsApp/Views/Monitor/FlowMapScene.swift`, with the
//! layout arithmetic `FlowMapView.swift` and `MapToolPile.swift` keep beside
//! their drawing (`FlowMapGeometry`, `cardX`, `labelType`, the pile's card).
//! Nodes, the arcs between them, and what each node says when inspected,
//! laid out in #1146's 580x760 design space reshaped to the pane. No widget
//! is built here, so the layout and its words are tested without a display.
//!
//! Only what the core reports is drawn. A node's words are a name the core
//! reports, a single word, a number, or core copy
//! (`MonitorFlowMapCopy`, `MonitorScreensCopy`).
//!
//! The Private AI map is here too (`privateAI`, `addingManagedRoutes`):
//! every tool that can send model calls here, joined to the credential
//! they are answered with, drawn from `harness_list`, `tool_destinations`
//! and `status`, the reads the window already makes.

use std::collections::BTreeMap;

use trace_commons_contributor::preview_copy::{MonitorFlowMapCopy, monitor_screens_copy};

use super::graph_model::CONTRIBUTED_STATUSES;
use super::traces_tree::{FolderNode, ProjectMode, SourceKind, SourceMode, ToolNode, TracesTree};
use trace_commons_contributor::harness_state::HarnessState;

use crate::model::{GateStatus, HistoryRecord, ToolDestinations};

/// A point in the design space or the field.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

impl Size {
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    pub fn contains(self, p: Point) -> bool {
        p.x >= 0.0 && p.y >= 0.0 && p.x <= self.width && p.y <= self.height
    }
}

/// #1146's design space.
pub const SIZE: Size = Size::new(580.0, 760.0);

/// The Private AI destination's state. Unknown is drawn apart from off: a
/// missing signal is never "off".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lamp {
    Answering,
    Running,
    Off,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// This computer: where sessions are collected.
    Hub,
    /// The commons library sessions are contributed to.
    Library { active: bool },
    /// A watched tool. `off` crosses it out; unset draws it plain.
    Tool { kind: SourceKind, off: bool },
    /// A folder, ringed by its rule (`None`: the core has not listed it).
    Folder(Option<ProjectMode>),
    /// A tool that can send model calls here (the Private AI map).
    Harness {
        tool: Option<SourceKind>,
        installed: bool,
    },
    /// The Private AI destination tools connect to.
    Destination(Lamp),
    /// Another device linked to the same account.
    Device,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub at: Point,
    pub radius: f64,
    /// Drawn beside or under the node; also its accessible name.
    pub label: String,
    /// The inspection card's detail line, and the node's accessible
    /// description.
    pub detail: String,
    pub dim: f64,
    /// The card's title when it is not the label (a tool's adds its folder
    /// count).
    pub title: Option<String>,
    /// A tool that is not watched: drawn with a dashed rim.
    pub dashed: bool,
    /// Ringed as the selection's tool.
    pub ringed: bool,
    /// A second line under the label.
    pub sublabel: Option<String>,
    /// A tool's traces waiting, for its coin's line.
    pub waiting: Option<usize>,
}

impl Node {
    pub fn new(
        id: &str,
        kind: NodeKind,
        at: Point,
        radius: f64,
        label: &str,
        detail: &str,
    ) -> Self {
        Self {
            id: id.to_string(),
            kind,
            at,
            radius,
            label: label.to_string(),
            detail: detail.to_string(),
            dim: 1.0,
            title: None,
            dashed: false,
            ringed: false,
            sublabel: None,
            waiting: None,
        }
    }

    /// Whether the label sits to the left of the node, on its line, rather
    /// than under it: folders and the Private AI tools, the nodes that come
    /// in long lists.
    pub fn label_beside(&self) -> bool {
        matches!(self.kind, NodeKind::Folder(_) | NodeKind::Harness { .. })
    }

    pub fn card_title(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.label)
    }
}

/// How many traces `list_history` is asked for (`ui::history`). A page
/// that came back this full may have been cut, so it is not a whole count.
pub const HISTORY_LIMIT: usize = crate::ui::history::HISTORY_LIMIT;

/// What has been contributed from this machine, from History's rows that
/// left and still stand. `None` counts while History is unread, after a
/// failed read (`App::history_records` is then `None`), or when the page
/// came back full: an unread or partial record is never said as nothing
/// contributed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contributions {
    pub total: Option<usize>,
    pub by_tool: BTreeMap<String, usize>,
    pub by_folder: BTreeMap<String, usize>,
}

impl Contributions {
    pub fn unread() -> Self {
        Self::default()
    }

    pub fn counted(total: usize, by_tool: &[(&str, usize)]) -> Self {
        Self {
            total: Some(total),
            by_tool: by_tool.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            by_folder: BTreeMap::new(),
        }
    }

    pub fn from_history(history: Option<&[HistoryRecord]>) -> Self {
        let Some(history) = history.filter(|rows| rows.len() < HISTORY_LIMIT) else {
            return Self::unread();
        };
        let mut counted = Self {
            total: Some(0),
            ..Self::default()
        };
        for row in history
            .iter()
            .filter(|row| CONTRIBUTED_STATUSES.contains(&row.status.as_str()))
        {
            counted.total = counted.total.map(|n| n + 1);
            if let Some(tool) = row
                .source
                .as_deref()
                .filter(|tool| SourceKind::from_wire(tool).is_some())
            {
                *counted.by_tool.entry(tool.to_string()).or_default() += 1;
            }
            if !row.project_id.is_empty() {
                *counted.by_folder.entry(row.project_id.clone()).or_default() += 1;
            }
        }
        counted
    }

    pub fn tool(&self, kind: SourceKind) -> Option<usize> {
        self.total
            .map(|_| self.by_tool.get(kind.wire()).copied().unwrap_or(0))
    }

    pub fn folder(&self, id: &str) -> Option<usize> {
        self.total
            .map(|_| self.by_folder.get(id).copied().unwrap_or(0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArcStyle {
    /// A quiet link: folder to tools, tools to this computer.
    Quiet,
    /// Configured but not carrying anything.
    Dashed,
    /// Carrying sessions: moving dashes, in the status-on colour.
    Flowing,
    /// Calls arrived in this tool's family but cannot be said to be this
    /// tool's: lit, never moving.
    Shared,
    /// The core could not say: dotted, never solid or moving.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arc {
    pub from: Point,
    pub control1: Point,
    pub control2: Point,
    pub to: Point,
    pub style: ArcStyle,
    pub dim: f64,
}

/// A count of nodes not drawn, as "+N".
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Overflow {
    pub at: Point,
    pub count: usize,
}

/// The pill under the folder column: "+N" while some are not drawn, and a
/// click draws the next [`FOLDERS_REVEALED`]; once a reveal has drawn them
/// all, a pill that folds them back.
#[derive(Clone, Debug, PartialEq)]
pub struct FolderReveal {
    /// The key `revealed` keeps this pill's reveal under.
    pub tool: String,
    /// The folder column on the pill's line: the pill ends where the names
    /// do, left of the dots.
    pub at: Point,
    /// The folders not drawn; 0 for the pill that folds them back.
    pub hidden: usize,
    pub dim: f64,
}

impl FolderReveal {
    /// How many folders a click draws.
    pub fn next(&self) -> usize {
        self.hidden.min(FOLDERS_REVEALED)
    }
}

/// The tool group between the folders and this computer: the tools as a
/// pile of coins, the front coin first, on a card the folders' lines meet
/// at the middle of its left side and the line to the pad leaves from the
/// middle of its right. Under the pointer the pile opens and the card
/// widens for the tools' names.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolStack {
    pub center: Point,
    /// The tools' node ids, the front of the pile first.
    pub tools: Vec<String>,
}

impl ToolStack {
    pub const COIN: f64 = 46.0;
    pub const WIDTH: f64 = 66.0;
    pub const OPEN_WIDTH: f64 = 172.0;
    pub const STEP: f64 = 9.0;
    pub const OPEN_STEP: f64 = 34.0;
    pub const PADDING: f64 = 11.0;

    pub fn port_left(&self) -> Point {
        Point::new(self.center.x - Self::WIDTH / 2.0, self.center.y)
    }

    pub fn port_right(&self) -> Point {
        Point::new(self.center.x + Self::WIDTH / 2.0, self.center.y)
    }

    /// The card's height, closed or open.
    pub fn height(&self, open: bool) -> f64 {
        let step = if open { Self::OPEN_STEP } else { Self::STEP };
        (self.tools.len().max(1) - 1) as f64 * step + Self::COIN + 2.0 * Self::PADDING
    }

    /// Where the coin at `index` sits, closed or open.
    pub fn coin_center(&self, index: usize, open: bool) -> Point {
        let step = if open { Self::OPEN_STEP } else { Self::STEP };
        let middle = (self.tools.len() as f64 - 1.0) / 2.0;
        Point::new(
            self.center.x,
            self.center.y + (index as f64 - middle) * step,
        )
    }

    /// The card's height `tall` of the way from closed to open (past
    /// either as its spring overshoots).
    pub fn height_at(&self, tall: f64) -> f64 {
        let (closed, open) = (self.height(false), self.height(true));
        closed + (open - closed) * tall
    }

    /// Where the coin at `index` (fractional while it moves) sits, its
    /// pile `lay` of the way from closed to open.
    pub fn coin_center_at(&self, index: f64, lay: f64) -> Point {
        let step = Self::STEP + (Self::OPEN_STEP - Self::STEP) * lay;
        let middle = (self.tools.len() as f64 - 1.0) / 2.0;
        Point::new(self.center.x, self.center.y + (index - middle) * step)
    }

    /// The card's rectangle `tall` and `wide` of the way open.
    pub fn card_at(&self, fit: &MapGeometry, tall: f64, wide: f64) -> (f64, f64, f64, f64) {
        let s = fit.scale;
        let height = self.height_at(tall) * s;
        let width = (Self::WIDTH + (Self::OPEN_WIDTH - Self::WIDTH) * wide) * s;
        let left = fit.point(self.port_left());
        (left.x, left.y - height / 2.0, width, height)
    }

    /// The card's rectangle in the field, `(x, y, width, height)`: the
    /// pile opens only while the pointer is over this.
    pub fn card(&self, fit: &MapGeometry, open: bool, wide: bool) -> (f64, f64, f64, f64) {
        let s = fit.scale;
        let height = self.height(open) * s;
        let width = if wide { Self::OPEN_WIDTH } else { Self::WIDTH } * s;
        let left = fit.point(self.port_left());
        (left.x, left.y - height / 2.0, width, height)
    }
}

/// The one state-precedence rule the Mac's screens share (`ScreenState`):
/// only `Ready` may draw anything as moving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenState {
    CoreDown,
    Loading,
    Paused,
    Unknown,
    Ready,
}

impl ScreenState {
    /// `failed`: the screen's last read failed. `loaded`: it has read
    /// anything yet. `paused`: watching paused, `None` when unsaid.
    /// `known`: every signal the screen needs was reported.
    ///
    /// The GTK worker does not tell an unreachable core from another
    /// failure, so a failed read is `Unknown` here where the Mac may say
    /// `CoreDown`; neither draws a flow.
    pub fn resolve(failed: bool, loaded: bool, paused: Option<bool>, known: bool) -> Self {
        if failed {
            return ScreenState::Unknown;
        }
        if !loaded {
            return ScreenState::Loading;
        }
        if paused == Some(true) {
            return ScreenState::Paused;
        }
        if paused.is_none() || !known {
            return ScreenState::Unknown;
        }
        ScreenState::Ready
    }

    pub fn is_healthy(self) -> bool {
        self == ScreenState::Ready
    }
}

/// What decides whether sessions move to the commons, as the core reports
/// it. Anything unread is closed: a missing signal never draws a flow.
#[derive(Clone, Debug, PartialEq)]
pub struct CommonsGate {
    pub state: ScreenState,
    pub status: Option<GateStatus>,
    pub destinations: Option<ToolDestinations>,
}

impl CommonsGate {
    /// The `sessions_route` values with somewhere to send sessions.
    pub const ROUTES: [&'static str; 2] = ["witness", "local"];

    pub fn with_state(state: ScreenState) -> Self {
        Self {
            state,
            status: None,
            destinations: None,
        }
    }

    /// Whether anything at all can move to the commons now: the screen is
    /// current, nothing is paused, held or voided, the contributor is
    /// signed in, and the core names a route for sessions.
    pub fn open(&self) -> bool {
        let (ScreenState::Ready, Some(status), Some(destinations)) =
            (self.state, &self.status, &self.destinations)
        else {
            return false;
        };
        status.paused == Some(false)
            && status.logged_in == Some(true)
            && status
                .automatic_contribution_held
                .as_ref()
                .and_then(|held| held.held_sessions)
                == Some(0)
            && status.grant_voids.as_ref().is_some_and(Vec::is_empty)
            && destinations
                .sessions_route
                .as_deref()
                .is_some_and(|route| Self::ROUTES.contains(&route))
    }

    /// Whether the core says this tool's sessions are watched and go to the
    /// commons.
    pub fn sends_to_commons(&self, kind: SourceKind) -> bool {
        self.destinations
            .as_ref()
            .and_then(|d| d.tools.iter().find(|t| t.tool == kind.wire()))
            .and_then(|t| t.sessions.as_ref())
            .is_some_and(|route| {
                route.watch == "watched" && route.to.iter().any(|to| to == "commons")
            })
    }
}

/// Another device linked to this account, named and dated as the Devices
/// section lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct OtherDevice {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub contributed: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub nodes: Vec<Node>,
    pub arcs: Vec<Arc>,
    pub overflows: Vec<Overflow>,
    pub reveals: Vec<FolderReveal>,
    /// The Traces map's tool group; `None` on the Private AI map.
    pub stack: Option<ToolStack>,
    /// Where the + that links another device sits; `None` when the map
    /// does not offer linking.
    pub add_device: Option<Point>,
    /// The design space this scene is laid out in.
    pub space: Size,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            arcs: Vec::new(),
            overflows: Vec::new(),
            reveals: Vec::new(),
            stack: None,
            add_device: None,
            space: SIZE,
        }
    }
}

// --- Layout constants ----------------------------------------------------

/// A folder line's least and most height.
pub const FOLDER_STEP_MIN: f64 = 22.0;
pub const FOLDER_STEP_MAX: f64 = 34.0;
/// How many folders the column draws before its "+N" pill.
pub const FOLDERS_SHOWN: usize = 10;
/// A folder dot's radius.
pub const FOLDER_RADIUS: f64 = 7.0;
/// How many more folders one click of the "+N" pill draws.
pub const FOLDERS_REVEALED: usize = 6;
/// The key `revealed` keeps the folder column's reveal under.
pub const FOLDER_COLUMN: &str = "folders";
/// Half the launch pad's width: the line out lands on its left edge.
pub const PAD_HALF_WIDTH: f64 = 32.0;
/// How many other devices the map draws before counting the rest.
pub const DEVICES_SHOWN: usize = 4;
/// How far below this computer's point the first other device may sit.
pub const DEVICE_CLEARANCE: f64 = 72.0;

/// The words this map says, from the core.
pub fn words() -> MonitorFlowMapCopy {
    monitor_screens_copy().flow_map
}

impl Scene {
    /// A design space with the pane's proportions and #1146's area, so the
    /// map fills the pane rather than letterboxing, at much the same
    /// scale. Very tall or very wide panes are held to a sensible shape.
    pub fn space_for(pane: Size) -> Size {
        if pane.width <= 0.0 || pane.height <= 0.0 {
            return SIZE;
        }
        let aspect = (pane.width / pane.height).clamp(0.42, 1.7);
        let area = SIZE.width * SIZE.height;
        Size::new((area * aspect).sqrt(), (area / aspect).sqrt())
    }

    /// The band nodes are laid out in: clear of the view switch at the top
    /// and the legend and zoom at the bottom.
    pub fn band(space: Size) -> (f64, f64) {
        (space.height * 0.13, space.height * 0.87)
    }

    /// Positions for `weights` stacked down the band, each given room in
    /// proportion to its weight, no more than `max_step` per unit and at
    /// least `min_step`, the group centred on the band.
    pub fn stack(
        weights: &[f64],
        band: (f64, f64),
        max_step: f64,
        min_step: f64,
    ) -> Vec<(f64, f64)> {
        if weights.is_empty() {
            return Vec::new();
        }
        let units: f64 = weights.iter().sum();
        let length = band.1 - band.0;
        let step = min_step.max(max_step.min(length / units.max(1.0)));
        let total = step * units;
        let mut y = (band.0 + band.1) / 2.0 - total / 2.0;
        weights
            .iter()
            .map(|weight| {
                let room = step * weight;
                let centre = y + room / 2.0;
                y += room;
                (centre, room)
            })
            .collect()
    }

    /// Anything moving, so the clock can stop when nothing is.
    pub fn flows(&self) -> bool {
        self.arcs.iter().any(|arc| arc.style == ArcStyle::Flowing)
    }

    pub fn folder_x(space: Size) -> f64 {
        (space.width * 0.3).max(124.0)
    }

    pub fn tool_x(space: Size) -> f64 {
        space.width * 0.5
    }

    pub fn hub(space: Size) -> Point {
        let band = Self::band(space);
        Point::new(space.width * 0.72, (band.0 + band.1) / 2.0)
    }

    /// The library sits straight above this computer.
    pub fn library(space: Size) -> Point {
        Point::new(
            Self::hub(space).x,
            Self::band(space).0 + space.height * 0.04,
        )
    }

    /// The folders the column's pill has drawn after a click on `reveal`:
    /// the next few, or, from the pill that folds them back, none.
    pub fn revealing(
        reveal: &FolderReveal,
        revealed: &BTreeMap<String, usize>,
    ) -> BTreeMap<String, usize> {
        let mut revealed = revealed.clone();
        if reveal.hidden > 0 {
            *revealed.entry(reveal.tool.clone()).or_default() += reveal.next();
        } else {
            revealed.remove(&reveal.tool);
        }
        revealed
    }

    /// The design space grown, its shape kept, until the band holds
    /// `units` lines at the least line height: folders a pill has drawn
    /// past what fits zoom the map out rather than run off it.
    pub fn space_holding(space: Size, units: f64) -> Size {
        let band = Self::band(space);
        let length = band.1 - band.0;
        let needed = units * FOLDER_STEP_MIN;
        if length <= 0.0 || needed <= length {
            return space;
        }
        let factor = needed / length;
        Size::new(space.width * factor, space.height * factor)
    }

    /// The folders the column draws: the tree's folders; a tree that lists
    /// none gives its tools' folders and the unplaced ones, each once.
    pub fn directories(tree: &TracesTree) -> Vec<FolderNode> {
        if !tree.folders.is_empty() {
            return tree.folders.clone();
        }
        let mut seen = std::collections::HashSet::new();
        tree.tools
            .iter()
            .flat_map(|tool| tool.folders.iter())
            .chain(tree.unplaced.iter())
            .filter(|folder| seen.insert(folder.id.clone()))
            .cloned()
            .collect()
    }

    /// The tools a folder's traces come from: the tools the tree places it
    /// under, and the sources of the traces waiting in it.
    pub fn carriers<'a>(folder: &FolderNode, tree: &'a TracesTree) -> Vec<&'a ToolNode> {
        tree.tools
            .iter()
            .filter(|tool| {
                tool.folders.iter().any(|f| f.id == folder.id)
                    || folder.sessions.iter().any(|s| s.source == tool.id())
            })
            .collect()
    }

    /// Every tool this build knows, the tree's first, then the ones the
    /// tree leaves out as not watched.
    pub fn map_tools(tree: &TracesTree) -> Vec<(ToolNode, bool)> {
        let mut tools: Vec<(ToolNode, bool)> =
            tree.tools.iter().map(|t| (t.clone(), true)).collect();
        for kind in SourceKind::ALL {
            if !tree.tools.iter().any(|t| t.kind == kind) {
                tools.push((ToolNode::new(kind, SourceMode::Unset, Vec::new()), false));
            }
        }
        tools
    }

    /// The Traces map: each folder once, ringed by its rule, with one line
    /// into the tool group; the tools as a pile of coins on that group's
    /// card; one line out to this computer's launch pad; and the library
    /// above. Nothing joins the pad to the library.
    ///
    /// A folder set to contribute automatically, under a tool that is not
    /// off, has a line that moves when the gate is open and the core says
    /// that tool's sessions go to the commons, and is dashed otherwise; the
    /// line out moves when any folder's does. The library is lit once
    /// anything has been contributed, or while anything moves.
    ///
    /// `selected_tool` is ringed, and the other tools and the folders it
    /// has no traces in are faded. `revealed` is how many folders past the
    /// column's cap its pill has drawn.
    pub fn traces(
        tree: &TracesTree,
        gate: &CommonsGate,
        contributed: &Contributions,
        selected_tool: Option<&str>,
        revealed: &BTreeMap<String, usize>,
        space: Size,
    ) -> Scene {
        let words = words();
        let screens = monitor_screens_copy();
        let mut scene = Scene::default();
        let tools: Vec<(ToolNode, bool)> = Self::map_tools(tree)
            .into_iter()
            .filter(|(tool, watched)| {
                let Some(destinations) = gate.destinations.as_ref() else {
                    return true;
                };
                if !destinations.tools.iter().any(|t| t.installed == Some(true)) {
                    return true;
                }
                // Watched and imported sources, and contribution history,
                // stay drawn after an uninstall.
                if *watched || contributed.tool(tool.kind).unwrap_or(0) > 0 {
                    return true;
                }
                if destinations
                    .managed_sessions
                    .as_ref()
                    .is_some_and(|rows| rows.iter().any(|row| row.source() == tool.id()))
                {
                    return true;
                }
                let row = destinations.tools.iter().find(|t| t.tool == tool.id());
                row.and_then(|r| r.installed) != Some(false)
                    || row
                        .and_then(|r| r.counts.as_ref())
                        .and_then(|c| c.sessions)
                        .unwrap_or(0)
                        > 0
            })
            .collect();
        let folders = Self::directories(tree);
        let shown = folders
            .len()
            .min(FOLDERS_SHOWN + revealed.get(FOLDER_COLUMN).copied().unwrap_or(0));
        let pill = folders.len() > FOLDERS_SHOWN;
        let lines = shown + usize::from(pill);
        let space = Self::space_holding(space, lines as f64);
        scene.space = space;
        let hub = Self::hub(space);
        let library = Self::library(space);
        let folder_x = Self::folder_x(space);
        let band = Self::band(space);
        let group = ToolStack {
            center: Point::new(Self::tool_x(space), (band.0 + band.1) / 2.0),
            tools: tools
                .iter()
                .map(|(t, _)| format!("tool:{}", t.id()))
                .collect(),
        };

        let slots = Self::stack(&vec![1.0; lines], band, FOLDER_STEP_MAX, FOLDER_STEP_MIN);
        let mut flowing = false;
        for (index, folder) in folders.iter().take(shown).enumerate() {
            let point = Point::new(folder_x, slots[index].0);
            let carriers = Self::carriers(folder, tree);
            let live: Vec<_> = carriers
                .iter()
                .filter(|t| t.mode != SourceMode::Off)
                .collect();
            let armed = folder.mode == Some(ProjectMode::AutoUpload) && !live.is_empty();
            let moves = armed && gate.open() && live.iter().any(|t| gate.sends_to_commons(t.kind));
            flowing |= moves;
            let faded = selected_tool.is_some_and(|id| !carriers.iter().any(|t| t.id() == id));
            let dim = if faded { 0.3 } else { 1.0 };
            let style = if moves {
                ArcStyle::Flowing
            } else if armed {
                ArcStyle::Dashed
            } else {
                ArcStyle::Quiet
            };
            scene.arcs.push(curve(
                Point::new(point.x + FOLDER_RADIUS + 1.0, point.y),
                group.port_left(),
                style,
                dim,
            ));
            let mut node = Node::new(
                &format!("folder:{}", folder.id),
                NodeKind::Folder(folder.mode),
                point,
                FOLDER_RADIUS,
                &folder.label,
                &folder_detail(&words, folder, contributed.folder(&folder.id)),
            );
            node.dim = dim;
            scene.nodes.push(node);
        }
        if pill && let Some(last) = slots.last() {
            // A line under the last name, in the names' column.
            scene.reveals.push(FolderReveal {
                tool: FOLDER_COLUMN.to_string(),
                at: Point::new(folder_x, last.0),
                hidden: folders.len() - shown,
                dim: 1.0,
            });
        }
        // One line out of the group, to the pad's left edge.
        scene.arcs.push(curve(
            group.port_right(),
            Point::new(hub.x - PAD_HALF_WIDTH, hub.y),
            if flowing {
                ArcStyle::Flowing
            } else {
                ArcStyle::Quiet
            },
            1.0,
        ));

        for (index, (tool, watched)) in tools.iter().enumerate() {
            let off = tool.mode == SourceMode::Off;
            let faded = selected_tool.is_some_and(|id| id != tool.id());
            let detail =
                tool_detail(&words, tool) + &managed_detail(tool.id(), gate.destinations.as_ref());
            let mut node = Node::new(
                &format!("tool:{}", tool.id()),
                NodeKind::Tool {
                    kind: tool.kind,
                    off,
                },
                group.coin_center(index, false),
                ToolStack::COIN / 2.0,
                tool.kind.display_name(),
                &detail,
            );
            node.dim = if off { 0.4 } else { 1.0 } * if faded { 0.3 } else { 1.0 };
            node.title = Some(tool_title(&words, tool));
            node.dashed = !watched;
            node.ringed = selected_tool == Some(tool.id());
            node.waiting = Some(tool.waiting());
            scene.nodes.push(node);
        }
        scene.stack = Some(group);

        let tools_waiting: usize = tree.tools.iter().map(ToolNode::waiting).sum();
        let unplaced_waiting: usize = tree.unplaced.iter().map(|f| f.sessions.len()).sum();
        let waiting = tools_waiting + unplaced_waiting;
        scene.nodes.insert(
            0,
            Node::new(
                "hub",
                NodeKind::Hub,
                hub,
                16.0,
                screens.computer,
                &hub_words(&words, waiting, contributed.total),
            ),
        );
        scene.nodes.push(Node::new(
            "library",
            NodeKind::Library {
                active: flowing || contributed.total.unwrap_or(0) > 0,
            },
            library,
            22.0,
            screens.commons,
            &library_words(&words, contributed.total),
        ));
        scene
    }

    /// The camera's centre for the binoculars: the tool's node, `None`
    /// when it is not drawn.
    pub fn focus_point(&self, tool: Option<&str>) -> Option<Point> {
        let id = format!("tool:{}", tool?);
        self.nodes.iter().find(|n| n.id == id).map(|n| n.at)
    }

    /// The other devices, in one column directly below this computer, with
    /// no line to the commons: the map's lines stay this computer's. Past
    /// [`DEVICES_SHOWN`] the rest are counted as "+N". With `offers_link`,
    /// the column ends in a + that starts linking another device. A scene
    /// with no commons is returned as it is.
    pub fn adding_devices(&self, devices: &[OtherDevice], offers_link: bool) -> Scene {
        let Some(hub) = self.nodes.iter().find(|n| n.id == "hub") else {
            return self.clone();
        };
        if (devices.is_empty() && !offers_link) || !self.nodes.iter().any(|n| n.id == "library") {
            return self.clone();
        }
        let mut scene = self.clone();
        let band = Self::band(self.space);
        let shown = &devices[..devices.len().min(DEVICES_SHOWN)];
        let more = devices.len() - shown.len();
        let top = hub.at.y + DEVICE_CLEARANCE;
        let lines = shown.len() as f64 + if more > 0 { 0.6 } else { 0.0 };
        let room = (band.1 - top).max(1.0);
        let units = lines.ceil() + if offers_link { 1.0 } else { 0.0 };
        let floor: f64 = if offers_link { 24.0 } else { 40.0 };
        let step = 58.0_f64.min(floor.max(room / units.max(1.0)));
        for (index, device) in shown.iter().enumerate() {
            scene.nodes.push(Node::new(
                &format!("device:{}", device.id),
                NodeKind::Device,
                Point::new(hub.at.x, top + step * index as f64),
                12.0,
                &device.name,
                &device.detail,
            ));
        }
        // What another device contributed stands in the same commons.
        if devices.iter().any(|d| d.contributed.unwrap_or(0) > 0)
            && let Some(library) = scene.nodes.iter_mut().find(|n| n.id == "library")
        {
            library.kind = NodeKind::Library { active: true };
        }
        if more > 0 {
            scene.overflows.push(Overflow {
                at: Point::new(hub.at.x, top + step * shown.len() as f64 - step * 0.4),
                count: more,
            });
        }
        if offers_link {
            let slot = shown.len() as f64 + if more > 0 { 0.7 } else { 0.0 };
            scene.add_device = Some(Point::new(hub.at.x, (top + step * slot).min(band.1)));
        }
        scene
    }
}

// --- The Private AI map ----------------------------------------------------

/// One `harness_list` row as the Private AI map reads it: the core's id,
/// name, whether the tool is on this computer, and its state label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessRow {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub state: String,
}

impl HarnessRow {
    pub fn from_model(row: &crate::model::Harness) -> Self {
        Self {
            id: row.id.clone(),
            name: row.name.clone(),
            installed: row.installed,
            state: row.state.clone(),
        }
    }
}

/// A Private AI tool line's least and most height.
pub const HARNESS_STEP_MIN: f64 = 30.0;
pub const HARNESS_STEP_MAX: f64 = 72.0;

/// The core's verdict for a row, from its label. A label this build has
/// never heard of is unknown, as `tc_harness_state_code` answers it.
pub fn harness_state(label: &str) -> HarnessState {
    HarnessState::from_label(label).unwrap_or(HarnessState::Unknown)
}

/// The sentence a row's card says (`MonitorWindowView.rowSentence`): a
/// tool that is not on this computer says so and nothing else; a state
/// with no sentence of its own says nothing, never a borrowed one.
pub fn row_sentence(row: &HarnessRow) -> Option<String> {
    if !row.installed {
        return Some(crate::copy::HARNESS_NOT_INSTALLED.to_string());
    }
    Some(crate::copy::harness_state_line(&row.state))
        .filter(|line| !line.is_empty())
        .map(str::to_string)
}

/// The core's Private AI state label: `tool_destinations`'s, then the
/// status's; `None` when neither says.
pub fn private_ai_label(
    destinations: Option<&ToolDestinations>,
    status: Option<&GateStatus>,
) -> Option<String> {
    destinations.and_then(|d| d.private_ai.clone()).or_else(|| {
        status
            .and_then(|s| s.private_inference_state.as_ref())
            .map(|state| state.state.clone())
    })
}

/// IronWire's harness ids to the tools that have artwork (`HarnessToolArt`).
pub fn harness_tool(id: &str) -> Option<SourceKind> {
    match id {
        "claude" | "claude-code" => Some(SourceKind::ClaudeCode),
        "codex" => Some(SourceKind::Codex),
        "gemini" | "gemini-cli" => Some(SourceKind::GeminiCli),
        "cline" => Some(SourceKind::Cline),
        "opencode" => Some(SourceKind::Opencode),
        _ => None,
    }
}

/// The two letters a Private AI tool with no artwork carries (owner,
/// 2026-10-14: a long list of blank circles reads as one), from its name,
/// as the Mac's `.other(initials:)` takes them.
pub fn harness_initials(node: &Node) -> Option<String> {
    match node.kind {
        NodeKind::Harness { tool: None, .. } => Some(initials(&node.label)),
        _ => None,
    }
}

/// The destination's card sentence (`sentence`) or the line under its
/// node: how many tools are connected.
pub fn connected_words(words: &MonitorFlowMapCopy, count: usize, sentence: bool) -> String {
    fill(
        if sentence {
            words.connected
        } else {
            words.connected_line
        },
        &counted(Some(count), words.tools_one, words.tools),
        None,
    )
}

impl Scene {
    /// The destination, right of centre on the band's middle; the tools in
    /// a column to its left, their names to the left of their icons.
    pub fn destination(space: Size) -> Point {
        let band = Self::band(space);
        Point::new(space.width * 0.68, (band.0 + band.1) / 2.0)
    }

    pub fn harness_x(space: Size) -> f64 {
        (space.width * 0.36).max(150.0)
    }

    /// The destination's lamp from the core's Private AI state label. A
    /// label this shell does not know is unknown, never off.
    pub fn lamp(private_ai: Option<&str>, answering: bool) -> Lamp {
        match private_ai {
            Some("running") if answering => Lamp::Answering,
            Some("running") => Lamp::Running,
            Some("off") => Lamp::Off,
            _ => Lamp::Unknown,
        }
    }

    /// The Private AI map: each tool that can send model calls here,
    /// joined to the destination, drawn from the core's state for it,
    /// never from `connected`: flowing only when its calls are answered,
    /// solid when connected with nothing answered, lit but still when its
    /// family's calls cannot be attributed, dashed when not connected,
    /// dotted when the core could not say.
    ///
    /// - `harnesses` is `harness_list`'s rows; `None` while unread or after
    ///   a failed read, which is not an empty list and draws nothing.
    /// - `private_ai` is the core's Private AI state label
    ///   ([`private_ai_label`]), `None` when it did not say.
    /// - `sentence` is the core's sentence for a row ([`row_sentence`]).
    /// - `state` is the core's verdict for a row ([`harness_state`]).
    pub fn private_ai(
        harnesses: Option<&[HarnessRow]>,
        private_ai: Option<&str>,
        sentence: impl Fn(&HarnessRow) -> Option<String>,
        state: impl Fn(&HarnessRow) -> HarnessState,
        destinations: Option<&ToolDestinations>,
        space: Size,
    ) -> Scene {
        let words = words();
        let screens = monitor_screens_copy();
        let mut scene = Scene {
            space,
            ..Scene::default()
        };
        // An unreadable list is not an empty configured list. Managed
        // routes, read apart, may still be added.
        let Some(harnesses) = harnesses else {
            return scene;
        };
        let has_installed = destinations
            .is_some_and(|d| d.tools.iter().any(|t| t.installed == Some(true)))
            || harnesses.iter().any(|row| row.installed);
        let rows: Vec<&HarnessRow> = harnesses
            .iter()
            .filter(|row| {
                if !has_installed {
                    return true;
                }
                let source = if row.id == "claude" {
                    "claude-code"
                } else {
                    row.id.as_str()
                };
                destinations
                    .and_then(|d| d.tools.iter().find(|t| t.tool == source))
                    .and_then(|t| t.installed)
                    .unwrap_or(row.installed)
            })
            .collect();
        let destination = Self::destination(space);
        let x = Self::harness_x(space);
        // Evenly down the band, no further apart than reads as one group,
        // centred on the destination; as many as fit at the least line
        // height, the rest counted as "+N" under them.
        let band = Self::band(space);
        let fit = (((band.1 - band.0) / HARNESS_STEP_MIN) as usize).max(1);
        let shown = if rows.len() > fit {
            &rows[..fit - 1]
        } else {
            &rows[..]
        };
        let mut weights = vec![1.0; shown.len()];
        if shown.len() < rows.len() {
            weights.push(0.7);
        }
        let slots = Self::stack(&weights, band, HARNESS_STEP_MAX, HARNESS_STEP_MIN);
        if shown.len() < rows.len()
            && let Some(last) = slots.last()
        {
            scene.overflows.push(Overflow {
                at: Point::new(x - 24.0, last.0),
                count: rows.len() - shown.len(),
            });
        }
        let mut answering = false;
        let mut connected = 0;
        for (index, row) in shown.iter().enumerate() {
            let y = slots[index].0;
            let style = match state(row) {
                HarnessState::Answering => {
                    answering = true;
                    connected += 1;
                    ArcStyle::Flowing
                }
                HarnessState::ConnectedNoCalls => {
                    connected += 1;
                    ArcStyle::Quiet
                }
                HarnessState::ActivityShared => {
                    connected += 1;
                    ArcStyle::Shared
                }
                HarnessState::NotConnected => ArcStyle::Dashed,
                HarnessState::Unknown => ArcStyle::Unknown,
            };
            let bend = (x + destination.x) / 2.0;
            scene.arcs.push(Arc {
                from: Point::new(x + 12.0, y),
                control1: Point::new(bend, y),
                control2: Point::new(bend, destination.y),
                to: destination,
                style,
                dim: 1.0,
            });
            let mut node = Node::new(
                &format!("harness:{}", row.id),
                NodeKind::Harness {
                    tool: harness_tool(&row.id),
                    installed: row.installed,
                },
                Point::new(x, y),
                13.0,
                &row.name,
                &sentence(row).unwrap_or_else(|| figure(None)),
            );
            node.dim = if row.installed { 1.0 } else { 0.5 };
            scene.nodes.push(node);
        }
        // The tools counted as "+N" still count: the credential says how
        // many are connected and whether any is answered, drawn or not.
        for row in &rows[shown.len()..] {
            match state(row) {
                HarnessState::Answering => {
                    answering = true;
                    connected += 1;
                }
                HarnessState::ConnectedNoCalls | HarnessState::ActivityShared => connected += 1,
                HarnessState::NotConnected | HarnessState::Unknown => {}
            }
        }
        let lamp = Self::lamp(private_ai, answering);
        // #1146's credential card: how many tools are connected, in a
        // sentence; unknown is said, never left to read as off.
        let mut detail = connected_words(&words, connected, true);
        if lamp == Lamp::Unknown {
            detail = format!("{detail} {}", screens.unknown);
        }
        let mut credential = Node::new(
            "destination",
            NodeKind::Destination(lamp),
            destination,
            26.0,
            words.credential,
            &detail,
        );
        credential.sublabel = Some(if rows.is_empty() {
            words.none_found.to_string()
        } else {
            connected_words(&words, connected, false)
        });
        scene.nodes.insert(0, credential);
        scene
    }

    /// False for a scene with nothing to draw (an unread harness list and
    /// no managed routes): the pane stays blank.
    pub fn draws_map(&self) -> bool {
        !self.nodes.is_empty()
    }

    /// Managed configuration has its own routes, right of the destination
    /// and in the band's lower third, each a tool and its provider joined
    /// by a dashed line. A process being alive never animates a model call
    /// or lights the global destination.
    pub fn adding_managed_routes(&self, destinations: Option<&ToolDestinations>) -> Scene {
        let Some((sessions, copy)) = destinations
            .and_then(|d| Some((d.managed_sessions.as_ref()?, d.managed_copy.as_ref()?)))
        else {
            return self.clone();
        };
        let screens = monitor_screens_copy();
        let mut scene = self.clone();
        let mut groups: BTreeMap<String, Vec<&crate::model::ManagedMapSession>> = BTreeMap::new();
        for row in sessions {
            groups
                .entry(format!("{}:{}", row.tool, row.provider))
                .or_default()
                .push(row);
        }
        let keys = groups.len();
        let band = Self::band(scene.space);
        for (index, (key, rows)) in groups.iter().enumerate() {
            let first = rows[0];
            // Below the destination's own card, so a managed route's name
            // (drawn left of its icon) never sits on the destination.
            let top = band.0 + (band.1 - band.0) * 0.66;
            let y = if keys > 1 {
                top + index as f64 * (band.1 - top) / (keys - 1) as f64
            } else {
                band.0 + (band.1 - band.0) * 0.85
            };
            let start = Point::new(scene.space.width * 0.8, y);
            let end = Point::new(scene.space.width * 0.93, y);
            let details = rows
                .iter()
                .take(4)
                .map(|row| row.detail(copy))
                .collect::<Vec<_>>()
                .join("\n");
            let overflow = if rows.len() > 4 {
                format!("\n+{}", rows.len() - 4)
            } else {
                String::new()
            };
            let detail = copy
                .get("map_scope")
                .cloned()
                .into_iter()
                .chain([details + &overflow])
                .collect::<Vec<_>>()
                .join("\n");
            let name = copy.get(&first.tool).unwrap_or(&first.tool);
            let mut tool = Node::new(
                &format!("managed:{key}"),
                NodeKind::Harness {
                    tool: harness_tool(&first.tool),
                    installed: true,
                },
                start,
                13.0,
                name,
                &detail,
            );
            tool.title = copy.get("title").cloned();
            tool.sublabel = copy
                .get("title")
                .map(|title| dot_pair(title, Some(rows.len())));
            scene.nodes.push(tool);
            scene.nodes.push(Node::new(
                &format!("managed-provider:{key}"),
                NodeKind::Destination(Lamp::Unknown),
                end,
                16.0,
                copy.get(&format!("provider_{}", first.provider))
                    .map_or(screens.unknown, String::as_str),
                &detail,
            ));
            scene.arcs.push(curve(start, end, ArcStyle::Dashed, 1.0));
        }
        scene
    }
}

fn curve(from: Point, to: Point, style: ArcStyle, dim: f64) -> Arc {
    let mid = (from.x + to.x) / 2.0;
    Arc {
        from,
        control1: Point::new(mid, from.y),
        control2: Point::new(mid, to.y),
        to,
        style,
        dim,
    }
}

// --- Words -----------------------------------------------------------------

/// A count the core may not have reported: a dash then.
pub fn figure(count: Option<usize>) -> String {
    count.map_or_else(|| "\u{2014}".to_string(), |n| n.to_string())
}

fn counted(count: Option<usize>, one: &str, many: &str) -> String {
    if count == Some(1) {
        one.to_string()
    } else {
        many.replace("{count}", &figure(count))
    }
}

fn fill(word: &str, label: &str, count: Option<Option<usize>>) -> String {
    let filled = word.replace("{label}", label);
    match count {
        Some(count) => filled.replace("{count}", &figure(count)),
        None => filled,
    }
}

/// This computer's card.
pub fn hub_words(words: &MonitorFlowMapCopy, waiting: usize, contributed: Option<usize>) -> String {
    fill(
        words.hub,
        &counted(Some(waiting), words.sessions_one, words.sessions),
        Some(contributed),
    )
}

/// The library's card.
pub fn library_words(words: &MonitorFlowMapCopy, contributed: Option<usize>) -> String {
    fill(
        words.library,
        &counted(contributed, words.traces_one, words.traces),
        None,
    )
}

/// A tool's card title: its name and how many folders.
pub fn tool_title(words: &MonitorFlowMapCopy, tool: &ToolNode) -> String {
    fill(
        words.tool_title,
        &counted(Some(tool.folders.len()), words.folders_one, words.folders),
        None,
    )
    .replace("{tool}", tool.kind.display_name())
}

/// A tool's card: what watching means, then what waits. Unset is never
/// said as off, and unknown is said as neither.
pub fn tool_words(words: &MonitorFlowMapCopy, mode: SourceMode, waiting: usize) -> String {
    let waits = if waiting > 0 {
        words.tool_waiting.replace("{count}", &waiting.to_string())
    } else {
        words.tool_nothing_waiting.to_string()
    };
    match mode {
        SourceMode::Watch => format!("{} {waits}", words.tool_watched),
        SourceMode::Off => words.tool_off.to_string(),
        SourceMode::Unset => words.tool_unset.to_string(),
        SourceMode::Unknown => waits,
    }
}

fn tool_detail(words: &MonitorFlowMapCopy, tool: &ToolNode) -> String {
    tool_words(words, tool.mode, tool.waiting())
}

/// A folder's card: its path, its rule in the core's mode names, and its
/// counts.
pub fn folder_words(
    words: &MonitorFlowMapCopy,
    path: Option<&str>,
    rule: Option<&str>,
    waiting: usize,
    contributed: Option<usize>,
) -> String {
    let rule = rule.map_or_else(
        || words.folder_rule_unset.to_string(),
        |rule| fill(words.folder_rule, rule, None),
    );
    let counts = fill(
        words.folder_counts,
        &counted(Some(waiting), words.sessions_one, words.sessions),
        Some(contributed),
    );
    let place = path.filter(|p| !p.is_empty()).map(|p| format!("{p}."));
    place
        .into_iter()
        .chain([rule, counts])
        .collect::<Vec<_>>()
        .join(" ")
}

fn folder_detail(
    words: &MonitorFlowMapCopy,
    folder: &FolderNode,
    contributed: Option<usize>,
) -> String {
    folder_words(
        words,
        folder.path.as_deref(),
        folder.mode.map(ProjectMode::label),
        folder.sessions.len(),
        contributed,
    )
}

/// The "+N" pill's accessible name: how many folders a click draws.
pub fn reveal_folders(words: &MonitorFlowMapCopy, count: usize) -> String {
    if count == 1 {
        words.reveal_folders_one.to_string()
    } else {
        words.reveal_folders.replace("{count}", &count.to_string())
    }
}

/// A coin's line under its name.
pub fn coin_waiting(words: &MonitorFlowMapCopy, count: usize) -> String {
    words.coin_waiting.replace("{count}", &count.to_string())
}

/// What the core's managed configuration says for this tool, after its
/// card: never a claim of a call or a contribution.
fn managed_detail(source: &str, destinations: Option<&ToolDestinations>) -> String {
    let Some((copy, rows)) =
        destinations.and_then(|d| Some((d.managed_copy.as_ref()?, d.managed_sessions.as_ref()?)))
    else {
        return String::new();
    };
    let rows: Vec<_> = rows.iter().filter(|row| row.source() == source).collect();
    if rows.is_empty() {
        return String::new();
    }
    let detail = rows
        .iter()
        .take(4)
        .map(|row| row.detail(copy))
        .collect::<Vec<_>>()
        .join("\n");
    let overflow = if rows.len() > 4 {
        format!("\n+{}", rows.len() - 4)
    } else {
        String::new()
    };
    format!(
        "\n{}\n{detail}{overflow}",
        copy.get("title").map(String::as_str).unwrap_or_default()
    )
}

/// A word and a count the core may not have reported, set apart by a
/// spaced dot, a dash in place of the count.
pub fn dot_pair(word: &str, count: Option<usize>) -> String {
    format!("{word} \u{b7} {}", figure(count))
}

// --- Drawing geometry ------------------------------------------------------

/// The zoom range and step of the map's -/+.
pub const ZOOM_MIN: f64 = 0.6;
pub const ZOOM_MAX: f64 = 2.0;
pub const ZOOM_STEP: f64 = 0.25;

/// One step of -/+, held in range.
pub fn zoomed(zoom: f64, enlarge: bool) -> f64 {
    if enlarge {
        (zoom + ZOOM_STEP).min(ZOOM_MAX)
    } else {
        (zoom - ZOOM_STEP).max(ZOOM_MIN)
    }
}

/// The design space fitted into the field, centred, and zoomed about the
/// centre; or, with a focus, scaled 1.6x about that point as #1146's
/// camera is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapGeometry {
    pub scale: f64,
    pub origin: Point,
}

impl MapGeometry {
    pub const FOCUS_SCALE: f64 = 1.6;

    pub fn new(size: Size, zoom: f64, focus: Option<Point>, space: Size) -> Self {
        let fit = (size.width / space.width).min(size.height / space.height);
        let centred = Point::new(
            (size.width - space.width * fit) / 2.0,
            (size.height - space.height * fit) / 2.0,
        );
        match focus {
            Some(focus) => {
                // #1146: translate(290 - 1.6(x - 50), 380 - 1.6y)
                // scale(1.6), in design units, then fitted.
                let k = Self::FOCUS_SCALE;
                Self {
                    scale: (fit * k).max(0.01),
                    origin: Point::new(
                        centred.x + fit * (space.width / 2.0 - k * (focus.x - 50.0)),
                        centred.y + fit * (space.height / 2.0 - k * focus.y),
                    ),
                }
            }
            None => {
                let scale = (fit * zoom).max(0.01);
                Self {
                    scale,
                    origin: Point::new(
                        (size.width - space.width * scale) / 2.0,
                        (size.height - space.height * scale) / 2.0,
                    ),
                }
            }
        }
    }

    pub fn point(&self, p: Point) -> Point {
        Point::new(
            self.origin.x + p.x * self.scale,
            self.origin.y + p.y * self.scale,
        )
    }
}

/// A node's halo, in design units scaled with the map.
pub fn ring_metrics(scale: f64) -> (f64, f64) {
    (4.0 * scale, 3.0 * scale)
}

/// The smallest factor the map's labels are drawn at.
pub const LABEL_FLOOR: f64 = 0.85;

/// A node's label and sublabel: `(label size, sublabel size, label offset,
/// sublabel offset)`, the offsets below the disc. Type and gaps scale
/// together, so the two lines never meet.
pub fn label_type(scale: f64) -> (f64, f64, f64, f64) {
    let unit = scale.max(LABEL_FLOOR);
    (11.0 * unit, 10.0 * unit, 12.0 * unit, 26.5 * unit)
}

/// A folder's name on the map: at most this many characters, the last an
/// ellipsis when it is cut. The full name is the node's accessible name.
pub const LABEL_LIMIT: usize = 14;

pub fn short_label(label: &str) -> String {
    if label.chars().count() > LABEL_LIMIT {
        label
            .chars()
            .take(LABEL_LIMIT - 1)
            .chain(['\u{2026}'])
            .collect()
    } else {
        label.to_string()
    }
}

/// The station's frame on the Traces map, in design units.
pub const STATION_SIZE: Size = Size::new(150.0, 130.0);
/// The node card's width (`size.nodeCardWidth`).
pub const NODE_CARD_WIDTH: f64 = 260.0;
/// The gap between a node's drawing and its card, and the card's least
/// distance from the field's edge.
pub const CARD_GAP: f64 = 12.0;
pub const CARD_MARGIN: f64 = 8.0;

/// The Traces map's style (`TracesMapStyle`, Settings): the pad, rocket
/// and station, the Mac's default, or the laptop and the library orb. A
/// shell preference: only the drawing changes, never what is sent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MapStyle {
    #[default]
    Launch,
    Standard,
}

/// How far the tool pile has opened (`MapToolPile`'s `tall`, `wide` and
/// `names`): its full column, then its card wide enough for the names,
/// then the names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PileStage {
    pub tall: bool,
    pub wide: bool,
    pub names: bool,
}

/// One stage of opening or closing the pile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PileStep {
    Tall(bool),
    Wide(bool),
    Names(bool),
}

impl PileStage {
    pub fn with(self, step: PileStep) -> Self {
        match step {
            PileStep::Tall(tall) => Self { tall, ..self },
            PileStep::Wide(wide) => Self { wide, ..self },
            PileStep::Names(names) => Self { names, ..self },
        }
    }

    /// The stage as drawn: while a trace moves the card keeps its closed
    /// size, so no line moves under it.
    pub fn drawn(self, moving: bool) -> Self {
        if moving { Self::default() } else { self }
    }

    /// The whole card open: tall and wide, drawn opaque.
    pub fn open(self) -> bool {
        self.tall && self.wide
    }
}

/// The pile's stages when the pointer comes over its card (`inside`) or
/// leaves it, each with its delay in milliseconds from then
/// (`MapToolPile.hover`); all at once under reduced motion.
pub fn pile_steps(inside: bool, reduce_motion: bool) -> [(u32, PileStep); 3] {
    let at = |ms: u32| if reduce_motion { 0 } else { ms };
    if inside {
        [
            (0, PileStep::Tall(true)),
            (at(260), PileStep::Wide(true)),
            (at(260 + 200), PileStep::Names(true)),
        ]
    } else {
        [
            (0, PileStep::Names(false)),
            (at(100), PileStep::Wide(false)),
            (at(100 + 220), PileStep::Tall(false)),
        ]
    }
}

/// The library orb's frame on the standard map, in design units
/// (`MapTransferDrawing.orbFrame`).
pub const ORB_FRAME: f64 = 104.0;

/// The library's frame on the Traces map for `style`, in design units.
pub fn library_size(style: MapStyle) -> Size {
    match style {
        MapStyle::Launch => STATION_SIZE,
        MapStyle::Standard => Size::new(ORB_FRAME, ORB_FRAME),
    }
}

/// Half the width the map draws a node at, in design units: on the Traces
/// map the station or the orb for the library and the pad or the laptop
/// for this computer, far wider than their discs; elsewhere the disc
/// (`FlowMapView.drawnHalfWidth`).
pub fn drawn_half_width(node: &Node, traces: bool, style: MapStyle) -> f64 {
    if !traces {
        return node.radius;
    }
    match node.kind {
        NodeKind::Library { .. } => library_size(style).width / 2.0,
        NodeKind::Hub => PAD_HALF_WIDTH,
        _ => node.radius,
    }
}

/// Where a node's card is centred across the field: beside the node's
/// drawing, never over it. On the right where the card fits whole, else
/// on the left; where neither side holds it, the roomier side, kept inside
/// the field.
pub fn card_x(centre: f64, half_width: f64, card: f64, field: f64) -> f64 {
    let right = centre + half_width + CARD_GAP + card / 2.0;
    if right + card / 2.0 + CARD_MARGIN <= field {
        return right;
    }
    let left = centre - half_width - CARD_GAP - card / 2.0;
    if left - card / 2.0 >= CARD_MARGIN {
        return left;
    }
    let x = if field - centre > centre { right } else { left };
    x.max(card / 2.0 + CARD_MARGIN)
        .min(field - card / 2.0 - CARD_MARGIN)
}

/// The card's vertical centre: by the node, held inside the field.
pub fn card_y(centre: f64, field: f64) -> f64 {
    (centre - 24.0).max(48.0).min(field - 48.0)
}

/// A folder node's project id (`folder:<id>`).
pub fn folder_id(node_id: &str) -> Option<&str> {
    node_id.strip_prefix("folder:")
}

/// A tool with no artwork is drawn as two letters of its name: a word's
/// first two ("Cursor" is "Cu"), or the first letters of its first two
/// words ("Claude Code" is "CC").
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let Some(first) = words.first() else {
        return "?".to_string();
    };
    let mut letters = first.chars();
    let lead: String = letters
        .next()
        .into_iter()
        .flat_map(char::to_uppercase)
        .collect();
    match words.get(1).and_then(|second| second.chars().next()) {
        Some(second) => lead + &second.to_uppercase().collect::<String>(),
        None => {
            lead + &letters
                .next()
                .into_iter()
                .flat_map(char::to_lowercase)
                .collect::<String>()
        }
    }
}

/// The credential's key, as line segments in the map's units around the
/// node's centre: an open bow on the left (a circle, `(centre, radius)`),
/// a level shaft and two bits.
pub fn key_glyph(centre: Point, scale: f64) -> ((Point, f64), [(Point, Point); 3]) {
    let s = scale;
    let bow = (Point::new(centre.x - 9.0 * s, centre.y), 6.0 * s);
    (
        bow,
        [
            (
                Point::new(centre.x - 3.0 * s, centre.y),
                Point::new(centre.x + 11.0 * s, centre.y),
            ),
            (
                Point::new(centre.x + 7.0 * s, centre.y),
                Point::new(centre.x + 7.0 * s, centre.y + 5.0 * s),
            ),
            (
                Point::new(centre.x + 11.0 * s, centre.y),
                Point::new(centre.x + 11.0 * s, centre.y + 4.0 * s),
            ),
        ],
    )
}

#[cfg(test)]
#[path = "../../../tests/support/map_scene_tests.rs"]
mod tests;
