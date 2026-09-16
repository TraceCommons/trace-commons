# Private AI credential storage

Release guidance for the native credential-store migration. Schedule this change after the v0.12.0 tag, following the maintainer's release sequencing request.

## Upgrade and rollback

On the first daemon start after upgrading, the app moves its Private AI inference key and renewable sign-in from settings into an OS credential store: the macOS data-protection keychain, Windows Credential Manager with Local persistence, or Linux Secret Service. On Windows and Linux, the operating system may ask for access to its credential store at startup. On macOS it does not: the data-protection keychain entry is reached by a team access group shared across every signed build, rather than bound to the one binary that wrote it, so there is no per-upgrade prompt to grant. A process without that entitlement -- an unsigned local build, or a command-line invocation -- cannot reach this store at all and refuses instead of falling back to a less-protected one. This is the Cloud credential only; see Commons credentials below for the account session and device identity, which are unchanged and can still prompt.

The app reads the new entry back before removing the legacy secrets from settings. If storage is locked or unavailable, the app preserves the existing settings, disables Private AI credentials in memory and shows recovery instructions. Unlock the store and restart the app to retry.

An older app ignores the new credential reference and sees no Private AI sign-in while retaining the other settings. After rolling back, sign in again; the old app may then save those credentials in its settings file.

## What moves

The Private AI inference key and renewable Private AI sign-in use the Cloud credential lifecycle. The Commons account session and device identity now use a separate OS namespace and binding domains; see [Commons credential storage](commons-credential-storage.md) for migration, rollback, and recovery behavior.

Copying or synchronizing the settings directory no longer copies the migrated Cloud secrets. Historical copies can still contain plaintext credentials, and the operating system's own credential-store backup rules still apply. This migration does not protect against every process running with the contributor's authority.

## Commons credentials

Commons credentials retain separate authority from Cloud authentication. Their migration preserves the enrolled device key and rejects stale sign-in and sign-out operations. Validation and platform-test instructions are recorded in the [Commons storage runbook](commons-credential-storage.md).
