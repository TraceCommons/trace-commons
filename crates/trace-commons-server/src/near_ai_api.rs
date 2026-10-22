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
pub const fn near_ai_staging() -> bool {
    cfg!(feature = "near-ai-staging")
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
