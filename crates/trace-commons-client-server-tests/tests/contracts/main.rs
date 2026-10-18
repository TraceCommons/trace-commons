//! Cross-crate contracts run together, without putting the server in the
//! contributor's dev-dependencies or splitting them into extra link units.
#![cfg(not(windows))]

mod e2e_client_admission_seam;
mod e2e_enroll_and_submit;
mod native_auth;
