//! The Traces map's transfers: traces the map shows moving.
//!
//! A native port of `MapTransfers.swift` and `MapTransferFeed.swift` in
//! `macos/Sources/TraceCommonsApp/Views/Monitor/`, and of the resting
//! motion `FlowMapView.restingMotion` decides. Pure data, no widgets: the
//! player runs on an explicit clock (seconds, the frame clock's monotonic
//! time), so every moment of a run and every card's life is tested without
//! a display.
//!
//! - A row History gained, sent after everything the earlier read held, is
//!   a contribution; the window's last keep a keep; its last dismissal a
//!   dismissal. The first read finds none, so what left before the window
//!   opened never plays (`contributions`, `kept`, `dismissed`).
//! - The player plays one run at a time, all of one action; each trace
//!   rides in, orbits its coin, rides out, (on the standard map) rises,
//!   and arrives (`Run::leg`). A run is one card at the map's foot: the
//!   trace's own, or one batch card for several (`MapTransfer::batch`). A
//!   dismissal moves nothing and only shows its card (`Player::announce`).
//! - Under reduced motion (`gtk-enable-animations` false) the player still
//!   plays, so the cards still show and expire; the map draws no run, no
//!   moving dash, and its clock is stopped (`drawn_run`, `resting_motion`).
//!
//! Deliberate platform differences:
//! - This shell has no `keep` action, so nothing feeds `kept` yet; it is
//!   ported and tested so a keep plays the Mac's run once one exists.
//! - This shell draws the launch style only (the pad and the rocket, the
//!   Mac's default); it has no style switch. The standard map's rise is
//!   ported with the run so the two stay one rule.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use trace_commons_contributor::preview_copy::MonitorFlowMapCopy;

use crate::model::{HistoryRecord, QueueEntry};

use super::map_scene::{Arc, Point};

/// What happened to a trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Contribute,
    Keep,
    Dismiss,
}

/// The Traces map's style: the pad, rocket and station, or the laptop and
/// the library orb. This shell draws `Launch`, the Mac's default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MapStyle {
    #[default]
    Launch,
    Standard,
}

/// A trace the Traces map shows moving, built only from what the core
/// reported, never from a guess.
#[derive(Clone, Debug, PartialEq)]
pub struct MapTransfer {
    /// The History row's submission id, or the queue entry's id.
    pub id: String,
    /// Its folder (`project_id`) and its tool (the adapter's source name).
    pub folder: String,
    pub tool: String,
    pub action: Action,
    /// When a contribution was sent, or when a kept or dismissed trace
    /// started; `None` when unrecorded.
    pub at: Option<DateTime<Utc>>,
    /// The folder's name as the core gave it; `None` when it gave none,
    /// and the card then looks the folder up on the map.
    pub folder_label: Option<String>,
    /// How many traces the card stands for: more than one for a batch.
    pub count: usize,
    /// For a batch, whether its traces came from more than one folder or
    /// tool; the card then names neither.
    pub mixed_folders: bool,
    pub mixed_tools: bool,
}

impl MapTransfer {
    pub fn new(id: &str, folder: &str, tool: &str, action: Action) -> Self {
        Self {
            id: id.to_string(),
            folder: folder.to_string(),
            tool: tool.to_string(),
            action,
            at: None,
            folder_label: None,
            count: 1,
            mixed_folders: false,
            mixed_tools: false,
        }
    }

    /// One card for a run of several traces: the first trace's folder,
    /// tool and time, and how many there are.
    pub fn batch(transfers: &[MapTransfer]) -> Option<MapTransfer> {
        let first = transfers.first()?;
        if transfers.len() == 1 {
            return Some(first.clone());
        }
        let folders: HashSet<&str> = transfers.iter().map(|t| t.folder.as_str()).collect();
        let tools: HashSet<&str> = transfers.iter().map(|t| t.tool.as_str()).collect();
        Some(MapTransfer {
            id: format!("batch:{}", first.id),
            count: transfers.len(),
            mixed_folders: folders.len() > 1,
            mixed_tools: tools.len() > 1,
            ..first.clone()
        })
    }
}

// --- Finding transfers ---------------------------------------------------------

fn non_empty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

/// The rows History gained between two reads, sent after everything the
/// earlier read held, in the order sent. An unread read on either side
/// finds none, and a row with no folder, tool or time is not one.
pub fn contributions(
    old: Option<&[HistoryRecord]>,
    new: Option<&[HistoryRecord]>,
) -> Vec<MapTransfer> {
    let (Some(old), Some(new)) = (old, new) else {
        return Vec::new();
    };
    let seen: HashSet<&str> = old.iter().map(|row| row.submission_id.as_str()).collect();
    let newest = old.iter().filter_map(|row| row.submitted_at).max();
    let mut found: Vec<MapTransfer> = new
        .iter()
        .filter_map(|row| {
            let sent = row.submitted_at?;
            if seen.contains(row.submission_id.as_str()) || newest.is_some_and(|n| sent <= n) {
                return None;
            }
            let folder = non_empty(&row.project_id)?;
            let tool = row.source.as_deref().and_then(non_empty)?;
            let mut transfer =
                MapTransfer::new(&row.submission_id, folder, tool, Action::Contribute);
            transfer.at = Some(sent);
            transfer.folder_label = non_empty(&row.project_label).map(str::to_string);
            Some(transfer)
        })
        .collect();
    found.sort_by_key(|transfer| transfer.at);
    found
}

/// The entry the window has just kept, found among the traces it was
/// listed with; `None` when it is not there.
pub fn kept(entry_id: Option<&str>, sessions: &[QueueEntry]) -> Option<MapTransfer> {
    decided(entry_id, Action::Keep, sessions)
}

