// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The one NEAR AI API base this server talks to.
//!
//! Login introspection (`GET /users/me`) and the NEAR AI payout rail both
//! read [`NEAR_AI_API_BASE_URL`], so the host that verified an org is the
//! host that pays it (payout spec section 2, step 6). It is a compiled-in
//! constant with no environment override: there is nothing configurable for
//! a host allowlist to protect, and making it configurable would create the
//! very thing the allowlist exists for.
//!
//! The `near-ai-staging` build feature switches it to the NEAR AI staging
//! API, for introspection and payout together. [`near_ai_staging`] says
//! whether this build carries the feature, so readiness can report it. No
//! release workflow or shipped manifest may enable the feature:
//! `scripts/ci/check-release-staging-feature.sh` refuses one that does.
//! There is no `compile_error!` on `--release`, because the staging run
//! needs an optimized build with the feature on.

/// The production NEAR AI API base, including `/v1`, no trailing slash.
#[cfg(not(feature = "near-ai-staging"))]
pub const NEAR_AI_API_BASE_URL: &str = "https://cloud-api.near.ai/v1";

/// The NEAR AI staging API base, including `/v1`, no trailing slash. The
/// default in cloud-api's own scripts; payout spec section 12, open item 2,
/// still has to confirm it.
#[cfg(feature = "near-ai-staging")]
pub const NEAR_AI_API_BASE_URL: &str = "https://cloud-stg-api.near.ai/v1";

/// Whether this build talks to NEAR AI staging (`near-ai-staging`).
/// SHA-256 of the NEAR AI staging API host. A default build carries the
/// staging host as this digest only, never in plain text.
#[cfg(not(feature = "near-ai-staging"))]
const STAGING_API_HOST_SHA256: &str =
    "061787df4438033eeb2502f81dff38864a5242f386169b1f1785788e13c058af";

pub const fn near_ai_staging() -> bool {
    cfg!(feature = "near-ai-staging")
}

/// The setting name [`near_ai_host_coherence`] reports for the compiled-in
/// base itself.
const COMPILED_BASE_SETTING: &str = "NEAR_AI_API_BASE_URL";

