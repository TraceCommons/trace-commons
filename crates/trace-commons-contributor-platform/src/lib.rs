//! OS-facing leaf shared by the Trace Commons contributor and its Insights
//! crate: per-entry OS credential storage, the durable 0600 atomic write, the
//! no-follow import opener, and the Windows current-user SID.
//!
//! Split out of `trace-commons-contributor` so that `trace-commons-contributor-insights`
//! can use these without depending on the contributor. The contributor
//! re-exports each item at its original path (`daemon::credential_store`,
//! `daemon::os_secret_store`, `config::write_atomic_0600`,
//! `evidence_import::open_import_file`,
//! `daemon::win_pipe::current_user_sid_string`).
//!
//! Every module here is `#[doc(hidden)]`. Items were crate-private before the
//! split and are public only so those two crates can keep calling them; they
//! are not a supported interface.
//!
//! # `test-credential-store`
//!
//! TEST-ONLY, forwarded from the contributor's feature of the same name (and
//! from the Insights crate's). Under it, every [`os_secret_store`] constructor
//! refuses outside this crate's own `cfg(test)`, so an integration test or a
//! dependent crate's unit test can never reach the user's real keychain. This
//! crate's own unit tests keep the exemption, because its `#[ignore]`d manual
//! round trips and macOS entitlement probes exist to reach the real store.

#[doc(hidden)]
pub mod credential_store;
#[doc(hidden)]
pub mod local_files;
#[doc(hidden)]
pub mod os_secret_store;
#[cfg(windows)]
#[doc(hidden)]
pub mod windows;

#[cfg(unix)]
#[doc(hidden)]
pub mod native_process_group;

#[doc(hidden)]
pub mod external_claude_credentials;

#[cfg(target_os = "macos")]
#[doc(hidden)]
pub mod native_pty;
