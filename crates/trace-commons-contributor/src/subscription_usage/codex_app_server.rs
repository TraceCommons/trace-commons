//! Codex usage from the contributor's own Codex, through `codex app-server`
//! (design 2.2): the documented interface, which answers from Codex's own
//! sign-in. This module never reads `auth.json`, never sees a token, never
//! touches the Keychain, and never sends `refreshToken`.
//!
//! The launch is exactly `<absolute codex> -s read-only -a never app-server`,
//! with the path from the daemon's own executable discovery
//! (`managed::profiles::find_native`), never a bare `codex`. The child gets
//! an environment cleared down to `HOME`, a fixed `PATH`, `LANG` and
//! `TMPDIR`, a daemon-owned empty working directory, and no stderr.
//!
//! The exchange is one JSON object per line: `initialize`, the
//! `initialized` notification, `account/read` with empty params, and
//! `account/rateLimits/read`. Then stdin closes, and the child gets 2 s to
//! leave, SIGTERM, 2 s more, SIGKILL, and is reaped. Budgets: 10 s for
//! `initialize`, 5 s per request, 20 s for the whole probe; a stdout line
//! over 1 MiB ends it as `unreadable`. Notifications are read and dropped.
//!
//! A recorded exchange with codex-cli 0.153.4 (`tests/fixtures/
//! subscription_usage/`) settled two things the design left open: no
//! `jsonrpc` field is needed, and `account/read` can answer `account: null`
//! with `requiresOpenaiAuth: false` while the usage read succeeds. So the
//! usage read answering is what makes a reading observed; the account only
//! names the reason when it does not.
//!
//! What is kept is parsed into narrow structs with no field for anything
//! else: never the email, the account id, or a reset credit's id, title or
//! description.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::de::{DeserializeOwned, IgnoredAny};

use super::UsageSource;
use super::snapshot::{Credits, Lane, Plan, Reading, Reason, Window};

/// The longest stdout line read, newline excluded.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;
/// The `PATH` the child gets, before the directory holding `codex` itself
/// (where an npm install keeps the `node` its launcher needs).
pub const CHILD_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
/// The furthest ahead a reset time is believed. Further is dropped, not
/// clamped.
pub const MAX_RESET_AHEAD: chrono::Duration = chrono::Duration::days(31);
/// The window lengths Codex reports for its five-hour and weekly limits.
const SESSION_MINS: i64 = 300;
const WEEKLY_MINS: i64 = 7 * 24 * 60;

/// How long each step may take.
#[derive(Debug, Clone, Copy)]
pub struct Budgets {
    pub initialize: Duration,
    pub request: Duration,
    pub total: Duration,
    /// After stdin closes, before SIGTERM.
    pub close_wait: Duration,
    /// After SIGTERM, before SIGKILL.
    pub term_wait: Duration,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            initialize: Duration::from_secs(10),
            request: Duration::from_secs(5),
            total: Duration::from_secs(20),
            close_wait: Duration::from_secs(2),
            term_wait: Duration::from_secs(2),
        }
    }
}

/// The Codex app-server source.
pub struct CodexAppServer {
    locate: Box<dyn Fn() -> Option<PathBuf> + Send + Sync>,
    running: Box<dyn Fn() -> bool + Send + Sync>,
    work_dir: PathBuf,
    budgets: Budgets,
}

impl CodexAppServer {
    /// The daemon's source: `codex` from its executable discovery, and the
    /// working directory `subscription-usage` under its state directory.
    #[must_use]
    pub fn for_daemon(state_dir: &Path) -> Self {
        Self {
            locate: Box::new(|| {
                crate::managed::profiles::find_native(crate::managed::ToolId::Codex).ok()
            }),
            running: Box::new(codex_is_running),
            work_dir: state_dir.join("subscription-usage"),
            budgets: Budgets::default(),
        }
    }

    /// A source with every input supplied, for tests.
    #[must_use]
    pub fn with(
        locate: Box<dyn Fn() -> Option<PathBuf> + Send + Sync>,
        running: Box<dyn Fn() -> bool + Send + Sync>,
        work_dir: PathBuf,
        budgets: Budgets,
    ) -> Self {
        Self {
            locate,
            running,
            work_dir,
            budgets,
        }
    }

    fn codex(&self) -> Result<PathBuf, Reason> {
        (self.locate)()
            .filter(|path| path.is_absolute())
            .ok_or(Reason::CodexNotFound)
    }
}

impl UsageSource for CodexAppServer {
    fn ready(&self) -> Result<(), Reason> {
        self.codex()?;
        // Never a second Codex beside one the contributor is using: it would
        // share that one's sign-in files while they are in use.
        if (self.running)() {
            return Err(Reason::CodexRunning);
        }
        Ok(())
    }

