use serde_json::{Map, Value};

const REDACTED: &str = "[REDACTED]";
const SENSITIVE_EXACT: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "api-key",
    "api_key",
    "access_token",
    "refresh_token",
    "session_token",
    "id_token",
    "token",
    "password",
    "passwd",
    "secret",
    "client_secret",
    "private_key",
    "apikey",
    "apisecret",
];

const SENSITIVE_PARTS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "credential",
    "authorization",
    "cookie",
    "apikey",
    "apisecret",
];
const TOKEN_PARTS: &[&str] = &["token", "jwt"];
const KEY_PARTS: &[&str] = &["key"];
const CONTEXT_PARTS: &[&str] = &[
    "auth",
    "oauth",
    "authorization",
    "api",
    "access",
    "refresh",
    "session",
    "bearer",
    "private",
    "client",
    "id",
    "app",
    "user",
    "application",
    "account",
];

fn split_camel_case_key_parts(key: &str) -> Vec<String> {
    if key.is_empty() {
        return Vec::new();
    }

    let chars: Vec<char> = key.chars().collect();
    let mut parts = Vec::new();
    let mut start = 0;

    for i in 1..chars.len() {
        let prev = chars[i - 1];
        let cur = chars[i];
        let next = chars.get(i + 1).copied();

        let boundary = (prev.is_ascii_lowercase() && cur.is_ascii_uppercase())
            || (prev.is_ascii_alphabetic() && cur.is_ascii_digit())
            || (prev.is_ascii_digit() && cur.is_ascii_alphabetic())
            || (prev.is_ascii_uppercase()
                && cur.is_ascii_uppercase()
                && next.map(|n| n.is_ascii_lowercase()).unwrap_or(false));

        if boundary {
            parts.push(chars[start..i].iter().collect::<String>());
            start = i;
        }
    }

    parts.push(chars[start..].iter().collect::<String>());
    parts
}

fn tokenize_key_parts(key: &str) -> Vec<String> {
    let mut parts = Vec::new();

    for segment in key.split(|c: char| !c.is_ascii_alphanumeric()) {
        if segment.is_empty() {
            continue;
        }

        parts.extend(split_camel_case_key_parts(segment));
    }

    parts.into_iter().map(|p| p.to_ascii_lowercase()).collect()
}

fn has_exact(parts: &[String], candidates: &[&str]) -> bool {
    parts
        .iter()
        .any(|part| candidates.iter().any(|candidate| part == candidate))
}

fn has_candidate_or_numbered_variant(parts: &[String], candidates: &[&str]) -> bool {
    parts.iter().any(|part| {
        candidates.iter().any(|candidate| {
            if part == candidate {
                return true;
            }
            let Some(suffix) = part.strip_prefix(candidate) else {
                return false;
            };
            !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
        })
    })
}

fn has_contextual_suffix(parts: &[String], candidates: &[&str]) -> bool {
    parts.iter().any(|part| {
        candidates.iter().any(|candidate| {
            let Some(prefix) = part.strip_suffix(candidate) else {
                return false;
            };
            !prefix.is_empty() && CONTEXT_PARTS.contains(&prefix)
        })
    })
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    if SENSITIVE_EXACT.contains(&lower.as_str()) {
        return true;
    }

    let parts = tokenize_key_parts(key);
    if parts.is_empty() {
        return false;
    }

    if has_candidate_or_numbered_variant(&parts, SENSITIVE_PARTS) {
        return true;
    }

    let has_token = has_candidate_or_numbered_variant(&parts, TOKEN_PARTS);
    let has_key = has_candidate_or_numbered_variant(&parts, KEY_PARTS);

    if has_token && has_key {
        return true;
    }

    if has_contextual_suffix(&parts, TOKEN_PARTS) || has_contextual_suffix(&parts, KEY_PARTS) {
        return true;
    }

    let has_context = has_exact(&parts, CONTEXT_PARTS);
    has_context && (has_token || has_key)
}

fn redact_in_place(value: &mut Value) {
    match value {
        Value::Object(map) => redact_object(map),
        Value::Array(items) => {
            for item in items {
                redact_in_place(item);
            }
        }
        _ => {}
    }
}

/// True when `map` is a k8s/env-style `{name, value}` object whose `name`
/// string is itself a sensitive key cue (e.g. `API_KEY`, `DATABASE_PASSWORD`).
///
/// The cue sits in a sibling value rather than in the JSON key, so
/// key-name redaction and the text cue-window never see it (#193 row 6).
fn is_env_array_secret_object(map: &Map<String, Value>) -> bool {
    let mut saw_value = false;
    let mut name_is_sensitive = false;
    for (key, val) in map {
        if key.eq_ignore_ascii_case("value") {
            saw_value = true;
        } else if key.eq_ignore_ascii_case("name") {
            if let Value::String(name) = val {
                // OR across duplicate casings (`name` + `Name`): a later
                // non-sensitive sibling must not clear an earlier hit.
                name_is_sensitive |= is_sensitive_key(name);
            }
        }
    }
    saw_value && name_is_sensitive
}

fn redact_object(map: &mut Map<String, Value>) {
    let redact_env_value = is_env_array_secret_object(map);
    for (key, val) in map.iter_mut() {
        if is_sensitive_key(key) || (redact_env_value && key.eq_ignore_ascii_case("value")) {
            *val = Value::String(REDACTED.to_string());
        } else {
            redact_in_place(val);
        }
    }
}

