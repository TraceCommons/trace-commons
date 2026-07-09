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
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/' || c == '+' || c == '='
}

fn scan(hay: &str) -> Vec<(&'static str, usize)> {
    vec![
        ("anthropic sk-ant-", count_keyish(hay, "sk-ant-", 10, token_char)),
        ("github ghp_", count_keyish(hay, "ghp_", 10, token_char)),
        ("github gho_", count_keyish(hay, "gho_", 10, token_char)),
        ("github pat", count_keyish(hay, "github_pat_", 10, token_char)),
        ("aws AKIA", count_keyish(hay, "AKIA", 16, |c| c.is_ascii_uppercase() || c.is_ascii_digit())),
        ("google AIza", count_keyish(hay, "AIza", 30, token_char)),
        ("slack xoxb", count_keyish(hay, "xoxb-", 10, token_char)),
        ("npm token", count_keyish(hay, "npm_", 30, token_char)),
        ("PEM private key", hay.matches("-----BEGIN").filter(|_| true).count().min(hay.matches("PRIVATE KEY-----").count())),
        ("bearer header", count_keyish(&hay.to_ascii_lowercase(), "bearer ", 16, token_char)),
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
    }
}

#[tokio::test]
#[ignore]
async fn audit_real_sessions_for_key_leakage() {
    let root = dirs::home_dir().unwrap().join(".claude/projects");
    let src = ClaudeCodeSource::new(root);
    let refs = src.discover().expect("discover");
    let cfg = audit_cfg();

    let mut sessions_ok = 0usize;
    let mut sessions_skipped = 0usize;
    let mut leaks: Vec<(String, String, usize)> = Vec::new();
    let mut pre: std::collections::BTreeMap<&'static str, usize> = Default::default();
    let mut post: std::collections::BTreeMap<&'static str, usize> = Default::default();

    for r in &refs {
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
                leaks.push((name.to_string(), fname, n));
            }
        }
        sessions_ok += 1;
    }

    println!("sessions redacted: {sessions_ok}, skipped (parse/redact err): {sessions_skipped}");
    println!("pattern                 pre-redaction   post-redaction");
    for (name, n) in &pre {
        println!("  {:<22} {:>8}        {:>8}", name, n, post.get(name).copied().unwrap_or(0));
    }
    if !leaks.is_empty() {
        println!("LEAKS (pattern, session file, count):");
        for (p, f, n) in &leaks {
            println!("  {p} | {f} | {n}");
        }
    }
    assert!(leaks.is_empty(), "key-shaped strings survived redaction");
}