    fn probe(&self, now: DateTime<Utc>) -> Result<Reading, Reason> {
        let codex = self.codex()?;
        let mut child = AppServer::launch(&codex, &self.work_dir, self.budgets)?;
        let outcome = child.exchange(now);
        child.close();
        outcome
    }
}

/// Whether any process named `codex` (the CLI, or the Codex app) is running.
fn codex_is_running() -> bool {
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );
    system.processes().values().any(|process| {
        process
            .name()
            .to_str()
            .is_some_and(|name| name.eq_ignore_ascii_case("codex"))
    })
}

/// One line from the child's stdout, as the reader thread hands it over.
enum Line {
    Text(Vec<u8>),
    TooLong,
    Closed,
}

/// Why one request did not answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fail {
    /// Codex answered with an error object.
    Refused,
    Timeout,
    Unreadable,
    /// Codex closed stdout or stopped reading.
    Gone,
}

impl Fail {
    const fn reason(self) -> Reason {
        match self {
            Self::Refused | Self::Gone => Reason::Failed,
            Self::Timeout => Reason::Timeout,
            Self::Unreadable => Reason::Unreadable,
        }
    }
}

/// A launched `codex app-server`.
struct AppServer {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Line>,
    deadline: Instant,
    budgets: Budgets,
}

impl AppServer {
    fn launch(codex: &Path, work_dir: &Path, budgets: Budgets) -> Result<Self, Reason> {
        private_dir(work_dir).map_err(|_| Reason::Failed)?;
        let mut command = Command::new(codex);
        command
            .args(["-s", "read-only", "-a", "never", "app-server"])
            .current_dir(work_dir)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(home) = dirs::home_dir() {
            command.env("HOME", home);
        }
        let mut path = CHILD_PATH.to_string();
        if let Some(dir) = codex.parent().and_then(Path::to_str)
            && !dir.contains(':')
        {
            path = format!("{path}:{dir}");
        }
        command.env("PATH", path);
        for key in ["LANG", "TMPDIR"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        let mut child = command.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                Reason::Blocked
            } else {
                Reason::Failed
            }
        })?;
        let stdin = child.stdin.take();
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Reason::Failed);
        };
        let (tx, lines) = std::sync::mpsc::channel();
        // Not joined: it ends when stdout closes, which a child holding it
        // open past its own exit could delay, and nothing waits on it.
        let reader = std::thread::Builder::new()
            .name("codex-app-server-stdout".to_string())
            .spawn(move || read_lines(stdout, &tx));
        let mut this = Self {
            child,
            stdin,
            lines,
            deadline: Instant::now() + budgets.total,
            budgets,
        };
        if reader.is_err() {
            this.close();
            return Err(Reason::Failed);
        }
        Ok(this)
    }

    fn exchange(&mut self, now: DateTime<Utc>) -> Result<Reading, Reason> {
        let initialize = serde_json::json!({
            "id": 1,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "trace_commons",
                    "title": null,
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": null,
            },
        });
        self.request::<IgnoredAny>(&initialize, 1, self.budgets.initialize)
            .map_err(Fail::reason)?;
        self.send(&serde_json::json!({ "method": "initialized" }))
            .map_err(Fail::reason)?;
        // `refreshToken` is never sent: Codex renews its own sign-in only
        // when it needs to, never because we asked.
        let account = match self.request::<AccountRead>(
            &serde_json::json!({ "id": 2, "method": "account/read", "params": {} }),
            2,
            self.budgets.request,
        ) {
            Ok(account) => Some(account),
            Err(Fail::Refused) => None,
            Err(fail) => return Err(fail.reason()),
        };
        let limits = self.request::<RateLimitsRead>(
            &serde_json::json!({ "id": 3, "method": "account/rateLimits/read" }),
            3,
            self.budgets.request,
        );
        conclude(account.as_ref(), limits, now)
    }

    fn send(&mut self, message: &serde_json::Value) -> Result<(), Fail> {
        let stdin = self.stdin.as_mut().ok_or(Fail::Gone)?;
        let mut line = serde_json::to_vec(message).map_err(|_| Fail::Gone)?;
        line.push(b'\n');
        stdin.write_all(&line).map_err(|_| Fail::Gone)?;
        stdin.flush().map_err(|_| Fail::Gone)
    }

    fn request<T: DeserializeOwned>(
        &mut self,
        message: &serde_json::Value,
        id: u64,
        budget: Duration,
    ) -> Result<T, Fail> {
        self.send(message)?;
        let deadline = self.deadline.min(Instant::now() + budget);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(Line::Text(line)) => {
                    if let Some(answer) = response::<T>(&line, id) {
                        return answer;
                    }
                }
                Ok(Line::TooLong) => return Err(Fail::Unreadable),
                Ok(Line::Closed) | Err(RecvTimeoutError::Disconnected) => return Err(Fail::Gone),
                Err(RecvTimeoutError::Timeout) => return Err(Fail::Timeout),
            }
        }
    }

    /// Close stdin, wait, SIGTERM, wait, SIGKILL, reap.
    fn close(&mut self) {
        drop(self.stdin.take());
        if exited_within(&mut self.child, self.budgets.close_wait) {
            return;
        }
        terminate(self.child.id());
        if exited_within(&mut self.child, self.budgets.term_wait) {
            return;
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for AppServer {
    /// A probe that panicked still leaves no child behind.
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Create `dir` readable by this user only, if it is not there.
fn private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

fn exited_within(child: &mut Child, wait: Duration) -> bool {
    let until = Instant::now() + wait;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(20)),
            // Not yet, or not answerable: the caller escalates.
            _ => return false,
        }
    }
}

