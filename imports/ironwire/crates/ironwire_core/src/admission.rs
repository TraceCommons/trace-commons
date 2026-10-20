//! Explicit, ephemeral admission metadata. No routing, credentials, or capture policy.

use std::collections::{BTreeMap, BTreeSet};

use crate::protocol::Protocol;

/// Maximum challenge lifetime accepted by the local control API.
pub const MAX_BINDING_SECONDS: i64 = 900;
/// Bound memory use even when clients abandon ceremonies.
pub const MAX_BINDINGS: usize = 32;
/// Expired sessions remain protected until explicit revocation or renewal.
pub const MAX_EXPIRED_SESSIONS: usize = 1024;

/// Fixed labels only; never includes a session, challenge, or body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionError {
    /// A request was not explicit or its canonical fields were invalid.
    #[error("admission-binding-invalid")]
    Invalid,
    /// The selected session no longer has a fresh challenge.
    #[error("admission-binding-expired")]
    Expired,
    /// The selected session was routed outside its consent scope.
    #[error("admission-binding-route-mismatch")]
    RouteMismatch,
    /// Existing metadata cannot safely carry the requested value.
    #[error("admission-binding-metadata-conflict")]
    MetadataConflict,
    /// The bounded registration set is full.
    #[error("admission-binding-capacity")]
    Capacity,
}

/// Immutable registration snapshot for an in-flight request.
#[derive(Clone)]
pub struct Binding {
    /// The sole backend permitted by this registration.
    pub backend: String,
    /// Canonical metadata; never log or expose through status.
    pub value: String,
    /// Unix second after which sending must be refused.
    pub expires_at: i64,
}

/// Memory-only registrations keyed by the client's exact session identifier.
#[derive(Default)]
pub struct AdmissionBindings(BTreeMap<String, Binding>, BTreeMap<String, i64>);

/// Same bounded identifier alphabet as the existing client session header.
#[must_use]
pub fn valid_session(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
}

impl AdmissionBindings {
    /// Record explicit consent without changing any general configuration.
    pub fn register(
        &mut self,
        session: &str,
        backend: &str,
        value: &str,
        confirmed: bool,
        now: i64,
    ) -> Result<i64, AdmissionError> {
        if !confirmed || !valid_session(session) || !valid_session(backend) {
            return Err(AdmissionError::Invalid);
        }
        let expires_at = expiry(value)?;
        if expires_at <= now {
            return Err(AdmissionError::Expired);
        }
        if expires_at > now.saturating_add(MAX_BINDING_SECONDS) {
            return Err(AdmissionError::Invalid);
        }
        // Forget expired payloads, never the fact that a session requires a
        // binding. When tombstones fill, retain the remaining expired entries
        // and refuse capacity instead of silently restoring unbound traffic.
        let expired: Vec<_> = self
            .0
            .iter()
            .filter(|(_, b)| b.expires_at <= now)
            .map(|(session, b)| (session.clone(), b.expires_at))
            .collect();
        for (session, expiry) in expired {
            if self.1.len() >= MAX_EXPIRED_SESSIONS {
                break;
            }
            self.1.insert(session.clone(), expiry);
            self.0.remove(&session);
        }
        if self.0.len() >= MAX_BINDINGS && !self.0.contains_key(session) {
            return Err(AdmissionError::Capacity);
        }
        self.0.insert(
            session.to_owned(),
            Binding {
                backend: backend.to_owned(),
                value: value.to_owned(),
                expires_at,
            },
        );
        self.1.remove(session);
        Ok(expires_at)
    }

    /// Revoke future insertion. In-flight requests already copied their bytes.
    pub fn revoke(&mut self, session: &str) -> Result<bool, AdmissionError> {
        if !valid_session(session) {
            return Err(AdmissionError::Invalid);
        }
        let active = self.0.remove(session).is_some();
        Ok(self.1.remove(session).is_some() || active)
    }

