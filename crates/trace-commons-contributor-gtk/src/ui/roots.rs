//! The first run before the daemon runs: Welcome and Tools.
//!
//! It runs BEFORE the daemon starts, and therefore before any IPC exists.
//! Until this window existed the Linux shell answered an undeclared-roots
//! refusal by printing a label and calling `std::process::exit(1)`: the
//! process simply vanished, with nothing on screen saying why.
//!
//! Welcome and Tools are the Mac's (`WelcomeScreen`, `ToolsScreen`), drawn
//! by the same code the first-run coordinator uses (`first_run_screens`,
//! `first_run_tools`). Continue writes Tools' declaration -- what the
//! planner's `Commit::Roots` would start the daemon with -- to the settings
//! file, and the caller starts the daemon. The coordinator then takes over
//! at Private AI with these answers ([`take_first_run_handoff`]).
//!
//! ## Discovery is not consent
//!
//! [`trace_commons_contributor::source::discovery`] finds the conventional
//! stores; nothing here starts selected. Each found tool has one check that
//! starts unticked, and an unticked tool is declared `off`, which has no
//! fallback. A tool not found is not asked; a missing Claude Code or Codex
//! is declared `off` too, so it is never read unasked once installed.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use trace_commons_contributor::daemon::settings::SourceDeclaration;
use trace_commons_contributor::source::discovery::{self, SourceCandidate};

use trace_commons_contributor::managed::copy::host_folder;

use crate::copy;
use crate::first_run::tools::{Found, ToolAnswer, Tools};

#[cfg(test)]
#[path = "../../tests/support/roots_submission_tests.rs"]
mod submission_tests;

thread_local! {
    static FIRST_RUN_HANDOFF: RefCell<Option<Tools>> = const { RefCell::new(None) };
}

/// A successful production Tools declaration carries first run into Private
/// AI after startup, with Tools' answers. Reading consumes the handoff;
/// injected display fixtures do not set it, and a failed declaration cannot
/// set it.
pub(super) fn take_first_run_handoff() -> Option<Tools> {
    FIRST_RUN_HANDOFF.with(|handoff| handoff.borrow_mut().take())
}

/// Show Welcome and Tools before the daemon is started.
pub fn present<F>(application: &adw::Application, dir: PathBuf, on_declared: F)
where
    F: Fn() + 'static,
{
    present_mode(
        application,
        dir,
        discovery::probe_this_machine(),
        move |tools| {
            FIRST_RUN_HANDOFF.with(|handoff| *handoff.borrow_mut() = Some(tools));
            on_declared();
        },
        true,
    );
}

/// The injected-candidate test entry: Tools only, no Welcome, no handoff.
#[cfg(all(test, target_os = "linux"))]
fn present_with<F>(
    application: &adw::Application,
    dir: PathBuf,
    candidates: Vec<SourceCandidate>,
    on_declared: F,
) where
    F: Fn() + 'static,
{
    present_mode(application, dir, candidates, move |_| on_declared(), false);
}

/// Tools' answers as the settings-file declaration `declare_sources` takes.
pub(super) fn answers(tools: &Tools) -> Option<Vec<(String, SourceDeclaration)>> {
    tools.session_roots()?;
    Some(
        tools
            .declarations()
            .into_iter()
            .filter_map(|(source, answer)| {
                let declaration = match answer {
                    ToolAnswer::Off => SourceDeclaration::Off,
                    ToolAnswer::Watch(path) => SourceDeclaration::Watch { path: path.into() },
                    // Only a daemon already running holds a folder.
                    ToolAnswer::Held | ToolAnswer::Undecided => return None,
                };
                Some((source, declaration))
            })
            .collect(),
    )
}

