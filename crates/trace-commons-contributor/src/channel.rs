//! The build's channel: every compiled-in NEAR AI host, Trace Commons host and
//! app identity value, with one value per channel.
//!
//! There are two channels. A default build is **production**. A build with
//! the `near-ai-staging` feature is **staging**: it signs in to NEAR AI
//! staging, contributes to the staging server and keeps its state, keychain
//! entries and passkeys apart from production's (design:
//! `docs/superpowers/specs/2026-10-21-staging-environment-design.md`,
//! sections 4.1 and 4.2). No release workflow or shipped manifest may enable
//! the feature: `scripts/ci/check-release-staging-feature.sh` refuses one that
//! does.
//!
//! Nothing outside this module may spell a host or identity value.
//! `tests/channel_hosts.rs` reads the source of the contributor, FFI and
//! platform crates and fails on any such literal elsewhere, so a value the
//! feature does not switch cannot be added by accident.
//!
//! Each build also holds the other channel's hosts, as SHA-256 digests only,
//! and refuses a service origin on one of them
//! ([`refuse_other_channel_origin`]). A production build therefore carries no
//! staging host name in plain text.

use sha2::{Digest, Sha256};
use trace_commons_contributor_platform::os_secret_store::KeychainIdentity;

/// The staging site. Every staging Trace Commons host is this name or a
/// subdomain of it, and it is the staging passkey relying party. The one
/// place the name is written.
#[cfg(feature = "near-ai-staging")]
macro_rules! staging_host {
    () => {
        "staging.tracecommons.ai"
    };
}

/// The staging site, as a value. See [`staging_host!`].
#[cfg(feature = "near-ai-staging")]
pub const STAGING_HOST: &str = staging_host!();

/// The channel's state directory override variable, as a literal so help
/// text can be built from it with `concat!`.
#[cfg(not(feature = "near-ai-staging"))]
macro_rules! state_dir_env {
    () => {
        "TRACE_COMMONS_CONTRIBUTOR_DIR"
    };
}
#[cfg(feature = "near-ai-staging")]
macro_rules! state_dir_env {
    () => {
        "TRACE_COMMONS_STAGING_CONTRIBUTOR_DIR"
    };
}

/// The app's URL scheme, as a literal so links can be built from it with
/// `concat!`.
#[cfg(not(feature = "near-ai-staging"))]
macro_rules! url_scheme {
    () => {
        "tracecommons"
    };
}
#[cfg(feature = "near-ai-staging")]
macro_rules! url_scheme {
    () => {
        "tracecommons-staging"
    };
}

/// Whether this build is the staging channel (`near-ai-staging`).
pub const fn near_ai_staging() -> bool {
    cfg!(feature = "near-ai-staging")
}

/// The channel's name, as a literal for `concat!`.
#[cfg(not(feature = "near-ai-staging"))]
macro_rules! channel_name {
    () => {
        "production"
    };
}
#[cfg(feature = "near-ai-staging")]
macro_rules! channel_name {
    () => {
        "staging"
    };
}

#[cfg(not(feature = "near-ai-staging"))]
mod values {
    pub const CHANNEL: &str = channel_name!();

    pub const STATE_DIR_NAME: &str = "trace-commons";
    pub const OTHER_STATE_DIR_NAME: &str = "trace-commons-staging";
    pub const STATE_DIR_ENV: &str = state_dir_env!();

    pub const KEYCHAIN_ACCESS_GROUP: &str = "KXSWJN7WY8.ai.tracecommons.shell";
    pub const KEYCHAIN_CLOUD_SERVICE: &str = "trace-commons.near-ai.credentials";
    pub const KEYCHAIN_ACCOUNT_SERVICE: &str = "trace-commons.account.credentials";
    pub const KEYCHAIN_INSIGHTS_SERVICE: &str = "trace-commons.insights.digest-key";
    pub const KEYCHAIN_CLAUDE_USAGE_SERVICE: &str = "trace-commons.claude-usage.token-cache";

    pub const NATIVE_ACCOUNT_ORIGIN: &str = "https://ingest.tracecommons.ai";
    pub const PASSKEY_RP_ID: &str = "tracecommons.ai";
    pub const PASSKEY_ORIGIN: &str = "https://tracecommons.ai";
    pub const URL_SCHEME: &str = url_scheme!();
    pub const PUBLIC_SITE_HOST: &str = "tracecommons.ai";
    pub const PUBLIC_RUNS_PREFIX: &str = "https://tracecommons.ai/runs/";
    pub const INVITE_PLACEHOLDER: &str = "https://issuer.tracecommons.ai/onboard#…";

