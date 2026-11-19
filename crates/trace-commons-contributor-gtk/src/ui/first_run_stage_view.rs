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
//! It draws at each step's own rate, as the Mac's timelines do
//! ([`stage::motion::frame_interval`]: Welcome and the Tools scan at the
//! display's rate, Uses and Traces at 60 a second, the rest at 30, and at
//! the display's rate while an actor, the thread, a beam, the ring or a
//! satellite is springing), and only while mapped: the tick is added on
//! map and removed on unmap. With GTK's animations off
//! (`gtk-enable-animations`), as under the Mac's Reduce Motion, it draws
//! each step's final frame and moves nothing, and it follows that setting
//! as it changes.
//!
//! The same tick moves the few widgets whose motion hangs on the stage's
//! clock: the Tools rows rising after the scan, the Private AI figure
//! counting up, the Traces estimate rising after the count, and the thread
//! coming in.
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
    /// Each found tool's beam: the opacity it fades from, to, and when.
    beams: BTreeMap<String, (f64, f64, f64)>,
    /// When the scan ends and the dashed ring tightens (and the Tools rows
    /// rise); None when no scan ran.
    ring_tight_at: Option<f64>,
    /// When the thread last came in.
    thread_at: f64,
    /// Private AI's figure has counted once: going back does not replay it
    /// (`FirstRunStageMemory.privateCounted`).
    private_counted: bool,
    /// When Customize opened or closed on Uses, and whether it is open.
    open_at: f64,
    /// Where Uses' graphic was last drawn, for its fade as Customize opens.
    uses_bounds: Option<gtk::graphene::Rect>,
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
    /// The thread, drawn moved and faded as it comes in.
    thread_lift: Lift,
    /// Private AI's figure and the count it reaches.
    private_count: RefCell<Option<(gtk::Label, usize)>>,
    /// Traces' estimate, risen once the count is done.
    estimate: RefCell<Option<Lift>>,
    /// Tools' rows: told None while the scan holds them back, then the
    /// seconds since they began to rise.
    tools_rows: RefCell<Option<Rc<dyn Fn(Option<f64>)>>>,
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
        let thread_lift = Lift::new(&thread);
        thread_lift.set_visible(false);
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
                beams: BTreeMap::new(),
                ring_tight_at: None,
                thread_at: at,
                private_counted: false,
                open_at: at - 3600.0,
                uses_bounds: None,
            }),
            started: Cell::new(false),
            palette: Palette::scheme(true),
            rises: RefCell::new(Vec::new()),
            backs: RefCell::new(Vec::new()),
            captions: RefCell::new(None),
            thread_lift,
            private_count: RefCell::new(None),
            estimate: RefCell::new(None),
            tools_rows: RefCell::new(None),
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
        // Minimised or suspended, the stage holds; shown again, it goes on
        // from where its clock now is.
        let weak = Rc::downgrade(&stage);
        super::glass_motion::on_window_state(&stage.area, move |area| {
            if let Some(stage) = weak.upgrade() {
                stage.stop_ticking();
                if area.is_mapped() {
                    stage.start_ticking();
                }
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
                    stage.drive(now());
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
    pub(super) fn thread(&self) -> &Lift {
        &self.thread_lift
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
        let at = now();
        self.advance(at, !animations());
        self.drive(at);
        self.area.queue_draw();
    }

    /// Private AI's figure, `label`, counts up to `count` on the first
    /// visit (`PrivateAIScreenLayout.counted`).
    pub(super) fn set_private_count(&self, label: &gtk::Label, count: usize) {
        *self.private_count.borrow_mut() = Some((label.clone(), count));
        self.drive(now());
    }

    /// Traces' estimate, `lift`, rises once the count is done.
    pub(super) fn set_estimate(&self, lift: &Lift) {
        *self.estimate.borrow_mut() = Some(lift.clone());
        self.drive(now());
    }

    /// Tools' rows, moved by `rise` (None: held back while the scan runs;
    /// otherwise the seconds since they began to rise).
    pub(super) fn set_tools_rows(&self, rise: Rc<dyn Fn(Option<f64>)>) {
        *self.tools_rows.borrow_mut() = Some(rise);
        self.drive(now());
    }

    /// Whether a SwiftUI animation would be under way at `at`, which the
    /// Mac draws at the display's rate whatever its timeline's interval.
    fn in_flight(&self, at: f64) -> bool {
        let inner = self.inner.borrow();
        let since = |t: f64| at - t;
        let spring = since(inner.moves.moved_at) < stage::SPATIAL_SETTLE;
        let beams = inner
            .beams
            .values()
            .any(|(_, _, t)| since(*t) < stage::motion::BEAM_FADE);
        let rows = self.tools_seen_count();
        let tight = inner.ring_tight_at.is_some_and(|t| {
            (0.0..stage::SPATIAL_SETTLE.max(stage::motion::rows_risen_by(rows))).contains(&since(t))
        });
        let satellites = inner.step == Step::Uses
            && stage::motion::satellites_moving(inner.sharing, since(inner.sharing_at));
        let estimate = inner.step == Step::Traces && {
            let done =
                since(inner.opened_at) - stage::traces::COUNT_DELAY - stage::traces::COUNT_DURATION;
            (0.0..stage::spring_settle(stage::motion::ESTIMATE_RISE)).contains(&done)
        };
        let open = since(inner.open_at) < stage::spring_settle(stage::customize::DURATION);
        spring || beams || tight || satellites || estimate || open
    }

    /// How many rows the Tools ring shows.
    fn tools_seen_count(&self) -> usize {
        self.tools
            .borrow()
            .discovered
            .as_deref()
            .map_or(0, |found| found.len())
    }

    /// The scan starts once Tools first has rows, and ends.
    fn advance(&self, at: f64, reduce: bool) {
        let (tools, discovered) = self.tools_seen();
        let mut inner = self.inner.borrow_mut();
        if inner.step == Step::Tools && discovered && !inner.scanned && inner.scan_started.is_none()
        {
            if reduce || tools.rows.is_empty() {
                inner.scanned = true;
            } else {
                inner.scan_started = Some(at);
                inner.ring_tight_at = Some(at + stage::SCAN_LEAD + stage::SCAN_TURN + 0.25);
            }
        }
        if let Some(started) = inner.scan_started {
            if at - started >= stage::SCAN_TOTAL || reduce {
                inner.scan_started = None;
                inner.scanned = true;
            }
        }
    }

    /// Moves the widgets that ride the stage's clock to where they are at
    /// `at`: the final frame with animations off.
    fn drive(&self, at: f64) {
        let reduce = !animations();
        let (step, opened, thread_at, tight_at, scanning) = {
            let inner = self.inner.borrow();
            (
                inner.step,
                inner.opened_at,
                inner.thread_at,
                inner.ring_tight_at,
                inner.scan_started.is_some(),
            )
        };
        if self.thread_lift.is_visible() {
            let (offset, opacity) = stage::motion::thread_shown(true, at - thread_at, reduce);
            self.thread_lift.set_offset(offset);
            self.thread_lift.set_opacity(opacity);
        }
        if step == Step::Tools {
            let rise = self.tools_rows.borrow().clone();
            if let Some(rise) = rise {
                rise(match tight_at {
                    _ if reduce => Some(f64::INFINITY),
                    Some(t) if scanning && at < t => None,
                    Some(t) => Some(at - t),
                    None => Some(f64::INFINITY),
                });
            }
        }
        if step == Step::PrivateAi {
            if let Some((label, count)) = self.private_count.borrow().as_ref() {
                let shown = stage::motion::private_counted(*count, at - opened, reduce).to_string();
                if label.text() != shown {
                    label.set_text(&shown);
                }
            }
        }
        if step == Step::Traces {
            if let Some(lift) = self.estimate.borrow().as_ref() {
                let done = opened + stage::traces::COUNT_DELAY + stage::traces::COUNT_DURATION;
                let (offset, opacity) = stage::motion::estimate_rise(at - done, reduce);
                lift.set_offset(offset);
                lift.set_opacity(opacity);
            }
        }
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
            if false && super::glass_motion::window_hidden(area) {
                // Ended here; the window's state change starts it again.
                stage.inner.borrow_mut().tick.take();
                return glib::ControlFlow::Break;
            }
            let frame = clock.frame_time();
            let at = now();
            let interval = {
                let inner = stage.inner.borrow();
                let (step, scanning) = (inner.step, inner.scan_started.is_some());
                drop(inner);
                stage::motion::frame_interval(step, scanning, stage.in_flight(at))
            };
            let due = {
                let mut inner = stage.inner.borrow_mut();
                let since = (frame - inner.last_frame) as f64 / 1e6;
                let due = stage::motion::frame_due(since, interval);
                if due {
                    inner.last_frame = frame;
                }
                due
            };
            if due {
                stage.advance(at, false);
                stage.drive(at);
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
        let was = self.inner.borrow().step;
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
                inner.opened_at = if (step == Step::Traces && inner.traces_counted)
                    || (step == Step::PrivateAi && inner.private_counted)
                {
                    at - 3600.0
                } else {
                    at
                };
                if step == Step::PrivateAi {
                    inner.private_counted = true;
                }
                inner.join_known = None;
                if step == Step::Welcome {
                    inner.welcome_at = at;
                }
                inner.step = step;
            }
        }
        self.rebuild_thread(step, was, enabled, on_pick);
        if step == Step::Welcome {
            self.raise_lines();
        }
        self.advance(at, !animations());
        self.drive(at);
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
        let mut inner = self.inner.borrow_mut();
        if inner.open != open {
            inner.open_at = now();
        }
        inner.open = open;
        drop(inner);
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

    fn rebuild_thread(
        self: &Rc<Self>,
        step: Step,
        was: Step,
        enabled: bool,
        on_pick: Rc<dyn Fn(Step)>,
    ) {
        while let Some(child) = self.thread.first_child() {
            self.thread.remove(&child);
        }
        self.backs.borrow_mut().clear();
        let Some(current) = stage::thread_index(step) else {
            self.thread_lift.set_visible(false);
            self.inner.borrow_mut().segments.clear();
            return;
        };
        if !self.thread_lift.is_visible() {
            // Coming in from Welcome: from 10 points above, faded.
            self.inner.borrow_mut().thread_at = now();
            self.thread_lift.set_visible(true);
        }
        let words = trace_commons_contributor::first_run_copy::first_run_copy().frame;
        let scale = stage::fit_scale(
            f64::from(self.area.width().max(1)),
            f64::from(self.area.height().max(1)),
        );
        // The titles are built as they read on the step before, and take
        // this step's look once drawn, so the current one lights after the
        // Mac's delay (`tc-thread-title`'s transition) rather than at once.
        let before = stage::thread_index(was).filter(|w| *w != current);
        let look = |index: usize, at: usize| {
            if index == at {
                Some("tc-thread-current")
            } else if index < at {
                Some("tc-thread-passed")
            } else {
                None
            }
        };
        let mut relook = Vec::new();
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
            let (from, to) = (look(index, before.unwrap_or(current)), look(index, current));
            if let Some(class) = from {
                label.add_css_class(class);
            }
            if from != to {
                relook.push((label.clone(), from, to));
            }
            if index == current {
                // The Mac's `.isSelected` on the current step's title.
                label.update_state(&[gtk::accessible::State::Selected(Some(true))]);
                self.thread.append(&label);
            } else if index < current {
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
        if !relook.is_empty() {
            super::first_run_fade::restyle_after_drawn(&self.thread, move || {
                for (label, from, to) in relook {
                    if let Some(class) = from {
                        label.remove_css_class(class);
                    }
                    if let Some(class) = to {
                        label.add_css_class(class);
                    }
                }
            });
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
        // The scan runs once per first run, when Tools first has rows.
        self.advance(at, reduce);
        let (tools, _) = self.tools_seen();
        let mut inner = self.inner.borrow_mut();
        inner.draws += 1;
        // Each found tool's beam fades as it is watched or not.
        if inner.step == Step::Tools {
            for (kind, found) in &tools.rows {
                if !found {
                    continue;
                }
                let to = if tools.watched.iter().any(|w| w == kind) {
                    1.0
                } else {
                    0.0
                };
                let entry = inner
                    .beams
                    .entry(kind.clone())
                    .or_insert((to, to, at - 60.0));
                if entry.1 != to {
                    let current =
                        stage::motion::beam_opacity(entry.0, entry.1, at - entry.2, reduce);
                    *entry = (current, to, at);
                }
            }
        }
        let beams: BTreeMap<String, f64> = inner
            .beams
            .iter()
            .map(|(k, (from, to, t))| {
                (
                    k.clone(),
                    stage::motion::beam_opacity(*from, *to, at - t, reduce),
                )
            })
            .collect();
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
            // Drawn where the thread's lift draws its titles, as faded.
            let (lift_y, lift_alpha) = (self.thread_lift.offset(), self.thread_lift.opacity());
            cr.push_group();
            cr.translate(0.0, lift_y);
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
            if cr.pop_group_to_source().is_ok() {
                let _ = cr.paint_with_alpha(lift_alpha);
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
        // Customize opening: Uses' graphic fades out where it was.
        let opening = inner.open && step == Step::Uses;
        let alpha = if opening {
            stage::motion::customize_fade_out(at - inner.open_at, reduce)
        } else {
            1.0
        };
        let slot = if opening {
            inner.uses_bounds.filter(|_| alpha > 0.0)
        } else {
            if step == Step::Uses && slot.is_some() {
                inner.uses_bounds = slot;
            }
            slot
        };
        let shown = !inner.agent;
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
        if alpha < 1.0 {
            cr.push_group();
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
            beams,
            ring_since: inner.ring_tight_at.map(|t| at - t),
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
        if alpha < 1.0 && cr.pop_group_to_source().is_ok() {
            let _ = cr.paint_with_alpha(alpha);
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

    /// Minimised, the stage's clock holds: no tick and no frame drawn,
    /// while the window stays mapped and its frame clock runs. Shown
    /// again, the clock runs on.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn the_stage_holds_while_its_window_is_minimised() {
        use super::super::glass_motion::minimise_for_test;
        assert_eq!(std::env::consts::OS, "linux");
        let context = glib::MainContext::default();
        let _owner = context.acquire().unwrap();
        adw::init().expect("GTK display unavailable");
        super::super::style::install();
        let settings = gtk::Settings::default().unwrap();
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(true);
        let stage = Stage::new(Rc::new(RefCell::new(Tools::default())));
        let window = adw::Window::builder()
            .default_width(680)
            .default_height(720)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("tc-first-run-stage");
        content.append(stage.thread());
        let tools = gtk::Box::new(gtk::Orientation::Vertical, 0);
        tools.append(&stage.slot(Step::Tools));
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .child(&tools)
            .build();
        stage.follow(&scroll);
        content.append(&scroll);
        window.set_content(Some(&stage.wrap(&content)));
        let pick: Rc<dyn Fn(Step)> = Rc::new(|_| {});
        stage.set_step(Step::Tools, true, pick);
        window.present();
        settle(&context, 300);
        assert!(stage.inner.borrow().tick.is_some(), "ticks while shown");

        minimise_for_test(&window, true);
        settle(&context, 100);
        assert!(window.is_mapped(), "a minimised window stays mapped");
        assert!(
            stage.inner.borrow().tick.is_none(),
            "the clock runs while minimised"
        );
        let draws = stage.inner.borrow().draws;
        settle(&context, 300);
        assert_eq!(
            stage.inner.borrow().draws,
            draws,
            "frames drawn while minimised"
        );

        minimise_for_test(&window, false);
        settle(&context, 100);
        assert!(
            stage.inner.borrow().tick.is_some(),
            "the clock does not run once shown again"
        );
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

    /// The widgets on the stage's clock: the thread comes in from 10
    /// points above, faded, and its new title lights only after it is
    /// drawn; Tools' rows are held while the scan runs and rise after it;
    /// Private AI's figure counts up once; Traces' estimate rises once the
    /// count is done.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn the_stage_moves_its_widgets_on_the_macs_clock() {
        assert_eq!(std::env::consts::OS, "linux");
        let context = glib::MainContext::default();
        let _owner = context.acquire().unwrap();
        adw::init().expect("GTK display unavailable");
        super::super::style::install();
        let settings = gtk::Settings::default().unwrap();
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(true);
        let mut found = Tools::default();
        found.record_discovery(
            ["codex", "claude-code"]
                .iter()
                .map(|source| crate::first_run::tools::Found {
                    source: (*source).into(),
                    path: format!("/home/x/{source}"),
                    exists: true,
                    session_count: 1,
                    total_bytes: None,
                })
                .collect(),
        );
        let stage = Stage::new(Rc::new(RefCell::new(found)));
        let window = adw::Window::builder()
            .default_width(680)
            .default_height(720)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(stage.thread());
        for step in [Step::Welcome, Step::Tools, Step::PrivateAi, Step::Traces] {
            content.append(&stage.slot(step));
        }
        let estimate = Lift::new(&gtk::Label::new(Some("estimate")));
        content.append(&estimate);
        let figure = gtk::Label::new(None);
        content.append(&figure);
        window.set_content(Some(&stage.wrap(&content)));
        let rows: Rc<RefCell<Vec<Option<f64>>>> = Rc::default();
        stage.set_tools_rows({
            let rows = rows.clone();
            Rc::new(move |since| rows.borrow_mut().push(since))
        });
        let pick: Rc<dyn Fn(Step)> = Rc::new(|_| {});
        stage.set_step(Step::Welcome, true, pick.clone());
        window.present();
        settle(&context, 200);

        stage.set_step(Step::Tools, true, pick.clone());
        let lift = stage.thread();
        assert!(lift.is_visible());
        assert!(lift.opacity() < 0.2, "{}", lift.opacity());
        assert!(lift.offset() < -8.0, "{}", lift.offset());
        assert_eq!(
            rows.borrow().last(),
            Some(&None),
            "held while the scan runs"
        );
        assert!(stage.in_flight(now()), "the step move runs at display rate");
        settle(&context, 1500);
        assert_eq!(lift.opacity(), 1.0);
        assert!(lift.offset().abs() < 0.05, "{}", lift.offset());
        settle(&context, 2300);
        let since = rows.borrow().last().copied().flatten();
        assert!(
            since.is_some_and(|s| s > 0.0),
            "rising after the scan: {since:?}"
        );

        let title = |stage: &Stage, index: usize| {
            let mut titles = Vec::new();
            let mut pending = vec![stage.thread.clone().upcast::<gtk::Widget>()];
            while let Some(widget) = pending.pop() {
                if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
                    titles.push(label);
                }
                let mut child = widget.last_child();
                while let Some(c) = child {
                    child = c.prev_sibling();
                    pending.push(c);
                }
            }
            titles[index].clone()
        };
        stage.set_step(Step::PrivateAi, true, pick.clone());
        stage.set_private_count(&figure, 3);
        assert!(
            !title(&stage, 1).has_css_class("tc-thread-current"),
            "not lit at once"
        );
        assert!(
            title(&stage, 0).has_css_class("tc-thread-current"),
            "as it read before"
        );
        assert_eq!(figure.text(), "0");
        settle(&context, 300);
        assert!(title(&stage, 1).has_css_class("tc-thread-current"));
        assert!(!title(&stage, 0).has_css_class("tc-thread-current"));
        settle(&context, 1400);
        assert_eq!(figure.text(), "3");

        stage.set_estimate(&estimate);
        stage.set_step(Step::Traces, true, pick.clone());
        assert_eq!(estimate.offset(), 8.0);
        assert_eq!(estimate.opacity(), 0.0);
        settle(&context, 4000);
        assert!(estimate.offset().abs() < 0.05, "{}", estimate.offset());
        assert_eq!(estimate.opacity(), 1.0);

        // Back to Private AI: counted already, it does not count again.
        stage.set_step(Step::PrivateAi, true, pick.clone());
        assert_eq!(figure.text(), "3");

        // Animations off: everything at its last frame at once.
        settings.set_gtk_enable_animations(false);
        stage.set_step(Step::Traces, true, pick);
        assert_eq!(estimate.opacity(), 1.0);
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
