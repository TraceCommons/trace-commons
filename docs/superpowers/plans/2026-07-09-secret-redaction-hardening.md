# Secret Redaction Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Execution model: Sonnet 5 implementers, controller (Fable) reviews between tasks. PROCESS RULE for every implementer: run all cargo commands in the FOREGROUND and wait inline; never background builds/tests, never arm monitors, never end a turn to "wait". NEVER run bare `cargo fmt` on the server crate (reformats 70k-line files) — format only touched files and confirm `git status` shows only intended files before committing.

**Goal:** Close the secret-leak gaps found by auditing real local Claude Code transcripts through the production redactor — broaden the pattern set, fix PEM whole-block redaction, add cue-gated high-entropy detection for unknown key formats, and wire a per-session fail-closed leaked-token guard — until the audit shows 0 surviving secrets across all local sessions.

**Architecture:** Three layers, all in the protocol crate's `DeterministicTraceRedactor` except the guard. (1) `SecretLeakDetector::scan` gains more regex patterns and a PEM whole-block fix. (2) A new cue-gated contextual-entropy pass runs inside `redact_text_with_state` after the pattern scan, redacting high-entropy tokens that sit next to a secret cue while allowlisting IDs/UUIDs/base64-content. (3) The contributor `submit` pipeline gains a per-session guard: after redaction, compare the original session text against the serialized redacted envelope with `canary_leaked_tokens`; any residual verbatim secret fails that session closed. The audit harness becomes the acceptance gate.

**Tech Stack:** Rust edition 2024. Existing deps only — `regex` (already in protocol crate), `sha2` (already), operator-client's `canary_leaked_tokens`. No new dependencies, no schema/migration changes.

## Global Constraints

- No new external dependencies. No schema migrations.
- Verify every task with `RUSTFLAGS="-D warnings"` for check/test on touched crates; CI clippy: `cargo clippy --workspace --all-targets -- -D warnings -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`; `cargo fmt -p <touched crate> -- --check`.
- Redaction is fail-closed: over-redaction (a false positive that removes a non-secret) is acceptable; under-redaction (a real secret surviving) is a defect. When entropy is ambiguous, redact.
- Hash-only/label-only surfaces: never log, print, or store a raw secret value or a raw path. The leaked-token guard reports counts and sha256 hashes only (that is what `canary_leaked_tokens` returns).
- No emojis. Commit subjects: short imperative, no `feat:`/`fix:` prefixes.
- Bumping redactor logic requires bumping `DETERMINISTIC_REDACTION_PIPELINE_VERSION` so stored envelopes record which pipeline scrubbed them.
- The audit test `crates/trace-commons-contributor/tests/local_redaction_audit.rs` is a LOCAL, developer-only, `#[ignore]`d test that reads `~/.claude/projects`. It must stay `#[ignore]`d (CI has no such data and must not depend on machine state). It is the human acceptance gate, not a CI gate.

## Key facts (single source of truth; file:line as of branch head, protocol crate = crates/trace-commons-protocol/src/trace_contribution.rs)