    /// Exact-session status exposes no registry enumeration or binding value.
    pub fn status(
        &self,
        session: &str,
        now: i64,
    ) -> Result<(&'static str, Option<i64>), AdmissionError> {
        if !valid_session(session) {
            return Err(AdmissionError::Invalid);
        }
        if let Some(expiry) = self.1.get(session) {
            return Ok(("expired", Some(*expiry)));
        }
        if let Some(binding) = self.0.get(session) {
            return Ok((
                if binding.expires_at > now {
                    "active"
                } else {
                    "expired"
                },
                Some(binding.expires_at),
            ));
        }
        Ok(("inactive", None))
    }

    /// A request snapshots its registration once; expiry is rechecked before send.
    pub fn snapshot(&self, session: &str, now: i64) -> Result<Option<Binding>, AdmissionError> {
        if self.status(session, now)?.0 == "expired" {
            return Err(AdmissionError::Expired);
        }
        Ok(self.0.get(session).cloned())
    }

    /// Only the exact registered session/backend/wire may acquire this metadata.
    /// `None` means a wholly unrelated request, whose bytes stay untouched.
    pub fn for_request(
        &self,
        session: Option<&str>,
        backend: &str,
        protocol: Protocol,
        now: i64,
    ) -> Result<Option<&str>, AdmissionError> {
        if session.is_some_and(|s| self.1.contains_key(s)) {
            return Err(AdmissionError::Expired);
        }
        let Some(binding) = session.and_then(|session| self.0.get(session)) else {
            return Ok(None);
        };
        if binding.expires_at <= now {
            return Err(AdmissionError::Expired);
        }
        if binding.backend != backend || protocol != Protocol::OpenAiChat {
            return Err(AdmissionError::RouteMismatch);
        }
        Ok(Some(&binding.value))
    }
}

