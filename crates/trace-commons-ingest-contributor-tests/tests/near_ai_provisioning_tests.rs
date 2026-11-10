// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! NEAR AI provisioning, the server against the contributor's own client
//! code. Moved out of the ingest crate's
//! `tests/near_ai_provisioning_tests.rs`, where the rest of that suite
//! stays, so the ingest crate does not depend on the contributor to build
//! its tests. The module names are kept so the test's path changes only in
//! its crate.

mod open_sign_up {
    use axum::http::StatusCode;
    use trace_commons_ingest::test_support::near_ai_provisioning_fixture::*;

    /// A staging commons (no admission ledger, no witness) publishes the
    /// issuer, audience and receipt endpoint on the v2 capabilities route, and
    /// the contributor's own v2 validator accepts that document and signs up
    /// on the routes it names (#1574 review, finding H1). A default build,
    /// with its ledger, answers the same.
    ///
    /// The client half is the contributor's: its capability validator, its
    /// route constants and its start body. The device signature is the shared
    /// preimage function signed by a fixture key, not `DeviceIdentity`, whose
    /// store would reach the OS keychain from this crate.
    #[tokio::test]
    async fn capabilities_v2_publishes_the_issuer_with_no_witness_and_the_client_signs_up_on_it() {
        use trace_commons_contributor::daemon::account_onboarding as client;
        let Some(h) = harness_with_admission(!cfg!(feature = "near-ai-staging")).await else {
            return;
        };
        let _witness = EnvVar::set("TRACE_COMMONS_NEAR_PROVISIONING_WITNESS_JSON", None);
        let _receipt = EnvVar::set(
            "TRACE_COMMONS_NEAR_PROVISIONING_RECEIPT_ENDPOINT",
            Some("https://receipts.example"),
        );
        let admin = h.admin().await;

        // The old route, unchanged: with no witness it publishes nothing.
        let legacy = send(&h.state, "GET", LEGACY_CAPABILITIES, None, None).await;
        assert_eq!(legacy.status, StatusCode::OK);
        assert_eq!(
            legacy.bytes,
            serde_json::to_vec(&serde_json::json!({
                "ready": false, "near_ai_login_ready": false, "funding_available": false,
            }))
            .unwrap(),
            "{:?}",
            legacy.json()
        );

        let reply = send(
            &h.state,
            "GET",
            client::NEAR_AI_CAPABILITIES_V2_PATH,
            None,
            None,
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK);
        let v2 = reply.json();
        assert_eq!(v2["near_ai_login_ready"], true, "{v2:?}");
        assert_eq!(v2["issuer_url"], "https://issuer.example");
        assert_eq!(v2["audience"], "upload");
        assert_eq!(v2["inference_receipt_endpoint"], "https://receipts.example");
        assert!(v2.get("witness").is_none(), "v2 publishes no witness");

        let capability =
            client::validate_near_ai_capability_v2("https://commons.example", v2.clone())
                .expect("the client accepts what the server publishes");
        assert_eq!(capability.issuer_url, "https://issuer.example");
        assert_eq!(capability.audience, "upload");
        assert!(capability.witness.is_none());
        assert_eq!(
            capability.receipt_endpoint.as_deref(),
            Some("https://receipts.example")
        );

        // Sign up on the routes the client calls, with the client's start body.
        let subject = unseen_subject();
        h.sign_in_to_near_ai_as(&subject);
        let device = Device::new();
        let (verifier, challenge) = pkce();
        let reply = send(
            &h.state,
            "POST",
            client::NEAR_AI_START_V2_PATH,
            Some(
                &trace_commons_contributor::daemon::nearai_onboarding::start_payload(
                    &challenge,
                    &device.public_b64(),
                ),
            ),
            None,
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
        let started = reply.json();
        let nonce: [u8; 32] = {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(started["nonce"].as_str().expect("nonce"))
                .expect("standard base64")
                .try_into()
                .expect("32 bytes")
        };
        let ceremony_id = started["ceremony_id"]
            .as_str()
            .expect("ceremony")
            .to_string();
        let preimage = trace_commons_protocol::onboarding::near_ai_provisioning_device_bytes(
            &nonce,
            &ceremony_id,
            &device.public_bytes(),
            &challenge,
            started["expires_at"].as_i64().expect("expiry"),
        );
        let reply = send(
            &h.state,
            "POST",
            client::NEAR_AI_FINISH_V2_PATH,
            Some(&serde_json::json!({
                "ceremony_id": ceremony_id,
                "code_verifier": verifier,
                "device_public_key": device.public_b64(),
                "device_signature": device.sign(&preimage),
                "access_token": "near-ai-access-token-fixture",
            })),
            None,
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
        let finished = reply.json();
        let tenant = finished["tenant_id"].as_str().expect("tenant").to_string();
        assert!(tenant.starts_with("nearai-"));
        assert!(
            finished["access_token"]
                .as_str()
                .is_some_and(|t| t.starts_with("tcn1_"))
        );
        assert_eq!(
            anchors_for(&admin, &identity().login_index_label(&subject))
                .await
                .len(),
            1
        );
        drop_tenants(&admin, &[&tenant]).await;
    }
}
