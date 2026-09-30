# Private AI credential storage

Release guidance for the native credential-store migration. Schedule this change after the v0.12.0 tag, following the maintainer's release sequencing request.

## Upgrade and rollback

On the first daemon start after upgrading, the app moves its Private AI inference key and renewable sign-in from settings into an OS credential store: the macOS data-protection keychain, Windows Credential Manager with Local persistence, or Linux Secret Service. On Windows and Linux, the operating system may ask for access to its credential store at startup. On macOS it does not: the data-protection keychain entry is reached by a team access group shared across every signed build, rather than bound to the one binary that wrote it, so there is no per-upgrade prompt to grant. A process without that entitlement -- an unsigned local build, or a command-line invocation -- cannot reach this store at all and refuses instead of falling back to a less-protected one. This is the Cloud credential only; see Commons credentials below for the account session and device identity, which are unchanged and can still prompt.

The app reads the new entry back before removing the legacy secrets from settings. If the stored credential cannot be loaded at startup, the app preserves the existing settings, disables Private AI credentials in memory, and reports one of three states, each with its own sentence and action:

| State | Cause | What the contributor sees and does |
| --- | --- | --- |
| `storage_unavailable` | The store did not answer: locked, access denied, or a platform error. | "Unlock your system credential store and restart the app." Restarting after unlocking retries. Forget is offered. |
| `storage_unentitled` | macOS: this binary is not signed with the keychain access group (a local ad-hoc build, or any build signed without the entitlement). | A sentence saying this copy of the app cannot use the store and that restarting will not change it. No action is offered: this build can neither read nor delete the entry, and a Forget here would drop the signed app's pointer to a sign-in this build cannot see. |
| `migration_available` | macOS: the store answered and holds nothing under the settings reference. The only other place it can be is the legacy login keychain an earlier build wrote. | A sentence saying the sign-in from an earlier version is still in the login keychain, and a **Move my sign-in** button. |

The daemon logs the state label (never the reference or a platform error) at startup.

### Upgrading on macOS from a legacy-keychain build

Builds before the data-protection move stored the Cloud credential in the legacy login keychain, bound to the binary that wrote it. After upgrading, settings still name that entry's reference, and the data-protection store has nothing under it, so the app starts in `migration_available`.

Nothing reads the legacy keychain at startup. That read can raise a login-password prompt, and an unexplained prompt at launch is the interruption this change removes. The contributor chooses **Move my sign-in**, and the daemon (`near_ai_credential_migrate`):

1. reads the legacy entry once. macOS may ask for the login password here, and the sentence beside the button says so beforehand;
2. checks it against the binding the settings record, and writes it into the data-protection store under the same reference, reading it back to verify;
3. picks the key up in the running daemon, exactly as after a sign-in. No browser sign-in is needed, and the existing `sk-` key and renewable sign-in keep working.

The move copies; it does not delete. The legacy entry is the contributor's live key, and it stays in the login keychain until a later sign-in or Forget supersedes that reference, at which point the ceremony tail (or Forget) removes it. The legacy sweep never deletes the reference settings currently name.

If neither store holds the entry (it was deleted by hand, or swept after a later sign-in and then restored from an older settings copy), the move drops the dangling reference, and the state becomes `absent` with Sign in offered. There was nothing left to keep. If the legacy read fails for any other reason, for example because the contributor declined the password prompt, the move reports an error and changes nothing, so it can be tried again.

### Downgrading

An older build reads only the legacy keychain:

- If the contributor has moved the sign-in and not signed in again since, the legacy entry is still there and the older build keeps working, prompting on upgrade as it always did.
- If the contributor signed in again on the newer build, the new credential exists only in the data-protection store, and the superseded legacy entry was removed. The older build finds nothing under the reference and shows its `storage_unavailable` sentence ("unlock and restart"). Restarting does not help there; sign in again on the older build.

An older app from before the OS-store migration ignores the credential reference entirely and sees no Private AI sign-in while retaining the other settings. After rolling back that far, sign in again; the old app may then save those credentials in its settings file.

## What moves

The Private AI inference key and renewable Private AI sign-in use the Cloud credential lifecycle. The Commons account session and device identity now use a separate OS namespace and binding domains; see [Commons credential storage](commons-credential-storage.md) for migration, rollback, and recovery behavior.