pub fn redact_sensitive_json(value: &Value) -> Value {
    let mut cloned = value.clone();
    redact_in_place(&mut cloned);
    cloned
}

#[cfg(test)]
mod tests {
    use super::{is_sensitive_key, redact_sensitive_json};

    #[test]
    fn redacts_exact_sensitive_keys() {
        let input = serde_json::json!({
            "headers": {
                "Authorization": "Bearer abc",
                "x-api-key": "k-123",
                "content-type": "application/json"
            },
            "password": "p@ss"
        });
        let out = redact_sensitive_json(&input);
        assert_eq!(out["headers"]["Authorization"], "[REDACTED]");
        assert_eq!(out["headers"]["x-api-key"], "[REDACTED]");
        assert_eq!(out["headers"]["content-type"], "application/json");
        assert_eq!(out["password"], "[REDACTED]");
    }

    #[test]
    fn redacts_nested_sensitive_keys() {
        let input = serde_json::json!({
            "body": {
                "clientSecret": "xyz",
                "nested": [{"authToken": "123"}, {"query": "ok"}]
            }
        });
        let out = redact_sensitive_json(&input);
        assert_eq!(out["body"]["clientSecret"], "[REDACTED]");
        assert_eq!(out["body"]["nested"][0]["authToken"], "[REDACTED]");
        assert_eq!(out["body"]["nested"][1]["query"], "ok");
    }

    #[test]
    fn does_not_over_redact_common_non_sensitive_keys() {
        assert!(!is_sensitive_key("author"));
        assert!(!is_sensitive_key("authorize_user"));
        assert!(!is_sensitive_key("token_count"));
        assert!(!is_sensitive_key("tokenize"));
        assert!(!is_sensitive_key("oauth_redirect_uri"));
    }

    #[test]
    fn still_redacts_expected_token_keys() {
        assert!(is_sensitive_key("auth_token"));
        assert!(is_sensitive_key("oauth_token"));
        assert!(is_sensitive_key("accessToken"));
        assert!(is_sensitive_key("apiKey"));
        assert!(is_sensitive_key("token_key"));
        assert!(is_sensitive_key("appTokenKey"));
        assert!(is_sensitive_key("userJwt"));
    }

    #[test]
    fn redacts_lowercase_digit_suffix_segments() {
        assert!(is_sensitive_key("password123"));
        assert!(is_sensitive_key("secret99"));
        assert!(is_sensitive_key("accounttoken2"));
    }

    #[test]
    fn redacts_json_key_name_cues_regardless_of_value_shape() {
        // #193 row 7: dominant tool-call-argument shape. Key-name cue is
        // enough; the value does not need to look like a known provider key.
        let input = serde_json::json!({
            "apiKey": "ck_live_not_a_known_prefix",
            "nested": {"clientSecret": "short"},
            "safe": "keep"
        });
        let out = redact_sensitive_json(&input);
        assert_eq!(out["apiKey"], "[REDACTED]");
        assert_eq!(out["nested"]["clientSecret"], "[REDACTED]");
        assert_eq!(out["safe"], "keep");
    }

    #[test]
    fn redacts_k8s_env_name_value_arrays_when_name_is_cue_like() {
        // #193 row 6: cue lives in the sibling `name` string, not the key.
        let input = serde_json::json!({
            "env": [
                {"name": "API_KEY", "value": "Zx9Qk2Lm7Pv4Rt8Wy1Nb6Hd3Fg5Jc0Ae"},
                {"name": "PORT", "value": "8080"},
                {"name": "DATABASE_PASSWORD", "value": "hunter2-adjacent"},
                {"Name": "OPENAI_API_KEY", "Value": "sk-not-scanned-here"}
            ]
        });
        let out = redact_sensitive_json(&input);
        assert_eq!(out["env"][0]["name"], "API_KEY");
        assert_eq!(out["env"][0]["value"], "[REDACTED]");
        assert_eq!(out["env"][1]["name"], "PORT");
        assert_eq!(out["env"][1]["value"], "8080");
        assert_eq!(out["env"][2]["value"], "[REDACTED]");
        assert_eq!(out["env"][3]["Value"], "[REDACTED]");
    }

    #[test]
    fn env_array_shape_does_not_redact_non_secret_names() {
        let input = serde_json::json!({
            "env": [
                {"name": "NODE_ENV", "value": "production"},
                {"name": "token_count", "value": "42"},
                {"name": "author", "value": "alice"}
            ]
        });
        let out = redact_sensitive_json(&input);
        assert_eq!(out, input);
    }

    #[test]
    fn env_array_ors_duplicate_name_casings() {
        // Pathological but legal JSON: both `Name` and `name` present.
        // A non-sensitive later sibling must not clear an earlier cue.
        let input = serde_json::json!({
            "Name": "API_KEY",
            "name": "PORT",
            "value": "Zx9Qk2Lm7Pv4Rt8Wy1Nb6Hd3Fg5Jc0Ae"
        });
        let out = redact_sensitive_json(&input);
        assert_eq!(out["value"], "[REDACTED]");
    }
}
