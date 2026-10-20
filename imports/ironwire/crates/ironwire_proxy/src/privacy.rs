//! Wiring the privacy filter into the request path.
//!
//! `ironwire_privacy` has no idea what a conversation is; this module gives it
//! one. Two jobs:
//!
//! 1. **Own the salts.** A conversation's salt has to be stable across its
//!    turns, or the same email mints a different token every turn, the prefix
//!    changes, and the provider's prompt cache is destroyed on every request —
//!    which would cost far more than the filter saves.
//! 2. **Reverse the stream.** The response arrives as bytes with boundaries the
//!    network chose, so the reverser needs UTF-8 reassembly on top of its own
//!    placeholder reassembly.
//!
//! Salts are memory-only and bounded. Nothing here is written to disk
//! (`docs/PRIVACY.md` §4).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use ironwire_core::config::{PrivacyConfig, PrivacyMode};
use ironwire_core::policy::ConversationKey;
use ironwire_privacy::{Detector, Exemptions, Map, ReverseError, Reverser, Salt, Tiers};
use ironwire_upstream::backend::UpstreamError;

/// How many conversations' salts to keep.
///
/// A salt is 32 bytes and a conversation is long-lived; this is generous. The
/// bound exists because a long-running daemon must not accumulate state
/// forever, and losing a salt is harmless — the conversation simply gets new
/// placeholders from the next turn, at the cost of one cold prompt cache.
const MAX_SALTS: usize = 512;

/// The filter, as the proxy holds it.
pub struct PrivacyFilter {
    detector: Detector,
    exemptions: Exemptions,
    salts: Mutex<HashMap<ConversationKey, Salt>>,
    /// Insertion order, for eviction. A full LRU would need a touch on every
    /// request; the difference does not matter for a cache whose miss costs one
    /// cold cache.
    order: Mutex<Vec<ConversationKey>>,
    summary: String,
}

impl std::fmt::Debug for PrivacyFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrivacyFilter")
            .field("summary", &self.summary)
            .finish_non_exhaustive()
    }
}

impl PrivacyFilter {
    /// Build a filter, or `None` when the config would not match anything.
    ///
    /// Enabled-but-empty returns `None` on purpose: a filter that matches
    /// nothing should read as off everywhere, not as protection.
    #[must_use]
    pub fn from_config(config: &PrivacyConfig) -> Option<Self> {
        if !config.is_active() {
            return None;
        }
        Some(Self {
            detector: Detector::new(&Tiers {
                // Every level above `off` substitutes credentials; the ladder is
                // cumulative, so this is the one place that mapping lives.
                secrets: config.secrets || config.mode() >= PrivacyMode::Pii,
                named_values: config.named_values.clone(),
                pii: config.mode() >= PrivacyMode::Pii,
            }),
            exemptions: Exemptions {
                code_blocks: !config.scan_code_blocks,
                tool_results: !config.scan_tool_results,
            },
            salts: Mutex::new(HashMap::new()),
            order: Mutex::new(Vec::new()),
            summary: config.summary(),
        })
    }

    /// One line describing what is running.
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// The salt for a conversation, minting one on first sight.
    fn salt_for(&self, key: &ConversationKey) -> Salt {
        let mut salts = match self.salts.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(existing) = salts.get(key) {
            return existing.clone();
        }

        let salt = Salt::random();
        salts.insert(key.clone(), salt.clone());

        let mut order = match self.order.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        order.push(key.clone());
        while order.len() > MAX_SALTS {
            let evicted = order.remove(0);
            salts.remove(&evicted);
        }
        salt
    }

    /// Substitute over a request body.
    #[must_use]
    pub fn apply(&self, key: &ConversationKey, body: &serde_json::Value) -> Applied {
        let salt = self.salt_for(key);
        let result = ironwire_privacy::substitute(&self.detector, &salt, &self.exemptions, body);
        Applied {
            body: result.body,
            map: Arc::new(result.map),
            substitutions: result.substitutions,
        }
    }
}

/// What the filter did to one request.
#[derive(Debug)]
pub struct Applied {
    /// Body to forward.
    pub body: serde_json::Value,
    /// Map that reverses it, shared with the response stream.
    pub map: Arc<Map>,
    /// Distinct values replaced.
    pub substitutions: usize,
}