    pub const NEAR_AI_API_ORIGIN: &str = "https://cloud-api.near.ai";
    pub const NEAR_AI_INFERENCE_BASE: &str = "https://cloud-api.near.ai/v1";
    pub const NEAR_AI_DASHBOARD_ORIGIN: &str = "https://cloud.near.ai";
    pub const NEAR_AI_WALLET_RECIPIENT: &str = "cloud.near.ai";
    /// The NEAR wallet first, then NEAR AI's GitHub and Google logins
    /// (#1380).
    pub const NEAR_AI_SIGN_IN_PROVIDERS: &[&str] = &["near", "github", "google"];

    pub const IRONWIRE_HOME_ENV: &str = "IRONWIRE_HOME";
    pub const IRONWIRE_HOME_DIR: &str = ".ironwire";
    pub const IRONWIRE_CONTROL_PORT: Option<u16> = None;
    pub const IRONWIRE_READS_POINTER: bool = true;
}

#[cfg(feature = "near-ai-staging")]
mod values {
    pub const CHANNEL: &str = channel_name!();

    pub const STATE_DIR_NAME: &str = "trace-commons-staging";
    pub const OTHER_STATE_DIR_NAME: &str = "trace-commons";
    pub const STATE_DIR_ENV: &str = state_dir_env!();

    pub const KEYCHAIN_ACCESS_GROUP: &str = "KXSWJN7WY8.ai.tracecommons.shell.staging";
    pub const KEYCHAIN_CLOUD_SERVICE: &str = "trace-commons-staging.near-ai.credentials";
    pub const KEYCHAIN_ACCOUNT_SERVICE: &str = "trace-commons-staging.account.credentials";
    pub const KEYCHAIN_INSIGHTS_SERVICE: &str = "trace-commons-staging.insights.digest-key";
    pub const KEYCHAIN_CLAUDE_USAGE_SERVICE: &str =
        "trace-commons-staging.claude-usage.token-cache";

    pub const NATIVE_ACCOUNT_ORIGIN: &str = concat!("https://ingest.", staging_host!());
    pub const PASSKEY_RP_ID: &str = staging_host!();
    pub const PASSKEY_ORIGIN: &str = concat!("https://", staging_host!());
    pub const URL_SCHEME: &str = url_scheme!();
    pub const PUBLIC_SITE_HOST: &str = staging_host!();
    pub const PUBLIC_RUNS_PREFIX: &str = concat!("https://", staging_host!(), "/runs/");
    pub const INVITE_PLACEHOLDER: &str = concat!("https://issuer.", staging_host!(), "/onboard#…");

    pub const NEAR_AI_API_ORIGIN: &str = "https://cloud-stg-api.near.ai";
    pub const NEAR_AI_INFERENCE_BASE: &str = "https://cloud-stg-api.near.ai/v1";
    pub const NEAR_AI_DASHBOARD_ORIGIN: &str = "https://cloud-stg.near.ai";
    /// NEAR AI staging's `NEAR_EXPECTED_RECIPIENT`, from its public
    /// deployment configuration (nearai/cvm-ansible-playbooks,
    /// `vars/cloud_api_stg.yaml`).
    pub const NEAR_AI_WALLET_RECIPIENT: &str = "cloud-stg.near.ai";
    /// NEAR AI's Google login only: "In the staging builds the sign in
    /// needs to be via Google and not your near wallet", and, asked about
    /// GitHub, "Google only for now" (owner, 2026-10-23). No NEAR wallet, so
    /// the staging ingest's refusal of wallet provisioning
    /// (`staging_wallet_provisioning_closed`) is never the first thing a
    /// tester meets.
    pub const NEAR_AI_SIGN_IN_PROVIDERS: &[&str] = &["google"];

    pub const IRONWIRE_HOME_ENV: &str = "IRONWIRE_STAGING_HOME";
    pub const IRONWIRE_HOME_DIR: &str = ".ironwire-staging";
    /// Production's IronWire answers on IronWire's default, 8463. Staging
    /// takes the next port so both proxies can run at once.
    pub const IRONWIRE_CONTROL_PORT: Option<u16> = Some(8464);
    pub const IRONWIRE_READS_POINTER: bool = false;
}

/// `production` or `staging`.
pub const CHANNEL: &str = values::CHANNEL;

