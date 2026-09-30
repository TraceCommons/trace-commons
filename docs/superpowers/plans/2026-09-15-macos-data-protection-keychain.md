# macOS data-protection keychain Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop macOS asking the contributor for their login password on every app upgrade, by moving the Cloud credential store to the data-protection keychain.

**Architecture:** `trace-commons.near-ai.credentials` moves from the legacy file keychain (per-binary ACL) to the data-protection keychain (team access group). The account and device store stays on the legacy keychain because the Homebrew CLI needs it and a CLI binary cannot carry the entitlement. The app bundle gains an entitlements file and an embedded provisioning profile; without the profile the app is killed at exec, so CI must launch the signed app.

**Tech Stack:** Rust (`trace-commons-contributor`), `apple-native-keyring-store` `=1.0.2` (`protected` feature), `security-framework` `=3.7.0`, Swift (`macos/Sources/TraceCommonsApp`), bash signing scripts, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-09-15-macos-data-protection-keychain-design.md`

## Global Constraints

- Verify with `RUSTFLAGS="-D warnings"`; plain `cargo check` does not catch what CI catches.
- Clippy allow-list: `-A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`. Do not widen it.
- Run `cargo fmt --all` before every commit.
- No emojis in commits, PRs, code or comments. Short imperative commit subjects, no `feat:`/`fix:` prefix.
- Hash-only or label-only on every stored row and log string. Never format an OS error into a surface; `storage_error` exists to prevent that.
- Fail closed: a configured control whose dependency is missing refuses the path with a named missing-control label. Never fall back silently.
- `trace-commons-contributor` is MIT OR Apache-2.0. It must not gain a dependency on `trace-commons-server`, `-gate-api` or `-gate-enclave`.
- The C ABI header exists in two copies that CI requires to be byte-for-byte identical: `crates/trace-commons-contributor-ffi/include/trace_commons.h` and `macos/Sources/CTraceCommons/include/trace_commons.h`.
- Access group string, used verbatim: `KXSWJN7WY8.ai.tracecommons.shell`
- Team identifier: `KXSWJN7WY8`. Bundle identifier: `ai.tracecommons.shell`.

---

### Task 1: The `Unentitled` credential error

An unentitled process can never succeed, unlike `Unavailable` which may be transient. The label must differ so the shell can say a different sentence.

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/credential_store.rs:20-31` (enum), `:33-46` (Display)

**Interfaces:**
- Consumes: nothing.
- Produces: `CredentialError::Unentitled`, displaying as `near_ai_credential_storage_unentitled`.

- [ ] **Step 1: Write the failing test**

Add to the test module in `crates/trace-commons-contributor/src/daemon/credential_store.rs`:

```rust
    /// The unentitled label is distinct from the unavailable one on purpose.
    /// "Unavailable" invites a retry; this condition is permanent for the
    /// process that hit it, and telling a contributor to try again would be
    /// a lie they could act on.
    #[test]
    fn unentitled_is_its_own_label() {
        assert_eq!(
            CredentialError::Unentitled.to_string(),
            "near_ai_credential_storage_unentitled"
        );
        assert_ne!(
            CredentialError::Unentitled.to_string(),
            CredentialError::Unavailable.to_string()
        );
    }
```

- [ ] **Step 2: Run it and watch it fail**

```bash
cd /tmp/tc-dpkeychain
cargo test -p trace-commons-contributor --lib unentitled_is_its_own_label
```

Expected: compile error, `no variant named 'Unentitled'`.

- [ ] **Step 3: Add the variant and its label**

In the enum at `credential_store.rs:20`, after `Unavailable`:

```rust
    /// This process is not entitled to the store that holds this credential.
    /// Distinct from `Unavailable`: no retry, no unlock and no later attempt
    /// changes it, because the answer is a property of the binary's code
    /// signature rather than of the store's state.
    Unentitled,
```

In the `Display` impl, after the `Unavailable` arm:

```rust
            Self::Unentitled => "near_ai_credential_storage_unentitled",
```

- [ ] **Step 4: Run it and watch it pass**

```bash
cargo test -p trace-commons-contributor --lib unentitled_is_its_own_label
```

Expected: `test result: ok. 1 passed`.

- [ ] **Step 5: Check every match on CredentialError still compiles**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --all-targets
```

If a match is now non-exhaustive, add an arm that treats `Unentitled` like `Unavailable` at that site and leave a comment saying why that site cannot distinguish them. Do not add a catch-all `_ =>`; the compiler naming every site is the point.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Give an unentitled credential store its own label"
```