fn present_mode<F>(
    application: &adw::Application,
    dir: PathBuf,
    candidates: Vec<SourceCandidate>,
    on_declared: F,
    welcome: bool,
) where
    F: Fn(Tools) + 'static,
{
    // The FIRST window the shell ever opens -- it runs before the daemon, so
    // nothing else has installed the stylesheets yet.
    super::style::install();
    super::community_brand::install();

    let window = adw::ApplicationWindow::builder()
        .application(application)
        .default_width(680)
        .default_height(720)
        .build();
    super::first_run::StageScheme::hold(&window);
    let words = trace_commons_contributor::first_run_copy::first_run_copy();

    let mut discovered = Tools::default();
    discovered.record_discovery(candidates.iter().map(Found::from).collect());
    let tools = Rc::new(RefCell::new(discovered));
    let pending = Rc::new(std::cell::Cell::new(false));
    let screen: Rc<RefCell<Option<Rc<super::first_run_tools::ToolsScreen>>>> =
        Rc::new(RefCell::new(None));
    let on_declared = Rc::new(on_declared);
    let built = super::first_run_tools::build(tools.clone(), window.upcast_ref(), {
        let screen = screen.clone();
        let tools = tools.clone();
        let window = window.downgrade();
        move || {
            if pending.replace(true) {
                return;
            }
            let Some(answers) = answers(&tools.borrow()) else {
                pending.set(false);
                return;
            };
            let (tx, rx) = async_channel::bounded(1);
            let dir = dir.clone();
            std::thread::spawn(move || {
                let answers: Vec<_> = answers
                    .iter()
                    .map(|(source, declaration)| (source.as_str(), declaration.clone()))
                    .collect();
                let _ = tx.send_blocking(crate::backend::declare_sources(&dir, &answers));
            });
            let pending = pending.clone();
            let screen = screen.clone();
            let tools = tools.clone();
            let window = window.clone();
            let on_declared = on_declared.clone();
            gtk::glib::spawn_future_local(async move {
                let result = rx.recv().await;
                pending.set(false);
                let ok = matches!(result, Ok(Ok(())));
                if let Some(screen) = screen.borrow().as_ref() {
                    // Fixed copy keeps paths and OS errors out of the UI.
                    screen.finish((!ok).then_some(copy::ROOTS_FAILED));
                }
                if ok {
                    // Held until the coordinator holds it, so the person's
                    // scheme does not flash between the two windows.
                    super::stage::StageScheme::begin_handoff(std::time::Duration::from_secs(30));
                    if let Some(window) = window.upgrade() {
                        window.close();
                    }
                    on_declared(tools.borrow().clone());
                }
            });
        }
    });
    *screen.borrow_mut() = Some(built.clone());

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("tc-root");
    let heading = adw::WindowTitle::new(copy::APP_NAME, "");
    content.append(&adw::HeaderBar::builder().title_widget(&heading).build());
    let tools_page = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&super::first_run_screens::column(&built.root))
        .build();
    if welcome {
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        // Back at the bar's left, as on the Mac (`FirstRunFrame.actionBar`).
        let back = gtk::Button::with_label(words.frame.back);
        back.set_halign(gtk::Align::Start);
        back.set_margin_start(24);
        back.set_margin_bottom(16);
        back.set_visible(false);
        let intro = super::first_run_screens::welcome(
            {
                let stack = stack.clone();
                let back = back.clone();
                let heading = heading.clone();
                move || {
                    stack.set_visible_child_name("tools");
                    heading.set_subtitle(words.frame.step_tools);
                    back.set_visible(true);
                }
            },
            {
                let stack = stack.clone();
                move || stack.set_visible_child_name("agent")
            },
        );
        let agent = super::first_run_screens::agent_configuration({
            let stack = stack.clone();
            move || stack.set_visible_child_name("welcome")
        });
        stack.add_named(&super::first_run_screens::column(&intro), Some("welcome"));
        stack.add_named(&super::first_run_screens::column(&agent), Some("agent"));
        stack.add_named(&tools_page, Some("tools"));
        back.connect_clicked({
            let stack = stack.clone();
            let heading = heading.clone();
            move |back| {
                stack.set_visible_child_name("welcome");
                heading.set_subtitle("");
                back.set_visible(false);
            }
        });
        stack.set_visible_child_name("welcome");
        content.append(&stack);
        content.append(&back);
    } else {
        content.append(&tools_page);
    }
    window.set_content(Some(&content));
    window.present();
}

