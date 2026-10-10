//! The macOS shell's counterpart of `tauri_copy_surface_is_central` (K4,
//! #1173): SwiftUI screens render the contributor core's consent and
//! disclosure sentences, read across the C ABI, and never write their own.
//!
//! Three checks over every Swift source under `macos/Sources`:
//!
//! 1. No pinned core sentence -- the automatic-contribution (`AUTO_*`),
//!    arming (`ARMING_*`), scrub, withdrawal, quit, ignore, legacy-migration
//!    and route-disclosure copy -- appears in a Swift string literal. A
//!    hand-typed copy is how a shell keeps saying a sentence after the core
//!    changed it.
//! 2. Every Swift screen that shows one of those surfaces calls the bridge
//!    function that reads it, and every bridge function calls its `tc_*`
//!    export. A screen that renders the surface and asks nobody is writing
//!    it.
//! 3. No Swift string literal says "private inference", the setting's
//!    internal name (`tauri_never_says_private_inference_to_a_contributor`).
//!
//! The scanner is a character walk over Swift, not a Swift parser: comments
//! are skipped, an interpolation reads as one placeholder character, and
//! literals joined with `+` are read as one literal so a sentence split
//! across lines is still seen whole.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("crate is two directories below the repo root")
        .to_path_buf()
}

fn swift_root() -> PathBuf {
    repo_root().join("macos/Sources")
}

fn read(rel: &str) -> String {
    let path = swift_root().join(rel);
    // Normalised to LF: the Windows runner checks sources out with CRLF,
    // and the checks below search for "\n"-delimited text.
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} is unreadable: {error}", path.display()))
        .replace("\r\n", "\n")
}

fn visit_swift(dir: &Path, found: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{} is unreadable: {error}", dir.display()));
    for entry in entries {
        let path = entry.expect("directory entry is readable").path();
        if path.is_dir() {
            visit_swift(&path, found);
        } else if path.extension().and_then(|e| e.to_str()) == Some("swift") {
            found.push(path);
        }
    }
}

/// Every Swift source under `macos/Sources`, keyed by its path below it.
fn swift_sources() -> Vec<(String, String)> {
    let root = swift_root();
    let mut paths = Vec::new();
    visit_swift(&root, &mut paths);
    paths.sort();
    let sources: Vec<(String, String)> = paths
        .iter()
        .map(|path| {
            // Keyed with `/` on every host, so the allowlist and the
            // surface table match on the Windows runner too.
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(path)
                .display()
                .to_string()
                .replace('\\', "/");
            let text = std::fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("{} is unreadable: {error}", path.display()));
            (rel, text)
        })
        .collect();
    // A scan that found nothing passes over nothing.
    assert!(
        sources.len() >= 86,
        "only {} Swift sources were found under {}; the whole tree is expected",
        sources.len(),
        root.display()
    );
    sources
}

/// What an interpolation reads as. A core sentence built with an argument
/// is generated with this in the argument's place, so `"Ignore \(project)?"`
/// and `ignore_project_title(HOLE)` compare equal.
const HOLE: &str = "\u{0}";
/// A number no sentence contains, standing in for a count argument; it is
/// replaced by [`HOLE`] once the sentence is built.
const COUNT: u32 = 7919;

enum Token {
    Literal(String),
    Plus,
    Code(char),
}

/// Skip a balanced `( ... )` starting at `chars[at] == '('`, honouring nested
/// string literals; returns the index after the closing `)`.
fn skip_balanced(chars: &[char], mut at: usize) -> usize {
    let mut depth = 0usize;
    while at < chars.len() {
        match chars[at] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return at + 1;
                }
            }
            '"' => {
                at += 1;
                while at < chars.len() && chars[at] != '"' {
                    if chars[at] == '\\' {
                        at += 1;
                    }
                    at += 1;
                }
            }
            _ => {}
        }
        at += 1;
    }
    at
}

/// Read one escape after a `\` at `chars[at]`, appending what it stands for.
/// `hashes` is the raw-string delimiter count the escape must carry.
fn read_escape(chars: &[char], at: usize, hashes: usize, text: &mut String) -> usize {
    let mut next = at + 1;
    for _ in 0..hashes {
        if chars.get(next) != Some(&'#') {
            text.push('\\');
            return at + 1;
        }
        next += 1;
    }
    match chars.get(next) {
        Some('(') => {
            text.push_str(HOLE);
            skip_balanced(chars, next)
        }
        Some('n') | Some('t') | Some('r') => {
            text.push(' ');
            next + 1
        }
        Some('\n') => {
            // A multi-line literal's line continuation.
            text.push(' ');
            next + 1
        }
        Some('u') if chars.get(next + 1) == Some(&'{') => {
            let close = chars[next..]
                .iter()
                .position(|c| *c == '}')
                .map_or(chars.len(), |p| next + p);
            let hex: String = chars[next + 2..close].iter().collect();
            if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                text.push(c);
            }
            close + 1
        }
        Some(c) => {
            text.push(*c);
            next + 1
        }
        None => next,
    }
}

