// INTEGRATION: nested in ui::monitor::traces_list under
// #[cfg(all(test, target_os = "linux"))] with
// #[path = "../../../tests/support/traces_list_tests.rs"] mod mounted_tests;
//
// The Traces tab's folder list, mounted in the real App (Worker::fixture, no
// daemon: every call is recorded and stays unanswered), laid out by GTK and
// measured against the Mac's numbers, then driven: a folder opens to its
// traces, a trace that cannot be sent draws its Submit turned off, and
// turning a folder Automatic asks first and sends nothing until it is
// confirmed.
//
// Run alone in the isolated Weston/XDG session:
// cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
// --lib ui::monitor::traces_list::mounted_tests -- --exact --ignored --test-threads=1
#![cfg(target_os = "linux")]

use std::sync::mpsc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;

use super::size;
use crate::model::{Project, QueueEntry};
use crate::ui::App;
use crate::worker::{Job, Worker};

fn entry(id: &str, project: &str, minutes_ago: i64) -> QueueEntry {
    QueueEntry {
        entry_id: id.into(),
        project_id: project.into(),
        project_label: project.into(),
        source: "claude-code".into(),
        state: "pending".into(),
        size_bytes: 96_000,
        discovered_at: Some(chrono::Utc::now() - chrono::Duration::minutes(minutes_ago)),
        started_at: Some(chrono::Utc::now() - chrono::Duration::minutes(minutes_ago)),
        ..QueueEntry::default()
    }
}

fn project(id: &str, mode: &str) -> Project {
    serde_json::from_value(serde_json::json!({
        "project_id": id,
        "project_label": id,
        "project_path": format!("~/src/{id}"),
        "mode": mode,
        "configured": true,
    }))
    .expect("a project decodes")
}

fn settle(context: &glib::MainContext) {
    let deadline = Instant::now() + Duration::from_millis(400);
    while Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn descendants(root: &gtk::Widget, found: &mut Vec<gtk::Widget>) {
    let mut child = root.first_child();
    while let Some(widget) = child {
        found.push(widget.clone());
        descendants(&widget, found);
        child = widget.next_sibling();
    }
}

fn with_class(root: &gtk::Widget, class: &str) -> Vec<gtk::Widget> {
    let mut all = Vec::new();
    descendants(root, &mut all);
    all.into_iter()
        .filter(|w| w.is_visible() && w.has_css_class(class))
        .collect()
}

fn bounds(widget: &gtk::Widget, to: &gtk::Widget) -> gtk::graphene::Rect {
    widget.compute_bounds(to).expect("placed")
}

/// Where `inner` sits inside `row`'s drawn box (its fill, padding
/// included): both measured in `page`, so the answer does not depend on
/// whether a widget's own origin is its border or its content box.
fn offset_in(inner: &gtk::Widget, row: &gtk::Widget, page: &gtk::Widget) -> (f32, f32) {
    let (inner, row) = (bounds(inner, page), bounds(row, page));
    (
        inner.x() - row.x(),
        (row.x() + row.width()) - (inner.x() + inner.width()),
    )
}

fn calls(jobs: &mpsc::Receiver<(u64, Job)>, seen: &mut Vec<(String, serde_json::Value)>) {
    while let Ok((_, job)) = jobs.try_recv() {
        if let Job::Call { method, params } = job {
            seen.push((method, params));
        }
    }
}

fn dialogs() -> Vec<adw::MessageDialog> {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<adw::MessageDialog>().ok())
        .filter(|d| d.is_visible())
        .collect()
}

