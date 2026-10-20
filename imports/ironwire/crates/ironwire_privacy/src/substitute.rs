//! Substituting over a request body.
//!
//! Walks the JSON a client sent, replaces what the detector found, and returns
//! the map that reverses it. The map is dropped with the request
//! (`docs/PRIVACY.md` §4).
//!
//! Two things this deliberately does *not* touch, both because substituting
//! them breaks the user's actual work rather than protecting it:
//!
//! - **Fenced code blocks.** A key or an address inside one is nearly always
//!   the code being edited.
//! - **Tool results.** They are output from the user's own machine that the
//!   model needs verbatim to reason about — a test failure with a substituted
//!   hostname is a test failure the model cannot diagnose.
//!
//! Both are defaults, not laws: a user who wants them scanned can say so.

use serde_json::Value;

use crate::detect::Detector;
use crate::mint::{Map, Salt};

/// What to leave alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exemptions {
    /// Skip text inside ``` fences.
    pub code_blocks: bool,
    /// Skip `tool_result` / `function_call_output` content.
    pub tool_results: bool,
}

impl Default for Exemptions {
    fn default() -> Self {
        Self {
            code_blocks: true,
            tool_results: true,
        }
    }
}

/// The upshot of substituting one request.
#[derive(Debug)]
pub struct Substituted {
    /// The body to send upstream.
    pub body: Value,
    /// The map that reverses it. Never persisted.
    pub map: Map,
    /// How many distinct values were replaced, for the ledger and for
    /// `ironwire log` — a turn full of customer data that produced zero
    /// substitutions is the signal a user needs to see (`docs/PRIVACY.md` §7).
    pub substitutions: usize,
}

/// Replace everything the detector finds in a request body.
#[must_use]
pub fn substitute(
    detector: &Detector,
    salt: &Salt,
    exemptions: &Exemptions,
    body: &Value,
) -> Substituted {
    let mut map = Map::new();
    let mut out = body.clone();
    walk(detector, salt, exemptions, &mut out, &mut map, false);
    let substitutions = map.len();
    Substituted {
        body: out,
        map,
        substitutions,
    }
}

/// Recursively rewrite string leaves.
fn walk(
    detector: &Detector,
    salt: &Salt,
    exemptions: &Exemptions,
    value: &mut Value,
    map: &mut Map,
    inside_exempt: bool,
) {
    match value {
        Value::String(text) => {
            if inside_exempt {
                return;
            }
            if let Some(rewritten) = rewrite(detector, salt, exemptions, text, map) {
                *text = rewritten;
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(detector, salt, exemptions, item, map, inside_exempt);
            }
        }
        Value::Object(fields) => {
            let exempt = inside_exempt || (exemptions.tool_results && is_tool_output(fields));
            for (_, field) in fields.iter_mut() {
                walk(detector, salt, exemptions, field, map, exempt);
            }
        }
        _ => {}
    }
}

/// Whether this object is a replayed tool result.
fn is_tool_output(fields: &serde_json::Map<String, Value>) -> bool {
    matches!(
        fields.get("type").and_then(Value::as_str),
        Some("tool_result" | "function_call_output")
    ) || fields.get("role").and_then(Value::as_str) == Some("tool")
}

/// Rewrite one string, or `None` if nothing matched.
fn rewrite(
    detector: &Detector,
    salt: &Salt,
    exemptions: &Exemptions,
    text: &str,
    map: &mut Map,
) -> Option<String> {
    let findings = detector.find(text);
    if findings.is_empty() {
        return None;
    }

    let fences = if exemptions.code_blocks {
        fenced_ranges(text)
    } else {
        Vec::new()
    };

    // Back to front, so earlier ranges stay valid as we splice.
    let mut out = text.to_string();
    let mut any = false;
    for finding in findings.iter().rev() {
        if fences
            .iter()
            .any(|fence| fence.0 <= finding.range.start && finding.range.end <= fence.1)
        {
            continue;
        }
        let plaintext = &text[finding.range.clone()];
        let token = map.insert(salt, finding.class, plaintext);
        out.replace_range(finding.range.clone(), &token);
        any = true;
    }

    any.then_some(out)
}

