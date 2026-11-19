// INTEGRATION: nested in ui::monitor::shell under #[cfg(test)] with
// #[path = "../../../tests/support/main_pane_fit_tests.rs"] mod fit_tests;
// A child of the shell module, so it reads the shell's own fields.
//
// The review's defect (fix pass on #2160): the hosted screens were wider than
// the Monitor's 440 main pane and were clipped at its right edge. This builds
// the real App (Worker::fixture, no daemon: every call stays unanswered),
// fills the Traces tab with folders and sessions whose names and paths are
// long, and requires every hosted screen and every notice card to fit 440
// with no sideways scrolling.
//
// Run alone in the existing isolated Weston/XDG session:
// cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
// --lib ui::monitor::shell::fit_tests -- --exact --ignored --test-threads=1
#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;

use super::super::pane_layout::MAIN_WIDTH;
use crate::model::{Project, QueueEntry};
use crate::ui::App;
use crate::worker::Worker;

fn entry(id: &str, project: &str, label: &str) -> QueueEntry {
    QueueEntry {
        entry_id: id.into(),
        project_id: project.into(),
        project_label: label.into(),
        project_path: format!("~/src/clients/{label}/services/{label}-ingest"),
        source: "claude-code".into(),
        state: "pending".into(),
        size_bytes: 96_000,
        discovered_at: Some(chrono::Utc::now()),
        ..QueueEntry::default()
    }
}

fn project(id: &str, label: &str) -> Project {
    serde_json::from_value(serde_json::json!({
        "project_id": id,
        "project_label": label,
        "project_path": format!("~/src/clients/{label}"),
        "mode": "review",
        "configured": true,
    }))
    .expect("a project decodes")
}

/// The least width `widget` can be given.
fn least(widget: &impl IsA<gtk::Widget>) -> i32 {
    widget.measure(gtk::Orientation::Horizontal, -1).0
}

/// Every shown folder-list row under `root`, in drawing order.
fn collect_rows(root: &gtk::Widget, found: &mut Vec<gtk::Widget>) {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if widget.is_visible() {
            if widget.has_css_class("tc-tree-row") {
                found.push(widget.clone());
            } else {
                collect_rows(&widget, found);
            }
        }
        child = widget.next_sibling();
    }
}

/// The shown buttons under `root`.
fn count_buttons(root: &impl IsA<gtk::Widget>) -> usize {
    let mut count = 0;
    let mut child = root.as_ref().first_child();
    while let Some(widget) = child {
        if widget.is_visible() {
            if (widget.is::<gtk::Button>()
                && !widget
                    .parent()
                    .is_some_and(|parent| parent.is::<gtk::MenuButton>()))
                || widget.is::<gtk::MenuButton>()
            {
                count += 1;
            }
            count += count_buttons(&widget);
        }
        child = widget.next_sibling();
    }
    count
}

