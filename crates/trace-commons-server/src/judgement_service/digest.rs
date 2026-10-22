// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The one thing the service reads out of a digest: how many marked events
//! it carries.
//!
//! `event_kind` is judged per marked event, for at most
//! [`JUDGEMENT_EVENT_KIND_MAX`] in digest order (spec section 3), so the
//! service has to count them. Everything else in the digest goes to the model
//! as data and is never interpreted here.
//!
//! The digest is built by `trace-commons-contributor-insights`
//! (`judgement_digest.rs`, addendum delivery step 1): a JSON object with
//! `schema_version: 1` and an `events` array of at most
//! [`DIGEST_EVENTS_MAX`] markers. This module reads those two fields
//! leniently, as `serde_json::Value`, rather than depending on that crate's
//! types, so a field the builder adds later does not refuse a digest. A
//! digest without them is refused as malformed.
//!
//! [`JUDGEMENT_EVENT_KIND_MAX`]: trace_commons_protocol::insights_judgement::JUDGEMENT_EVENT_KIND_MAX

use trace_commons_protocol::insights_judgement::JudgementRefusal;

/// The digest schema version this service reads.
pub const DIGEST_SCHEMA_VERSION: u64 = 1;
/// The most markers a digest carries (`MAX_DIGEST_EVENTS` in the builder).
pub const DIGEST_EVENTS_MAX: usize = 64;

/// The number of marked events in `digest`.
///
/// [`JudgementRefusal::RequestMalformed`] when the digest is not a JSON
/// object, its `schema_version` is not 1, `events` is not an array, or the
/// array is longer than [`DIGEST_EVENTS_MAX`].
pub fn marked_event_count(digest: &str) -> Result<u32, JudgementRefusal> {
    let document: serde_json::Value =
        serde_json::from_str(digest).map_err(|_| JudgementRefusal::RequestMalformed)?;
    if document
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        != Some(DIGEST_SCHEMA_VERSION)
    {
        return Err(JudgementRefusal::RequestMalformed);
    }
    let events = document
        .get("events")
        .and_then(serde_json::Value::as_array)
        .ok_or(JudgementRefusal::RequestMalformed)?;
    if events.len() > DIGEST_EVENTS_MAX {
        return Err(JudgementRefusal::RequestMalformed);
    }
    u32::try_from(events.len()).map_err(|_| JudgementRefusal::RequestMalformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_the_events_array() {
        assert_eq!(
            marked_event_count(r#"{"schema_version":1,"events":[]}"#),
            Ok(0)
        );
        assert_eq!(
            marked_event_count(
                r#"{"schema_version":1,"counts":{},"events":[{"prompt_ix":0,"kind":"denial"},{"prompt_ix":3,"kind":"undo"}]}"#
            ),
            Ok(2)
        );
    }

    #[test]
    fn refuses_what_is_not_a_version_one_digest() {
        let too_many = format!(
            r#"{{"schema_version":1,"events":[{}]}}"#,
            vec![r#"{"prompt_ix":0,"kind":"denial"}"#; DIGEST_EVENTS_MAX + 1].join(",")
        );
        for digest in [
            "not json",
            "[]",
            r#"{"events":[]}"#,
            r#"{"schema_version":2,"events":[]}"#,
            r#"{"schema_version":"1","events":[]}"#,
            r#"{"schema_version":1}"#,
            r#"{"schema_version":1,"events":{}}"#,
            too_many.as_str(),
        ] {
            assert_eq!(
                marked_event_count(digest),
                Err(JudgementRefusal::RequestMalformed),
                "{digest}"
            );
        }
    }
}
