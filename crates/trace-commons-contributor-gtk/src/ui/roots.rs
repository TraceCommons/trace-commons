//! The roots screen: which session folders this shell may watch.
//!
//! It runs BEFORE the daemon starts, and therefore before any IPC exists.
//! The production Welcome/Tools handoff resumes onboarding after startup.
//! The connected Tools variant uses IPC only once the daemon is available. Until this window existed the Linux shell answered an undeclared-roots
//! refusal by printing a label and calling `std::process::exit(1)`: the
//! process simply vanished, with nothing on screen saying why.
//!
//! ## Discovery is not consent
//!
//! [`trace_commons_contributor::source::discovery`] finds the conventional
//! stores and describes them -- where, whether they exist, how many sessions,
//! how recently touched -- so the question is "watch these 946 Claude Code
//! sessions?" rather than an empty text field. Nothing here starts selected.
//! A pre-ticked box plus a habitual Continue is the shape of consent people
//! click through, and it would recover most of the fail-open behaviour while
//! looking like it had asked.
//!
//! ## Why "I don't use this" is a button and not a blank
//!
//! Leaving a source unanswered does not mean "skip it". An unanswered source
//! is `None`, which the daemon reads as "watch the conventional location" --
//! so the answer a privacy-conscious contributor is most likely to give was,
//! before [`SourceDeclaration`], the one answer that scanned their work. The
//! button writes [`SourceDeclaration::Off`], which has no fallback.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use trace_commons_contributor::daemon::settings::SourceDeclaration;
use trace_commons_contributor::source::discovery::{self, SourceCandidate};

use super::style::{self, space};
use crate::copy;

#[cfg(test)]
#[path = "../../tests/support/roots_submission_tests.rs"]
mod submission_tests;

/// One agent's row: the discovered evidence, and the two answers.
struct Choice {
    group: gtk::Box,
    /// The folder [`SourceDeclaration::Watch`] would name. Starts at the
    /// discovered path and changes only if the contributor picks another.
    ///
    /// Shared with the folder chooser's response handler rather than read
    /// back off the visible label: a label is display text, and rebuilding a
    /// path from it would go wrong on the first folder name that does not
    /// survive the round trip.
    path: Rc<RefCell<PathBuf>>,
    watch: gtk::CheckButton,
    off: gtk::CheckButton,
    /// `claude-code` or `codex`. Rows are looked up by this rather than by
    /// position: indexing would put a contributor's Codex answer on their
    /// Claude store the day discovery returns them in another order.
    source: String,
}

impl Choice {
    /// What this row currently says, or `None` while it says nothing.
    ///
    /// `None` is the unanswered state and is why Continue stays insensitive;
    /// it is never written to the settings file, because there it would mean
    /// "never asked" and fall back to the real location.
    fn declaration(&self) -> Option<SourceDeclaration> {
        if self.off.is_active() {
            return Some(SourceDeclaration::Off);
        }
        if self.watch.is_active() {
            return Some(SourceDeclaration::Watch {
                path: self.path.borrow().clone(),
            });
        }
        None
    }
}

thread_local! {
    static FIRST_RUN_HANDOFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A successful production Tools declaration carries first run into Private AI
/// after startup. Reading consumes the handoff; injected display fixtures do not
/// set it, and a failed declaration cannot set it.
pub(super) fn take_first_run_handoff() -> bool {
    FIRST_RUN_HANDOFF.with(|handoff| handoff.replace(false))
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
        move || {
            FIRST_RUN_HANDOFF.with(|handoff| handoff.set(true));
            on_declared();
        },
        true,
    );
}

/// Existing injected-candidate test entry retains its direct Tools behavior.
#[cfg(all(test, target_os = "linux"))]
fn present_with<F>(
    application: &adw::Application,
    dir: PathBuf,
    candidates: Vec<SourceCandidate>,
    on_declared: F,
) where
    F: Fn() + 'static,
{
    present_mode(application, dir, candidates, on_declared, false);
}