---

### Task 2: Map errSecMissingEntitlement onto it

`apple-native-keyring-store` recognises `-34018` and then folds it into `PlatformFailure` alongside everything else (`protected.rs:602`). The OSStatus survives inside the boxed `security_framework::base::Error`, so we recover it by downcast.

**The version pin matters.** The downcast only succeeds if we depend on the *same* `security-framework` the store crate resolved. A mismatched version makes `downcast_ref` return `None` forever, silently producing `Unavailable` instead of `Unentitled` — a wrong label that no compiler catches. Step 1's test is what catches it.

**Files:**
- Modify: `crates/trace-commons-contributor/Cargo.toml` (add macOS dependency)
- Modify: `crates/trace-commons-contributor/src/daemon/os_secret_store.rs:120-131` (`storage_error`)

**Interfaces:**
- Consumes: `CredentialError::Unentitled` from Task 1.
- Produces: `storage_error` returning `CredentialError::Unentitled` for OSStatus `-34018`.

- [ ] **Step 1: Write the failing test**

Add to the test module at the end of `os_secret_store.rs`. This runs the *real* protected store from an unentitled test process, which is exactly the condition we are classifying:

```rust
    /// `cargo test` runs unentitled, so the real data-protection store
    /// answers -34018 here. That makes this the one test that can prove the
    /// downcast in `storage_error` actually fires -- if the
    /// `security-framework` pin ever drifts from the one
    /// `apple-native-keyring-store` resolved, the downcast silently stops
    /// matching and every unentitled failure quietly reports as
    /// `Unavailable`. Nothing else would notice.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_unentitled_process_is_reported_as_unentitled() {
        let backend = OsSecretBackend::new().expect("store handle constructs");
        let reference = CredentialReference::allocate();
        match backend.read(&reference) {
            Err(CredentialError::Unentitled) => {}
            other => panic!("expected Unentitled from an unentitled process, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run it and watch it fail**

```bash
cargo test -p trace-commons-contributor --lib an_unentitled_process_is_reported_as_unentitled
```

Expected: FAIL — `expected Unentitled ..., got Err(Unavailable)`. (Before Task 3 this exercises the legacy store and will report `NoEntry` instead; that is still a failure, and Task 3 is what makes the test meaningful. Leave it failing and proceed — Task 3 is the other half of this change.)

- [ ] **Step 3: Add the dependency**

In `crates/trace-commons-contributor/Cargo.toml`, in the macOS-specific dependency table beside `apple-native-keyring-store`:

```toml
# Pinned to the exact version `apple-native-keyring-store` resolves, because
# `storage_error` downcasts to this crate's `base::Error` to recover the
# OSStatus. A different version is a different type and the downcast would
# silently never match.
security-framework = "=3.7.0"
```

- [ ] **Step 4: Classify the status**

In `os_secret_store.rs`, replace the catch-all arm of `storage_error`:

```rust
fn storage_error(error: KeyringError) -> CredentialError {
    // Platform errors can contain secret bytes or identifying metadata. Never
    // format, log, or retain them in the daemon's error surface.
    match error {
        KeyringError::NoEntry => CredentialError::NoEntry,
        KeyringError::TooLong(_, _) => CredentialError::TooLarge,
        KeyringError::BadEncoding(_) | KeyringError::BadDataFormat(_, _) => {
            CredentialError::InvalidBundle
        }
        #[cfg(target_os = "macos")]
        KeyringError::PlatformFailure(ref inner) if is_missing_entitlement(inner) => {
            CredentialError::Unentitled
        }
        _ => CredentialError::Unavailable,
    }
}

/// errSecMissingEntitlement, recovered from the boxed platform error.
///
/// The store crate matches -34018 by name and then returns it as an ordinary
/// `PlatformFailure`, so the variant alone cannot distinguish it. The OSStatus
/// survives on the boxed value.
#[cfg(target_os = "macos")]
fn is_missing_entitlement(error: &(dyn std::error::Error + Send + Sync)) -> bool {
    error
        .downcast_ref::<security_framework::base::Error>()
        .is_some_and(|error| error.code() == -34018)
}
```

If `PlatformFailure`'s payload type differs from `Box<dyn Error + Send + Sync>` in this version, adjust the signature of `is_missing_entitlement` to match what the variant actually holds. Do not change the downcast target.

- [ ] **Step 5: Verify**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --all-targets
```

