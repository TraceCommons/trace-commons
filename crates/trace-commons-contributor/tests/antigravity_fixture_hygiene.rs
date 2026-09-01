//! Permanent identity-leak gate on the committed Antigravity API fixtures.
//!
//! Successive redaction passes on these fixtures each fixed exactly what
//! the previous review named and missed a category nobody had thought to
//! name yet: the operator's local username, `thinkingSignature` blobs,
//! vendor session/response correlation ids, vendor execution/message
//! correlation ids, and then two more `id` values that an over-broad
//! allowlist let through, plus a hiding place inside embedded JSON strings
//! (`argumentsJson`) that a one-level walk never reached. A human sweep
//! does not scale to "whatever the next capture contains" — this test
//! does, by working from a rule instead of an enumeration: any key that
//! looks like an identifier and is not on the small, deliberate, *scoped*
//! allowlist below must hold a `REDACTED`-prefixed value, full stop, and
//! that rule is applied recursively into any string value that itself
//! parses as JSON.
//!
//! Run: `cargo test -p trace-commons-contributor --test antigravity_fixture_hygiene`

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Keys that look like identifiers but must stay real, because later tests
/// depend on their structure or on the relationships between them:
///
/// - `trajectoryId`, `cascadeId`, `conversationId` — conversation identity.
///   `listing.json`'s entry and the step fixtures describe the same
///   conversation (cascade `39f32a85-...`), and a test resolves one
///   through the other (see the fixture README).
/// - `TaskId` / `task_id` — shaped `<cascadeId>/task-N`, derived from the
///   above.
///
/// `id` is deliberately NOT a blanket allowlist entry: a bare `id` shows up
/// on vendor message objects (e.g. `agentMessage.id`) that carry no
/// structural meaning here and must be redacted like any other vendor
/// correlation id. Two shapes of bare `id` are real — recognized by
/// `is_scoped_real_id` below, not by the key name alone:
///
/// - a tool call's `id` (object also carries `name` and `argumentsJson`,
///   or the value itself is `call_<digits>`) — the call/result linkage
///   the Trajectory-v1 reader needs, and which the multi-turn/single-turn
///   pair is used to test.
/// - a `taskDetails.id` (object also carries `logUri` and `description`)
///   — always byte-identical to the same task's `TaskId`/`task_id` value
///   elsewhere in the same file. Redacting only this occurrence would
///   create a mismatch the real capture never has: `taskDetails.id` is
///   the schema's obvious join key to `TaskId`/`task_id`, so a future
///   reader correlating task-status events would join on it, get
///   nothing, and hunt a bug that exists only in this fixture. Leaving it
///   real costs nothing — the value is already public in the same file
///   under an allowlisted key.
const ALLOWED_ID_KEYS: &[&str] = &[
    "trajectoryId",
    "cascadeId",
    "conversationId",
    "TaskId",
    "task_id",
];

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/antigravity")
}

fn is_id_shaped_key(key: &str) -> bool {
    key.ends_with("Id") || key.ends_with("ID") || key.ends_with("id")
}