/// SIGTERM to the child. Not yet reaped, so its pid is still its own.
fn terminate(pid: u32) {
    let pid = sysinfo::Pid::from_u32(pid);
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    if let Some(process) = system.process(pid) {
        let _ = process.kill_with(sysinfo::Signal::Term);
    }
}

/// Hand each stdout line to `tx`, ending at the first line over the cap or
/// when stdout closes.
fn read_lines(stdout: impl Read, tx: &Sender<Line>) {
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = (&mut reader)
            .take(MAX_LINE_BYTES as u64 + 1)
            .read_until(b'\n', &mut line);
        match read {
            Ok(0) | Err(_) => {
                let _ = tx.send(Line::Closed);
                return;
            }
            Ok(_) => {
                if line.last() == Some(&b'\n') {
                    line.pop();
                } else if line.len() > MAX_LINE_BYTES {
                    let _ = tx.send(Line::TooLong);
                    return;
                }
                if tx.send(Line::Text(std::mem::take(&mut line))).is_err() {
                    return;
                }
            }
        }
    }
}

/// Just enough of a line to tell whether it answers request `id`.
#[derive(Deserialize)]
struct Head {
    #[serde(default)]
    id: Option<serde_json::Value>,
    #[serde(default)]
    method: Option<IgnoredAny>,
    #[serde(default)]
    error: Option<IgnoredAny>,
}

#[derive(Deserialize)]
struct Body<T> {
    result: T,
}

/// `None` when the line is not the answer to request `id`: a notification,
/// a request from Codex, another id, or not JSON at all.
fn response<T: DeserializeOwned>(line: &[u8], id: u64) -> Option<Result<T, Fail>> {
    let head: Head = serde_json::from_slice(line).ok()?;
    if head.method.is_some() || head.id.as_ref().and_then(serde_json::Value::as_u64) != Some(id) {
        return None;
    }
    if head.error.is_some() {
        return Some(Err(Fail::Refused));
    }
    Some(
        serde_json::from_slice::<Body<T>>(line)
            .map(|body| body.result)
            .map_err(|_| Fail::Unreadable),
    )
}

/// `account/read`: the sign-in's kind and plan, and nothing else of it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountRead {
    #[serde(default)]
    account: Option<Account>,
    #[serde(default)]
    requires_openai_auth: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Account {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    plan_type: Option<String>,
}

/// `account/rateLimits/read`. No field for `accountId` or
/// `rateLimitUpsell`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RateLimitsRead {
    rate_limits: Snapshot,
    #[serde(default)]
    rate_limits_by_limit_id: Option<BTreeMap<String, Snapshot>>,
    #[serde(default)]
    rate_limit_reset_credits: Option<ResetCredits>,
    /// Named in the design's schema; absent from 0.153.4's. Read when sent.
    #[serde(default)]
    ordinary_usage_allowed: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    #[serde(default)]
    limit_id: Option<String>,
    #[serde(default)]
    limit_name: Option<String>,
    #[serde(default)]
    primary: Option<WireWindow>,
    #[serde(default)]
    secondary: Option<WireWindow>,
    #[serde(default)]
    credits: Option<WireCredits>,
    #[serde(default)]
    plan_type: Option<String>,
    /// `None` when the field is absent; `Some(false)` when it is `null`
    /// (no limit reached), `Some(true)` for any reached type.
    #[serde(default, deserialize_with = "present_and_set")]
    rate_limit_reached_type: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireWindow {
    #[serde(default)]
    used_percent: Option<f64>,
    #[serde(default)]
    window_duration_mins: Option<i64>,
    #[serde(default)]
    resets_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireCredits {
    #[serde(default)]
    has_credits: Option<bool>,
    #[serde(default)]
    unlimited: Option<bool>,
    #[serde(default)]
    balance: Option<String>,
}

/// Only the count: no credit's id, title or description is read.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResetCredits {
    #[serde(default)]
    available_count: Option<i64>,
}

