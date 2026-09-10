# Private AI credential storage

Release guidance for the native credential-store migration. Schedule this change after the v0.12.0 tag, following the maintainer's release sequencing request.

## Upgrade and rollback

On the first daemon start after upgrading, the app moves its Private AI inference key and renewable sign-in from settings into macOS Keychain, Windows Credential Manager with Local persistence, or Linux Secret Service. The operating system may ask for access to its credential store at startup.

The app reads the new entry back before removing the legacy secrets from settings. If storage is locked or unavailable, the app preserves the existing settings, disables Private AI credentials in memory and shows recovery instructions. Unlock the store and restart the app to retry.

An older app ignores the new credential reference and sees no Private AI sign-in while retaining the other settings. After rolling back, sign in again; the old app may then save those credentials in its settings file.

## What moves

The Private AI inference key and renewable Private AI sign-in use the Cloud credential lifecycle. The Commons account session and device identity now use a separate OS namespace and binding domains; see [Commons credential storage](commons-credential-storage.md) for migration, rollback, and recovery behavior.

Copying or synchronizing the settings directory no longer copies the migrated Cloud secrets. Historical copies can still contain plaintext credentials, and the operating system's own credential-store backup rules still apply. This migration does not protect against every process running with the contributor's authority.

## Commons credentials

Commons credentials retain separate authority from Cloud authentication. Their migration preserves the enrolled device key and rejects stale sign-in and sign-out operations. Validation and platform-test instructions are recorded in the [Commons storage runbook](commons-credential-storage.md).