/// The entry the window has just dismissed, likewise.
pub fn dismissed(entry_id: Option<&str>, sessions: &[QueueEntry]) -> Option<MapTransfer> {
    decided(entry_id, Action::Dismiss, sessions)
}

fn decided(entry_id: Option<&str>, action: Action, sessions: &[QueueEntry]) -> Option<MapTransfer> {
    let entry_id = entry_id?;
    let entry = sessions.iter().find(|entry| entry.entry_id == entry_id)?;
    let mut transfer = MapTransfer::new(&entry.entry_id, &entry.project_id, &entry.source, action);
    transfer.at = entry.started_at;
    transfer.folder_label = non_empty(&entry.project_label).map(str::to_string);
    Some(transfer)
}

// --- A run ---------------------------------------------------------------------

/// Where one trace is, some seconds into its run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Leg {
    Waiting,
    /// Riding its folder's line, 0..1.
    Inbound(f64),
    /// In its coin while the rim light goes round, 0..1.
    Orbit(f64),
    /// Riding the line out to the pad, 0..1.
    Outbound(f64),
    /// Rising from the laptop to the library's orb, 0..1 (the standard
    /// map's contributions only).
    Ascent(f64),
    /// Gone into the rocket, the pad or the orb, this many seconds ago.
    Arrived(f64),
}

/// One run of transfers the map plays together, all of one action.
/// Everything is a function of the seconds since it began.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub transfers: Vec<MapTransfer>,
    /// When it began, on the player's clock.
    pub start: f64,
    pub style: MapStyle,
}

impl Run {
    /// The seconds between traces setting off, and each leg's length.
    pub const STEP: f64 = 1.05;
    pub const INBOUND: f64 = 0.70;
    pub const ORBIT: f64 = 0.45;
    pub const OUTBOUND: f64 = 0.70;
    /// The pause on the pad before lift-off, the flight, and what follows.
    pub const SETTLE: f64 = 0.5;
    pub const FLIGHT: f64 = 2.6;
    pub const LINGER: f64 = 1.3;
    pub const KEPT_LINGER: f64 = 1.6;
    /// The pulse a trace gives off as it vanishes at the pad.
    pub const PULSE: f64 = 0.9;
    /// A contributed trace's rise from the laptop to the orb.
    pub const ASCENT: f64 = 1.1;

    pub fn new(transfers: Vec<MapTransfer>, start: f64, style: MapStyle) -> Self {
        Self {
            transfers,
            start,
            style,
        }
    }

    pub fn action(&self) -> Action {
        self.transfers
            .first()
            .map_or(Action::Contribute, |t| t.action)
    }

    /// Whether each contributed trace rises to the orb on its own.
    pub fn rises(&self) -> bool {
        self.style == MapStyle::Standard && self.action() == Action::Contribute
    }

    pub fn leg(&self, index: usize, t: f64) -> Leg {
        let mut local = t - index as f64 * Self::STEP;
        if local < 0.0 {
            return Leg::Waiting;
        }
        if local < Self::INBOUND {
            return Leg::Inbound(local / Self::INBOUND);
        }
        local -= Self::INBOUND;
        if local < Self::ORBIT {
            return Leg::Orbit(local / Self::ORBIT);
        }
        local -= Self::ORBIT;
        if local < Self::OUTBOUND {
            return Leg::Outbound(local / Self::OUTBOUND);
        }
        local -= Self::OUTBOUND;
        if self.rises() {
            if local < Self::ASCENT {
                return Leg::Ascent(local / Self::ASCENT);
            }
            local -= Self::ASCENT;
        }
        Leg::Arrived(local)
    }

    /// When the last trace reaches the pad, or on the standard map the orb.
    pub fn all_arrived(&self) -> f64 {
        let legs = Self::INBOUND
            + Self::ORBIT
            + Self::OUTBOUND
            + if self.rises() { Self::ASCENT } else { 0.0 };
        self.transfers.len().saturating_sub(1) as f64 * Self::STEP + legs
    }

    /// When the rocket lifts off (a contribution's run on the launch map).
    pub fn lift_off(&self) -> f64 {
        self.all_arrived() + Self::SETTLE
    }

    /// When the run is over, in seconds from its start.
    pub fn end(&self) -> f64 {
        if self.action() != Action::Contribute {
            return self.all_arrived() + Self::KEPT_LINGER;
        }
        if self.rises() {
            self.all_arrived() + Self::LINGER
        } else {
            self.lift_off() + Self::FLIGHT + Self::LINGER
        }
    }

    /// How many traces have arrived by `t`.
    pub fn arrived(&self, t: f64) -> usize {
        (0..self.transfers.len())
            .filter(|&i| matches!(self.leg(i, t), Leg::Arrived(_)))
            .count()
    }

    /// The pile's order at `t`, front first: `base` with each trace's tool
    /// moved to the front as that trace set off.
    pub fn order(&self, base: &[String], t: f64) -> Vec<String> {
        let mut order = base.to_vec();
        for (index, transfer) in self.transfers.iter().enumerate() {
            if self.leg(index, t) == Leg::Waiting {
                continue;
            }
            let id = format!("tool:{}", transfer.tool);
            if let Some(at) = order.iter().position(|tool| *tool == id) {
                let tool = order.remove(at);
                order.insert(0, tool);
            }
        }
        order
    }

    /// The tools whose coin is grown at `t` (a trace is in it), with how
    /// far round the rim light has gone.
    pub fn orbits(&self, t: f64) -> Vec<(String, f64)> {
        let mut orbits: Vec<(String, f64)> = Vec::new();
        for (index, transfer) in self.transfers.iter().enumerate() {
            if let Leg::Orbit(progress) = self.leg(index, t) {
                let id = format!("tool:{}", transfer.tool);
                orbits.retain(|(tool, _)| *tool != id);
                orbits.push((id, progress));
            }
        }
        orbits.sort_by(|a, b| a.0.cmp(&b.0));
        orbits
    }

