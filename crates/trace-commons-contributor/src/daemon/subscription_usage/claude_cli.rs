//! Claude usage from the user's own `claude` CLI, as a fallback when the
//! OAuth reader cannot produce a reading and only on a user-pressed Refresh.
//!
//! Claude Code reads its own Keychain item, so this path never makes macOS
//! ask about this app. The CLI runs under a PTY in a private working
//! directory, with no tools, no MCP servers, no hooks and transcripts turned
//! off; the app types `/usage`, reads the session and weekly figures off the
//! rendered screen, and kills the CLI's process group. Screen text is never
//! logged and never leaves this module except as parsed figures.
//!
//! The pure parts (terminal replay, screen classification, figure and reset
//! parsing) are separate from the process driver so fixtures test them.
//!
//! The driver is macOS only (the PTY layer is Darwin's, and the Keychain
//! prompt this works around exists only there). Elsewhere the screen code is
//! compiled and tested but has no caller.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};

use super::Window;

pub(super) const COLUMNS: u16 = 160;
pub(super) const ROWS: u16 = 200;

// ---------------------------------------------------------------- screen

/// Replay the cursor and erase sequences a TUI uses onto a bounded grid and
/// return the visible text, one line per row, trailing blanks trimmed. Other
/// escape sequences (colour, OSC titles, private modes) are dropped.
pub(super) fn render(bytes: &[u8]) -> String {
    let mut screen = Grid::new(COLUMNS as usize, ROWS as usize);
    let text = String::from_utf8_lossy(bytes);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            screen.put(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                let mut params = String::new();
                let mut fin = None;
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        fin = Some(c);
                        break;
                    }
                    if params.len() < 32 {
                        params.push(c);
                    }
                }
                if let Some(fin) = fin {
                    screen.csi(&params, fin);
                }
            }
            // OSC, DCS, PM, APC: skip to BEL or ESC \.
            Some(']' | 'P' | '^' | '_') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // Charset designations take one more character.
            Some('(' | ')' | '*' | '+') => {
                chars.next();
            }
            _ => {}
        }
    }
    screen.text()
}

struct Grid {
    cols: usize,
    rows: usize,
    cells: Vec<Vec<char>>,
    row: usize,
    col: usize,
}

/// A spacer for the second cell of a wide character.
const SPACER: char = '\0';

impl Grid {
    fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            cells: vec![vec![' '; cols]; rows],
            row: 0,
            col: 0,
        }
    }

    fn newline(&mut self) {
        self.col = 0;
        if self.row + 1 == self.rows {
            self.cells.remove(0);
            self.cells.push(vec![' '; self.cols]);
        } else {
            self.row += 1;
        }
    }

    fn clear(&mut self, row: usize, col: usize) {
        if col > 0 && self.cells[row][col] == SPACER {
            self.cells[row][col - 1] = ' ';
        }
        if col + 1 < self.cols && self.cells[row][col + 1] == SPACER {
            self.cells[row][col + 1] = ' ';
        }
        self.cells[row][col] = ' ';
    }

    fn put(&mut self, c: char) {
        match c {
            '\r' => self.col = 0,
            '\n' => self.newline(),
            '\u{8}' => self.col = self.col.min(self.cols - 1).saturating_sub(1),
            '\t' => self.col = ((self.col / 8 + 1) * 8).min(self.cols - 1),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {}
            c => {
                let width = width(c);
                if width == 0 {
                    return;
                }
                if self.col + width > self.cols {
                    self.newline();
                }
                for col in self.col..self.col + width {
                    self.clear(self.row, col);
                }
                self.cells[self.row][self.col] = c;
                if width == 2 {
                    self.cells[self.row][self.col + 1] = SPACER;
                }
                self.col += width;
            }
        }
    }

    fn csi(&mut self, params: &str, fin: char) {
        if !params.bytes().all(|b| b.is_ascii_digit() || b == b';') {
            // Private modes (`?25l`) and anything else unknown.
            return;
        }
        let bound = self.cols.max(self.rows);
        let mut values = params
            .split(';')
            .map(|v| v.parse::<usize>().unwrap_or(0).min(bound));
        let first = values.next().unwrap_or(0);
        let second = values.next().unwrap_or(0);
        let amount = first.max(1);
        self.col = self.col.min(self.cols - 1);
        match fin {
            'A' => self.row = self.row.saturating_sub(amount),
            'B' => self.row = (self.row + amount).min(self.rows - 1),
            'C' => self.col = (self.col + amount).min(self.cols - 1),
            'D' => self.col = self.col.saturating_sub(amount),
            'G' => self.col = (amount - 1).min(self.cols - 1),
            'H' | 'f' => {
                self.row = (amount - 1).min(self.rows - 1);
                self.col = (second.max(1) - 1).min(self.cols - 1);
            }
            'K' | 'J' if first <= 2 => {
                let cursor = self.row * self.cols + self.col;
                let (start, end) = if fin == 'K' {
                    (self.row * self.cols, (self.row + 1) * self.cols)
                } else {
                    (0, self.rows * self.cols)
                };
                let range = match first {
                    0 => cursor..end,
                    1 => start..cursor + 1,
                    _ => start..end,
                };
                for cell in range {
                    self.clear(cell / self.cols, cell % self.cols);
                }
            }
            _ => {}
        }
    }

    fn text(&self) -> String {
        let mut lines: Vec<String> = self
            .cells
            .iter()
            .map(|row| {
                let line: String = row.iter().filter(|c| **c != SPACER).collect();
                line.trim_end().to_owned()
            })
            .collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }
}

