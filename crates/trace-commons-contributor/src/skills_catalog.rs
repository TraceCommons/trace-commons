//! The keys this build trusts for the server's signed Skills catalog, one
//! file per environment (`skills-catalog/<environment>.json`). Separate from
//! the feature-flag profile, so pinning a catalog key does not change the
//! profile digest or drop cached flag decisions.
//!
//! The files ship with no key: until the owner pins one, every catalog is
//! refused and the daemon keeps fetching publishers from GitHub.

use serde::Deserialize;
use std::collections::BTreeSet;
use trace_commons_protocol::skills_catalog::{Environment, parse_public_key_hex};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinnedKeys {
    schema_version: u32,
    environment: Environment,
    signing_keys: BTreeSet<String>,
}

fn raw(environment: Environment) -> &'static str {
    match environment {
        Environment::Production => include_str!("../skills-catalog/production.json"),
        Environment::Staging => include_str!("../skills-catalog/staging.json"),
        Environment::Testing => include_str!("../skills-catalog/testing.json"),
    }
}

/// Raw Ed25519 keys pinned for `environment`. A malformed file is an error,
/// never an empty list that a caller could mistake for "no key yet".
pub fn pinned_keys(environment: Environment) -> anyhow::Result<Vec<[u8; 32]>> {
    let pinned: PinnedKeys = serde_json::from_str(raw(environment))?;
    anyhow::ensure!(
        pinned.schema_version == 1 && pinned.environment == environment,
        "skills-catalog-keys-identity"
    );
    pinned
        .signing_keys
        .iter()
        .map(|key| {
            parse_public_key_hex(key).ok_or_else(|| anyhow::anyhow!("skills-catalog-key-invalid"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_environment_file_parses_and_ships_without_a_key() {
        for environment in [
            Environment::Production,
            Environment::Staging,
            Environment::Testing,
        ] {
            assert!(pinned_keys(environment).unwrap().is_empty());
        }
    }
}
