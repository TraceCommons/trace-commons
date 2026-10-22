// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Staging admission by allow-list (staging spec S5, section 3.12).
//!
//! Compiled only under `near-ai-staging`. A staging NEAR AI account alone
//! admits nobody, since anyone can make one, so a staging ingest admits a
//! NEAR AI login that has no account only when its verified subject is on a
//! list the operator keeps. A login that already has an account signs in as
//! before; an invite still elevates an admitted account.
//!
//! The list is a file named by [`ALLOWED_SUBJECTS_PATH_ENV`]: one subject per
//! line, compared exactly (no case folding: subjects are opaque identifiers),
//! with surrounding whitespace, blank lines and `#` comment lines ignored. It
//! is read on every sign-in that reaches this check, so an edit takes effect
//! without a restart. An unset setting, a missing, unreadable or oversized
//! file, and an empty list all admit nobody.
//!
//! The subject never reaches a log: refusals and admissions are logged by the
//! anchor's blind index only.

use trace_commons_server::account_onboarding::NearAiNewAccount;

/// The setting naming the allow-list file. Read only by a staging build.
pub(super) const ALLOWED_SUBJECTS_PATH_ENV: &str =
    "TRACE_COMMONS_STAGING_ALLOWED_NEAR_AI_SUBJECTS_PATH";

/// A list past this size is not an operator-kept list of testers; refuse it
/// rather than read it.
const MAX_LIST_BYTES: u64 = 1024 * 1024;

/// The answer for a verified subject: allowed, or not.
pub(super) async fn new_account_for(subject: &str) -> NearAiNewAccount {
    match read_list().await {
        Some(list) if is_listed(&list, subject) => NearAiNewAccount::AllowListed,
        _ => NearAiNewAccount::NotListed,
    }
}

/// The list's contents, or `None` for every way it can be absent.
async fn read_list() -> Option<String> {
    let path = std::env::var_os(ALLOWED_SUBJECTS_PATH_ENV)?;
    let metadata = tokio::fs::metadata(&path).await.ok()?;
    if !metadata.is_file() || metadata.len() > MAX_LIST_BYTES {
        return None;
    }
    tokio::fs::read_to_string(&path).await.ok()
}

fn is_listed(list: &str, subject: &str) -> bool {
    !subject.is_empty()
        && list
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .any(|line| line == subject)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subject_matches_only_its_own_line_exactly() {
        let list = "# testers\n\n  auth0|alice  \ngithub|42\n";
        assert!(is_listed(list, "auth0|alice"));
        assert!(is_listed(list, "github|42"));
        for other in ["auth0|ALICE", "auth0|alic", "auth0|alicex", "github|4", ""] {
            assert!(!is_listed(list, other), "{other:?}");
        }
    }

    #[test]
    fn comments_and_blank_lines_list_nobody() {
        let list = "# auth0|alice\n\n   \n";
        assert!(!is_listed(list, "auth0|alice"));
        assert!(!is_listed(list, "# auth0|alice"));
        assert!(!is_listed("", ""));
    }
}