Expected: clean. The new test still fails until Task 3.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Recover errSecMissingEntitlement from the boxed platform error"
```

---

### Task 3: Move the Cloud store to the data-protection keychain

**Files:**
- Modify: `crates/trace-commons-contributor/Cargo.toml:611` (crate features)
- Modify: `crates/trace-commons-contributor/src/daemon/os_secret_store.rs:14` (constants), `:26-55` (`new`, `commons`)

**Interfaces:**
- Consumes: `CredentialError::Unentitled` (Task 1), the `storage_error` mapping (Task 2).
- Produces: `OsSecretBackend::new()` backed by the data-protection store; `OsSecretBackend::commons()` unchanged on the legacy store.

- [ ] **Step 1: Write the failing differential test**

Add beside the Task 2 test in `os_secret_store.rs`:

```rust
    /// The two stores are deliberately different backends, and this is the
    /// assertion that says so out loud. An unentitled process cannot reach
    /// the Cloud store at all, while the account store -- which the Homebrew
    /// CLI reads for `login`, `whoami` and `submit` -- keeps working exactly
    /// as it did. If someone later "simplifies" these onto one backend, this
    /// fails rather than the CLI silently losing its device identity.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_account_store_is_not_moved_with_the_cloud_store() {
        let reference = CredentialReference::allocate();
        assert!(matches!(
            OsSecretBackend::new().unwrap().read(&reference),
            Err(CredentialError::Unentitled)
        ));
        assert!(matches!(
            OsSecretBackend::commons().unwrap().read(&reference),
            Err(CredentialError::NoEntry)
        ));
    }
```

- [ ] **Step 2: Run both macOS tests and watch them fail**

```bash
cargo test -p trace-commons-contributor --lib os_secret_store
```

Expected: both new tests FAIL — the Cloud store is still the legacy one, so it answers `NoEntry`, not `Unentitled`.

- [ ] **Step 3: Enable the feature**

`crates/trace-commons-contributor/Cargo.toml:611`:

```toml
apple-native-keyring-store = { version = "=1.0.2", features = ["keychain", "protected"] }
```

Both features stay on: `keychain` serves the account store and the orphan sweep in Task 5, `protected` serves Cloud credentials. `protected` maps to `security-framework/OSX_10_15`, a feature of a crate already in the graph — no new packages.

- [ ] **Step 4: Construct the data-protection store**

In `os_secret_store.rs`, beside `SERVICE`:

```rust
// The team access group the app's entitlement and provisioning profile both
// name. Access is granted by this group rather than by a per-binary ACL,
// which is what stops an upgrade prompting: any build signed by the same team
// with this group reads the item, and there is no ACL to invalidate.
#[cfg(target_os = "macos")]
const ACCESS_GROUP: &str = "KXSWJN7WY8.ai.tracecommons.shell";
```

Replace the macOS arm of `new()`:

```rust
        #[cfg(target_os = "macos")]
        let store: Arc<NativeStore> = {
            // `cloud-sync` is left at its default of false, selecting the
            // device-local store. Stated rather than inherited: the retained
            // refresh token is device authority and must not synchronise to
            // iCloud.
            let configuration = std::collections::HashMap::from([("access-group", ACCESS_GROUP)]);
            apple_native_keyring_store::protected::Store::new_with_configuration(&configuration)
                .map_err(storage_error)?
        };
