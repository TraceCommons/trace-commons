//! The first run's small switch (`FirstRunSwitchStyle`, 30 by 18): the
//! switch first, its words after (Ron, 2026-10-22: toggles on the left), as
//! Customize's past traces read.
//!
//! It is a real `gtk::CheckButton`, so assistive tech reads it with its
//! label and the keyboard, pointer and focus ring are GTK's; the button's
//! own box is cleared by `.tc-first-run-switch` in `style.css` and keeps
//! its room, and a drawing over that room draws the Mac's track and knob
//! ([`super::first_run_stage_paint::small_switch`]). Toggled, the knob
//! slides and the colours cross over 0.2 s on the token curve, on a tick
//! that runs only while it slides and the switch is mapped. With GTK's
//! animations off it is at rest at once.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::first_run_stage::motion;
use super::first_run_stage_paint::{self as paint, Palette};

fn now() -> f64 {
    glib::monotonic_time() as f64 / 1e6
}

fn animations() -> bool {
    gtk::Settings::default().is_none_or(|s| s.is_gtk_enable_animations())
}

pub struct SmallSwitch {
    pub root: gtk::Overlay,
    pub check: gtk::CheckButton,
    area: gtk::DrawingArea,
    since: Rc<Cell<Option<f64>>>,
    tick: Rc<Cell<Option<gtk::TickCallbackId>>>,
}

impl SmallSwitch {
    /// A small switch over `check`, a check button already labelled and set.
    pub fn new(check: gtk::CheckButton) -> Rc<Self> {
        check.add_css_class("tc-first-run-switch");
        let (w, h) = motion::SWITCH_SIZE;
        let area = gtk::DrawingArea::builder()
            .content_width(w as i32)
            .content_height(h as i32)
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
            since: Rc::new(Cell::new(None)),
            tick: Rc::new(Cell::new(None)),
        });
        let palette = Palette::scheme(true);
        this.area.set_draw_func({
            let (check, since) = (this.check.clone(), this.since.clone());
            move |_, cr, _, _| {
                let elapsed = since.get().map(|s| now() - s);
                let on = motion::switch_on(check.is_active(), elapsed, !animations());
                paint::small_switch(cr, (0.0, 0.0), on, check.is_sensitive(), &palette);
            }
        });
        // The check holds what it needs, so the switch lives with its row.
        this.check.connect_toggled({
            let (area, since, tick) = (this.area.clone(), this.since.clone(), this.tick.clone());
            move |_| {
                since.set(Some(now()));
                animate(&area, &since, &tick);
            }
        });
        this.check.connect_sensitive_notify({
            let area = this.area.clone();
            move |_| area.queue_draw()
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

    /// Whether the knob's tick is running.
    #[cfg(test)]
    pub fn ticking(&self) -> bool {
        let id = self.tick.take();
        let running = id.is_some();
        self.tick.set(id);
        running
    }
}

/// Draws the knob's next frames while it slides and is shown.
fn animate(
    area: &gtk::DrawingArea,
    since: &Rc<Cell<Option<f64>>>,
    tick: &Rc<Cell<Option<gtk::TickCallbackId>>>,
) {
    {
        area.queue_draw();
        let elapsed = since.get().map(|s| now() - s);
        let running = tick.take();
        let ticking = running.is_some();
        tick.set(running);
        if !area.is_mapped() || !motion::switch_moving(elapsed, !animations()) || ticking {
            return;
        }
        let (since, tick_slot) = (since.clone(), tick.clone());
        let id = area.add_tick_callback(move |area, _| {
            area.queue_draw();
            let elapsed = since.get().map(|s| now() - s);
            if motion::switch_moving(elapsed, !animations()) {
                glib::ControlFlow::Continue
            } else {
                // Ended by returning Break.
                drop(tick_slot.take());
                glib::ControlFlow::Break
            }
        });
        tick.set(Some(id));
    }
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

    /// The switch ticks only while its knob slides (0.2 s) and only with
    /// animations on; it stays a check button to assistive tech.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn the_small_switch_ticks_only_while_it_slides() {
        assert_eq!(std::env::consts::OS, "linux");
        let context = glib::MainContext::default();
        let _owner = context.acquire().unwrap();
        adw::init().expect("GTK display unavailable");
        super::super::style::install();
        let settings = gtk::Settings::default().unwrap();
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(true);
        let switch = SmallSwitch::new(gtk::CheckButton::with_label("x"));
        assert_eq!(
            switch.check.accessible_role(),
            gtk::AccessibleRole::Checkbox
        );
        let window = adw::Window::builder().content(&switch.root).build();
        window.present();
        settle(&context, 200);
        assert!(!switch.ticking());
        assert_eq!(switch.area.width(), 30);
        assert_eq!(switch.area.height(), 18);
        switch.check.set_active(true);
        assert!(switch.ticking(), "slides when toggled");
        settle(&context, 350);
        assert!(!switch.ticking(), "stops once at rest");
        settings.set_gtk_enable_animations(false);
        switch.check.set_active(false);
        assert!(!switch.ticking(), "no tick with animations off");
        settings.set_gtk_enable_animations(previous);
        window.close();
    }
}