/// Wrap a response stream so placeholders are put back on the way to the client.
///
/// The stream fails rather than forwarding a partial or unreversed placeholder
/// (`docs/PRIVACY.md` §5). That is the whole point: a failed turn the user can
/// retry is vastly better than a corrupted transcript they will not notice for
/// a week.
pub fn reverse_stream<S>(
    inner: S,
    map: Arc<Map>,
) -> impl Stream<Item = Result<Bytes, UpstreamError>> + Send
where
    S: Stream<Item = Result<Bytes, UpstreamError>> + Send + 'static,
{
    async_stream::stream! {
        let mut reverser = Reverser::new();
        // Bytes that were not valid UTF-8 on their own because a codepoint was
        // split across the chunk boundary. Distinct from the reverser's own
        // buffer, which holds *valid* text it cannot yet classify.
        let mut carry: Vec<u8> = Vec::new();
        let mut inner = std::pin::pin!(inner);

        while let Some(chunk) = inner.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    yield Err(error);
                    return;
                }
            };

            carry.extend_from_slice(&chunk);
            let text = match std::str::from_utf8(&carry) {
                Ok(text) => {
                    let owned = text.to_string();
                    carry.clear();
                    owned
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    // A genuinely invalid sequence, not a split codepoint: pass
                    // the bytes through untouched rather than mangling someone's
                    // response to fit our expectations.
                    if error.error_len().is_some() {
                        yield Ok(Bytes::from(std::mem::take(&mut carry)));
                        continue;
                    }
                    let owned = String::from_utf8_lossy(&carry[..valid]).to_string();
                    carry.drain(..valid);
                    owned
                }
            };

            let out = reverser.push(&map, &text);
            if !out.is_empty() {
                yield Ok(Bytes::from(out));
            }
        }

        match reverser.finish(&map) {
            Ok(rest) => {
                if !rest.is_empty() {
                    yield Ok(Bytes::from(rest));
                }
                if !carry.is_empty() {
                    yield Ok(Bytes::from(carry));
                }
            }
            Err(ReverseError::Unreversed { count }) => {
                tracing::error!(
                    count,
                    "the response ended with unreversed placeholders; failing the \
                     exchange rather than writing them into the transcript"
                );
                yield Err(UpstreamError::Transport {
                    backend: ironwire_core::protocol::BackendId::from("ironwire-privacy"),
                    detail: format!(
                        "the response ended mid-substitution ({count} value(s) could not \
                         be restored). IronWire failed the turn rather than writing \
                         placeholder tokens into your transcript, which would be \
                         permanent. Retry, or turn the privacy filter off."
                    ),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    use ironwire_core::protocol::Protocol;

    fn config() -> PrivacyConfig {
        PrivacyConfig {
            enabled: true,
            secrets: false,
            named_values: vec!["alice@corp.com".to_string()],
            ..PrivacyConfig::default()
        }
    }

    fn key(seed: &str) -> ConversationKey {
        ConversationKey::derive(Protocol::AnthropicMessages, seed, &[])
    }

    #[test]
    fn a_disabled_filter_is_not_built_at_all() {
        assert!(PrivacyFilter::from_config(&PrivacyConfig::default()).is_none());
    }

    #[test]
    fn enabled_but_matching_nothing_reads_as_off() {
        // Otherwise `ironwire status` would say a filter is running when it
        // cannot possibly do anything, which is the exact sort of false
        // reassurance TRUST I7 forbids.
        let empty = PrivacyConfig {
            enabled: true,
            secrets: false,
            named_values: Vec::new(),
            ..PrivacyConfig::default()
        };
        assert!(PrivacyFilter::from_config(&empty).is_none());
    }

    #[test]
    fn a_conversation_keeps_one_salt_across_turns() {
        // If it did not, the same value would mint a different token every
        // turn, the prefix would change, and the provider's prompt cache would
        // be destroyed on every request — costing far more than the filter
        // saves.
        let filter = PrivacyFilter::from_config(&config()).expect("builds");
        let body = serde_json::json!({"messages": [{"role": "user", "content": "alice@corp.com"}]});
        let first = filter.apply(&key("session-a"), &body);
        let second = filter.apply(&key("session-a"), &body);
        assert_eq!(first.body, second.body, "the token changed between turns");
    }

    #[test]
    fn two_conversations_get_different_tokens_for_the_same_value() {
        let filter = PrivacyFilter::from_config(&config()).expect("builds");
        let body = serde_json::json!({"messages": [{"role": "user", "content": "alice@corp.com"}]});
        let a = filter.apply(&key("session-a"), &body);
        let b = filter.apply(&key("session-b"), &body);
        assert_ne!(a.body, b.body);
    }

    #[test]
    fn the_salt_table_is_bounded() {
        // A long-running daemon must not accumulate state forever. Losing a
        // salt is harmless: the conversation gets new placeholders and pays for
        // one cold prompt cache.
        let filter = PrivacyFilter::from_config(&config()).expect("builds");
        let body = serde_json::json!({"messages": []});
        for i in 0..(MAX_SALTS + 50) {
            let _ = filter.apply(&key(&format!("s{i}")), &body);
        }
        assert!(filter.salts.lock().expect("lock").len() <= MAX_SALTS);
    }

    async fn collect(
        stream: impl Stream<Item = Result<Bytes, UpstreamError>> + Send,
    ) -> Result<String, String> {
        let mut stream = std::pin::pin!(stream);
        let mut out = String::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => out.push_str(&String::from_utf8_lossy(&bytes)),
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(out)
    }

    #[tokio::test]
    async fn a_response_is_reversed_on_the_way_out() {
        let filter = PrivacyFilter::from_config(&config()).expect("builds");
        let body = serde_json::json!({"messages": [{"role": "user", "content": "alice@corp.com"}]});
        let applied = filter.apply(&key("s"), &body);
        let token = applied
            .map
            .placeholders()
            .next()
            .expect("minted")
            .to_string();

        let chunks = vec![
            Ok(Bytes::from(format!("mail {token}"))),
            Ok(Bytes::from(" now".to_string())),
        ];
        let reversed = reverse_stream(stream::iter(chunks), applied.map);
        assert_eq!(
            collect(reversed).await.expect("no failure"),
            "mail alice@corp.com now"
        );
    }

    #[tokio::test]
    async fn a_placeholder_split_across_chunks_is_reversed() {
        let filter = PrivacyFilter::from_config(&config()).expect("builds");
        let body = serde_json::json!({"messages": [{"role": "user", "content": "alice@corp.com"}]});
        let applied = filter.apply(&key("s"), &body);
        let token = applied
            .map
            .placeholders()
            .next()
            .expect("minted")
            .to_string();
        let text = format!("x{token}y");

        // Split inside the token, on a character boundary.
        let mid = text
            .char_indices()
            .map(|(i, _)| i)
            .find(|i| *i > 4)
            .expect("boundary");
        let chunks = vec![
            Ok(Bytes::from(text[..mid].to_string())),
            Ok(Bytes::from(text[mid..].to_string())),
        ];
        let reversed = reverse_stream(stream::iter(chunks), applied.map);
        assert_eq!(
            collect(reversed).await.expect("no failure"),
            "xalice@corp.comy"
        );
    }

    #[tokio::test]
    async fn a_multibyte_codepoint_split_across_chunks_survives() {
        // The reverser works on `&str`; this wrapper is what makes that safe.
        let filter = PrivacyFilter::from_config(&config()).expect("builds");
        let applied = filter.apply(&key("s"), &serde_json::json!({}));

        let text = "日本語のテキスト";
        let bytes = text.as_bytes();
        let chunks = vec![
            Ok(Bytes::copy_from_slice(&bytes[..5])), // mid-codepoint
            Ok(Bytes::copy_from_slice(&bytes[5..])),
        ];
        let reversed = reverse_stream(stream::iter(chunks), applied.map);
        assert_eq!(collect(reversed).await.expect("no failure"), text);
    }

    #[tokio::test]
    async fn a_truncated_placeholder_fails_the_exchange() {
        // Forwarding it would put a token into the client's permanent
        // transcript. A failed turn the user can retry is vastly better.
        let filter = PrivacyFilter::from_config(&config()).expect("builds");
        let body = serde_json::json!({"messages": [{"role": "user", "content": "alice@corp.com"}]});
        let applied = filter.apply(&key("s"), &body);
        let token = applied
            .map
            .placeholders()
            .next()
            .expect("minted")
            .to_string();
        let truncated = token[..token.len() - 5].to_string();

        let chunks = vec![Ok(Bytes::from(format!("mail {truncated}")))];
        let reversed = reverse_stream(stream::iter(chunks), applied.map);
        let error = collect(reversed).await.expect_err("must fail");
        assert!(error.contains("mid-substitution"), "got: {error}");
    }

    #[tokio::test]
    async fn a_stream_with_no_substitutions_passes_through_unchanged() {
        let filter = PrivacyFilter::from_config(&config()).expect("builds");
        let applied = filter.apply(&key("s"), &serde_json::json!({}));
        let text = "an entirely ordinary response";
        let chunks = vec![Ok(Bytes::from(text))];
        let reversed = reverse_stream(stream::iter(chunks), applied.map);
        assert_eq!(collect(reversed).await.expect("no failure"), text);
    }

    #[tokio::test]
    async fn an_upstream_error_is_forwarded_not_swallowed() {
        let filter = PrivacyFilter::from_config(&config()).expect("builds");
        let applied = filter.apply(&key("s"), &serde_json::json!({}));
        let chunks = vec![
            Ok(Bytes::from("partial")),
            Err(UpstreamError::Transport {
                backend: ironwire_core::protocol::BackendId::from("x"),
                detail: "connection reset".to_string(),
            }),
        ];
        let reversed = reverse_stream(stream::iter(chunks), applied.map);
        let error = collect(reversed).await.expect_err("must surface");
        assert!(error.contains("connection reset"), "got: {error}");
    }
}