/// Terminal cell width: combining marks and joiners take none, East Asian
/// wide and emoji take two, everything else (box drawing, block bars) one.
fn width(c: char) -> usize {
    let v = c as u32;
    if (0x300..=0x36f).contains(&v)
        || (0x200b..=0x200f).contains(&v)
        || (0xfe00..=0xfe0f).contains(&v)
    {
        return 0;
    }
    const WIDE: &[(u32, u32)] = &[
        (0x1100, 0x115f),
        (0x2e80, 0x303e),
        (0x3041, 0x33ff),
        (0x3400, 0x4dbf),
        (0x4e00, 0x9fff),
        (0xa000, 0xa4cf),
        (0xac00, 0xd7a3),
        (0xf900, 0xfaff),
        (0xfe30, 0xfe6f),
        (0xff00, 0xff60),
        (0xffe0, 0xffe6),
        (0x1f300, 0x1f64f),
        (0x1f900, 0x1f9ff),
        (0x20000, 0x3fffd),
    ];
    if WIDE.iter().any(|(lo, hi)| (*lo..=*hi).contains(&v)) {
        2
    } else {
        1
    }
}

/// Lowercase with all whitespace removed, for matching labels across the
/// TUI's spacing.
fn compact(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

// ------------------------------------------------------- classification

const INSIGHTS_MARKER: &str = "what'scontributingtoyourlimitsusage?";

/// What the rendered screen is showing, as far as the driver needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScreenState {
    /// Claude Code wants a sign-in, an API key, or first-run onboarding.
    /// Never answered: Enter there could start a browser sign-in.
    NeedsSignIn,
    /// The workspace trust dialog for the probe's own directory, with the
    /// keys that move toward or confirm "Yes", or none while the selection
    /// marker is not visible yet.
    Trust(Option<&'static str>),
    /// The usage panel with a session figure.
    Usage,
    /// Claude Code said it could not load usage data.
    UsageFailed,
    /// A subscription notice with no quota figures.
    NoQuota,
    /// A dialog this driver does not know: a numbered chooser, or a settings
    /// error. Never answered.
    Unexpected,
    /// Anything else: still starting, the prompt, or the panel loading.
    Other,
}

pub(super) fn classify(screen: &str) -> ScreenState {
    let all = compact(screen);
    let quota = all.split(INSIGHTS_MARKER).next().unwrap_or_default();
    if has_session_figure(quota) {
        return ScreenState::Usage;
    }
    const SIGN_IN: &[&str] = &[
        "selectloginmethod",
        "loginwithyourclaude",
        "pleaserun/login",
        "run/logintostart",
        "invalidapikey",
        "oauthaccountinformationnotfound",
        "notloggedin",
        "chooseyourtextstyle",
        "choosethetextstyle",
        "oauthtokenhasexpired",
        "authentication_error",
    ];
    if SIGN_IN.iter().any(|marker| all.contains(marker)) {
        return ScreenState::NeedsSignIn;
    }
    if all.contains("failedtoloadusagedata") {
        return ScreenState::UsageFailed;
    }
    if all.contains("currentlyusingyoursubscription")
        && all.contains("claudecodeusage")
        && !all.contains("currentsession")
        && !all.contains("currentweek")
    {
        return ScreenState::NoQuota;
    }
    if all.contains("doyoutrustthefilesinthisfolder")
        || all.contains("yes,itrustthisfolder")
        || all.contains("quicksafetycheck")
        || all.contains("readytocodehere?")
    {
        return ScreenState::Trust(trust_keys(screen));
    }
    if all.contains("settingserror") || screen.lines().any(numbered_choice) {
        return ScreenState::Unexpected;
    }
    ScreenState::Other
}

/// A selected entry of a numbered chooser: `❯` then a number and a dot.
/// Not the prompt's own `❯`, which is followed by typed text.
fn numbered_choice(line: &str) -> bool {
    line.split('❯').skip(1).any(|after| {
        let after = after.trim_start();
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        digits > 0 && after[digits..].starts_with('.')
    })
}

/// The key that moves the `❯` marker toward the "Yes" option, or Enter when
/// it is on it. `None` when no marker is visible in the option block, so
/// Enter can never confirm another option.
fn trust_keys(screen: &str) -> Option<&'static str> {
    let lines: Vec<&str> = screen.lines().collect();
    let is_yes = |line: &str| {
        let line = compact(line);
        line.contains("yes,itrustthisfolder") || line.contains("yes,proceed")
    };
    let yes = lines.iter().position(|line| is_yes(line))?;
    let blank = |row: usize| lines[row].trim().is_empty();
    let mut first = yes;
    while first > 0 && !blank(first - 1) {
        first -= 1;
    }
    let mut last = yes;
    while last + 1 < lines.len() && !blank(last + 1) {
        last += 1;
    }
    let marker = (first..=last).find(|row| lines[*row].contains('❯'))?;
    Some(match marker.cmp(&yes) {
        std::cmp::Ordering::Equal => "\r",
        std::cmp::Ordering::Less => "\u{1b}[B",
        std::cmp::Ordering::Greater => "\u{1b}[A",
    })
}