fn present_mode<F>(
    application: &adw::Application,
    dir: PathBuf,
    candidates: Vec<SourceCandidate>,
    on_declared: F,
    welcome: bool,
) where
    F: Fn() + 'static,
{
    // This screen uses the community brand's type scale, and it is the FIRST
    // window the shell ever opens -- it runs before the daemon, so neither
    // the history view nor settings has had a chance to install the provider.
    // Without this call the brand classes below resolve to nothing and the
    // screen renders in GTK's defaults: the 27px heading sets at body size
    // and the whole page flattens into one undifferentiated column. Nothing
    // errors, because `add_css_class` takes a string and never fails, which
    // is why 126 passing tests said nothing about it.
    // The main stylesheet, for the same reason and with a sharper edge: it is
    // installed by `App::build`, which cannot run without a Worker, which
    // cannot exist without a started daemon. This screen exists precisely
    // because the daemon did not start -- so until now it drew with no
    // stylesheet at all, and every `tc-` class on it, brand or otherwise, was
    // inert.
    super::style::install();
    super::community_brand::install();

    let window = adw::ApplicationWindow::builder()
        .application(application)
        .default_width(620)
        .default_height(640)
        .build();

    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(space::L)
        .margin_top(space::XL)
        .margin_bottom(space::XL)
        .margin_start(space::XL)
        .margin_end(space::XL)
        .build();

    let heading = gtk::Label::builder()
        .label(copy::ROOTS_TITLE)
        .xalign(0.0)
        .wrap(true)
        .build();
    heading.add_css_class("tc-brand-dialog-title");
    outer.append(&heading);
    outer.append(&body_label(copy::ROOTS_BODY));
    let consequence = body_label(copy::ROOTS_BOTH);
    // A notice box rather than a colour on running text. This is the sentence
    // that makes "I don't use this" mean something -- leave it as prose and it
    // reads as the third paragraph of an intro nobody finishes. The brand
    // sheet loads at a higher priority than style.css, so an emphasis picked
    // from style.css would lose to `tc-brand-body`'s own size and colour and
    // silently do nothing, which is the same failure this screen already had.
    consequence.add_css_class("tc-brand-notice");
    outer.append(&consequence);

    let failure = gtk::Label::builder()
        .label(copy::ROOTS_FAILED)
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .build();
    failure.add_css_class("tc-refused");

    let continue_button = gtk::Button::with_label(copy::ROOTS_CONTINUE);
    continue_button.add_css_class("suggested-action");
    continue_button.set_sensitive(false);

    // A rule between the two source blocks. Without one the boundary is
    // carried by whitespace and a bold title alone, and the three controls of
    // the first block sit closer to the second block's title than that title
    // sits to its own controls -- so the eye groups them wrongly. Each block
    // asks for a separator ahead of itself except the first, which is the same
    // rule the history cells use for their divider.
    let choices: Rc<Vec<Choice>> = Rc::new(
        candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| {
                if index > 0 {
                    let rule = gtk::Box::builder()
                        .orientation(gtk::Orientation::Horizontal)
                        .margin_top(space::S)
                        .margin_bottom(space::S)
                        .build();
                    rule.add_css_class("tc-rule");
                    outer.append(&rule);
                }
                build_choice(&outer, candidate, &window)
            })
            .collect(),
    );

    // Re-evaluated on every toggle rather than tracked incrementally: the
    // rule is "both answered", and reading both is cheaper than keeping a
    // counter honest across three widgets per row.
    let pending = Rc::new(std::cell::Cell::new(false));
    let refresh = {
        let choices = choices.clone();
        let continue_button = continue_button.clone();
        let pending = pending.clone();
        move || {
            let complete = choices.iter().all(|c| c.declaration().is_some());
            for choice in choices.iter() {
                choice.group.set_sensitive(!pending.get());
            }
            continue_button.set_sensitive(complete && !pending.get());
        }
    };
    let refresh = Rc::new(refresh);
    for choice in choices.iter() {
        for toggle in [&choice.watch, &choice.off] {
            let refresh = refresh.clone();
            toggle.connect_toggled(move |_| refresh());
        }
    }

    outer.append(&failure);

    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .halign(gtk::Align::End)
        .spacing(space::S)
        .vexpand(true)
        .valign(gtk::Align::End)
        .build();
    row.append(&continue_button);
    outer.append(&row);

    continue_button.connect_clicked({
        let choices = choices.clone();
        let window = window.clone();
        let failure = failure.clone();
        let on_declared = Rc::new(on_declared);
        move |_| {
            if pending.replace(true) {
                return;
            }
            refresh();
            // Every answered choice, not a hand-listed pair: this screen
            // renders one row per discovered source, so a source it can
            // show is a source whose answer must be written. The button is
            // insensitive until all of them are answered, and this
            // re-reads rather than trusting that -- an unanswered row is
            // simply absent, and `declare_sources` refuses an incomplete
            // declaration.
            let answers: Vec<(String, SourceDeclaration)> = choices
                .iter()
                .filter_map(|c| c.declaration().map(|d| (c.source.clone(), d)))
                .collect();
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
            let refresh = refresh.clone();
            let failure = failure.clone();
            let window = window.clone();
            let on_declared = on_declared.clone();
            gtk::glib::spawn_future_local(async move {
                let result = rx.recv().await;
                pending.set(false);
                refresh();
                match result {
                    Ok(Ok(())) => {
                        failure.set_visible(false);
                        window.close();
                        on_declared();
                    }
                    // Fixed copy keeps paths and OS errors out of the UI.
                    _ => failure.set_visible(true),
                }
            });
        }
    });

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("tc-root");
    content.append(
        &adw::HeaderBar::builder()
            .title_widget(&adw::WindowTitle::new(copy::APP_NAME, ""))
            .build(),
    );
    if welcome {
        let words = trace_commons_contributor::first_run_copy::first_run_copy();
        heading.set_label(&format!(
            "{} {}",
            words.tools.title_light, words.tools.title_bold
        ));
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        let intro = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(space::L)
            .margin_top(space::XL)
            .margin_bottom(space::XL)
            .margin_start(space::XL)
            .margin_end(space::XL)
            .build();
        let title = body_label(words.welcome.title);
        title.add_css_class("tc-brand-dialog-title");
        intro.append(&title);
        intro.append(&body_label(words.welcome.body));
        let start = gtk::Button::with_label(words.welcome.start);
        start.add_css_class("suggested-action");
        intro.append(&start);
        let back = gtk::Button::with_label(words.frame.back);
        back.add_css_class("flat");
        outer.prepend(&back);
        let tools = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&outer)
            .build();
        stack.add_named(&intro, Some("welcome"));
        stack.add_named(&tools, Some("tools"));
        let pages = stack.clone();
        start.connect_clicked(move |_| pages.set_visible_child_name("tools"));
        let pages = stack.clone();
        back.connect_clicked(move |_| pages.set_visible_child_name("welcome"));
        stack.set_visible_child_name("welcome");
        content.append(&stack);
    } else {
        content.append(&outer);
    }
    window.set_content(Some(&content));
    window.present();
}

