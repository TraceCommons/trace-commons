//! The first run's stage as a widget: one drawing under the whole
//! first-run window, as the Mac's `FirstRunSteps` holds one scene behind
//! every step (`OnboardingCoordinatorView`).
//!
//! Each step's page carries an empty slot where the Mac draws its graphic,
//! and the thread row carries an empty place for each stretch between two
//! titles; the stage finds them where GTK laid them out and draws there,
//! under the page. The N and the tool tiles are the same objects on every
//! step (`firstRunActor`): when the step changes, each moves from where it
//! was drawn to its place on the new step along the Mac's spatial spring,
//! a tile the new step does not show fades out, and one it newly shows
//! fades in.
//!
//! It animates at most 30 times a second, as the Mac's timelines do, and
//! only while mapped: the tick is added on map and removed on unmap. With
//! GTK's animations off (`gtk-enable-animations`), as under the Mac's Reduce
//! Motion, it draws each step's final frame and moves nothing.
//!
//! It draws no words. The thread's titles are real labels (the core's
//! `frame.step_*`), and a step already passed is a button back to it.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::first_run_stage::{self as stage, Frame};
use super::first_run_stage_paint::{self as paint, Palette, Scene};
use super::lift::Lift;
use crate::first_run::{Sharing, Step, tools::Tools};

/// How long a tile fades in or out between steps.
const FADE: f64 = 0.35;
/// The thread's stretch grows over this long (`timed(duration: 0.52)`).
const THREAD_GROW: f64 = 0.52;
/// At most 30 frames a second, as the Mac's timelines.
const FRAME_MICROS: i64 = 33_000;

fn now() -> f64 {
    glib::monotonic_time() as f64 / 1e6
}

#[derive(Default)]
struct Moves {
    /// Where the N was drawn when the step last changed, and when.
    hub_from: Option<Frame>,
    tiles_from: BTreeMap<String, Frame>,
    moved_at: f64,
    /// What was last drawn, in the stage's own coordinates.
    hub_drawn: Option<Frame>,
    tiles_drawn: BTreeMap<String, Frame>,
    /// The tiles last drawn lit (watched, on Tools).
    lit_drawn: Vec<String>,
}

struct Inner {
    step: Step,
    agent: bool,
    open: bool,
    slots: Vec<(Step, gtk::Widget)>,
    segments: Vec<gtk::Widget>,
    /// Each stretch's grown share when the step last changed.
    segments_from: Vec<f64>,
    moves: Moves,
    opened_at: f64,
    welcome_at: f64,
    /// When Welcome's lines were last all set in place.
    welcome_risen_at: Option<f64>,
    scan_started: Option<f64>,
    scanned: bool,
    sharing: Sharing,
    sharing_at: f64,
    signed_in_at: Option<f64>,
    /// Whether Join last read an account signed in, on this visit.
    join_known: Option<bool>,
    /// Traces' count has risen once: going back to it does not replay it
    /// (`FirstRunStageMemory.tracesCounted`).
    traces_counted: bool,
    /// The scroller the steps sit in: the graphic is clipped to it.
    scroller: Option<gtk::Widget>,
    tick: Option<gtk::TickCallbackId>,
    last_frame: i64,
    /// How many times the stage has been drawn.
    draws: u64,
}

pub(super) struct Stage {
    area: gtk::DrawingArea,
    thread: gtk::Box,
    tools: Rc<RefCell<Tools>>,
    inner: RefCell<Inner>,
    started: Cell<bool>,
    /// The stage's colours, read once: the first run is dark only.
    palette: Palette,
    /// Welcome's lines and when each rises (`WelcomeScreen`'s `Rise`).
    rises: RefCell<Vec<(Lift, f64)>>,
    /// The thread's buttons back to a passed step.
    backs: RefCell<Vec<gtk::Button>>,
    /// Uses' captions laid out again at the stage's scale.
    captions: RefCell<Option<Rc<dyn Fn()>>>,
}

fn animations() -> bool {
    gtk::Settings::default().is_none_or(|s| s.is_gtk_enable_animations())
}