fn has_session_figure(compact_quota: &str) -> bool {
    compact_quota
        .find("currentsession")
        .is_some_and(|at| percent_in(&compact_quota[at..]).is_some())
}

/// The first `N%` or `N.N%` in `text`.
fn percent_in(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let percent = bytes.iter().position(|b| *b == b'%')?;
    let mut start = percent;
    while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.') {
        start -= 1;
    }
    // Allow spaces between the number and the sign.
    if start == percent {
        let mut end = percent;
        while end > 0 && bytes[end - 1] == b' ' {
            end -= 1;
        }
        start = end;
        while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.') {
            start -= 1;
        }
        if start == end {
            return percent_in(&text[percent + 1..]);
        }
        return text[start..end]
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite());
    }
    text[start..percent]
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
}

// --------------------------------------------------------------- figures

/// Session, weekly and model-scoped windows from a rendered `/usage` panel.
/// `now` is local wall time and `local_zone` the IANA name of the local
/// zone, if known; a reset stated in another zone is left unknown.
pub(super) fn parse_usage<Tz: TimeZone>(
    screen: &str,
    now: DateTime<Tz>,
    local_zone: Option<&str>,
) -> Result<Vec<Window>, &'static str> {
    // Everything below the insights heading is tool names and usage shares,
    // never quota.
    let mut lines: Vec<&str> = Vec::new();
    for line in screen.lines() {
        if compact(line).contains(INSIGHTS_MARKER) {
            break;
        }
        lines.push(line);
    }
    // A redraw can leave an earlier frame above the latest panel.
    let start = lines
        .iter()
        .rposition(|line| compact(line).contains("currentsession"))
        .ok_or("cli_unparsed")?;
    let lines = &lines[start..];
    let mut windows: Vec<Window> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let label = compact(line);
        let (lane, model, duration) = if label.contains("currentsession") {
            ("session", None, 300)
        } else if let Some(model) = weekly_model(line) {
            if compact(&model) == "allmodels" {
                ("weekly", None, 10080)
            } else {
                ("model", Some(model), 10080)
            }
        } else {
            continue;
        };
        let mut used = None;
        let mut reset = None;
        for candidate in lines.iter().skip(index + 1).take(12) {
            let c = compact(candidate);
            if c.starts_with("current") || c.starts_with("extrausage") {
                break;
            }
            if used.is_none() {
                used = used_percent(candidate);
            }
            if reset.is_none() {
                reset = reset_text(candidate);
            }
        }
        // The label line itself can carry the figure on narrow layouts.
        if used.is_none() {
            used = used_percent(line);
        }
        if reset.is_none() {
            reset = reset_text(line);
        }
        if lane == "session" && used.is_none() {
            continue;
        }
        if windows.iter().any(|w| w.lane == lane && w.label == model) {
            continue;
        }
        windows.push(Window {
            lane: lane.into(),
            label: model,
            duration_mins: Some(duration),
            used_permille: used.map(|used: f64| (used * 10.0).round() as u16),
            resets_at: reset.and_then(|text| parse_reset(&text, now.clone(), local_zone)),
        });
        if windows.len() >= 16 {
            break;
        }
    }
    if !windows.iter().any(|w| w.lane == "session") {
        return Err("cli_unparsed");
    }
    Ok(windows)
}

/// The model in `Current week (<model>)`, without a trailing "only".
fn weekly_model(line: &str) -> Option<String> {
    let lower = line.to_lowercase();
    let at = lower.find("current week")?;
    let rest = &line[at..];
    let open = rest.find('(')?;
    let close = rest[open..].find(')')? + open;
    let model = rest[open + 1..close].trim();
    let model = model
        .strip_suffix(" only")
        .or_else(|| model.strip_suffix(" Only"))
        .unwrap_or(model)
        .trim();
    (!model.is_empty() && model.len() <= 64 && !model.chars().any(char::is_control))
        .then(|| model.to_owned())
}

/// Percent used, from "N% used" or "N% left/remaining/available". A bare
/// percentage is ambiguous and ignored, as is a status-bar context meter.
fn used_percent(line: &str) -> Option<f64> {
    let lower = line.to_lowercase();
    if lower.contains('|')
        && ["opus", "sonnet", "haiku", "default"]
            .iter()
            .any(|m| lower.contains(m))
    {
        return None;
    }
    let value = percent_in(&lower)?.clamp(0.0, 100.0);
    if ["used", "spent", "consumed"]
        .iter()
        .any(|k| lower.contains(k))
    {
        Some(value)
    } else if ["left", "remaining", "available"]
        .iter()
        .any(|k| lower.contains(k))
    {
        Some(100.0 - value)
    } else {
        None
    }
}

/// The text after "Reset"/"Resets" on a line, if any.
fn reset_text(line: &str) -> Option<String> {
    let lower = line.to_lowercase();
    let at = lower.find("reset")?;
    let rest = &line[at + "reset".len()..];
    let rest = rest.strip_prefix('s').unwrap_or(rest);
    let text = rest.trim().trim_end_matches(')').trim();
    // Restore the zone's closing parenthesis if the trim took it.
    let text = if text.matches('(').count() > text.matches(')').count() {
        format!("{text})")
    } else {
        text.to_owned()
    };
    (!text.is_empty() && text.len() <= 80).then_some(text)
}