    /// The trace the details describe at `t`: the latest to set off that
    /// has not yet arrived.
    pub fn current(&self, t: f64) -> Option<&MapTransfer> {
        (0..self.transfers.len())
            .rev()
            .find(|&i| !matches!(self.leg(i, t), Leg::Waiting | Leg::Arrived(_)))
            .map(|i| &self.transfers[i])
    }

    /// Whether any trace is still moving through the map at `t`.
    pub fn moving(&self, t: f64) -> bool {
        self.arrived(t) < self.transfers.len()
    }

    /// How far the rocket has flown, 0..1, or `None` while it waits on the
    /// pad, or when there is no rocket (a kept run, or the standard map).
    pub fn flight(&self, t: f64) -> Option<f64> {
        if self.action() != Action::Contribute || self.rises() || t < self.lift_off() {
            return None;
        }
        Some(((t - self.lift_off()) / Self::FLIGHT).min(1.0))
    }

    /// The share of the flight's path the rocket has covered: still for
    /// the ignition's first quarter, then slowly away and faster.
    pub fn climb(progress: f64) -> f64 {
        let ignition = 0.26;
        if progress <= ignition {
            return 0.0;
        }
        ((progress - ignition) / (1.0 - ignition)).powf(2.4)
    }
}

/// The ease the traces ride their lines with: in and out, cubic.
pub fn ease(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// The point `t` of the way along an arc's curve.
pub fn point_on(arc: &Arc, t: f64) -> Point {
    let v = t;
    let u = 1.0 - v;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * v, 3.0 * u * v * v, v * v * v);
    Point::new(
        a * arc.from.x + b * arc.control1.x + c * arc.control2.x + d * arc.to.x,
        a * arc.from.y + b * arc.control1.y + c * arc.control2.y + d * arc.to.y,
    )
}

// --- The player ----------------------------------------------------------------

/// Plays the runs the window hands it, one at a time: what arrives while a
/// run plays waits for the next, and a run is all of one action. `cards`
/// are the traces the cards describe, oldest first: a run's card joins as
/// it sets off and stays `HOLD` seconds past its end; a dismissal's joins
/// at once and stays `HOLD` seconds. Each card's X closes it alone. Under
/// reduced motion the runs still play, so the cards still show.
#[derive(Clone, Debug, Default)]
pub struct Player {
    run: Option<Run>,
    /// Each card and the time it goes.
    cards: Vec<(MapTransfer, f64)>,
    queue: Vec<MapTransfer>,
    played: HashSet<String>,
    /// The style for runs still to play; one playing keeps its own.
    pub style: MapStyle,
}

impl Player {
    /// How long a card stays after its run ends, or after a dismissal.
    pub const HOLD: f64 = 5.0;
    /// The most cards kept; the oldest go first past it.
    pub const STACK_LIMIT: usize = 6;
    /// The most cards drawn at once, the newest first.
    pub const VISIBLE: usize = 3;

    pub fn run(&self) -> Option<&Run> {
        self.run.as_ref()
    }

    /// The cards, oldest first.
    pub fn cards(&self) -> Vec<&MapTransfer> {
        self.cards.iter().map(|(card, _)| card).collect()
    }

    /// The card in front, the newest.
    pub fn front(&self) -> Option<&MapTransfer> {
        self.cards.last().map(|(card, _)| card)
    }

    /// The cards drawn: the newest few, the front one first.
    pub fn visible(&self) -> Vec<&MapTransfer> {
        self.cards
            .iter()
            .rev()
            .take(Self::VISIBLE)
            .map(|(card, _)| card)
            .collect()
    }

    pub fn enqueue(&mut self, transfers: Vec<MapTransfer>, now: f64) {
        let fresh: Vec<MapTransfer> = transfers
            .into_iter()
            .filter(|t| t.action != Action::Dismiss && self.played.insert(t.id.clone()))
            .collect();
        if fresh.is_empty() {
            return;
        }
        self.queue.extend(fresh);
        if self.run.is_none() {
            self.next(now);
        }
    }

    /// A dismissed trace: nothing moves on the map, its card joins.
    pub fn announce(&mut self, transfer: MapTransfer, now: f64) {
        if !self.played.insert(transfer.id.clone()) {
            return;
        }
        self.add(transfer, now + Self::HOLD);
    }

    /// A card's X: that card goes now; the rest, and a run still playing,
    /// carry on.
    pub fn close(&mut self, id: &str) {
        self.cards.retain(|(card, _)| card.id != id);
    }

    /// Ends a run that is over (starting the next one now) and drops the
    /// cards whose time is up. Whether anything changed.
    pub fn tick(&mut self, now: f64) -> bool {
        let mut changed = false;
        while let Some(run) = self.run.as_ref() {
            if now < run.start + run.end() {
                break;
            }
            self.next(now);
            changed = true;
        }
        let before = self.cards.len();
        self.cards.retain(|(_, until)| *until > now);
        changed || self.cards.len() != before
    }

    /// The next moment `tick` has something to do: a run's end or a
    /// card's expiry.
    pub fn next_deadline(&self) -> Option<f64> {
        let run_end = self.run.as_ref().map(|run| run.start + run.end());
        self.cards
            .iter()
            .map(|(_, until)| *until)
            .chain(run_end)
            .min_by(f64::total_cmp)
    }

