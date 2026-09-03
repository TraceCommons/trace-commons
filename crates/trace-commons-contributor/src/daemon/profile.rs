//! The public roster profile over the daemon socket: `set_public_profile`,
//! `clear_public_profile`, and `get_public_profile`.
//!
//! `set_public_profile` and `clear_public_profile` are the socket-facing
//! entry points onto `crate::submit::set_profile` and
//! `crate::submit::clear_profile` -- the same `PUT` and `DELETE` on
//! `/v1/community/profile` the CLI's `profile` command drives. Neither shell
//! hand-rolls a second copy of the network call, so a socket caller and a
//! terminal caller claim a handle identically.
//!
//! # Why the handle is cached locally
//!
//! There is no `GET /v1/community/profile`: the server derives the principal
//! from the authenticated request and offers no read-back of a contributor's
//! own row. A shell therefore has nowhere to read the handle it just
//! published from, and the settings panel that displays the profile would
//! render empty the moment the process restarted. So a successful `set`
//! writes the handle, bio, and roster date the *server* reported into
//! `ContributorConfig`, a successful `clear` removes them, and
//! `get_public_profile` reports that cache. It is a cache and says so: it
//! reflects what this device last did, not what the server holds now, and it
//! is not an authorization input anywhere.
//!
//! The write itself lives in `crate::submit`
//! (`cache_public_profile` / `clear_cached_public_profile`) rather than here,
//! because the CLI's `profile` command has to make the same write and two
//! copies of "what a claimed handle leaves behind on this machine" is how
//! the two shells drift into disagreeing about whether a contributor is on
//! the roster.
//!
//! A withdrawal drops one more thing: the roster standing the poller cached
//! in `daemon::state`. That is served by `history_rollup` on its own
//! freshness rule, so a withdrawal that only cleared the config would leave
//! clients being handed a rank and an accept rate until the poller next ran.
//!
//! # Going public is two calls, not one
//!
//! Adding `public_attribution` to the consent scopes (`set_consent_scopes`)
//! and claiming a handle (`set_public_profile`) are separate operations, and
//! the second is the one that actually puts a row on the roster. The server
//! authorizes the `PUT` against the grant ceiling on the claim, not against
//! the local scope list, so this module deliberately does not pre-check
//! `cfg.consent_scopes` -- the local set can be narrower than what the
//! credential carries, and refusing here would refuse contributors the server
//! would have allowed. The CLI's `profile` command makes the same choice for
//! the same reason.
//!
//! # What crosses this socket
//!
//! A handle and a bio are public by construction -- that is what claiming
//! them means -- so they are the one thing here that may appear in a
//! response. Nothing else does: an underlying failure is never echoed back
//! (it can carry a server response body or a URL) and no handle or bio is
//! ever written to a log line or an audit entry.

use serde_json::json;

use trace_commons_protocol::community_handle::{
    BioValidationError, HandleValidationError, validate_bio, validate_handle,
};

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use crate::config::ContributorConfig;

/// Fixed labels for the shared validation rules in
/// `trace_commons_protocol::community_handle`. The rules themselves are not
/// restated here: the client and the server must refuse the same strings,
/// and a second copy of "what a handle may look like" is exactly how those
/// two drift apart.
fn handle_error_label(err: HandleValidationError) -> &'static str {
    match err {
        HandleValidationError::TooShort => "handle-too-short",
        HandleValidationError::TooLong => "handle-too-long",
        HandleValidationError::InvalidCharacter => "handle-invalid-character",
        HandleValidationError::InvalidBoundary => "handle-invalid-boundary",
        HandleValidationError::ConsecutiveSeparators => "handle-consecutive-separators",
        HandleValidationError::Reserved => "handle-reserved",
    }
}

fn bio_error_label(err: BioValidationError) -> &'static str {
    match err {
        BioValidationError::TooLong => "bio-too-long",
        BioValidationError::InvalidCharacter => "bio-invalid-character",
    }
}

/// The profile as this daemon knows it, from the local cache. Shared by all
/// three methods so a client parses one shape whichever call it made.
///
/// `public_url` is always `null`. The daemon knows the ingest origin it
/// uploads to, not the origin the community website serves profiles from,
/// and inventing one would hand a shell a link that does not resolve. A
/// client that wants a "View public profile" link must get the origin from
/// somewhere else.
fn profile_value(cfg: &ContributorConfig) -> serde_json::Value {
    json!({
        "on_roster": cfg.display_handle.is_some(),
        "handle": cfg.display_handle,
        "bio": cfg.public_bio,
        "public_since": cfg.public_since,
        "public_url": serde_json::Value::Null,
    })
}

