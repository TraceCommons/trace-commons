//! The dependency invariant that keeps IronWire's recursive walkers safe.
//!
//! `peek::json_contains_key`, `peek::json_contains_type`,
//! `peek::serialized_len_hint` and the privacy filter's substitution walk all
//! recurse over a parsed body. Recursion over input the process did not choose
//! is how a daemon dies — and a stack overflow is not a caught panic, so it
//! takes every other conversation on the machine with it.
//!
//! Nothing in IronWire bounds that depth. What bounds it is `serde_json`'s
//! parser, which refuses documents nested deeper than 128. That protection is
//! real and it is also **incidental**: `serde_json` ships an `unbounded_depth`
//! feature, and any dependency in the tree enabling it would silently turn
//! every walker above into a crash vector with no code change here at all.
//!
//! So the invariant is asserted rather than assumed. If this test fails,
//! IronWire needs its own depth bound before the parser stops providing one.

/// The depth `serde_json` allows. Not ours to choose, but ours to depend on.
const EXPECTED_LIMIT: usize = 128;

fn nested(depth: usize) -> String {
    "[".repeat(depth) + &"]".repeat(depth)
}

#[test]
fn serde_json_still_refuses_deeply_nested_documents() {
    assert!(
        serde_json::from_str::<serde_json::Value>(&nested(EXPECTED_LIMIT - 1)).is_ok(),
        "a document just under the limit should still parse; if this fails the \
         limit moved down and legitimate requests are being refused"
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(&nested(EXPECTED_LIMIT)).is_err(),
        "serde_json no longer bounds nesting depth. IronWire's recursive walkers \
         (peek::json_contains_key, the privacy substitution walk) are now \
         reachable with arbitrary depth, and a stack overflow there kills the \
         daemon and every conversation on the machine. Add an explicit depth \
         check before shipping this."
    );
}

#[test]
fn a_pathologically_deep_document_is_refused_rather_than_walked() {
    // The shape an agent having a bad day would actually produce.
    assert!(serde_json::from_str::<serde_json::Value>(&nested(50_000)).is_err());
}

#[test]
fn the_depth_that_matters_for_real_bodies_is_far_below_the_limit() {
    // A sanity check on the trade: if real requests were anywhere near 128 deep
    // the bound would be refusing legitimate traffic, and we would need our own
    // limit for a different reason.
    let realistic = serde_json::json!({
        "model": "claude-opus-4-6",
        "system": [{"type": "text", "text": "You are Claude Code",
                    "cache_control": {"type": "ephemeral"}}],
        "tools": [{"name": "Read", "input_schema": {"type": "object",
                   "properties": {"path": {"type": "string"}}}}],
        "messages": [{"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_1",
             "content": [{"type": "text", "text": "ok"}]}
        ]}],
    });

    fn depth(value: &serde_json::Value) -> usize {
        match value {
            serde_json::Value::Array(items) => 1 + items.iter().map(depth).max().unwrap_or(0),
            serde_json::Value::Object(fields) => 1 + fields.values().map(depth).max().unwrap_or(0),
            _ => 0,
        }
    }

    let observed = depth(&realistic);
    assert!(
        observed < 20,
        "a realistic Claude Code body is {observed} deep, which is close enough \
         to the 128 limit to be worth reconsidering"
    );
}
