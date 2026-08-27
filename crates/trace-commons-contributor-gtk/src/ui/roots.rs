//! The roots screen: which session folders this shell may watch.
//!
//! It runs BEFORE the daemon starts, and therefore before any IPC exists.
//! That is why it is not one of the six onboarding screens in
//! [`super::onboarding`] -- every one of those is answered by a daemon call,
//! so none of them could ever clear a refusal that stops the daemon coming
//! up. Until this window existed the Linux shell answered an undeclared-roots
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

use super::style::space;
use crate::copy;

/// One agent's row: the discovered evidence, and the two answers.
struct Choice {
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

/// Show the roots window. `on_declared` runs once both answers are saved.
pub fn present<F>(application: &adw::Application, dir: PathBuf, on_declared: F)
where
    F: Fn() + 'static,
{
    present_with(
        application,
        dir,
        discovery::probe_this_machine(),
        on_declared,
    );
}

/// The testable half: the candidates are injected rather than probed, so a
/// test never has to look at the developer's real home directory.
fn present_with<F>(
    application: &adw::Application,
    dir: PathBuf,
    candidates: Vec<SourceCandidate>,
    on_declared: F,
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
    let refresh = {
        let choices = choices.clone();
        let continue_button = continue_button.clone();
        move || {
            let complete = choices.iter().all(|c| c.declaration().is_some());
            continue_button.set_sensitive(complete);
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
            // Every answered choice, not a hand-listed pair: this screen
            // renders one row per discovered source, so a source it can
            // show is a source whose answer must be written. The button is
            // insensitive until all of them are answered, and this
            // re-reads rather than trusting that -- an unanswered row is
            // simply absent, and `declare_sources` refuses an incomplete
            // declaration.
            let answers: Vec<(&str, SourceDeclaration)> = choices
                .iter()
                .filter_map(|c| c.declaration().map(|d| (c.source.as_str(), d)))
                .collect();
            match crate::backend::declare_sources(&dir, &answers) {
                Ok(()) => {
                    failure.set_visible(false);
                    window.close();
                    on_declared();
                }
                // The label is fixed and never carries the path: a settings
                // failure here is the one input that is itself a filesystem
                // location.
                Err(_) => failure.set_visible(true),
            }
        }
    });

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("tc-root");
    content.append(
        &adw::HeaderBar::builder()
            .title_widget(&adw::WindowTitle::new(copy::APP_NAME, ""))
            .build(),
    );
    content.append(&outer);
    window.set_content(Some(&content));
    window.present();
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

    let evidence = gtk::Label::builder()
        .label(evidence_line(candidate))
        .xalign(0.0)
        .wrap(true)
        .build();
    evidence.add_css_class("tc-meta");
    group.append(&evidence);

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
        let window = window.clone();
        let path_label = path_label.clone();
        let watch = watch.clone();
        let path = path.clone();
        move |_| {
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
            chooser.connect_response(move |chooser, response| {
                if response == gtk::ResponseType::Accept
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
        path,
        watch,
        off,
        source: candidate.source.clone(),
    }
}

fn source_title(source: &str) -> &'static str {
    match source {
        trace_commons_contributor::source::SOURCE_CODEX => copy::ROOTS_CODEX,
        _ => copy::ROOTS_CLAUDE,
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

    fn candidate(source: &str, count: u64, exists: bool) -> SourceCandidate {
        SourceCandidate {
            source: source.to_string(),
            path: PathBuf::from("/home/x/.claude/projects"),
            exists,
            session_count: count,
            most_recent: None,
            relocated_by_env: false,
        }
    }

    #[test]
    fn an_absent_store_says_so_rather_than_reporting_zero_sessions() {
        // "0 sessions" and "the folder isn't there" are different facts and
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
        assert!(line.contains("946 sessions"), "got: {line}");
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
    }
}