/// The Tools screen on its own, for a window attached to a daemon with no
/// session folders declared (the background service after a switch-over,
/// owner 2026-11-17). The same screen and words first run shows at its
/// Tools step (`first_run_tools::build`); Continue sends the declaration to
/// the attached daemon ([`declare_to_daemon`]), and the window closes once
/// the daemon confirms it. The first-run stage's dark scheme is not held
/// here: this is a dialog over the main window, not the first run.
pub(super) fn present_connected(app: &Rc<super::App>) -> adw::Window {
    super::style::install();
    super::community_brand::install();
    let words = trace_commons_contributor::first_run_copy::first_run_copy();
    let window = adw::Window::builder()
        .transient_for(&app.window)
        .modal(true)
        .default_width(680)
        .default_height(720)
        .build();
    let tools = Rc::new(RefCell::new(Tools::default()));
    let pending = Rc::new(std::cell::Cell::new(false));
    let screen: Rc<RefCell<Option<Rc<super::first_run_tools::ToolsScreen>>>> =
        Rc::new(RefCell::new(None));
    let built = super::first_run_tools::build(tools.clone(), window.upcast_ref(), {
        let screen = screen.clone();
        let weak_window = window.downgrade();
        let weak_app = Rc::downgrade(app);
        move || {
            let Some(app) = weak_app.upgrade() else {
                return;
            };
            if pending.replace(true) {
                return;
            }
            let Some(roots) = tools.borrow().session_roots() else {
                pending.set(false);
                return;
            };
            // Every tool declared is one the daemon already watches:
            // nothing to send (`set_settings` refuses an empty object).
            if roots.is_empty() {
                pending.set(false);
                app.refresh();
                if let Some(window) = weak_window.upgrade() {
                    window.close();
                }
                return;
            }
            let pending = pending.clone();
            let screen = screen.clone();
            let weak_window = weak_window.clone();
            declare_to_daemon(&app, serde_json::json!(roots), move |app, answer, hint| {
                pending.set(false);
                let confirmed = answer.is_some_and(|(sent, result)| {
                    result.is_ok_and(|settings| declarations_confirmed(&sent, &settings))
                });
                let Some(window) = weak_window.upgrade() else {
                    return;
                };
                if let Some(screen) = screen.borrow().as_ref() {
                    screen.finish(
                        (!confirmed).then_some(
                            trace_commons_contributor::first_run_copy::first_run_copy()
                                .folders
                                .settings_failed,
                        ),
                    );
                    screen.show_host_hint(hint.as_ref());
                }
                if confirmed {
                    app.refresh();
                    window.close();
                }
            });
        }
    });
    *screen.borrow_mut() = Some(built.clone());
    follow_held_watches(app, window.upcast_ref(), &built);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("tc-root");
    content.append(
        &adw::HeaderBar::builder()
            .title_widget(&adw::WindowTitle::new(
                copy::APP_NAME,
                words.frame.step_tools,
            ))
            .build(),
    );
    content.append(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&super::first_run_screens::column(&built.root))
            .build(),
    );
    window.set_content(Some(&content));
    window.present();
    window
}

/// Show on `screen` the tools the attached daemon already watches: read
/// now, and again each time `window` comes to the front, as the Mac's
/// Tools reads again when the app becomes active. A folder declared from a
/// terminal (the old-portal command) then shows here without a relaunch.
pub(super) fn follow_held_watches(
    app: &Rc<super::App>,
    window: &gtk::Window,
    screen: &Rc<super::first_run_tools::ToolsScreen>,
) {
    let read = {
        let weak_app = Rc::downgrade(app);
        let weak_screen = Rc::downgrade(screen);
        move || {
            let Some(app) = weak_app.upgrade() else {
                return;
            };
            let weak_screen = weak_screen.clone();
            app.call("get_settings", serde_json::json!({}), move |_, result| {
                if let (Some(screen), Ok(settings)) = (weak_screen.upgrade(), result) {
                    screen.record_held(crate::first_run::tools::held_watches(&settings));
                }
            });
        }
    };
    read();
    window.connect_is_active_notify(move |window| {
        if window.is_active() {
            read();
        }
    });
}

