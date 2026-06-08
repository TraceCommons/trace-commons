# Onboarding Wire Types Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Trace Commons onboarding request/response/error wire types to `trace-commons-protocol` so IronClaw and the server share one contract.

**Architecture:** Add a focused `onboarding` protocol module with schema constants, serde structs, a typed error enum, and a shared `device_key_id` derivation helper. Export it from the protocol crate root. Keep server endpoint, DB registry, and issuer auth-branch work out of this slice.

**Tech Stack:** Rust, serde, sha2, hex, cargo tests.

---

### Task 1: Add Wire-Type Tests

**Files:**
- Create: `crates/trace-commons-protocol/src/onboarding.rs`
- Modify: `crates/trace-commons-protocol/src/lib.rs`

- [x] **Step 1: Write failing tests in `onboarding.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboard_request_round_trips() {
        let request = TraceOnboardRequest {
            schema_version: TRACE_ONBOARD_REQUEST_SCHEMA_VERSION.to_string(),
            invite_code: "INV9K3RT5FBQ72JX".to_string(),
            device_public_key: "cHVibGljLWtleS1ieXRlcw==".to_string(),
            client_info: TraceOnboardClientInfo {
                agent: "ironclaw".to_string(),
                version: "0.x.y".to_string(),
            },
        };
        let encoded = serde_json::to_string(&request).expect("serialize request");
        let decoded: TraceOnboardRequest =
            serde_json::from_str(&encoded).expect("deserialize request");
        assert_eq!(decoded, request);
    }

    #[test]
    fn onboard_response_round_trips_with_optional_community_urls() {
        let response = TraceOnboardResponse {
            schema_version: TRACE_ONBOARD_RESPONSE_SCHEMA_VERSION.to_string(),
            tenant_id: "tenant-zaki-pilot".to_string(),
            ingest_url: "https://ingest.tracecommons.ai".to_string(),
            issuer_url: "https://issuer.tracecommons.ai".to_string(),
            audience: "trace-commons-ingest".to_string(),
            device_key_id: "sha256:ad745f4e0af66a2c7ba9e95cf8ea65addb47d86ed989854c6f84f62fc177bd83".to_string(),
            contributor_label: Some("closed-alpha-batch-1".to_string()),
            community_url: Some("https://tracecommons.ai".to_string()),
            profile_url: Some("https://tracecommons.ai/profile".to_string()),
            leaderboard_url: Some("https://tracecommons.ai/leaderboard".to_string()),
        };
        let encoded = serde_json::to_string(&response).expect("serialize response");
        assert!(encoded.contains("profile_url"));
        let decoded: TraceOnboardResponse =
            serde_json::from_str(&encoded).expect("deserialize response");
        assert_eq!(decoded, response);
    }

    #[test]
    fn onboard_error_codes_use_exact_wire_names() {
        assert_eq!(
            serde_json::to_string(&TraceOnboardErrorCode::InviteNotValid).unwrap(),
            "\"InviteNotValid\""
        );
        assert_eq!(
            serde_json::from_str::<TraceOnboardErrorCode>("\"OnboardRateLimited\"").unwrap(),
            TraceOnboardErrorCode::OnboardRateLimited
        );
    }

    #[test]
    fn device_key_id_hashes_raw_public_key_bytes() {
        let id = device_key_id_from_public_key_bytes(b"public-key-bytes");
        assert_eq!(
            id,
            "sha256:ad745f4e0af66a2c7ba9e95cf8ea65addb47d86ed989854c6f84f62fc177bd83"
        );
    }
}
```

- [x] **Step 2: Run tests to verify they fail**

Run: `cargo test -p trace-commons-protocol onboarding`

Expected: compile failure because `TraceOnboardRequest`, `TraceOnboardResponse`, `TraceOnboardErrorCode`, constants, and helper are not defined.

### Task 2: Add Minimal Implementation

**Files:**
- Modify: `crates/trace-commons-protocol/src/onboarding.rs`
- Modify: `crates/trace-commons-protocol/src/lib.rs`

- [x] **Step 1: Implement the module**

Add serde structs, constants, exact error-code wire names, and the shared helper.

- [x] **Step 2: Export the module**

Add `pub mod onboarding;` to `crates/trace-commons-protocol/src/lib.rs`.

- [x] **Step 3: Verify focused tests pass**

Run: `cargo test -p trace-commons-protocol onboarding`

Expected: all onboarding tests pass.

- [x] **Step 4: Verify warning-clean protocol check**

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-protocol`

Expected: check passes.