fn present_and_set<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    Option::<IgnoredAny>::deserialize(d).map(|value| Some(value.is_some()))
}

/// The probe's answer from what the two reads returned.
fn conclude(
    account: Option<&AccountRead>,
    limits: Result<RateLimitsRead, Fail>,
    now: DateTime<Utc>,
) -> Result<Reading, Reason> {
    match limits {
        Ok(limits) => Ok(reading(limits, account, now)),
        Err(Fail::Refused) => Err(match account {
            Some(AccountRead {
                account: Some(account),
                ..
            }) if account.kind != "chatgpt" => Reason::NotSubscription,
            Some(AccountRead {
                account: None,
                requires_openai_auth: true,
            }) => Reason::SignedOut,
            _ => Reason::Failed,
        }),
        Err(fail) => Err(fail.reason()),
    }
}

fn reading(limits: RateLimitsRead, account: Option<&AccountRead>, now: DateTime<Utc>) -> Reading {
    let main = &limits.rate_limits;
    let plan = main
        .plan_type
        .as_deref()
        .or_else(|| account?.account.as_ref()?.plan_type.as_deref())
        .map_or(Plan::Unknown, Plan::from_codex);
    let mut windows: Vec<Window> = [
        (main.primary.as_ref(), Lane::Session),
        (main.secondary.as_ref(), Lane::Weekly),
    ]
    .into_iter()
    .filter_map(|(window, fallback)| window_from(window?, lane_for(window?, fallback), None, now))
    .collect();
    let main_id = main.limit_id.as_deref().unwrap_or("codex");
    for (id, snapshot) in limits.rate_limits_by_limit_id.iter().flatten() {
        if id == main_id {
            continue;
        }
        let label = snapshot.limit_name.as_deref().and_then(checked_label);
        for window in [snapshot.primary.as_ref(), snapshot.secondary.as_ref()]
            .into_iter()
            .flatten()
        {
            windows.extend(window_from(window, Lane::Model, label.clone(), now));
        }
    }
    let limit_reached = main
        .rate_limit_reached_type
        .or(limits.ordinary_usage_allowed.map(|allowed| !allowed));
    Reading {
        at: now,
        plan,
        limit_reached,
        windows,
        credits: main.credits.as_ref().and_then(credits_from),
        reset_credits: limits
            .rate_limit_reset_credits
            .and_then(|credits| u32::try_from(credits.available_count?).ok()),
    }
}

/// 300 minutes is the session window and 10080 the weekly one, wherever
/// Codex reports them (a weekly-only plan reports it as primary). Any other
/// length keeps its slot's lane.
fn lane_for(window: &WireWindow, fallback: Lane) -> Lane {
    match window.window_duration_mins {
        Some(SESSION_MINS) => Lane::Session,
        Some(WEEKLY_MINS) => Lane::Weekly,
        _ => fallback,
    }
}

/// `None` when the percentage is missing or outside 0..=100: that window is
/// unknown, so it is absent, never drawn as a number.
fn window_from(
    window: &WireWindow,
    lane: Lane,
    label: Option<String>,
    now: DateTime<Utc>,
) -> Option<Window> {
    let percent = window.used_percent?;
    if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
        return None;
    }
    // In range, so the cast cannot truncate.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let used_permille = (percent * 10.0).round() as u16;
    Some(Window {
        lane,
        label,
        duration_mins: window
            .window_duration_mins
            .filter(|mins| *mins > 0)
            .and_then(|mins| u32::try_from(mins).ok()),
        used_permille,
        resets_at: window
            .resets_at
            .and_then(|secs| DateTime::from_timestamp(secs, 0))
            .filter(|at| *at > now && *at <= now + MAX_RESET_AHEAD),
    })
}

/// A model limit's name, only when it is short plain text.
fn checked_label(name: &str) -> Option<String> {
    let ok = (1..=40).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b' ' | b'.' | b'_' | b'-'));
    ok.then(|| name.to_string())
}

fn credits_from(credits: &WireCredits) -> Option<Credits> {
    Some(Credits {
        has_credits: credits.has_credits?,
        unlimited: credits.unlimited?,
        balance: credits
            .balance
            .as_deref()
            .filter(|balance| is_decimal(balance))
            .map(str::to_string),
    })
}