/// The source as tokens: literals with their escapes read, `+`, and every
/// other non-space, non-comment character.
fn tokens(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && next == Some('*') {
            let mut depth = 1;
            i += 2;
            while i + 1 < chars.len() && depth > 0 {
                if chars[i] == '/' && chars[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars[i + 1] == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        // A literal, possibly raw (`#"..."#`) and possibly multi-line.
        let mut hashes = 0;
        while chars.get(i + hashes) == Some(&'#') {
            hashes += 1;
        }
        if chars.get(i + hashes) == Some(&'"') {
            let start = i + hashes;
            let multi = chars.get(start + 1) == Some(&'"') && chars.get(start + 2) == Some(&'"');
            let quote = if multi { 3 } else { 1 };
            let mut at = start + quote;
            let mut text = String::new();
            let closes = |at: usize| -> bool {
                (0..quote).all(|k| chars.get(at + k) == Some(&'"'))
                    && (0..hashes).all(|k| chars.get(at + quote + k) == Some(&'#'))
            };
            while at < chars.len() && !closes(at) {
                if chars[at] == '\\' {
                    at = read_escape(&chars, at, hashes, &mut text);
                    continue;
                }
                if !multi && chars[at] == '\n' {
                    break;
                }
                text.push(chars[at]);
                at += 1;
            }
            out.push(Token::Literal(text));
            i = at + quote + hashes;
            continue;
        }
        if c == '+' {
            out.push(Token::Plus);
        } else if !c.is_whitespace() {
            out.push(Token::Code(c));
        }
        i += 1;
    }
    out
}

/// Whitespace collapsed and apostrophes straightened, so a reflowed or
/// typographically quoted copy still compares equal.
fn normalise(text: &str) -> String {
    text.replace(['\u{2019}', '\u{2018}'], "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every string literal in a Swift source, `+`-joined literals read as one.
fn swift_literals(source: &str) -> Vec<String> {
    let mut literals: Vec<String> = Vec::new();
    // `after_literal`: the last token was a literal. `joining`: the last two
    // were a literal and a `+`, so the next literal continues it.
    let (mut after_literal, mut joining) = (false, false);
    for token in tokens(source) {
        match token {
            Token::Literal(text) => {
                match literals.last_mut() {
                    Some(last) if joining => last.push_str(&text),
                    _ => literals.push(text),
                }
                (after_literal, joining) = (true, false);
            }
            Token::Plus => (after_literal, joining) = (false, after_literal),
            Token::Code(_) => (after_literal, joining) = (false, false),
        }
    }
    literals.iter().map(|text| normalise(text)).collect()
}

/// The source with comments and string literals removed: what a call-site
/// check may look at, so a comment or a literal naming a function cannot
/// satisfy it.
fn swift_code(source: &str) -> String {
    tokens(source)
        .into_iter()
        .map(|token| match token {
            Token::Literal(_) => ' ',
            Token::Plus => '+',
            Token::Code(c) => c,
        })
        .collect()
}

/// A pinned sentence's fragments: its sentences, split after `.`, `?`, `!`
/// or `:` and at line breaks, keeping those of at least four words. Shorter
/// ones -- "Quit", "Not now", "Cancel" -- are button labels every shell
/// shares with a hundred other buttons, and a literal holding one is not a
/// copy of the core's sentence.
fn fragments(sentence: &str) -> Vec<String> {
    let text = normalise(&sentence.replace('\n', " \u{1} "));
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (at, c) in chars.iter().enumerate() {
        if *c == '\u{1}' {
            out.push(std::mem::take(&mut current));
            continue;
        }
        current.push(*c);
        let ends = matches!(c, '.' | '?' | '!' | ':')
            && chars.get(at + 1).is_none_or(|n| n.is_whitespace());
        if ends {
            out.push(std::mem::take(&mut current));
        }
    }
    out.push(current);
    out.into_iter()
        .map(|f| normalise(&f))
        .filter(|f| f.split_whitespace().count() >= 4)
        .collect()
}

/// Collect every string leaf of a JSON value.
fn table_strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) => out.push(text.clone()),
        serde_json::Value::Array(items) => items.iter().for_each(|v| table_strings(v, out)),
        serde_json::Value::Object(map) => map.values().for_each(|v| table_strings(v, out)),
        _ => {}
    }
}

/// Every string in a serialised copy table.
fn table(value: serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    table_strings(&value, &mut out);
    out
}

/// A copy table as JSON, the shape every shell receives it in.
macro_rules! json {
    ($copy:expr) => {
        serde_json::to_value(&$copy).expect("copy tables serialise")
    };
}

/// A first-run sentence with each `{name}` placeholder replaced by [`HOLE`],
/// so it compares equal to the Swift literal that interpolates the argument.
fn holed(text: String) -> String {
    placeholders().fold(text, |text, name| {
        text.replace(&format!("{{{name}}}"), HOLE)
    })
}

/// Every `{name}` a pinned table may carry: the first run's and the
/// monitor's (#1241).
fn placeholders() -> impl Iterator<Item = &'static &'static str> {
    trace_commons_contributor::first_run_copy::PLACEHOLDERS
        .iter()
        .chain(trace_commons_contributor::preview_copy::MONITOR_PLACEHOLDERS)
        .chain(trace_commons_contributor::shell_words_copy::SHELL_WORDS_PLACEHOLDERS)
}

/// The core sentences no Swift literal may hold, by where they come from.
fn pinned_sentences() -> Vec<(&'static str, String)> {
    use trace_commons_contributor::{
        consent_copy as consent, daemon::automatic_gate::Disclosure, daemon::policy::ProjectMode,
        preview_copy, privacy_scan_copy, project_copy, quit_copy, withdraw,
    };
    let counted = |text: String| text.replace(&COUNT.to_string(), HOLE);
    let mut pinned: Vec<(&'static str, String)> = Vec::new();
    let mut add = |origin: &'static str, sentences: Vec<String>| {
        pinned.extend(sentences.into_iter().map(|s| (origin, s)));
    };

    // Automatic contribution (Flow 1) and the consent gate.
    add(
        "consent_copy::AUTO_*",
        [
            consent::AUTO_SCRUB_SCOPE,
            consent::AUTO_SCRUB_LIMIT,
            consent::AUTO_NO_REVIEW,
            consent::AUTO_PATTERNS_ONLY_SCOPE,
            consent::AUTO_PATTERNS_ONLY_LIMIT,
            consent::AUTO_RAW_SEND_BOTH_ENCLAVES,
            consent::AUTO_PROJECT_DISCLOSURE_UNAVAILABLE,
            consent::AUTO_SCOPE_REQUIRED,
            consent::AUTO_PATH_AUTOMATIC,
            consent::AUTO_PATH_ASK_FIRST,
            consent::AUTO_PATH_AUTOMATIC_TITLE,
            consent::AUTO_PATH_AUTOMATIC_DETAIL,
            consent::AUTO_PATH_ASK_FIRST_TITLE,
            consent::AUTO_PATH_ASK_FIRST_DETAIL,
        ]
        .map(str::to_owned)
        .to_vec(),
    );
    add(
        "consent_copy::automatic_grant_copy",
        [Disclosure::PatternsOnly, Disclosure::ModelScrubbed]
            .into_iter()
            .flat_map(|d| table(json!(consent::automatic_grant_copy(d))))
            .collect(),
    );
    add(
        "consent_copy::GATE_*",
        [
            consent::GATE_STATEMENT,
            consent::GATE_READY_HELP,
            consent::GATE_NOT_PINNED_HELP,
            consent::GATE_HELD_TITLE,
            consent::GATE_HELD_RELEASE,
            consent::GATE_HELD_ASK_FIRST,
        ]
        .map(str::to_owned)
        .to_vec(),
    );

    // Arming and ignoring a project.
    add(
        "project_copy::ARMING_*",
        [
            project_copy::ARMING_BODY,
            project_copy::ARMING_BODY_WITH_BACKLOG,
            project_copy::ARMING_OFFER_CONFIRM,
            project_copy::ARMING_OFFER_DECLINE,
        ]
        .map(str::to_owned)
        .to_vec(),
    );
    add(
        "project_copy::arming_offer_copy",
        table(json!(project_copy::arming_offer_copy(HOLE, COUNT)))
            .into_iter()
            .map(counted)
            .collect(),
    );
    // The first-run wording of #1030.
    add(
        "first_run_copy::first_run_copy",
        table(json!(
            trace_commons_contributor::first_run_copy::first_run_copy()
        ))
        .into_iter()
        .map(holed)
        .collect(),
    );
    // Ron's #1146 inspector words (#1241), the parts of the monitor tables
    // that carry them. The tables' older single words are not pinned here.
    {
        let traces = json!(preview_copy::monitor_traces_copy());
        let screens = json!(preview_copy::monitor_screens_copy());
        let mut ron = Vec::new();
        for key in [
            "tree",
            "counts",
            "inspector",
            "summary_panel",
            "session_review",
            "look_inside",
            "undo",
            "optional_automation",
            "dismiss_action",
        ] {
            ron.extend(table(traces[key].clone()));
        }
        ron.extend(table(screens["safeguards"].clone()));
        assert!(ron.len() > 100, "Ron's inspector words are not being read");
        add(
            "preview_copy::monitor_*_copy (#1146 inspector)",
            ron.into_iter().map(holed).collect(),
        );
        // History's refresh and sign-in controls, in native words (#1241).
        add(
            "preview_copy::monitor_screens_copy history_actions",
            table(screens["history_actions"].clone())
                .into_iter()
                .map(holed)
                .collect(),
        );
        // Ron's #1146 Home and History structure words (glass parity).
        add(
            "preview_copy::monitor_screens_copy home_history",
            table(screens["home_history"].clone())
                .into_iter()
                .map(holed)
                .collect(),
        );
    }
    // The menu-bar Contribution mode pill, its override confirmations and
    // their refusal lines (#1173).
    add(
        "project_copy::contribution_mode_copy",
        table(json!(project_copy::contribution_mode_copy())),
    );
    add(
        "project_copy::contribution_override_confirm_copy",
        [
            ProjectMode::NotifyOnly,
            ProjectMode::AutoUpload,
            ProjectMode::Ignore,
        ]
        .into_iter()
        .flat_map(|m| {
            table(json!(project_copy::contribution_override_confirm_copy(
                m, None
            )))
        })
        .collect(),
    );
    add(
        "project_copy::CONTRIBUTION_OVERRIDE_REFUSED*",
        [
            project_copy::CONTRIBUTION_OVERRIDE_REFUSED,
            project_copy::CONTRIBUTION_OVERRIDE_REFUSED_NO_TERMS,
        ]
        .map(str::to_owned)
        .to_vec(),
    );
    add(
        "project_copy::ignore_project_copy",
        table(json!(project_copy::ignore_project_copy(
            HOLE,
            COUNT as usize
        )))
        .into_iter()
        .chain(table(json!(project_copy::ignore_project_copy(HOLE, 0))))
        .chain(project_copy::ignore_project_reconciled(
            HOLE,
            1,
            u64::from(COUNT),
        ))
        .map(counted)
        .collect(),
    );

    // Scrubbing: the extra privacy scan, the removed-summary panel, and the
    // surviving-secret line.
    add(
        "privacy_scan_copy",
        table(json!(privacy_scan_copy::privacy_scan_copy())),
    );
    // The health banner (R6/R7, #1173): every label's words and the
    // core-down banner. `daily-cap-reached` is left out: its title is
    // `DAILY_BUDGET_TITLE`, which `DailyBudgetCopy.swift` holds as Swift
    // until the budget banner has an export of its own.
    {
        use trace_commons_contributor::{daemon::health::ALL_LABELS, health_copy};
        let mut lines = table(json!(health_copy::core_down_copy()));
        for label in ALL_LABELS
            .iter()
            .filter(|label| **label != "daily-cap-reached")
        {
            lines.extend(table(json!(health_copy::health_copy_for_label(
                label, None
            ))));
        }
        add("health_copy", lines);
    }
    add(
        "preview_copy::REDACTION_CATEGORY_*",
        [
            preview_copy::REDACTION_CATEGORY_LOCAL_PATH,
            preview_copy::REDACTION_CATEGORY_SECRET,
            preview_copy::REDACTION_CATEGORY_PRIVACY_FILTER,
            preview_copy::REDACTION_CATEGORY_SENSITIVE_FIELD,
            preview_copy::REDACTION_CATEGORY_TOOL_SENSITIVE_FIELD,
            preview_copy::REDACTION_CATEGORY_RESIDUAL,
            preview_copy::REDACTION_CATEGORY_UNKNOWN,
        ]
        .map(str::to_owned)
        .to_vec(),
    );
    add(
        "preview_copy::residual_secret_line",
        vec![
            preview_copy::residual_secret_line(1, &[]),
            counted(preview_copy::residual_secret_line(COUNT, &[])),
        ],
    );

    // History's word for each contribution status, the one table every
    // shell reads (`tc_public_run_copy`'s `history_status_labels`).
    add(
        "history_copy::STATUS_LABELS",
        trace_commons_contributor::history_copy::STATUS_LABELS
            .iter()
            .map(|row| row.label.to_owned())
            .chain([trace_commons_contributor::history_copy::STATUS_UNAVAILABLE.to_owned()])
            .collect(),
    );

    // Withdrawal and quitting.
    add(
        "withdraw::confirmation_prompt_unknown",
        vec![withdraw::confirmation_prompt_unknown().to_owned()],
    );
    add(
        "quit_copy::quit_prompt",
        [
            quit_copy::QuitRole::Hosting,
            quit_copy::QuitRole::Attached,
            quit_copy::QuitRole::Unavailable,
        ]
        .into_iter()
        .flat_map(|role| table(json!(quit_copy::quit_prompt(role))))
        .collect(),
    );

    // The words the macOS shell wrote in Swift until #1146 parity
    // (2026-10-07): withdrawal, the public profile, the queue's and
    // History's legacy words, the scrubbing caveat and Settings.
    add(
        "shell_words_copy::shell_words_copy",
        table(json!(
            trace_commons_contributor::shell_words_copy::shell_words_copy()
        ))
        .into_iter()
        .map(holed)
        .collect(),
    );

    // The legacy invite migration, connecting inference, and where sessions
    // go -- surfaces macOS has not built yet, pinned so that when it does
    // they arrive with the core's words.
    let mut legacy = table(json!(consent::legacy_migration_offer()));
    legacy.extend(
        [
            consent::LEGACY_MIGRATION_NOTICE_TITLE,
            consent::LEGACY_MIGRATION_NOTICE_BODY,
            consent::LEGACY_MIGRATION_NOTICE_ARMED_KEPT,
            consent::LEGACY_MIGRATION_NOTICE_NOTHING_ARMED,
            consent::LEGACY_MIGRATION_NOTICE_ACKNOWLEDGE,
        ]
        .map(str::to_owned),
    );
    for label in [
        "legacy_migration_tenant_pooled",
        "legacy_migration_admission_not_ready",
        "legacy_migration_invite_needed",
        "no-such-label",
    ] {
        legacy.push(consent::legacy_migration_refusal_line(label).to_owned());
    }
    add("consent_copy::legacy_migration_*", legacy);
    // The Missions disclosure (M4, #1173): not built in macOS yet, pinned
    // so the Missions tab arrives with the core's words.
    add(
        "consent_copy::missions_disclosure_copy",
        table(json!(consent::missions_disclosure_copy())),
    );
    add(
        "consent_copy::inference_connection_copy",
        table(json!(consent::inference_connection_copy())),
    );
    add(
        "consent_copy::DISCLOSURE_*",
        [
            consent::DISCLOSURE_TITLE,
            consent::DISCLOSURE_UNREADABLE,
            consent::DISCLOSURE_SESSION_UNREADABLE,
            consent::DISCLOSURE_ROUTE_WITNESS_REFUSING,
            consent::DISCLOSURE_ROUTE_LOCAL,
            consent::DISCLOSURE_ROUTE_NOT_ENROLLED,
            consent::DISCLOSURE_ROUTE_SETTINGS_UNREADABLE,
            consent::DISCLOSURE_WITNESS_CHECK,
            consent::DISCLOSURE_WITNESS_CLASSIFIER,
            consent::DISCLOSURE_RECEIPTS_OFF,
            consent::DISCLOSURE_RECEIPTS_CHECKED,
            consent::DISCLOSURE_RECEIPTS_UNCHECKED,
            consent::DISCLOSURE_ATTESTED_BODIES,
            consent::DISCLOSURE_SESSION_BEFORE_WITNESS,
            consent::DISCLOSURE_SESSION_BEFORE_LOCAL,
            consent::DISCLOSURE_SESSION_NOTHING_SENT,
            consent::DISCLOSURE_SESSION_AFTER,
        ]
        .map(str::to_owned)
        .to_vec(),
    );
    pinned
}

/// Swift literals that legitimately repeat a pinned core fragment, each
/// with the reason it is not a copy the core should own instead. An entry
/// names a file and the fragment, so a NEW file repeating the same words
/// still fails. Remove an entry when its reason stops being true.
const ALLOWED: &[(&str, &str, &str)] = &[
    // The export-failure fallback (`HealthCopy.onHoldFallback`): the core's
    // `on_hold_copy()`, verbatim, drawn only when `tc_health_copy_json`
    // returns NULL for a reported label -- a caught panic -- so a reported
    // condition is never drawn as healthy.
    (
        "TraceCommonsApp/HealthCopy.swift",
        "Contributions are on hold.",
        "health export-failure fallback",
    ),
    (
        "TraceCommonsApp/HealthCopy.swift",
        "Something is stopping traces from being sent.",
        "health export-failure fallback",
    ),
    (
        "TraceCommonsApp/HealthCopy.swift",
        "Nothing has been lost, and nothing has gone out.",
        "health export-failure fallback",
    ),
    // The legacy History rollup tallies and the queue's week figures, which
    // held the core's status words as headings, left the shell with the
    // legacy screens (R15); their allowances went with them.
    //
    // Ron's #1146 inspector words (#1241) that native screens already say
    // in Swift. Each goes when its screen reads the core's table instead.
    // The legacy queue's no-longer-waiting group left QueueView.swift for
    // `shell_words_copy` at #1146 parity (2026-10-07).
    // Not a sentence of its own: Look inside reads the core's
    // `session_review.cannot_show_title`, and this verbatim copy of it is
    // the fallback for a table that did not decode, so the cannot-show
    // notice is never drawn without a title (`HealthCopy.onHoldFallback`).
    (
        "TraceCommonsApp/Views/PreviewSheet.swift",
        "This one can't be shown.",
        "fallback copy of the core's cannot_show_title (#1241 Task 7)",
    ),
    // Not a copy: the scanner matches substrings, and "N waiting for your
    // decision" contains Ron's "N waiting for you".
    (
        "TCShellCore/MenuBarStatus.swift",
        "waiting for you",
        "substring of a different sentence (\"waiting for your decision\")",
    ),
];

#[test]
fn no_pinned_core_sentence_is_a_swift_literal() {
    let pinned = pinned_sentences();
    let mut checked = 0usize;
    let mut offenders = Vec::new();
    let mut allowance_used = vec![false; ALLOWED.len()];
    let sources = swift_sources();
    let literals: Vec<(String, Vec<String>)> = sources
        .iter()
        .map(|(path, text)| (path.clone(), swift_literals(text)))
        .collect();
    for (origin, sentence) in &pinned {
        for fragment in fragments(sentence) {
            checked += 1;
            for (path, file_literals) in &literals {
                if !file_literals.iter().any(|lit| lit.contains(&fragment)) {
                    continue;
                }
                let allowed = ALLOWED
                    .iter()
                    .position(|(file, words, _)| file == path && fragment.contains(words));
                match allowed {
                    Some(at) => allowance_used[at] = true,
                    None => offenders.push(format!(
                        "{path} holds {origin} as a literal: {:?}",
                        fragment.replace(HOLE, "\\(...)")
                    )),
                }
            }
        }
    }
    // A pinned set that reduced to nothing would pass over nothing.
    assert!(
        checked >= 100,
        "only {checked} core fragments were pinned; the pinned set is not being read"
    );
    assert!(
        offenders.is_empty(),
        "macOS writes core consent and disclosure copy itself instead of reading it \
         across the C ABI (see TCCoreCopy and TCConsentCopy):\n{}",
        offenders.join("\n")
    );
    for (at, used) in allowance_used.iter().enumerate() {
        assert!(
            used,
            "the allowlist entry for {} no longer matches anything; remove it",
            ALLOWED[at].0
        );
    }
}

/// A core sentence that carries a `{name}` placeholder is pinned with a
/// [`HOLE`] in its place, the way the scanner reads a Swift interpolation.
/// Pinned with the braces, `"Include every past trace in \(folder)"`
/// would never match it.
#[test]
fn a_placeholder_sentence_is_pinned_the_way_swift_interpolates_it() {
    let pinned = pinned_sentences();
    let braced: Vec<&String> = pinned
        .iter()
        .map(|(_, sentence)| sentence)
        .filter(|sentence| placeholders().any(|name| sentence.contains(&format!("{{{name}}}"))))
        .collect();
    assert!(
        braced.is_empty(),
        "pinned sentences still carry a brace placeholder: {braced:?}"
    );
    let probe = swift_literals(r#"let t = "Include every past trace in \(folder)""#);
    assert!(
        pinned.iter().any(|(_, sentence)| fragments(sentence)
            .iter()
            .any(|fragment| probe.iter().any(|lit| lit.contains(fragment)))),
        "a Swift re-authoring of a placeholder sentence is not caught: {probe:?}"
    );
}

/// Each safety surface a macOS screen shows, the screen's file, the bridge
/// call that screen must make, the bridge file, and the export that call
/// must reach.
const SURFACES: &[(&str, &str, &str, &str, &str)] = &[
    (
        "armed folder disclosure in Traces",
        "TraceCommonsApp/Views/Monitor/TracesStore.swift",
        "TCCoreCopy.automaticGrantCopyJSON",
        "TCBridge/TCCoreCopy.swift",
        "tc_automatic_grant_copy_json",
    ),
    (
        "monitor Traces words",
        "TraceCommonsApp/Views/Monitor/TracesStore.swift",
        "TCCoreCopy.monitorTracesCopyJSON",
        "TCBridge/TCCoreCopy.swift",
        "tc_monitor_traces_copy_json",
    ),
    (
        "monitor screens words",
        "TraceCommonsApp/Views/Monitor/MonitorWords.swift",
        "TCCoreCopy.monitorScreensCopyJSON",
        "TCBridge/TCCoreCopy.swift",
        "tc_monitor_screens_copy_json",
    ),
    (
        "withdrawal, public profile, legacy queue/History, scrubbing and Settings words",
        "TraceCommonsApp/Views/ShellWords.swift",
        "TCCoreCopy.shellWordsCopyJSON",
        "TCBridge/TCCoreCopy.swift",
        "tc_shell_words_copy_json",
    ),
    (
        "monitor Traces badge words",
        "TraceCommonsApp/Views/MonitorWindowView.swift",
        "TCCoreCopy.decisionsOwedText",
        "TCBridge/TCCoreCopy.swift",
        "tc_decisions_owed_text",
    ),
    (
        "consent gate",
        "TraceCommonsApp/Views/PreviewSheet.swift",
        "TCConsentCopy.copyJSON",
        "TCBridge/TCConsentCopy.swift",
        "tc_consent_copy",
    ),
    // Look inside is read-only since #1241 Task 7: Contribute, and its
    // tooltip, are the inspector's session card (`SessionReviewCard`,
    // #1241 Task 6).
    (
        "consent gate help",
        "TraceCommonsApp/Views/Monitor/SessionReviewCard.swift",
        "TCConsentCopy.gateHelp",
        "TCBridge/TCConsentCopy.swift",
        "tc_consent_gate_help",
    ),
    (
        "grant void notice",
        "TraceCommonsApp/Views/ShellNotices.swift",
        "TCConsentCopy.voidNoticeJSON",
        "TCBridge/TCConsentCopy.swift",
        "tc_grant_void_notice",
    ),
    (
        "arming rewording notice",
        "TraceCommonsApp/Views/ShellNotices.swift",
        "TCConsentCopy.armingRewordedNoticeJSON",
        "TCBridge/TCConsentCopy.swift",
        "tc_arming_reworded_notice",
    ),
    (
        "gate held notice",
        "TraceCommonsApp/AppModel.swift",
        "TCConsentCopy.gateHeldNoticeJSON",
        "TCBridge/TCConsentCopy.swift",
        "tc_gate_held_notice",
    ),
    (
        "legacy migration notice",
        "TraceCommonsApp/AppModel.swift",
        "TCConsentCopy.legacyMigrationNoticeJSON",
        "TCBridge/TCConsentCopy.swift",
        "tc_legacy_migration_notice",
    ),
    (
        "witness capacity notice",
        "TraceCommonsApp/HealthCopy.swift",
        "TCConsentCopy.witnessCapacityNoticeJSON",
        "TCBridge/TCConsentCopy.swift",
        "tc_witness_capacity_notice",
    ),
    (
        "route disclosure",
        "TraceCommonsApp/AppModel.swift",
        "TCConsentCopy.routeDisclosureJSON",
        "TCBridge/TCConsentCopy.swift",
        "tc_route_disclosure_copy",
    ),
    (
        "route disclosure unreadable",
        "TraceCommonsApp/AppModel.swift",
        "TCConsentCopy.routeDisclosureUnreadableJSON",
        "TCBridge/TCConsentCopy.swift",
        "tc_route_disclosure_unreadable_copy",
    ),
    (
        "ignore project",
        "TraceCommonsApp/Views/Monitor/TracesViews.swift",
        "TCCoreCopy.projectIgnoreCopyJSON",
        "TCBridge/TCCoreCopy.swift",
        "tc_project_ignore_copy_json",
    ),
    (
        "ignore project reconciliation",
        "TraceCommonsApp/AppModel.swift",
        "TCCoreCopy.projectIgnoreReconciled",
        "TCBridge/TCCoreCopy.swift",
        "tc_project_ignore_reconciled_text",
    ),
    (
        "contribution mode pill",
        "TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift",
        "TCCoreCopy.contributionModeCopyJSON",
        "TCBridge/TCCoreCopy.swift",
        "tc_contribution_mode_copy_json",
    ),
    (
        "contribution override confirmation",
        "TraceCommonsApp/Views/Monitor/MenuPanelStore.swift",
        "TCCoreCopy.contributionOverrideConfirmJSON",
        "TCBridge/TCCoreCopy.swift",
        "tc_contribution_override_confirm_json",
    ),
    (
        "contribution override refusal",
        "TraceCommonsApp/Views/Monitor/MenuPanelStore.swift",
        "TCCoreCopy.contributionOverrideRefusalLine",
        "TCBridge/TCCoreCopy.swift",
        "tc_contribution_override_refusal_text",
    ),
    (
        "arming offer",
        "TraceCommonsApp/Views/Monitor/TracesOffers.swift",
        "TCCoreCopy.armingOfferCopyJSON",
        "TCBridge/TCCoreCopy.swift",
        "tc_arming_offer_copy_json",
    ),
    (
        "arming confirmation",
        "TraceCommonsApp/Views/Settings/ProjectsSection.swift",
        "TCCoreCopy.armingOfferCopyJSON",
        "TCBridge/TCCoreCopy.swift",
        "tc_arming_offer_copy_json",
    ),
    (
        "first-run arming confirmation",
        "TraceCommonsApp/Views/FirstRun/RulesScreen.swift",
        "TCCoreCopy.armingOfferCopyJSON",
        "TCBridge/TCCoreCopy.swift",
        "tc_arming_offer_copy_json",
    ),
    (
        "surviving secret in Traces",
        "TraceCommonsApp/Views/Monitor/TracesStore.swift",
        "TCCoreCopy.residualSecretLine",
        "TCBridge/TCCoreCopy.swift",
        "tc_residual_secret_line_text",
    ),
    // Look inside's What's in it tab left with #1241 Task 7; the scrubbing
    // panel is the session card's (see the gate help above).
    (
        "scrubbing panel",
        "TraceCommonsApp/Views/Monitor/SessionReviewCard.swift",
        "TCCoreCopy.redactionSummaryJSON",
        "TCBridge/TCCoreCopy.swift",
        "tc_redaction_summary_json",
    ),
    (
        "extra privacy scan recovery",
        "TraceCommonsApp/HealthCopy.swift",
        "TCCoreCopy.healthCopyJSON",
        "TCBridge/TCCoreCopy.swift",
        "tc_health_copy_json",
    ),
    (
        "health banner in Traces",
        "TraceCommonsApp/Views/Monitor/TracesStore.swift",
        "HealthCopy.core(",
        "TCBridge/TCCoreCopy.swift",
        "tc_health_copy_json",
    ),
    (
        "withdrawal of an unknown reach",
        "TraceCommonsApp/Views/WithdrawalCopy.swift",
        "TCCoreCopy.withdrawalConfirmationPrompt",
        "TCBridge/TCCoreCopy.swift",
        "tc_withdrawal_confirmation_prompt_text",
    ),
    (
        "quit prompt",
        "TraceCommonsApp/AppModel.swift",
        "quitPromptJSON()",
        "TCBridge/TCDaemon.swift",
        "tc_quit_prompt_json",
    ),
    (
        "quit prompt without a watcher",
        "TraceCommonsApp/AppDelegate.swift",
        "TCCoreCopy.quitPromptWithoutWatcherJSON",
        "TCBridge/TCCoreCopy.swift",
        "tc_quit_prompt_json",
    ),
    (
        "first run",
        "TraceCommonsApp/Views/OnboardingCoordinatorView.swift",
        "TCCoreCopy.firstRunCopyJSON",
        "TCBridge/TCCoreCopy.swift",
        "tc_first_run_copy_json",
    ),
    (
        "first-run sharing card",
        "TraceCommonsApp/Views/FirstRun/UsesScreen.swift",
        "TCCoreCopy.automaticContributionCopyJSON",
        "TCBridge/TCCoreCopy.swift",
        "tc_automatic_contribution_copy_json",
    ),
];

/// Bridge functions for surfaces macOS has not built yet. Each must still
/// reach its export, so the screen that comes has the core's words to read.
const BRIDGE_ONLY: &[(&str, &str)] = &[
    (
        "TCBridge/TCCoreCopy.swift",
        "tc_legacy_migration_offer_json",
    ),
    (
        "TCBridge/TCCoreCopy.swift",
        "tc_legacy_migration_refusal_text",
    ),
    (
        "TCBridge/TCCoreCopy.swift",
        "tc_inference_connection_copy_json",
    ),
];

#[test]
fn swift_screens_render_core_copy_at_every_safety_surface() {
    for (surface, screen, call, bridge, export) in SURFACES {
        let screen_code = swift_code(&read(screen));
        assert!(
            screen_code.contains(call),
            "{screen} shows the {surface} without calling `{call}`: it must render the \
             core's words, not its own"
        );
        let bridge_code = swift_code(&read(bridge));
        assert!(
            bridge_code.contains(&format!("{export}(")),
            "{bridge} no longer reaches `{export}` for the {surface}"
        );
    }
    for (bridge, export) in BRIDGE_ONLY {
        assert!(
            swift_code(&read(bridge)).contains(&format!("{export}(")),
            "{bridge} no longer reaches `{export}`"
        );
    }

    // The quit alert shows the prompt it was handed and is not confirmable
    // without one; the hosting sentence it used to hard-code is false for
    // an attached app.
    let delegate = read("TraceCommonsApp/AppDelegate.swift");
    let delegate_code = swift_code(&delegate);
    for rendered in [
        "prompt.title",
        "prompt.body",
        "prompt.confirm",
        "prompt.cancel",
        "guardletpromptelse{returnfalse}",
    ] {
        assert!(
            delegate_code
                .replace(char::is_whitespace, "")
                .contains(&rendered.replace(char::is_whitespace, "")),
            "the quit alert must use `{rendered}`"
        );
    }
}

/// A History row's status tag is the core's word for the status
/// (`history_copy::STATUS_LABELS`, decoded as
/// `PublicRunCopy.historyStatusLabels`), never a switch of Swift literals.
/// The sentence ratchet above cannot see the three-word labels ("In the
/// commons", "Withdrawn by you"), so the row function is checked directly.
#[test]
fn history_rows_read_the_cores_status_words() {
    let models =
        swift_code(&read("TraceCommonsApp/PublicRunModels.swift")).replace(char::is_whitespace, "");
    assert!(
        models.contains("funchistoryStatusLabel(forvalue:String)->String{historyStatusLabels."),
        "PublicRunCopy.historyStatusLabel(for:) no longer reads historyStatusLabels"
    );

    let history = read("TraceCommonsApp/Views/Monitor/HomeViews.swift");
    let start = history
        .find("static func historyStatusLabel(")
        .expect("HomeFormat.historyStatusLabel exists");
    let end = history[start..]
        .find("\n    }\n")
        .map(|at| start + at)
        .expect("historyStatusLabel has a body");
    let body = &history[start..end];
    assert!(
        swift_code(body).contains("historyStatusLabel(for:"),
        "HomeFormat.historyStatusLabel must read the core's table"
    );
    let literals = swift_literals(body);
    for row in trace_commons_contributor::history_copy::STATUS_LABELS {
        assert!(
            !literals.iter().any(|lit| lit.contains(row.label)),
            "HomeFormat.historyStatusLabel types the core's word {:?} for {}",
            row.label,
            row.status
        );
    }
}

/// A project mode reads by the core's one name (owner decision,
/// 2026-10-02): `ProjectCopy.modeChoiceLabel(_:)` looks the mode up in the
/// pill's table (`project_copy::FOLDER_MODE_LABELS`, decoded as
/// `ContributionModeCopy`) and no Swift literal in the shared label sources
/// spells a mode -- neither the core's names nor the retired ones. The
/// sentence ratchet above cannot see one- and two-word labels, so the
/// sources are checked directly. Scoped to the shared label sources: the
/// glass screens and onboarding views are being rebuilt separately.
#[test]
fn project_mode_names_are_the_cores() {
    let words = swift_code(&read("TraceCommonsApp/ProjectModeWords.swift"))
        .replace(char::is_whitespace, "");
    assert!(
        words.contains(
            "ContributionModeCopy.decode(fromJSON:TCCoreCopy.contributionModeCopyJSON())"
        ),
        "ProjectModeWords must decode the core's pill table"
    );
    assert!(
        words.contains("staticfuncmodeChoiceLabel(_mode:ProjectMode)->String{"),
        "ProjectCopy.modeChoiceLabel(_:) must be defined beside the core's table"
    );
    assert!(words.contains(".label(for:mode)"));
    let copy = swift_code(&read("TCShellCore/ContributionModeCopy.swift"))
        .replace(char::is_whitespace, "");
    assert!(
        copy.contains("funclabel(formode:ProjectMode)->String?{choice(for:mode.rawValue)?.label")
    );

    let retired = [
        "Ask me first",
        "Contribute automatically",
        "Never offer this one",
        "Auto contribute",
        "Ignored",
    ];
    for rel in [
        "TCShellCore/ProjectRow.swift",
        "TCShellCore/ContributionModeCopy.swift",
        "TraceCommonsApp/ProjectModeWords.swift",
    ] {
        let literals = swift_literals(&read(rel));
        for (mode, label) in trace_commons_contributor::project_copy::FOLDER_MODE_LABELS {
            assert!(
                !literals.iter().any(|lit| lit == label),
                "{rel} types the core's name {label:?} for {mode}"
            );
        }
        for word in retired {
            assert!(
                !literals.iter().any(|lit| lit.contains(word)),
                "{rel} still says the retired mode name {word:?}"
            );
        }
    }
}

/// "Private inference" is the setting's internal name and a privacy claim
/// the feature does not make; the destination is "Private AI". No Swift
/// literal may put it in front of a contributor.
#[test]
fn swift_never_says_private_inference_to_a_contributor() {
    let offenders: Vec<String> = swift_sources()
        .iter()
        .filter(|(_, text)| {
            swift_literals(text)
                .iter()
                .any(|lit| lit.to_lowercase().contains("private inference"))
        })
        .map(|(path, _)| path.clone())
        .collect();
    assert!(
        offenders.is_empty(),
        "\"private inference\" is contributor-facing in: {offenders:?}"
    );
}

/// The scanner reads what the checks above depend on it reading.
#[test]
fn the_swift_scanner_reads_literals_the_way_swift_writes_them() {
    let source = r##"
        // "This removes 2 waiting traces" in a comment is not a literal.
        let a = "Ignore \(project)?"
        let b = "Nothing already submitted is affected. "
            + "You can undo this in Settings."
        let c = """
            Sessions from this project will be scrubbed \
            and contributed
            """
        let d = #"raw \#(x) text"#
        f("one") ; g("two")
        let e = count + "three"
    "##;
    let literals = swift_literals(source);
    assert_eq!(
        literals,
        [
            format!("Ignore {HOLE}?"),
            "Nothing already submitted is affected. You can undo this in Settings.".to_owned(),
            "Sessions from this project will be scrubbed and contributed".to_owned(),
            format!("raw {HOLE} text"),
            "one".to_owned(),
            "two".to_owned(),
            "three".to_owned(),
        ]
    );
    let code = swift_code(source);
    assert!(!code.contains("removes"));
    assert!(!code.contains("Ignore"));
    assert_eq!(
        fragments("Quit? Not now. This means your message text is sent.\nFour words are here"),
        [
            "This means your message text is sent.",
            "Four words are here"
        ]
    );
}
