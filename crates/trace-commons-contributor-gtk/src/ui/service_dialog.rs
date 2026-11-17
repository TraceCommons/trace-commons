//! The background-service dialogs: not installed, installed but not
//! running, too old. Flow and words approved by the owner 2026-11-17; the
//! words are the core's (`managed::copy::background_service`), the commands
//! the core's fixed strings (`daemon::service`). See `crate::service` for
//! when a dialog appears.
//!
//! The install command is only ever copied to the clipboard. "Start it"
//! asks the systemd user manager to restart the fixed unit (which starts it
//! at once, even while it waits out `RestartSec`); nothing else on the host
//! is touched.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use trace_commons_contributor::daemon::service::{
    SERVICE_INSTALL_COMMAND, SERVICE_INSTALL_GUIDE_URL, SERVICE_STATUS_COMMAND, ServiceState,
};
use trace_commons_contributor::managed::copy::background_service as words;

use crate::service::{Dialog, Tries, WATCH_INTERVAL, WatchStep, watch_step};

thread_local! {
    /// "Not now" was pressed this run: first setup and Launch stay quiet.
    static DISMISSED: Cell<bool> = const { Cell::new(false) };
    /// The dialog on screen, so a second trigger raises it instead of
    /// stacking another.
    static OPEN: RefCell<Option<gtk::glib::WeakRef<adw::Window>>> = const { RefCell::new(None) };
}

/// Whether "Not now" was pressed this run.
pub fn dismissed() -> bool {
    DISMISSED.with(Cell::get)
}

/// For mounted tests: set or clear "Not now" for this thread.
#[cfg(all(test, target_os = "linux"))]
pub(crate) fn set_dismissed(dismissed: bool) {
    DISMISSED.with(|d| d.set(dismissed));
}

/// How a dialog ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The service runs; the dialog closed showing nothing else.
    Running,
    /// "Not now", or the window closed.
    NotNow,
}

/// Where a dialog gets its answers. First setup (no daemon yet) and an open
/// window (a worker with a backend) answer differently.
pub trait ServiceHost {
    /// Classify the service now, off the GTK thread; `done` runs on it.
    fn state(&self, done: Box<dyn FnOnce(ServiceState)>);
    /// Stop a daemon this window runs, ask systemd to restart the unit, and
    /// use the service once it answers (bounded); `done` runs on the GTK
    /// thread either way.
    fn switch_over(&self, done: Box<dyn FnOnce()>);
}

/// The widgets of one dialog, before any behaviour is wired.
pub struct Parts {
    pub window: adw::Window,
    pub title: gtk::Label,
    pub body: gtk::Label,
    /// The install command, for the install and update dialogs.
    pub command: Option<gtk::Label>,
    /// "Copy command", or "Start it".
    pub primary: gtk::Button,
    /// "Open install guide", in the install dialog only.
    pub guide: Option<gtk::Button>,
    pub not_now: gtk::Button,
    /// "Waiting for the install to finish…", shown once the command is
    /// copied or the guide opened.
    pub waiting: gtk::Label,
    /// "It didn't start…" and the status command, after a failed start.
    pub failed: gtk::Box,
}

fn command_label(command: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(command)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::Char)
        .selectable(true)
        .xalign(0.0)
        .css_classes(["monospace"])
        .build()
}