/// A reset moment from Claude Code's wording: `5am`, `12:59pm`,
/// `Dec 24 at 3:59pm`, `Jan 2, 2026, 10:59pm`, each optionally followed by
/// `(Zone/Name)`. A date without a time, or a time in a zone other than the
/// local one, stays unknown rather than guessed.
pub(super) fn parse_reset<Tz: TimeZone>(
    text: &str,
    now: DateTime<Tz>,
    local_zone: Option<&str>,
) -> Option<DateTime<Utc>> {
    let mut text = text.trim().to_owned();
    if let Some(open) = text.rfind('(') {
        let zone = text[open + 1..].trim_end_matches(')').trim().to_owned();
        if !zone.is_empty() && Some(zone.as_str()) != local_zone {
            return None;
        }
        text.truncate(open);
    }
    let words: Vec<String> = text
        .replace(',', " ")
        .split_whitespace()
        .filter(|w| !w.eq_ignore_ascii_case("at"))
        .map(str::to_lowercase)
        .collect();
    let time_word = words.last()?;
    let time = parse_time(time_word)?;
    let date_words = &words[..words.len() - 1];
    let zone = now.timezone();
    let today = now.date_naive();
    let local = |date: NaiveDate| {
        zone.from_local_datetime(&date.and_time(time))
            .earliest()
            .map(|at| at.with_timezone(&Utc))
    };
    let now_utc = now.with_timezone(&Utc);
    let at = match date_words {
        [] => {
            let at = local(today)?;
            if at > now_utc {
                at
            } else {
                local(today.succ_opt()?)?
            }
        }
        [month, day] | [month, day, _] => {
            let month = month_number(month)?;
            let day: u32 = day.parse().ok()?;
            let explicit_year = match date_words {
                [_, _, year] => Some(year.parse::<i32>().ok()?),
                _ => None,
            };
            let year = explicit_year.unwrap_or(today.year());
            let at = local(NaiveDate::from_ymd_opt(year, month, day)?)?;
            if explicit_year.is_none() && at + Duration::days(1) < now_utc {
                local(NaiveDate::from_ymd_opt(year + 1, month, day)?)?
            } else {
                at
            }
        }
        _ => return None,
    };
    // A quota window is at most a week; anything far outside it is a
    // misread, not a reset.
    (at > now_utc - Duration::hours(1) && at < now_utc + Duration::days(8)).then_some(at)
}

fn parse_time(word: &str) -> Option<NaiveTime> {
    let (clock, pm) = if let Some(clock) = word.strip_suffix("pm") {
        (clock, true)
    } else {
        (word.strip_suffix("am")?, false)
    };
    let (hour, minute) = match clock.split_once(':') {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None => (clock.parse::<u32>().ok()?, 0),
    };
    if !(1..=12).contains(&hour) {
        return None;
    }
    let hour = match (hour, pm) {
        (12, false) => 0,
        (12, true) => 12,
        (h, true) => h + 12,
        (h, false) => h,
    };
    NaiveTime::from_hms_opt(hour, minute, 0)
}

fn month_number(word: &str) -> Option<u32> {
    let months = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let prefix: String = word.chars().take(3).collect();
    months
        .iter()
        .position(|m| *m == prefix)
        .map(|i| i as u32 + 1)
}

/// The IANA name of the local zone: `TZ` when it names one, else the target
/// of `/etc/localtime`. `None` when neither says.
pub(super) fn local_zone_name() -> Option<String> {
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim_start_matches(':');
        if tz.contains('/') && !tz.starts_with('/') {
            return Some(tz.to_owned());
        }
    }
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let target = target.to_string_lossy();
    let (_, zone) = target.split_once("zoneinfo/")?;
    (!zone.is_empty()).then(|| zone.to_owned())
}

// ------------------------------------------------------------- the probe

/// The `claude` process side, behind a seam: production drives the real CLI
/// on macOS; test builds never launch anything.
pub(super) trait CliProbe: Send + Sync {
    /// Run on a leased native thread. The rendered `/usage` screen, or a
    /// label. Never returns screen text in an error.
    fn usage_screen(&self, login: &super::LoginSpec) -> Result<String, &'static str>;
}

/// No CLI: test builds, and platforms without the PTY driver.
pub(super) struct NoCli;

impl CliProbe for NoCli {
    fn usage_screen(&self, _login: &super::LoginSpec) -> Result<String, &'static str> {
        Err("native_cli_unavailable")
    }
}

pub(super) fn for_build() -> std::sync::Arc<dyn CliProbe> {
    if cfg!(any(test, feature = "test-credential-store")) {
        return std::sync::Arc::new(NoCli);
    }
    #[cfg(target_os = "macos")]
    {
        std::sync::Arc::new(driver::ClaudeCli)
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::sync::Arc::new(NoCli)
    }
}

/// Claude Code's project directory name for `path`: every character outside
/// `[0-9A-Za-z]` becomes `-`. `None` past 200 characters, where Claude Code
/// truncates and appends a hash, which would also hide the probe suffix the
/// session discovery exclusion keys on.
pub(super) fn project_dir_name(path: &std::path::Path) -> Option<String> {
    let name: String = path
        .to_str()?
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    (name.len() <= 200).then_some(name)
}