/// What a folder declaration became on its way to the attached daemon, and
/// the daemon's answer; `None` when nothing was sent.
pub(super) type Declared = Option<(serde_json::Value, Result<serde_json::Value, String>)>;

/// Send a folder declaration (`set_settings`, as `Tools::session_roots`
/// builds it) to the attached daemon. Every folder declaration a running
/// daemon is sent goes through here: the Roots commit of the first run and
/// the Tools screen above, whether the folder was picked in the chooser or
/// dropped.
///
/// Attached to the host daemon from the sandbox, a folder the chooser or a
/// drop handed over as a document-portal path is sent as the host folder it
/// stands for, as Launch does, or not at all: `done` then gets `None`. The
/// daemon in the sandbox opens the portal path itself. A window that moved
/// off the host link while the portal answered sends nothing either.
///
/// Nothing is sent while a switch-over is under way
/// (`Worker::declaration_target`): a reopen queued ahead of the
/// declaration replaces the backend first, so a mapping decided for the
/// backend the window last recorded could reach the other one -- a portal
/// path to the host daemon, or a host path to the sandbox's.
pub(super) fn declare_to_daemon(
    app: &Rc<super::App>,
    params: serde_json::Value,
    done: impl FnOnce(&Rc<super::App>, Declared, Option<HostFolderHint>) + 'static,
) {
    let Some(on_host) = app.worker.declaration_target() else {
        done(app, None, None);
        return;
    };
    if !crate::document_portal::declarations_need_mapping(&params, on_host) {
        send_declaration(app, params, move |app, declared| done(app, declared, None));
        return;
    }
    // One bounded D-Bus round trip per folder, off the GTK thread. When
    // the mapping fails for one folder a terminal can declare, one more:
    // whether the portal is too old to map anything.
    let weak = Rc::downgrade(app);
    super::service_dialog::off_thread(
        move || {
            let refused = params.clone();
            crate::document_portal::host_declarations(params).ok_or_else(|| {
                old_portal_hint(
                    &refused,
                    crate::document_portal::doc_root().as_deref(),
                    crate::document_portal::portal_version,
                )
            })
        },
        move |mapped| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            match mapped {
                Ok(params) if app.worker.declaration_target() == Some(true) => {
                    send_declaration(&app, params, move |app, declared| done(app, declared, None))
                }
                Ok(_) => done(&app, None, None),
                Err(hint) => done(&app, None, hint),
            }
        },
    );
}

/// What the Tools screen adds under its failure line when the folder was
/// refused because this system's document portal cannot say where it is on
/// the host: a line, and the command that declares it from a terminal in
/// that folder, which is only ever copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HostFolderHint {
    pub line: &'static str,
    pub command: String,
    /// The tool whose folder was refused: once the daemon newly holds it,
    /// the command was run and Tools hides the hint.
    pub tool: &'static str,
}

/// [`HostFolderHint`] for the tool whose settings key is `settings_key`:
/// the owner's line (`host_folder::add_on_host_line`) and the command;
/// `None` for a tool the command cannot declare, and the failure line then
/// shows alone.
pub(super) fn host_folder_hint(settings_key: &str) -> Option<HostFolderHint> {
    Some(HostFolderHint {
        line: host_folder::add_on_host_line()?,
        command: trace_commons_contributor::daemon::service::host_folder_command(settings_key)?,
        tool: crate::first_run::tools::tool_for_settings_key(settings_key)?,
    })
}

