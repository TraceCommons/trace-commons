// Mounted consent tests for Settings, nested in ui::settings. Linux only;
// run alone in the Weston session with --exact --ignored --test-threads=1.

use adw::prelude::*;

use crate::ui::consent_fixture::Fixture;

pub(super) fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut out = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(widget) = pending.pop() {
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            pending.push(current);
        }
        out.push(widget);
    }
    out
}

/// The Mac offers no control over the approval hold; neither does this
/// window. Only the quiet-time and interruption knobs remain.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_settings_have_no_undo_window_control() {
    let fixture = Fixture::new("NoHoldKnob", &[]);
    let view = &fixture.app.settings;
    let known: [&gtk::Widget; 3] = [
        view.quiescence_minutes.upcast_ref(),
        view.digest_hours.upcast_ref(),
        view.routing_port.upcast_ref(),
    ];
    for spin in descendants(view.root.upcast_ref())
        .into_iter()
        .filter(|w| w.is::<gtk::SpinButton>())
    {
        assert!(
            known.contains(&&spin),
            "a dial other than quiet time, interruptions or the routing port"
        );
    }
}