/// Remove what a probe session can leave under the Claude config root: its
/// transcript and session folder in the probe's own project directory, and
/// that directory when nothing else is in it. Only that directory is
/// touched.
pub(super) fn clean_probe_artifacts(config_root: &std::path::Path, project: &str, session: &str) {
    let dir = config_root.join("projects").join(project);
    let Ok(meta) = std::fs::symlink_metadata(&dir) else {
        return;
    };
    if !meta.is_dir() {
        return;
    }
    let _ = std::fs::remove_file(dir.join(format!("{session}.jsonl")));
    let session_dir = dir.join(session);
    if std::fs::symlink_metadata(&session_dir).is_ok_and(|m| m.is_dir()) {
        let _ = std::fs::remove_dir_all(&session_dir);
    }
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl")
                && entry.file_type().is_ok_and(|t| t.is_file())
            {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let _ = std::fs::remove_dir(&dir);
}

#[cfg(target_os = "macos")]
mod driver {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{Duration, Instant};

    use trace_commons_contributor_platform::native_pty::PtyChild;

    use super::super::LoginSpec;
    use super::{COLUMNS, ROWS, ScreenState, classify, clean_probe_artifacts, project_dir_name};
    use crate::source::claude_code::CLAUDE_USAGE_PROBE_DIR;

    const TOTAL: Duration = Duration::from_secs(45);
    const STARTUP_QUIET: Duration = Duration::from_millis(2500);
    const IDLE: Duration = Duration::from_millis(250);
    const MAX_OUTPUT: usize = 4 * 1024 * 1024;
    const CURSOR_QUERY: &[u8] = b"\x1b[6n";
    /// Process-local settings for the probe only; saved settings are not
    /// changed. Hooks off, and Remote Control not started.
    const SETTINGS: &str = r#"{"remoteControlAtStartup":false,"disableAllHooks":true}"#;
    /// Inherited variables passed through. Everything else, including any
    /// API key or token in the daemon's environment, is withheld.
    const PASSED: &[&str] = &[
        "HOME", "PATH", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE",
    ];

    pub(in super::super) struct ClaudeCli;

    impl super::CliProbe for ClaudeCli {
        fn usage_screen(&self, login: &LoginSpec) -> Result<String, &'static str> {
            let executable = crate::managed::profiles::find_native(crate::managed::ToolId::Claude)
                .map_err(|_| "native_cli_unavailable")?;
            let workdir = Workdir::create()?;
            let project = project_dir_name(&workdir.path).ok_or("cli_unavailable")?;
            let session = uuid::Uuid::new_v4().to_string();
            let result = run(&executable, login, &workdir.path, &session);
            clean_probe_artifacts(&login.root, &project, &session);
            drop(workdir);
            result
        }
    }

    /// The probe's private working directory, at a fixed name under the
    /// per-user temporary directory so Claude Code keeps one trust entry for
    /// it, recreated empty with mode 0700 and removed afterwards.
    struct Workdir {
        path: PathBuf,
    }

    impl Workdir {
        fn create() -> Result<Self, &'static str> {
            use std::os::unix::fs::DirBuilderExt;
            let base =
                std::fs::canonicalize(std::env::temp_dir()).map_err(|_| "cli_unavailable")?;
            let path = base.join(CLAUDE_USAGE_PROBE_DIR);
            if let Ok(meta) = std::fs::symlink_metadata(&path) {
                if !meta.is_dir() {
                    return Err("cli_unavailable");
                }
                std::fs::remove_dir_all(&path).map_err(|_| "cli_unavailable")?;
            }
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .map_err(|_| "cli_unavailable")?;
            Ok(Self { path })
        }
    }

    impl Drop for Workdir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn command(executable: &Path, login: &LoginSpec, workdir: &Path, session: &str) -> Command {
        let mut command = Command::new(executable);
        command
            .args(["--tools", "", "--strict-mcp-config", "--settings", SETTINGS])
            .args(["--session-id", session])
            .current_dir(workdir)
            .env_clear();
        for name in PASSED {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .env("PWD", workdir)
            .env("TERM", "xterm-256color")
            .env("DISABLE_AUTOUPDATER", "1")
            // No transcript or prompt history for this session.
            .env("CLAUDE_CODE_SKIP_PROMPT_HISTORY", "1");
        // The same profile the OAuth reader used: its config directory and
        // secure-storage override, so Claude Code reads its own credential.
        if let Some(dir) = login.config_dir.as_deref().filter(|d| !d.is_empty()) {
            command.env("CLAUDE_CONFIG_DIR", dir);
        }
        if let Some(dir) = login.secure_storage_dir.as_deref() {
            command.env("CLAUDE_SECURESTORAGE_CONFIG_DIR", dir);
        }
        command
    }

    fn run(
        executable: &Path,
        login: &LoginSpec,
        workdir: &Path,
        session: &str,
    ) -> Result<String, &'static str> {
        let mut child = PtyChild::spawn(
            &mut command(executable, login, workdir, session),
            COLUMNS,
            ROWS,
        )
        .map_err(|_| "cli_unavailable")?;
        let result = drive(&mut child);
        child.terminate();
        result
    }

    fn drive(child: &mut PtyChild) -> Result<String, &'static str> {
        let started = Instant::now();
        let deadline = started + TOTAL;
        let mut output: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 16 * 1024];
        let mut last_output = started;
        let mut ready_at = started + STARTUP_QUIET;
        let mut sent = false;
        let mut last_enter = started;
        let mut enters = 0;
        let mut trust_keys = 6;
        let mut settled_since: Option<Instant> = None;
        while Instant::now() < deadline {
            match child.read_some(&mut chunk, Duration::from_millis(100)) {
                Ok(0) => {}
                Ok(read) => {
                    let tail = output.len().saturating_sub(CURSOR_QUERY.len() - 1);
                    output.extend_from_slice(&chunk[..read]);
                    if output.len() > MAX_OUTPUT {
                        return Err("cli_output_too_large");
                    }
                    if output[tail..]
                        .windows(CURSOR_QUERY.len())
                        .any(|w| w == CURSOR_QUERY)
                    {
                        // Answer the cursor position query or the TUI waits.
                        child.write_all(b"\x1b[1;1R")?;
                    }
                    last_output = Instant::now();
                    settled_since = None;
                }
                Err(_) => return Err("cli_exited"),
            }
            let now = Instant::now();
            if now.duration_since(last_output) < IDLE {
                continue;
            }
            let screen = super::render(&output);
            match classify(&screen) {
                ScreenState::NeedsSignIn => return Err("cli_signed_out"),
                ScreenState::Unexpected => return Err("cli_unexpected_dialog"),
                ScreenState::UsageFailed if sent => return Err("cli_failed"),
                ScreenState::NoQuota if sent => return Err("cli_no_quota"),
                ScreenState::Trust(keys) => {
                    if let Some(keys) = keys.filter(|_| trust_keys > 0) {
                        trust_keys -= 1;
                        child.write_all(keys.as_bytes())?;
                        ready_at = now + Duration::from_secs(2);
                    }
                }
                ScreenState::Usage if sent => {
                    // Let the panel finish drawing its other rows.
                    let since = *settled_since.get_or_insert(now);
                    if now.duration_since(since) >= Duration::from_millis(800) {
                        return Ok(screen);
                    }
                }
                _ if !sent && now >= ready_at => {
                    child.write_all(b"/usage")?;
                    std::thread::sleep(Duration::from_millis(200));
                    child.write_all(b"\r")?;
                    sent = true;
                    last_enter = Instant::now();
                }
                _ if sent
                    && enters < 3
                    && now.duration_since(last_enter) > Duration::from_millis(1500) =>
                {
                    // The command palette can need a second Enter.
                    child.write_all(b"\r")?;
                    enters += 1;
                    last_enter = now;
                }
                _ => {}
            }
            if !child.running() {
                return Err("cli_exited");
            }
        }
        Err("cli_timeout")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn script(dir: &Path, body: &str) -> PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join("claude");
            std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            path
        }
        /// `run` against a freshly written script. A parallel test's fork
        /// can briefly hold the script's write descriptor, which makes exec
        /// fail with ETXTBSY; that is a fixture race, so retry the spawn.
        fn run_script(exe: &Path, login: &LoginSpec, work: &Path) -> Result<String, &'static str> {
            for _ in 0..20 {
                match run(exe, login, work, "s") {
                    Err("cli_unavailable") => std::thread::sleep(Duration::from_millis(50)),
                    other => return other,
                }
            }
            run(exe, login, work, "s")
        }
        fn login(root: &Path) -> LoginSpec {
            LoginSpec {
                provider: super::super::super::Provider::Claude,
                root: root.into(),
                id: "claude-fixture".into(),
                label: "Claude".into(),
                default_login: false,
                native_executable: None,
                config_dir: Some(root.to_string_lossy().into()),
                secure_storage_dir: None,
                metadata_revision: None,
                native_revision: None,
            }
        }

        /// A stand-in CLI, never the real one: a trust dialog with the
        /// marker on Yes, then a prompt that answers `/usage`.
        #[test]
        fn the_driver_accepts_its_own_trust_dialog_and_reads_usage() {
            let dir = tempfile::tempdir().unwrap();
            let exe = script(
                dir.path(),
                "printf 'Do you trust the files in this folder?\\r\\n\\r\\n \\342\\235\\257 1. Yes, I trust this folder\\r\\n   2. No, exit\\r\\n'\n\
                 read ans\n\
                 printf '\\033[2J\\033[H> '\n\
                 read cmd\n\
                 [ \"$cmd\" = /usage ] && printf '\\r\\nCurrent session\\r\\n 23%% used\\r\\n'\n\
                 sleep 30\n",
            );
            let work = tempfile::tempdir().unwrap();
            let screen = run_script(&exe, &login(dir.path()), work.path()).unwrap();
            assert!(screen.contains("23% used"), "{screen}");
        }

        #[test]
        fn a_sign_in_screen_ends_the_probe_without_answering() {
            let dir = tempfile::tempdir().unwrap();
            let marker = dir.path().join("answered");
            let exe = script(
                dir.path(),
                &format!(
                    "printf 'Select login method:\\r\\n \\342\\235\\257 1. Claude account\\r\\n'\n\
                     read ans && touch '{}'\n\
                     sleep 30\n",
                    marker.display()
                ),
            );
            let work = tempfile::tempdir().unwrap();
            assert_eq!(
                run_script(&exe, &login(dir.path()), work.path()).unwrap_err(),
                "cli_signed_out"
            );
            std::thread::sleep(Duration::from_millis(200));
            assert!(!marker.exists());
        }

        #[test]
        fn the_cli_gets_no_inherited_secrets_and_the_same_profile() {
            let dir = tempfile::tempdir().unwrap();
            let mut spec = login(dir.path());
            spec.secure_storage_dir = Some(String::new());
            let command = command(Path::new("/bin/true"), &spec, dir.path(), "s");
            let mut set = Vec::new();
            for (name, value) in command.get_envs() {
                let name = name.to_string_lossy().into_owned();
                assert!(value.is_some(), "{name} removed rather than cleared");
                set.push(name);
            }
            for name in &set {
                assert!(
                    PASSED.contains(&name.as_str())
                        || [
                            "PWD",
                            "TERM",
                            "DISABLE_AUTOUPDATER",
                            "CLAUDE_CODE_SKIP_PROMPT_HISTORY",
                            "CLAUDE_CONFIG_DIR",
                            "CLAUDE_SECURESTORAGE_CONFIG_DIR",
                        ]
                        .contains(&name.as_str()),
                    "unexpected variable {name}"
                );
            }
            assert!(set.iter().any(|n| n == "CLAUDE_CONFIG_DIR"));
            assert!(set.iter().any(|n| n == "CLAUDE_SECURESTORAGE_CONFIG_DIR"));
            let args: Vec<_> = command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            assert_eq!(
                args,
                [
                    "--tools",
                    "",
                    "--strict-mcp-config",
                    "--settings",
                    SETTINGS,
                    "--session-id",
                    "s"
                ]
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Europe::Vienna;

    fn now() -> DateTime<chrono_tz::Tz> {
        Vienna.with_ymd_and_hms(2026, 10, 28, 9, 30, 0).unwrap()
    }

    /// A `/usage` panel in the shape Claude Code draws it, written for these
    /// tests.
    const PANEL: &str = "\
 Settings:  Status   Config   Usage  (tab to cycle)

 Current session
 ███████████▌                                       23% used
 Resets 2pm (Europe/Vienna)

 Current week (all models)
 ███▌                                               7% used
 Resets Nov 3 at 9:59am (Europe/Vienna)

 Current week (Sonnet only)
                                                    0% used
 Resets Nov 3 at 9:59am (Europe/Vienna)

 Current week (Fable)
 ████████████████████                               41% used
 Resets Nov 1, 2026, 6pm (Europe/Vienna)

 What's contributing to your limits usage?
 Bash 88% used of tool calls
 Current week (Leaked) 99% used
";

    #[test]
    fn a_usage_panel_gives_session_weekly_and_model_windows() {
        let windows = parse_usage(PANEL, now(), Some("Europe/Vienna")).unwrap();
        let summary: Vec<_> = windows
            .iter()
            .map(|w| {
                (
                    w.lane.as_str(),
                    w.label.as_deref(),
                    w.used_permille,
                    w.duration_mins,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("session", None, Some(230), Some(300)),
                ("weekly", None, Some(70), Some(10080)),
                ("model", Some("Sonnet"), Some(0), Some(10080)),
                ("model", Some("Fable"), Some(410), Some(10080)),
            ]
        );
        let at = |y, m, d, h, min| {
            Some(
                Vienna
                    .with_ymd_and_hms(y, m, d, h, min, 0)
                    .unwrap()
                    .with_timezone(&Utc),
            )
        };
        assert_eq!(windows[0].resets_at, at(2026, 10, 28, 14, 0));
        assert_eq!(windows[1].resets_at, at(2026, 11, 3, 9, 59));
        assert_eq!(windows[3].resets_at, at(2026, 11, 1, 18, 0));
    }

    #[test]
    fn insights_rows_never_become_quota() {
        let windows = parse_usage(PANEL, now(), None).unwrap();
        assert!(windows.iter().all(|w| w.label.as_deref() != Some("Leaked")));
    }

    #[test]
    fn a_reset_in_another_zone_or_without_a_time_stays_unknown() {
        let panel = "Current session\n 40% used (Resets 11am (America/New_York))\nCurrent week (all models)\n 10% left (Resets Nov 27)\n";
        let windows = parse_usage(panel, now(), Some("Europe/Vienna")).unwrap();
        assert_eq!(windows[0].used_permille, Some(400));
        assert_eq!(windows[0].resets_at, None);
        assert_eq!(windows[1].used_permille, Some(900));
        assert_eq!(windows[1].resets_at, None);
        // Unknown local zone: any stated zone stays unknown.
        assert_eq!(parse_reset("2pm (Europe/Vienna)", now(), None), None);
        // No zone stated: local time, next occurrence.
        assert_eq!(
            parse_reset("8am", now(), None),
            Some(
                Vienna
                    .with_ymd_and_hms(2026, 10, 29, 8, 0, 0)
                    .unwrap()
                    .with_timezone(&Utc)
            )
        );
        assert_eq!(parse_reset("Nov 30 at 1pm", now(), None), None);
    }

    #[test]
    fn a_panel_without_a_session_figure_is_not_a_reading() {
        assert_eq!(
            parse_usage("Current session\n Loading usage data…\n", now(), None).unwrap_err(),
            "cli_unparsed"
        );
        assert_eq!(parse_usage("", now(), None).unwrap_err(), "cli_unparsed");
        // A bare percentage is ambiguous: neither used nor left.
        assert!(parse_usage("Current session\n 23%\n", now(), None).is_err());
        // A status-bar context meter is not quota.
        assert!(parse_usage("Current session\n Opus | 12% used\n", now(), None).is_err());
    }

    #[test]
    fn the_latest_redrawn_panel_wins() {
        let screen = "Current session\n 5% used\nCurrent session\n 23% used\n";
        let windows = parse_usage(screen, now(), None).unwrap();
        assert_eq!(windows[0].used_permille, Some(230));
    }

    #[test]
    fn the_terminal_replay_follows_cursor_moves_and_erases() {
        // Draw a stale figure, erase the line and redraw it.
        let bytes = b"\x1b[?25l\x1b]0;title\x07Current session\r\n 5% used\x1b[2K\r 23% used\r\n\x1b[31mdone\x1b[0m";
        assert_eq!(render(bytes), "Current session\n 23% used\ndone");
        // Relative moves: up a row and overwrite it.
        assert_eq!(render(b"a\r\nb\x1b[1A\rc"), "c\nb");
        // Absolute positioning and erase-to-end.
        assert_eq!(render(b"abcdef\x1b[1;3H\x1b[K"), "ab");
        // Wide characters take two cells; block bars one.
        assert_eq!(render("中a\r\x1b[2Cb".as_bytes()), "中b");
        assert_eq!(render("█▌ 3%".as_bytes()), "█▌ 3%");
        // Oversized parameters are bounded, not panics.
        assert_eq!(
            render(b"x\x1b[99999999999A\x1b[99999;99999Hy")
                .lines()
                .count(),
            200
        );
    }

    #[test]
    fn screens_are_classified_without_ever_answering_a_sign_in() {
        assert_eq!(classify(PANEL), ScreenState::Usage);
        for screen in [
            "Select login method:\n ❯ 1. Claude account with subscription",
            "Invalid API key · Please run /login",
            "Choose the text style that looks best with your terminal",
        ] {
            assert_eq!(classify(screen), ScreenState::NeedsSignIn, "{screen}");
        }
        assert_eq!(
            classify("Current session\n Failed to load usage data"),
            ScreenState::UsageFailed
        );
        assert_eq!(
            classify("You are currently using your subscription to power your Claude Code usage"),
            ScreenState::NoQuota
        );
        assert_eq!(classify("> "), ScreenState::Other);
        assert_eq!(classify("❯ /usage"), ScreenState::Other);
        // An unknown chooser or a settings error is never answered.
        assert_eq!(
            classify("Pick a model\n ❯ 1. Default\n   2. Other"),
            ScreenState::Unexpected
        );
        assert_eq!(
            classify("Settings Error\n Fix with Claude / Exit / Continue"),
            ScreenState::Unexpected
        );
    }

    #[test]
    fn the_trust_dialog_is_answered_only_toward_yes_with_a_visible_marker() {
        let on_yes = "Do you trust the files in this folder?\n\n ❯ 1. Yes, I trust this folder\n   2. No, exit\n";
        assert_eq!(classify(on_yes), ScreenState::Trust(Some("\r")));
        let on_no = "Do you trust the files in this folder?\n\n   1. Yes, I trust this folder\n ❯ 2. No, exit\n";
        assert_eq!(classify(on_no), ScreenState::Trust(Some("\u{1b}[A")));
        let no_marker = "Do you trust the files in this folder?\n\n   1. Yes, I trust this folder\n   2. No, exit\n";
        assert_eq!(classify(no_marker), ScreenState::Trust(None));
    }

    #[test]
    fn the_probe_directory_name_matches_the_discovery_exclusion() {
        let path = std::path::Path::new("/private/var/folders/ab/xyz/T/tc-claude-usage-probe");
        let name = project_dir_name(path).unwrap();
        assert_eq!(name, "-private-var-folders-ab-xyz-T-tc-claude-usage-probe");
        assert!(name.ends_with(&format!(
            "-{}",
            crate::source::claude_code::CLAUDE_USAGE_PROBE_DIR
        )));
        let long = std::path::PathBuf::from(format!("/{}/tc-claude-usage-probe", "a".repeat(200)));
        assert_eq!(project_dir_name(&long), None);
    }

    #[test]
    fn cleanup_removes_only_the_probes_own_artifacts() {
        let root = tempfile::tempdir().unwrap();
        let project = "-tmp-tc-claude-usage-probe";
        let probe = root.path().join("projects").join(project);
        let session = "22222222-2222-2222-2222-222222222222";
        std::fs::create_dir_all(probe.join(session).join("subagents")).unwrap();
        std::fs::write(probe.join(format!("{session}.jsonl")), "{}").unwrap();
        std::fs::write(probe.join("other.jsonl"), "{}").unwrap();
        let mine = root.path().join("projects").join("-Users-me-code-app");
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::write(mine.join("keep.jsonl"), "{}").unwrap();
        clean_probe_artifacts(root.path(), project, session);
        assert!(!probe.exists());
        assert!(mine.join("keep.jsonl").exists());
        // Nothing there: a no-op.
        clean_probe_artifacts(root.path(), project, session);
    }
}