/// `^call_\d+$`, checked by hand rather than pulling in a regex crate for
/// one shape.
fn looks_like_call_id(value: &str) -> bool {
    match value.strip_prefix("call_") {
        Some(rest) => !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

/// `^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`.
fn looks_like_uuid(value: &str) -> bool {
    let expected_lens = [8, 4, 4, 4, 12];
    let parts: Vec<&str> = value.split('-').collect();
    parts.len() == expected_lens.len()
        && parts
            .iter()
            .zip(expected_lens)
            .all(|(part, len)| part.len() == len && part.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// `^<uuid>/task-\d+$` — the `taskDetails.id` shape, matching the sibling
/// `TaskId`/`task_id` values it must stay byte-identical to.
fn looks_like_task_id(value: &str) -> bool {
    match value.rsplit_once("/task-") {
        Some((cascade_id, task_num)) => {
            looks_like_uuid(cascade_id)
                && !task_num.is_empty()
                && task_num.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    }
}

/// A bare `id` is only real when the object it lives on has one of two
/// recognized shapes AND the value itself has the shape that object type
/// actually produces — the allowlist is scoped by shape, not a blanket
/// pass for any value sitting on an object of that type, matching the
/// team's scoping rule exactly rather than trusting the key name in
/// isolation.
fn is_scoped_real_id(
    key: &str,
    value: &str,
    containing_object: &serde_json::Map<String, Value>,
) -> bool {
    if key != "id" {
        return false;
    }
    let is_tool_call = containing_object.contains_key("name")
        && containing_object.contains_key("argumentsJson")
        && looks_like_call_id(value);
    let is_task_details = containing_object.contains_key("logUri")
        && containing_object.contains_key("description")
        && looks_like_task_id(value);
    // No trailing `|| looks_like_call_id(value)`: that is an unconditional
    // pass for any call-shaped value anywhere, which makes the object-shape
    // half of the rule decorative. The gate is object shape AND value shape.
    is_tool_call || is_task_details
}

/// Walks every object in `value` (including recursing into any string
/// value that itself successfully parses as JSON — the `argumentsJson`
/// hiding place), collecting `(key, value)` for every string-valued,
/// id-shaped key that is not covered by the allowlist above.
fn find_non_allowlisted_id_values(value: &Value, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if let Value::String(s) = v {
                    if is_id_shaped_key(k) {
                        let allowed =
                            ALLOWED_ID_KEYS.contains(&k.as_str()) || is_scoped_real_id(k, s, map);
                        if !allowed && !s.starts_with("REDACTED") {
                            out.push((k.clone(), s.clone()));
                        }
                    } else if let Ok(embedded) = serde_json::from_str::<Value>(s) {
                        // Not itself an id-shaped key, but the string it
                        // holds parses as JSON (e.g. `argumentsJson`) —
                        // apply the same rule inside it.
                        find_non_allowlisted_id_values(&embedded, out);
                    }
                }
                find_non_allowlisted_id_values(v, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                find_non_allowlisted_id_values(item, out);
            }
        }
        _ => {}
    }
}

fn find_thinking_signatures<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if k == "thinkingSignature" {
                    if let Value::String(s) = v {
                        out.push(s.as_str());
                    }
                }
                find_thinking_signatures(v, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                find_thinking_signatures(item, out);
            }
        }
        _ => {}
    }
}

fn find_user_paths(text: &str) -> BTreeSet<&str> {
    let mut found = BTreeSet::new();
    let mut rest = text;
    while let Some(pos) = rest.find("/Users/") {
        let start = pos + "/Users/".len();
        let tail = &rest[start..];
        let end = tail
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.'))
            .unwrap_or(tail.len());
        found.insert(&tail[..end]);
        rest = &rest[start + end..];
    }
    found
}

fn assert_fixture_is_clean(name: &str) {
    let path = fixtures_dir().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()));
    let value: Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("fixture {} is not valid JSON: {e}", path.display()));

    let mut leaked_ids = Vec::new();
    find_non_allowlisted_id_values(&value, &mut leaked_ids);
    assert!(
        leaked_ids.is_empty(),
        "{name} carries a vendor identifier in plaintext (possibly nested inside \
         an embedded-JSON string field such as argumentsJson): {leaked_ids:?}.\n\
         This fixture is committed to a PUBLIC repository. If this key is a \
         vendor correlation id with no structural role in the tests, redact \
         it (replace the value with \"REDACTED-<UPPERCASE-KEY>\"). If a test \
         genuinely needs this key's real value, add it to ALLOWED_ID_KEYS (or \
         extend is_scoped_real_id) in antigravity_fixture_hygiene.rs \
         deliberately, with a comment saying why."
    );

    let mut signatures = Vec::new();
    find_thinking_signatures(&value, &mut signatures);
    for sig in signatures {
        assert_eq!(
            sig, "REDACTED-THINKING-SIGNATURE",
            "{name} carries an unredacted toolCalls[].thinkingSignature value. \
             This is an opaque encrypted model-internals blob and must never be \
             committed; replace it with the literal string \
             \"REDACTED-THINKING-SIGNATURE\"."
        );
    }

    let user_paths = find_user_paths(&text);
    let stray: Vec<&&str> = user_paths.iter().filter(|p| **p != "anonymized").collect();
    assert!(
        stray.is_empty(),
        "{name} carries a /Users/<name> path other than /Users/anonymized: {stray:?}.\n\
         This fixture is committed to a PUBLIC repository and must not carry \
         the operator's real local username. Replace it with \"anonymized\" \
         (see the fixture README for the redaction method)."
    );
}

#[test]
fn steps_single_turn_fixture_is_clean() {
    assert_fixture_is_clean("steps-single-turn.json");
}

#[test]
fn steps_multi_turn_fixture_is_clean() {
    assert_fixture_is_clean("steps-multi-turn.json");
}

#[test]
fn listing_fixture_is_clean() {
    assert_fixture_is_clean("listing.json");
}