/// Check that every NEAR AI host this server will call belongs to this
/// build's channel (staging design, section 3.9).
///
/// `settings` pairs each setting name with the URL it resolves to,
/// configured or defaulted; the caller leaves out a setting that names no
/// host it will call. The compiled-in [`NEAR_AI_API_BASE_URL`] is checked
/// too, so the function states the whole invariant.
///
/// - A `near-ai-staging` build refuses any host that is not the NEAR AI
///   staging API host, and any value that does not parse as a URL with a
///   host: what cannot be shown to be staging is not staging.
/// - A default build refuses a host that is the NEAR AI staging API host. It
///   carries that host only as a SHA-256 digest. A value that does not parse
///   is left to the setting's own validation.
///
/// On a refusal it returns the setting's name, never its value.
pub fn near_ai_host_coherence(settings: &[(&'static str, &str)]) -> Result<(), &'static str> {
    std::iter::once((COMPILED_BASE_SETTING, NEAR_AI_API_BASE_URL))
        .chain(settings.iter().copied())
        .try_for_each(|(setting, url)| {
            if host_is_this_channel(url) {
                Ok(())
            } else {
                Err(setting)
            }
        })
}

/// The URL's host, lower case, without a trailing dot.
fn normalized_host(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    Some(host.trim_end_matches('.').to_ascii_lowercase())
}

#[cfg(feature = "near-ai-staging")]
fn host_is_this_channel(url: &str) -> bool {
    match (normalized_host(url), normalized_host(NEAR_AI_API_BASE_URL)) {
        (Some(host), Some(staging)) => host == staging,
        _ => false,
    }
}

#[cfg(not(feature = "near-ai-staging"))]
fn host_is_this_channel(url: &str) -> bool {
    use sha2::{Digest, Sha256};
    let Some(host) = normalized_host(url) else {
        return true;
    };
    hex::encode(Sha256::digest(host.as_bytes())) != STAGING_API_HOST_SHA256
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_base_is_https_with_v1_and_no_trailing_slash() {
        let url = reqwest::Url::parse(NEAR_AI_API_BASE_URL).unwrap();
        assert_eq!(url.scheme(), "https");
        assert!(url.username().is_empty() && url.password().is_none());
        assert_eq!(url.path(), "/v1");
        assert!(!NEAR_AI_API_BASE_URL.ends_with('/'));
    }

    const PRODUCTION_API: &str = "https://cloud-api.near.ai/v1";
    const STAGING_API: &str = "https://cloud-stg-api.near.ai/v1";
    const SETTINGS: [&str; 3] = [
        "TRACE_COMMONS_NEAR_AI_BASE_URL",
        "TRACE_COMMONS_NEAR_AI_ATTESTATION_BASE_URL",
        "TRACE_NEAR_AI_PRIVACY_BASE_URL",
    ];

    fn all(url: &'static str) -> Vec<(&'static str, &'static str)> {
        SETTINGS.iter().map(|setting| (*setting, url)).collect()
    }

    #[cfg(feature = "near-ai-staging")]
    #[test]
    fn a_staging_ingest_refuses_a_production_near_ai_host() {
        assert_eq!(near_ai_host_coherence(&all(STAGING_API)), Ok(()));
        for (index, setting) in SETTINGS.iter().enumerate() {
            for production in [
                PRODUCTION_API,
                "https://qwen3-8-27b.completions.near.ai/v1",
                "https://scorer.invalid/v1",
            ] {
                let mut settings = all(STAGING_API);
                settings[index].1 = production;
                assert_eq!(
                    near_ai_host_coherence(&settings),
                    Err(*setting),
                    "{setting} on another host"
                );
            }
        }
        // Case and a trailing dot do not make a different host.
        assert_eq!(
            near_ai_host_coherence(&all("https://CLOUD-STG-API.near.ai./v1")),
            Ok(())
        );
        // An unparseable value is not a staging host.
        assert_eq!(
            near_ai_host_coherence(&[(SETTINGS[0], "not a url")]),
            Err(SETTINGS[0])
        );
    }

    #[cfg(not(feature = "near-ai-staging"))]
    #[test]
    fn a_default_ingest_refuses_a_staging_near_ai_host() {
        assert_eq!(near_ai_host_coherence(&all(PRODUCTION_API)), Ok(()));
        for (index, setting) in SETTINGS.iter().enumerate() {
            for staging in [STAGING_API, "https://Cloud-Stg-Api.near.ai./v1/"] {
                let mut settings = all(PRODUCTION_API);
                settings[index].1 = staging;
                assert_eq!(
                    near_ai_host_coherence(&settings),
                    Err(*setting),
                    "{setting} on the staging host"
                );
            }
        }
    }

    /// A default build holds the staging API host as a digest only.
    #[cfg(not(feature = "near-ai-staging"))]
    #[test]
    fn the_staging_host_digest_is_the_staging_api_host() {
        use sha2::{Digest, Sha256};
        assert_eq!(
            hex::encode(Sha256::digest(b"cloud-stg-api.near.ai")),
            STAGING_API_HOST_SHA256
        );
    }

    #[cfg(not(feature = "near-ai-staging"))]
    #[test]
    fn a_default_build_talks_to_production() {
        assert!(!near_ai_staging());
        assert_eq!(NEAR_AI_API_BASE_URL, "https://cloud-api.near.ai/v1");
    }

    #[cfg(feature = "near-ai-staging")]
    #[test]
    fn a_staging_build_says_so() {
        assert!(near_ai_staging());
        assert_eq!(NEAR_AI_API_BASE_URL, "https://cloud-stg-api.near.ai/v1");
    }
}
