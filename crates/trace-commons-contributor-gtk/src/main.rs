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
    // The main window's stack and onboarding's steps are different things:
    // `--start-page` moves the former, and onboarding is a modal with its own
    // pages that had no way to be opened past its first screen.
    let onboarding_page = std::env::args()
        .position(|a| a == "--onboarding-page")
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

    let drivers = std::rc::Rc::new(Drivers {
        exit_after_realize,
        realize_seconds,
        open_preview,
        search_term,
        preview_tab,
        start_page,
        onboarding_page,
    });

    application.connect_activate(move |application| {
        start_or_ask(application, dir.clone(), drivers.clone());
    });

    // GTK's own argument parsing would choke on the flags above.
    application.run_with_args::<&str>(&[]);
    Ok(())
}

/// The headless-run drivers, carried together so the start path can be
/// re-entered after the roots screen without threading six arguments.
struct Drivers {
    exit_after_realize: bool,
    realize_seconds: u32,
    open_preview: bool,
    search_term: Option<String>,
    preview_tab: Option<String>,
    start_page: Option<String>,
    onboarding_page: Option<String>,
}

/// Start the shell, or ask which folders it may watch and then start it.
///
/// Undeclared session roots are the one start failure with an answer, so it
/// is the one that opens a window instead of leaving. Every other failure is
/// still fatal: a shell that cannot reach its own state directory has nothing
/// to offer a contributor, and turning that into a window would be pretending
/// otherwise.
///
/// Before this existed, `Worker::start` failing meant `std::process::exit(1)`
/// for every reason including this one -- so a Linux contributor who had
/// never declared their roots got a process that vanished with a line in the
/// journal they would never see.
fn start_or_ask(
    application: &adw::Application,
    dir: std::path::PathBuf,
    drivers: std::rc::Rc<Drivers>,
) {
    let worker = match Worker::start(dir.clone()) {
        Ok(worker) => worker,
        Err(error)
            if error.to_string()
                == trace_commons_contributor_gtk::backend::ERR_ROOTS_NOT_DECLARED =>
        {
            let reentry = application.clone();
            let ask_dir = dir.clone();
            // The headless run has to be able to reach this window too.
            // Without it, `--exit-after-realize` against an undeclared state
            // directory would hang forever, and the smoke run that is
            // supposed to prove the shell starts would prove nothing about
            // the one path that used to make it disappear.
            if drivers.exit_after_realize {
                let application = application.clone();
                gtk::glib::timeout_add_seconds_local(drivers.realize_seconds, move || {
                    println!("trace-commons-shell: roots screen realized, quitting");
                    application.quit();
                    gtk::glib::ControlFlow::Break
                });
            }
            ui::roots::present(application, dir, move || {
                // Re-entered rather than continuing inline: the declaration
                // is on disk now, so this is an ordinary start, and the
                // second refusal that cannot happen would still be handled
                // the same way if it did.
                start_or_ask(&reentry, ask_dir.clone(), drivers.clone());
            });
            return;
        }
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
    if let Some(page) = drivers.start_page.clone() {
        app.stack.set_visible_child_name(&page);
    }
    if let Some(page) = drivers.onboarding_page.clone() {
        let app = app.clone();
        // After the status handler has had its chance to present onboarding
        // itself, so this replaces that window rather than racing it.
        gtk::glib::timeout_add_seconds_local(2, move || {
            if !ui::onboarding::present_at_page(&app, &page) {
                eprintln!("trace-commons-shell: unknown --onboarding-page {page}");
            }
            gtk::glib::ControlFlow::Break
        });
    }
    if drivers.open_preview {
        let app = app.clone();
        let search_term = drivers.search_term.clone();
        let preview_tab = drivers.preview_tab.clone();
        gtk::glib::timeout_add_seconds_local(3, move || {
            ui::preview::open_with_search(&app, 0, search_term.clone(), preview_tab.clone());
            gtk::glib::ControlFlow::Break
        });
    }

    if drivers.exit_after_realize {
        let application = application.clone();
        gtk::glib::timeout_add_seconds_local(drivers.realize_seconds, move || {
            println!("trace-commons-shell: realized, quitting (--exit-after-realize)");
            application.quit();
            gtk::glib::ControlFlow::Break
        });
    }
}
