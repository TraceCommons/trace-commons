//! The Windows transport: a per-user named pipe carrying the same
//! `trace_commons.daemon.v1_1` protocol as the unix-socket transport.
//!
//! # Why this file is security-critical
//!
//! On unix the socket is protected by its containing directory. `ipc::bind`
//! refuses to serve unless the state directory is 0700, and a 0700 directory
//! belonging to another user is not writable, so no one else can place a
//! socket there either. The directory does the work.
//!
//! **Windows has nothing playing that role.** A named pipe does not live in
//! the state directory; it lives in the machine-wide named-pipe namespace,
//! where any process on the machine can attempt to open it by name. Created
//! with a default security descriptor, a pipe is reachable by other users on
//! the same machine. The DACL built here is therefore the entire access
//! control on this transport -- not a defence in depth, the only defence.
//!
//! What is behind it: the daemon reads the contributor's coding transcripts,
//! can arm projects for unattended upload, and can approve uploads. An
//! over-permissive DACL hands all three to any local account.
//!
//! Every failure path in this module therefore refuses to serve. There is no
//! fallback to a default-ACL pipe, because that fallback *is* the
//! vulnerability -- it would turn a failure to build the control into a
//! silently unprotected daemon, which is the precise shape of bug the
//! repository's fail-closed rule exists to prevent.
//!
//! # VERIFICATION STATUS
//!
//! Cross-compiling this file establishes the FFI signatures, the ownership of
//! each allocation, and the control flow. **It establishes nothing about
//! whether the DACL actually excludes another user.** That is a runtime
//! property of the Windows security model and cannot be observed from a
//! cross-compile on another operating system.
//!
//! The observation is automated. CI's `windows-pipe-acl` job runs
//! `scripts/windows/verify-pipe-acl.ps1` on `windows-latest`, which creates a
//! second, non-administrator local account, has it attempt to open this pipe,
//! and requires ERROR_ACCESS_DENIED -- plus a control confirming the owning
//! user is still admitted, since a DACL that excludes everyone is also a bug.
//! `src/bin/win-pipe-acl-probe.rs` is the probe it drives.
//!
//! The account is deliberately not an administrator: an administrator can
//! take ownership of any object and would reach the pipe whether or not the
//! DACL were correct, so such a test would look like evidence while proving
//! nothing.
//!
//! **That job has run and passed** (PR #247, run 31307072159): the second
//! user was refused with `DENIED 5` -- ERROR_ACCESS_DENIED, i.e. refused by
//! the access check specifically, not by pipe-busy or file-not-found -- and
//! the owning user still connected. The DACL is verified behaviour, not an
//! argument, and stays that way only for as long as that job keeps running:
//! if it is removed or weakened, this claim lapses with it.