#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_the_folder_list_is_the_macs() {
    let context = glib::MainContext::default();
    let _owner = context.acquire().unwrap();
    adw::init().expect("GTK display unavailable");
    let application = adw::Application::builder()
        .application_id("ai.tracecommons.TracesListTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application
        .register(None::<&gtk::gio::Cancellable>)
        .unwrap();
    let dir = std::env::temp_dir().join(format!("tc-traces-list-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let (worker, jobs, _results) = Worker::fixture(dir.clone());
    let app = App::build(&application, worker);

    let mut blocked = entry("b1", "web", 5);
    blocked.eligibility = Some("ineligible_configuration".into());
    *app.projects.borrow_mut() = vec![
        project("api", "notify_only"),
        // The daemon's count: nothing in web can be sent.
        serde_json::from_value(serde_json::json!({
            "project_id": "web", "project_label": "web", "project_path": "~/src/web",
            "mode": "auto_upload", "configured": true,
            "pending_count": 1, "contributable_count": 0,
        }))
        .unwrap(),
        project("idle", "notify_only"),
        project("quiet", "ignore"),
    ];
    *app.entries.borrow_mut() = vec![entry("a1", "api", 1), entry("a2", "api", 30), blocked];
    crate::ui::queue::render(&app);
    assert!(app.open_screen("queue"), "the Traces tab opens");
    app.window.present();
    settle(&context);
    let page: gtk::Widget = app.queue.root.clone().upcast();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !(page.is_mapped() && page.width() > 0) && Instant::now() < deadline {
        settle(&context);
    }
    assert!(
        page.is_mapped(),
        "the Traces tab is not on screen to measure"
    );

    // The heading's round History button, `controlLarge` square.
    let round = with_class(&page, "tc-tree-round").remove(0);
    assert_eq!(
        (round.width(), round.height()),
        (size::ROUND_BUTTON, size::ROUND_BUTTON),
        "the History button"
    );

    // The column titles: the folders in this computer's words, Submit and
    // Auto, as the Mac's eyebrow capitals.
    let titles: Vec<String> = with_class(&page, "tc-tree-column")
        .iter()
        .map(|w| w.downcast_ref::<gtk::Label>().unwrap().text().to_string())
        .collect();
    assert_eq!(titles, ["FOLDERS ON THIS COMPUTER", "SUBMIT", "AUTO"]);
    let submit_title = &with_class(&page, "tc-tree-column")[1];
    assert!(
        submit_title.width() >= size::SUBMIT_TITLE,
        "Submit's title is {} wide",
        submit_title.width()
    );

    // A row per listed folder, the ignored one left out, newest waiting
    // trace first, the idle one after; every folder closed.
    let rows = with_class(&page, "tc-tree-row");
    assert_eq!(rows.len(), 3, "one row per folder, none open");
    let first_title = with_class(&rows[0], "tc-tree-title-folder")[0]
        .downcast_ref::<gtk::Label>()
        .unwrap()
        .text();
    assert_eq!(
        first_title.as_str(),
        "api",
        "the newest waiting trace's folder first"
    );

    // Laid out at the Mac's measures: a 40 row, a 22 tile 8 + 16 + 8 in,
    // and the row menu's 22 column ending 6 in from the row's end.
    for row in &rows {
        // One line of sub-line: exactly the Mac's least height.
        assert_eq!(row.height(), size::ROW, "a row {} high", row.height());
        let tile = with_class(row, "tc-tree-tile-folder").remove(0);
        let at = bounds(&tile, row);
        assert_eq!(
            (at.width() as i32, at.height() as i32),
            (size::TILE, size::TILE)
        );
        assert_eq!(
            offset_in(&tile, row, &page).0.round() as i32,
            super::leading_inset(super::Depth::Folder) + size::CHEVRON + size::COLUMN_GAP,
            "the tile's inset"
        );
        let kebab = with_class(row, "tc-tree-kebab").remove(0);
        assert_eq!(bounds(&kebab, row).width() as i32, size::KEBAB);
        assert_eq!(
            offset_in(&kebab, row, &page).1.round() as i32,
            size::TRAILING,
            "the row menu's trailing inset"
        );
    }
    // The rows sit 2 apart.
    let gap = bounds(&rows[1], &page).y() - bounds(&rows[0], &page).y() - rows[0].height() as f32;
    assert!(gap >= size::ROW_GAP as f32 - 0.5, "rows {gap} apart");

    // api offers Submit · 2 and an Automatic switch that is off; web,
    // whose only trace cannot go, offers no Submit at all.
    let api = &rows[0];
    let pill = with_class(api, "tc-tree-submit").remove(0);
    assert_eq!(pill.height(), size::SUBMIT_PILL, "the Submit pill's height");
    // The Automatic switch is the Mac's watch switch, and the Auto title
    // stands over it: their right edges meet.
    let switch = with_class(api, "tc-tree-switch").remove(0);
    // Its drawn box, padding included (`width()` is the content box).
    let drawn = bounds(&switch, &page);
    assert_eq!(
        (drawn.width().round() as i32, drawn.height().round() as i32),
        (size::SWITCH, size::SWITCH_HEIGHT),
        "the Automatic switch"
    );
    let auto = &with_class(&page, "tc-tree-column")[2];
    let title_end = bounds(auto, &page).x() + bounds(auto, &page).width();
    let switch_end = bounds(&switch, &page).x() + bounds(&switch, &page).width();
    assert!(
        (title_end - switch_end).abs() <= 1.0,
        "Auto ends at {title_end}, its switches at {switch_end}"
    );
    assert_eq!(
        pill.downcast_ref::<gtk::Button>()
            .unwrap()
            .label()
            .as_deref(),
        Some("Submit \u{00b7} 2")
    );
    let web = rows
        .iter()
        .find(|row| {
            with_class(row, "tc-tree-title-folder")[0]
                .downcast_ref::<gtk::Label>()
                .unwrap()
                .text()
                .as_str()
                == "web"
        })
        .unwrap();
    assert!(
        with_class(web, "tc-tree-submit").is_empty(),
        "no Submit for nothing sendable"
    );

    // The chevron opens api: its two traces under it, newest first, each
    // with its tool's tile 18 further in than the folder's.
    let chevron = with_class(api, "tc-tree-chevron").remove(0);
    chevron
        .downcast_ref::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    settle(&context);
    let rows = with_class(&page, "tc-tree-row");
    assert_eq!(rows.len(), 5, "api opened to its two traces");
    let trace = &rows[1];
    let tile = with_class(trace, "tc-tree-tile").remove(0);
    assert_eq!(
        offset_in(&tile, trace, &page).0.round() as i32,
        super::leading_inset(super::Depth::Session) + size::CHEVRON + size::COLUMN_GAP,
        "a trace's tile is one level in"
    );
    let pill = with_class(trace, "tc-tree-submit").remove(0);
    assert!(pill.is_sensitive(), "an eligible trace's Submit is live");
    // Open web too: its trace's Submit is drawn, turned off.
    let web_chevron = with_class(&rows[3], "tc-tree-chevron");
    if let Some(chevron) = web_chevron.first() {
        chevron
            .downcast_ref::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        settle(&context);
    }
    let blocked_pills: Vec<_> = with_class(&page, "tc-tree-submit")
        .into_iter()
        .filter(|w| !w.is_sensitive())
        .collect();
    assert_eq!(
        blocked_pills.len(),
        1,
        "the ineligible trace's Submit is off"
    );

    // Automatic on api asks first; Not now sends nothing.
    let mut seen = Vec::new();
    calls(&jobs, &mut seen);
    seen.clear();
    let switch = with_class(&with_class(&page, "tc-tree-row")[0], "tc-tree-switch").remove(0);
    switch
        .downcast_ref::<gtk::Switch>()
        .unwrap()
        .set_active(true);
    settle(&context);
    let asked = dialogs();
    assert_eq!(asked.len(), 1, "turning a folder Automatic asks first");
    asked[0].response("cancel");
    settle(&context);
    calls(&jobs, &mut seen);
    assert!(
        !seen.iter().any(|(method, _)| method == "set_project_mode"),
        "nothing armed without the confirmation: {seen:?}"
    );
    let switch = with_class(&with_class(&page, "tc-tree-row")[0], "tc-tree-switch").remove(0);
    assert!(
        !switch.downcast_ref::<gtk::Switch>().unwrap().is_active(),
        "the switch shows the daemon's mode"
    );

    // Offscreen renders of the tab, for the review, when asked for.
    if let Ok(out) = std::env::var("TC_RENDER_DIR") {
        for (scheme, dark) in [("light", false), ("dark", true)] {
            adw::StyleManager::default().set_color_scheme(if dark {
                adw::ColorScheme::ForceDark
            } else {
                adw::ColorScheme::ForceLight
            });
            settle(&context);
            let (width, height) = (page.width(), page.height());
            let paintable = gtk::WidgetPaintable::new(Some(&page));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
            if let Some(node) = snapshot.to_node() {
                let renderer =
                    gtk::gsk::Renderer::for_surface(&app.window.surface().unwrap()).unwrap();
                let texture = renderer.render_texture(&node, None);
                renderer.unrealize();
                let _ = texture.save_to_png(format!("{out}/traces-list-{scheme}.png"));
            }
        }
    }

    app.window.close();
    let _ = std::fs::remove_dir_all(dir);
}

/// The folder list's stock icons resolve in the theme the app runs with.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_the_folder_lists_icons_are_in_the_theme() {
    adw::init().expect("GTK display unavailable");
    let display = gtk::gdk::Display::default().expect("a display");
    let theme = gtk::IconTheme::for_display(&display);
    for name in [
        "pan-end-symbolic",
        "pan-down-symbolic",
        "view-more-symbolic",
        "document-open-recent-symbolic",
    ] {
        assert!(theme.has_icon(name), "{name} is not in the icon theme");
    }
}

/// The row menu's Dismiss trace asks first, through the queue's one
/// confirmation (`queue::confirm_dismiss`), as the Mac's `dismissModal`:
/// Keep it sends nothing, and only Dismiss sends `dismiss` for that trace.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_the_row_menus_dismiss_asks_first() {
    let context = glib::MainContext::default();
    let _owner = context.acquire().unwrap();
    adw::init().expect("GTK display unavailable");
    let application = adw::Application::builder()
        .application_id("ai.tracecommons.TracesListDismissTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application
        .register(None::<&gtk::gio::Cancellable>)
        .unwrap();
    let dir = std::env::temp_dir().join(format!("tc-traces-dismiss-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let (worker, jobs, _results) = Worker::fixture(dir.clone());
    let app = App::build(&application, worker);
    app.window.present();
    settle(&context);

    let tree = trace_commons_contributor::preview_copy::monitor_traces_copy().tree;
    let row = super::session_row(&app, &entry("d1", "api", 3), false, false);
    let kebab = with_class(&row, "tc-tree-kebab")
        .remove(0)
        .downcast::<gtk::MenuButton>()
        .expect("the row menu is a menu button");
    let popover = kebab.popover().expect("the row menu's popover");
    let mut items = Vec::new();
    descendants(popover.upcast_ref(), &mut items);
    let dismiss = items
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Button>().ok())
        .find(|b| b.label().as_deref() == Some(tree.dismiss_session))
        .expect("the row menu offers Dismiss trace");

    let mut seen = Vec::new();
    calls(&jobs, &mut seen);
    seen.clear();
    let sent_dismiss = |seen: &[(String, serde_json::Value)]| {
        seen.iter()
            .filter(|(method, _)| method == "dismiss")
            .map(|(_, params)| params.clone())
            .collect::<Vec<_>>()
    };

    // Picking it asks, in the core's words, and sends nothing yet.
    dismiss.emit_clicked();
    settle(&context);
    let asked = dialogs();
    assert_eq!(asked.len(), 1, "Dismiss trace asks first");
    assert_eq!(
        asked[0].heading().as_deref(),
        Some(tree.dismiss_session_title)
    );
    calls(&jobs, &mut seen);
    assert!(
        sent_dismiss(&seen).is_empty(),
        "sent before the answer: {seen:?}"
    );

    // Keep it sends nothing.
    asked[0].response("keep");
    settle(&context);
    calls(&jobs, &mut seen);
    assert!(sent_dismiss(&seen).is_empty(), "Keep it sent: {seen:?}");
    assert!(dialogs().is_empty(), "Keep it closes the dialog");

    // Dismiss sends `dismiss` for this trace, once.
    dismiss.emit_clicked();
    settle(&context);
    let asked = dialogs();
    assert_eq!(asked.len(), 1, "asked again");
    asked[0].response("dismiss");
    settle(&context);
    calls(&jobs, &mut seen);
    assert_eq!(
        sent_dismiss(&seen),
        [serde_json::json!({"entry_id": "d1"})],
        "one dismiss for the trace"
    );

    app.window.close();
    let _ = std::fs::remove_dir_all(dir);
}
