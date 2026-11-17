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
                    ToolAnswer::Undecided => return None,
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
        .child(&built.root)
        .build();
    if welcome {
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        let back = gtk::Button::with_label(words.frame.back);
        back.set_halign(gtk::Align::End);
        back.set_margin_end(24);
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
        stack.add_named(&intro, Some("welcome"));
        stack.add_named(&agent, Some("agent"));
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
