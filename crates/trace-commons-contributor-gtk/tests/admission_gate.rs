//! The sheet's admission-preparation control, checked against its own source.
//!
//! `admission_control_visible` is unit-tested next to itself in
//! `ui::preview`, and a mutation inside it goes red there. The wiring that
//! hands it the enrolment is not: it lives on a `gtk::Button` inside a
//! `Sheet` that cannot be built without a display, so a call site that
//! passed a literal `true` for `required` -- offering the button to a
//! contributor enrolled by invite, whose only possible outcome is a refusal
//! -- would leave every test green.
//!
//! That exact mistake is what an audit believed it had found here. It had
//! not; the wiring is correct and has been since the control was added. This
//! reads the source from outside, the way `shell_wording.rs` does and the
//! way `AdmissionPreparationTests.cs` does on Windows, so that the next edit
//! to `sync_witness` cannot quietly make the audit right.

use std::path::Path;

#[test]
fn the_sheet_asks_the_enrolment_before_it_shows_the_preparation_control() {
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/preview.rs"),
    )
    .expect("preview.rs is readable");

    // Prose is stripped first. A guard that fails source for naming a symbol
    // in a doc comment teaches the next reader to delete the explanation.
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    let needle = "set_visible(admission_control_visible(";
    let calls: Vec<usize> = code.match_indices(needle).map(|(at, _)| at).collect();
    assert_eq!(
        calls.len(),
        1,
        "expected exactly one place to decide the control's visibility, found {}",
        calls.len()
    );

    let arguments = &code[calls[0] + needle.len()..];
    let end = arguments
        .find("));")
        .expect("the visibility call is closed on the same statement");
    let arguments = &arguments[..end];

    assert!(
        arguments.contains("self.admission_required.get()"),
        "the visibility call does not consult the enrolment: {arguments:?}"
    );
    assert!(
        arguments.contains("self.admission_supported.get()"),
        "the visibility call does not consult the daemon's method list: {arguments:?}"
    );
    assert!(
        arguments.contains("self.pinned.get()"),
        "the visibility call does not consult the pinned gate: {arguments:?}"
    );
}
