//! Contributor-side client for trace-commons-server: discovers local coding
//! agent transcripts, redacts them through the deterministic pipeline, and
//! submits TraceContributionEnvelopes under instance-vouched per-user
//! identities.

pub mod account_auth;
pub mod account_contribution;
pub(crate) mod antigravity;
pub mod attach_copy;
pub mod brand;
pub mod channel;
pub mod cli;
pub mod commands;
pub mod commons_credit;
pub mod compute;
pub mod config;
pub mod consent;
pub mod consent_copy;
pub mod contribution_missions;
pub mod credit_estimate_check;
pub mod credit_estimate_table;
pub mod daemon;
pub mod deep_link;
pub mod disclosure;
pub mod disclosure_copy;
pub mod envelope;
pub mod external_url;
pub mod first_run_copy;
pub mod flow1;
pub mod harness_state;
pub mod health_copy;
pub mod history_copy;
pub mod identity;
pub mod inference_connection;
// Insights lives in the `trace-commons-contributor-insights` crate, so client
// builds that edit the rest of this crate do not recompile it; re-exported at
// its old path.
pub mod insights {
    pub use trace_commons_contributor_insights::insights::*;

    /// The Insights service at its old path, with every entry point that can
    /// open a store when the caller names no directory bound to this build's
    /// channel: its default store under the channel's state directory name,
    /// and its keychain for the OS-held digest key. Every other item is the
    /// insights crate's own.
    pub mod service {
        pub use trace_commons_contributor_insights::insights::service::*;

        use super::{LocalInsight, LocalInsightStore};
        use trace_commons_contributor_insights::insights::service as inner;

        /// This build's Insights store and keychain.
        pub const STORE_CHANNEL: inner::StoreChannel = inner::StoreChannel {
            keychain: crate::channel::KEYCHAIN,
            state_dir_name: crate::channel::STATE_DIR_NAME,
        };

        /// This build's default Insights store.
        pub fn default_store_dir() -> anyhow::Result<std::path::PathBuf> {
            inner::default_store_dir(STORE_CHANNEL.state_dir_name)
        }

        /// [`inner::execute`] in this build's channel.
        pub fn execute(request: LocalInsightsRequest) -> anyhow::Result<LocalInsightsResponse> {
            inner::execute_for(request, STORE_CHANNEL)
        }

        /// [`inner::dispatch_json`] in this build's channel.
        pub fn dispatch_json(bytes: &[u8]) -> anyhow::Result<String> {
            inner::dispatch_json_for(bytes, STORE_CHANNEL)
        }

        /// [`inner::open_store`] in this build's channel.
        pub fn open_store(
            store_dir: Option<&std::path::Path>,
        ) -> anyhow::Result<LocalInsightStore> {
            inner::open_store_for(store_dir, STORE_CHANNEL)
        }

        /// [`inner::list_saved`] in this build's channel.
        pub fn list_saved(
            store_dir: Option<&std::path::Path>,
        ) -> anyhow::Result<Vec<LocalInsight>> {
            inner::list_saved_for(store_dir, STORE_CHANNEL)
        }
    }
}
pub mod install_guides;
pub mod issuer_client;
pub mod managed;
pub mod mission_attempt;
pub mod mission_catalog;
pub mod mission_draft;
pub mod mission_draft_service;
pub(crate) mod near_ai_login;
pub mod nudge_copy;
pub mod nudge_render;
pub mod onboarding_copy;
pub mod outcome_copy;
pub mod payout_link;
pub mod picker;
pub mod preview_copy;
pub mod pricing;
pub mod privacy_scan_copy;
pub mod private_inference_copy;
pub mod project_copy;
pub mod quit_copy;
pub mod redaction_labels;
pub mod redaction_summary;
pub mod routing;
pub mod routing_copy;
pub mod shell_words_copy;
pub mod skill_loop;
pub mod source;
pub mod source_copy;
pub mod submit;
#[cfg(feature = "subscription-usage")]
pub mod subscription_usage;
pub mod token_bundle;
pub mod token_capture_client;
pub mod token_review_lease;
pub mod update;
pub mod watch_events;
pub mod withdraw;
pub mod witness;
pub mod witness_copy;

pub mod evidence_import;
pub mod public_run;

pub use trace_commons_protocol::trace_contribution::secret_leak_pattern_names;
