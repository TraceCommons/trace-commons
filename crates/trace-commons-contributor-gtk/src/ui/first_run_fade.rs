//! The first run's steps crossfading, as the Mac's do: each screen's
//! `.transition(.opacity)` under the coordinator's spatial spring (0.9 s,
//! bounce 0.04; `OnboardingCoordinatorView`), not a stack's own timed fade.
//!
//! The step going away is drawn as it last was (a still of it, so nothing
//! of it can be clicked) over the step coming in, fading out as the new
//! one fades in, both along [`stage::motion::crossfade`]. The stage's N and
//! tiles are not in either still: the stage moves them itself.
//!
//! The tick runs only while the fade does and only while mapped. With
//! GTK's animations off the new step shows at once, and turning them off
//! mid-fade ends the fade at its last frame.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::first_run_stage::{self as stage, motion};

pub(super) struct StepFade {
    root: gtk::Overlay,
    stack: gtk::Stack,
    /// The step going away, as it last was.
    ghost: gtk::Picture,
    running: RefCell<Option<(gtk::TickCallbackId, gtk::Widget)>>,
}

fn animations() -> bool {
    gtk::Settings::default().is_none_or(|s| s.is_gtk_enable_animations())
}

impl StepFade {
    /// Crossfades `stack`'s pages; [`Self::root`] takes the stack's place.
    pub(super) fn new(stack: &gtk::Stack) -> Rc<Self> {
        stack.set_transition_type(gtk::StackTransitionType::None);
        let ghost = gtk::Picture::builder()
            .can_target(false)
            .can_focus(false)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .visible(false)
            .build();
        ghost.set_can_shrink(true);
        let root = gtk::Overlay::new();
        root.set_child(Some(stack));
        root.add_overlay(&ghost);
        let fade = Rc::new(Self {
            root,
            stack: stack.clone(),
            ghost,
            running: RefCell::new(None),
        });
        let weak = Rc::downgrade(&fade);
        fade.stack.connect_unmap(move |_| {
            if let Some(fade) = weak.upgrade() {
                fade.finish();
            }
        });
        if let Some(settings) = gtk::Settings::default() {
            let weak = Rc::downgrade(&fade);
            settings.connect_gtk_enable_animations_notify(move |_| {
                if let Some(fade) = weak.upgrade() {
                    if !animations() {
                        fade.finish();
                    }
                }
            });
        }
        fade
    }

    pub(super) fn root(&self) -> &gtk::Overlay {
        &self.root
    }

    /// Whether a fade is under way.
    #[cfg(test)]
    pub(super) fn fading(&self) -> bool {
        self.running.borrow().is_some()
    }

    /// The still of the step going away, while it fades.
    #[cfg(test)]
    pub(super) fn ghost(&self) -> &gtk::Picture {
        &self.ghost
    }

    /// `widget`, about to leave the layout, is drawn where it was as a still
    /// fading out along `opacity` (seconds in, to opacity) until `settle`
    /// seconds: Customize's title and the Uses captions giving way
    /// (`FirstRunFrame`'s `.transition(.opacity)` under the open spring).
    /// Nothing is drawn with animations off.
    pub(super) fn fade_away(
        &self,
        widget: &impl IsA<gtk::Widget>,
        opacity: fn(f64) -> f64,
        settle: f64,
    ) {
        let widget = widget.upcast_ref::<gtk::Widget>();
        if !animations()
            || super::glass_motion::window_hidden(widget)
            || !widget.is_mapped()
            || widget.width() == 0
            || widget.height() == 0
        {
            return;
        }
        let Some(at) = widget.compute_point(&self.root, &gtk::graphene::Point::new(0.0, 0.0))
        else {
            return;
        };
        let still = gtk::WidgetPaintable::new(Some(widget)).current_image();
        let ghost = gtk::Picture::builder()
            .paintable(&still)
            .can_target(false)
            .can_focus(false)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .margin_start(at.x().round().max(0.0) as i32)
            .margin_top(at.y().round().max(0.0) as i32)
            .build();
        ghost.set_can_shrink(true);
        ghost.set_size_request(widget.width(), widget.height());
        self.root.add_overlay(&ghost);
        let began = std::cell::Cell::new(None::<i64>);
        let pace = super::glass_motion::Pace::default();
        let root = self.root.downgrade();
        ghost.add_tick_callback(move |ghost, clock| {
            let now = clock.frame_time();
            let start = *began.get().get_or_insert(now);
            began.set(Some(start));
            let since = (now - start) as f64 / 1e6;
            if since >= settle || !animations() {
                if let Some(root) = root.upgrade() {
                    root.remove_overlay(ghost);
                }
                return glib::ControlFlow::Break;
            }
            if pace.due(ghost, now, 0.0) {
                ghost.set_opacity(opacity(since));
            }
            glib::ControlFlow::Continue
        });
    }