/// Build one dialog's widgets with the approved words.
pub fn build(dialog: Dialog) -> Parts {
    let (title, body) = match dialog {
        Dialog::Install => (words::NOT_INSTALLED_TITLE, words::NOT_INSTALLED_BODY),
        Dialog::Start => (words::NOT_RUNNING_TITLE, words::NOT_RUNNING_BODY),
        Dialog::Update => (words::TOO_OLD_TITLE, words::TOO_OLD_BODY),
    };
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);
    let title_label = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["title-2"])
        .build();
    content.append(&title_label);
    let body_label = super::style::body(body);
    content.append(&body_label);
    let command = (dialog != Dialog::Start).then(|| {
        let label = command_label(SERVICE_INSTALL_COMMAND);
        content.append(&label);
        label
    });
    let failed = gtk::Box::new(gtk::Orientation::Vertical, 6);
    failed.append(&super::style::body(words::START_FAILED));
    failed.append(&command_label(SERVICE_STATUS_COMMAND));
    failed.set_visible(false);
    content.append(&failed);
    let waiting = super::style::meta(words::WAITING);
    waiting.set_visible(false);
    content.append(&waiting);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let not_now = gtk::Button::with_label(words::NOT_NOW);
    let guide =
        (dialog == Dialog::Install).then(|| gtk::Button::with_label(words::OPEN_INSTALL_GUIDE));
    let primary = gtk::Button::with_label(match dialog {
        Dialog::Start => words::START_IT,
        Dialog::Install | Dialog::Update => words::COPY_COMMAND,
    });
    primary.add_css_class("suggested-action");
    buttons.append(&not_now);
    if let Some(guide) = &guide {
        buttons.append(guide);
    }
    buttons.append(&primary);
    content.append(&buttons);
    let window = adw::Window::builder()
        .title(title)
        .modal(true)
        .default_width(520)
        .content(&content)
        .build();
    Parts {
        window,
        title: title_label,
        body: body_label,
        command,
        primary,
        guide,
        not_now,
        waiting,
        failed,
    }
}

/// Show `dialog` over `parent` and run it until it ends. A dialog already
/// on screen is raised instead, and `on_end` is not called for this one.
pub fn present(
    parent: Option<&gtk::Window>,
    application: Option<&adw::Application>,
    dialog: Dialog,
    host: Rc<dyn ServiceHost>,
    on_end: impl Fn(Ended) + 'static,
) {
    if let Some(open) = OPEN.with(|open| open.borrow().as_ref().and_then(|weak| weak.upgrade())) {
        open.present();
        return;
    }
    let parts = build(dialog);
    let window = parts.window.clone();
    window.set_transient_for(parent);
    if let Some(application) = application {
        // Before the main window exists (first setup), the application must
        // count this window, or it would quit with none open.
        window.set_application(Some(application));
    }
    OPEN.with(|open| *open.borrow_mut() = Some(window.downgrade()));

    // Exactly one end, however the dialog goes away.
    let ended: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let on_end: Rc<dyn Fn(Ended)> = Rc::new(on_end);
    let finish = {
        let ended = Rc::clone(&ended);
        let window = window.downgrade();
        let on_end = Rc::clone(&on_end);
        Rc::new(move |how: Ended| {
            if ended.replace(true) {
                return;
            }
            if how == Ended::NotNow {
                DISMISSED.with(|d| d.set(true));
            }
            OPEN.with(|open| *open.borrow_mut() = None);
            // What comes next opens first, so the application never has no
            // window (first setup holds nothing else).
            on_end(how);
            if let Some(window) = window.upgrade() {
                window.close();
            }
        })
    };
    {
        let finish = Rc::clone(&finish);
        window.connect_close_request(move |_| {
            finish(Ended::NotNow);
            gtk::glib::Propagation::Proceed
        });
    }
    {
        let finish = Rc::clone(&finish);
        parts
            .not_now
            .connect_clicked(move |_| finish(Ended::NotNow));
    }
    if let Some(guide) = &parts.guide {
        let waiting = parts.waiting.clone();
        guide.connect_clicked(move |_| {
            let _ = gtk::gio::AppInfo::launch_default_for_uri(
                SERVICE_INSTALL_GUIDE_URL,
                None::<&gtk::gio::AppLaunchContext>,
            );
            waiting.set_visible(true);
        });
    }

    let busy = Rc::new(Cell::new(false));
    // The automatic switch-overs made, and when the last one ended: one
    // that did not bring the service up is tried again after a pause.
    let tries = Rc::new(Cell::new(0_u32));
    let last_ended: Rc<Cell<Option<std::time::Instant>>> = Rc::new(Cell::new(None));
    match dialog {
        Dialog::Install | Dialog::Update => {
            let waiting = parts.waiting.clone();
            let show_waiting = dialog == Dialog::Install;
            parts.primary.connect_clicked(move |button| {
                button.clipboard().set_text(SERVICE_INSTALL_COMMAND);
                waiting.set_visible(show_waiting);
            });
        }
        Dialog::Start => {
            let host = Rc::clone(&host);
            let failed = parts.failed.clone();
            let busy = Rc::clone(&busy);
            let finish = Rc::clone(&finish);
            parts.primary.connect_clicked(move |button| {
                if busy.replace(true) {
                    return;
                }
                button.set_sensitive(false);
                failed.set_visible(false);
                let host_after = Rc::clone(&host);
                let failed = failed.clone();
                let button = button.clone();
                let busy = Rc::clone(&busy);
                let finish = Rc::clone(&finish);
                host.switch_over(Box::new(move || {
                    host_after.state(Box::new(move |state| {
                        busy.set(false);
                        button.set_sensitive(true);
                        if state == ServiceState::Running {
                            finish(Ended::Running);
                        } else {
                            failed.set_visible(true);
                        }
                    }));
                }));
            });
        }
    }

    // While open, look again: the service may appear (installed from a
    // terminal) or start by itself.
    {
        let ended = Rc::clone(&ended);
        let failed = parts.failed.clone();
        let waiting = parts.waiting.clone();
        let host = Rc::clone(&host);
        let finish = Rc::clone(&finish);
        gtk::glib::timeout_add_local(WATCH_INTERVAL, move || {
            if ended.get() {
                return gtk::glib::ControlFlow::Break;
            }
            if busy.replace(true) {
                return gtk::glib::ControlFlow::Continue;
            }
            let host_next = Rc::clone(&host);
            let busy = Rc::clone(&busy);
            let failed = failed.clone();
            let waiting = waiting.clone();
            let tries = Rc::clone(&tries);
            let last_ended = Rc::clone(&last_ended);
            let finish = Rc::clone(&finish);
            host.state(Box::new(move |state| {
                let made = Tries {
                    made: tries.get(),
                    since_last: last_ended.get().map(|at| at.elapsed()),
                };
                match watch_step(dialog, state, made) {
                    WatchStep::Close => {
                        busy.set(false);
                        finish(Ended::Running);
                    }
                    WatchStep::SwitchOver => {
                        tries.set(tries.get() + 1);
                        host_next.switch_over(Box::new(move || {
                            last_ended.set(Some(std::time::Instant::now()));
                            busy.set(false);
                        }));
                    }
                    step @ (WatchStep::Failed | WatchStep::Keep) => {
                        // Dialog B's line answers its own "Start it".
                        if dialog != Dialog::Start {
                            show_step(&failed, &waiting, step);
                        }
                        busy.set(false);
                    }
                }
            }));
            gtk::glib::ControlFlow::Continue
        });
    }
    window.present();
}