Copying or synchronizing the settings directory no longer copies the migrated Cloud secrets. Historical copies can still contain plaintext credentials, and the operating system's own credential-store backup rules still apply. This migration does not protect against every process running with the contributor's authority.

## Commons credentials

Commons credentials retain separate authority from Cloud authentication. Their migration preserves the enrolled device key and rejects stale sign-in and sign-out operations. Validation and platform-test instructions are recorded in the [Commons storage runbook](commons-credential-storage.md).

## The Tauri desktop app on macOS

The Tauri desktop app (`tauri-desktop/`, bundle id `ai.tracecommons.desktop`) links the same contributor crate as the native shell, so it stores the Cloud credential in the same data-protection keychain under the same access group, `KXSWJN7WY8.ai.tracecommons.shell`. Access groups are scoped to the team, not to the App ID, so a credential either app stored is readable by the other with no prompt and no re-sign-in. Both apps share the `tracecommons://` scheme, and a contributor who moves from one to the other keeps their Private AI sign-in.

The two apps have different App IDs, so each needs its own signing inputs:

| | Native shell | Tauri desktop app |
| --- | --- | --- |
| Entitlements | `macos/entitlements.plist` | `tauri-desktop/src-tauri/entitlements.plist` |
| `com.apple.application-identifier` | `KXSWJN7WY8.ai.tracecommons.shell` | `KXSWJN7WY8.ai.tracecommons.desktop` |
| `keychain-access-groups` | `KXSWJN7WY8.ai.tracecommons.shell` | `KXSWJN7WY8.ai.tracecommons.shell` (the same group, deliberately) |
| Provisioning profile | `macos/TraceCommons-DeveloperID.provisionprofile` | `tauri-desktop/src-tauri/TraceCommonsDesktop-DeveloperID.provisionprofile` |

Neither profile can stand in for the other. Both grant `KXSWJN7WY8.*`, so the access-group check passes either way, but a profile names exactly one App ID, and a signed application identifier that does not match the embedded profile's gets the app killed at exec. `scripts/ci/verify-macos-entitlements.sh` checks that match for both apps; it reads the bundle id and executable from the bundle's own `Info.plist`.

`tauri.release.conf.json` passes the entitlements file to the Tauri bundler (`bundle.macOS.entitlements`) and copies the profile into the bundle as `Contents/embedded.provisionprofile` (`bundle.macOS.files`). `tauri-desktop/scripts/package-macos-release.sh` refuses to start without the profile, and runs the verifier against the signed app before notarizing. The verifier runs its static checks only for the Tauri app: its launch check needs the app to answer `TRACE_COMMONS_CREDENTIAL_STORE_CHECK_OUT`, which only the native shell does so far.

A Tauri build signed without these inputs -- `pnpm tauri dev`, or any build that does not pass `tauri.release.conf.json` -- is unentitled, and shows `storage_unentitled` for Private AI, as an ad-hoc native build does.

### Creating the Tauri profile

This needs the Apple Developer account for team `KXSWJN7WY8`, and is done once; the profile does not carry a private key and is committed like the native one.

1. In Certificates, Identifiers & Profiles, register an explicit App ID for macOS with bundle id `ai.tracecommons.desktop` (Identifiers > App IDs > App, platform macOS). No extra capability is needed for the access group: the native shell's profile carries no capability beyond the defaults, and its `keychain-access-groups` grant is the default `KXSWJN7WY8.*`.
2. Under Profiles, create a **Developer ID** provisioning profile (Distribution > Developer ID) for that App ID, selecting the same Developer ID Application certificate the releases are signed with.
3. Download it and commit it as `tauri-desktop/src-tauri/TraceCommonsDesktop-DeveloperID.provisionprofile`.
4. Confirm it grants what the app requests:

   ```bash
   security cms -D -i tauri-desktop/src-tauri/TraceCommonsDesktop-DeveloperID.provisionprofile \
     | plutil -extract Entitlements xml1 -o - -
   ```

   `com.apple.application-identifier` must be `KXSWJN7WY8.ai.tracecommons.desktop`, and `keychain-access-groups` must contain `KXSWJN7WY8.*`.