/// Tools for a running daemon. Each row still requires an explicit answer;
/// discovery alone does not grant access or change a watched source. The UI
/// uses the existing native folder chooser so Linux portal semantics are kept.
pub(super) fn connected(app: &Rc<super::App>, on_saved: impl Fn() + 'static) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, space::L);
    let words = trace_commons_contributor::first_run_copy::first_run_copy();
    let title = body_label(&format!(
        "{} {}",
        words.tools.title_light, words.tools.title_bold
    ));
    title.add_css_class("tc-brand-dialog-title");
    root.append(&title);
    root.append(&body_label(copy::ROOTS_BODY));
    let loading = gtk::Spinner::new();
    loading.start();
    root.append(&loading);
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(discovery::probe_this_machine());
    });
    let generation = Rc::new(std::cell::Cell::new(0_u64));
    root.connect_unmap({
        let generation = generation.clone();
        move |_| generation.set(generation.get().wrapping_add(1))
    });
    let weak_root = root.downgrade();
    let weak_app = Rc::downgrade(app);
    gtk::glib::spawn_future_local(async move {
        let Ok(candidates) = rx.recv().await else {
            return;
        };
        let (Some(root), Some(app)) = (weak_root.upgrade(), weak_app.upgrade()) else {
            return;
        };
        root.remove(&loading);
        let choices: Rc<Vec<_>> = Rc::new(
            candidates
                .iter()
                .map(|candidate| build_choice(&root, candidate, &app.window))
                .collect(),
        );
        let failure = body_label(copy::ROOTS_FAILED);
        failure.add_css_class("tc-refused");
        failure.set_visible(false);
        root.append(&failure);
        let button = gtk::Button::with_label(words.frame.continue_button);
        button.add_css_class("suggested-action");
        button.set_sensitive(false);
        root.append(&button);
        let pending = Rc::new(std::cell::Cell::new(false));
        let update: Rc<dyn Fn()> = {
            let choices = choices.clone();
            let pending = pending.clone();
            let button = button.clone();
            Rc::new(move || {
                let complete = !choices.is_empty()
                    && choices.iter().all(|choice| choice.declaration().is_some());
                for choice in choices.iter() {
                    choice.group.set_sensitive(!pending.get());
                }
                button.set_sensitive(complete && !pending.get());
            })
        };
        for choice in choices.iter() {
            for toggle in [&choice.watch, &choice.off] {
                let update = update.clone();
                toggle.connect_toggled(move |_| update());
            }
        }
        let on_saved = Rc::new(on_saved);
        let weak_root = root.downgrade();
        let weak_app = Rc::downgrade(&app);
        button.connect_clicked(move |_| {
            let (Some(root), Some(app)) = (weak_root.upgrade(), weak_app.upgrade()) else {
                return;
            };
            if !root.is_mapped() || pending.replace(true) {
                return;
            }
            update();
            let params = declaration_params(&choices);
            let Some(params) = params else {
                pending.set(false);
                update();
                failure.set_visible(true);
                return;
            };
            let pending = pending.clone();
            let update = update.clone();
            let failure = failure.clone();
            let on_saved = on_saved.clone();
            let weak_root = root.downgrade();
            let expected = params.clone();
            let submitted_generation = generation.get();
            let generation = generation.clone();
            app.call("set_settings", params, move |_, result| {
                pending.set(false);
                update();
                if generation.get() != submitted_generation
                    || !weak_root.upgrade().is_some_and(|root| root.is_mapped())
                {
                    return;
                }
                let confirmed = result
                    .as_ref()
                    .is_ok_and(|settings| declarations_confirmed(&expected, settings));
                failure.set_visible(!confirmed);
                if confirmed {
                    on_saved();
                }
            });
        });
    });
    root
}