/// Byte ranges covered by ``` fences.
///
/// Deliberately crude: an unclosed fence covers the rest of the string. Over-
/// exempting costs privacy for text the user was probably editing anyway;
/// under-exempting costs them a broken file. The asymmetry favours this.
fn fenced_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut open: Option<usize> = None;
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find("```") {
        let at = cursor + offset;
        match open {
            Some(start) => {
                ranges.push((start, at + 3));
                open = None;
            }
            None => open = Some(at),
        }
        cursor = at + 3;
    }
    if let Some(start) = open {
        ranges.push((start, text.len()));
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::Tiers;
    use serde_json::json;

    fn detector(named: &[&str]) -> Detector {
        Detector::new(&Tiers {
            secrets: true,
            named_values: named.iter().map(|s| (*s).to_string()).collect(),
            pii: false,
        })
    }

    fn round_trip(result: &Substituted, text: &str) -> String {
        // Reverse by hand, to assert the map alone is sufficient.
        let mut out = text.to_string();
        for token in result.map.placeholders() {
            if let Some(plain) = result.map.plaintext(token) {
                out = out.replace(token, plain);
            }
        }
        out
    }

    #[test]
    fn a_nominated_value_is_replaced_and_reverses_exactly() {
        let body = json!({"messages": [{"role": "user", "content": "email alice@corp.com"}]});
        let result = substitute(
            &detector(&["alice@corp.com"]),
            &Salt::fixed(1),
            &Exemptions::default(),
            &body,
        );
        let sent = result.body["messages"][0]["content"]
            .as_str()
            .expect("string");
        assert!(!sent.contains("alice@corp.com"), "value survived: {sent}");
        assert_eq!(round_trip(&result, sent), "email alice@corp.com");
        assert_eq!(result.substitutions, 1);
    }

    #[test]
    fn a_body_with_nothing_to_substitute_is_returned_unchanged() {
        // The overwhelmingly common case. It must be byte-for-byte the same
        // structure, so nothing downstream sees a difference.
        let body = json!({"model": "claude-opus-4-6", "messages": [{"role": "user", "content": "fix the test"}]});
        let result = substitute(
            &detector(&["alice@corp.com"]),
            &Salt::fixed(2),
            &Exemptions::default(),
            &body,
        );
        assert_eq!(result.body, body);
        assert_eq!(result.substitutions, 0);
        assert!(result.map.is_empty());
    }

    #[test]
    fn field_order_survives() {
        // `serde_json` is built with `preserve_order`, and the native lane
        // depends on it. A substitution must not reshuffle a body.
        let body: Value = serde_json::from_str(
            r#"{"model":"m","zzz":1,"messages":[{"role":"user","content":"alice@corp.com"}],"aaa":2}"#,
        )
        .expect("parses");
        let result = substitute(
            &detector(&["alice@corp.com"]),
            &Salt::fixed(3),
            &Exemptions::default(),
            &body,
        );
        let keys: Vec<&str> = result
            .body
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["model", "zzz", "messages", "aaa"]);
    }

    #[test]
    fn a_value_inside_a_code_block_is_left_alone() {
        // It is nearly always the code being edited, and substituting it makes
        // the model rewrite a file into something that does not work.
        let content = "fix this:\n```\nconst OWNER = \"alice@corp.com\";\n```\nthanks";
        let body = json!({"messages": [{"role": "user", "content": content}]});
        let result = substitute(
            &detector(&["alice@corp.com"]),
            &Salt::fixed(4),
            &Exemptions::default(),
            &body,
        );
        assert_eq!(result.substitutions, 0);
        assert_eq!(result.body, body);
    }

    #[test]
    fn the_code_block_exemption_can_be_turned_off() {
        let content = "```\nalice@corp.com\n```";
        let body = json!({"messages": [{"role": "user", "content": content}]});
        let result = substitute(
            &detector(&["alice@corp.com"]),
            &Salt::fixed(5),
            &Exemptions {
                code_blocks: false,
                tool_results: true,
            },
            &body,
        );
        assert_eq!(result.substitutions, 1);
    }

    #[test]
    fn a_tool_result_is_left_alone() {
        // Output from the user's own machine that the model needs verbatim: a
        // test failure with a substituted hostname cannot be diagnosed.
        let body = json!({
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu_1",
                    "content": "connect failed: alice@corp.com"
                }]
            }]
        });
        let result = substitute(
            &detector(&["alice@corp.com"]),
            &Salt::fixed(6),
            &Exemptions::default(),
            &body,
        );
        assert_eq!(result.substitutions, 0);
    }

    #[test]
    fn one_value_appearing_many_times_costs_one_placeholder() {
        let body = json!({
            "messages": (0..20)
                .map(|i| json!({"role": "user", "content": format!("turn {i}: alice@corp.com")}))
                .collect::<Vec<_>>()
        });
        let result = substitute(
            &detector(&["alice@corp.com"]),
            &Salt::fixed(7),
            &Exemptions::default(),
            &body,
        );
        assert_eq!(
            result.substitutions, 1,
            "a replayed history is not 20 entries"
        );
        // ...but every occurrence is replaced.
        assert!(!result.body.to_string().contains("alice@corp.com"));
    }

    #[test]
    fn substitution_is_stable_across_turns() {
        // The property that makes the per-request map work: next turn sees the
        // same plaintext and must mint the same token.
        let salt = Salt::fixed(8);
        let detector = detector(&["alice@corp.com"]);
        let turn_1 = json!({"messages": [{"role": "user", "content": "alice@corp.com"}]});
        let turn_2 = json!({"messages": [
            {"role": "user", "content": "alice@corp.com"},
            {"role": "assistant", "content": "ok"},
            {"role": "user", "content": "again: alice@corp.com"},
        ]});

        let a = substitute(&detector, &salt, &Exemptions::default(), &turn_1);
        let b = substitute(&detector, &salt, &Exemptions::default(), &turn_2);
        let token_a = a.map.placeholders().next().expect("minted").to_string();
        let token_b = b.map.placeholders().next().expect("minted").to_string();
        assert_eq!(token_a, token_b);
    }

    #[test]
    fn a_value_containing_json_metacharacters_survives_encoding() {
        let awkward = r#"quote " backslash \ newline"#;
        let body = json!({"messages": [{"role": "user", "content": format!("x {awkward} y")}]});
        let result = substitute(
            &detector(&[awkward]),
            &Salt::fixed(9),
            &Exemptions::default(),
            &body,
        );
        assert_eq!(result.substitutions, 1);
        // It must still be valid JSON after the splice.
        let encoded = serde_json::to_string(&result.body).expect("encodes");
        let reparsed: Value = serde_json::from_str(&encoded).expect("reparses");
        assert_eq!(reparsed, result.body);
    }

    #[test]
    fn non_string_leaves_are_untouched() {
        let body = json!({"stream": true, "max_tokens": 4096, "temperature": 0.7, "nothing": null});
        let result = substitute(
            &detector(&["4096"]),
            &Salt::fixed(10),
            &Exemptions::default(),
            &body,
        );
        assert_eq!(result.body, body, "a number must not become a placeholder");
    }
}