/// What a refused `params` adds under the failure line: the hint for its
/// one portal folder when the portal mounted at `doc_root` is too old to
/// map any folder (`document_portal::lacks_host_paths`); `None` for every
/// other refusal. `version` is read last, only when there is a hint to give.
fn old_portal_hint(
    params: &serde_json::Value,
    doc_root: Option<&std::path::Path>,
    version: impl FnOnce() -> Option<u32>,
) -> Option<HostFolderHint> {
    let key = crate::document_portal::sole_portal_declaration_in(params, doc_root)?;
    let hint = host_folder_hint(&key)?;
    crate::document_portal::lacks_host_paths(version()).then_some(hint)
}

fn send_declaration(
    app: &Rc<super::App>,
    params: serde_json::Value,
    done: impl FnOnce(&Rc<super::App>, Declared) + 'static,
) {
    let sent = params.clone();
    app.call("set_settings", params, move |app, result| {
        done(app, Some((sent, result)))
    });
}

/// Whether the daemon's settings answer took every declaration sent: the
/// answer reports each one's mode, never the declaration itself
/// (`first_run::roots_acknowledged`).
fn declarations_confirmed(expected: &serde_json::Value, settings: &serde_json::Value) -> bool {
    expected
        .as_object()
        .is_some_and(|fields| crate::first_run::roots_acknowledged(fields.iter(), settings))
}