fn declarations_confirmed(expected: &serde_json::Value, settings: &serde_json::Value) -> bool {
    expected.as_object().is_some_and(|fields| {
        !fields.is_empty()
            && fields
                .iter()
                .all(|(key, value)| settings.get(key) == Some(value))
    })
}

fn declaration_params(choices: &[Choice]) -> Option<serde_json::Value> {
    if choices.is_empty() {
        return None;
    }
    let mut fields = serde_json::Map::new();
    for choice in choices {
        let declaration = choice.declaration()?;
        let key = trace_commons_contributor::daemon::settings::source_settings_key(&choice.source)?;
        fields.insert(key.to_owned(), serde_json::to_value(declaration).ok()?);
    }
    Some(serde_json::Value::Object(fields))
}

fn build_choice(
    outer: &gtk::Box,
    candidate: &SourceCandidate,
    window: &adw::ApplicationWindow,
) -> Choice {
    let group = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(space::XS)
        .build();

    let title = gtk::Label::builder()
        .label(source_title(&candidate.source))
        .xalign(0.0)
        .build();
    title.add_css_class("tc-card-title");
    group.append(&title);

    let path_label = gtk::Label::builder()
        .label(candidate.path.to_string_lossy().as_ref())
        .xalign(0.0)
        .wrap(true)
        .build();
    path_label.add_css_class("tc-ledger");
    group.append(&path_label);

    style::append_meta(&group, evidence_line(candidate));

    // Neither is active. GTK gives a grouped CheckButton radio behaviour,
    // and a group with nothing active is exactly the unanswered state this
    // screen needs to be able to represent.
    let watch = gtk::CheckButton::with_label(copy::ROOTS_WATCH);
    let off = gtk::CheckButton::with_label(copy::ROOTS_OFF);
    off.set_group(Some(&watch));
    group.append(&watch);
    group.append(&off);

    let choose = gtk::Button::with_label(copy::ROOTS_CHOOSE);
    choose.add_css_class("flat");
    // Left-aligned and shrunk to its label. A flat button that fills the
    // column centres its text, and centred bold text between two rows reads
    // as a heading for the row BELOW it rather than a control belonging to
    // the row above -- which is exactly how it looked in the first photograph
    // of this screen.
    choose.set_halign(gtk::Align::Start);
    group.append(&choose);

    outer.append(&group);

    let path = Rc::new(RefCell::new(candidate.path.clone()));

    choose.connect_clicked({
        let group = group.clone();
        let window = window.clone();
        let path_label = path_label.clone();
        let watch = watch.clone();
        let path = path.clone();
        move |_| {
            if !group.is_sensitive() {
                return;
            }
            let chooser = gtk::FileChooserNative::new(
                Some(copy::ROOTS_CHOOSE),
                Some(&window),
                gtk::FileChooserAction::SelectFolder,
                None,
                None,
            );
            let path_label = path_label.clone();
            let watch = watch.clone();
            let path = path.clone();
            let group = group.clone();
            chooser.connect_response(move |chooser, response| {
                if group.is_sensitive()
                    && response == gtk::ResponseType::Accept
                    && let Some(chosen) = chooser.file().and_then(|f| f.path())
                {
                    path_label.set_label(chosen.to_string_lossy().as_ref());
                    *path.borrow_mut() = chosen;
                    // Picking a folder is an affirmative act, so it answers
                    // the row -- with the folder that was actually picked.
                    // Activating the toggle is all that is needed: its
                    // handler re-reads BOTH rows, where setting Continue
                    // sensitive here would enable it on one answer.
                    watch.set_active(true);
                }
                chooser.destroy();
            });
            chooser.show();
        }
    });

    Choice {
        group,
        path,
        watch,
        off,
        source: candidate.source.clone(),
    }
}