impl Stage {
    pub(super) fn new(tools: Rc<RefCell<Tools>>) -> Rc<Self> {
        let area = gtk::DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .can_target(false)
            .can_focus(false)
            .build();
        let thread = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        thread.set_halign(gtk::Align::Center);
        thread.set_margin_top(stage::THREAD_TOP as i32);
        thread.set_height_request(stage::THREAD_HEIGHT as i32);
        thread.set_visible(false);
        let at = now();
        let stage = Rc::new(Self {
            area,
            thread,
            tools,
            inner: RefCell::new(Inner {
                step: Step::Welcome,
                agent: false,
                open: false,
                slots: Vec::new(),
                segments: Vec::new(),
                segments_from: Vec::new(),
                moves: Moves::default(),
                opened_at: at,
                welcome_at: at,
                welcome_risen_at: None,
                scan_started: None,
                scanned: false,
                sharing: Sharing::Ask,
                sharing_at: at,
                signed_in_at: None,
                join_known: None,
                traces_counted: false,
                scroller: None,
                tick: None,
                last_frame: 0,
                draws: 0,
            }),
            started: Cell::new(false),
            palette: Palette::scheme(true),
            rises: RefCell::new(Vec::new()),
            backs: RefCell::new(Vec::new()),
            captions: RefCell::new(None),
        });
        let weak = Rc::downgrade(&stage);
        stage.area.set_draw_func(move |_, cr, w, h| {
            if let Some(stage) = weak.upgrade() {
                stage.draw(cr, f64::from(w), f64::from(h));
            }
        });
        let weak = Rc::downgrade(&stage);
        stage.area.connect_resize(move |_, w, h| {
            // Not during the allocation that reported the size: sizing the
            // slots queues another layout.
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(stage) = weak.upgrade() {
                    stage.size_places(f64::from(w), f64::from(h));
                }
            });
        });
        let weak = Rc::downgrade(&stage);
        stage.area.connect_map(move |_| {
            if let Some(stage) = weak.upgrade() {
                stage.start_ticking();
            }
        });
        let weak = Rc::downgrade(&stage);
        stage.area.connect_unmap(move |_| {
            if let Some(stage) = weak.upgrade() {
                stage.stop_ticking();
            }
        });
        if let Some(settings) = gtk::Settings::default() {
            let weak = Rc::downgrade(&stage);
            settings.connect_gtk_enable_animations_notify(move |_| {
                if let Some(stage) = weak.upgrade() {
                    stage.stop_ticking();
                    if stage.area.is_mapped() {
                        stage.start_ticking();
                    }
                    stage.raise_lines();
                    stage.area.queue_draw();
                }
            });
        }
        stage
    }

    /// `content` over the stage: the stage under it, the size its own.
    pub(super) fn wrap(&self, content: &impl IsA<gtk::Widget>) -> gtk::Overlay {
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&self.area));
        overlay.add_overlay(content);
        overlay.set_measure_overlay(content, true);
        overlay
    }

    /// The thread row, drawn over every step but Welcome.
    pub(super) fn thread(&self) -> &gtk::Box {
        &self.thread
    }

    /// An empty slot where `step` draws its graphic, to put at the top of
    /// its page.
    pub(super) fn slot(&self, step: Step) -> gtk::Box {
        let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        slot.set_can_target(false);
        slot.set_height_request(stage::graphic_height(step) as i32);
        self.inner
            .borrow_mut()
            .slots
            .push((step, slot.clone().upcast()));
        slot
    }

    fn size_places(&self, w: f64, h: f64) {
        let scale = stage::fit_scale(w, h);
        let inner = self.inner.borrow();
        for (step, slot) in &inner.slots {
            slot.set_height_request((stage::graphic_height(*step) * scale).round() as i32);
        }
        for segment in &inner.segments {
            segment.set_width_request((stage::SEGMENT_WIDTH * scale).round() as i32);
        }
        drop(inner);
        let relayout = self.captions.borrow().clone();
        if let Some(relayout) = relayout {
            relayout();
        }
    }

    /// The scale the canvas is drawn at now.
    pub(super) fn scale(&self) -> f64 {
        stage::fit_scale(
            f64::from(self.area.width().max(1)),
            f64::from(self.area.height().max(1)),
        )
    }

    /// Uses' captions, laid out again by `relayout` when the scale
    /// changes.
    pub(super) fn set_captions(&self, relayout: Rc<dyn Fn()>) {
        *self.captions.borrow_mut() = Some(relayout);
    }

    /// Something the stage draws changed outside it (Tools found or
    /// watched a tool): draw again. With animations off nothing else
    /// would, as no tick runs.
    pub(super) fn refresh(&self) {
        self.area.queue_draw();
    }

    /// Welcome's lines, each with the delay it rises after: hidden and
    /// lowered until then, unless motion is reduced.
    pub(super) fn set_rises(&self, rises: Vec<(Lift, f64)>) {
        *self.rises.borrow_mut() = rises;
        self.raise_lines();
    }

    /// Welcome's lines where they are this moment, on Welcome's clock.
    fn raise_lines(&self) {
        let reduce = !animations();
        let elapsed = now() - self.inner.borrow().welcome_at;
        for (lift, delay) in self.rises.borrow().iter() {
            lift.set_risen(stage::welcome::risen(elapsed, *delay, reduce));
        }
    }

    /// Whether Welcome's lines still have to be raised: until a frame
    /// past the last one's rise has set every line in place, however late
    /// that frame comes.
    fn lines_rising(&self) -> bool {
        let inner = self.inner.borrow();
        inner.step == Step::Welcome
            && !inner.agent
            && inner
                .welcome_risen_at
                .is_none_or(|at| at < inner.welcome_at)
            && !self.rises.borrow().is_empty()
    }

    /// The thread's buttons back are offered only while no commit runs.
    pub(super) fn set_enabled(&self, enabled: bool) {
        for button in self.backs.borrow().iter() {
            button.set_sensitive(enabled);
        }
    }

    fn start_ticking(self: &Rc<Self>) {
        if !animations() || self.inner.borrow().tick.is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let id = self.area.add_tick_callback(move |area, clock| {
            let Some(stage) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let frame = clock.frame_time();
            let due = {
                let mut inner = stage.inner.borrow_mut();
                let due = frame - inner.last_frame >= FRAME_MICROS;
                if due {
                    inner.last_frame = frame;
                }
                due
            };
            if due {
                if stage.lines_rising() {
                    stage.raise_lines();
                    let mut inner = stage.inner.borrow_mut();
                    let at = now();
                    if at - inner.welcome_at > stage::welcome::RISEN_BY {
                        inner.welcome_risen_at = Some(at);
                    }
                }
                area.queue_draw();
            }
            glib::ControlFlow::Continue
        });
        self.inner.borrow_mut().tick = Some(id);
    }

    fn stop_ticking(&self) {
        if let Some(id) = self.inner.borrow_mut().tick.take() {
            id.remove();
        }
    }

    /// The step on screen changed to `step`. `on_pick` takes a passed step
    /// the thread offers back; `enabled` is false while a commit runs.
    pub(super) fn set_step(self: &Rc<Self>, step: Step, enabled: bool, on_pick: Rc<dyn Fn(Step)>) {
        let at = now();
        {
            let mut inner = self.inner.borrow_mut();
            let first = !self.started.replace(true);
            let changed = inner.step != step || first;
            if changed {
                let before = thread_grown(
                    inner.step,
                    inner.segments_from.len(),
                    at,
                    &inner,
                    inner.moves.moved_at,
                );
                inner.segments_from = before;
                let moves = &mut inner.moves;
                moves.hub_from = if first { None } else { moves.hub_drawn };
                moves.tiles_from = moves.tiles_drawn.clone();
                moves.moved_at = at;
                inner.opened_at = if step == Step::Traces && inner.traces_counted {
                    at - 3600.0
                } else {
                    at
                };
                inner.join_known = None;
                if step == Step::Welcome {
                    inner.welcome_at = at;
                }
                inner.step = step;
            }
        }
        self.rebuild_thread(step, enabled, on_pick);
        if step == Step::Welcome {
            self.raise_lines();
        }
        self.area.queue_draw();
    }

    /// Welcome's agent prompt is up (no N), or gone. Coming back from it
    /// replays Welcome, as the Mac's `onAppear` resets its clock.
    pub(super) fn set_agent(&self, agent: bool) {
        let back = {
            let mut inner = self.inner.borrow_mut();
            let back = inner.agent && !agent;
            inner.agent = agent;
            if back {
                inner.welcome_at = now();
            }
            back
        };
        if back {
            self.raise_lines();
        }
        self.area.queue_draw();
    }

    /// Customize is open on Uses: the graphic gives way to the card.
    pub(super) fn set_open(&self, open: bool) {
        let inner = self.inner.borrow();
        for (step, slot) in &inner.slots {
            if *step == Step::Uses {
                slot.set_visible(!open);
            }
        }
        drop(inner);
        self.inner.borrow_mut().open = open;
        self.area.queue_draw();
    }

    /// The sharing path chosen on Uses: the flow restarts from it.
    pub(super) fn set_sharing(&self, sharing: Sharing) {
        let mut inner = self.inner.borrow_mut();
        if inner.sharing != sharing {
            inner.sharing = sharing;
            inner.sharing_at = now();
        }
        drop(inner);
        self.area.queue_draw();
    }

    /// Join read whether an account is signed in. A sign-in while Join is
    /// up sends a ring of light out from the N; one already held when Join
    /// opens does not (the Mac's `onChange` of `signedIn`).
    pub(super) fn join_signed_in(&self, signed: bool) {
        let mut inner = self.inner.borrow_mut();
        if inner.join_known == Some(false) && signed {
            inner.signed_in_at = Some(now());
        }
        inner.join_known = Some(signed);
        drop(inner);
        self.area.queue_draw();
    }

    /// The steps scroll in `scroller`: the stage follows it, and draws the
    /// graphic only inside it, never under the header or the thread.
    pub(super) fn follow(self: &Rc<Self>, scroller: &gtk::ScrolledWindow) {
        let weak = Rc::downgrade(self);
        scroller.vadjustment().connect_value_changed(move |_| {
            if let Some(stage) = weak.upgrade() {
                stage.area.queue_draw();
            }
        });
        self.inner.borrow_mut().scroller = Some(scroller.clone().upcast());
    }

    fn rebuild_thread(self: &Rc<Self>, step: Step, enabled: bool, on_pick: Rc<dyn Fn(Step)>) {
        while let Some(child) = self.thread.first_child() {
            self.thread.remove(&child);
        }
        self.backs.borrow_mut().clear();
        let Some(current) = stage::thread_index(step) else {
            self.thread.set_visible(false);
            self.inner.borrow_mut().segments.clear();
            return;
        };
        self.thread.set_visible(true);
        let words = trace_commons_contributor::first_run_copy::first_run_copy().frame;
        let scale = stage::fit_scale(
            f64::from(self.area.width().max(1)),
            f64::from(self.area.height().max(1)),
        );
        let mut segments = Vec::new();
        for (index, thread_step) in stage::THREAD_STEPS.iter().enumerate() {
            if index > 0 {
                let place = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                place.set_size_request((stage::SEGMENT_WIDTH * scale).round() as i32, 3);
                place.set_valign(gtk::Align::Center);
                place.set_can_target(false);
                self.thread.append(&place);
                segments.push(place.upcast::<gtk::Widget>());
            }
            let title = match thread_step {
                Step::Tools => words.step_tools,
                Step::PrivateAi => words.step_private_ai,
                Step::Traces => words.step_traces,
                Step::Uses => words.step_uses,
                Step::Terms => words.step_terms,
                Step::Join | Step::Welcome => words.step_join,
            };
            let label = gtk::Label::new(Some(title));
            label.add_css_class("tc-thread-title");
            if index == current {
                label.add_css_class("tc-thread-current");
                // The Mac's `.isSelected` on the current step's title.
                label.update_state(&[gtk::accessible::State::Selected(Some(true))]);
                self.thread.append(&label);
            } else if index < current {
                label.add_css_class("tc-thread-passed");
                let button = gtk::Button::builder().child(&label).build();
                button.add_css_class("flat");
                button.add_css_class("tc-thread-back");
                button.set_sensitive(enabled);
                let pick = on_pick.clone();
                let target = *thread_step;
                button.connect_clicked(move |_| pick(target));
                self.thread.append(&button);
                self.backs.borrow_mut().push(button);
            } else {
                self.thread.append(&label);
            }
        }
        let mut inner = self.inner.borrow_mut();
        let count = segments.len();
        inner.segments = segments;
        inner.segments_from.resize(count, 0.0);
    }

    /// What the stage knows of the tools this moment.
    fn tools_seen(&self) -> (stage::ToolsSeen, bool) {
        let tools = self.tools.borrow();
        let discovered = tools.discovered.is_some();
        let rows: Vec<(&str, bool)> = tools
            .discovered
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|f| (f.source.as_str(), f.exists))
            .collect();
        let seen = stage::ToolsSeen {
            rows: stage::ring_rows(&rows)
                .into_iter()
                .map(|(k, e)| (k.to_owned(), e))
                .collect(),
            watched: tools
                .watched_kinds()
                .iter()
                .map(|k| (*k).to_owned())
                .collect(),
            connectable: {
                let connectable = tools.connectable();
                stage::RING_ORDER
                    .iter()
                    .filter(|k| connectable.iter().any(|f| f.source == **k))
                    .map(|k| (*k).to_owned())
                    .collect()
            },
        };
        (seen, discovered)
    }

    fn draw(&self, cr: &gtk::cairo::Context, w: f64, h: f64) {
        let reduce = !animations();
        let at = now();
        let p = &self.palette;
        let (tools, discovered) = self.tools_seen();
        let mut inner = self.inner.borrow_mut();
        inner.draws += 1;
        // The scan runs once per first run, when Tools first has rows.
        if inner.step == Step::Tools && discovered && !inner.scanned && inner.scan_started.is_none()
        {
            if reduce || tools.rows.is_empty() {
                inner.scanned = true;
            } else {
                inner.scan_started = Some(at);
            }
        }
        if let Some(started) = inner.scan_started {
            if at - started >= stage::SCAN_TOTAL || reduce {
                inner.scan_started = None;
                inner.scanned = true;
            }
        }
        let time = at;
        paint::backdrop(cr, w, h, p, stage::backdrop_phase(time, reduce));
        let scale = stage::fit_scale(w, h);

        // The thread's stretches.
        if let Some(current) = stage::thread_index(inner.step) {
            let grown = thread_grown(
                inner.step,
                inner.segments.len(),
                at,
                &inner,
                inner.moves.moved_at,
            );
            for (i, place) in inner.segments.iter().enumerate() {
                let Some(b) = place.compute_bounds(&self.area) else {
                    continue;
                };
                let width = f64::from(b.width());
                let y = f64::from(b.y()) + (f64::from(b.height()) - 3.0) / 2.0;
                let segment = stage::segment(i + 1, current);
                let share = if reduce {
                    if segment == stage::Segment::Ahead {
                        0.0
                    } else {
                        1.0
                    }
                } else {
                    grown[i]
                };
                paint::thread_segment(cr, (f64::from(b.x()), y), width, segment, share, p);
            }
        }

        let step = inner.step;
        let slot = inner
            .slots
            .iter()
            .find(|(s, _)| *s == step)
            .and_then(|(_, slot)| {
                slot.is_visible()
                    .then(|| slot.compute_bounds(&self.area))
                    .flatten()
            });
        let shown = !inner.agent && !(inner.open && step == Step::Uses);
        let Some(bounds) = slot.filter(|_| shown) else {
            inner.moves.hub_drawn = None;
            inner.moves.tiles_drawn.clear();
            inner.moves.lit_drawn.clear();
            return;
        };
        let _ = cr.save();
        if let Some(view) = inner
            .scroller
            .as_ref()
            .and_then(|s| s.compute_bounds(&self.area))
        {
            cr.rectangle(
                f64::from(view.x()),
                f64::from(view.y()),
                f64::from(view.width()),
                f64::from(view.height()),
            );
            cr.clip();
        }
        let origin = (
            f64::from(bounds.x()) + f64::from(bounds.width()) / 2.0 - stage::CANVAS.0 / 2.0 * scale,
            f64::from(bounds.y()),
        );
        let place = |f: Frame| Frame {
            center: (origin.0 + f.center.0 * scale, origin.1 + f.center.1 * scale),
            side: f.side * scale,
        };
        let scene = Scene {
            tools,
            scan: inner.scan_started.map(|s| at - s),
            opened: if reduce { 60.0 } else { at - inner.opened_at },
            sharing: inner.sharing,
            sharing_since: at - inner.sharing_at,
            signed_in: inner.signed_in_at.map(|s| at - s),
            time,
            reduce_motion: reduce,
        };
        // A step's art behind its actors, in its canvas.
        let _ = cr.save();
        cr.translate(origin.0, origin.1);
        cr.scale(scale, scale);
        paint::step_back(cr, step, &scene, p);
        let _ = cr.restore();

        let moving = if reduce {
            1.0
        } else {
            at - inner.moves.moved_at
        };
        let t = if reduce { 1.0 } else { stage::spatial(moving) };
        let fade_in = if reduce {
            1.0
        } else {
            (moving / FADE).clamp(0.0, 1.0)
        };

        // The N.
        let target = place(stage::hub_frame(step));
        let hub = match inner.moves.hub_from {
            Some(from) if moving < stage::SPATIAL_SETTLE => stage::tween(from, target, t),
            _ => target,
        };
        let look = paint::hub_look(step, &scene, at - inner.welcome_at);
        paint::hub(cr, hub, look, time, reduce, 1.0, p);
        inner.moves.hub_drawn = Some(hub);

        // The tiles: moved from the last step, faded in, or fading out.
        let mut drawn = BTreeMap::new();
        let mut lit_drawn = Vec::new();
        for kind in stage::RING_ORDER {
            let Some((frame, opacity, flash)) = paint::live_tile(step, kind, &scene) else {
                continue;
            };
            let target = place(frame);
            let (frame, alpha) = match inner.moves.tiles_from.get(kind) {
                Some(from) if moving < stage::SPATIAL_SETTLE => {
                    (stage::tween(*from, target, t), opacity)
                }
                Some(_) => (target, opacity),
                None => (target, opacity * fade_in),
            };
            let lit = step == Step::Tools && scene.tools.watched.iter().any(|w| w == kind);
            if lit {
                lit_drawn.push(kind.to_owned());
            }
            paint::tool_face(cr, frame, kind, lit, alpha, p);
            if let Some(flash) = flash {
                paint::tile_flash(cr, frame, flash, p);
            }
            drawn.insert(kind.to_owned(), frame);
        }
        if !reduce && moving < FADE {
            let from = inner.moves.tiles_from.clone();
            for (kind, frame) in from.iter().filter(|(k, _)| !drawn.contains_key(*k)) {
                paint::tool_face(cr, *frame, kind, false, 1.0 - fade_in, p);
            }
        }
        inner.moves.tiles_drawn = drawn;
        inner.moves.lit_drawn = lit_drawn;
        if step == Step::Traces && stage::traces::counted(scene.opened, reduce) >= 1.0 {
            inner.traces_counted = true;
        }
        let _ = cr.restore();
    }
}