    /// Shows the page `name`, the one before fading into it.
    pub(super) fn show(self: &Rc<Self>, name: &str) {
        let before = self.stack.visible_child();
        let after = self.stack.child_by_name(name);
        if after.is_none() || before == after {
            self.stack.set_visible_child_name(name);
            return;
        }
        self.finish();
        // A still of the step going away, taken while it is still shown.
        let still = before
            .filter(|b| {
                animations()
                    && !super::glass_motion::window_hidden(b)
                    && b.is_mapped()
                    && b.width() > 0
                    && b.height() > 0
            })
            .map(|b| {
                let still = gtk::WidgetPaintable::new(Some(&b)).current_image();
                (still, b.width(), b.height())
            });
        self.stack.set_visible_child_name(name);
        let (Some((still, width, height)), Some(after)) = (still, after) else {
            return;
        };
        self.ghost.set_paintable(Some(&still));
        self.ghost.set_size_request(width, height);
        self.ghost.set_opacity(1.0);
        self.ghost.set_visible(true);
        after.set_opacity(0.0);
        let began = std::cell::Cell::new(None::<i64>);
        let pace = super::glass_motion::Pace::default();
        let weak = Rc::downgrade(self);
        let id = self.root.add_tick_callback(move |root, clock| {
            let Some(fade) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let now = clock.frame_time();
            let start = *began.get().get_or_insert(now);
            began.set(Some(start));
            let since = (now - start) as f64 / 1e6;
            if since >= stage::SPATIAL_SETTLE {
                // The tick ends itself by returning Break.
                fade.settle(false);
                return glib::ControlFlow::Break;
            }
            if !pace.due(root, now, 0.0) {
                return glib::ControlFlow::Continue;
            }
            let shown = motion::crossfade(since, false);
            if let Some((_, after)) = fade.running.borrow().as_ref() {
                after.set_opacity(shown);
            }
            fade.ghost.set_opacity(1.0 - shown);
            glib::ControlFlow::Continue
        });
        *self.running.borrow_mut() = Some((id, after));
    }

    /// Ends a fade at its last frame: the new step whole, the still gone.
    fn finish(&self) {
        self.settle(true);
    }

    /// The last frame; `remove` takes the tick off too (not from inside it).
    fn settle(&self, remove: bool) {
        let running = self.running.borrow_mut().take();
        if let Some((id, after)) = running {
            if remove {
                id.remove();
            }
            after.set_opacity(1.0);
        }
        self.ghost.set_visible(false);
        self.ghost.set_paintable(None::<&gtk::gdk::Paintable>);
    }
}

/// Runs `restyle` once `widget` has been drawn with the style it was built
/// with (its second frame mapped), so a CSS transition runs from that style
/// to the one `restyle` gives: a widget rebuilt for a new state (the
/// thread's titles, Join's card) moves from the old state as the Mac's
/// does. At once with animations off, or if the widget is never shown.
pub(super) fn restyle_after_drawn(
    widget: &impl IsA<gtk::Widget>,
    restyle: impl FnOnce() + 'static,
) {
    if !animations() || super::glass_motion::window_hidden(widget) {
        restyle();
        return;
    }
    let restyle = RefCell::new(Some(restyle));
    let frames = std::cell::Cell::new(0u32);
    widget.add_tick_callback(move |widget, _| {
        frames.set(frames.get() + 1);
        if frames.get() < 2 || !widget.is_mapped() {
            return glib::ControlFlow::Continue;
        }
        if let Some(restyle) = restyle.borrow_mut().take() {
            restyle();
        }
        glib::ControlFlow::Break
    });
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

    /// A step change crossfades on the spatial spring: the step going away
    /// drawn as a still over the one coming in, both partway early on, the
    /// still gone and the new step whole by 1.4 s; with animations off the
    /// new step shows at once with no still.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn steps_crossfade_on_the_spatial_spring() {
        assert_eq!(std::env::consts::OS, "linux");
        let context = glib::MainContext::default();
        let _owner = context.acquire().unwrap();
        adw::init().expect("GTK display unavailable");
        let settings = gtk::Settings::default().unwrap();
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(true);
        let stack = gtk::Stack::new();
        let a = gtk::Label::new(Some("a"));
        let b = gtk::Label::new(Some("b"));
        stack.add_named(&a, Some("a"));
        stack.add_named(&b, Some("b"));
        let fade = StepFade::new(&stack);
        assert_eq!(stack.transition_type(), gtk::StackTransitionType::None);
        let window = adw::Window::builder()
            .default_width(400)
            .default_height(300)
            .content(fade.root())
            .build();
        window.present();
        settle(&context, 200);
        fade.show("b");
        assert!(fade.fading());
        assert!(fade.ghost().is_visible());
        assert!(
            fade.ghost().paintable().is_some(),
            "a still of the step going away"
        );
        assert!(!fade.ghost().can_target(), "nothing of it can be clicked");
        settle(&context, 250);
        let shown = b.opacity();
        assert!(shown > 0.0 && shown < 1.0, "partway: {shown}");
        assert!((fade.ghost().opacity() - (1.0 - shown)).abs() < 0.05);
        settle(&context, 1400);
        assert!(!fade.fading());
        assert!(!fade.ghost().is_visible());
        assert_eq!(b.opacity(), 1.0);

        settings.set_gtk_enable_animations(false);
        fade.show("a");
        assert!(!fade.fading());
        assert!(!fade.ghost().is_visible());
        assert_eq!(a.opacity(), 1.0);
        assert_eq!(stack.visible_child_name().as_deref(), Some("a"));

        // Turned off mid-fade, it ends at its last frame.
        settings.set_gtk_enable_animations(true);
        fade.show("b");
        assert!(fade.fading());
        settings.set_gtk_enable_animations(false);
        assert!(!fade.fading());
        assert_eq!(b.opacity(), 1.0);
        settings.set_gtk_enable_animations(previous);
        window.close();
    }
}