/// The state directory's name under the platform's per-user base.
pub const STATE_DIR_NAME: &str = values::STATE_DIR_NAME;
/// The other channel's state directory name. An override variable that
/// resolves to the other channel's default directory is refused
/// (`crate::config::refuse_other_channel_state_dir`).
pub const OTHER_STATE_DIR_NAME: &str = values::OTHER_STATE_DIR_NAME;
/// The environment variable that overrides the state directory. A staging
/// build reads only its own, so a production override never points it at
/// production's enrollment.
pub const STATE_DIR_ENV: &str = values::STATE_DIR_ENV;
/// The CLI's `--config-dir` help, which names [`STATE_DIR_ENV`].
pub const CONFIG_DIR_HELP: &str = concat!(
    "Override the config directory (default: $",
    state_dir_env!(),
    ", then OS config dir)"
);

/// The keychain team access group the channel's entitlement grants.
pub const KEYCHAIN_ACCESS_GROUP: &str = values::KEYCHAIN_ACCESS_GROUP;
/// The keychain namespace every OS secret store of this build uses.
pub const KEYCHAIN: KeychainIdentity = KeychainIdentity {
    access_group: values::KEYCHAIN_ACCESS_GROUP,
    cloud_service: values::KEYCHAIN_CLOUD_SERVICE,
    account_service: values::KEYCHAIN_ACCOUNT_SERVICE,
    insights_digest_key_service: values::KEYCHAIN_INSIGHTS_SERVICE,
    claude_usage_service: values::KEYCHAIN_CLAUDE_USAGE_SERVICE,
};

/// The native app's account API: the one origin trusted before any config
/// exists. An IPC URL or a retained credential cannot establish another.
pub const NATIVE_ACCOUNT_ORIGIN: &str = values::NATIVE_ACCOUNT_ORIGIN;
/// The passkey relying party, which the signed app's associated domain names.
pub const PASSKEY_RP_ID: &str = values::PASSKEY_RP_ID;
/// The origin a passkey ceremony's client data must carry.
pub const PASSKEY_ORIGIN: &str = values::PASSKEY_ORIGIN;
/// The app's URL scheme (the bundle registers it; spec 4.5). The deep-link
/// parser accepts only this one, so a staging build never opens a
/// production link or the reverse.
pub const URL_SCHEME: &str = values::URL_SCHEME;
/// What every link in [`URL_SCHEME`] starts with.
pub const DEEP_LINK_PREFIX: &str = concat!(url_scheme!(), "://");
/// The link that opens the review queue.
pub const REVIEW_DEEP_LINK: &str = concat!(url_scheme!(), "://review");
/// The public site's host, where public run pages live.
pub const PUBLIC_SITE_HOST: &str = values::PUBLIC_SITE_HOST;
/// The prefix of a public run page.
pub const PUBLIC_RUNS_PREFIX: &str = values::PUBLIC_RUNS_PREFIX;
/// The example invite shown in an empty invite field.
pub const INVITE_PLACEHOLDER: &str = values::INVITE_PLACEHOLDER;

/// NEAR AI's API: sign-in, credentials, organisations.
pub const NEAR_AI_API_ORIGIN: &str = values::NEAR_AI_API_ORIGIN;
/// NEAR AI's OpenAI-compatible base: Private AI inference, the model list,
/// receipts and skill evaluation calls.
pub const NEAR_AI_INFERENCE_BASE: &str = values::NEAR_AI_INFERENCE_BASE;
/// The only NEAR AI host that publishes the per-model attestation registry
/// (a per-model completions host answers with its own single enclave and no
/// registry). Also the base IronWire is given, so the key sent here is the
/// key IronWire already sends here.
pub const NEAR_AI_ATTESTATION_GATEWAY: &str = values::NEAR_AI_INFERENCE_BASE;
/// The client-side NEAR AI privacy filter's default base.
pub const NEAR_AI_PRIVACY_FILTER_BASE: &str = values::NEAR_AI_INFERENCE_BASE;
/// NEAR AI's console: the billing dashboard.
pub const NEAR_AI_DASHBOARD_ORIGIN: &str = values::NEAR_AI_DASHBOARD_ORIGIN;
/// The NEP-413 recipient NEAR AI expects in a NEAR wallet sign-in.
pub const NEAR_AI_WALLET_RECIPIENT: &str = values::NEAR_AI_WALLET_RECIPIENT;
/// The NEAR AI sign-in providers this build offers, in the order a picker
/// lists them: `near` is the NEAR wallet ceremony, `github` and `google`
/// NEAR AI's own login through that provider. The daemon refuses a
/// ceremony for any other name, so a provider left out here is not reachable
/// from any shell. The shells read the list from the core
/// (`tc_build_channel_json`) and never keep one of their own.
pub const NEAR_AI_SIGN_IN_PROVIDERS: &[&str] = values::NEAR_AI_SIGN_IN_PROVIDERS;
/// The provider a sign-in that names none starts: the first one offered.
/// Production's is the NEAR wallet; staging's is NEAR AI's Google login,
/// its only one.
pub const NEAR_AI_DEFAULT_SIGN_IN_PROVIDER: &str = values::NEAR_AI_SIGN_IN_PROVIDERS[0];

