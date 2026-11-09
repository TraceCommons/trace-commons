//! The Terms of Service document the community site publishes at
//! `/legal/terms.json`, and the acceptance record a client posts to the
//! account API. The client hashes the exact bytes it rendered; the server
//! stores the version id and that hash and does not validate them against the
//! published document.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

pub const TERMS_SCHEMA_VERSION: &str = "trace_commons.terms.v1";
pub const TERMS_ACCEPTANCE_PATH: &str = "/v1/account/terms-acceptance";

/// How far ahead of the server clock an `accepted_at` may be.
const ACCEPTED_AT_SKEW_MINUTES: i64 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermsDocument {
    pub schema_version: String,
    pub version: String,
    pub effective: String,
    pub in_effect: bool,
    pub title: String,
    pub blocks: Vec<TermsBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TermsBlock {
    Heading {
        text: String,
    },
    Paragraph {
        text: String,
    },
    List {
        items: Vec<String>,
    },
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Callout {
        text: String,
    },
}

impl TermsBlock {
    fn texts(&self) -> Vec<&str> {
        match self {
            TermsBlock::Heading { text }
            | TermsBlock::Paragraph { text }
            | TermsBlock::Callout { text } => vec![text.as_str()],
            TermsBlock::List { items } => items.iter().map(String::as_str).collect(),
            TermsBlock::Table { header, rows } => header
                .iter()
                .chain(rows.iter().flatten())
                .map(String::as_str)
                .collect(),
        }
    }
}

impl TermsDocument {
    /// Parse the exact bytes the site served. Refusal labels are safe to log.
    pub fn parse(bytes: &[u8]) -> Result<TermsDocument, &'static str> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| "terms-malformed")?;
        match value.get("schema_version").and_then(|v| v.as_str()) {
            Some(TERMS_SCHEMA_VERSION) => {}
            Some(_) => return Err("terms-schema-unknown"),
            None => return Err("terms-malformed"),
        }
        let doc: TermsDocument = serde_json::from_value(value).map_err(|_| "terms-malformed")?;
        if !valid_terms_version(&doc.version) {
            return Err("terms-version-invalid");
        }
        if doc.blocks.is_empty() {
            return Err("terms-empty");
        }
        let texts = doc
            .blocks
            .iter()
            .flat_map(TermsBlock::texts)
            .chain(std::iter::once(doc.title.as_str()));
        for text in texts {
            if !links_allowed(text) {
                return Err("terms-link-refused");
            }
        }
        Ok(doc)
    }
}

/// Every `](target)` in the text must be an `https://` or `mailto:` target.
fn links_allowed(text: &str) -> bool {
    let mut rest = text;
    while let Some(at) = rest.find("](") {
        let after = &rest[at + 2..];
        let Some(end) = after.find(')') else {
            return false;
        };
        let target = &after[..end];
        if !(target.starts_with("https://") || target.starts_with("mailto:")) {
            return false;
        }
        rest = &after[end + 1..];
    }
    true
}