- `SecretLeakDetector` unit struct (:2041); `fn scan(&self, content: &str) -> SecretLeakScan` (:2048) iterates `secret_leak_patterns()` running `pattern.regex.find_iter`. `SecretLeakSeverity { High, Critical }` (:2016); `SecretLeakMatch { pattern_name: &'static str, severity, location: Range<usize> }` (:2022); `SecretLeakScan { matches: Vec<SecretLeakMatch> }` with `is_clean()` (:2029-2035).
- `struct SecretLeakPattern { name: &'static str, severity: SecretLeakSeverity, regex: Regex }` (:2064); `fn secret_leak_patterns() -> &'static [SecretLeakPattern]` = `LazyLock<Vec<..>>` (:2070-2103). Current 5 patterns: `openai_api_key` `\bsk-[A-Za-z0-9_-]{20,}\b`; `github_token` `\bgh[pousr]_[A-Za-z0-9_]{20,}\b`; `aws_access_key` `\bAKIA[0-9A-Z]{16}\b`; `provider_token` `(?i)\b(?:rk|pk|glpat|xox[baprs])[-_a-z0-9]{8,}\b`; `pem_private_key` `-----BEGIN [A-Z ]*PRIVATE KEY-----`.
- `redact_text` (:2225) → `redact_text_with_state(&self, input, state) -> (String, RedactionReport)` (:2230). Order: private-emails (:2236), generic paths (:2237), known-path prefixes (:2238), then `self.leak_detector.scan(&redacted)` (:2240); if clean, early-return (:2241-2243); else per match `report.increment("secret")` + `report.increment("secret:{pattern_name}")` + set `blocked_secret_detected` for High|Critical (:2251-2256), then `apply_redaction_ranges(&redacted, &ranges)` (:2261) replaces the matched byte ranges only.
- **PEM bug:** the pem pattern matches only the `-----BEGIN ... PRIVATE KEY-----` header, so `apply_redaction_ranges` replaces just that header line — the base64 key body survives. Fix belongs in the pattern (match the whole block) or in a dedicated pre-pass.
- `RedactionReport { counts: BTreeMap<String,u32>, pii_labels_present: Vec<String>, warnings: Vec<String>, blocked_secret_detected: bool }` (:1379); `report.increment(key)` (:1389 area).
- `redact_trace` (:2373-2490): event `content` and `outcome.human_correction` go through `redact_text_with_state` + `apply_privacy_filter_to_text`; `structured_payload` goes through `redact_json_value` (:2404), which runs `redact_sensitive_json` (key-name) AND `redact_json_strings` → `redact_text_with_state` on each string leaf (:2278-2303). So string leaves in payloads DO get the pattern scan; non-string values and the whole never see the privacy filter. Improving `redact_text_with_state` therefore improves payload string coverage automatically.
- `apply_redaction_ranges` and `apply_placeholder_regex` are the existing redaction primitives near :2160-2225 (private helpers on the redactor). New passes should reuse `apply_placeholder_regex` where a regex→placeholder fits, or `apply_redaction_ranges` for computed ranges.
- `DETERMINISTIC_REDACTION_PIPELINE_VERSION = "ironclaw-deterministic-secret-path-v1"` (:30). `redaction_pipeline_version(backend)` (~:1614) assembles the stamped version.
- `canary_leaked_tokens(original: &str, redacted: &str) -> Vec<String>` lives in `crates/trace-commons-operator-client/src/privacy_filter.rs:364` (re-exported at that crate's lib.rs:28). Splits `original` on whitespace, skips tokens < 4 chars, returns `canonical_hash(token)` (`"sha256:<hex>"`) for any token appearing verbatim in `redacted`, deduped. Already a dependency of the contributor crate.
- Contributor submit path: `crates/trace-commons-contributor/src/submit.rs` per-session flow — `source.load` → dedupe receipts → `build_redactor_with` → once-per-batch `canary_self_test_async` (:116-121) → `build_raw_contribution` → `redact_to_envelope` → `envelope_size_ok` → dry-run short-circuit → mint claim → `apply_granted_scopes` → `upload_with_retry`. `SubmitOutcome::{Submitted, AlreadySubmitted, SkippedParseFailure, Refused{reason_label}, Failed{reason_label}}`.
- `SessionTranscript` (crates/trace-commons-contributor/src/source/mod.rs): `events: Vec<SessionEvent>`, each `SessionEvent { kind, timestamp, content: Option<String>, structured: serde_json::Value, tool_name, token_counts }`. The original per-session text (for the guard) is reconstructable by concatenating every event's `content` plus `structured.to_string()`.
- Audit harness: `crates/trace-commons-contributor/tests/local_redaction_audit.rs`, `#[ignore]`d test `audit_real_sessions_for_key_leakage`, run with `cargo test -p trace-commons-contributor --test local_redaction_audit -- --ignored --nocapture`. It loads real sessions, redacts, and scans the serialized envelope. Baseline before this plan: leaks present (bearer tokens, 1 github, npm, PEM). Target after: 0.

---

### Task 1: Broaden the secret pattern set and fix PEM whole-block redaction

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs` (`secret_leak_patterns()` :2070-2103; PEM handling in `redact_text_with_state` :2230-2262)
- Test: same file's `#[cfg(test)] mod tests` (:3822) — first inline `redact_text` unit tests.

**Interfaces:**
- Produces: no new public API. `secret_leak_patterns()` gains entries: `jwt` (Critical) `\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b`; `npm_token` (Critical) `\bnpm_[A-Za-z0-9]{36}\b`; `google_api_key` (High) `\bAIza[0-9A-Za-z_-]{35}\b`; `slack_token` is already covered by `provider_token`'s `xox[baprs]` — do NOT duplicate. Widen `github_token` tail from `{20,}` to `{10,}` (real `ghp_` tokens seen at 36 but a fine-grained-PAT segment can be shorter; the audit had a 1-survivor case just under the old floor).
- PEM: replace the header-only pattern with a whole-block redaction. Add a dedicated pre-pass in `redact_text_with_state` BEFORE the leak scan: `apply_placeholder_regex(&redacted, pem_block_regex(), "secret", "<REDACTED_PRIVATE_KEY>")` where `pem_block_regex()` is `(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----`. Remove `pem_private_key` from `secret_leak_patterns()` (the pre-pass supersedes it). A PEM header with no matching END (truncated transcript) still must not leak: add a fallback pattern `pem_header_orphan` (Critical) `-----BEGIN [A-Z ]*PRIVATE KEY-----[A-Za-z0-9+/=\s]*` to the leak set so an unterminated block's body is still caught by range redaction. Increment reporting stays `report.increment("secret")` + `report.increment("secret:{name}")`.

- [ ] **Step 1: Write failing unit tests** in `mod tests` (:3822), needing `use super::*;`:

```rust
#[test]
fn redact_text_strips_broadened_secret_shapes() {
    let r = DeterministicTraceRedactor::new(vec![]).unwrap();
    // JWT (three base64url segments)
    let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N";
    let (out, rep) = r.redact_text(&format!("Authorization: Bearer {jwt}"));
    assert!(!out.contains(jwt), "jwt survived: {out}");
    assert!(rep.blocked_secret_detected);
    // npm + google
    let (o2, _) = r.redact_text("token npm_abcdefghijklmnopqrstuvwxyz0123456789 done");
    assert!(!o2.contains("npm_abcdefghijklmnopqrstuvwxyz0123456789"));
    let (o3, _) = r.redact_text("key AIzaSyA1234567890abcdefghijklmnopqrstuvw end");
    assert!(!o3.contains("AIzaSyA1234567890abcdefghijklmnopqrstuvw"));
}

#[test]
fn redact_text_removes_entire_pem_block_not_just_header() {
    let r = DeterministicTraceRedactor::new(vec![]).unwrap();
    let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA1234secretbody5678\nabcDEFghiJKL==\n-----END RSA PRIVATE KEY-----";
    let (out, rep) = r.redact_text(&format!("here is a key:\n{pem}\ntrailing"));
    assert!(!out.contains("1234secretbody5678"), "pem body survived: {out}");
    assert!(!out.contains("abcDEFghiJKL"), "pem body line 2 survived: {out}");
    assert!(out.contains("trailing"));
    assert!(rep.blocked_secret_detected);
}

#[test]
fn redact_text_catches_orphan_pem_header_without_end() {
    let r = DeterministicTraceRedactor::new(vec![]).unwrap();
    let truncated = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAAsecretbytes";
    let (out, _) = r.redact_text(truncated);
    assert!(!out.contains("secretbytes"), "orphan pem body survived: {out}");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol redact_text_ -- --nocapture`
Expected: FAIL — jwt/npm/google not matched; PEM body survives (header-only redaction).

- [ ] **Step 3: Implement** the new patterns, the PEM whole-block pre-pass + orphan fallback, and the `github_token` tail widening. Add `fn pem_block_regex() -> &'static Regex` as a `LazyLock` beside the other regex helpers. Wire the pre-pass into `redact_text_with_state` immediately before `self.leak_detector.scan`.

- [ ] **Step 4: Run to green**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol redact_text_ && RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol` (full protocol suite — the redaction.rs key-name tests and existing behavior must stay green).
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-protocol/src/trace_contribution.rs
git commit -m "Broaden secret patterns and redact whole PEM blocks"
```

---

### Task 2: Cue-gated contextual-entropy detection for unknown key formats

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs` (`redact_text_with_state` :2230; new helpers beside `SecretLeakDetector`)
- Test: `mod tests` (:3822)

**Interfaces:**
- Produces (protocol-private):
  ```rust
  /// Shannon entropy in bits/char.
  fn token_shannon_entropy(s: &str) -> f64
  /// Byte ranges of high-entropy tokens that sit within CUE_WINDOW chars after a
  /// secret cue (authorization/bearer/api[-_]?key/secret/password/token/key=/: ...),
  /// excluding allowlisted ID/UUID/base64-content shapes. Fail-closed: when unsure, include.
  fn contextual_entropy_secret_ranges(content: &str) -> Vec<std::ops::Range<usize>>
  ```
- Constants: `const CUE_WINDOW: usize = 48;` `const ENTROPY_MIN_LEN: usize = 16;` `const ENTROPY_BITS_MIN: f64 = 3.2;`.
- Cue regex (case-insensitive), matched ending just before the candidate token: `(?i)(authorization|bearer|api[_-]?key|secret|password|passwd|access[_-]?token|client[_-]?secret|private[_-]?key|token|apikey)["'\x60:=\s]{1,6}$` applied to the 48-char window preceding each candidate.
- Candidate token regex: `[A-Za-z0-9+/=_.\-]{16,}`.
- Allowlist (a candidate matching ANY is NOT redacted): UUID `^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`; ID prefixes `msg_ req_ mcp_ toolu_ chatcmpl run_ file_ asst_ resp_ call_`; pure-hex of length 7/8/40/64 (git short/long sha, common hashes); all-lowercase-hex of any length ≥ 32 with NO uppercase and NO non-hex (content hash). Rationale documented inline: these are structural identifiers, verified against real transcripts (105k such tokens vs ~20 real secrets — the prototype that justified this gate).
- Redaction: high-entropy candidates whose entropy ≥ ENTROPY_BITS_MIN, length ≥ ENTROPY_MIN_LEN, that are cue-adjacent and not allowlisted, get their byte ranges redacted via the existing `apply_redaction_ranges`. Reporting: `report.increment("secret")` + `report.increment("secret:contextual_entropy")` and set `blocked_secret_detected = true`.
- Placement: run AFTER the `SecretLeakDetector::scan` block in `redact_text_with_state`, over the already-pattern-redacted text (so known patterns are handled by their named rule and entropy only mops up unknown formats). Merge ranges so a token already redacted by a pattern is not double-counted (dedupe overlapping ranges before applying).

- [ ] **Step 1: Write failing tests** in `mod tests`:

```rust
#[test]
fn contextual_entropy_redacts_unknown_key_after_cue() {
    let r = DeterministicTraceRedactor::new(vec![]).unwrap();
    // opaque high-entropy token, no known prefix, but preceded by a cue
    let secret = "Zx9Qk2Lm7Pv4Rt8Wy1Nb6Hd3Fg5Jc0Ae";
    let (out, rep) = r.redact_text(&format!("api_key: {secret}"));
    assert!(!out.contains(secret), "cue-adjacent secret survived: {out}");
    assert!(rep.blocked_secret_detected);
}

#[test]
fn contextual_entropy_spares_ids_and_hashes_and_uncued_tokens() {
    let r = DeterministicTraceRedactor::new(vec![]).unwrap();
    // message id after a cue-shaped word must NOT be redacted (allowlisted prefix)
    let (o1, _) = r.redact_text("token: msg_01ABCDEFghijklmnopqrstuvwx");
    assert!(o1.contains("msg_01ABCDEFghijklmnopqrstuvwx"), "allowlisted id got redacted: {o1}");
    // git sha after cue must survive (hex len 40)
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let (o2, _) = r.redact_text(&format!("key {sha}"));
    assert!(o2.contains(sha), "git sha got redacted: {o2}");
    // high-entropy token with NO cue nearby must survive (avoids shredding base64 content)
    let blob = "CAESabcdef0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let (o3, _) = r.redact_text(&format!("the encoded value {blob} appears here"));
    assert!(o3.contains(blob), "uncued blob got redacted: {o3}");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol contextual_entropy_ -- --nocapture`
Expected: FAIL — helper does not exist; cue-adjacent secret survives.

- [ ] **Step 3: Implement** `token_shannon_entropy`, `contextual_entropy_secret_ranges`, the constants, and the wiring into `redact_text_with_state` (after the pattern scan, dedupe ranges, apply). Keep it pure/synchronous.

- [ ] **Step 4: Run to green**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol && RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --no-run`
Expected: protocol suite PASS (existing tests unaffected — the uncued-blob test pins that non-secret content is spared); contributor still compiles.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-protocol/src/trace_contribution.rs
git commit -m "Add cue-gated high-entropy secret detection"
```

---

### Task 3: Per-session fail-closed leaked-token guard in submit

> CORRECTION (2026-07-09, during execution): the original `canary_leaked_tokens` token-diff mechanism was a plan defect — it flags every surviving prose token, over-refusing real sessions. The implemented guard instead RE-SCANS the finished envelope with the secret DETECTOR: `envelope_has_residual_secret(redactor, envelope)` serializes the envelope and calls `redactor.redact_text(json)`, returning `report.blocked_secret_detected`. Only secret shapes trip it; ordinary prose does not. Same fail-closed placement and `Refused{"secret-leak-detected"}` outcome. The subsections below describing `session_original_text`/`envelope_leaked_tokens`/`canary_leaked_tokens` are SUPERSEDED by this note.


**Files:**
- Modify: `crates/trace-commons-contributor/src/envelope.rs` (add the guard helper)
- Modify: `crates/trace-commons-contributor/src/submit.rs` (call it per session, before upload)

**Interfaces:**
- Consumes: `trace_commons_operator_client::canary_leaked_tokens` (re-exported at that crate's root); `SessionTranscript`, `TraceContributionEnvelope`.
- Produces in `envelope.rs`:
  ```rust
  /// Reconstruct the pre-redaction text of a session (every event's content plus
  /// its structured payload rendered as JSON), for leaked-token comparison.
  pub fn session_original_text(t: &crate::source::SessionTranscript) -> String
  /// Returns the sha256 hashes of any >=4-char token from `original` that still
  /// appears verbatim in the serialized `envelope`. Empty = clean.
  pub fn envelope_leaked_tokens(original: &str, envelope: &TraceContributionEnvelope) -> anyhow::Result<Vec<String>>
  ```
  `envelope_leaked_tokens` serializes the envelope with `serde_json::to_string` and calls `trace_commons_operator_client::canary_leaked_tokens(original, &json)`.
- `submit.rs` flow change: after `redact_to_envelope` and `apply_granted_scopes` (for the real, non-dry-run path) AND on the dry-run path after redaction, compute `let leaked = envelope_leaked_tokens(&session_original_text(&transcript), &envelope)?;` and if `!leaked.is_empty()`, push `SubmitOutcome::Refused { reason_label: "secret-leak-detected" }`, log `tracing::warn!(leaked_token_count = leaked.len(), "refusing session: secret survived redaction")` (count only — the hashes are non-identifying but we keep the surface minimal), and `continue` — never upload. Placement: immediately after the envelope is finalized (post-stamp) and before `envelope_size_ok`/upload; on the dry-run branch, before printing the dry-run line (a leaking session must report `Refused`, not a clean dry-run).
- This guard is defense-in-depth: even if Tasks 1-2 miss a format, the session fails closed instead of uploading. Over-refusal (a false positive where a non-secret 4+ char token coincidentally survives) is acceptable per the fail-closed constraint; the guard hashes whole whitespace tokens, and the reconstructed original is the same text the redactor saw, so a clean redaction yields an empty result. NOTE the known-benign case: very short common words are already excluded (`< 4` chars); tokens like `true`/`null` that appear in both original and envelope are a possible false positive — accept it (a refused session is safe; the contributor can inspect). If false-positive refusals prove common in the audit, Task 5 documents tightening as follow-up rather than weakening the guard now.

- [ ] **Step 1: Write failing tests** in `submit.rs` tests (stub issuer/ingest already exist in that module):

```rust
#[tokio::test]
async fn session_with_surviving_secret_is_refused_not_uploaded() {
    // A source whose transcript embeds a token that (for this test) we force to
    // survive by making the envelope carry it verbatim. Use a fixture transcript
    // whose content contains a distinctive token, and a redactor that leaves it
    // (no cue, unknown format) — then assert Refused + zero ingest deliveries.
    // Build the selection from a temp session file containing:
    //   {"type":"user","message":{"role":"user","content":"plain token ZZmarker4242 stays"},...}
    // where ZZmarker4242 is >=4 chars, not secret-shaped, not cue-adjacent, so it
    // survives redaction and MUST trip the guard because it appears verbatim in
    // both the original text and the envelope.
    // (Full harness mirrors the existing submits_fixture_session test; assert
    //  outcomes[0] is SubmitOutcome::Refused { reason_label } with
    //  reason_label == "secret-leak-detected" and the ingest stub received 0 bodies.)
}
```

Implementer note: write the test concretely using the existing `submit.rs` test scaffolding (temp `ConfigStore`, stub issuer, counting stub ingest, a `ClaudeCodeSource` over a temp fixture dir). The assertion is `matches!(outcomes[0], SubmitOutcome::Refused { .. })`, the reason label equals `"secret-leak-detected"`, and the ingest counter is 0. The guard trips because `session_original_text` and the envelope both contain `ZZmarker4242` verbatim — that is exactly the residual-token condition the guard detects. (This doubles as proof the guard catches ANY survivor, not just known secret shapes.)

- [ ] **Step 2: Run to verify failure**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor session_with_surviving_secret -- --nocapture`
Expected: FAIL — helper missing / session currently uploads instead of refusing.

- [ ] **Step 3: Implement** `session_original_text`, `envelope_leaked_tokens`, and the submit-path guard on both the real and dry-run branches.

- [ ] **Step 4: Run to green**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`
Expected: full crate PASS. The pre-existing `submits_fixture_session_and_is_idempotent_on_rerun` test must still pass — its fixture has no surviving secret, so the guard is a no-op there. If that test now refuses, the fixture contains a coincidental surviving token; fix by making the guard's expectation explicit in that test (assert Submitted) and, if it genuinely trips, adjust the fixture text, not the guard.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/envelope.rs crates/trace-commons-contributor/src/submit.rs
git commit -m "Refuse any session whose secret survives redaction"
```

---

### Task 4: Bump pipeline version and stamp the hardened redactor

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs` (`DETERMINISTIC_REDACTION_PIPELINE_VERSION` :30)
- Test: existing `redaction_pipeline_version_emits_per_backend_suffix` (:3893) — update expected string.

**Interfaces:**
- Produces: `DETERMINISTIC_REDACTION_PIPELINE_VERSION = "ironclaw-deterministic-secret-path-v2"` (v1 → v2). Any test or fixture asserting the v1 string is updated to v2. This is the signal that a materially stronger scrub ran, so downstream consumers can tell v1-scrubbed envelopes from v2.

- [ ] **Step 1: Update the constant and find all references**

Run: `grep -rn "ironclaw-deterministic-secret-path-v1\|deterministic-secret-path-v1" crates/ docs/`
Change the constant to `...-v2`. Update every test/fixture asserting the literal v1 string (the version test at :3893, and any e2e/contributor assertion). Do NOT change the suffix consts (sidecar/near-ai/server-rescrub) — only the deterministic base.

- [ ] **Step 2: Run the version tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol redaction_pipeline_version && RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --no-run`
Expected: PASS; contributor compiles (it does not hardcode the version string — verify via the grep that no contributor assertion breaks; if one does, update it to v2).

- [ ] **Step 3: Commit**

```bash
git add crates/trace-commons-protocol/src/trace_contribution.rs
git commit -m "Bump deterministic redaction pipeline to v2"
```

---

### Task 5: Prove zero leaks against real transcripts and document

**Files:**
- Modify: `crates/trace-commons-contributor/tests/local_redaction_audit.rs` (extend pattern set to match the broadened detector; keep `#[ignore]`)
- Modify: `crates/trace-commons-contributor/README.md` (redaction section: document the three layers + the fail-closed guard)

**Interfaces:** none (acceptance gate + docs).

- [ ] **Step 1: Extend the audit scanner** so its `scan` covers every shape the detector now targets — add JWT (`eyJ...` three-segment), npm_, AIza, and a cue-gated-entropy check mirroring the detector's logic — so the audit is a faithful oracle, not a weaker one. Keep the test `#[ignore]`d and reading `~/.claude/projects`.

- [ ] **Step 2: Run the audit against real local sessions**

Run: `cargo test -p trace-commons-contributor --test local_redaction_audit -- --ignored --nocapture`
Expected: the printed pre/post table shows post-redaction counts of 0 for every pattern, and the test PASSES (no `LEAKS` block, no panic). If any survivor remains, that is a real gap — return to Task 1 or 2 with the surviving shape (the audit prints prefix/length/entropy only, never the secret) rather than weakening the audit.

- [ ] **Step 3: Update the README redaction section** to describe: (1) known-pattern scrubbing (list the families), (2) whole-PEM-block redaction, (3) cue-gated high-entropy catch-all for unknown formats, (4) the per-session fail-closed leaked-token guard that refuses any session where a token survives, and (5) that the NEAR AI pass remains an optional add-on for prose PII. State plainly that redaction is fail-closed: sessions with a surviving secret are refused, not uploaded.

- [ ] **Step 4: Full verification sweep** (all must pass; paste outputs in the report):

```bash
cargo fmt -p trace-commons-protocol -p trace-commons-contributor -- --check
RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy --workspace --all-targets -- -D warnings -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
cargo test -p trace-commons-contributor --test local_redaction_audit -- --ignored --nocapture
```

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/tests/local_redaction_audit.rs crates/trace-commons-contributor/README.md
git commit -m "Prove zero secret leaks against real transcripts and document redaction layers"
```
