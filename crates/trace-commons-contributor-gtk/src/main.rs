//! The Linux contributor shell.
//!
//! Starting this application does not require a daemon and does not fail if
//! one is already running: whoever holds the exclusive lock runs the loop,
//! and the other connects. See `backend`.
//!
//! `--exit-after-realize` starts everything -- backend, window, widgets,
//! subscription -- and then quits. It is what the headless container run
//! uses: enough to prove the application starts and talks to a daemon, and
//! honest about proving nothing at all about how the result looks.

use adw::prelude::*;
use trace_commons_contributor_gtk::{ui, worker::Worker};

fn main() -> anyhow::Result<()> {
    // Answered before anything is initialised, so a person can identify the
    // build they installed without a display or a daemon. Same ad hoc argument
    // idiom as the flags below.
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!(
            "{}",
            trace_commons_build_info::identity(env!("CARGO_BIN_NAME"), env!("CARGO_PKG_VERSION"))
        );
        return Ok(());
    }

    let exit_after_realize = std::env::args().any(|a| a == "--exit-after-realize");
    // How long to stay up before quitting, so a headless run has time to be
    // photographed before the process leaves.
    let realize_seconds: u32 = std::env::args()
        .position(|a| a == "--realize-seconds")
        .and_then(|i| std::env::args().nth(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let open_preview = std::env::args().any(|a| a == "--open-preview");
    let search_term = std::env::args()
        .position(|a| a == "--search")
        .and_then(|i| std::env::args().nth(i + 1));
    // Which tab of the preview sheet to photograph. "search" is what a
    // person lands on; "would-be-sent" is the one a capture run has to ask
    // for, and it is the one carrying the redaction highlighting.
    let preview_tab = std::env::args()
        .position(|a| a == "--preview-tab")
        .and_then(|i| std::env::args().nth(i + 1));
    let start_page = std::env::args()
        .position(|a| a == "--start-page")
        .and_then(|i| std::env::args().nth(i + 1));
    // A state directory can be named explicitly, mostly so a container run
    // does not touch a real one.
    let dir = match std::env::args().position(|a| a == "--state-dir") {
        Some(i) => std::env::args()
            .nth(i + 1)
            .map(std::path::PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("--state-dir needs a directory"))?,
        None => trace_commons_contributor_gtk::state_dir()?,
    };

    // The `x-scheme-handler/tracecommons` registration in the desktop entry
    // launches us with the URL as an argument, so an invite clicked in mail
    // lands on the Connect screen instead of being retyped.
    //
    // Two things this deliberately does not do. It does not enrol: the
    // invite is filled in and the button is left for a person to press,
    // because which commons to join is the decision that screen exists to
    // ask. And it does not log the URL -- an invite is a credential, and
    // `max_uses` on the registry side means a captured one stays usable.
    //
    // Worth knowing: unlike the macOS URL-event path, a scheme handler
    // receives this as argv, which is readable by other processes on this
    // machine via /proc. That exposure is inherent to scheme handlers and
    // is why the pasted path stays the recommended one for an invite with
    // a large `max_uses`.
    if let Some(invite) = std::env::args()
        .skip(1)
        .find_map(|a| trace_commons_contributor_gtk::ui::onboarding::invite_from_deep_link(&a))
    {
        trace_commons_contributor_gtk::ui::onboarding::set_pending_invite(invite);
    }

    let application = adw::Application::builder()
        .application_id(ui::APP_ID)
        // The window is the primary surface on this platform, so the
        // ordinary GTK behaviour -- the process lives as long as a window
        // does -- is the right one. Nothing here depends on a tray.
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();

    application.connect_activate(move |application| {
        let worker = match Worker::start(dir.clone()) {
            Ok(worker) => worker,
            Err(error) => {
                // Fixed labels only: this string can reach a journal.
                eprintln!("trace-commons-shell: cannot reach the contributor state: {error}");
                std::process::exit(1);
            }
        };
        let app = ui::App::build(application, worker);
        app.window.present();

        // Debug drivers for the headless container run. They open a surface
        // that a person would otherwise have to click to, so a screenshot
        // can show it. Neither approves anything: there is no flag in this
        // application that contributes a trace without a person pressing
        // Contribute in the preview sheet.
        if let Some(page) = start_page.clone() {
            app.stack.set_visible_child_name(&page);
        }
        if open_preview {
            let app = app.clone();
            let search_term = search_term.clone();
            let preview_tab = preview_tab.clone();
            gtk::glib::timeout_add_seconds_local(3, move || {
                ui::preview::open_with_search(&app, 0, search_term.clone(), preview_tab.clone());
                gtk::glib::ControlFlow::Break
            });
        }

        if exit_after_realize {
            let application = application.clone();
            gtk::glib::timeout_add_seconds_local(realize_seconds, move || {
                println!("trace-commons-shell: realized, quitting (--exit-after-realize)");
                application.quit();
                gtk::glib::ControlFlow::Break
            });
        }
    });

    // GTK's own argument parsing would choke on the flags above.
    application.run_with_args::<&str>(&[]);
    Ok(())
}