    fn next(&mut self, now: f64) {
        let Some(action) = self.queue.first().map(|t| t.action) else {
            self.run = None;
            return;
        };
        let count = self.queue.iter().take_while(|t| t.action == action).count();
        let batch: Vec<MapTransfer> = self.queue.drain(..count).collect();
        let run = Run::new(batch, now, self.style);
        // One card for the run as it sets off: the trace's, or the batch's.
        if let Some(card) = MapTransfer::batch(&run.transfers) {
            self.add(card, now + run.end() + Self::HOLD);
        }
        self.run = Some(run);
    }

    fn add(&mut self, transfer: MapTransfer, until: f64) {
        self.cards.retain(|(card, _)| card.id != transfer.id);
        self.cards.push((transfer, until));
        while self.cards.len() > Self::STACK_LIMIT {
            self.cards.remove(0);
        }
    }
}

// --- The feed ------------------------------------------------------------------

/// Hands the player what the core reported between two reads: a row
/// History gained, the window's last keep, its last dismissal. A kept or
/// dismissed trace is found among the traces the read before listed, once
/// the reload has dropped it.
#[derive(Clone, Debug, Default)]
pub struct Feed {
    history: Option<Vec<HistoryRecord>>,
    listed: Vec<QueueEntry>,
    listed_before: Vec<QueueEntry>,
    last_kept: Option<String>,
    last_dismissed: Option<String>,
}

impl Feed {
    /// One observation of the window's reads, at `now`.
    pub fn observe(
        &mut self,
        player: &mut Player,
        entries: &[QueueEntry],
        history: Option<&[HistoryRecord]>,
        last_kept: Option<&str>,
        last_dismissed: Option<&str>,
        now: f64,
    ) {
        let ids = |list: &[QueueEntry]| -> Vec<String> {
            list.iter().map(|entry| entry.entry_id.clone()).collect()
        };
        if ids(entries) != ids(&self.listed) {
            self.listed_before = std::mem::replace(&mut self.listed, entries.to_vec());
        } else {
            self.listed = entries.to_vec();
        }
        let found = contributions(self.history.as_deref(), history);
        self.history = history.map(<[HistoryRecord]>::to_vec);
        player.enqueue(found, now);

        let listed: Vec<QueueEntry> = self
            .listed
            .iter()
            .chain(self.listed_before.iter())
            .cloned()
            .collect();
        if last_kept != self.last_kept.as_deref() {
            self.last_kept = last_kept.map(str::to_string);
            if let Some(transfer) = kept(last_kept, &listed) {
                player.enqueue(vec![transfer], now);
            }
        }
        if last_dismissed != self.last_dismissed.as_deref() {
            self.last_dismissed = last_dismissed.map(str::to_string);
            if let Some(transfer) = dismissed(last_dismissed, &listed) {
                player.announce(transfer, now);
            }
        }
    }
}

// --- Motion --------------------------------------------------------------------

/// How the map's resting clock runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RestingMotion {
    /// Seconds between redraws.
    pub interval: f64,
    pub paused: bool,
}

/// Moving dashes redraw at 30 frames a second.
pub const DASH_INTERVAL: f64 = 1.0 / 30.0;
/// The library's own rate, `GlassLibraryStation.frameInterval`.
pub const LIBRARY_INTERVAL: f64 = 1.0 / 20.0;
/// A run redraws at 60 frames a second.
pub const RUN_INTERVAL: f64 = 1.0 / 60.0;

/// How the map's resting clock runs: at 30 frames a second while arcs
/// carry moving dashes, at the library's rate while only the library
/// turns, and not at all when nothing moves or under reduced motion. An
/// unhealthy map draws its flows still. This shell's library does not
/// turn yet (the look slice), so it passes `library: false`.
pub fn resting_motion(
    reduce_motion: bool,
    flows: bool,
    healthy: bool,
    library: bool,
) -> RestingMotion {
    let dashes = !reduce_motion && flows && healthy;
    let turns = !reduce_motion && library;
    RestingMotion {
        interval: if dashes {
            DASH_INTERVAL
        } else {
            LIBRARY_INTERVAL
        },
        paused: !(dashes || turns),
    }
}

/// How far the flowing dashes have moved, in design units, `seconds`
/// into the clock; none under reduced motion.
pub fn dash_phase(seconds: f64, reduce_motion: bool) -> f64 {
    if reduce_motion { 0.0 } else { seconds * 18.0 }
}

/// The run the map draws: none under reduced motion, where the map stays
/// still and only the card says what is happening.
pub fn drawn_run(run: Option<&Run>, reduce_motion: bool) -> Option<&Run> {
    if reduce_motion { None } else { run }
}

// --- The card ------------------------------------------------------------------

/// A card's tone, as the Mac's tag tone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    On,
    Neutral,
}

/// The card's tag, its tone and its note, by what is happening. `tool` is
/// `None` for a batch from several tools, whose note names none.
///
/// Platform difference: a keep's note, "The trace stays on this Mac...",
/// names the Mac, so this shell draws none (and has no keep action).
pub fn card_state(
    action: Action,
    tool: Option<&str>,
    words: &MonitorFlowMapCopy,
) -> (&'static str, Tone, Option<String>) {
    match action {
        Action::Contribute => (
            words.transfer_contributing,
            Tone::On,
            Some(match tool {
                Some(tool) => words.transfer_contribute_note.replace("{tool}", tool),
                None => words.transfer_contribute_note_mixed.to_string(),
            }),
        ),
        Action::Keep => (words.transfer_keeping, Tone::Neutral, None),
        Action::Dismiss => (
            words.transfer_dismissed,
            Tone::Neutral,
            Some(words.transfer_dismiss_note.to_string()),
        ),
    }
}

