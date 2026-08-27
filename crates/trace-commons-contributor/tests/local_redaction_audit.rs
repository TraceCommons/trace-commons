//! Local redaction audit. Runs every real local Claude Code session through
//! the production redaction pipeline and re-scans the serialized envelopes
//! for key-shaped strings. Counts and structural shapes only; never prints
//! secret values.
//!
//! Run: `cargo test --test local_redaction_audit -- --ignored --nocapture`
//!
//! `#[ignore]`d because it reads whatever sessions happen to exist under
//! `~/.claude/projects`, so it is machine-dependent and cannot gate CI as
//! written. It is still the only empirical check that client-side redaction
//! holds against real transcripts, so treat a failure as real until the
//! shape signature in the LEAKS report says otherwise.
//!
//! ## Matchers must mirror the detector exactly
//!
//! Every matcher here is a hand-rolled mirror of a production detector regex
//! in `trace_contribution.rs`. Widening one "to be safe" is the opposite of
//! safe: this harness scans *post*-redaction text, so anything it matches
//! that the detector does not is reported as a surviving secret. A run in
//! July 2026 reported five leaks — an `sk-ant-` key, a `ghp_` token, a
//! bearer header, and two PEM private keys — every one of which was a false
//! positive from a session where the agent had been reading this very
//! redaction code. The causes were a character class admitting `.` and `/`
//! (so `sk-ant-EXAMPLE...` read as a 20-char key), minimum tails shorter
//! than the detector's, a missing `\b` boundary, and a PEM check that
//! counted `-----BEGIN` and `PRIVATE KEY-----` as independent substrings
//! (pairing a public-key header with a mention 1.28 MB away).
//!
//! `scan_does_not_flag_source_code_and_docs_about_secrets` pins that class
//! of input. Add a case to it before loosening any matcher.

use trace_commons_contributor::config::{CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ContributorConfig};
use trace_commons_contributor::envelope::{
    build_raw_contribution, build_redactor_with, redact_to_envelope,
};
use trace_commons_contributor::source::{TraceSource, claude_code::ClaudeCodeSource};

/// Regex `\w`: the class `\b` is defined against.
fn word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Count occurrences of `anchor` preceded by a `\b` boundary and followed by
/// at least `min_tail` chars from `tail_class`.
///
/// The leading boundary check is load-bearing: anchors are short, and without
/// it `sk-` matches inside `task-`, `ghp_` inside `highp_`, and so on. The
/// detector regexes all begin with `\b`, so a mirror that omits it reports
/// matches the detector never makes.
fn count_keyish(hay: &str, anchor: &str, min_tail: usize, tail_class: fn(char) -> bool) -> usize {
    let mut n = 0;
    let mut from = 0;
    while let Some(pos) = hay[from..].find(anchor) {
        let anchor_start = from + pos;
        let start = anchor_start + anchor.len();
        let boundary_ok = hay[..anchor_start]
            .chars()
            .next_back()
            .is_none_or(|c| !word_char(c));
        let tail = hay[start..].chars().take_while(|c| tail_class(*c)).count();
        if boundary_ok && tail >= min_tail {
            n += 1;
        }
        from = start;
    }
    n
}

/// The class shared by the `sk-`, `AIza`, and provider-token detector
/// regexes: `[A-Za-z0-9_-]`. Deliberately excludes `.`, `/`, `+`, and `=`.
/// Admitting those was the single largest source of false positives in this
/// harness: an elided placeholder like `sk-ant-EXAMPLE...` reads as a
/// 20-char key only if the dots count toward the tail.
fn detector_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// The class in `\bgh[pousr]_[A-Za-z0-9_]{10,}\b`. Narrower still: no
/// hyphen, so a slash- or hyphen-separated path beginning `ghp_` does not
/// read as a token.
fn github_tail_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Char class used by the detector's JWT regex segments
/// (`[A-Za-z0-9_-]`) — narrower than `token_char` so it doesn't run past
/// the `.` segment separators.
fn jwt_seg_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Manual mirror of the detector's JWT regex
/// (`\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b`):
/// header segment starting `eyJ` (>=10 trailing chars), `.`, payload segment
/// starting `eyJ` (>=10 trailing chars), `.`, signature segment (>=10 chars).
/// All matched chars are single-byte ASCII, so char counts double as byte
/// offsets for slicing.
fn count_jwt(hay: &str) -> usize {
    let mut n = 0;
    let mut from = 0;
    while let Some(pos) = hay[from..].find("eyJ") {
        let seg1_start = from + pos;
        let after1 = seg1_start + 3;
        let seg1_len = hay[after1..]
            .chars()
            .take_while(|c| jwt_seg_char(*c))
            .count();
        let seg1_end = after1 + seg1_len;
        'matched: {
            if seg1_len < 10 || !hay[seg1_end..].starts_with('.') {
                break 'matched;
            }
            let seg2_start = seg1_end + 1;
            if !hay[seg2_start..].starts_with("eyJ") {
                break 'matched;
            }
            let after2 = seg2_start + 3;
            let seg2_len = hay[after2..]
                .chars()
                .take_while(|c| jwt_seg_char(*c))
                .count();
            let seg2_end = after2 + seg2_len;
            if seg2_len < 10 || !hay[seg2_end..].starts_with('.') {
                break 'matched;
            }
            let seg3_start = seg2_end + 1;
            let seg3_len = hay[seg3_start..]
                .chars()
                .take_while(|c| jwt_seg_char(*c))
                .count();
            if seg3_len >= 10 {
                n += 1;
            }
        }
        from = seg1_start + 3;
    }
    n
}