pub fn valid_terms_version(v: &str) -> bool {
    (1..=64).contains(&v.len())
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

pub fn valid_content_sha256(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TermsAcceptanceRequest {
    pub terms_version: String,
    pub content_sha256: String,
    pub accepted_at: DateTime<Utc>,
}

impl TermsAcceptanceRequest {
    pub fn validate(&self, now: DateTime<Utc>) -> Result<(), &'static str> {
        if !valid_terms_version(&self.terms_version) {
            return Err("terms-version-invalid");
        }
        if !valid_content_sha256(&self.content_sha256) {
            return Err("terms-hash-invalid");
        }
        if self.accepted_at > now + Duration::minutes(ACCEPTED_AT_SKEW_MINUTES) {
            return Err("terms-accepted-at-future");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermsAcceptanceResponse {
    pub recorded_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/terms-v1.json");

    fn doc_with(mutate: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let mut v: serde_json::Value = serde_json::from_slice(FIXTURE).unwrap();
        mutate(&mut v);
        serde_json::to_vec(&v).unwrap()
    }

    #[test]
    fn parses_every_block_kind() {
        let doc = TermsDocument::parse(FIXTURE).unwrap();
        assert!(!doc.in_effect);
        assert_eq!(doc.version, "2026-08-22-draft");
        let kinds: Vec<&str> = doc
            .blocks
            .iter()
            .map(|b| match b {
                TermsBlock::Heading { .. } => "heading",
                TermsBlock::Paragraph { .. } => "paragraph",
                TermsBlock::List { .. } => "list",
                TermsBlock::Table { .. } => "table",
                TermsBlock::Callout { .. } => "callout",
            })
            .collect();
        assert_eq!(kinds, ["heading", "paragraph", "list", "table", "callout"]);
    }

    #[test]
    fn refuses_unknown_schema() {
        let bytes = doc_with(|v| v["schema_version"] = "trace_commons.terms.v2".into());
        assert_eq!(TermsDocument::parse(&bytes), Err("terms-schema-unknown"));
    }

    #[test]
    fn refuses_unknown_block_kind() {
        let bytes = doc_with(|v| {
            v["blocks"] = serde_json::json!([{"kind": "video", "text": "x"}]);
        });
        assert_eq!(TermsDocument::parse(&bytes), Err("terms-malformed"));
        assert_eq!(TermsDocument::parse(b"not json"), Err("terms-malformed"));
    }

    #[test]
    fn refuses_empty_and_bad_version() {
        let bytes = doc_with(|v| v["blocks"] = serde_json::json!([]));
        assert_eq!(TermsDocument::parse(&bytes), Err("terms-empty"));
        let bytes = doc_with(|v| v["version"] = "a b".into());
        assert_eq!(TermsDocument::parse(&bytes), Err("terms-version-invalid"));
    }

    #[test]
    fn refuses_relative_and_javascript_links() {
        for target in [
            "[x](/a)",
            "[x](javascript:alert(1))",
            "[x](http://a.example)",
        ] {
            let bytes = doc_with(|v| {
                v["blocks"] = serde_json::json!([{"kind": "paragraph", "text": target}]);
            });
            assert_eq!(
                TermsDocument::parse(&bytes),
                Err("terms-link-refused"),
                "{target}"
            );
        }
        let bytes = doc_with(|v| {
            v["blocks"] = serde_json::json!([{"kind": "paragraph", "text": "[x](mailto:a@b.c)"}]);
        });
        assert!(TermsDocument::parse(&bytes).is_ok());
    }

    #[test]
    fn version_shape() {
        assert!(!valid_terms_version(""));
        assert!(!valid_terms_version(&"a".repeat(65)));
        assert!(!valid_terms_version("a b"));
        assert!(valid_terms_version(&"a".repeat(64)));
        assert!(valid_terms_version("2026-08-22-draft"));
    }

    #[test]
    fn hash_shape() {
        assert!(valid_content_sha256(&"a0".repeat(32)));
        assert!(!valid_content_sha256(&"A0".repeat(32)));
        assert!(!valid_content_sha256(&"a0".repeat(31)));
        assert!(!valid_content_sha256(&"g0".repeat(32)));
    }

    fn request_at(accepted_at: DateTime<Utc>) -> TermsAcceptanceRequest {
        TermsAcceptanceRequest {
            terms_version: "2026-08-22-draft".into(),
            content_sha256: "ab".repeat(32),
            accepted_at,
        }
    }

    #[test]
    fn accepted_at_bounds() {
        let now = Utc::now();
        assert_eq!(request_at(now + Duration::minutes(4)).validate(now), Ok(()));
        assert_eq!(
            request_at(now + Duration::minutes(6)).validate(now),
            Err("terms-accepted-at-future")
        );
        assert_eq!(request_at(now - Duration::days(30)).validate(now), Ok(()));
        let mut bad = request_at(now);
        bad.content_sha256 = "XY".into();
        assert_eq!(bad.validate(now), Err("terms-hash-invalid"));
        bad = request_at(now);
        bad.terms_version = String::new();
        assert_eq!(bad.validate(now), Err("terms-version-invalid"));
    }

    #[test]
    fn request_rejects_unknown_fields() {
        let json = serde_json::json!({
            "terms_version": "v1",
            "content_sha256": "ab".repeat(32),
            "accepted_at": "2026-11-09T00:00:00Z",
            "extra": 1
        });
        assert!(serde_json::from_value::<TermsAcceptanceRequest>(json).is_err());
    }
}