/// Parse `bio` out of the request. Absent is an error, not "leave it alone":
/// the server upserts with `bio = excluded.bio`, so the `PUT` replaces the
/// whole profile and there is no partial update to express. Requiring the
/// caller to say which it means is the difference between clearing a
/// published bio because the contributor asked and clearing it because they
/// renamed their handle. `null` publishes no bio; a string publishes that
/// bio. The CLI refuses the same ambiguity with `--bio` / `--no-bio`.
fn parse_bio(params: &serde_json::Value) -> Result<Option<String>, &'static str> {
    match params.get("bio") {
        None => Err("bio-required-or-null"),
        Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err("bio-invalid"),
    }
}

/// Claim or update this contributor's public handle, then cache what the
/// server reported.
///
/// Performs real network I/O, so it is reached only through
/// `handle_request_async` -- see the "Sync vs. async dispatch" note on
/// `ipc`.
pub(super) async fn handle_set_public_profile(shared: &DaemonShared, req: &Request) -> Response {
    let Some(handle) = req.params.get("handle").and_then(|v| v.as_str()) else {
        return Response::err(req.id, ERR_BAD_PARAMS, "handle-required");
    };
    let handle = match validate_handle(handle) {
        Ok(v) => v,
        Err(e) => return Response::err(req.id, ERR_BAD_PARAMS, handle_error_label(e)),
    };
    let bio = match parse_bio(&req.params) {
        Ok(b) => b,
        Err(m) => return Response::err(req.id, ERR_BAD_PARAMS, m),
    };
    if let Some(text) = bio.as_deref() {
        if let Err(e) = validate_bio(text) {
            return Response::err(req.id, ERR_BAD_PARAMS, bio_error_label(e));
        }
    }
    let Ok(Some(mut cfg)) = shared.store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, "not-logged-in");
    };

    // The validated display form, not the raw parameter: `validate_handle`
    // trims, and the server stores what it is sent.
    let profile = match crate::submit::set_profile(
        &shared.store,
        &cfg,
        &handle.display,
        bio.as_deref(),
    )
    .await
    {
        Ok(p) => p,
        // Never echo the underlying error: it can carry a server response
        // body or a URL.
        Err(_e) => return Response::err(req.id, ERR_UNAVAILABLE, "profile-update-failed"),
    };

    // A failed cache write is NOT reported as a failed call. The handle is
    // published either way -- the server already accepted it -- and
    // answering `unavailable` here would have a shell tell the contributor
    // their profile did not go up when it did. `handle_persisted` is how a
    // client learns the weaker thing that is actually true: that
    // `get_public_profile` will not report this profile until the next
    // successful `set`. The write itself is `submit`'s, shared with the
    // CLI's `profile` command so the two shells cache identically.
    let handle_persisted = crate::submit::cache_public_profile(&shared.store, &mut cfg, &profile);

    let mut result = profile_value(&cfg);
    result["handle_persisted"] = json!(handle_persisted);
    Response::ok(req.id, result)
}

/// Withdraw this contributor's public attribution and drop the local cache.
///
/// Performs real network I/O, so it is reached only through
/// `handle_request_async`.
pub(super) async fn handle_clear_public_profile(shared: &DaemonShared, req: &Request) -> Response {
    let Ok(Some(mut cfg)) = shared.store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, "not-logged-in");
    };
    if let Err(_e) = crate::submit::clear_profile(&shared.store, &cfg).await {
        return Response::err(req.id, ERR_UNAVAILABLE, "profile-withdraw-failed");
    }
    // The cached roster standing goes with the handle, in this same
    // operation. `history_rollup` serves `state.community` whenever it is
    // fresh, and `refresh_community` drops it only at its next due poll --
    // a full poll interval away, 15 minutes by default -- so leaving it here
    // means the daemon keeps handing clients a rank and an accept rate for a
    // contributor who has just withdrawn public attribution. Withdrawal is
    // precisely the request not to be shown, and it does not get to wait for
    // a timer.
    //
    // Done before the config write so that a withdrawal whose config write
    // fails still stops serving the standing immediately. A failed state
    // write is not surfaced: the in-memory copy is already cleared, so this
    // daemon serves nothing either way, and the file is rewritten on the
    // next poll.
    {
        let mut state = shared.state.lock().expect("state lock");
        if state.community.take().is_some() {
            let _ = state.save(&shared.store);
        }
    }
    // The reverse of `set`'s reasoning: the row is gone from the server
    // regardless, and a cache that still names a withdrawn handle is worse
    // than one that is merely stale, so the caller is told when the clear
    // did not stick locally rather than being shown a success it would then
    // contradict on the next `get`.
    let handle_persisted = crate::submit::clear_cached_public_profile(&shared.store, &mut cfg);

    let mut result = profile_value(&cfg);
    result["withdrawn"] = json!(true);
    result["handle_persisted"] = json!(handle_persisted);
    Response::ok(req.id, result)
}

