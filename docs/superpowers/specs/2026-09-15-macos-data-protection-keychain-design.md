# macOS data-protection keychain for Cloud credentials

## Problem

Every app upgrade asks the contributor for their login password before the app
can read its own NEAR AI credential.

Cloud credentials live in `login.keychain-db`, the legacy file keychain, where
each item carries an ACL naming the binaries allowed to decrypt it.
`apple_native_keyring_store::keychain` writes items with no `SecAccess`, so an
item gets the default ACL: trust the creating binary and nothing else. An
upgrade replaces that binary, the new one is not on the list, and macOS asks.

Choosing "Always Allow" does not settle it. That adds the *current* binary, so
the next release asks again. At the current release cadence -- 0.12.0 through
0.12.6 inside a week -- a contributor is asked on most upgrades.

The app's designated requirement is version-independent
(`identifier "ai.tracecommons.shell" and anchor apple generic and certificate
leaf[subject.OU] = KXSWJN7WY8`), so "signed by the same key" is expressible.
The legacy keychain simply does not express it, and the crate we use offers no
way to set an ACL that would.

## Approach

Move the Cloud credential store to the **data-protection keychain**, where
access is granted by team access group rather than per-binary ACL. Any build
signed by the same team with the same access group reads the item with no
prompt and no ACL to invalidate.

This was proven before the design was written, on this machine, with the real
Developer ID identity:

| Configuration | Launch | Data-protection keychain |
| --- | --- | --- |
| ad-hoc, no entitlement | runs | `errSecMissingEntitlement` (-34018) |
| Developer ID + entitlement, no profile | **SIGKILL (137)** | -- |
| Developer ID + entitlement + embedded profile | runs | `errSecSuccess`, read and write |
| different binary, re-signed, same key | runs | read the earlier build's item, no prompt |

The last row is the requirement. The second row is the risk, and it is the
reason this document keeps returning to the provisioning profile: without one,
an app carrying this entitlement is killed at exec. The failure is not a
degraded feature. It is an application that does not start.

## Scope

Only `trace-commons.near-ai.credentials` moves.

`trace-commons.account.credentials` -- the device identity key
(`commons_credentials::Kind::Device`) and the account session (`Kind::Account`)
-- **stays on the legacy keychain**. It is reached from `config.rs`, which the
Homebrew CLI uses for `login`, `whoami`, `submit` and `status`. Command-line
binaries cannot carry a keychain access group entitlement, so moving that store
would lock the CLI out of its own device identity. The upgrade prompt is worth
removing; `submit` is not worth breaking to remove it.

Both crate features stay enabled. `keychain` still serves the account store and
the one-shot sweep below; `protected` serves Cloud credentials.

## Store selection

`OsSecretBackend::new()` constructs, on macOS:

```
protected::Store::new_with_configuration({"access-group": "KXSWJN7WY8.ai.tracecommons.shell"})
```

`cloud-sync` is left at its default of false, which selects the device-local
store. That default is load-bearing and must be stated in the code rather than
inherited silently: the retained refresh token is device authority and has no
business synchronising to iCloud.

`OsSecretBackend::commons()` is unchanged, and gains a comment recording why it
is deliberately not moving.

Linux and Windows are untouched.

### Dependency cost

None. `protected = ["security-framework/OSX_10_15"]` is a feature of a crate
already pinned at `=1.0.2`. No new packages enter the graph.

## Fail-closed behavior

An unentitled process -- the Homebrew CLI, a local `cargo run`, any ad-hoc
signed build -- receives `errSecMissingEntitlement` from every operation.

This gets its own missing-control label,
`near_ai_credential_storage_unentitled`, distinct from the existing
`near_ai_credential_storage_unavailable`. The two conditions differ in a way a
contributor can act on:

- *unavailable*: the OS store is locked or broken. Retrying may work.
- *unentitled*: this process cannot hold this credential, by construction.
  Retrying never works.

Reporting the second as the first would tell a contributor to try again at
something that cannot succeed.

The ceremony refuses at `begin()`, before a browser opens. The current flow
binds a listener, opens a browser, and writes on completion. Discovering an
unwritable store after the contributor has authenticated at NEAR AI wastes a
real sign-in and leaves a session at the service that we never stored. The
check is free and belongs first.

## Migration

There is none. The next sign-in writes into the data-protection store.

This costs nothing, because a sign-in is already required: 0.12.5 refuses to
refresh a session stored without a `user_agent`, so every pre-0.12.5 session is
already dead and every contributor must run the ceremony once regardless.
Spending a prompt to migrate an item that must be replaced anyway buys nothing.

### Orphan sweep

The legacy item survives as an orphan holding a real, dead refresh token. The
existing cleanup journal cannot reach it: after this change `cleanup()` talks to
the data-protection store and would look for the old reference in the wrong
place.

A one-shot best-effort sweep runs through a legacy backend handle: attempt the
delete, ignore failure, never block startup, never prompt. If macOS demands
authorization to delete, the item is left alone. Spending a password prompt to
tidy up would reintroduce the exact interruption this work removes.

## Bundle, entitlements and signing

### The profile is committed to the repository

At `macos/TraceCommons-DeveloperID.provisionprofile`.

