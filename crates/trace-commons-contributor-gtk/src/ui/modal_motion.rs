//! The Mac's modal motion for the shell's confirmations
//! (`GlassModalHost` in `macos/Sources/TCDesign/Components/Modal.swift`):
//! the modal fades in over `motion.standard` on the token curve, and what
//! it covers is blurred by `size.modalScrimBlur` under `modalScrim`'s dim.
//!
//! The Mac draws its modal inside the window; here a confirmation is its
//! own window (`adw::MessageDialog`), transient for the one it covers. So
//! the dialog's window fades in, and the covered window's content takes the
//! blur and the dim (`motion_css`: a CSS filter, which fades over the same
//! 0.22 s, and GTK drops while `gtk-enable-animations` is off). Two
//! differences remain: the blur fades with the dim, where the Mac's blur
//! is at once; and the dialog goes at once when answered, because the
//! toolkit destroys it, while the covered window's blur and dim still fade
//! back out.
//!
//! One watcher on the list of toplevels reaches every confirmation, with
//! no change at the 25 places that raise one.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::glass_motion::{self, Tween};
use super::motion_css::{MODAL_COVERED, MODAL_HOST};
use super::style::brand_tokens::motion;

/// How long the modal takes to fade in: `GlassMotion.standard`.
pub const FADE: f64 = motion::STANDARD;

/// Watches every toplevel the shell opens, once per thread.
pub fn install() {
    thread_local! {
        static INSTALLED: Cell<bool> = const { Cell::new(false) };
    }
    if INSTALLED.with(|done| done.replace(true)) {
        return;
    }
    let toplevels = gtk::Window::toplevels();
    for index in 0..toplevels.n_items() {
        if let Some(window) = toplevels.item(index) {
            watch(&window);
        }
    }
    toplevels.connect_items_changed(|list, position, _removed, added| {
        for index in position..position + added {
            if let Some(window) = list.item(index) {
                watch(&window);
            }
        }
    });
}

/// Every window is watched and the type checked when it is shown: a GTK
/// that lists a window while it is still being built would show it as a
/// plain `GtkWindow` here.
fn watch(object: &glib::Object) {
    if let Some(window) = object.downcast_ref::<gtk::Window>() {
        cover(window);
    }
}

fn is_confirmation(window: &gtk::Window) -> bool {
    window.is::<adw::MessageDialog>()
}

/// The content a modal over `window` blurs and dims.
fn covered(window: &gtk::Window) -> Option<gtk::Widget> {
    window.child()
}

/// Whether another mapped confirmation still covers `parent`.
fn still_covered(parent: &gtk::Window, leaving: &gtk::Window) -> bool {
    let toplevels = gtk::Window::toplevels();
    (0..toplevels.n_items())
        .filter_map(|i| toplevels.item(i).and_downcast::<adw::MessageDialog>())
        .any(|other| {
            other.upcast_ref::<gtk::Window>() != leaving
                && other.is_mapped()
                && other.transient_for().as_ref() == Some(parent)
        })
}

/// Fades `dialog` in each time it is shown and covers its parent while it
/// is up, if it is a confirmation.
pub fn cover(dialog: &gtk::Window) {
    let fade = Rc::new(Tween::new(1.0));
    let parent: Rc<RefCell<Option<glib::WeakRef<gtk::Window>>>> = Rc::default();
    {
        let fade = Rc::clone(&fade);
        let parent = Rc::clone(&parent);
        dialog.connect_map(move |dialog| {
            if !is_confirmation(dialog) {
                return;
            }
            if let Some(window) = dialog.transient_for() {
                if let Some(content) = covered(&window) {
                    content.add_css_class(MODAL_HOST);
                    content.add_css_class(MODAL_COVERED);
                }
                *parent.borrow_mut() = Some(window.downgrade());
            }
            fade.set(0.0, |_| {});
            dialog.set_opacity(0.0);
            let weak = dialog.downgrade();
            fade.to(dialog, 1.0, FADE, glass_motion::curve, move |opacity| {
                if let Some(dialog) = weak.upgrade() {
                    dialog.set_opacity(opacity);
                }
            });
        });
    }
    dialog.connect_unmap(move |dialog| {
        if !is_confirmation(dialog) {
            return;
        }
        dialog.set_opacity(1.0);
        let Some(window) = parent.borrow_mut().take().and_then(|w| w.upgrade()) else {
            return;
        };
        if still_covered(&window, dialog) {
            return;
        }
        if let Some(content) = covered(&window) {
            content.remove_css_class(MODAL_COVERED);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use adw::prelude::*;

    /// The fade is the Mac's: `GlassMotion.standard`, 0.22 s.
    #[test]
    fn the_modal_fades_over_the_macs_standard() {
        let modal = include_str!("../../../../macos/Sources/TCDesign/Components/Modal.swift");
        assert!(modal.contains("// #1146's `tc-fade` on its scrim, at `--tc-dur`."));
        assert!(modal.contains(".transition(.opacity)"));
        assert_eq!(FADE, 0.22);
    }

    /// A confirmation fades in through the middle, blurs and dims the
    /// window it covers while it is up, and gives it back when answered.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_a_confirmation_fades_in_and_covers_its_window() {
        adw::init().unwrap();
        let settings = gtk::Settings::default().unwrap();
        settings.set_gtk_enable_animations(true);
        install();
        let window = gtk::Window::new();
        let content = gtk::Label::new(Some("behind"));
        window.set_child(Some(&content));
        window.present();
        let dialog = adw::MessageDialog::new(Some(&window), Some("Sure?"), None);
        dialog.add_response("cancel", "Cancel");
        let context = glib::MainContext::default();
        let pump_until = |done: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !done() && std::time::Instant::now() < deadline {
                context.iteration(false);
            }
        };
        dialog.present();
        pump_until(&|| dialog.is_mapped());
        assert!(
            content.has_css_class(MODAL_COVERED),
            "the window is not covered"
        );
        let between = Cell::new(false);
        pump_until(&|| {
            let o = dialog.opacity();
            between.set(between.get() || (o > 0.0 && o < 1.0));
            o == 1.0
        });
        assert!(between.get(), "the confirmation did not fade in");
        dialog.close();
        pump_until(&|| !dialog.is_mapped());
        assert!(
            !content.has_css_class(MODAL_COVERED),
            "the window stayed covered"
        );
        assert!(
            content.has_css_class(MODAL_HOST),
            "the dim has nothing to fade out on"
        );
        window.destroy();
    }
}