fn settle(context: &glib::MainContext) {
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn every_hosted_screen_fits_the_440_main_pane() {
    let context = glib::MainContext::default();
    let _owner = context.acquire().unwrap();
    adw::init().expect("GTK display unavailable");
    let application = adw::Application::builder()
        .application_id("ai.tracecommons.MainPaneFitTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application
        .register(None::<&gtk::gio::Cancellable>)
        .unwrap();
    let dir = std::env::temp_dir().join(format!("tc-main-pane-fit-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let (worker, _jobs, _results) = Worker::fixture(dir.clone());
    let app = App::build(&application, worker);

    // Two folders on the Traces tab, every group control drawn.
    let labels = [("p1", "payments-reconciliation-service"), ("p2", "web")];
    *app.projects.borrow_mut() = labels
        .iter()
        .map(|(id, label)| project(id, label))
        .collect();
    *app.entries.borrow_mut() = vec![
        entry("e1", "p1", labels[0].1),
        entry("e2", "p1", labels[0].1),
        entry("e3", "p2", labels[1].1),
    ];
    crate::ui::queue::render(&app);
    app.window.present();
    settle(&context);

    let shell = &app.shell;
    assert_eq!(
        shell.content_scroller().hscrollbar_policy(),
        gtk::PolicyType::Never,
        "the main pane scrolls sideways"
    );
    let pane = MAIN_WIDTH as i32;
    let check = |what: &str| {
        let mut page = shell.content.first_child();
        while let Some(widget) = page {
            let name = shell
                .content
                .page(&widget)
                .name()
                .map(|name| name.to_string())
                .unwrap_or_default();
            assert!(
                least(&widget) <= pane,
                "{what}: the {name} screen needs {} wide, the pane is {pane}",
                least(&widget)
            );
            page = widget.next_sibling();
        }
    };
    check("folders");
    // History with a community panel: its metric strip once made the left
    // pane's whole page stack wider than 440, clipping every left-pane page.
    crate::ui::history::show_community_for_test(&app);
    settle(&context);
    check("history with a community panel");
    // Laid out, not only measured (fix pass 3): every button in each folder
    // row is drawn inside its row, and no row runs under the next. On
    // GTK 4.14 the folder cards' "Submit as…" hung over a card's foot and
    // "Never" was not drawn.
    shell.pick_tab(super::Tab::Traces);
    settle(&context);
    let traces: gtk::Widget = shell
        .content
        .child_by_name("queue")
        .expect("the Traces tab");
    let laid_out = |what: &str| {
        let mut rows = Vec::new();
        collect_rows(&traces, &mut rows);
        assert!(!rows.is_empty(), "{what}: no folder row is drawn");
        let mut buttons = 0;
        for (index, row) in rows.iter().enumerate() {
            buttons += count_buttons(row);
            let outside = crate::ui::wrap_row::buttons_outside(row, row);
            assert!(outside.is_empty(), "{what}: row {index}: {outside:?}");
            if let Some(next) = rows.get(index + 1) {
                let this = row.compute_bounds(&traces).expect("placed");
                let below = next.compute_bounds(&traces).expect("placed");
                assert!(
                    this.y() + this.height() <= below.y() + 0.5,
                    "{what}: row {index} {this:?} runs under the next {below:?}"
                );
            }
        }
        buttons
    };
    let buttons = laid_out("folders");
    assert!(buttons >= 3, "the folder rows draw {buttons} buttons");

    // A folder opened: its traces under it, each with its Submit and menu.
    app.queue.expanded.borrow_mut().insert("p1".into());
    crate::ui::queue::render(&app);
    settle(&context);
    laid_out("a folder's sessions");
    check("a folder's sessions");

    // Every notice card, shown over its tab, fits with its X inside it.
    for slot in &shell.notice_slots {
        if let Some(frame) = slot.card.parent() {
            assert!(
                least(&frame) <= pane,
                "the {:?} notice needs {} wide",
                slot.notice,
                least(&frame)
            );
        }
    }
    // The Private AI offer, in front on its tab: "Not now" and "Turn on"
    // both drawn whole, inside the stack laid over the tab and inside the
    // card. On GTK 4.14 "Turn on" was clipped away.
    let settings: crate::model::Settings =
        serde_json::from_value(serde_json::json!({})).expect("settings decode");
    crate::ui::queue::render_private_inference_offer(&app, &settings);
    shell.pick_tab(super::Tab::Inference);
    settle(&context);
    let offer = shell
        .notice_slots
        .iter()
        .find(|slot| slot.notice == super::Notice::PrivateAiOffer)
        .expect("the offer has a slot");
    assert!(offer.frame.is_visible(), "the offer is not in front");
    let stack = offer
        .frame
        .ancestor(gtk::ScrolledWindow::static_type())
        .expect("the notices are in a stack that scrolls");
    assert_eq!(count_buttons(&offer.card), 2, "the offer's two buttons");
    let outside = crate::ui::wrap_row::buttons_outside(&offer.card, &stack);
    assert!(outside.is_empty(), "outside the stack: {outside:?}");
    let outside = crate::ui::wrap_row::buttons_outside(&offer.card, &offer.card);
    assert!(outside.is_empty(), "outside the card: {outside:?}");

    // And the whole main pane at the least: the column the overlay the
    // notices are laid over sits in.
    let main = shell
        .content_scroller()
        .ancestor(gtk::Overlay::static_type())
        .and_then(|overlay| overlay.parent())
        .expect("the main pane");
    assert!(
        least(&main) <= pane,
        "the main pane needs {} wide",
        least(&main)
    );
    app.window.close();
    let _ = std::fs::remove_dir_all(dir);
}