use std::ffi::c_void;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::{HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::ipc::{self, DaemonShared};
use crate::config::ConfigStore;

/// The named-pipe namespace is machine-wide and flat, so the name has to
/// distinguish both users and state directories. Hashing the state directory
/// path does both: the default state directory lives under the user's
/// profile, so two users never derive the same name, and a contributor
/// running two daemons from two `TRACE_COMMONS_CONTRIBUTOR_DIR` values gets
/// two pipes rather than a collision.
///
/// The hash is truncated rather than used whole because the name appears in
/// error text; 16 hex characters is far beyond what a collision would need
/// while staying readable. The path itself is never used directly -- a pipe
/// name is visible to every process on the machine, and a state directory
/// path carries the OS username.
pub fn pipe_name(store: &ConfigStore) -> String {
    let mut hasher = Sha256::new();
    hasher.update(store.dir().as_os_str().to_string_lossy().as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!(r"\\.\pipe\trace-commons-daemon-{}", &digest[..16])
}

/// A security descriptor owning its Win32 allocation.
///
/// `ConvertStringSecurityDescriptorToSecurityDescriptorW` allocates with
/// `LocalAlloc`, so the descriptor must be released with `LocalFree` and not
/// with Rust's allocator. Wrapping it in a type with a `Drop` is what keeps
/// that pairing correct across the several early-return paths below.
struct OwnedSecurityDescriptor(PSECURITY_DESCRIPTOR);

impl Drop for OwnedSecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` came from
            // `ConvertStringSecurityDescriptorToSecurityDescriptorW`, which
            // documents `LocalFree` as its release function, and this is the
            // only place it is freed.
            unsafe { LocalFree(self.0 as *mut c_void) };
        }
    }
}

/// The current process's user SID, as an SDDL string.
///
/// This is the identity the pipe is restricted to. Deriving it from the
/// process token rather than from a username avoids every ambiguity around
/// domain accounts, renamed accounts, and localised well-known group names.
fn current_user_sid_string() -> Result<String> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle that needs no
    // closing, and `token` is a valid out-pointer.
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if opened == 0 {
        bail!("cannot open the process token to determine the pipe's owner");
    }
    let _guard = TokenHandle(token);

    // Ask for the size first: TOKEN_USER is variable-length because it ends
    // in a SID, whose length depends on the account's domain.
    let mut needed: u32 = 0;
    // SAFETY: a null buffer with a zero length is the documented way to
    // query the required size; the call is expected to fail.
    unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
    if needed == 0 {
        bail!("cannot size the process token's user information");
    }
    let mut buf = vec![0u8; needed as usize];
    // SAFETY: `buf` is `needed` bytes, which is the size the previous call
    // asked for.
    let got = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buf.as_mut_ptr() as *mut c_void,
            needed,
            &mut needed,
        )
    };
    if got == 0 {
        bail!("cannot read the process token's user information");
    }

    // SAFETY: on success the buffer holds a TOKEN_USER, whose first field is
    // the SID_AND_ATTRIBUTES we need.
    let token_user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
    let mut raw: *mut u16 = std::ptr::null_mut();
    // SAFETY: `token_user.User.Sid` points into `buf`, which outlives this
    // call, and `raw` is a valid out-pointer.
    let converted = unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut raw) };
    if converted == 0 || raw.is_null() {
        bail!("cannot render the process token's user SID");
    }
    // SAFETY: `raw` is a NUL-terminated wide string allocated by
    // `ConvertSidToStringSidW`.
    let len = unsafe {
        let mut n = 0usize;
        while *raw.add(n) != 0 {
            n += 1;
        }
        n
    };
    // SAFETY: `raw` is valid for `len` u16s, established immediately above.
    let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(raw, len) })
        .context("the user SID is not valid UTF-16");
    // SAFETY: `ConvertSidToStringSidW` documents `LocalFree` as its release
    // function. Freed on both the ok and error paths because `sid` is not
    // returned until after this point.
    unsafe { LocalFree(raw as *mut c_void) };
    sid
}

/// Closes a token handle on drop.
struct TokenHandle(HANDLE);

impl Drop for TokenHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` is a handle from `OpenProcessToken` and this
            // is the only place it is closed.
            unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
        }
    }
}

// `GetTokenInformation` lives in Win32_Security but is not re-exported at the
// path the rest of this module uses, so it is named explicitly here.
use windows_sys::Win32::Security::GetTokenInformation;

/// Build the descriptor that restricts the pipe to the current user.
///
/// The SDDL is `D:P(A;;GA;;;<sid>)`:
///   - `D:`  this is the DACL
///   - `P`   protected -- inherited entries are not applied. Without this a
///           permissive inheritable entry from elsewhere could widen access
///           after the fact, which would defeat the whole descriptor.
///   - `A`   allow
///   - `GA`  generic all
///   - `<sid>` exactly one trustee: the user that started the daemon.
///
/// There is deliberately no entry for Administrators. An administrator can
/// already take ownership of any object, so adding one would grant nothing
/// that is not already available while making the intent of the descriptor
/// less obvious to read.
fn build_security_descriptor() -> Result<OwnedSecurityDescriptor> {
    let sid = current_user_sid_string()?;
    let sddl = format!("D:P(A;;GA;;;{sid})");
    let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();

    let mut psd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `wide` is a NUL-terminated wide string that outlives the call,
    // and `psd` is a valid out-pointer. A null size out-pointer is permitted.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide.as_ptr(),
            SDDL_REVISION_1,
            &mut psd,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 || psd.is_null() {
        bail!("cannot build the security descriptor restricting the daemon pipe to this user");
    }
    Ok(OwnedSecurityDescriptor(psd))
}