/// Where a staging build fetches its signed NEAR AI measurement pins (spec
/// 5.3): a static file on the staging site. The device never takes this from
/// configuration, from ingest or from IPC.
#[cfg(feature = "near-ai-staging")]
pub const STAGING_PIN_URL: &str =
    concat!("https://", staging_host!(), "/near-ai-pins/current.json");

/// The Ed25519 public key a staging pin set must be signed by (spec 5.4),
/// as its 32 raw bytes.
///
/// The private half is the Secret Manager secret
/// `tc-staging-nearai-pin-signing-key`, held off the staging host
/// (`docs/operator/staging-pin-signing.md`). This value is also listed in
/// `scripts/ci/staging-pin-public-keys.txt`, so a production release
/// carrying it is refused. Key id `f4325194de2624f3`.
#[cfg(feature = "near-ai-staging")]
pub const STAGING_PIN_PUBLIC_KEY: &[u8] = &[
    0x15, 0x3e, 0x60, 0xb6, 0x1f, 0x65, 0xbd, 0x5f, 0x28, 0x29, 0xd0, 0xf2, 0x37, 0x8a, 0x3c, 0xc1,
    0x70, 0x58, 0xa0, 0x6e, 0xaa, 0x86, 0x8e, 0xb1, 0x2e, 0xb3, 0x5b, 0xf1, 0x03, 0xe1, 0xe5, 0x86,
];

/// The environment variable that moves the IronWire home (spec 4.2, S10).
/// A staging build reads only its own, so a production `IRONWIRE_HOME` never
/// points it at production's proxy, ledger or token.
pub const IRONWIRE_HOME_ENV: &str = values::IRONWIRE_HOME_ENV;
/// The IronWire home's directory name under the user's home, when
/// [`IRONWIRE_HOME_ENV`] is unset.
pub const IRONWIRE_HOME_DIR: &str = values::IRONWIRE_HOME_DIR;
/// The port this build's embedded IronWire binds. `None` is IronWire's own
/// configuration (production, as before); staging fixes one apart from
/// production's so the two proxies can run side by side.
pub const IRONWIRE_CONTROL_PORT: Option<u16> = values::IRONWIRE_CONTROL_PORT;
/// Whether this build reads the `ironwire` CLI's discovery pointer,
/// `~/.ironwire/endpoint.json`. That file belongs to production's IronWire
/// whatever `IRONWIRE_HOME` says, so a staging build never reads it and
/// accepts only a control URL and token under its own home.
pub const IRONWIRE_READS_POINTER: bool = values::IRONWIRE_READS_POINTER;

/// What `--version` adds after the build identity: the channel, so the
/// macOS bundle scripts can check the bundled CLI against the bundle's
/// Info.plist before signing (spec 4.3).
pub const VERSION_CHANNEL_SUFFIX: &str = concat!(" channel ", channel_name!());

/// Every host-bearing value of this build, for the ratchet test.
pub const HOST_VALUES: &[&str] = &[
    NATIVE_ACCOUNT_ORIGIN,
    PASSKEY_RP_ID,
    PASSKEY_ORIGIN,
    PUBLIC_SITE_HOST,
    PUBLIC_RUNS_PREFIX,
    INVITE_PLACEHOLDER,
    NEAR_AI_API_ORIGIN,
    NEAR_AI_INFERENCE_BASE,
    NEAR_AI_ATTESTATION_GATEWAY,
    NEAR_AI_PRIVACY_FILTER_BASE,
    NEAR_AI_DASHBOARD_ORIGIN,
    NEAR_AI_WALLET_RECIPIENT,
    #[cfg(feature = "near-ai-staging")]
    STAGING_PIN_URL,
];