/// Report the locally cached public profile. No network I/O: see the module
/// doc for why there is nothing to read back from the server.
pub(super) fn handle_get_public_profile(shared: &DaemonShared, req: &Request) -> Response {
    let Ok(Some(cfg)) = shared.store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, "not-logged-in");
    };
    Response::ok(req.id, profile_value(&cfg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn shared() -> DaemonShared {
        let (_d, store) = crate::config::tests_support::temp_store();
        std::mem::forget(_d);
        DaemonShared::load(store).unwrap()
    }

    fn req(method: &str, params: serde_json::Value) -> Request {
        Request {
            id: 1,
            method: method.to_string(),
            params,
        }
    }

    fn enrolled_config() -> ContributorConfig {
        ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "http://issuer.invalid".into(),
            ingest_url: "http://ingest.invalid".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: "device-1".into(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
            display_handle: None,
            public_bio: None,
            public_since: None,
            witness: None,
        }
    }

    #[tokio::test]
    async fn set_without_a_handle_is_a_param_error() {
        let s = shared();
        let r = handle_set_public_profile(&s, &req("set_public_profile", json!({}))).await;
        let err = r.error.unwrap();
        assert_eq!(err.code, ERR_BAD_PARAMS);
        assert_eq!(err.message, "handle-required");
    }

    #[tokio::test]
    async fn set_rejects_a_reserved_handle_with_the_shared_rule() {
        // The refusal comes from `community_handle`, not from a second copy
        // of the rules living in this module: a client that passes here and
        // is refused by the server would be the failure worth catching.
        let s = shared();
        let r = handle_set_public_profile(
            &s,
            &req(
                "set_public_profile",
                json!({"handle": "admin", "bio": null}),
            ),
        )
        .await;
        let err = r.error.unwrap();
        assert_eq!(err.code, ERR_BAD_PARAMS);
        assert_eq!(err.message, "handle-reserved");
    }

    #[tokio::test]
    async fn set_refuses_an_ambiguous_bio_rather_than_guessing() {
        // The PUT replaces the whole profile, so "leave the bio alone" is not
        // a thing the server can be asked for. Omitting `bio` must be an
        // error, not an accidental erasure.
        let s = shared();
        let r =
            handle_set_public_profile(&s, &req("set_public_profile", json!({"handle": "manian"})))
                .await;
        let err = r.error.unwrap();
        assert_eq!(err.code, ERR_BAD_PARAMS);
        assert_eq!(err.message, "bio-required-or-null");
    }

    #[tokio::test]
    async fn set_rejects_an_over_long_bio() {
        let s = shared();
        let long = "a".repeat(trace_commons_protocol::community_handle::BIO_MAX_BYTES + 1);
        let r = handle_set_public_profile(
            &s,
            &req(
                "set_public_profile",
                json!({"handle": "manian", "bio": long}),
            ),
        )
        .await;
        let err = r.error.unwrap();
        assert_eq!(err.code, ERR_BAD_PARAMS);
        assert_eq!(err.message, "bio-too-long");
    }

    #[tokio::test]
    async fn set_without_an_enrollment_says_so_before_any_network_call() {
        let s = shared();
        let r = handle_set_public_profile(
            &s,
            &req(
                "set_public_profile",
                json!({"handle": "manian", "bio": null}),
            ),
        )
        .await;
        let err = r.error.unwrap();
        assert_eq!(err.code, ERR_UNAVAILABLE);
        assert_eq!(err.message, "not-logged-in");
    }

    #[tokio::test]
    async fn clear_without_an_enrollment_says_so() {
        let s = shared();
        let r = handle_clear_public_profile(&s, &req("clear_public_profile", json!({}))).await;
        let err = r.error.unwrap();
        assert_eq!(err.code, ERR_UNAVAILABLE);
        assert_eq!(err.message, "not-logged-in");
    }

    #[tokio::test]
    async fn withdrawing_drops_the_cached_standing_in_the_same_operation() {
        // `history_rollup` serves `state.community` whenever it is fresh, and
        // the poller drops it only at its next due poll. Without this, a
        // contributor who withdrew public attribution kept being handed their
        // own rank and accept rate for up to a full poll interval afterwards.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = axum::Router::new()
            .route(
                "/v1/trace-upload-claim",
                axum::routing::post(|| async {
                    axum::Json(json!({
                        "access_token": "stub-claim-jwt",
                        "token_type": "Bearer",
                        "expires_at": Utc::now() + chrono::Duration::seconds(300),
                        "expires_in": 300,
                        "consent_scopes": ["debugging_evaluation"],
                        "allowed_uses": ["debugging"],
                    }))
                }),
            )
            .route(
                "/v1/community/profile",
                axum::routing::delete(|| async { axum::http::StatusCode::NO_CONTENT }),
            );
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let base = format!("http://{addr}");

        let s = shared();
        let mut cfg = enrolled_config();
        cfg.issuer_url = base.clone();
        cfg.ingest_url = base;
        cfg.allowed_hosts = Some("127.0.0.1".into());
        cfg.display_handle = Some("quiet-otter".into());
        cfg.public_since = Some(Utc::now());
        s.store.save_config(&cfg).unwrap();
        {
            let mut state = s.state.lock().expect("state lock");
            state.community = Some(crate::daemon::community::CommunityStanding {
                rank: Some(14),
                novelty_credit: 1240.0,
                accepted_in_window: 12,
                accept_rate: Some(0.75),
                window_label: "7d".into(),
                public_since: None,
                // Fresh, so `history_rollup` would serve it.
                snapshot_at: Some(Utc::now()),
                analytics_withheld: true,
            });
        }

        let r = handle_clear_public_profile(&s, &req("clear_public_profile", json!({}))).await;
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(r.result.unwrap()["withdrawn"], true);

        assert!(
            s.state.lock().expect("state lock").community.is_none(),
            "the cached standing must go with the handle, not wait for the poller"
        );
        let saved = s.store.load_config().unwrap().unwrap();
        assert!(saved.display_handle.is_none());
        // The standing is gone from the file too, not just from memory.
        let reloaded = crate::daemon::state::DaemonState::load(&s.store).unwrap();
        assert!(reloaded.community.is_none());
    }

    #[test]
    fn get_reports_not_on_the_roster_until_a_handle_is_claimed() {
        let s = shared();
        s.store.save_config(&enrolled_config()).unwrap();
        let r = handle_get_public_profile(&s, &req("get_public_profile", json!({})));
        let result = r.result.unwrap();
        assert_eq!(result["on_roster"], false);
        assert!(result["handle"].is_null());
        assert!(result["public_url"].is_null());
    }

    #[test]
    fn get_reports_the_cached_profile_a_successful_set_wrote() {
        // The read-back a settings panel renders. There is no
        // `GET /v1/community/profile`, so if this cache is not written the
        // panel is empty for a contributor who is in fact on the roster.
        let s = shared();
        let mut cfg = enrolled_config();
        cfg.display_handle = Some("manian".into());
        cfg.public_bio = Some("Ships billing systems by day.".into());
        cfg.public_since = Some(Utc::now());
        s.store.save_config(&cfg).unwrap();

        let r = handle_get_public_profile(&s, &req("get_public_profile", json!({})));
        let result = r.result.unwrap();
        assert_eq!(result["on_roster"], true);
        assert_eq!(result["handle"], "manian");
        assert_eq!(result["bio"], "Ships billing systems by day.");
        assert!(!result["public_since"].is_null());
    }

    #[test]
    fn get_without_an_enrollment_says_so() {
        let s = shared();
        let r = handle_get_public_profile(&s, &req("get_public_profile", json!({})));
        let err = r.error.unwrap();
        assert_eq!(err.code, ERR_UNAVAILABLE);
        assert_eq!(err.message, "not-logged-in");
    }

    #[test]
    fn a_config_written_before_these_fields_existed_still_loads() {
        // `#[serde(default)]` earning its place: serde requires an `Option`
        // field to be present unless it is defaulted, so without it every
        // contributor who enrolled before this revision would be unable to
        // load their own config -- a logout-and-re-enrol, not a missing
        // handle.
        let s = shared();
        let older = serde_json::json!({
            "schema_version": crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,
            "issuer_url": "http://issuer.invalid",
            "ingest_url": "http://ingest.invalid",
            "audience": "trace-commons-upload",
            "tenant_id": "tenant-abc",
            "instance_id": "instance-1",
            "user_subject": "alice",
            "device_key_id": "device-1",
            "consent_scopes": ["debugging_evaluation"],
            "pii_filter": null,
            "allowed_hosts": null,
        });
        std::fs::write(
            s.store.dir().join("contributor.json"),
            serde_json::to_vec(&older).unwrap(),
        )
        .unwrap();
        let loaded = s.store.load_config().unwrap().expect("config loads");
        assert!(loaded.display_handle.is_none());
        assert!(loaded.public_bio.is_none());
        assert!(loaded.public_since.is_none());
    }
}