/// What a look shows in the not-installed and too-old dialogs: after the
/// last automatic switch-over failed, dialog B's failure line and status
/// command in place of the waiting line ("Waiting for the install to
/// finish" no longer holds once it has); otherwise not that line. Dialog B
/// shows its own after "Start it" and is left alone here.
fn show_step(failed: &gtk::Box, waiting: &gtk::Label, step: WatchStep) {
    let show = step == WatchStep::Failed;
    failed.set_visible(show);
    if show {
        waiting.set_visible(false);
    }
}

/// Run `work` off the GTK thread and hand its result to `done` on it.
pub fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T) + 'static,
) {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(work());
    });
    gtk::glib::spawn_future_local(async move {
        if let Ok(value) = rx.recv().await {
            done(value);
        }
    });
}

/// First setup: no daemon runs yet, so the facts are gathered with no
/// window daemon, and a switch-over is only systemd's `RestartUnit` and a
/// bounded wait for the unit to answer. `local` is the state directory's
/// socket, where a unit on the host answers when there is no host link.
pub struct FirstSetupHost {
    pub local: std::path::PathBuf,
}

impl ServiceHost for FirstSetupHost {
    fn state(&self, done: Box<dyn FnOnce(ServiceState)>) {
        let local = self.local.clone();
        off_thread(
            move || {
                trace_commons_contributor::daemon::service::classify(&crate::service::gather(
                    false,
                    Some(&local),
                ))
            },
            done,
        );
    }