fn expiry(value: &str) -> Result<i64, AdmissionError> {
    let fields: Vec<_> = value.split(':').collect();
    let hex = |v: &str| {
        v.len() == 64
            && v.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    if fields.len() != 4 || fields[0] != "tcad1" || !hex(fields[1]) || !hex(fields[2]) {
        return Err(AdmissionError::Invalid);
    }
    let timestamp: i64 = fields[3].parse().map_err(|_| AdmissionError::Invalid)?;
    if timestamp <= 0 || timestamp.to_string() != fields[3] {
        return Err(AdmissionError::Invalid);
    }
    Ok(timestamp)
}

/// Insert only the selected metadata key, preserving all unrelated source bytes.
/// Duplicate object keys are refused instead of choosing an interpretation.
pub fn insert_binding(body: &[u8], binding: &str) -> Result<Vec<u8>, AdmissionError> {
    expiry(binding)?;
    let top_members = members(body)?;
    let metadata = top_members.iter().find(|(key, _, _)| key == "metadata");
    let (at, fragment) = if let Some((_, start, end)) = metadata {
        let entries = members(&body[*start..*end])?;
        if let Some((_, value_start, value_end)) = entries
            .iter()
            .find(|(key, _, _)| key == "trace_commons_admission")
        {
            let existing =
                serde_json::from_slice::<String>(&body[start + value_start..start + value_end])
                    .map_err(|_| AdmissionError::MetadataConflict)?;
            return if existing == binding {
                Ok(body.to_vec())
            } else {
                Err(AdmissionError::MetadataConflict)
            };
        }
        (
            *end - 1,
            format!(
                "{}\"trace_commons_admission\":\"{binding}\"",
                if entries.is_empty() { "" } else { "," }
            ),
        )
    } else {
        (
            body.iter()
                .rposition(|b| *b == b'}')
                .ok_or(AdmissionError::MetadataConflict)?,
            format!(
                "{}\"metadata\":{{\"trace_commons_admission\":\"{binding}\"}}",
                if top_members.is_empty() { "" } else { "," }
            ),
        )
    };
    let mut result = Vec::with_capacity(body.len() + fragment.len());
    result.extend_from_slice(&body[..at]);
    result.extend_from_slice(fragment.as_bytes());
    result.extend_from_slice(&body[at..]);
    Ok(result)
}

type Member = (String, usize, usize);
pub(crate) fn members(body: &[u8]) -> Result<Vec<Member>, AdmissionError> {
    let fail = AdmissionError::MetadataConflict;
    let whitespace = |at: &mut usize| {
        while body.get(*at).is_some_and(u8::is_ascii_whitespace) {
            *at += 1;
        }
    };
    let mut at = 0;
    whitespace(&mut at);
    if body.get(at) != Some(&b'{') {
        return Err(fail);
    }
    at += 1;
    whitespace(&mut at);
    let mut members = Vec::new();
    let mut names = BTreeSet::new();
    let mut after_comma = false;
    loop {
        if body.get(at) == Some(&b'}') {
            if after_comma {
                return Err(fail);
            }
            at += 1;
            whitespace(&mut at);
            return if at == body.len() {
                Ok(members)
            } else {
                Err(fail)
            };
        }
        let mut key = serde_json::Deserializer::from_slice(&body[at..]).into_iter::<String>();
        let name = key.next().ok_or(fail)?.map_err(|_| fail)?;
        at += key.byte_offset();
        if !names.insert(name.clone()) {
            return Err(fail);
        }
        whitespace(&mut at);
        if body.get(at) != Some(&b':') {
            return Err(fail);
        }
        at += 1;
        whitespace(&mut at);
        let start = at;
        let mut value =
            serde_json::Deserializer::from_slice(&body[at..]).into_iter::<serde::de::IgnoredAny>();
        value.next().ok_or(fail)?.map_err(|_| fail)?;
        at += value.byte_offset();
        members.push((name, start, at));
        whitespace(&mut at);
        after_comma = body.get(at) == Some(&b',');
        if after_comma {
            at += 1;
            whitespace(&mut at);
        } else if body.get(at) != Some(&b'}') {
            return Err(fail);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding(expiry: i64) -> String {
        format!("tcad1:{}:{}:{expiry}", "a".repeat(64), "b".repeat(64))
    }

    #[test]
    fn only_the_exact_fresh_session_backend_and_protocol_receives_a_binding() {
        let mut state = AdmissionBindings::default();
        let token = binding(1000);
        assert!(
            state
                .register("session", "nearai", &token, false, 500)
                .is_err()
        );
        state
            .register("session", "nearai", &token, true, 500)
            .unwrap();
        assert_eq!(
            state
                .for_request(Some("session"), "nearai", Protocol::OpenAiChat, 999)
                .unwrap(),
            Some(token.as_str())
        );
        assert_eq!(
            state
                .for_request(Some("other"), "nearai", Protocol::OpenAiChat, 999)
                .unwrap(),
            None
        );
        assert_eq!(
            state
                .for_request(None, "nearai", Protocol::OpenAiChat, 999)
                .unwrap(),
            None
        );
        assert_eq!(
            state.for_request(Some("session"), "other", Protocol::OpenAiChat, 999),
            Err(AdmissionError::RouteMismatch)
        );
        assert_eq!(
            state.for_request(Some("session"), "nearai", Protocol::AnthropicMessages, 999),
            Err(AdmissionError::RouteMismatch)
        );
        assert_eq!(
            state.for_request(Some("session"), "nearai", Protocol::OpenAiChat, 1000),
            Err(AdmissionError::Expired)
        );
        assert!(state.revoke("session").unwrap());
        assert_eq!(
            state
                .for_request(Some("session"), "nearai", Protocol::OpenAiChat, 1001)
                .unwrap(),
            None
        );
    }

    #[test]
    fn oversized_lifetime_ambiguous_or_noncanonical_bindings_are_refused() {
        let mut state = AdmissionBindings::default();
        for token in [
            binding(2000),
            binding(1000).replace("tcad1", "tcad2"),
            binding(1000).replace('a', "A"),
            binding(1000).replace(":1000", ":01000"),
        ] {
            assert!(
                state
                    .register("session", "nearai", &token, true, 500)
                    .is_err()
            );
        }
        assert!(
            state
                .register("bad session", "nearai", &binding(1000), true, 500)
                .is_err()
        );
    }

    #[test]
    fn abandoned_bindings_are_bounded_and_restart_drops_every_registration() {
        let mut state = AdmissionBindings::default();
        let token = binding(1000);
        for index in 0..MAX_BINDINGS {
            state
                .register(&format!("session-{index}"), "nearai", &token, true, 500)
                .unwrap();
        }
        assert_eq!(
            state.register("overflow", "nearai", &token, true, 500),
            Err(AdmissionError::Capacity)
        );
        state
            .register("session-0", "nearai", &token, true, 500)
            .unwrap();
        state.revoke("session-1").unwrap();
        state
            .register("replacement", "nearai", &token, true, 500)
            .unwrap();
        assert_eq!(
            AdmissionBindings::default()
                .for_request(Some("session-0"), "nearai", Protocol::OpenAiChat, 501)
                .unwrap(),
            None
        );
    }

    #[test]
    fn targeted_insertion_preserves_unknown_fields_whitespace_and_identical_values() {
        let token = binding(1000);
        let body = br#" { "model":"x", "unknown":1.500, "metadata" : { "key": ["}", "a\\b"] } } "#;
        let inserted = insert_binding(body, &token).unwrap();
        let addition = format!(",\"trace_commons_admission\":\"{token}\"");
        assert_eq!(
            String::from_utf8(inserted.clone())
                .unwrap()
                .replace(&addition, "")
                .as_bytes(),
            body
        );
        assert_eq!(insert_binding(&inserted, &token).unwrap(), inserted);
        for bad in [
            r#"{"metadata":null}"#,
            r#"{"metadata":{},"metadata":{}}"#,
            r#"{"metadata":{"trace_commons_admission":"different"}}"#,
            r#"{"metadata":{"x":1,"x":2}}"#,
            r#"{"x":1,}"#,
        ] {
            assert!(insert_binding(bad.as_bytes(), &token).is_err());
        }
        let inserted = insert_binding(b"{}", &token).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&inserted).unwrap()["metadata"]["trace_commons_admission"],
            token
        );
    }
    #[test]
    fn expiration_reclaims_active_capacity_without_unbinding_old_sessions() {
        let mut state = AdmissionBindings::default();
        for n in 0..MAX_BINDINGS {
            state
                .register(&format!("old-{n}"), "nearai", &binding(1000), true, 500)
                .unwrap();
        }
        state
            .register("new", "nearai", &binding(1500), true, 1000)
            .unwrap();
        assert_eq!(state.0.len(), 1);
        assert_eq!(
            state.status("old-0", 1000).unwrap(),
            ("expired", Some(1000))
        );
        assert_eq!(
            state.for_request(Some("old-0"), "nearai", Protocol::OpenAiChat, 1000),
            Err(AdmissionError::Expired)
        );
        assert_eq!(state.status("unrelated", 1000).unwrap(), ("inactive", None));
        state
            .register("old-0", "nearai", &binding(1500), true, 1000)
            .unwrap();
        assert_eq!(state.status("old-0", 1000).unwrap(), ("active", Some(1500)));
        assert!(state.revoke("old-1").unwrap());
        assert_eq!(state.status("old-1", 1000).unwrap(), ("inactive", None));
    }
    #[test]
    fn bounded_tombstones_never_forget_an_expired_session() {
        let mut state = AdmissionBindings::default();
        for n in 0..MAX_EXPIRED_SESSIONS {
            state.1.insert(format!("old-{n}"), 500);
        }
        for n in 0..MAX_BINDINGS {
            state
                .register(&format!("live-{n}"), "nearai", &binding(1000), true, 500)
                .unwrap();
        }
        assert_eq!(
            state.register("overflow", "nearai", &binding(1500), true, 1000),
            Err(AdmissionError::Capacity)
        );
        assert!(matches!(
            state.snapshot("old-0", 1000),
            Err(AdmissionError::Expired)
        ));
        assert!(matches!(
            state.snapshot("live-0", 1000),
            Err(AdmissionError::Expired)
        ));
        state.revoke("old-0").unwrap();
        state
            .register("replacement", "nearai", &binding(1500), true, 1000)
            .unwrap();
        assert_eq!(state.1.len(), MAX_EXPIRED_SESSIONS);
    }
}
