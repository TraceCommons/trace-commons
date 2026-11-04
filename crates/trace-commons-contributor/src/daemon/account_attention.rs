//! Whether this app's account needs attention (#1836 section 3.2a, fix F1).
//!
//! #1837 lets a NEAR AI sign-in through without the Commons check in two
//! cases no retry fixes: the stored account record cannot be read, or this
//! build's signup allowlist refuses the address the account was set up
//! with ([`super::account_session_renewal::FreshLogin::Unchecked`]). The
//! Commons session cannot renew in either, so uploads and account reads
//! fail while the app looks signed in. [`account_attention`] reports the
//! same two cases on every status build, signed in to NEAR AI or not, as
//! `status.account_state: "needs_attention"` with a fixed reason. Signing
//! in again does not fix it; unenrolling and setting up again does. The
//! daemon never unenrolls on its own.
//!
//! # Same cases as #1837
//!
//! The gate is #1837's own ([`checks_near_ai_login`]: a NEAR AI tenant, a
//! device key not revoked, a device that did not join by one-time code),
//! then a stored record (signed out is not attention), then the record, then
//! the address ([`commons_address_refused`], the check
//! [`super::account_session_renewal::commons_client`] makes).
//!
//! # Status reads no credential store
//!
//! A status build must not wait on, or raise, a credential-store prompt,
//! and must not write. So it never reads the record itself. Every read of
//! the account record -- renewal, sign-in, the session every account
//! feature presents -- leaves what it found under the snapshot it read
//! ([`commons_credentials::observed_account_read`]), and status asks only
//! that, for the snapshot it took. "Unreadable" therefore shows once
//! anything has tried to read the record since it last changed (the app's
//! account reads do on every screen), and goes the moment the files change.

use super::account_session_renewal::{checks_near_ai_login, commons_address_refused};
use super::commons_credentials::{self, AccountLoadFailure, Snapshot};
use crate::config::{ConfigStore, ContributorConfig};
use crate::consent_copy::{ACCOUNT_ATTENTION_ADDRESS_REFUSED, ACCOUNT_ATTENTION_RECORD_UNREADABLE};

/// The reason this app's account needs attention, or `None` when it does
/// not (module doc): [`ACCOUNT_ATTENTION_RECORD_UNREADABLE`] or
/// [`ACCOUNT_ATTENTION_ADDRESS_REFUSED`]. `snapshot` is the account
/// snapshot status already took. Reads two small marker files; no
/// credential store, no network, no writes.
pub(crate) fn account_attention(
    store: &ConfigStore,
    cfg: Option<&ContributorConfig>,
    snapshot: Option<&Snapshot>,
) -> Option<&'static str> {
    let cfg = cfg?;
    let snapshot = snapshot?;
    if !checks_near_ai_login(store, cfg) || !snapshot.holds_record() {
        return None;
    }
    if commons_credentials::observed_account_read(store, snapshot)
        == Some(AccountLoadFailure::Unreadable)
    {
        return Some(ACCOUNT_ATTENTION_RECORD_UNREADABLE);
    }
    address_refused(store, cfg).then_some(ACCOUNT_ATTENTION_ADDRESS_REFUSED)
}

/// [`commons_address_refused`]. Under `cfg(test)` a directory routed to a
/// test's stub passes, as `check_fresh_login_live` does.
fn address_refused(store: &ConfigStore, cfg: &ContributorConfig) -> bool {
    #[cfg(test)]
    if super::account_session_renewal::test_route(store).is_some() {
        return false;
    }
    let _ = store;
    commons_address_refused(cfg)
}