/// A non-negative decimal: digits, optionally a point and more digits.
fn is_decimal(text: &str) -> bool {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, "0"));
    text.len() <= 32
        && !whole.is_empty()
        && !fraction.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && fraction.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const FIXTURE: &str =
        include_str!("../../tests/fixtures/subscription_usage/codex-app-server-0.153.4.json");

    /// The day of the recording: the fixture's weekly reset is a week out.
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 26, 12, 0, 0).unwrap()
    }

    /// The fixture's answers, as the lines Codex printed.
    fn fixture_lines() -> Vec<Vec<u8>> {
        let answers: Vec<serde_json::Value> = serde_json::from_str(FIXTURE).unwrap();
        answers
            .iter()
            .map(|answer| serde_json::to_vec(answer).unwrap())
            .collect()
    }

    fn limits_with(result: serde_json::Value) -> RateLimitsRead {
        let line = serde_json::to_vec(&serde_json::json!({"id": 3, "result": result})).unwrap();
        response::<RateLimitsRead>(&line, 3).unwrap().unwrap()
    }

    fn window(used: serde_json::Value, mins: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"usedPercent": used, "windowDurationMins": mins, "resetsAt": null})
    }

    #[test]
    fn the_recorded_exchange_reads_as_one_observed_weekly_window() {
        let lines = fixture_lines();
        assert!(response::<IgnoredAny>(&lines[0], 1).unwrap().is_ok());
        let account = response::<AccountRead>(&lines[1], 2).unwrap().unwrap();
        assert!(account.account.is_none());
        assert!(!account.requires_openai_auth);
        let limits = response::<RateLimitsRead>(&lines[2], 3).unwrap();
        let reading = conclude(Some(&account), limits, now()).unwrap();
        assert_eq!(
            reading,
            Reading {
                at: now(),
                plan: Plan::Business,
                limit_reached: Some(false),
                windows: vec![Window {
                    lane: Lane::Weekly,
                    label: None,
                    duration_mins: Some(10_080),
                    used_permille: 0,
                    resets_at: DateTime::from_timestamp(1_793_617_253, 0),
                }],
                credits: Some(Credits {
                    has_credits: false,
                    unlimited: false,
                    balance: None,
                }),
                reset_credits: Some(2),
            }
        );
        // The main limit repeated under `rateLimitsByLimitId` is not a model
        // window, and nothing absent became a zero.
        assert_eq!(reading.windows.len(), 1);
    }

    #[test]
    fn nothing_identifying_is_kept_from_the_recording() {
        let lines = fixture_lines();
        let limits = response::<RateLimitsRead>(&lines[2], 3).unwrap().unwrap();
        let rendered =
            super::super::snapshot::login_value(Some(&reading(limits, None, now())), None, false)
                .to_string();
        for secret in ["REDACTED", "Full reset", "Thanks", "RateLimitResetCredit"] {
            assert!(!rendered.contains(secret), "{secret} in {rendered}");
        }
    }

    #[test]
    fn other_lines_are_skipped_and_an_error_is_a_refusal() {
        for line in [
            &br#"{"method":"account/rateLimits/updated","params":{}}"#[..],
            br#"{"id":2,"result":{}}"#,
            br#"{"id":3,"method":"item/tool/call","params":{}}"#,
            b"not json",
        ] {
            assert!(response::<IgnoredAny>(line, 3).is_none());
        }
        assert_eq!(
            response::<IgnoredAny>(br#"{"id":3,"error":{"code":-32600,"message":"x"}}"#, 3)
                .unwrap()
                .unwrap_err(),
            Fail::Refused
        );
        assert_eq!(
            response::<RateLimitsRead>(br#"{"id":3,"result":{"rateLimits":7}}"#, 3)
                .unwrap()
                .unwrap_err(),
            Fail::Unreadable
        );
    }

    #[test]
    fn a_refused_usage_read_is_named_by_the_account() {
        let account = |json: &str| serde_json::from_str::<AccountRead>(json).unwrap();
        let signed_out = account(r#"{"account":null,"requiresOpenaiAuth":true}"#);
        let api_key = account(r#"{"account":{"type":"apiKey"},"requiresOpenaiAuth":true}"#);
        let chatgpt = account(
            r#"{"account":{"type":"chatgpt","email":"a@b.c","planType":"plus"},"requiresOpenaiAuth":true}"#,
        );
        let refused = || Err(Fail::Refused);
        assert_eq!(
            conclude(Some(&signed_out), refused(), now()),
            Err(Reason::SignedOut)
        );
        assert_eq!(
            conclude(Some(&api_key), refused(), now()),
            Err(Reason::NotSubscription)
        );
        assert_eq!(
            conclude(Some(&chatgpt), refused(), now()),
            Err(Reason::Failed)
        );
        assert_eq!(conclude(None, refused(), now()), Err(Reason::Failed));
        assert_eq!(
            conclude(Some(&chatgpt), Err(Fail::Timeout), now()),
            Err(Reason::Timeout)
        );
        // The account's plan is the fallback when the usage read has none.
        let limits = limits_with(serde_json::json!({"rateLimits": {}}));
        let reading = conclude(Some(&chatgpt), Ok(limits), now()).unwrap();
        assert_eq!(reading.plan, Plan::Plus);
        assert_eq!(reading.limit_reached, None);
        assert!(reading.windows.is_empty());
    }

    #[test]
    fn lanes_follow_the_duration_then_the_slot() {
        let lanes = |primary: serde_json::Value, secondary: serde_json::Value| {
            let limits = limits_with(serde_json::json!({"rateLimits": {
                "primary": primary, "secondary": secondary,
            }}));
            reading(limits, None, now())
                .windows
                .iter()
                .map(|w| (w.lane, w.duration_mins))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            lanes(
                window(42.into(), 300.into()),
                window(13.into(), 10_080.into())
            ),
            vec![(Lane::Session, Some(300)), (Lane::Weekly, Some(10_080))]
        );
        assert_eq!(
            lanes(window(0.into(), 10_080.into()), serde_json::Value::Null),
            vec![(Lane::Weekly, Some(10_080))]
        );
        assert_eq!(
            lanes(window(5.into(), 120.into()), window(5.into(), 1_440.into())),
            vec![(Lane::Session, Some(120)), (Lane::Weekly, Some(1_440))]
        );
        assert_eq!(
            lanes(
                window(5.into(), serde_json::Value::Null),
                window(5.into(), serde_json::Value::Null)
            ),
            vec![(Lane::Session, None), (Lane::Weekly, None)]
        );
    }

    #[test]
    fn a_percentage_out_of_range_drops_only_that_window() {
        for bad in [
            serde_json::json!(-1),
            serde_json::json!(100.5),
            serde_json::json!(1_000),
            serde_json::Value::Null,
        ] {
            let limits = limits_with(serde_json::json!({"rateLimits": {
                "primary": window(bad.clone(), 300.into()),
                "secondary": window(12.34.into(), 10_080.into()),
            }}));
            let windows = reading(limits, None, now()).windows;
            assert_eq!(windows.len(), 1, "{bad}");
            assert_eq!(windows[0].lane, Lane::Weekly);
            assert_eq!(windows[0].used_permille, 123);
        }
        let limits = limits_with(serde_json::json!({"rateLimits": {
            "primary": window(100.into(), 300.into()),
        }}));
        assert_eq!(reading(limits, None, now()).windows[0].used_permille, 1_000);
    }

    #[test]
    fn a_reset_in_the_past_or_too_far_ahead_is_dropped_not_clamped() {
        let at = |secs: i64| {
            let limits = limits_with(serde_json::json!({"rateLimits": {
                "primary": {"usedPercent": 1, "windowDurationMins": 300, "resetsAt": secs},
            }}));
            reading(limits, None, now()).windows[0].resets_at
        };
        let soon = now().timestamp() + 3_600;
        assert_eq!(at(soon), DateTime::from_timestamp(soon, 0));
        assert_eq!(at(now().timestamp() - 1), None);
        assert_eq!(at(now().timestamp() + 32 * 86_400), None);
        // Milliseconds are far past the 31-day horizon.
        assert_eq!(at(soon * 1_000), None);
    }

    #[test]
    fn model_limits_are_labelled_only_with_a_checked_name() {
        let model = |name: serde_json::Value| {
            serde_json::json!({
                "limitId": "x", "limitName": name,
                "primary": window(50.into(), serde_json::Value::Null),
            })
        };
        let limits = limits_with(serde_json::json!({
            "rateLimits": {"limitId": "codex", "primary": window(1.into(), 300.into())},
            "rateLimitsByLimitId": {
                "codex": {"limitId": "codex", "primary": window(1.into(), 300.into())},
                "a_spark": model("GPT-5.3 Codex Spark".into()),
                "b_bad": model("https://example.com/".into()),
                "c_long": model("x".repeat(41).into()),
                "d_none": model(serde_json::Value::Null),
            },
        }));
        let windows = reading(limits, None, now()).windows;
        let models: Vec<(Lane, Option<&str>, u16)> = windows
            .iter()
            .skip(1)
            .map(|w| (w.lane, w.label.as_deref(), w.used_permille))
            .collect();
        assert_eq!(
            models,
            vec![
                (Lane::Model, Some("GPT-5.3 Codex Spark"), 500),
                (Lane::Model, None, 500),
                (Lane::Model, None, 500),
                (Lane::Model, None, 500),
            ]
        );
    }

    #[test]
    fn limit_reached_is_reported_never_inferred() {
        let reached = |main: serde_json::Value, extra: serde_json::Value| {
            let mut result = serde_json::json!({"rateLimits": main});
            if let (Some(result), Some(extra)) = (result.as_object_mut(), extra.as_object()) {
                result.extend(extra.clone());
            }
            reading(limits_with(result), None, now()).limit_reached
        };
        let full = window(100.into(), 300.into());
        assert_eq!(
            reached(serde_json::json!({"primary": full}), serde_json::json!({})),
            None
        );
        assert_eq!(
            reached(
                serde_json::json!({"rateLimitReachedType": null}),
                serde_json::json!({})
            ),
            Some(false)
        );
        assert_eq!(
            reached(
                serde_json::json!({"rateLimitReachedType": "rate_limit_reached"}),
                serde_json::json!({})
            ),
            Some(true)
        );
        assert_eq!(
            reached(
                serde_json::json!({}),
                serde_json::json!({"ordinaryUsageAllowed": false})
            ),
            Some(true)
        );
    }

    #[test]
    fn credits_keep_a_balance_only_when_it_is_a_decimal() {
        let balance = |value: serde_json::Value| {
            let limits = limits_with(serde_json::json!({"rateLimits": {"credits": {
                "hasCredits": true, "unlimited": false, "balance": value,
            }}}));
            reading(limits, None, now()).credits.unwrap().balance
        };
        assert_eq!(balance("12.50".into()), Some("12.50".to_string()));
        assert_eq!(balance("7".into()), Some("7".to_string()));
        for bad in ["-1", "1e3", "", ".5", "5.", "$5", "12,50"] {
            assert_eq!(balance(bad.into()), None, "{bad}");
        }
        let limits = limits_with(serde_json::json!({"rateLimits": {"credits": {"balance": "1"}}}));
        assert_eq!(reading(limits, None, now()).credits, None);
    }

    #[test]
    fn a_line_over_the_cap_ends_the_read() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut input = b"{\"id\":1}\n".to_vec();
        input.extend(std::iter::repeat_n(b'a', MAX_LINE_BYTES + 1));
        input.push(b'\n');
        read_lines(&input[..], &tx);
        assert!(matches!(rx.recv().unwrap(), Line::Text(line) if line == b"{\"id\":1}"));
        assert!(matches!(rx.recv().unwrap(), Line::TooLong));
        // Exactly at the cap is still a line.
        let (tx, rx) = std::sync::mpsc::channel();
        let mut input = vec![b'a'; MAX_LINE_BYTES];
        input.push(b'\n');
        read_lines(&input[..], &tx);
        assert!(matches!(rx.recv().unwrap(), Line::Text(line) if line.len() == MAX_LINE_BYTES));
        assert!(matches!(rx.recv().unwrap(), Line::Closed));
    }

    /// A stand-in `codex`: a shell script that records how it was launched
    /// and answers from the recording, or misbehaves as told.
    #[cfg(unix)]
    mod process {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        fn script(dir: &Path, body: &str) -> PathBuf {
            let path = dir.join("codex");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        fn short() -> Budgets {
            Budgets {
                initialize: Duration::from_secs(5),
                request: Duration::from_millis(500),
                total: Duration::from_secs(5),
                close_wait: Duration::from_millis(200),
                term_wait: Duration::from_millis(200),
            }
        }

        fn source(codex: PathBuf, work: PathBuf, budgets: Budgets) -> CodexAppServer {
            CodexAppServer::with(
                Box::new(move || Some(codex.clone())),
                Box::new(|| false),
                work,
                budgets,
            )
        }

        #[test]
        fn the_recorded_answers_through_a_real_process() {
            let dir = tempfile::tempdir().unwrap();
            let record = dir.path().join("launch");
            let answers: Vec<PathBuf> = fixture_lines()
                .into_iter()
                .enumerate()
                .map(|(i, line)| {
                    let path = dir.path().join(format!("answer-{i}"));
                    std::fs::write(&path, [line, b"\n".to_vec()].concat()).unwrap();
                    path
                })
                .collect();
            // A notification before the first answer, which is skipped; the
            // `initialized` notification is read before `account/read`.
            let body = format!(
                "echo \"$@\" > '{record}'\npwd >> '{record}'\nenv | sort >> '{record}'\n\
                 echo 'stderr is discarded' >&2\n\
                 read -r line\n\
                 echo '{{\"method\":\"account/rateLimits/updated\",\"params\":{{}}}}'\n\
                 cat '{a0}'\n\
                 read -r line\nread -r line\ncat '{a1}'\n\
                 read -r line\ncat '{a2}'\n\
                 while read -r line; do :; done",
                record = record.display(),
                a0 = answers[0].display(),
                a1 = answers[1].display(),
                a2 = answers[2].display(),
            );
            let codex = script(dir.path(), &body);
            let work = dir.path().join("work");
            let source = source(codex, work.clone(), short());
            assert_eq!(source.ready(), Ok(()));
            let reading = source.probe(Utc::now()).unwrap();
            assert_eq!(reading.plan, Plan::Business);
            assert_eq!(reading.reset_credits, Some(2));
            let launch = std::fs::read_to_string(&record).unwrap();
            let mut lines = launch.lines();
            assert_eq!(lines.next(), Some("-s read-only -a never app-server"));
            assert_eq!(
                std::fs::canonicalize(lines.next().unwrap()).unwrap(),
                std::fs::canonicalize(&work).unwrap()
            );
            let names: Vec<&str> = lines.filter_map(|l| l.split('=').next()).collect();
            for name in &names {
                assert!(
                    [
                        "HOME", "PATH", "LANG", "TMPDIR", "PWD", "SHLVL", "_", "OLDPWD"
                    ]
                    .contains(name),
                    "the child inherited {name}"
                );
            }
            assert!(names.contains(&"PATH"));
            assert!(std::fs::read_dir(&work).unwrap().next().is_none());
        }

        #[test]
        fn a_silent_codex_times_out_and_is_reaped() {
            let dir = tempfile::tempdir().unwrap();
            let pid_file = dir.path().join("pid");
            // Ignores SIGTERM and never answers: only SIGKILL ends it.
            let codex = script(
                dir.path(),
                &format!(
                    "echo $$ > '{}'\ntrap '' TERM\nwhile :; do sleep 1; done",
                    pid_file.display()
                ),
            );
            // Long enough for macOS to assess a script it has not seen
            // before and start it.
            let budgets = Budgets {
                initialize: Duration::from_secs(3),
                ..short()
            };
            let source = source(codex, dir.path().join("work"), budgets);
            let started = Instant::now();
            assert_eq!(source.probe(Utc::now()), Err(Reason::Timeout));
            // The budget, then 0.2 s before SIGTERM and 0.2 s before SIGKILL.
            assert!(started.elapsed() < Duration::from_secs(5));
            let pid: u32 = std::fs::read_to_string(&pid_file)
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            let mut system = sysinfo::System::new();
            let pid = sysinfo::Pid::from_u32(pid);
            system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
            assert!(system.process(pid).is_none(), "the child was reaped");
        }

        #[test]
        fn a_line_over_the_cap_is_unreadable() {
            let dir = tempfile::tempdir().unwrap();
            let codex = script(
                dir.path(),
                "read -r line\nhead -c 1100000 /dev/zero | tr '\\0' 'a'\necho\nwhile read -r line; do :; done",
            );
            let source = source(codex, dir.path().join("work"), short());
            assert_eq!(source.probe(Utc::now()), Err(Reason::Unreadable));
        }

        #[test]
        fn a_codex_that_exits_is_failed() {
            let dir = tempfile::tempdir().unwrap();
            let codex = script(dir.path(), "exit 3");
            let source = source(codex, dir.path().join("work"), short());
            assert_eq!(source.probe(Utc::now()), Err(Reason::Failed));
        }

        #[test]
        fn codex_running_or_missing_stops_before_any_launch() {
            let dir = tempfile::tempdir().unwrap();
            let marker = dir.path().join("launched");
            let codex = script(dir.path(), &format!("touch '{}'", marker.display()));
            let running = CodexAppServer::with(
                Box::new(move || Some(codex.clone())),
                Box::new(|| true),
                dir.path().join("work"),
                short(),
            );
            assert_eq!(running.ready(), Err(Reason::CodexRunning));
            let missing = CodexAppServer::with(
                Box::new(|| None),
                Box::new(|| false),
                dir.path().join("work"),
                short(),
            );
            assert_eq!(missing.ready(), Err(Reason::CodexNotFound));
            let relative = CodexAppServer::with(
                Box::new(|| Some(PathBuf::from("codex"))),
                Box::new(|| false),
                dir.path().join("work"),
                short(),
            );
            assert_eq!(relative.ready(), Err(Reason::CodexNotFound));
            assert!(!marker.exists());
        }
    }
}