/// A batch card's count.
pub fn traces_count(words: &MonitorFlowMapCopy, count: usize) -> String {
    words.transfer_traces.replace("{count}", &count.to_string())
}

/// The card's title: the tool's name, or for a batch from several tools
/// its count.
pub fn card_title(transfer: &MapTransfer, words: &MonitorFlowMapCopy) -> String {
    if transfer.mixed_tools {
        traces_count(words, transfer.count)
    } else {
        tool_name(&transfer.tool)
    }
}

/// A tool's name by its wire name, or the wire name itself.
pub fn tool_name(tool: &str) -> String {
    super::traces_tree::SourceKind::from_wire(tool)
        .map_or_else(|| tool.to_string(), |kind| kind.display_name().to_string())
}

/// The heading over the trace: "Trace", or a batch's count.
pub fn card_eyebrow(transfer: &MapTransfer, words: &MonitorFlowMapCopy) -> String {
    if transfer.count > 1 {
        traces_count(words, transfer.count)
    } else {
        words.transfer_trace.to_string()
    }
}

/// The trace's folder: the core's name, else the map's, else its id.
pub fn card_folder(transfer: &MapTransfer, from_map: Option<&str>) -> String {
    transfer
        .folder_label
        .clone()
        .or_else(|| from_map.map(str::to_string))
        .unwrap_or_else(|| transfer.folder.clone())
}

/// When the trace was recorded, as an abbreviated date and a short time
/// in local time; `None` when unrecorded.
pub fn card_time(at: Option<DateTime<Utc>>) -> Option<String> {
    at.map(|at| {
        at.with_timezone(&chrono::Local)
            .format("%b %-d, %Y %H:%M")
            .to_string()
    })
}