/// A host the other channel owns, held as a digest so a build never carries
/// the other channel's name in plain text.
#[derive(Debug, Clone, Copy)]
pub struct OtherChannelHost {
    /// Lower-case hex SHA-256 of the host name.
    pub sha256: &'static str,
    /// Whether every subdomain is the other channel's too.
    pub subdomains: bool,
}

/// SHA-256 of each staging host: the staging site (and so every subdomain of
/// it), the NEAR AI staging API and the NEAR AI staging console. In both
/// builds: digests only.
pub const STAGING_HOST_DIGESTS: &[&str] = &[
    // The staging site.
    "766d0f9835a64339a1d20708e3257355304aa44509b2b49a65d88980da78d147",
    // The NEAR AI staging API.
    "061787df4438033eeb2502f81dff38864a5242f386169b1f1785788e13c058af",
    // The NEAR AI staging console.
    "0b4df4a714bdf1ffdb0aa062949542e007b0ffb3c93fb2855c0a0361fcf43854",
];

/// The other channel's hosts this build refuses as a service origin.
#[cfg(not(feature = "near-ai-staging"))]
pub const OTHER_CHANNEL_HOSTS: &[OtherChannelHost] = &[
    OtherChannelHost {
        sha256: STAGING_HOST_DIGESTS[0],
        subdomains: true,
    },
    OtherChannelHost {
        sha256: STAGING_HOST_DIGESTS[1],
        subdomains: false,
    },
    OtherChannelHost {
        sha256: STAGING_HOST_DIGESTS[2],
        subdomains: false,
    },
];

/// The other channel's hosts this build refuses as a service origin: the
/// production site, its ingest and issuer, the NEAR AI API and console. Exact
/// names: the production apex is the staging site's parent, so a subdomain
/// match would refuse staging itself.
#[cfg(feature = "near-ai-staging")]
pub const OTHER_CHANNEL_HOSTS: &[OtherChannelHost] = &[
    // The production site.
    OtherChannelHost {
        sha256: "634ff87a58a1416e8fca1f6c44ffd382c5911cd21353adbad35060ea0e44fce0",
        subdomains: false,
    },
    // Its www name.
    OtherChannelHost {
        sha256: "cfed99aae198e169c17d08e5e81a4c587514af8a2e5ff5e8bfe833c1b65385e9",
        subdomains: false,
    },
    // The production ingest.
    OtherChannelHost {
        sha256: "adcb447bf411178d52fb45549d31677eef5f46c0e191cc72113a34b2d5bab597",
        subdomains: false,
    },
    // The production issuer.
    OtherChannelHost {
        sha256: "da0e9ce4b438f75e3cc3fa69630e81e9afe0e756395af951efaa03032c66589f",
        subdomains: false,
    },
    // The NEAR AI API.
    OtherChannelHost {
        sha256: "066ae42a7eb4f0bae28df21290e6f3f29c2e2aa7d41da8c88927c62cf111ffcb",
        subdomains: false,
    },
    // The NEAR AI console.
    OtherChannelHost {
        sha256: "88cc08786749137edf854ed0ea1b8a5faf48bb1882f76445b750ca2942d24926",
        subdomains: false,
    },
];

/// The fixed refusal for a service origin on the other channel's host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("service-origin-other-channel")]
pub struct OtherChannelOrigin;

/// Refuse `url` when its host belongs to the other channel. Anything that
/// does not parse as a URL with a host is left to the caller's own
/// validation; this check only ever refuses.
pub fn refuse_other_channel_origin(url: &str) -> Result<(), OtherChannelOrigin> {
    let Ok(parsed) = url::Url::parse(url) else {
        return Ok(());
    };
    let Some(host) = parsed.host_str() else {
        return Ok(());
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let digest = |name: &str| hex::encode(Sha256::digest(name.as_bytes()));
    let exact = digest(&host);
    // The host and each parent domain, for entries that cover subdomains.
    let mut parents = Vec::new();
    let mut rest = host.as_str();
    while let Some((_, parent)) = rest.split_once('.') {
        parents.push(digest(parent));
        rest = parent;
    }
    for entry in OTHER_CHANNEL_HOSTS {
        if entry.sha256 == exact || (entry.subdomains && parents.iter().any(|p| p == entry.sha256))
        {
            return Err(OtherChannelOrigin);
        }
    }
    Ok(())
}