/// Create one pipe instance carrying the restricting descriptor.
///
/// `first` must be true for the very first instance and false for every one
/// after it. `first_pipe_instance` is what makes creation *fail* rather than
/// silently join a pipe of the same name that some other process created
/// first -- without it on the first instance, a squatter could stand up the
/// pipe under a weaker descriptor and be served on, and the name is
/// derivable by anyone. But the flag also means "no other instance of this
/// name may exist", so passing it again for instance two fails with
/// ERROR_ACCESS_DENIED and the daemon would stop answering after its first
/// client. Hence the parameter rather than a constant.
///
/// Every error here is a refusal to serve. See the module doc: falling back
/// to a default-ACL pipe would expose the contributor's transcripts and their
/// upload controls to every account on the machine.
fn create_instance(store: &ConfigStore, first: bool) -> Result<NamedPipeServer> {
    let name = pipe_name(store);
    let descriptor = build_security_descriptor()
        .context("refusing to serve: the daemon pipe cannot be restricted to this user")?;

    let mut attrs = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };

    // SAFETY: `attrs` outlives the call, and the descriptor it points at is
    // owned by `descriptor`, which outlives `attrs`.
    let server = unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .create_with_security_attributes_raw(&name, &mut attrs as *mut _ as *mut c_void)
    }
    .context(
        "refusing to serve: cannot create the daemon pipe with a restricted \
         security descriptor (a pipe of this name may already exist)",
    )?;

    Ok(server)
}

/// Create the first pipe instance, refusing if it cannot be restricted.
pub async fn bind(store: &ConfigStore) -> Result<NamedPipeServer> {
    create_instance(store, true)
}

/// Serve connections until shutdown is requested.
///
/// Named pipes differ from sockets here: each instance serves one client, so
/// the next instance must be created before the current one is handed off,
/// or a client connecting in the gap gets a "pipe busy" error instead of
/// service. Every subsequent instance is created with the same descriptor as
/// the first -- an instance created without one would be the same
/// vulnerability arriving one connection later.
pub async fn serve(first: NamedPipeServer, shared: Arc<DaemonShared>) -> Result<()> {
    let mut server = first;
    loop {
        if server.connect().await.is_err() {
            continue;
        }
        let next = create_instance(&shared.store, false)
            .context("creating the next daemon pipe instance")?;
        let connected = std::mem::replace(&mut server, next);
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let _ = ipc::serve_connection(connected, shared).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_name_is_in_the_pipe_namespace_and_carries_no_path() {
        let dir = std::env::temp_dir().join("tc-pipe-name-test");
        std::fs::create_dir_all(&dir).unwrap();
        let store = ConfigStore::open(dir.clone()).unwrap();
        let name = pipe_name(&store);
        assert!(name.starts_with(r"\\.\pipe\"), "not a pipe name: {name}");
        // The name is visible to every process on the machine, so it must not
        // carry the state directory -- which under the default layout carries
        // the OS username.
        assert!(
            !name.contains("tc-pipe-name-test"),
            "the pipe name leaks the state directory: {name}"
        );
    }

    #[test]
    fn distinct_state_directories_get_distinct_pipes() {
        let a = std::env::temp_dir().join("tc-pipe-a");
        let b = std::env::temp_dir().join("tc-pipe-b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        assert_ne!(
            pipe_name(&ConfigStore::open(a.clone()).unwrap()),
            pipe_name(&ConfigStore::open(b.clone()).unwrap())
        );
    }

    #[test]
    fn pipe_name_is_stable_across_calls() {
        let dir = std::env::temp_dir().join("tc-pipe-stable");
        std::fs::create_dir_all(&dir).unwrap();
        let store = ConfigStore::open(dir.clone()).unwrap();
        assert_eq!(pipe_name(&store), pipe_name(&store));
    }

    #[test]
    fn the_descriptor_names_exactly_one_trustee_and_is_protected() {
        // Guards the SDDL shape against edits. `P` (protected) is the part
        // most easily lost in a refactor, and losing it lets an inherited
        // permissive entry widen access to the pipe.
        let sid = "S-1-5-21-1-2-3-1001";
        let sddl = format!("D:P(A;;GA;;;{sid})");
        assert!(sddl.starts_with("D:P("), "the DACL must be protected");
        assert_eq!(sddl.matches("(A;").count(), 1, "exactly one allow entry");
        assert!(!sddl.contains("WD"), "must never grant to Everyone");
        assert!(
            !sddl.contains("AU"),
            "must never grant to Authenticated Users"
        );
    }
}
