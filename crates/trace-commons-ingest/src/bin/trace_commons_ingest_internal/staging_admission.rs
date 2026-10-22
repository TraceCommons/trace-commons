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
//! file, and an empty list all admit nobody; every one but the empty list
//! also logs `staging_allow_list_unavailable` with a bare reason.
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
///
/// The file is read on every call, deliberately: an operator's edit takes
/// effect on the next sign-in, without a restart. Do not move this into a
/// startup cache. The cost is bounded: the finish routes are rate-limited and
/// the file is capped at [`MAX_LIST_BYTES`].
pub(super) async fn new_account_for(subject: &str) -> NearAiNewAccount {
    match read_list_at(std::env::var_os(ALLOWED_SUBJECTS_PATH_ENV)).await {
        Ok(list) if is_listed(&list, subject) => NearAiNewAccount::AllowListed,
        Ok(_) => NearAiNewAccount::NotListed,
        Err(reason) => {
            // A bare label and a reason, never the path or the subject. A
            // wrong path otherwise looks the same as a subject that is simply
            // not listed.
            tracing::warn!(
                control = "staging_allow_list_unavailable",
                reason = reason.label(),
                "the staging allow-list cannot be used; it admits nobody"
            );
            NearAiNewAccount::NotListed
        }
    }
}

/// Why the list could not be used. Each one admits nobody.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unavailable {
    /// The setting is not set.
    Unset,
    /// The setting names nothing.
    Missing,
    /// The setting names a directory or other non-file.
    NotFile,
    /// The file is larger than [`MAX_LIST_BYTES`].
    Oversized,
    /// The file could not be read, or is not UTF-8.
    Unreadable,
}

impl Unavailable {
    fn label(self) -> &'static str {
        match self {
            Self::Unset => "unset",
            Self::Missing => "missing",
            Self::NotFile => "not_file",
            Self::Oversized => "oversized",
            Self::Unreadable => "unreadable",
        }
    }
}

/// The list's contents, or why there are none.
///
/// The size is checked twice: once from the metadata, and again on the read,
/// which stops at one byte past the cap, so a file that grows between the two
/// is refused rather than read whole.
async fn read_list_at(path: Option<std::ffi::OsString>) -> Result<String, Unavailable> {
    use tokio::io::AsyncReadExt as _;

    let path = path.ok_or(Unavailable::Unset)?;
    let metadata = match tokio::fs::metadata(&path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(Unavailable::Missing);
        }
        Err(_) => return Err(Unavailable::Unreadable),
    };
    if !metadata.is_file() {
        return Err(Unavailable::NotFile);
    }
    if metadata.len() > MAX_LIST_BYTES {
        return Err(Unavailable::Oversized);
    }
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| Unavailable::Unreadable)?;
    let mut bytes = Vec::new();
    file.take(MAX_LIST_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| Unavailable::Unreadable)?;
    if bytes.len() as u64 > MAX_LIST_BYTES {
        return Err(Unavailable::Oversized);
    }
    String::from_utf8(bytes).map_err(|_| Unavailable::Unreadable)
}

fn is_listed(list: &str, subject: &str) -> bool {
    // `str::trim` keeps a byte-order mark, which some editors write first.
    let list = list.strip_prefix('\u{FEFF}').unwrap_or(list);
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

    /// A list saved by an editor that writes a byte-order mark still lists
    /// its first subject.
    #[test]
    fn a_leading_byte_order_mark_is_not_part_of_the_first_subject() {
        let list = "\u{FEFF}auth0|alice\ngithub|42\n";
        assert!(is_listed(list, "auth0|alice"));
        assert!(is_listed(list, "github|42"));
        assert!(!is_listed(list, "\u{FEFF}auth0|alice"));
    }

    /// Every way the list can be unusable has its own bare label, so an
    /// operator with a wrong path reads why, not only "not admitted".
    #[tokio::test]
    async fn an_unusable_list_names_why() {
        let dir = tempfile::tempdir().expect("dir");
        let file = dir.path().join("allowed-subjects");

        assert_eq!(read_list_at(None).await, Err(Unavailable::Unset));
        assert_eq!(
            read_list_at(Some(file.clone().into())).await,
            Err(Unavailable::Missing)
        );
        assert_eq!(
            read_list_at(Some(dir.path().into())).await,
            Err(Unavailable::NotFile)
        );
        std::fs::write(&file, vec![b'a'; MAX_LIST_BYTES as usize + 1]).expect("write");
        assert_eq!(
            read_list_at(Some(file.clone().into())).await,
            Err(Unavailable::Oversized)
        );
        std::fs::write(&file, [0xff, 0xfe, b'\n']).expect("write");
        assert_eq!(
            read_list_at(Some(file.clone().into())).await,
            Err(Unavailable::Unreadable)
        );
        std::fs::write(&file, "auth0|alice\n").expect("write");
        assert_eq!(
            read_list_at(Some(file.into())).await,
            Ok("auth0|alice\n".to_string())
        );

        for reason in [
            Unavailable::Unset,
            Unavailable::Missing,
            Unavailable::NotFile,
            Unavailable::Oversized,
            Unavailable::Unreadable,
        ] {
            let label = reason.label();
            assert!(
                label.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                "{label} is a bare name"
            );
        }
    }
}
