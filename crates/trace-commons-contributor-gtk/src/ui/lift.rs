//! A one-child container that draws its child moved up or down and faded,
//! without moving it in the layout: SwiftUI's `.offset(y:)` and
//! `.opacity`, which change only where a view is drawn. The first run uses
//! it for Welcome's lines rising into place (`WelcomeScreen`'s `Rise`) and
//! for the Customize card rising to where Uses' card was (`FirstRunFrame`'s
//! `open`).
//!
//! Nothing here keeps time on its own except [`Lift::slide_from`], whose
//! tick runs only while the lift is mapped and only until it settles. With
//! GTK's animations off it never starts: the child is drawn in place.

use std::cell::{Cell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use super::first_run_stage as stage;

/// Where a slide started, once the lift has been laid out again.
struct Slide {
    ancestor: glib::WeakRef<gtk::Widget>,
    from_y: f64,
    /// The drawn offset at the start, known after the first allocation.
    dy: Option<f64>,
    started: f64,
    tick: Option<gtk::TickCallbackId>,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Lift {
        pub(super) offset: Cell<f64>,
        pub(super) slide: RefCell<Option<Slide>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Lift {
        const NAME: &'static str = "TcLift";
        type Type = super::Lift;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for Lift {
        fn dispose(&self) {
            if let Some(tick) = self.slide.borrow_mut().as_mut().and_then(|s| s.tick.take()) {
                tick.remove();
            }
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Lift {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            self.obj().begin_slide();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let Some(child) = widget.first_child() else {
                return;
            };
            snapshot.save();
            snapshot.translate(&gtk::graphene::Point::new(0.0, self.offset.get() as f32));
            widget.snapshot_child(&child, snapshot);
            snapshot.restore();
        }

        fn unmap(&self) {
            self.parent_unmap();
            self.obj().settle();
        }
    }
}

glib::wrapper! {
    pub struct Lift(ObjectSubclass<imp::Lift>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

fn now() -> f64 {
    glib::monotonic_time() as f64 / 1e6
}

fn animations() -> bool {
    gtk::Settings::default().is_none_or(|s| s.is_gtk_enable_animations())
}

impl Lift {
    pub fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let lift: Self = glib::Object::new();
        child.set_parent(&lift);
        lift
    }

    /// The child drawn `offset` points below its place.
    pub fn set_offset(&self, offset: f64) {
        if (self.imp().offset.get() - offset).abs() > f64::EPSILON {
            self.imp().offset.set(offset);
            self.queue_draw();
        }
    }

    pub fn offset(&self) -> f64 {
        self.imp().offset.get()
    }

    /// A line `amount` of the way risen into place: faded in, and lowered
    /// [`stage::welcome::RISE_LIFT`] points at nothing (Welcome's `Rise`).
    pub fn set_risen(&self, amount: f64) {
        self.set_opacity(amount.clamp(0.0, 1.0));
        self.set_offset(stage::welcome::RISE_LIFT * (1.0 - amount));
    }

    /// The child rises (or falls) from `from_y`, its top's old place in
    /// `ancestor`, to where the next layout puts it, faded in on the way,
    /// along the first run's spatial spring at the open duration
    /// ([`stage::customize`]). Nothing moves with animations off.
    pub fn slide_from(&self, ancestor: &impl IsA<gtk::Widget>, from_y: f64) {
        self.settle();
        if !animations() {
            return;
        }
        *self.imp().slide.borrow_mut() = Some(Slide {
            ancestor: ancestor.upcast_ref::<gtk::Widget>().downgrade(),
            from_y,
            dy: None,
            started: now(),
            tick: None,
        });
        self.set_opacity(0.0);
        self.queue_allocate();
    }

    /// Whether a slide is under way.
    pub fn sliding(&self) -> bool {
        self.imp().slide.borrow().is_some()
    }

    /// The first allocation after [`Self::slide_from`]: the offset that
    /// keeps the child where it was, and the tick that brings it home.
    fn begin_slide(&self) {
        let mut slot = self.imp().slide.borrow_mut();
        let Some(slide) = slot.as_mut().filter(|s| s.dy.is_none()) else {
            return;
        };
        let Some(ancestor) = slide.ancestor.upgrade() else {
            drop(slot);
            self.settle();
            return;
        };
        let Some(here) = self.compute_point(&ancestor, &gtk::graphene::Point::new(0.0, 0.0)) else {
            return;
        };
        let dy = slide.from_y - f64::from(here.y());
        slide.dy = Some(dy);
        slide.started = now();
        self.imp().offset.set(dy);
        let weak = self.downgrade();
        slide.tick = Some(self.add_tick_callback(move |_, _| {
            let Some(lift) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if lift.step_slide() {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        }));
    }

    /// One frame of the slide; false once it has settled.
    fn step_slide(&self) -> bool {
        let (dy, started) = match self.imp().slide.borrow().as_ref() {
            Some(Slide {
                dy: Some(dy),
                started,
                ..
            }) => (*dy, *started),
            _ => return false,
        };
        let t = now() - started;
        if t >= stage::SPATIAL_SETTLE {
            // The tick ends itself by returning Break.
            self.imp().slide.borrow_mut().take();
            self.set_opacity(1.0);
            self.set_offset(0.0);
            return false;
        }
        let (offset, opacity) = stage::customize::slide(dy, t);
        self.set_opacity(opacity);
        self.set_offset(offset);
        true
    }

    /// Ends any slide at once: the child in place, fully drawn.
    pub fn settle(&self) {
        if let Some(mut slide) = self.imp().slide.borrow_mut().take() {
            if let Some(tick) = slide.tick.take() {
                tick.remove();
            }
        }
        self.set_opacity(1.0);
        self.set_offset(0.0);
    }
}