/// The title on a roots row: the only thing naming the store a contributor is
/// agreeing to.
///
/// Every known source is matched explicitly. The arm this replaced sent
/// anything unrecognised to the Claude Code title, so when a third source
/// appeared the screen offered two rows both called "Claude Code traces",
/// one of them pointing at `~/.gemini/tmp`. Agreeing to a store under another
/// store's name is not agreeing, and a fallback that reads as a safe default
/// is how that shipped.
///
/// The unknown arm now says so rather than borrowing a name. A source with no
/// title here is a bug -- `no_two_sources_share_a_title` catches the shape of
/// it -- but showing the raw slug beside its real path is honest, where
/// showing the wrong product name is not.
fn source_title(source: &str) -> &'static str {
    use trace_commons_contributor::source::{
        SOURCE_CLAUDE_CODE, SOURCE_CLINE, SOURCE_CODEX, SOURCE_GEMINI_CLI, SOURCE_OPENCODE,
    };
    match source {
        SOURCE_CLAUDE_CODE => copy::ROOTS_CLAUDE,
        SOURCE_CODEX => copy::ROOTS_CODEX,
        SOURCE_GEMINI_CLI => copy::ROOTS_GEMINI,
        SOURCE_CLINE => copy::ROOTS_CLINE,
        SOURCE_OPENCODE => copy::ROOTS_OPENCODE,
        _ => copy::ROOTS_UNKNOWN_SOURCE,
    }
}

