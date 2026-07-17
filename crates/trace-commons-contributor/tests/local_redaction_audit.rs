//! THROWAWAY LOCAL AUDIT — never commit. Runs every real local Claude Code
//! session through the production redaction pipeline and re-scans the
//! serialized envelopes for key-shaped strings. Counts only; never prints
//! secret values. Run: cargo test --test local_redaction_audit -- --ignored --nocapture

use trace_commons_contributor::config::{CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ContributorConfig};
use trace_commons_contributor::envelope::{
    build_raw_contribution, build_redactor_with, redact_to_envelope,
};
use trace_commons_contributor::source::{TraceSource, claude_code::ClaudeCodeSource};

/// Count occurrences of `anchor` followed by at least `min_tail` chars from
/// `tail_class` (a rough token-shape check; recall over precision).
fn count_keyish(hay: &str, anchor: &str, min_tail: usize, tail_class: fn(char) -> bool) -> usize {
    let mut n = 0;
    let mut from = 0;
    while let Some(pos) = hay[from..].find(anchor) {
        let start = from + pos + anchor.len();
        let tail = hay[start..].chars().take_while(|c| tail_class(*c)).count();
        if tail >= min_tail {
            n += 1;
        }
        from = start;
    }
    n
}

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || c == '_'
        || c == '-'
        || c == '.'
        || c == '/'
        || c == '+'
        || c == '='
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
/// IDs, UUIDs, and content hashes are not secrets even at high entropy.
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
    if is_pure_hex(token) && matches!(token.len(), 7 | 8 | 40 | 64) {
        return true;
    }
    if token.len() >= 32
        && token
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return true;
    }
    false
}

const CUE_WINDOW: usize = 48;
const ENTROPY_MIN_LEN: usize = 16;
const ENTROPY_BITS_MIN: f64 = 3.2;

/// True for the detector cue regex's separator class: `[\x22'`:=\s]`.
fn cue_sep_char(c: char) -> bool {
    matches!(c, '"' | '\'' | '`' | ':' | '=') || c.is_whitespace()
}

/// Mirror of the detector's `secret_cue_regex`
/// (`(?i)(authorization|bearer|api[_-]?key|secret|password|passwd|
/// access[_-]?token|client[_-]?secret|private[_-]?key|token|apikey)
/// [\x22'`:=\s]{1,6}$`): true when `window` (already lowercased) ends with
/// one of the cue words followed immediately by 1-6 separator chars and
/// nothing else.
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
        if let Some(pos) = window.rfind(cue) {
            let tail = &window[pos + cue.len()..];
            let tail_len = tail.chars().count();
            if (1..=6).contains(&tail_len) && tail.chars().all(cue_sep_char) {
                return true;
            }
        }
    }
    false
}

/// Recall-oriented mirror of the detector's cue-gated high-entropy
/// catch-all: any run of entropy-candidate chars (len >= 16), not
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

/// Safe diagnostic for a surviving match: a short (<=4 char) prefix, the
/// full matched length, and its Shannon entropy. Never returns the full
/// secret value.
fn safe_diag(token: &str) -> (String, usize, f64) {
    let prefix: String = token.chars().take(4).collect();
    (prefix, token.chars().count(), token_shannon_entropy(token))
}

/// Locate one example occurrence of `pattern_name` in `hay` (mirroring the
/// matching logic in `scan`) and return a safe diagnostic for it, so a
/// LEAKS report can show *why* something survived without ever printing
/// the secret itself.
fn diag_for(hay: &str, pattern_name: &str) -> Option<(String, usize, f64)> {
    match pattern_name {
        "anthropic sk-ant-" => first_keyish(hay, "sk-ant-", 10, token_char).map(safe_diag),
        "github ghp_" => first_keyish(hay, "ghp_", 10, token_char).map(safe_diag),
        "github gho_" => first_keyish(hay, "gho_", 10, token_char).map(safe_diag),
        "github pat" => first_keyish(hay, "github_pat_", 10, token_char).map(safe_diag),
        "aws AKIA" => first_keyish(hay, "AKIA", 16, |c| {
            c.is_ascii_uppercase() || c.is_ascii_digit()
        })
        .map(safe_diag),
        "google AIza" => first_keyish(hay, "AIza", 30, token_char).map(safe_diag),
        "slack xoxb" => first_keyish(hay, "xoxb-", 10, token_char).map(safe_diag),
        "npm token" => first_keyish(hay, "npm_", 30, |c| c.is_ascii_alphanumeric()).map(safe_diag),
        "PEM private key" => hay.find("-----BEGIN").map(|start| {
            let end = hay[start..]
                .find("PRIVATE KEY-----")
                .map(|p| start + p + "PRIVATE KEY-----".len())
                .unwrap_or(hay.len());
            safe_diag(&hay[start..end.min(hay.len())])
        }),
        "bearer header" => {
            let lower = hay.to_ascii_lowercase();
            first_keyish(&lower, "bearer ", 16, token_char).map(safe_diag)
        }
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
        (
            "anthropic sk-ant-",
            count_keyish(hay, "sk-ant-", 10, token_char),
        ),
        ("github ghp_", count_keyish(hay, "ghp_", 10, token_char)),
        ("github gho_", count_keyish(hay, "gho_", 10, token_char)),
        (
            "github pat",
            count_keyish(hay, "github_pat_", 10, token_char),
        ),
        (
            "aws AKIA",
            count_keyish(hay, "AKIA", 16, |c| {
                c.is_ascii_uppercase() || c.is_ascii_digit()
            }),
        ),
        ("google AIza", count_keyish(hay, "AIza", 30, token_char)),
        ("slack xoxb", count_keyish(hay, "xoxb-", 10, token_char)),
        (
            "npm token",
            count_keyish(hay, "npm_", 30, |c| c.is_ascii_alphanumeric()),
        ),
        (
            "PEM private key",
            hay.matches("-----BEGIN")
                .filter(|_| true)
                .count()
                .min(hay.matches("PRIVATE KEY-----").count()),
        ),
        (
            "bearer header",
            count_keyish(&hay.to_ascii_lowercase(), "bearer ", 16, token_char),
        ),
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
        devfolio_submission_id: None,
    }
}

#[tokio::test]
#[ignore]
async fn audit_real_sessions_for_key_leakage() {
    let root = dirs::home_dir().unwrap().join(".claude/projects");
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
    let mut leaks: Vec<(String, String, usize, Option<(String, usize, f64)>)> = Vec::new();
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
                leaks.push((name.to_string(), fname, n, diag_for(&json, name)));
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
    if !leaks.is_empty() {
        println!(
            "LEAKS (pattern, session file, count, sample prefix/len/entropy — never the full secret):"
        );
        for (p, f, n, diag) in &leaks {
            match diag {
                Some((prefix, len, entropy)) => {
                    println!(
                        "  {p} | {f} | {n} | prefix={prefix:?} len={len} entropy={entropy:.2}"
                    );
                }
                None => println!("  {p} | {f} | {n} | (no sample located)"),
            }
        }
    }
    assert!(leaks.is_empty(), "key-shaped strings survived redaction");
}