    fn switch_over(&self, done: Box<dyn FnOnce()>) {
        let local = self.local.clone();
        off_thread(
            move || {
                let _ = crate::service::restart_unit();
                crate::service::wait_for_service(crate::service::SWITCH_WAIT, &local);
            },
            move |()| done(),
        );
    }
}

/// An open window: the facts come from its worker's backend, and a
/// switch-over replaces that backend (`worker::Job::Reopen`).
pub struct AppHost(pub std::rc::Weak<super::App>);

impl ServiceHost for AppHost {
    fn state(&self, done: Box<dyn FnOnce(ServiceState)>) {
        if let Some(app) = self.0.upgrade() {
            app.service_state(move |_, state| done(state));
        }
    }

    fn switch_over(&self, done: Box<dyn FnOnce()>) {
        if let Some(app) = self.0.upgrade() {
            app.reopen(true, move |_| done());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(root: &gtk::Widget, out: &mut Vec<String>) {
        if let Some(label) = root.downcast_ref::<gtk::Label>() {
            out.push(label.label().to_string());
        }
        if let Some(button) = root.downcast_ref::<gtk::Button>() {
            out.push(button.label().map(|l| l.to_string()).unwrap_or_default());
        }
        let mut child = root.first_child();
        while let Some(widget) = child {
            labels(&widget, out);
            child = widget.next_sibling();
        }
    }

    fn mounted(dialog: Dialog) -> (Parts, Vec<String>) {
        assert_eq!(
            std::env::consts::OS,
            "linux",
            "requires a Linux GTK display"
        );
        adw::init().expect("GTK display unavailable");
        let parts = build(dialog);
        let mut out = Vec::new();
        labels(parts.window.content().as_ref().unwrap(), &mut out);
        (parts, out)
    }

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_not_installed_dialog_has_the_approved_words_and_buttons() {
        let (parts, shown) = mounted(Dialog::Install);
        assert_eq!(parts.title.label(), words::NOT_INSTALLED_TITLE);
        assert_eq!(parts.body.label(), words::NOT_INSTALLED_BODY);
        assert_eq!(
            parts.command.as_ref().unwrap().label(),
            SERVICE_INSTALL_COMMAND
        );
        assert_eq!(parts.primary.label().unwrap(), words::COPY_COMMAND);
        assert_eq!(
            parts.guide.as_ref().unwrap().label().unwrap(),
            words::OPEN_INSTALL_GUIDE
        );
        assert_eq!(parts.not_now.label().unwrap(), words::NOT_NOW);
        assert_eq!(parts.waiting.label(), words::WAITING);
        assert!(!parts.waiting.is_visible());
        assert!(!parts.failed.is_visible());
        for word in [
            words::NOT_INSTALLED_TITLE,
            words::NOT_INSTALLED_BODY,
            SERVICE_INSTALL_COMMAND,
            words::COPY_COMMAND,
            words::OPEN_INSTALL_GUIDE,
            words::NOT_NOW,
        ] {
            assert!(shown.iter().any(|s| s == word), "{word}");
        }
        assert!(!shown.iter().any(|s| s == words::START_IT));
    }

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_not_running_dialog_has_start_it_and_the_status_command_on_failure() {
        let (parts, shown) = mounted(Dialog::Start);
        assert_eq!(parts.title.label(), words::NOT_RUNNING_TITLE);
        assert_eq!(parts.body.label(), words::NOT_RUNNING_BODY);
        assert!(parts.command.is_none());
        assert!(parts.guide.is_none());
        assert_eq!(parts.primary.label().unwrap(), words::START_IT);
        assert_eq!(parts.not_now.label().unwrap(), words::NOT_NOW);
        assert!(!parts.failed.is_visible());
        assert!(shown.iter().any(|s| s == words::START_FAILED));
        assert!(shown.iter().any(|s| s == SERVICE_STATUS_COMMAND));
        assert!(!shown.iter().any(|s| s == SERVICE_INSTALL_COMMAND));
    }

    /// Once its automatic switch-overs are spent, the not-installed dialog
    /// shows dialog B's failure line and status command in place of its
    /// waiting line, with no other words.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_not_installed_dialog_shows_the_status_command_after_its_last_try() {
        let (parts, shown) = mounted(Dialog::Install);
        assert!(shown.iter().any(|s| s == words::START_FAILED));
        assert!(shown.iter().any(|s| s == SERVICE_STATUS_COMMAND));
        parts.waiting.set_visible(true);
        show_step(&parts.failed, &parts.waiting, WatchStep::Keep);
        assert!(!parts.failed.is_visible());
        assert!(parts.waiting.is_visible());
        show_step(&parts.failed, &parts.waiting, WatchStep::Failed);
        assert!(parts.failed.is_visible());
        assert!(!parts.waiting.is_visible());
        let mut failed = Vec::new();
        labels(parts.failed.upcast_ref(), &mut failed);
        assert_eq!(
            failed,
            vec![
                words::START_FAILED.to_string(),
                SERVICE_STATUS_COMMAND.to_string()
            ]
        );
        // The service gone again: the line goes with it.
        show_step(&parts.failed, &parts.waiting, WatchStep::Keep);
        assert!(!parts.failed.is_visible());
    }

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_too_old_dialog_has_the_install_command_and_copy() {
        let (parts, shown) = mounted(Dialog::Update);
        assert_eq!(parts.title.label(), words::TOO_OLD_TITLE);
        assert_eq!(parts.body.label(), words::TOO_OLD_BODY);
        assert_eq!(
            parts.command.as_ref().unwrap().label(),
            SERVICE_INSTALL_COMMAND
        );
        assert_eq!(parts.primary.label().unwrap(), words::COPY_COMMAND);
        assert!(parts.guide.is_none());
        assert_eq!(parts.not_now.label().unwrap(), words::NOT_NOW);
        assert!(!shown.iter().any(|s| s == words::OPEN_INSTALL_GUIDE));
    }

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_copy_command_puts_the_fixed_command_on_the_clipboard() {
        let (parts, _) = mounted(Dialog::Install);
        // Wire the dialog's behaviour onto these parts' twin.
        struct Never;
        impl ServiceHost for Never {
            fn state(&self, _: Box<dyn FnOnce(ServiceState)>) {}
            fn switch_over(&self, _: Box<dyn FnOnce()>) {}
        }
        drop(parts);
        present(None, None, Dialog::Install, Rc::new(Never), |_| {});
        let window = OPEN
            .with(|open| open.borrow().as_ref().and_then(|w| w.upgrade()))
            .expect("open");
        let mut buttons = Vec::new();
        fn find(root: &gtk::Widget, out: &mut Vec<gtk::Button>) {
            if let Some(button) = root.downcast_ref::<gtk::Button>() {
                out.push(button.clone());
            }
            let mut child = root.first_child();
            while let Some(widget) = child {
                find(&widget, out);
                child = widget.next_sibling();
            }
        }
        find(window.content().as_ref().unwrap(), &mut buttons);
        let copy = buttons
            .iter()
            .find(|b| b.label().as_deref() == Some(words::COPY_COMMAND))
            .unwrap();
        copy.emit_clicked();
        let context = gtk::glib::MainContext::default();
        let text = Rc::new(RefCell::new(None));
        let read = Rc::clone(&text);
        copy.clipboard()
            .read_text_async(None::<&gtk::gio::Cancellable>, move |r| {
                *read.borrow_mut() = Some(r.ok().flatten().map(|s| s.to_string()));
            });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while text.borrow().is_none() && std::time::Instant::now() < deadline {
            context.iteration(true);
        }
        assert_eq!(
            text.borrow().clone().flatten().as_deref(),
            Some(SERVICE_INSTALL_COMMAND)
        );
        // "Not now" ends it and quiets the automatic triggers.
        let not_now = buttons
            .iter()
            .find(|b| b.label().as_deref() == Some(words::NOT_NOW))
            .unwrap();
        not_now.emit_clicked();
        assert!(dismissed());
        assert!(OPEN.with(|open| open.borrow().is_none()));
    }
}