A provisioning profile carries no private key. It is an Apple-signed grant
naming the team's public certificate and the entitlements it permits, and it
ships world-readable inside every distributed bundle. Committing it removes a CI
secret rather than adding one, and makes its expiry a reviewable fact in the
tree instead of an invisible property of a secret that fails during a release.

The profile in hand grants:

```
com.apple.application-identifier      KXSWJN7WY8.ai.tracecommons.shell
com.apple.developer.team-identifier   KXSWJN7WY8
keychain-access-groups                KXSWJN7WY8.*
ExpirationDate                        2044-09-10   (TimeToLive 6570 days)
```

Keychain sharing required no portal capability. The grant is implicit in the App
ID prefix, which is why the Developer portal has no "Keychain Sharing" checkbox
to find.

### Entitlements

A new `macos/entitlements.plist` requests only what is used:

```
com.apple.application-identifier      KXSWJN7WY8.ai.tracecommons.shell
com.apple.developer.team-identifier   KXSWJN7WY8
keychain-access-groups                [KXSWJN7WY8.ai.tracecommons.shell]
```

The access group is the specific group, not the `KXSWJN7WY8.*` wildcard the
profile permits. The profile is the ceiling; the entitlement sits at the floor.

### Signing

`macos/scripts/make-release-dmg.sh` copies the profile to
`$APP/Contents/embedded.provisionprofile` **before** the outer `codesign`, which
gains `--entitlements`. Order is not cosmetic: a profile added after the bundle
is sealed is not covered by the signature.

The comment at `make-release-dmg.sh:197` is rewritten, not deleted. It records a
deliberate decision -- "there is deliberately no entitlements file: this app
needs no exception to the hardened runtime, and adding entitlements it does not
use would widen what a compromised process could do for no benefit" -- and that
reasoning stays correct. What changed is that there is now an entitlement the
app does use, for a benefit it does get. The replacement says so, and says that
the embedded profile is load-bearing: without it the app is killed at launch
rather than degraded.

### Local development

`make-app-bundle.sh` ad-hoc signs and must **not** apply the entitlement: an
ad-hoc binary carrying it is killed at exec.

A locally built app therefore runs unentitled and cannot reach Cloud
credentials. Work on the sign-in ceremony needs a Developer ID-signed build.
This is accepted rather than worked around; a second credential path selected by
build profile is where split-brain bugs live, and the fail-closed convention
prefers a refusal that is always a refusal.

## Testing

Entitlements are invisible to `cargo test`. Nothing in the unit suite can
establish that a signed bundle launches, so the load-bearing checks live in the
signing pipeline.

1. **Unit tests** continue against the substituted test backend
   (`cloud_credential_lifecycle::native()` already has a `#[cfg(test)]` arm).
   New tests cover the unentitled label mapping, the ceremony refusing at
   `begin()`, and the orphan sweep tolerating a failed delete without failing
   startup.
2. **Signed-launch check** in the macOS release job. After signing, execute the
   app with a self-check flag that exercises the data-protection keychain and
   exits non-zero on failure. This must be the app binary; the CLI cannot be
   entitled. This is the check that prevents shipping an application that does
   not start, and today nothing in the pipeline launches the signed app at all.
3. **Bundle content assertion**: fail the build when
   `Contents/embedded.provisionprofile` is absent after signing.
4. **Entitlement/profile consistency**: assert the entitlement's access group
   falls within the profile's grant and that the application-identifier matches
   the signing identifier.
5. **Profile expiry warning**: fail when fewer than 180 days remain on the
   committed profile's `ExpirationDate`. With a 2044 date this stays dormant
   for years, which is the intent -- it is insurance against the signing
   certificate lapsing, not a countdown being managed.

Check 2 is the one that proves anything. Checks 3 through 5 are assertions about
files, and each catches a different route to the same silent brick.

## Risks

**Notarization is unproven.** The spike signed but never notarized. No claim is
made that an entitled, profiled bundle notarizes as cleanly as the current one
until a real notarization succeeds. The first release carrying this change is
the evidence.

**The failure mode is total.** A CI change that drops, corrupts or fails to
embed the profile ships an app that does not launch. Every check that exists
today would stay green. This is the entire justification for check 2.

**Certificate, not profile, is the renewal driver.** The profile runs to 2044.
The Developer ID certificate it is bound to does not, and a profile is
invalidated when its certificate expires or is revoked.

## Prerequisites

The App ID `ai.tracecommons.shell` and the Developer ID provisioning profile
exist as of 2026-09-15 in the Iqlusion Inc (KXSWJN7WY8) team. The profile file
must be committed to the repository before any of this can be built.

## Release note draft

Not an announcement file: the version and date are unknown until release, and
this repo writes announcements in the release-prep commit.

> Upgrading the macOS app no longer asks for your login password.
>
> macOS ties a stored credential to the exact application that created it, so
> every new build was a different application as far as the keychain was
> concerned, and "Always Allow" only ever allowed the build already running.
> The app now stores its NEAR AI credential in a place keyed to its signing
> identity instead, which every future build shares.
>
> - Upgrades no longer prompt for a password.
> - A credential stored by an earlier version is not carried over. Signing in
>   once on this version replaces it -- which 0.12.5 already required.
>
> Contributing and private inference were never affected.
