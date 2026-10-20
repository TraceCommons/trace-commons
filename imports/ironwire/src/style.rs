//! Colour for the terminal, and the rules about when there isn't any.
//!
//! Colour here is never the only carrier of meaning. Every state that gets a
//! colour also gets words — `exhausted`, `skipping after 5 consecutive
//! failures`, `unknown` — so the screen reads identically piped to a file, on
//! a monochrome terminal, and to someone who cannot distinguish red from
//! green. Colour makes the important line findable in a screen full of text;
//! it is not allowed to *be* the information.
//!
//! Detection follows the conventions people already have configured:
//! `NO_COLOR` wins over everything, then `--color`, then `FORCE_COLOR` /
//! `CLICOLOR_FORCE`, then whether the stream is a terminal that is not `dumb`.
//! The default is auto, so `ironwire status | less` and
//! `ironwire status --json | jq` are unchanged.
//!
//! *Which* stream depends on what is being written. The rendered screen goes
//! to stdout and asks about stdout; `tracing` goes to stderr and asks about
//! stderr (`logs_coloured`). They are separate questions because the two are
//! redirected separately, and a log line with escape sequences in it is the
//! same bug as a coloured status screen in a pipe.

use std::fmt::Display;
use std::io::IsTerminal;

/// What the user asked for on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub(crate) enum ColorChoice {
    /// Colour when stdout is a terminal.
    #[default]
    Auto,
    /// Always, even into a pipe.
    Always,
    /// Never.
    Never,
}

/// The palette, or the absence of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Style {
    enabled: bool,
}

/// SGR codes. Kept to the sixteen basic colours: the 256-colour cube renders
/// as something else entirely under a theme that remaps it, and this screen is
/// read inside other people's terminals.
mod sgr {
    pub(super) const RESET: &str = "\x1b[0m";
    pub(super) const BOLD: &str = "\x1b[1m";
    pub(super) const DIM: &str = "\x1b[2m";
    pub(super) const RED: &str = "\x1b[31m";
    pub(super) const GREEN: &str = "\x1b[32m";
    pub(super) const YELLOW: &str = "\x1b[33m";
    pub(super) const BLUE: &str = "\x1b[34m";
    pub(super) const CYAN: &str = "\x1b[36m";
}

impl Style {
    /// Resolve a choice against the environment, for what goes to stdout.
    pub(crate) fn resolve(choice: ColorChoice) -> Self {
        Self {
            enabled: decide(choice, std::io::stdout().is_terminal()),
        }
    }

    /// No colour. What tests render against, so an assertion matches on the
    /// words rather than on escape sequences.
    #[cfg(test)]
    pub(crate) const fn plain() -> Self {
        Self { enabled: false }
    }

    fn paint(self, code: &str, text: impl Display) -> String {
        if self.enabled {
            format!("{code}{text}{}", sgr::RESET)
        } else {
            text.to_string()
        }
    }

    /// A section heading.
    pub(crate) fn heading(self, text: impl Display) -> String {
        self.paint(sgr::BOLD, text)
    }

    /// A backend or subject name.
    pub(crate) fn name(self, text: impl Display) -> String {
        self.paint(sgr::CYAN, text)
    }

    /// Supporting detail: units, ages, column headers, the parts of a line
    /// that are structure rather than news.
    pub(crate) fn dim(self, text: impl Display) -> String {
        self.paint(sgr::DIM, text)
    }

    /// A measured figure worth the eye landing on.
    pub(crate) fn value(self, text: impl Display) -> String {
        self.paint(sgr::BOLD, text)
    }

    /// Everything is fine.
    pub(crate) fn good(self, text: impl Display) -> String {
        self.paint(sgr::GREEN, text)
    }

    /// Worth knowing about, not yet a problem.
    pub(crate) fn warn(self, text: impl Display) -> String {
        self.paint(sgr::YELLOW, text)
    }

    /// A thing that has gone wrong or is about to.
    pub(crate) fn bad(self, text: impl Display) -> String {
        self.paint(sgr::RED, text)
    }

    /// Something the user could act on.
    pub(crate) fn action(self, text: impl Display) -> String {
        self.paint(sgr::BLUE, text)
    }

