// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use trace_commons_protocol::trace_contribution::SourceSessionIdentity;
use trace_commons_server::trace_session_identity::{canonical_source_session, session_digest};

fn identity(adapter: &str, native_id: &str) -> SourceSessionIdentity {
    SourceSessionIdentity {
        adapter: adapter.into(),
        native_id: native_id.into(),
    }
}

#[test]
fn canonical_identity_accepts_supported_adapters() {
    let uuid = "b3ddbb00-52dd-438e-b516-601985ed6f0d";
    for (adapter, native_id) in [
        ("codex", uuid),
        ("claude-code", uuid),
        ("opencode", "ses_abc-123"),
        ("cline", "task_abc-123"),
        ("gemini-cli", "session_abc-123"),
    ] {
        assert!(
            canonical_source_session(&identity(adapter, native_id)).is_ok(),
            "{adapter}"
        );
    }
}

#[test]
fn canonical_identity_rejects_missing_or_path_like_values() {
    let uuid = "b3ddbb00-52dd-438e-b516-601985ed6f0d";
    for (adapter, native_id) in [
        ("trajectory", "abc"),
        ("Codex", uuid),
        ("", uuid),
        ("opencode", ""),
        ("opencode", "../abc"),
        ("opencode", "a/b"),
        ("opencode", "a\\b"),
        ("opencode", "has space"),
        ("opencode", "abc\n"),
        ("opencode", "nonascii-é"),
        ("opencode", &"a".repeat(129)),
        ("codex", "B3DDBB00-52DD-438E-B516-601985ED6F0D"),
        ("claude-code", "not-a-uuid"),
        ("codex", "b3ddbb0052dd438eb516601985ed6f0d"),
    ] {
        assert!(
            canonical_source_session(&identity(adapter, native_id)).is_err(),
            "{adapter}: {native_id}"
        );
    }
}

#[test]
fn digest_is_domain_separated_and_unambiguous() {
    let hash = |adapter, native_id| {
        session_digest(&canonical_source_session(&identity(adapter, native_id)).unwrap())
    };
    let first = hash("opencode", "ses_abc");
    let first_hex = first
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        first_hex,
        "9c0b07ec166c92d1a0d102ad507509c9472ca6114b4fd35c85e1f046aa50116c"
    );
    assert_eq!(first, hash("opencode", "ses_abc"));
    assert_ne!(first, hash("opencode", "ses_abd"));
    assert_ne!(first, hash("cline", "ses_abc"));
    assert_eq!(first.len(), 32);
}

#[test]
fn debug_output_omits_native_ids() {
    let source = identity("opencode", "session_native_secret");
    let canonical = canonical_source_session(&source).unwrap();
    assert!(!format!("{source:?}").contains("session_native_secret"));
    assert!(!format!("{canonical:?}").contains("session_native_secret"));
    let untrusted_adapter = identity("adapter_private_secret", "id");
    assert!(!format!("{untrusted_adapter:?}").contains("adapter_private_secret"));
}
