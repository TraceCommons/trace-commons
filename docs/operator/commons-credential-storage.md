# Commons account and device credentials

The contributor stores its Commons account bearer and Ed25519 device key in
macOS Keychain, Windows Credential Manager, or Linux Secret Service. These are
separate from the Private AI inference key and refresh token. Their OS service
namespace is `trace-commons.account.credentials`; each entry has an opaque UUID.
No new dependency or plaintext fallback is used.

The existing device-key file and `account-session.json` contain versioned
references after migration. Device keys and account sessions have distinct
binding domains. Bindings include the canonical state directory and reference;
account sessions also bind to the enrollment's server, tenant, subject, instance,
audience, and device. Copying a reference to another directory or credential kind
does not grant access there.

## Migration and recovery

Migration happens on the next credential read. An existing PKCS#8 key is moved
without generating a replacement, preserving the enrolled device ID. The old
file remains intact until the immutable OS entry passes readback verification.
New sign-in and enrollment writes use the OS store from the outset. A native
credential-service failure does not fall back to a new plaintext secret.

Unlock the system credential store and retry if access is unavailable. A missing
OS device entry is an error, never permission to silently generate a replacement.
If an entry has been permanently lost, recover it through the OS or explicitly
log out and enroll again. Copying only the contributor directory does not restore
its credentials.

Older binaries cannot parse the migrated key reference as PKCS#8, so rollback
refuses to load it instead of rotating the identity. Older account-session readers
see no usable bearer. Do not remove a device reference just to bypass that error.

## Sign-out, cancellation, and cleanup

Sign-out invalidates the local account session before trying remote revocation.
A durable generation prevents an older browser completion or a paused OS write
from restoring it. Revocation uses only the removed session; an older sign-out
cannot remove a newer session. Wallet cancellation also invalidates pending
publication. Logout clears both credential references while preserving the
cleanup journal if the native service cannot delete an entry immediately.

The lifecycle reuses the Private AI storage/commit file locks, bounded reference
journal, immutable write/readback primitive, and directory durability helper.
OS calls happen outside the commit lock. Unpublished and retired entries remain
journaled until deletion succeeds; a later credential write or logout retries
cleanup. Enrollment publishes its config only after the credential verifies.

This protects secrets from copies, sync, and backups of the contributor state
directory. It does not protect against arbitrary code running as the same OS user,
and it does not make claims about an OS provider's own backup policy.

## Validation

Synthetic-backend tests exercise migration, unchanged identity, failed readback,
missing or locked entries, swapped references, directory copies, account switching,
stale sign-in/sign-out, paused writes, and cleanup retry. The opt-in native test
creates and deletes only a fresh synthetic entry:

```sh
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib \
  real_commons_os_round_trip -- --ignored
```

Run that test on each platform with its native store available. A macOS result
alone does not establish Windows or Linux runtime behavior.