/// The line under the pad while a run plays: a contribution's gathering,
/// "Collected 2 of 3", while several traces wait for lift-off.
///
/// Platform difference: after a keep the Mac says "Kept on this Mac"; that
/// word names the Mac, so this shell says nothing there.
pub fn line_under_pad(run: &Run, t: f64, words: &MonitorFlowMapCopy) -> Option<String> {
    let arrived = run.arrived(t);
    match run.action() {
        Action::Contribute
            if run.style == MapStyle::Launch
                && run.transfers.len() > 1
                && arrived > 0
                && t < run.lift_off() =>
        {
            Some(
                words
                    .collected
                    .replace("{count}", &arrived.to_string())
                    .replace("{total}", &run.transfers.len().to_string()),
            )
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::monitor::map_scene;

    fn run(count: usize, action: Action) -> Run {
        let tools = ["claude-code", "codex", "opencode"];
        Run::new(
            (0..count)
                .map(|i| MapTransfer::new(&format!("t{i}"), &format!("f{i}"), tools[i % 3], action))
                .collect(),
            0.0,
            MapStyle::Launch,
        )
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-4
    }

    fn row(id: &str, project: &str, source: Option<&str>, at: Option<i64>) -> HistoryRecord {
        HistoryRecord {
            submission_id: id.into(),
            project_id: project.into(),
            project_label: project.into(),
            source: source.map(str::to_string),
            submitted_at: at.map(|s| DateTime::from_timestamp(s, 0).unwrap()),
            ..Default::default()
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

    // A run (TracesMapPileTests).

    #[test]
    fn a_traces_legs_follow_one_another() {
        let run = run(2, Action::Contribute);
        assert_eq!(run.leg(0, 0.0), Leg::Inbound(0.0));
        assert_eq!(run.leg(1, 0.0), Leg::Waiting);
        let Leg::Orbit(round) = run.leg(0, Run::INBOUND + Run::ORBIT / 2.0) else {
            panic!("the trace goes round its coin");
        };
        assert!(close(round, 0.5));
        assert!(matches!(
            run.leg(0, Run::INBOUND + Run::ORBIT + 0.1),
            Leg::Outbound(_)
        ));
        assert!(
            matches!(run.leg(1, Run::STEP + 0.1), Leg::Inbound(_)),
            "the second sets off"
        );
        assert_eq!(run.arrived(run.all_arrived()), 2);
        assert_eq!(run.arrived(run.all_arrived() - 0.01), 1);
        assert!(!run.moving(run.all_arrived()));
    }

    #[test]
    fn the_rocket_waits_for_every_trace_then_climbs_faster() {
        let run = run(3, Action::Contribute);
        assert_eq!(run.flight(run.all_arrived()), None);
        assert_eq!(run.flight(run.lift_off() - 0.01), None);
        assert_eq!(run.flight(run.lift_off()), Some(0.0));
        assert_eq!(run.flight(run.lift_off() + Run::FLIGHT * 2.0), Some(1.0));
        assert_eq!(Run::climb(0.2), 0.0, "still through ignition");
        assert!(close(Run::climb(1.0), 1.0));
        let steps: Vec<f64> = (3..=10).map(|i| Run::climb(f64::from(i) / 10.0)).collect();
        let gains: Vec<f64> = steps.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gains.iter().all(|g| *g > 0.0));
        assert!(gains.windows(2).all(|w| w[1] > w[0]), "it speeds up");
        assert!(close(run.end(), run.lift_off() + Run::FLIGHT + Run::LINGER));
        let kept = self::run(1, Action::Keep);
        assert_eq!(kept.flight(kept.end()), None, "a kept trace has no rocket");
        assert!(close(kept.end(), kept.all_arrived() + Run::KEPT_LINGER));
    }

    #[test]
    fn a_traces_coin_shuffles_to_the_front_and_grows() {
        let run = run(2, Action::Contribute);
        let base: Vec<String> = ["tool:opencode", "tool:claude-code", "tool:codex"]
            .map(String::from)
            .to_vec();
        assert_eq!(run.order(&base, -1.0), base);
        assert_eq!(run.order(&base, 0.0)[0], "tool:claude-code");
        assert_eq!(run.order(&base, Run::STEP)[0], "tool:codex");
        assert!(run.orbits(0.0).is_empty());
        let orbits: Vec<String> = run.orbits(Run::INBOUND).into_iter().map(|o| o.0).collect();
        assert_eq!(orbits, ["tool:claude-code"]);
        assert_eq!(run.current(0.0).map(|t| t.id.as_str()), Some("t0"));
        assert_eq!(
            run.current(Run::STEP + 0.1).map(|t| t.id.as_str()),
            Some("t1")
        );
        assert_eq!(run.current(run.all_arrived()), None);
    }

    #[test]
    fn the_standard_map_raises_each_trace_to_the_orb() {
        let mut run = run(2, Action::Contribute);
        run.style = MapStyle::Standard;
        assert!(run.rises());
        let rising = Run::INBOUND + Run::ORBIT + Run::OUTBOUND + 0.1;
        assert!(matches!(run.leg(0, rising), Leg::Ascent(_)));
        assert!(close(
            run.all_arrived(),
            Run::STEP + rising - 0.1 + Run::ASCENT
        ));
        assert_eq!(
            run.flight(run.end()),
            None,
            "the standard map has no rocket"
        );
        assert!(close(run.end(), run.all_arrived() + Run::LINGER));
        let mut kept = self::run(1, Action::Keep);
        kept.style = MapStyle::Standard;
        assert!(!kept.rises());
        assert!(
            matches!(kept.leg(0, rising), Leg::Arrived(_)),
            "a kept trace ends at this computer"
        );
    }

    #[test]
    fn a_trace_rides_its_arc_from_end_to_end() {
        let arc = Arc {
            from: Point::new(0.0, 0.0),
            control1: Point::new(10.0, 0.0),
            control2: Point::new(20.0, 10.0),
            to: Point::new(30.0, 10.0),
            style: map_scene::ArcStyle::Flowing,
            dim: 1.0,
        };
        assert_eq!(point_on(&arc, 0.0), arc.from);
        assert_eq!(point_on(&arc, 1.0), arc.to);
        assert_eq!(ease(0.0), 0.0);
        assert_eq!(ease(1.0), 1.0);
        assert!(close(ease(0.5), 0.5));
        assert!(ease(0.25) < 0.25 && ease(0.75) > 0.75, "slow at both ends");
    }

    // Finding transfers.

    #[test]
    fn contributions_are_rows_history_gained() {
        let t0 = 1_800_000_000;
        let old = vec![row("a", "p1", Some("claude-code"), Some(t0))];
        let new = vec![
            row("c", "p2", Some("codex"), Some(t0 + 20)),
            row("b", "p1", Some("claude-code"), Some(t0 + 10)),
            row("a", "p1", Some("claude-code"), Some(t0)),
            row("older", "p1", Some("claude-code"), Some(t0 - 100)),
            row("nofolder", "", Some("codex"), Some(t0 + 30)),
            row("notool", "p1", None, Some(t0 + 30)),
            row("unsent", "p1", Some("codex"), None),
        ];
        assert!(
            contributions(None, Some(&new[..])).is_empty(),
            "the first read finds none"
        );
        assert!(contributions(Some(&old[..]), None).is_empty());
        let found = contributions(Some(&old[..]), Some(&new[..]));
        let ids: Vec<&str> = found.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["b", "c"]);
        let folders: Vec<&str> = found.iter().map(|t| t.folder.as_str()).collect();
        assert_eq!(folders, ["p1", "p2"]);
        let tools: Vec<&str> = found.iter().map(|t| t.tool.as_str()).collect();
        assert_eq!(tools, ["claude-code", "codex"]);
        assert!(found.iter().all(|t| t.action == Action::Contribute));
        let fresh: Vec<String> = contributions(Some(&[][..]), Some(&old[..]))
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(fresh, ["a"], "nothing contributed before: every row is new");
    }

    #[test]
    fn a_keep_and_a_dismissal_are_found_among_the_listed_traces() {
        let listed = vec![entry("1", "p1", "codex"), entry("2", "p2", "claude-code")];
        let kept = kept(Some("2"), &listed).expect("found");
        assert_eq!(kept.folder, "p2");
        assert_eq!(kept.tool, "claude-code");
        assert_eq!(kept.action, Action::Keep);
        assert_eq!(super::kept(Some("missing"), &listed), None);
        assert_eq!(super::kept(None, &listed), None);
        let gone = dismissed(Some("1"), &listed).expect("found");
        assert_eq!(gone.action, Action::Dismiss);
        assert_eq!(gone.folder_label.as_deref(), Some("p1"));
        assert_eq!(dismissed(Some("missing"), &listed), None);
    }

    // The player.

    #[test]
    fn the_player_batches_by_action_and_shows_the_trace() {
        let mut player = Player::default();
        let one = MapTransfer::new("1", "f", "codex", Action::Contribute);
        let two = MapTransfer::new("2", "f", "codex", Action::Contribute);
        let keep = MapTransfer::new("3", "f", "codex", Action::Keep);
        player.enqueue(vec![one.clone(), two, keep], 0.0);
        let ids = |p: &Player| -> Vec<String> {
            p.run()
                .map(|r| r.transfers.iter().map(|t| t.id.clone()).collect())
                .unwrap_or_default()
        };
        assert_eq!(ids(&player), ["1", "2"]);
        let cards: Vec<&str> = player.cards().iter().map(|c| c.id.as_str()).collect();
        assert_eq!(cards, ["batch:1"], "one card for the batch, as it sets off");
        assert_eq!(player.cards()[0].count, 2);
        assert!(!player.cards()[0].mixed_folders);
        assert_eq!(
            MapTransfer::batch(std::slice::from_ref(&one)),
            Some(one.clone())
        );
        let other = MapTransfer::new("4", "g", "claude-code", Action::Contribute);
        let mixed = MapTransfer::batch(&[one.clone(), other]).unwrap();
        assert!(mixed.mixed_folders && mixed.mixed_tools);
        player.enqueue(vec![one], 0.1);
        assert_eq!(ids(&player), ["1", "2"], "a trace plays once");
        player.style = MapStyle::Standard;
        assert_eq!(
            player.run().map(|r| r.style),
            Some(MapStyle::Launch),
            "a run playing keeps its style"
        );
    }

    #[test]
    fn the_next_run_plays_when_the_first_ends() {
        let mut player = Player::default();
        let one = MapTransfer::new("1", "f", "codex", Action::Contribute);
        let keep = MapTransfer::new("3", "f", "codex", Action::Keep);
        player.enqueue(vec![one, keep], 0.0);
        let end = player.run().unwrap().end();
        assert_eq!(player.next_deadline(), Some(end));
        assert!(!player.tick(end - 0.01), "nothing is over yet");
        assert!(player.tick(end));
        let run = player.run().expect("the keep plays next");
        assert_eq!(run.action(), Action::Keep);
        assert_eq!(run.start, end, "it starts when the first ends");
        // Both cards stay until their hold is out.
        assert_eq!(player.cards().len(), 2);
        let keep_end = end + run.end();
        player.tick(keep_end);
        assert!(player.run().is_none(), "nothing more to play");
        assert_eq!(player.cards().len(), 2, "the cards outlast their runs");
        player.tick(end + Player::HOLD);
        assert_eq!(player.cards().len(), 1, "the first card has gone");
        player.tick(keep_end + Player::HOLD);
        assert!(player.cards().is_empty());
        assert_eq!(player.next_deadline(), None);
    }

    #[test]
    fn a_dismissal_only_shows_its_card() {
        let mut player = Player::default();
        let gone = MapTransfer::new("9", "f", "codex", Action::Dismiss);
        player.enqueue(vec![gone.clone()], 0.0);
        assert!(player.run().is_none());
        player.announce(gone.clone(), 0.0);
        assert_eq!(player.front(), Some(&gone));
        assert!(player.run().is_none());
        assert_eq!(player.next_deadline(), Some(Player::HOLD));
        player.tick(Player::HOLD);
        assert!(player.front().is_none(), "it goes after its hold");
    }

    #[test]
    fn cards_stack_and_close_one_at_a_time() {
        let mut player = Player::default();
        player.enqueue(
            vec![MapTransfer::new("a", "f", "codex", Action::Contribute)],
            0.0,
        );
        let gone: Vec<MapTransfer> = (0..Player::STACK_LIMIT)
            .map(|i| MapTransfer::new(&format!("d{i}"), "f", "codex", Action::Dismiss))
            .collect();
        player.announce(gone[0].clone(), 0.0);
        let ids = |p: &Player| -> Vec<String> { p.cards().iter().map(|c| c.id.clone()).collect() };
        assert_eq!(ids(&player), ["a", "d0"]);
        assert_eq!(player.front().map(|c| c.id.as_str()), Some("d0"));
        player.close("d0");
        assert_eq!(
            player.front().map(|c| c.id.as_str()),
            Some("a"),
            "the card behind comes forward"
        );
        assert!(
            player.run().is_some(),
            "closing a card leaves its run playing"
        );
        for transfer in gone.into_iter().skip(1) {
            player.announce(transfer, 0.0);
        }
        player.announce(MapTransfer::new("late", "f", "codex", Action::Dismiss), 0.0);
        assert_eq!(player.cards().len(), Player::STACK_LIMIT);
        assert_eq!(player.front().map(|c| c.id.as_str()), Some("late"));
        assert!(
            !ids(&player).contains(&"a".to_string()),
            "the oldest goes past the limit"
        );
    }

    #[test]
    fn the_stack_draws_the_newest_in_front() {
        let mut player = Player::default();
        for i in 0..5 {
            player.announce(
                MapTransfer::new(&i.to_string(), "f", "codex", Action::Dismiss),
                0.0,
            );
        }
        let visible: Vec<&str> = player.visible().iter().map(|c| c.id.as_str()).collect();
        assert_eq!(visible, ["4", "3", "2"]);
        assert!(Player::default().visible().is_empty());
    }

    // The feed (MapTransferFeed).

    #[test]
    fn the_feed_plays_what_history_gained_and_never_the_first_read() {
        let mut feed = Feed::default();
        let mut player = Player::default();
        let first = vec![row("a", "p1", Some("codex"), Some(100))];
        feed.observe(&mut player, &[], Some(&first[..]), None, None, 0.0);
        assert!(player.run().is_none(), "what left before the window opened");
        let second = vec![
            row("b", "p1", Some("codex"), Some(200)),
            row("a", "p1", Some("codex"), Some(100)),
        ];
        feed.observe(&mut player, &[], Some(&second[..]), None, None, 1.0);
        let run = player.run().expect("the new row plays");
        assert_eq!(run.transfers[0].id, "b");
        // The same read again plays nothing more.
        feed.observe(&mut player, &[], Some(&second[..]), None, None, 2.0);
        assert_eq!(player.run().unwrap().transfers.len(), 1);
    }

    #[test]
    fn the_feed_finds_a_dismissal_after_the_reload_dropped_it() {
        let mut feed = Feed::default();
        let mut player = Player::default();
        let listed = vec![entry("1", "p1", "codex"), entry("2", "p2", "codex")];
        feed.observe(&mut player, &listed, None, None, None, 0.0);
        // The reload has already dropped it when the dismissal is seen.
        let after = vec![entry("2", "p2", "codex")];
        feed.observe(&mut player, &after, None, None, Some("1"), 1.0);
        let front = player.front().expect("its card shows");
        assert_eq!(front.id, "1");
        assert_eq!(front.action, Action::Dismiss);
        assert!(player.run().is_none(), "nothing moves on the map");
        // Seen again, it does not show twice.
        player.close("1");
        feed.observe(&mut player, &after, None, None, Some("1"), 2.0);
        assert!(player.front().is_none());
    }

    #[test]
    fn the_feed_plays_a_keep_as_a_run() {
        let mut feed = Feed::default();
        let mut player = Player::default();
        let listed = vec![entry("1", "p1", "codex")];
        feed.observe(&mut player, &listed, None, None, None, 0.0);
        feed.observe(&mut player, &listed, None, Some("1"), None, 1.0);
        assert_eq!(player.run().map(Run::action), Some(Action::Keep));
    }

    // Motion (FlowMapMotionTests).

    #[test]
    fn resting_motion_runs_only_while_something_moves() {
        let dashes = resting_motion(false, true, true, false);
        assert_eq!(dashes.interval, 1.0 / 30.0);
        assert!(!dashes.paused);
        let both = resting_motion(false, true, true, true);
        assert_eq!(both.interval, 1.0 / 30.0);
        assert!(!both.paused);
        let library = resting_motion(false, false, true, true);
        assert_eq!(library.interval, LIBRARY_INTERVAL);
        assert!(!library.paused);
        // Unhealthy flows are drawn still: only the library turns.
        let unhealthy = resting_motion(false, true, false, true);
        assert_eq!(unhealthy.interval, LIBRARY_INTERVAL);
        assert!(!unhealthy.paused);
        assert!(resting_motion(false, false, true, false).paused);
        assert!(resting_motion(false, true, false, false).paused);
        assert!(resting_motion(true, true, true, true).paused);
    }

    #[test]
    fn reduced_motion_draws_no_run_and_no_moving_dash() {
        let run = run(1, Action::Contribute);
        assert!(drawn_run(Some(&run), false).is_some());
        assert!(drawn_run(Some(&run), true).is_none());
        assert_eq!(dash_phase(10.0, true), 0.0);
        assert_eq!(dash_phase(10.0, false), 180.0);
    }

    // The card.

    #[test]
    fn the_card_says_what_is_happening_in_the_cores_words() {
        let words = map_scene::words();
        let (tag, tone, note) = card_state(Action::Contribute, Some("Codex"), &words);
        assert_eq!(tag, words.transfer_contributing);
        assert_eq!(tone, Tone::On);
        assert!(note.unwrap().contains("Codex"));
        let (_, _, mixed) = card_state(Action::Contribute, None, &words);
        assert_eq!(mixed.as_deref(), Some(words.transfer_contribute_note_mixed));
        let (tag, _, note) = card_state(Action::Dismiss, Some("Codex"), &words);
        assert_eq!(tag, words.transfer_dismissed);
        assert_eq!(note.as_deref(), Some(words.transfer_dismiss_note));
        // A keep's note names the Mac: none here.
        assert_eq!(card_state(Action::Keep, None, &words).2, None);

        let mut kept = MapTransfer::new("1", "p1", "codex", Action::Keep);
        assert_eq!(card_folder(&kept, Some("from the map")), "from the map");
        kept.folder_label = Some("payments".into());
        assert_eq!(card_folder(&kept, Some("ignored")), "payments");
        assert_eq!(
            card_folder(&MapTransfer::new("1", "p1", "x", Action::Keep), None),
            "p1"
        );
        assert_eq!(card_title(&kept, &words), "Codex");
        assert_eq!(card_eyebrow(&kept, &words), words.transfer_trace);
        assert_eq!(card_time(None), None);
        assert!(card_time(Some(DateTime::from_timestamp(1_800_000_000, 0).unwrap())).is_some());
    }

    #[test]
    fn a_mixed_batch_names_no_tool() {
        let words = map_scene::words();
        let one = MapTransfer::new("1", "f", "codex", Action::Contribute);
        let other = MapTransfer::new("2", "g", "claude-code", Action::Contribute);
        let mixed = MapTransfer::batch(&[other, one]).unwrap();
        assert_eq!(card_title(&mixed, &words), traces_count(&words, 2));
        assert_eq!(card_eyebrow(&mixed, &words), traces_count(&words, 2));
        let note = card_state(Action::Contribute, None, &words).2.unwrap();
        assert!(!note.contains('{'));
        for kind in super::super::traces_tree::SourceKind::ALL {
            assert!(
                !note.contains(kind.display_name()),
                "{}",
                kind.display_name()
            );
        }
    }

    #[test]
    fn the_pad_counts_a_contributions_gathering_only() {
        let words = map_scene::words();
        let three = run(3, Action::Contribute);
        assert_eq!(
            line_under_pad(&three, 0.0, &words),
            None,
            "none arrived yet"
        );
        let line = line_under_pad(&three, three.all_arrived() - Run::STEP + 0.01, &words).unwrap();
        assert!(line.contains('2') && line.contains('3'), "{line}");
        assert_eq!(line_under_pad(&three, three.lift_off(), &words), None);
        assert_eq!(
            line_under_pad(&run(1, Action::Contribute), 5.0, &words),
            None
        );
        let kept = run(2, Action::Keep);
        assert_eq!(line_under_pad(&kept, kept.all_arrived(), &words), None);
    }
}
