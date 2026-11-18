//! The first run's marker check (`FirstRunMarkerCheckStyle`): Tools' Watch
//! boxes, an oversized check drawn like a marker stroke that runs past its
//! box (Ron, 2026-10-22). Ticked is Watch, unticked is Not used.
//!
//! It is a real `gtk::CheckButton`, so assistive tech reads a checkbox
//! with its label, and the keyboard, the pointer and the focus ring are
//! GTK's. The button's own indicator is cleared by `.tc-marker-check` in
//! `style.css` and keeps its room; a drawing laid over that room draws the
//! Mac's box and stroke ([`super::first_run_stage_paint::marker_check`]).
//!
//! The stroke draws on over 0.34 s, or off over 0.16 s, from when the box
//! last changed (`since`, kept by the caller across a rebuild of the rows),
//! on a tick that runs only while it moves and the check is mapped. With
//! GTK's animations off the stroke is at rest at once.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::first_run_stage::marker_check as m;
use super::first_run_stage_paint::{self as paint, Palette};

/// The drawing's room: the stroke runs 4 points above the 26-point frame
/// and 5 to its right.
pub const ROOM: (i32, i32) = (32, 30);
/// The frame's top-left in the drawing.
pub const FRAME_AT: (f64, f64) = (0.0, 4.0);

fn now() -> f64 {
    glib::monotonic_time() as f64 / 1e6
}

fn animations() -> bool {
    gtk::Settings::default().is_none_or(|s| s.is_gtk_enable_animations())
}

/// A marker check over `check`, a check button already labelled and set.
/// `since` is when it last changed (monotonic seconds), so a check rebuilt
/// mid-stroke carries on where it was.
pub struct MarkerCheck {
    pub root: gtk::Overlay,
    pub check: gtk::CheckButton,
    pub area: gtk::DrawingArea,
    since: Rc<Cell<Option<f64>>>,
    tick: Rc<Cell<Option<gtk::TickCallbackId>>>,
}

impl MarkerCheck {
    pub fn new(check: gtk::CheckButton, since: Option<f64>) -> Rc<Self> {
        check.add_css_class("tc-marker-check");
        let area = gtk::DrawingArea::builder()
            .content_width(ROOM.0)
            .content_height(ROOM.1)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Center)
            .can_target(false)
            .can_focus(false)
            .build();
        let root = gtk::Overlay::new();
        root.set_child(Some(&check));
        root.add_overlay(&area);
        let this = Rc::new(Self {
            root,
            check,
            area,
            since: Rc::new(Cell::new(since)),
            tick: Rc::new(Cell::new(None)),
        });
        let hovered = Rc::new(Cell::new(false));
        let motion = gtk::EventControllerMotion::new();
        motion.connect_enter({
            let (hovered, area) = (hovered.clone(), this.area.clone());
            move |_, _, _| {
                hovered.set(true);
                area.queue_draw();
            }
        });
        motion.connect_leave({
            let (hovered, area) = (hovered.clone(), this.area.clone());
            move |_| {
                hovered.set(false);
                area.queue_draw();
            }
        });
        this.check.add_controller(motion);
        let palette = Palette::scheme(true);
        this.area.set_draw_func({
            let check = this.check.clone();
            let since = this.since.clone();
            move |_, cr, _, _| {
                let on = check.is_active();
                let elapsed = since.get().map(|s| now() - s);
                let trim = m::trim(on, elapsed, !animations());
                paint::marker_check(
                    cr,
                    FRAME_AT,
                    on,
                    trim,
                    hovered.get(),
                    check.is_sensitive(),
                    &palette,
                );
            }
        });
        this.check.connect_toggled({
            let weak = Rc::downgrade(&this);
            move |_| {
                if let Some(this) = weak.upgrade() {
                    this.since.set(Some(now()));
                    this.animate();
                }
            }
        });
        this.check.connect_sensitive_notify({
            let area = this.area.clone();
            move |_| area.queue_draw()
        });
        this.area.connect_map({
            let weak = Rc::downgrade(&this);
            move |_| {
                if let Some(this) = weak.upgrade() {
                    this.animate();
                }
            }
        });
        this.area.connect_unmap({
            let tick = this.tick.clone();
            move |_| {
                if let Some(id) = tick.take() {
                    id.remove();
                }
            }
        });
        this
    }

    fn moving(&self) -> bool {
        let elapsed = self.since.get().map(|s| now() - s);
        m::moving(self.check.is_active(), elapsed, !animations())
    }

    /// Whether the stroke's tick is running.
    pub fn ticking(&self) -> bool {
        let id = self.tick.take();
        let running = id.is_some();
        self.tick.set(id);
        running
    }

    /// Draws the stroke's next frames while it moves and is shown.
    fn animate(&self) {
        self.area.queue_draw();
        if !self.area.is_mapped() || !self.moving() || self.ticking() {
            return;
        }
        let (since, check, tick) = (self.since.clone(), self.check.clone(), self.tick.clone());
        let id = self.area.add_tick_callback(move |area, _| {
            area.queue_draw();
            let elapsed = since.get().map(|s| now() - s);
            if m::moving(check.is_active(), elapsed, !animations()) {
                glib::ControlFlow::Continue
            } else {
                // Ended by returning Break.
                drop(tick.take());
                glib::ControlFlow::Break
            }
        });
        self.tick.set(Some(id));
    }
}