/// The evidence under a row, in words rather than fields.
fn evidence_line(candidate: &SourceCandidate) -> String {
    let mut line = if !candidate.exists {
        copy::ROOTS_ABSENT.to_string()
    } else if candidate.session_count == 0 {
        copy::ROOTS_EMPTY.to_string()
    } else {
        let ago = candidate
            .most_recent
            .map(|when| copy::roots_ago((chrono::Utc::now() - when).num_seconds().max(0)));
        copy::roots_evidence(candidate.session_count, ago.as_deref())
    };
    if candidate.relocated_by_env {
        line.push_str(" - ");
        line.push_str(copy::ROOTS_RELOCATED);
    }
    line
}

fn body_label(text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .build();
    label.add_css_class("tc-brand-body");
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_handoff_is_consumed_once() {
        FIRST_RUN_HANDOFF.with(|handoff| handoff.set(true));
        assert!(take_first_run_handoff());
        assert!(!take_first_run_handoff());
    }

    #[test]
    fn connected_tools_requires_acknowledgement_of_every_declaration() {
        use serde_json::json;
        let expected = json!({"claude_source": {"mode":"off"}, "codex_source": {"mode":"watch", "path":"/chosen/codex"}});
        assert!(declarations_confirmed(&expected, &expected));
        assert!(!declarations_confirmed(&expected, &json!({"ok":true})));
        assert!(!declarations_confirmed(
            &expected,
            &json!({"claude_source":{"mode":"off"}})
        ));
        let mut changed = expected.clone();
        changed["codex_source"]["path"] = json!("/different/path");
        assert!(!declarations_confirmed(&expected, &changed));
        assert!(!declarations_confirmed(&json!({}), &json!({})));
    }

    fn candidate(source: &str, count: u64, exists: bool) -> SourceCandidate {
        SourceCandidate {
            source: source.to_string(),
            path: PathBuf::from("/home/x/.claude/projects"),
            exists,
            session_count: count,
            most_recent: None,
            total_bytes: None,
            relocated_by_env: false,
            answers_at: None,
        }
    }

    #[test]
    fn an_absent_store_says_so_rather_than_reporting_zero_sessions() {
        // "0 traces" and "the folder isn't there" are different facts and
        // lead to different answers.
        let line = evidence_line(&candidate("claude-code", 0, false));
        assert_eq!(line, copy::ROOTS_ABSENT);
    }

    #[test]
    fn an_existing_but_empty_store_is_distinguishable_from_an_absent_one() {
        let line = evidence_line(&candidate("claude-code", 0, true));
        assert_eq!(line, copy::ROOTS_EMPTY);
        assert_ne!(line, copy::ROOTS_ABSENT);
    }

    #[test]
    fn a_populated_store_reports_its_count() {
        let line = evidence_line(&candidate("claude-code", 946, true));
        assert!(line.contains("946 traces"), "got: {line}");
    }

    #[test]
    fn a_relocated_store_says_why_its_path_is_unusual() {
        let mut c = candidate("codex", 3, true);
        c.relocated_by_env = true;
        let line = evidence_line(&c);
        assert!(line.contains(copy::ROOTS_RELOCATED), "got: {line}");
    }

    #[test]
    fn each_source_gets_its_own_title() {
        assert_eq!(source_title("codex"), copy::ROOTS_CODEX);
        assert_eq!(source_title("claude-code"), copy::ROOTS_CLAUDE);
        assert_eq!(source_title("gemini-cli"), copy::ROOTS_GEMINI);
        assert_eq!(source_title("cline"), copy::ROOTS_CLINE);
    }

    /// Titles must be distinct, because this label is the only thing naming
    /// the store a contributor is agreeing to. A catch-all arm previously sent
    /// Gemini to the Claude Code title, so the screen offered two rows both
    /// called "Claude Code traces", one of them pointing at ~/.gemini/tmp.
    /// Agreeing to a store under another store's name is not agreeing.
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