/// Each thread stretch's grown share at `at`: moving from where it was at
/// the last change (`inner.segments_from`) to its new state over
/// [`THREAD_GROW`] along the token ease.
fn thread_grown(step: Step, count: usize, at: f64, inner: &Inner, changed: f64) -> Vec<f64> {
    let Some(current) = stage::thread_index(step) else {
        return vec![0.0; count];
    };
    let progress = stage::motion_curve((at - changed) / THREAD_GROW);
    (0..count)
        .map(|i| {
            let target = if stage::segment(i + 1, current) == stage::Segment::Ahead {
                0.0
            } else {
                1.0
            };
            let from = inner.segments_from.get(i).copied().unwrap_or(target);
            from + (target - from) * progress
        })
        .collect()
}

#[cfg(test)]
mod mounted_tests {
    use super::*;

    fn settle(context: &glib::MainContext, millis: u64) {
        let end = std::time::Instant::now() + std::time::Duration::from_millis(millis);
        while std::time::Instant::now() < end {
            context.iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// The stage under a first-run window: the thread on every step but
    /// Welcome, each step's slot sized to its graphic at the window's
    /// scale, the tick only while mapped and only with animations on, and
    /// the final frame drawn when they are off.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn the_stage_draws_under_the_steps_and_ticks_only_while_shown() {
        assert_eq!(std::env::consts::OS, "linux");
        let context = glib::MainContext::default();
        let _owner = context.acquire().unwrap();
        adw::init().expect("GTK display unavailable");
        super::super::style::install();
        let stage = Stage::new(Rc::new(RefCell::new(Tools::default())));
        let window = adw::Window::builder()
            .default_width(680)
            .default_height(720)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("tc-first-run-stage");
        content.append(stage.thread());
        let welcome = gtk::Box::new(gtk::Orientation::Vertical, 0);
        welcome.append(&stage.slot(Step::Welcome));
        // Taller than the window, so the steps scroll.
        let filler = gtk::Box::new(gtk::Orientation::Vertical, 0);
        filler.set_size_request(-1, 2000);
        welcome.append(&filler);
        let tools = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let tools_slot = stage.slot(Step::Tools);
        tools.append(&tools_slot);
        let stack = gtk::Stack::new();
        stack.add_named(&welcome, Some("welcome"));
        stack.add_named(&tools, Some("tools"));
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .child(&stack)
            .build();
        stage.follow(&scroll);
        content.append(&scroll);
        window.set_content(Some(&stage.wrap(&content)));
        let pick: Rc<dyn Fn(Step)> = Rc::new(|_| {});
        stage.set_step(Step::Welcome, true, pick.clone());
        // Animations on, whatever the runner's setting, so the tick's
        // stopping below is not vacuous.
        let settings = gtk::Settings::default().unwrap();
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(true);
        window.present();
        settle(&context, 300);
        assert!(!stage.thread().is_visible());
        assert!(stage.inner.borrow().tick.is_some(), "ticks while shown");

        stack.set_visible_child_name("tools");
        stage.set_step(Step::Tools, true, pick.clone());
        settle(&context, 300);
        assert!(stage.thread().is_visible());
        assert_eq!(stage.inner.borrow().segments.len(), 5);
        let scale = stage::fit_scale(
            f64::from(stage.area.width()),
            f64::from(stage.area.height()),
        );
        assert!(scale < 1.0);
        assert_eq!(
            tools_slot.height(),
            (stage::ring::HEIGHT * scale).round() as i32
        );

        // Hidden, it stops; shown, it starts again.
        window.set_visible(false);
        settle(&context, 100);
        assert!(stage.inner.borrow().tick.is_none(), "no tick while hidden");
        let draws = stage.inner.borrow().draws;
        settle(&context, 150);
        assert_eq!(stage.inner.borrow().draws, draws, "no draws while hidden");
        window.present();
        settle(&context, 100);
        assert!(
            stage.inner.borrow().tick.is_some(),
            "ticks when shown again"
        );

        // Animations off: no tick, and Welcome's N drawn whole at once.
        settings.set_gtk_enable_animations(false);
        settle(&context, 100);
        assert!(stage.inner.borrow().tick.is_none());
        stack.set_visible_child_name("welcome");
        stage.set_step(Step::Welcome, true, pick);
        settle(&context, 200);
        let (w, h) = (stage.area.width(), stage.area.height());
        let render = || {
            let surface =
                gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w, h).unwrap();
            {
                let cr = gtk::cairo::Context::new(&surface).unwrap();
                stage.draw(&cr, f64::from(w), f64::from(h));
            }
            (surface, stage.inner.borrow().moves.hub_drawn)
        };
        // Scrolled, the N moves with its page.
        let (_, before) = render();
        scroll.vadjustment().set_value(40.0);
        settle(&context, 100);
        let (_, after) = render();
        let (before, after) = (before.unwrap(), after.unwrap());
        assert!((before.center.1 - after.center.1 - 40.0).abs() < 0.5);
        scroll.vadjustment().set_value(0.0);
        settle(&context, 100);
        let (mut surface, hub) = render();
        let hub = hub.unwrap();
        surface.flush();
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        // The N's left stem, in the glow's blue.
        let x = (hub.center.0 - hub.side * 0.4) as usize;
        let y = hub.center.1 as usize;
        let i = y * stride + x * 4;
        let (b, g, r) = (data[i], data[i + 1], data[i + 2]);
        assert!(b > 200 && g > 100 && r < 60, "{r} {g} {b}");
        drop(data);
        settings.set_gtk_enable_animations(previous);
        window.close();
    }

