// Mounted test for the terms hold, nested in ui::terms_hold. Linux only;
// run alone in the Weston session with --exact --ignored --test-threads=1.

use adw::prelude::*;

use crate::ui::consent_fixture::Fixture;

fn labels(root: &gtk::Widget) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(widget) = pending.pop() {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            out.push(label.text().to_string());
        }
        if let Some(button) = widget.downcast_ref::<gtk::Button>() {
            out.push(format!("button:{}", button.label().unwrap_or_default()));
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            pending.push(current);
        }
    }
    out
}

/// While the daemon holds uploads for new terms, the notice says so in the
/// core's words, with its one button and no close; it goes with the hold.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_terms_hold_notice_follows_the_hold() {
    let fixture = Fixture::new("TermsHold", &[]);
    let words = trace_commons_contributor::first_run_copy::first_run_copy().terms_hold;
    let held: crate::model::Status =
        serde_json::from_value(serde_json::json!({"terms_hold": true})).unwrap();
    fixture.app.render_grant_voids(&held);
    let shown = labels(fixture.app.void_notices.upcast_ref());
    assert!(shown.iter().any(|l| l == words.notice_title));
    assert!(shown.iter().any(|l| l == words.notice_body));
    let buttons: Vec<_> = shown.iter().filter(|l| l.starts_with("button:")).collect();
    assert_eq!(buttons, [&format!("button:{}", words.review)]);

    let lifted: crate::model::Status = serde_json::from_value(serde_json::json!({})).unwrap();
    fixture.app.render_grant_voids(&lifted);
    let shown = labels(fixture.app.void_notices.upcast_ref());
    assert!(!shown.iter().any(|l| l == words.notice_title));
}
