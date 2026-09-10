# Private AI credential storage

Release guidance for the native credential-store migration. Schedule this change after the v0.12.0 tag, following the maintainer's release sequencing request.

## Upgrade and rollback

On the first daemon start after upgrading, the app moves its Private AI inference key and renewable sign-in from settings into macOS Keychain, Windows Credential Manager with Local persistence, or Linux Secret Service. The operating system may ask for access to its credential store at startup.

The app reads the new entry back before removing the legacy secrets from settings. If storage is locked or unavailable, the app preserves the existing settings, disables Private AI credentials in memory and shows recovery instructions. Unlock the store and restart the app to retry.

An older app ignores the new credential reference and sees no Private AI sign-in while retaining the other settings. After rolling back, sign in again; the old app may then save those credentials in its settings file.

## What moves

Only the Private AI inference key and renewable Private AI sign-in move. The TraceCommons account access token in `account-session.json` and the device identity private key remain in local files, protected by the existing file permissions. The next credential-storage change must cover those two independently of Cloud authentication.

Copying or synchronizing the settings directory no longer copies the migrated Cloud secrets. Historical copies can still contain plaintext credentials, and the operating system's own credential-store backup rules still apply. This migration does not protect against every process running with the contributor's authority.

## Follow-up: Commons credential storage

Move the Commons account session and device identity into OS storage with independently reviewed migration and recovery behavior. Preserve the existing device identity through upgrade; replacing it would change the server's device binding.

Acceptance requires verified write-before-removal, unavailable-store recovery, withdrawal and sign-out races, restart during migration, rollback behavior, and real platform round-trips. Account switching must never expose a previous contributor's account token. Track the account-session and device-key work together while retaining separate authority from the Cloud credential pair.