/// A tool's name, as the Mac's `SourceKind.displayName` gives it: the only
/// thing naming the store a contributor is agreeing to.
///
/// Every known source is matched explicitly. The arm this replaced sent
/// anything unrecognised to the Claude Code title, so when a third source
/// appeared the screen offered two rows both called "Claude Code traces",
/// one of them pointing at `~/.gemini/tmp`. The unknown arm says so rather
/// than borrowing a name.
pub(super) fn source_title(source: &str) -> &'static str {
    use trace_commons_contributor::source::{
        SOURCE_CLAUDE_CODE, SOURCE_CLINE, SOURCE_CODEX, SOURCE_GEMINI_CLI, SOURCE_OPENCODE,
    };
    use trace_commons_contributor::source_copy::SourceTool;
    let tool = match source {
        SOURCE_CLAUDE_CODE => SourceTool::Claude,
        SOURCE_CODEX => SourceTool::Codex,
        SOURCE_GEMINI_CLI => SourceTool::Gemini,
        SOURCE_CLINE => SourceTool::Cline,
        SOURCE_OPENCODE => SourceTool::OpenCode,
        _ => return copy::ROOTS_UNKNOWN_SOURCE,
    };
    // The core's one name for each tool (`source_copy`).
    tool.name()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder refused because the document portal is too old to map it
    /// (interface version 4, GNOME 46) shows the owner's approved line
    /// (2026-11-17) and the fixed, copy-only command; nothing else does.
    #[test]
    fn the_host_folder_line_shows_with_its_command_only_on_an_old_portal() {
        let root = std::path::Path::new("/run/user/1000/doc");
        let portal = |id: &str| format!("/run/user/1000/doc/{id}/projects");
        let one = |key: &str| serde_json::json!({ key: {"mode": "watch", "path": portal("a1")} });
        let hint = |params: &serde_json::Value, version: Option<u32>| {
            old_portal_hint(params, Some(root), move || version)
        };
        assert_eq!(
            hint(&one("claude_source"), Some(4)),
            Some(HostFolderHint {
                line: "To add this folder anyway, open a terminal in it and run:",
                command: format!(
                    "{} daemon settings --set \"claude_root=$PWD\"",
                    trace_commons_contributor::channel::HOST_CLI
                ),
                tool: "claude-code",
            })
        );
        assert_eq!(
            hint(&one("codex_source"), Some(4)).map(|h| h.command),
            trace_commons_contributor::daemon::service::host_folder_command("codex_source")
        );
        // A portal new enough to map (a mapping that failed some other
        // way), or one whose version is unknown: the failure line alone.
        assert_eq!(hint(&one("claude_source"), Some(5)), None);
        assert_eq!(hint(&one("claude_source"), Some(6)), None);
        assert_eq!(hint(&one("claude_source"), None), None);
        // A tool the command cannot declare.
        assert_eq!(hint(&one("gemini_source"), Some(4)), None);
        // Two portal folders: one terminal declares one.
        let two = serde_json::json!({
            "claude_source": {"mode": "watch", "path": portal("a1")},
            "codex_source": {"mode": "watch", "path": portal("b2")},
        });
        assert_eq!(hint(&two, Some(4)), None);
        // A folder outside the portal, or no portal mount.
        let host = serde_json::json!({"claude_source": {"mode": "watch", "path": "/home/u/x"}});
        assert_eq!(hint(&host, Some(4)), None);
        assert_eq!(
            old_portal_hint(&one("claude_source"), None, || Some(4)),
            None
        );
    }

    fn tools(watch_claude: bool) -> Tools {
        let mut tools = Tools::default();
        tools.record_discovery(vec![
            Found {
                source: "claude-code".into(),
                path: "/home/x/.claude/projects".into(),
                exists: true,
                session_count: 2,
                total_bytes: None,
            },
            Found {
                source: "codex".into(),
                path: "/home/x/.codex/sessions".into(),
                exists: false,
                session_count: 0,
                total_bytes: None,
            },
        ]);
        if watch_claude {
            let claude = tools.listed()[0].clone();
            tools.set_watched(&claude, true);
        }
        tools
    }

    #[test]
    fn startup_handoff_is_consumed_once() {
        FIRST_RUN_HANDOFF.with(|handoff| *handoff.borrow_mut() = Some(tools(true)));
        assert!(take_first_run_handoff().is_some());
        assert!(take_first_run_handoff().is_none());
    }

    #[test]
    fn the_file_declaration_is_what_the_roots_commit_would_send() {
        let answers = answers(&tools(true)).unwrap();
        assert!(answers.contains(&(
            "claude-code".into(),
            SourceDeclaration::Watch {
                path: "/home/x/.claude/projects".into()
            }
        )));
        // Codex is missing and unanswered: declared off, never read unasked.
        assert!(answers.contains(&("codex".into(), SourceDeclaration::Off)));
        assert!(super::answers(&Tools::default()).is_none());
    }

    #[test]
    fn connected_tools_requires_acknowledgement_of_every_declaration() {
        use serde_json::json;
        let expected = json!({"claude_source": {"mode":"off"}, "codex_source": {"mode":"watch", "path":"/chosen/codex"}});
        // `set_settings` answers without the declarations, with their modes.
        let answer = json!({"claude_source_mode": "off", "codex_source_mode": "watch"});
        assert!(declarations_confirmed(&expected, &answer));
        assert!(!declarations_confirmed(
            &expected,
            &json!({"claude_source_mode": "off", "codex_source_mode": "unset"})
        ));
        assert!(!declarations_confirmed(&expected, &json!({"ok":true})));
        assert!(!declarations_confirmed(&json!({}), &json!({})));
    }

    #[test]
    fn each_source_gets_its_own_title() {
        // The Mac's `SourceKind.displayName`, in the core's words.
        assert_eq!(source_title("codex"), "Codex");
        assert_eq!(source_title("claude-code"), "Claude Code");
        assert_eq!(source_title("gemini-cli"), "Gemini CLI");
        assert_eq!(source_title("cline"), "Cline");
        assert_eq!(source_title("opencode"), "OpenCode");
    }

    /// Titles must be distinct, because this label is the only thing naming
    /// the store a contributor is agreeing to.
    #[test]
    fn no_two_sources_share_a_title() {
        let titles = [
            source_title("claude-code"),
            source_title("codex"),
            source_title("gemini-cli"),
            source_title("cline"),
            source_title("opencode"),
        ];
        assert!(
            !titles.contains(&copy::ROOTS_UNKNOWN_SOURCE),
            "a registered source fell through to the unknown title: {titles:?}"
        );
        for (i, a) in titles.iter().enumerate() {
            for b in titles.iter().skip(i + 1) {
                assert_ne!(a, b, "two sources share the title {a:?}");
            }
        }
    }
}