    /// Colour by how much of a window is gone.
    ///
    /// The thresholds are the usage monitor's — green below half, yellow to
    /// ninety, red past it — chosen there because ninety percent of a
    /// five-hour window is the point at which the remaining time stops being
    /// enough to finish what you were doing.
    pub(crate) fn by_usage(self, used_pct: f64, text: impl Display) -> String {
        if used_pct >= 90.0 {
            self.bad(text)
        } else if used_pct >= 50.0 {
            self.warn(text)
        } else {
            self.good(text)
        }
    }
}

/// Whether the log writer should emit colour.
///
/// The same rules asked about stderr, because that is where the logs go. Two
/// invocations depend on it being a separate question: `ironwire serve 2>log`
/// should still colour a status screen on the terminal it is talking to, and
/// `ironwire status 2>&1 | grep` has to come back greppable. A `tracing` line
/// that carries escape sequences into a pipe is the same bug as a status
/// screen that does — it just lands in someone's log file instead.
pub(crate) fn logs_coloured(choice: ColorChoice) -> bool {
    decide(choice, std::io::stderr().is_terminal())
}

fn decide(choice: ColorChoice, is_terminal: bool) -> bool {
    match choice {
        ColorChoice::Never => false,
        ColorChoice::Always => true,
        ColorChoice::Auto => auto(is_terminal),
    }
}

/// `NO_COLOR` first, because a user who set it has said so once and should not
/// have to say it per tool. Then the force variables, then the stream.
fn auto(is_terminal: bool) -> bool {
    if std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()) {
        return false;
    }
    if std::env::var_os("FORCE_COLOR").is_some() || std::env::var_os("CLICOLOR_FORCE").is_some() {
        return true;
    }
    if std::env::var("TERM").is_ok_and(|term| term == "dumb") {
        return false;
    }
    is_terminal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_style_emits_no_escape_sequences_at_all() {
        // Everything downstream is asserted against words, and a stray escape
        // in piped output is what makes a status screen ungreppable.
        let plain = Style::plain();
        assert_eq!(plain.bad("exhausted"), "exhausted");
        assert_eq!(plain.heading("Balance"), "Balance");
        assert_eq!(plain.by_usage(99.0, "99%"), "99%");
        assert!(!plain.dim("x").contains('\x1b'));
    }

    #[test]
    fn colour_wraps_and_always_resets() {
        // An unterminated sequence bleeds into the user's next prompt.
        let coloured = Style { enabled: true };
        let painted = coloured.bad("exhausted");
        assert!(painted.starts_with("\x1b[31m"));
        assert!(painted.ends_with("\x1b[0m"));
        assert!(painted.contains("exhausted"));
    }

    #[test]
    fn never_and_always_do_not_consult_the_environment() {
        assert_eq!(Style::resolve(ColorChoice::Never), Style::plain());
        assert_eq!(Style::resolve(ColorChoice::Always), Style { enabled: true });
    }

    /// The rule the journey script enforces end to end, at the one place that
    /// decides it: a stream that is not a terminal gets no escape sequences,
    /// whether it is the status screen or a `tracing` line on stderr.
    #[test]
    fn a_stream_that_is_not_a_terminal_gets_no_colour() {
        assert!(!decide(ColorChoice::Auto, false));
        assert!(
            decide(ColorChoice::Always, false),
            "--color always is a decision, not a guess"
        );
        assert!(!decide(ColorChoice::Never, true));
    }

    #[test]
    fn usage_colour_follows_the_monitors_thresholds() {
        let s = Style { enabled: true };
        assert!(s.by_usage(10.0, "x").starts_with(sgr::GREEN));
        assert!(s.by_usage(49.9, "x").starts_with(sgr::GREEN));
        assert!(s.by_usage(50.0, "x").starts_with(sgr::YELLOW));
        assert!(s.by_usage(89.9, "x").starts_with(sgr::YELLOW));
        assert!(s.by_usage(90.0, "x").starts_with(sgr::RED));
        assert!(s.by_usage(250.0, "x").starts_with(sgr::RED));
    }
}