/// Char class used by the detector's contextual-entropy candidate regex
/// (`[A-Za-z0-9+/=_.\-]{16,}`).
fn entropy_candidate_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '.' | '-')
}

/// Shannon entropy in bits/char, mirroring the detector's
/// `token_shannon_entropy`.
fn token_shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let mut counts: std::collections::HashMap<u8, usize> = std::collections::HashMap::new();
    for byte in s.bytes() {
        *counts.entry(byte).or_insert(0) += 1;
    }
    let len = s.len() as f64;
    counts
        .values()
        .map(|&count| {
            let p = count as f64 / len;
            -p * p.log2()
        })
        .sum()
}

fn is_pure_hex(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_uuid_shape(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && s.bytes().enumerate().all(|(i, b)| {
            matches!(i, 8 | 13 | 18 | 23)
                || (b as char).is_ascii_hexdigit() && !(b as char).is_ascii_uppercase()
        })
}

const ALLOWLISTED_ID_PREFIXES: &[&str] = &[
    "msg_", "req_", "mcp_", "toolu_", "chatcmpl", "run_", "file_", "asst_", "resp_", "call_",
];

/// Exact `RedactionReport` metric-key label fragments the production
/// pipeline emits via `report.increment("secret:<label>")` /
/// `report.increment("secret:contextual_entropy")` (see
/// `trace_contribution.rs`, e.g. lines 2402, 2425, 3843). These are
/// diagnostic counter names describing *how many* secrets of each shape
/// were redacted, embedded in the envelope as metadata alongside the word
/// "secret:" — not secret content themselves. Scanning the whole
/// serialized envelope (as this harness does) means the harness's own
/// cue-gated-entropy check would otherwise flag its own counter names as
/// "leaks" whenever the detector successfully redacted something of that
/// shape. Exact-match only: a real secret value is astronomically
/// unlikely to equal one of these literal label strings verbatim.
const REPORT_METRIC_LABELS: &[&str] = &[
    "contextual_entropy",
    "openai_api_key",
    "github_token",
    "aws_access_key",
    "provider_token",
    "npm_token",
    "google_api_key",
    "pem_header_orphan",
    "pem_private_key",
];

/// Mirror of the detector's `is_allowlisted_entropy_candidate`: structural
/// IDs, UUIDs, and short git SHAs are not secrets even at high entropy.
/// Content-hash hex ≥32 / 40 / 64 is intentionally absent when cued (#193
/// row 4); this mirror is only consulted after a cue, matching production.
/// Also excludes this harness's own false-positive surface: the redaction
/// pipeline's report metric label names (see `REPORT_METRIC_LABELS`).
fn is_allowlisted_entropy_candidate(token: &str) -> bool {
    if is_uuid_shape(token) {
        return true;
    }
    if ALLOWLISTED_ID_PREFIXES.iter().any(|p| token.starts_with(p)) {
        return true;
    }
    if REPORT_METRIC_LABELS.contains(&token) {
        return true;
    }
    if is_pure_hex(token) && matches!(token.len(), 7 | 8) {
        return true;
    }
    false
}

/// Patterns that are deliberately BROADER than any production detector, and
/// so must not fail the run.
///
/// Every other matcher here mirrors a detector exactly, which makes a
/// post-redaction hit a genuine leak. `bearer value` is different by design:
/// production covers bearer tokens through the cue-gated entropy rule, and
/// #193 closed the short-cued and cued-hex evasions -- the latter having since
/// regressed and been restored (#432) -- but UUID-shaped and low-entropy
/// tokens still evade (plus accepted zero-separator glue).
/// This matcher exists to make that residual visible, so its hits are
/// expected until a dedicated bearer rule lands. Reporting them as hard
/// failures would leave the audit permanently red and train everyone to
/// ignore it -- the exact failure mode that let five false positives sit
/// unexamined. They are printed for triage instead.
const ADVISORY_PATTERNS: &[&str] = &["bearer value"];

// Hand-written mirror of the detector's constants in
// `trace-commons-protocol/src/trace_contribution.rs`. Parity is this file's
// entire purpose: a mirror that disagrees with the detector reports on inputs
// the detector never considered.
//
// ENTROPY_MIN_LEN was left at 8 when #225 raised the detector to 16, so the
// audit over-reported every 8-to-15 character value the detector had no
// intention of redacting (#432). Advisory-only, so it cost noise rather than a
// missed secret -- but noise in an audit is how real findings get ignored.
const CUE_WINDOW: usize = 48;
const ENTROPY_MIN_LEN: usize = 16;
const ENTROPY_BITS_MIN: f64 = 3.2;

/// True for the detector cue regex's separator class: `[\x22'`:=\s]`.
fn cue_sep_char(c: char) -> bool {
    matches!(c, '"' | '\'' | '`' | ':' | '=') || c.is_whitespace()
}

/// True for the detector cue regex's trailing-identifier class:
/// `[A-Za-z0-9_-]`.
fn cue_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Mirror of the detector's `secret_cue_regex`
/// (`(?i)(authorization|bearer|api[_-]?key|secret|password|passwd|
/// access[_-]?token|client[_-]?secret|private[_-]?key|token|apikey)
/// [A-Za-z0-9_-]*[\x22'`:=\s]{1,6}$`): true when `window` (already
/// lowercased) ends with one of the cue words, then any run of identifier
/// chars, then 1-6 separator chars and nothing else.
fn window_has_cue(window: &str) -> bool {
    const CUES: &[&str] = &[
        "authorization",
        "bearer",
        "api_key",
        "api-key",
        "apikey",
        "secret",
        "password",
        "passwd",
        "access_token",
        "access-token",
        "client_secret",
        "client-secret",
        "private_key",
        "private-key",
        "token",
    ];
    for cue in CUES {
        // Every occurrence, not just the last: the trailing identifier run
        // means an earlier occurrence can reach the separator when a later
        // one cannot.
        for (pos, _) in window.match_indices(cue) {
            let tail = &window[pos + cue.len()..];
            let sep = tail.trim_start_matches(cue_ident_char);
            let sep_len = sep.chars().count();
            if (1..=6).contains(&sep_len) && sep.chars().all(cue_sep_char) {
                return true;
            }
        }
    }
    false
}

/// Recall-oriented mirror of the detector's cue-gated high-entropy
/// catch-all: any run of entropy-candidate chars (len >= 8), not
/// allowlisted, with Shannon entropy >= 3.2 bits/char, immediately preceded
/// (within the 48-char window, and immediately abutting the candidate
/// modulo 1-6 separator chars) by a secret-shaped cue word.
fn count_cue_gated_entropy(hay: &str) -> usize {
    let lower = hay.to_ascii_lowercase();
    let mut n = 0;
    let mut i = 0;
    let bytes = hay.as_bytes();
    while i < bytes.len() {
        let c = hay[i..].chars().next().unwrap();
        if !entropy_candidate_char(c) {
            i += c.len_utf8();
            continue;
        }
        let start = i;
        let mut end = i;
        for c in hay[i..].chars() {
            if entropy_candidate_char(c) {
                end += c.len_utf8();
            } else {
                break;
            }
        }
        let token = &hay[start..end];
        if token.len() >= ENTROPY_MIN_LEN
            && !is_allowlisted_entropy_candidate(token)
            && token_shannon_entropy(token) >= ENTROPY_BITS_MIN
        {
            let window_start = start.saturating_sub(CUE_WINDOW);
            let mut ws = window_start;
            while ws > 0 && !hay.is_char_boundary(ws) {
                ws -= 1;
            }
            let window = &lower[ws..start];
            if window_has_cue(window) {
                n += 1;
            }
        }
        i = end;
    }
    n
}

/// Structural signature of a token with every information-bearing
/// character erased: lowercase letters become `a`, uppercase `A`, digits
/// `9`, whitespace `_`, and punctuation is kept verbatim. Runs of the same
/// class collapse to `class{n}`.
///
/// This distinguishes a live credential from a placeholder or a source-code
/// literal without ever revealing the value. `sk-ant-api03-Xy7...` shapes as
/// `a{2}-a{3}-a{3}9{2}-...`, whereas the regex literal
/// `-----BEGIN [A-Z ]*PRIVATE KEY-----` shapes with its brackets intact.
/// A leak report that cannot tell those apart cannot be acted on.
fn shape_signature(token: &str, max_runs: usize) -> String {
    let class_of = |c: char| -> char {
        if c.is_ascii_lowercase() {
            'a'
        } else if c.is_ascii_uppercase() {
            'A'
        } else if c.is_ascii_digit() {
            '9'
        } else if c.is_whitespace() {
            '_'
        } else {
            c
        }
    };
    let mut out = String::new();
    let mut runs = 0;
    let mut chars = token.chars().map(class_of).peekable();
    while let Some(c) = chars.next() {
        if runs >= max_runs {
            out.push_str("...");
            break;
        }
        let mut n = 1;
        while chars.peek() == Some(&c) {
            chars.next();
            n += 1;
        }
        if matches!(c, 'a' | 'A' | '9' | '_') && n > 1 {
            out.push_str(&format!("{c}{{{n}}}"));
        } else {
            for _ in 0..n {
                out.push(c);
            }
        }
        runs += 1;
    }
    out
}

/// Locate a real private-key header: `-----BEGIN`, then only uppercase and
/// spaces, then `PRIVATE KEY-----`, contiguously. Returns the byte range of
/// the header.
///
/// Mirrors the production `-----BEGIN [A-Z ]*PRIVATE KEY-----` used by both
/// the whole-block redaction and the `pem_header_orphan` pattern. The prior
/// implementation counted `-----BEGIN` and `PRIVATE KEY-----` as independent
/// substrings and took the minimum, which paired a `-----BEGIN PUBLIC
/// KEY-----` header with a mention of `PRIVATE KEY-----` elsewhere in the
/// document. That is how a single "leak" came to span 1.28 MB.
fn find_pem_header(hay: &str, from: usize) -> Option<std::ops::Range<usize>> {
    const OPEN: &str = "-----BEGIN ";
    const CLOSE: &str = "PRIVATE KEY-----";
    let mut search = from;
    while let Some(pos) = hay[search..].find(OPEN) {
        let start = search + pos;
        let after_open = start + OPEN.len();
        let run_len = hay[after_open..]
            .chars()
            .take_while(|c| c.is_ascii_uppercase() || *c == ' ')
            .count();
        // `[A-Z ]*` is greedy but backtracks. `RSA PRIVATE KEY` is entirely
        // uppercase-and-space, so consuming the whole run overshoots the
        // `PRIVATE KEY-----` we need to land on. Walk every split point, as
        // the regex engine would. `-----BEGIN PUBLIC KEY-----` has no split
        // point that works, so it is correctly rejected.
        for split in after_open..=after_open + run_len {
            if hay[split..].starts_with(CLOSE) {
                return Some(start..split + CLOSE.len());
            }
        }
        search = after_open;
    }
    None
}

fn count_pem_private_key(hay: &str) -> usize {
    let mut n = 0;
    let mut from = 0;
    while let Some(range) = find_pem_header(hay, from) {
        n += 1;
        from = range.end;
    }
    n
}

/// Count credential-shaped values following a `Bearer ` header.
///
/// This matcher deliberately does NOT mirror a single detector regex, and it
/// is the one place in this file where being broader than production is
/// correct. Production covers bearer tokens through the cue-gated entropy
/// rule. #193 closed the short-cued (8–15) and cued-lowercase-hex≥32
/// evasions; a realistic opaque token can still evade in two deliberate
/// ways — a UUID-shaped token is allowlisted, and a low-entropy static
/// credential falls under the 3.2 bits/char threshold — plus the accepted
/// zero-separator form (`BearerSECRET`). A bearer header is an unambiguous
/// declaration that what follows is a credential, so anything of plausible
/// token shape after it is worth surfacing even when production would not
/// redact it.
///
/// Prose is excluded by requiring at least one digit or uppercase character:
/// "bearer per-a-slot-based" is all-lowercase kebab case, whereas real
/// opaque tokens essentially always carry mixed case or digits. That one
/// condition is what separates this from the loose matcher removed earlier,
/// which fired on exactly that phrase.
fn count_bearer_values(hay: &str) -> usize {
    const MIN_LEN: usize = 8;
    let lower = hay.to_ascii_lowercase();
    let mut n = 0;
    let mut from = 0;
    while let Some(pos) = lower[from..].find("bearer ") {
        let start = from + pos + "bearer ".len();
        let token: String = hay[start..]
            .chars()
            .take_while(|c| entropy_candidate_char(*c))
            .collect();
        let looks_like_a_token = token.len() >= MIN_LEN
            && token
                .chars()
                .any(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
            && !is_placeholder_identifier(&token);
        if looks_like_a_token {
            n += 1;
        }
        from = start;
    }
    n
}

/// True for screaming-snake identifiers such as `YOUR_API_KEY` or
/// `SLACK_BOT_TOKEN`. These are environment-variable names and
/// documentation placeholders, never credential values, and they dominated
/// this matcher's output on a real corpus before being excluded.
fn is_placeholder_identifier(token: &str) -> bool {
    token.contains('_')
        && token
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// First `Bearer `-following value, for the leak diagnostic.
fn first_bearer_value(hay: &str) -> Option<&str> {
    let lower = hay.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find("bearer ") {
        let start = from + pos + "bearer ".len();
        let len = hay[start..]
            .chars()
            .take_while(|c| entropy_candidate_char(*c))
            .count();
        let token = &hay[start..start + len];
        if token.len() >= 8
            && token
                .chars()
                .any(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
            && !is_placeholder_identifier(token)
        {
            return Some(token);
        }
        from = start;
    }
    None
}

/// Safe diagnostic for a surviving match: the matched length, its Shannon
/// entropy, and a structural shape signature. Never returns the value, and
/// deliberately no raw prefix: the pattern name already names the credential
/// family, so a prefix adds nothing but real secret bytes to the output.
fn safe_diag(token: &str) -> (usize, f64, String) {
    (
        token.chars().count(),
        token_shannon_entropy(token),
        shape_signature(token, 24),
    )
}

/// Locate one example occurrence of `pattern_name` in `hay` (mirroring the
/// matching logic in `scan`) and return a safe diagnostic for it, so a
/// LEAKS report can show *why* something survived without ever printing
/// the secret itself.
fn diag_for(hay: &str, pattern_name: &str) -> Option<(usize, f64, String)> {
    match pattern_name {
        "anthropic sk-ant-" => first_keyish(hay, "sk-", 20, detector_token_char).map(safe_diag),
        "github ghp_" => first_keyish(hay, "ghp_", 10, github_tail_char).map(safe_diag),
        "github gh*_" => ["gho_", "ghu_", "ghs_", "ghr_"]
            .iter()
            .find_map(|a| first_keyish(hay, a, 10, github_tail_char))
            .map(safe_diag),
        "github pat" => first_keyish(hay, "github_pat_", 10, github_tail_char).map(safe_diag),
        "aws AKIA" => first_keyish(hay, "AKIA", 16, |c| {
            c.is_ascii_uppercase() || c.is_ascii_digit()
        })
        .map(safe_diag),
        "google AIza" => first_keyish(hay, "AIza", 35, detector_token_char).map(safe_diag),
        "provider token" => ["rk", "pk", "glpat", "xoxb", "xoxa", "xoxp", "xoxr", "xoxs"]
            .iter()
            .find_map(|a| first_keyish(hay, a, 8, detector_token_char))
            .map(safe_diag),
        "npm token" => first_keyish(hay, "npm_", 36, |c| c.is_ascii_alphanumeric()).map(safe_diag),
        "PEM private key" => find_pem_header(hay, 0).map(|r| safe_diag(&hay[r])),
        "bearer value" => first_bearer_value(hay).map(safe_diag),
        "jwt (eyJ.eyJ.)" => first_jwt(hay).map(safe_diag),
        "cue-gated entropy" => first_cue_gated_entropy(hay).map(safe_diag),
        _ => None,
    }
}

/// Like `count_keyish` but returns the first matched slice (anchor + tail)
/// instead of a count.
fn first_keyish<'a>(
    hay: &'a str,
    anchor: &str,
    min_tail: usize,
    tail_class: fn(char) -> bool,
) -> Option<&'a str> {
    let mut from = 0;
    while let Some(pos) = hay[from..].find(anchor) {
        let start = from + pos + anchor.len();
        let tail = hay[start..].chars().take_while(|c| tail_class(*c)).count();
        if tail >= min_tail {
            return Some(&hay[from + pos..start + tail]);
        }
        from = start;
    }
    None
}

/// Like `count_jwt` but returns the first matched slice.
fn first_jwt(hay: &str) -> Option<&str> {
    let mut from = 0;
    while let Some(pos) = hay[from..].find("eyJ") {
        let seg1_start = from + pos;
        let after1 = seg1_start + 3;
        let seg1_len = hay[after1..]
            .chars()
            .take_while(|c| jwt_seg_char(*c))
            .count();
        let seg1_end = after1 + seg1_len;
        'matched: {
            if seg1_len < 10 || !hay[seg1_end..].starts_with('.') {
                break 'matched;
            }
            let seg2_start = seg1_end + 1;
            if !hay[seg2_start..].starts_with("eyJ") {
                break 'matched;
            }
            let after2 = seg2_start + 3;
            let seg2_len = hay[after2..]
                .chars()
                .take_while(|c| jwt_seg_char(*c))
                .count();
            let seg2_end = after2 + seg2_len;
            if seg2_len < 10 || !hay[seg2_end..].starts_with('.') {
                break 'matched;
            }
            let seg3_start = seg2_end + 1;
            let seg3_len = hay[seg3_start..]
                .chars()
                .take_while(|c| jwt_seg_char(*c))
                .count();
            if seg3_len >= 10 {
                return Some(&hay[seg1_start..seg3_start + seg3_len]);
            }
        }
        from = seg1_start + 3;
    }
    None
}

/// Like `count_cue_gated_entropy` but returns the first matched token slice.
fn first_cue_gated_entropy(hay: &str) -> Option<&str> {
    let lower = hay.to_ascii_lowercase();
    let mut i = 0;
    let bytes = hay.as_bytes();
    while i < bytes.len() {
        let c = hay[i..].chars().next().unwrap();
        if !entropy_candidate_char(c) {
            i += c.len_utf8();
            continue;
        }
        let start = i;
        let mut end = i;
        for c in hay[i..].chars() {
            if entropy_candidate_char(c) {
                end += c.len_utf8();
            } else {
                break;
            }
        }
        let token = &hay[start..end];
        if token.len() >= ENTROPY_MIN_LEN
            && !is_allowlisted_entropy_candidate(token)
            && token_shannon_entropy(token) >= ENTROPY_BITS_MIN
        {
            let window_start = start.saturating_sub(CUE_WINDOW);
            let mut ws = window_start;
            while ws > 0 && !hay.is_char_boundary(ws) {
                ws -= 1;
            }
            let window = &lower[ws..start];
            if window_has_cue(window) {
                return Some(token);
            }
        }
        i = end;
    }
    None
}

fn scan(hay: &str) -> Vec<(&'static str, usize)> {
    vec![
        // Each anchor/min-tail/class triple below mirrors the corresponding
        // production detector regex EXACTLY. Using a wider class or a shorter
        // minimum than the detector does not make the harness more cautious —
        // it makes it report survivors the detector was never going to
        // redact, which is indistinguishable from a real leak in the output.
        // Mirrors `\bsk-[A-Za-z0-9_-]{20,}\b`.
        (
            "anthropic sk-ant-",
            count_keyish(hay, "sk-", 20, detector_token_char),
        ),
        // Mirrors `\bgh[pousr]_[A-Za-z0-9_]{10,}\b` — note no hyphen.
        (
            "github ghp_",
            count_keyish(hay, "ghp_", 10, github_tail_char),
        ),
        (
            "github gh*_",
            ["gho_", "ghu_", "ghs_", "ghr_"]
                .iter()
                .map(|a| count_keyish(hay, a, 10, github_tail_char))
                .sum(),
        ),
        (
            "github pat",
            count_keyish(hay, "github_pat_", 10, github_tail_char),
        ),
        // Mirrors `\bAKIA[0-9A-Z]{16}\b`.
        (
            "aws AKIA",
            count_keyish(hay, "AKIA", 16, |c| {
                c.is_ascii_uppercase() || c.is_ascii_digit()
            }),
        ),
        // Mirrors `\bAIza[0-9A-Za-z_-]{35,}\b`.
        (
            "google AIza",
            count_keyish(hay, "AIza", 35, detector_token_char),
        ),
        // Mirrors the full provider-token regex
        // `(?i)\b(?:rk|pk|glpat|xox[baprs])[-_a-z0-9]{8,}\b`. Checking only
        // `xoxb-` left the harness NARROWER than production, which is its own
        // kind of blind spot.
        (
            "provider token",
            ["rk", "pk", "glpat", "xoxb", "xoxa", "xoxp", "xoxr", "xoxs"]
                .iter()
                .map(|a| count_keyish(hay, a, 8, detector_token_char))
                .sum(),
        ),
        // Mirrors `\bnpm_[A-Za-z0-9]{36}\b`.
        (
            "npm token",
            count_keyish(hay, "npm_", 36, |c| c.is_ascii_alphanumeric()),
        ),
        ("PEM private key", count_pem_private_key(hay)),
        ("bearer value", count_bearer_values(hay)),
        ("jwt (eyJ.eyJ.)", count_jwt(hay)),
        ("cue-gated entropy", count_cue_gated_entropy(hay)),
    ]
}

fn audit_cfg() -> ContributorConfig {
    ContributorConfig {
        schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
        issuer_url: "https://issuer.tracecommons.ai".into(),
        ingest_url: "https://ingest.example".into(),
        audience: "trace-commons-upload".into(),
        tenant_id: "tenant-local-audit".into(),
        instance_id: "local-audit".into(),
        user_subject: "local-audit".into(),
        device_key_id: "sha256:00".into(),
        consent_scopes: vec!["debugging_evaluation".into()],
        pii_filter: None,
        allowed_hosts: None,
        display_handle: None,
        public_bio: None,
        public_since: None,
    }
}

#[tokio::test]
#[ignore]
async fn audit_real_sessions_for_key_leakage() {
    // Default: every session under ~/.claude/projects (~1000 files / 676 MB
    // on the author's machine, about two minutes). Set
    // `TRACE_AUDIT_SESSION_ROOT` to point at a curated directory instead —
    // useful for iterating on a matcher against a handful of known-tricky
    // sessions rather than re-scanning the whole corpus each time.
    let root = match std::env::var("TRACE_AUDIT_SESSION_ROOT") {
        Ok(path) if !path.trim().is_empty() => std::path::PathBuf::from(path),
        _ => dirs::home_dir().unwrap().join(".claude/projects"),
    };
    println!("session root: {}", root.display());
    let src = ClaudeCodeSource::new(root);
    let refs = src.discover().expect("discover");
    let cfg = audit_cfg();

    // The Claude Code session driving *this* test run is itself being
    // appended to while the audit executes (and, in the course of building
    // this harness, had redactor source/test-fixture text pasted into its
    // own transcript via tool output). A session file that is actively
    // being written during its own audit is not a valid, stable audit
    // target, so it is excluded rather than treated as a leak.
    let live_session_id = std::env::var("CLAUDE_CODE_SESSION_ID").ok();

    let mut sessions_ok = 0usize;
    let mut sessions_skipped = 0usize;
    let mut sessions_excluded_live = 0usize;
    let mut leaks: Vec<(String, String, usize, Option<(usize, f64, String)>)> = Vec::new();
    let mut advisories: Vec<(String, String, usize, Option<(usize, f64, String)>)> = Vec::new();
    let mut pre: std::collections::BTreeMap<&'static str, usize> = Default::default();
    let mut post: std::collections::BTreeMap<&'static str, usize> = Default::default();

    for r in &refs {
        if let Some(live_id) = &live_session_id {
            let is_live = r
                .path
                .file_stem()
                .map(|s| s.to_string_lossy() == live_id.as_str())
                .unwrap_or(false);
            if is_live {
                sessions_excluded_live += 1;
                continue;
            }
        }
        let raw_bytes = std::fs::read_to_string(&r.path).unwrap_or_default();
        for (name, n) in scan(&raw_bytes) {
            *pre.entry(name).or_default() += n;
        }
        let t = match src.load(r) {
            Ok(t) => t,
            Err(_) => {
                sessions_skipped += 1;
                continue;
            }
        };
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        let redactor = build_redactor_with(&cfg, t.cwd.as_deref(), None).expect("redactor");
        let envelope = match redact_to_envelope(&redactor, raw).await {
            Ok(e) => e,
            Err(_) => {
                sessions_skipped += 1;
                continue;
            }
        };
        let json = serde_json::to_string(&envelope).expect("serialize");
        for (name, n) in scan(&json) {
            *post.entry(name).or_default() += n;
            if n > 0 {
                let fname = r
                    .path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                if ADVISORY_PATTERNS.contains(&name) {
                    advisories.push((name.to_string(), fname, n, diag_for(&json, name)));
                } else {
                    leaks.push((name.to_string(), fname, n, diag_for(&json, name)));
                }
            }
        }
        sessions_ok += 1;
    }

    println!(
        "sessions redacted: {sessions_ok}, skipped (parse/redact err): {sessions_skipped}, excluded (live/self session): {sessions_excluded_live}"
    );
    println!("pattern                 pre-redaction   post-redaction");
    for (name, n) in &pre {
        println!(
            "  {:<22} {:>8}        {:>8}",
            name,
            n,
            post.get(name).copied().unwrap_or(0)
        );
    }
    if !advisories.is_empty() {
        println!("ADVISORY (deliberately broader than production; triage, does not fail the run):");
        for (p, f, n, diag) in &advisories {
            match diag {
                Some((len, entropy, shape)) => {
                    println!("  {p} | {f} | {n} | len={len} entropy={entropy:.2} shape={shape}");
                }
                None => println!("  {p} | {f} | {n}"),
            }
        }
    }
    if !leaks.is_empty() {
        println!(
            "LEAKS (pattern, session file, count, sample len/entropy/shape — never the full secret):"
        );
        for (p, f, n, diag) in &leaks {
            match diag {
                Some((len, entropy, shape)) => {
                    println!("  {p} | {f} | {n} | len={len} entropy={entropy:.2} shape={shape}");
                }
                None => println!("  {p} | {f} | {n} | (no sample located)"),
            }
        }
    }
    assert!(leaks.is_empty(), "key-shaped strings survived redaction");
}

/// Text of the exact kind this harness historically mis-flagged: a session in
/// which the agent read and discussed *secret-handling source code*. Every
/// string below is documentation, a regex literal, a public-key header, or a
/// placeholder. None is a credential, so `scan` must report zero for all of
/// them. Each case here corresponds to a real false positive this harness
/// produced against live sessions.
const SECRET_SHAPED_PROSE: &str = concat!(
    // Doc comment describing the PEM block regex (trace_contribution.rs).
    "/// `-----BEGIN ... PRIVATE KEY-----` .. `-----END ... PRIVATE KEY-----`\n",
    // A public key header. Not secret, and not a private key.
    "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE\n-----END PUBLIC KEY-----\n",
    // The words appear far apart and unrelated; counting them independently
    // pairs the public header above with this mention.
    "The orphan rule matches a bare PRIVATE KEY----- header with no body.\n",
    // Regex literals naming token shapes.
    "Regex::new(r\"\\bgh[pousr]_[A-Za-z0-9_]{10,}\\b\")\n",
    "Regex::new(r\"\\bsk-[A-Za-z0-9_-]{20,}\\b\")\n",
    // A path-like string that starts with a token prefix but contains slashes.
    "see ghp_/DOCS/ATna/ghp_/AAA/BBB for the naming scheme\n",
    // An elided placeholder. The trailing "..." is literal: `.` is outside
    // the detector's [A-Za-z0-9_-] class, so the detector correctly stops
    // short of its 20-char minimum and leaves this alone. The harness's
    // wider class ran straight through the dots and called it a key.
    "ANTHROPIC key looks like sk-ant-EXAMPLEabcdef...\n",
    // Kebab-case prose following the word bearer.
    "the bearer per-a-slot-based scheme\n",
);

/// Credential-shaped strings that MUST still be counted. Synthetic, and
/// deliberately built to the exact shapes the production detector regexes
/// match, so a matcher tightened too far fails here rather than silently
/// going blind.
const REAL_SECRET_SHAPES: &str = concat!(
    "ghp_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789\n",
    "sk-ant-api03-QQvXbTnWzRkLmPjHgFdSaZxCvBnM\n",
    "AKIAIOSFODNN7EXAMPLE\n",
    "AIzaSyD-abcdefghijklmnopqrstuvwxyz0123456789\n",
    "npm_abcdefghijklmnopqrstuvwxyz0123456789AB\n",
    "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA0123456789abcdef\n-----END RSA PRIVATE KEY-----\n",
);

#[test]
fn scan_does_not_flag_source_code_and_docs_about_secrets() {
    // `cue-gated entropy` is excluded deliberately, and only it. That check
    // already mirrors the detector faithfully (283 candidates, 0 survivors
    // against real sessions), and it fires *pre*-redaction on any
    // high-entropy token next to a cue word — including the harmless
    // "bearer per-a-slot-based" below, which the detector then redacts. A
    // pre-redaction hit it will remove is correct behavior, not a false
    // positive. The shape matchers are different: nothing downstream
    // removes what they flag, so a hit from them is reported as a survivor.
    let hits: Vec<(&str, usize)> = scan(SECRET_SHAPED_PROSE)
        .into_iter()
        .filter(|(name, n)| *n > 0 && *name != "cue-gated entropy")
        .collect();
    assert!(
        hits.is_empty(),
        "prose and source code about secrets must not count as leaks, got: {hits:?}"
    );
}

#[test]
fn scan_still_counts_real_credential_shapes() {
    let counts: std::collections::HashMap<&str, usize> =
        scan(REAL_SECRET_SHAPES).into_iter().collect();
    for pattern in [
        "github ghp_",
        "anthropic sk-ant-",
        "aws AKIA",
        "google AIza",
        "npm token",
        "PEM private key",
    ] {
        assert!(
            counts.get(pattern).copied().unwrap_or(0) > 0,
            "{pattern} must still be detected; tightening went too far. counts={counts:?}"
        );
    }
}

/// Bearer values that the production cue-gated entropy rule does NOT redact.
/// Each line encodes one documented evasion, so if the detector is ever
/// hardened these become the regression cases proving it.
const BEARER_EVASIONS: &str = concat!(
    // UUID-shaped: explicitly allowlisted by is_allowlisted_entropy_candidate.
    "Authorization: Bearer 3f2504e0-4f89-11d3-9a0c-0305e82c3301\n",
    // Lowercase hex, 32+ chars. NO LONGER a detector evasion: the cued-hex
    // narrowing (#432) removed the content-hash allowlist from the cued path,
    // and production now redacts this. Kept here because this fixture measures
    // the audit matcher's own breadth, not the detector's -- the matcher must
    // still surface it. The regression case proving the detector catches it is
    // `a_cued_lowercase_hex_bearer_value_is_no_longer_an_evasion` in
    // trace-commons-protocol.
    "Authorization: Bearer 9f86d081884c7d659a2feaa0c55ad015\n",
    // Under the 16-char entropy-candidate minimum.
    "Authorization: Bearer Tk9QRTEyMw\n",
    // Long but low entropy: below the 3.2 bits/char threshold.
    "Authorization: Bearer AAAAAAAAAAAAAAAAAAAAAAAA1\n",
);

#[test]
fn bearer_matcher_catches_values_the_detector_misses() {
    // This matcher is intentionally broader than production. It exists to
    // surface the gap, not to mirror it -- see count_bearer_values.
    let counts: std::collections::HashMap<&str, usize> =
        scan(BEARER_EVASIONS).into_iter().collect();
    assert_eq!(
        counts.get("bearer value").copied().unwrap_or(0),
        4,
        "every documented bearer evasion must be surfaced, counts={counts:?}"
    );
}

#[test]
fn bearer_matcher_ignores_prose() {
    // The matcher this replaced fired on exactly this phrase, which is what
    // made it look like a leak. All-lowercase kebab case is prose, not a
    // credential.
    let counts: std::collections::HashMap<&str, usize> =
        scan("the bearer per-a-slot-based scheme\n")
            .into_iter()
            .collect();
    assert_eq!(counts.get("bearer value").copied().unwrap_or(0), 0);
}