```

Add to `commons()`:

```rust
    /// The account and device store, deliberately still on the legacy file
    /// keychain.
    ///
    /// It is reached from `config.rs` for `login`, `whoami`, `submit` and
    /// `status`, and the Homebrew CLI is not and cannot be entitled: a
    /// command-line binary cannot carry a keychain access group. Moving this
    /// store to match the Cloud one would lock the CLI out of its own device
    /// identity. The upgrade prompt is worth removing; `submit` is not worth
    /// breaking to remove it.
    pub(crate) fn commons() -> Result<Self, CredentialError> {
```

`commons()` must keep constructing the legacy store. If `new()` is what it currently calls to get its handle, split the store construction so `commons()` reaches `keychain::Store::new()` directly rather than inheriting the data-protection one.

- [ ] **Step 5: Run the tests**

```bash
cargo test -p trace-commons-contributor --lib os_secret_store
```

Expected: both macOS tests pass, every other test in the module still passes.

- [ ] **Step 6: Full verification**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib
RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
cargo clippy -p trace-commons-contributor --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
```

Expected: all clean.

- [ ] **Step 7: Do not write a migration, and confirm you did not**

The spec chooses no migration deliberately. The next sign-in writes into the
new store, which costs nothing because 0.12.5 already refuses to refresh any
session stored without a `user_agent` -- every contributor must run the
ceremony once regardless. Reading the legacy item to carry it across would
spend the exact password prompt this work removes, to preserve a session that
is already dead.

```bash
git diff --stat
```

Expected: no new code that reads the legacy store. Task 5 deletes from it; it
never reads it.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Hold Cloud credentials in the data-protection keychain"
```

---

### Task 4: Refuse the ceremony before a browser opens

Discovering an unwritable store *after* the contributor has authenticated at NEAR AI wastes a real sign-in and leaves a session at the service we never stored.

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/cloud_credential_lifecycle.rs` (add the probe)
- Modify: `crates/trace-commons-contributor/src/daemon/nearai_credential/ceremony.rs` (`begin`)
- Test: `crates/trace-commons-contributor/src/daemon/nearai_credential/ceremony_regression_tests.rs`

**Interfaces:**
- Consumes: `CredentialError::Unentitled`.
- Produces: `cloud_credential_lifecycle::store_is_reachable(&ConfigStore) -> Result<()>`, erroring with `near_ai_credential_storage_unentitled`.

- [ ] **Step 1: Write the failing test**

In `ceremony_regression_tests.rs`:

```rust
/// A ceremony that cannot store its result must say so before it opens a
/// browser. The contributor authenticates at NEAR AI, which mints a real
/// session; failing after that point spends a sign-in and strands a session
/// at the service that this machine will never hold.
#[tokio::test]
async fn an_unstorable_ceremony_refuses_before_opening_a_browser() {
    let fixture = Fixture::new();
    fixture.backend.fail_next_with_unentitled();
    let error = begin(&fixture.store, "github").await.unwrap_err();
    assert_eq!(error.to_string(), "near_ai_credential_storage_unentitled");
    assert_eq!(fixture.browser_urls_served(), 0, "no browser URL was minted");
}
```

Use whatever fixture type this file already uses. If the test-support backend has no way to force an error, add one to `crates/trace-commons-contributor/src/daemon/cloud_credential_test_support.rs` in the same shape as its existing controls, and assert on the count of served browser URLs using whatever `begin` returns (it hands back a JSON value containing the URL).

- [ ] **Step 2: Run it and watch it fail**

```bash
cargo test -p trace-commons-contributor --lib an_unstorable_ceremony_refuses_before_opening_a_browser
```

Expected: FAIL — `begin` currently returns a browser URL.

- [ ] **Step 3: Add the probe**

In `cloud_credential_lifecycle.rs`:

```rust
/// Can this process hold a Cloud credential at all?
///
/// A read of a reference that does not exist answers `NoEntry` when the store
/// is reachable and `Unentitled` when it is not, so one read distinguishes
/// them without writing anything.
pub(crate) fn store_is_reachable(store: &ConfigStore) -> Result<()> {
    match native(store)?.probe() {
        Err(CredentialError::Unentitled) => {
            Err(anyhow!("near_ai_credential_storage_unentitled"))
        }
        _ => Ok(()),
    }
}
```

`CloudCredentialLifecycle` has no backend accessor -- `new(store, backend)` keeps
it as a private field -- so add the probe as a method on the struct rather than
reaching in from outside. Routing through `native()` is what keeps the
`#[cfg(test)]` substitution working; a probe that constructed `OsSecretBackend`
directly would touch the real keychain from the unit suite:

```rust
impl<B: SecretBackend> CloudCredentialLifecycle<B> {
    /// One read of a reference that was never stored. `NoEntry` means the
    /// store answered; `Unentitled` means this process cannot reach it at all.
    pub(crate) fn probe(&self) -> Result<(), CredentialError> {
        match self.backend.read(&CredentialReference::allocate()) {
            Err(CredentialError::Unentitled) => Err(CredentialError::Unentitled),
            _ => Ok(()),
        }
    }
}
```

Every other outcome, including a genuine failure, is deliberately treated as reachable: this check exists to catch the permanent condition, not to become a second health gate that can refuse a working machine.

- [ ] **Step 4: Call it first in `begin`**

In `ceremony.rs`, as the first statement of `begin`, before `CloudApi::live()`:

```rust
    // Before anything that reaches the network or a browser: a ceremony this
    // process could not store is a wasted sign-in at the service.
    crate::daemon::cloud_credential_lifecycle::store_is_reachable(store)?;
```

- [ ] **Step 5: Run the tests**

```bash
cargo test -p trace-commons-contributor --lib nearai_credential
```

Expected: the new test passes, every existing ceremony test still passes.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Refuse a ceremony this process could never store"
```

---

### Task 5: Sweep the orphaned legacy item

After Task 3 the legacy Cloud item is unreachable by the normal cleanup path — `cleanup()` now talks to the data-protection store and would look for the old reference in the wrong place.

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/cloud_credential_lifecycle.rs` (add `sweep_legacy_cloud_entries`)
- Modify: the daemon startup path that already calls `cleanup_native`
- Test: `crates/trace-commons-contributor/src/daemon/cloud_credential_recovery_tests.rs`

**Interfaces:**
- Consumes: the cleanup `Journal` type already in this module.
- Produces: `cloud_credential_lifecycle::sweep_legacy_cloud_entries(&ConfigStore)`, returning `()` and never failing.

- [ ] **Step 1: Write the failing test**

```rust
/// The sweep is housekeeping, not a control. A legacy entry that refuses to
/// delete -- because macOS wants authorization for it, which is the exact
/// interruption this work removes -- must leave startup untouched.
#[test]
fn a_refused_legacy_delete_does_not_fail_startup() {
    let fixture = Fixture::new();
    fixture.legacy_backend.fail_all_deletes();
    sweep_legacy_cloud_entries(&fixture.store);
    assert!(fixture.store.exists(), "startup state is intact");
}
```

Match the fixture and assertions already used in this file.

- [ ] **Step 2: Run it and watch it fail**

```bash
cargo test -p trace-commons-contributor --lib a_refused_legacy_delete_does_not_fail_startup
```

Expected: compile error, `cannot find function 'sweep_legacy_cloud_entries'`.

- [ ] **Step 3: Implement the sweep**

```rust
/// Delete Cloud entries left in the legacy keychain by builds before the
/// data-protection move. Best effort by construction.
///
/// It returns nothing and swallows every failure on purpose. macOS may want
/// authorization to delete a legacy item, and spending a password prompt to
/// tidy up would reintroduce the exact interruption this work exists to
/// remove. An entry we cannot delete is left alone.
pub(crate) fn sweep_legacy_cloud_entries(store: &ConfigStore) {
    #[cfg(target_os = "macos")]
    {
        let Ok(journal) = Journal::read(store) else { return };
        let Ok(backend) = crate::daemon::os_secret_store::OsSecretBackend::legacy_cloud() else {
            return;
        };
        for reference in journal.references {
            let _ = backend.delete(&reference);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = store;
}
```

Add `OsSecretBackend::legacy_cloud()` to `os_secret_store.rs`: a constructor using `keychain::Store::new()` with `service: SERVICE`. It exists only for this sweep and should say so in a doc comment.

- [ ] **Step 4: Call it once at startup**

At the site that already calls `cleanup_native`, add the sweep immediately after it. It must not gate anything or change a return value.

- [ ] **Step 5: Run the tests**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Sweep Cloud entries left in the legacy keychain"
```

---

### Task 6: Entitlements and the embedded profile

**Files:**
- Create: `macos/entitlements.plist`
- Modify: `macos/scripts/make-release-dmg.sh:197-203`
- Already committed: `macos/TraceCommons-DeveloperID.provisionprofile`

**Interfaces:**
- Consumes: nothing in Rust.
- Produces: a signed bundle containing `Contents/embedded.provisionprofile` and carrying `keychain-access-groups`.

- [ ] **Step 1: Create the entitlements file**

`macos/entitlements.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>com.apple.application-identifier</key>
  <string>KXSWJN7WY8.ai.tracecommons.shell</string>
  <key>com.apple.developer.team-identifier</key>
  <string>KXSWJN7WY8</string>
  <key>keychain-access-groups</key>
  <array>
    <string>KXSWJN7WY8.ai.tracecommons.shell</string>
  </array>
</dict>
</plist>
```

The access group is the specific group, not the `KXSWJN7WY8.*` wildcard the profile permits. The profile is the ceiling; the entitlement sits at the floor.

- [ ] **Step 2: Embed the profile and sign with entitlements**

In `make-release-dmg.sh`, replace the comment at line 197 and the `codesign` that follows it:

```bash
# Hardened runtime is required for notarization. The entitlements file requests
# exactly one thing, and the app uses it: `keychain-access-groups`, which is
# what lets a build read the Cloud credential an earlier build stored without
# asking the contributor for their password. The legacy keychain binds an item
# to the binary that created it, so every upgrade prompted, and "Always Allow"
# only ever added the binary that was already running.
#
# The embedded profile is load-bearing, not decoration. A binary carrying this
# entitlement WITHOUT a profile that grants it is killed by the kernel at exec
# -- measured, not assumed. The failure is an application that does not start,
# which is why CI launches the signed app rather than trusting that it signed.
cp macos/TraceCommons-DeveloperID.provisionprofile "$APP/Contents/embedded.provisionprofile"
codesign --force --timestamp --options runtime \
  --entitlements macos/entitlements.plist \
  --sign "$MACOS_SIGNING_IDENTITY" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"
```

The copy must precede the `codesign`: a profile added after the bundle is sealed is not covered by the signature.

- [ ] **Step 3: Leave the local bundle script alone, and say why**

In `macos/scripts/make-app-bundle.sh`, above its ad-hoc `codesign`:

```bash
# Deliberately no --entitlements here. This script ad-hoc signs, and an ad-hoc
# binary carrying `keychain-access-groups` is killed at exec. A locally built
# app therefore runs unentitled and cannot reach Cloud credentials; work on the
# sign-in ceremony needs a Developer ID-signed build. See the data-protection
# keychain spec.
```

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "Sign the app with the keychain access group it uses"
```

---

### Task 7: A self-check the signed app can run

CI must be able to ask a signed bundle whether it can actually reach its credential store. `SelfTest` is the established shape for this: environment-gated, writes a report, inert otherwise.

**Files:**
- Modify: `crates/trace-commons-contributor-ffi/src/lib.rs` (new export)
- Modify: `crates/trace-commons-contributor-ffi/include/trace_commons.h` and `macos/Sources/CTraceCommons/include/trace_commons.h` (both, byte-identical)
- Modify: `macos/Sources/TraceCommonsApp/SelfTest.swift`
- Test: `crates/trace-commons-contributor-ffi/tests/` and `macos/Tests/TraceCommonsAppTests/`

**Interfaces:**
- Consumes: `OsSecretBackend::new()` from Task 3.
- Produces: `int32_t tc_credential_store_self_check(void)` returning `0` when the store is reachable, `1` when unentitled, `2` on any other failure.

- [ ] **Step 1: Write the failing Rust test**

In a new `crates/trace-commons-contributor-ffi/tests/credential_store_self_check.rs`:

```rust
/// The test process is unentitled, so the honest answer here is 1. A 0 would
/// mean the check cannot tell the two apart, which would make it useless as
/// the thing standing between us and shipping an app that does not launch.
#[test]
#[cfg(target_os = "macos")]
fn an_unentitled_process_reports_one() {
    assert_eq!(unsafe { tc_credential_store_self_check() }, 1);
}
```

Declare the extern exactly as the other tests in that directory declare theirs.

- [ ] **Step 2: Run it and watch it fail**

```bash
cargo test -p trace-commons-contributor-ffi --test credential_store_self_check
```

Expected: link error, symbol not found.

- [ ] **Step 3: Export the check**

In `crates/trace-commons-contributor-ffi/src/lib.rs`:

```rust
/// Can this process reach the Cloud credential store?
///
/// Exists so a release pipeline can ask a *signed bundle* the question, which
/// no unit test can answer: entitlements are a property of the code signature
/// and `cargo test` never has one. Returns 0 reachable, 1 unentitled, 2
/// otherwise. Reads nothing and writes nothing.
#[unsafe(no_mangle)]
pub extern "C" fn tc_credential_store_self_check() -> i32 {
    trace_commons_contributor::daemon::credential_store_self_check()
}
```

Add the corresponding `pub fn credential_store_self_check() -> i32` in the daemon module, performing the same probe read as Task 4's `store_is_reachable` and mapping `Unentitled` to 1, everything else reachable to 0, and a failure to construct the backend to 2. Match the existing `#[unsafe(no_mangle)]` / `#[no_mangle]` spelling already used in that file.

- [ ] **Step 4: Update BOTH header copies identically**

Append to both `crates/trace-commons-contributor-ffi/include/trace_commons.h` and `macos/Sources/CTraceCommons/include/trace_commons.h`:

```c
/*
 * Can this process reach the Cloud credential store?
 *
 * 0 reachable, 1 unentitled, 2 otherwise. Reads and writes nothing. Exists so
 * a release pipeline can ask a signed bundle a question no unit test can
 * answer.
 */
int32_t tc_credential_store_self_check(void);
```

Then prove they match, because CI requires it:

```bash
diff crates/trace-commons-contributor-ffi/include/trace_commons.h \
     macos/Sources/CTraceCommons/include/trace_commons.h && echo "headers identical"
```

- [ ] **Step 5: Run the Rust test**

```bash
cargo test -p trace-commons-contributor-ffi --test credential_store_self_check
```

Expected: PASS.

- [ ] **Step 6: Add the Swift hook**

In `SelfTest.swift`, add a function and call it from `runIfRequested` beside the existing hooks:

```swift
    /// Writes whether this bundle can reach the Cloud credential store.
    ///
    /// The release pipeline runs this against the *signed* app. A build whose
    /// provisioning profile went missing is killed at exec and never writes
    /// the file at all, which is itself the failure signal CI checks for.
    @MainActor
    private static func runCredentialStoreCheckIfRequested() {
        guard let path = ProcessInfo.processInfo.environment["TRACE_COMMONS_CREDENTIAL_STORE_CHECK_OUT"],
              !path.isEmpty
        else { return }
        let code = tc_credential_store_self_check()
        let report = code == 0 ? "reachable\n" : "unreachable code=\(code)\n"
        try? report.write(toFile: path, atomically: true, encoding: .utf8)
        NSLog("trace-commons: credential store check wrote \(path)")
        NSApp.terminate(nil)
    }
```

Call `runCredentialStoreCheckIfRequested()` from `runIfRequested`.

- [ ] **Step 7: Run the Swift suite**

```bash
cargo build -p trace-commons-contributor-ffi
cd macos && swift test 2>&1 | tail -20
```

Expected: builds and passes. A stale FFI dylib produces phantom interop failures, so the `cargo build` first is not optional.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Let a signed bundle report whether it can reach its store"
```

---

### Task 8: Make CI prove it

Four assertions, each catching a different route to the same silent brick. Only the first proves anything by execution.

**Files:**
- Create: `scripts/ci/verify-macos-entitlements.sh`
- Modify: `.github/workflows/release-apps.yml` (after "Build, sign, notarize and staple", before "Rename and checksum")
- Modify: `.github/workflows/ci.yml` (`macos-app-tests`, line 910)

**Interfaces:**
- Consumes: `tc_credential_store_self_check` (Task 7), the entitlements and profile (Task 6).
- Produces: a release job that fails when the signed app cannot reach its store.

- [ ] **Step 1: Write the verification script**

`scripts/ci/verify-macos-entitlements.sh`:

```bash
#!/usr/bin/env bash
# Assertions about a signed TraceCommons.app that its own signature cannot make.
#
# The failure this guards against is total: an app carrying the keychain
# entitlement without a profile that grants it is killed at exec. Every check
# that existed before this one stayed green through that.
set -euo pipefail

APP="${1:?usage: verify-macos-entitlements.sh <path to TraceCommons.app>}"
PROFILE="$APP/Contents/embedded.provisionprofile"

echo "--- the profile is embedded"
test -f "$PROFILE" || { echo "FAIL: no embedded.provisionprofile in the bundle"; exit 1; }

echo "--- the entitlement is present"
ENTS="$(codesign -d --entitlements - --xml "$APP" 2>/dev/null | plutil -convert xml1 -o - -)"
echo "$ENTS" | grep -q "KXSWJN7WY8.ai.tracecommons.shell" \
  || { echo "FAIL: keychain access group absent from the signature"; exit 1; }

echo "--- the profile grants what the entitlement requests"
GRANTED="$(security cms -D -i "$PROFILE" | plutil -extract Entitlements.keychain-access-groups xml1 -o - -)"
echo "$GRANTED" | grep -qE "KXSWJN7WY8\.(\*|ai\.tracecommons\.shell)" \
  || { echo "FAIL: profile does not grant the requested access group"; exit 1; }

echo "--- the profile is not near expiry"
EXPIRES="$(security cms -D -i "$PROFILE" | plutil -extract ExpirationDate raw -o - -)"
EXPIRES_EPOCH="$(date -j -f "%Y-%m-%dT%H:%M:%SZ" "$EXPIRES" +%s 2>/dev/null || date -d "$EXPIRES" +%s)"
DAYS_LEFT=$(( (EXPIRES_EPOCH - $(date +%s)) / 86400 ))
echo "profile expires $EXPIRES ($DAYS_LEFT days)"
test "$DAYS_LEFT" -gt 180 \
  || { echo "FAIL: under 180 days left; renew the profile or the certificate"; exit 1; }

echo "--- the signed app can actually reach its store"
OUT="$(mktemp)"
TRACE_COMMONS_CREDENTIAL_STORE_CHECK_OUT="$OUT" "$APP/Contents/MacOS/TraceCommonsApp" &
PID=$!
for _ in $(seq 1 30); do
  [ -s "$OUT" ] && break
  sleep 1
done
kill "$PID" 2>/dev/null || true
# An empty file means the app never got far enough to write one, which is what
# a kernel kill at exec looks like from here. That is the case this exists for.
test -s "$OUT" || { echo "FAIL: the signed app wrote nothing; it may not have launched"; exit 1; }
grep -q "^reachable" "$OUT" || { echo "FAIL: $(cat "$OUT")"; exit 1; }
echo "PASS: signed app reached the data-protection keychain"
```

- [ ] **Step 2: Make it executable and run it against a local ad-hoc bundle**

```bash
chmod +x scripts/ci/verify-macos-entitlements.sh
./macos/scripts/make-app-bundle.sh
./scripts/ci/verify-macos-entitlements.sh macos/.build/TraceCommons.app; echo "exit=$?"
```

Expected: FAIL at the first check — a locally built bundle has no profile. That is the script working. Confirm the message names the missing profile rather than erroring some other way.

- [ ] **Step 3: Wire it into the release job**

In `.github/workflows/release-apps.yml`, immediately after "Build, sign, notarize and staple":

```yaml
      # The signed app is launched here, not merely inspected. An app carrying
      # the keychain entitlement without its provisioning profile is killed by
      # the kernel at exec, and nothing else in this pipeline would notice: the
      # DMG builds, signs, notarizes and staples exactly as it should.
      - name: Verify entitlements and reach the credential store
        run: ./scripts/ci/verify-macos-entitlements.sh macos/.build/TraceCommons.app
```

- [ ] **Step 4: Run the macOS-only Rust tests in CI**

In `.github/workflows/ci.yml`, in `macos-app-tests` (line 910), after the existing `cargo build -p trace-commons-contributor-ffi` step:

```yaml
      # The only lane that runs these. They assert the data-protection store's
      # behaviour from an unentitled process, which is exactly what a CI runner
      # is, and they are the reason a drifted `security-framework` pin cannot
      # silently mislabel every unentitled failure.
      - name: macOS credential store tests
        run: |
          RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib os_secret_store
          RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-ffi --test credential_store_self_check
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Launch the signed app in CI rather than trusting it signed"
```

---

### Task 9: Documentation and release note

**Files:**
- Modify: `README.md` (macOS install/upgrade section, if it mentions the prompt)
- Create: `docs/announcements/<date>-v0-13-0.md` when the release is cut

- [ ] **Step 1: Write the announcement**

```markdown
# Trace Commons v0.13.0

Upgrading the macOS app no longer asks for your login password.

macOS ties a stored credential to the exact application that created it, so
every new build was a different application as far as the keychain was
concerned, and "Always Allow" only ever allowed the build already running.
The app now stores its NEAR AI credential in a place keyed to its signing
identity instead, which every future build shares.

- Upgrades no longer prompt for a password.
- A credential stored by an earlier version is not carried over. Signing in
  once on this version replaces it -- which 0.12.5 already required.

Contributing and private inference were never affected.
```

- [ ] **Step 2: Commit**

```bash
git add -A
git commit -m "Announce the macOS keychain change"
```

---

## Verification before opening the PR

```bash
cd /tmp/tc-dpkeychain
cargo fmt --all --check
RUSTFLAGS="-D warnings" cargo test --workspace
RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
cargo clippy -p trace-commons-contributor --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
diff crates/trace-commons-contributor-ffi/include/trace_commons.h macos/Sources/CTraceCommons/include/trace_commons.h
cargo build -p trace-commons-contributor-ffi && ( cd macos && swift test )
```

Use `cargo test --workspace`, not `--lib`. The FFI integration tests live in a
different crate and `--lib` does not run them; that gap is how a serialization
change shipped to CI unnoticed earlier in this branch's history.

## What this plan cannot prove

**Notarization.** No step here notarizes an entitled, profiled bundle. The
first release carrying this change is the evidence, and until one succeeds the
claim is unproven rather than true.

**That a contributor stops being prompted.** The spike proved a re-signed
binary reads without a prompt. The shipping equivalent -- upgrade 0.13.0 over
0.13.1 on a real machine and see no dialog -- happens after release. Do not
write the announcement's first line as fact until someone has watched it.