    /// With animations off nothing ticks, so a change on Tools must ask
    /// for its own draw: ticking a tool on the real Tools screen lights its
    /// tile without the test drawing anything itself.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn a_tool_watched_with_animations_off_is_drawn_lit() {
        assert_eq!(std::env::consts::OS, "linux");
        let context = glib::MainContext::default();
        let _owner = context.acquire().unwrap();
        adw::init().expect("GTK display unavailable");
        super::super::style::install();
        let settings = gtk::Settings::default().unwrap();
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(false);
        let mut found = Tools::default();
        found.record_discovery(vec![crate::first_run::tools::Found {
            source: "codex".into(),
            path: "/home/x/.codex/sessions".into(),
            exists: true,
            session_count: 1,
            total_bytes: None,
        }]);
        let tools = Rc::new(RefCell::new(found));
        let stage = Stage::new(tools.clone());
        let window = adw::Window::builder()
            .default_width(680)
            .default_height(720)
            .build();
        let screen =
            super::super::first_run_tools::build(tools.clone(), window.upcast_ref(), || {});
        screen.root.prepend(&stage.slot(Step::Tools));
        screen.connect_changed({
            let stage = Rc::downgrade(&stage);
            move || {
                if let Some(stage) = stage.upgrade() {
                    stage.refresh();
                }
            }
        });
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(stage.thread());
        content.append(&screen.root);
        window.set_content(Some(&stage.wrap(&content)));
        stage.set_step(Step::Tools, true, Rc::new(|_| {}));
        window.present();
        settle(&context, 300);
        assert!(stage.inner.borrow().tick.is_none());
        let draws = stage.inner.borrow().draws;
        assert!(draws > 0, "drawn once shown");
        assert!(stage.inner.borrow().moves.lit_drawn.is_empty());
        let mut pending = vec![screen.root.clone().upcast::<gtk::Widget>()];
        let mut check = None;
        while let Some(widget) = pending.pop() {
            if let Ok(found) = widget.clone().downcast::<gtk::CheckButton>() {
                check = Some(found);
                break;
            }
            let mut child = widget.first_child();
            while let Some(c) = child {
                child = c.next_sibling();
                pending.push(c);
            }
        }
        check.expect("codex's check").set_active(true);
        settle(&context, 200);
        let inner = stage.inner.borrow();
        assert!(inner.draws > draws, "drawn again after the watch");
        assert_eq!(inner.moves.lit_drawn, ["codex"]);
        drop(inner);
        settings.set_gtk_enable_animations(previous);
        window.close();
    }

    /// Welcome's lines rise on Welcome's clock: hidden and 18 points low
    /// when it opens, in place by 2.56 s; at once with animations off; and
    /// coming back from the agent prompt starts them again.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn welcome_lines_rise_and_replay_after_the_agent_prompt() {
        assert_eq!(std::env::consts::OS, "linux");
        let context = glib::MainContext::default();
        let _owner = context.acquire().unwrap();
        adw::init().expect("GTK display unavailable");
        super::super::style::install();
        let settings = gtk::Settings::default().unwrap();
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(true);
        let stage = Stage::new(Rc::new(RefCell::new(Tools::default())));
        let (welcome, rises) = super::super::first_run_screens::welcome(|| {}, || {});
        let lifts: Vec<Lift> = rises.iter().map(|(l, _)| l.clone()).collect();
        welcome.prepend(&stage.slot(Step::Welcome));
        let window = adw::Window::builder()
            .default_width(680)
            .default_height(720)
            .build();
        window.set_content(Some(&stage.wrap(&welcome)));
        stage.set_step(Step::Welcome, true, Rc::new(|_| {}));
        stage.set_rises(rises);
        window.present();
        settle(&context, 200);
        for lift in &lifts {
            assert_eq!(lift.opacity(), 0.0, "hidden before its delay");
            assert_eq!(lift.offset(), 18.0);
        }
        settle(&context, 2700);
        for lift in &lifts {
            assert_eq!(lift.opacity(), 1.0);
            assert!(lift.offset().abs() < 1e-9);
        }
        // Back from the agent prompt: the clock starts again.
        stage.set_agent(true);
        stage.set_agent(false);
        assert_eq!(lifts[0].opacity(), 0.0);
        // Animations off: everything in place at once.
        settings.set_gtk_enable_animations(false);
        settle(&context, 50);
        for lift in &lifts {
            assert_eq!(lift.opacity(), 1.0);
            assert!(lift.offset().abs() < 1e-9);
        }
        stage.set_agent(true);
        stage.set_agent(false);
        assert_eq!(lifts[0].opacity(), 1.0, "no replay with animations off");
        settings.set_gtk_enable_animations(previous);
        window.close();
    }

    /// The thread's buttons back follow a commit: withdrawn while one
    /// runs, offered again after.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn the_threads_way_back_follows_a_commit() {
        assert_eq!(std::env::consts::OS, "linux");
        let context = glib::MainContext::default();
        let _owner = context.acquire().unwrap();
        adw::init().expect("GTK display unavailable");
        let stage = Stage::new(Rc::new(RefCell::new(Tools::default())));
        stage.set_step(Step::Traces, true, Rc::new(|_| {}));
        let backs = stage.backs.borrow().clone();
        assert_eq!(backs.len(), 2, "Tools and Private AI are behind Traces");
        stage.set_enabled(false);
        assert!(backs.iter().all(|b| !b.is_sensitive()));
        stage.set_enabled(true);
        assert!(backs.iter().all(|b| b.is_sensitive()));
    }
}
