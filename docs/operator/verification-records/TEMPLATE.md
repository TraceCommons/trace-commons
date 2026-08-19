# Client end-to-end verification pass record — app-vFILLME

Copy this file to `app-v<version>.md` in this directory, run the campaign in
[`../client-end-to-end-verification.md`](../client-end-to-end-verification.md),
and fill it in as you go. Every value below starts as `FILLME` so that an
unedited copy cannot satisfy the release gate.

The gate is
[`scripts/operator/check-verification-record.sh`](../../../scripts/operator/check-verification-record.sh),
which the `version` job of `.github/workflows/release-apps.yml` runs on every
`app-v*` tag push.

## Pass record

The block below is the machine-readable half. Keep it a fenced `pass-record`
block; the gate parses it and nothing else in this file.

```pass-record
version: FILLME
date: FILLME
operator: FILLME
artifact_sha256_macos: FILLME
artifact_sha256_linux: FILLME
artifact_sha256_windows: FILLME
invite_hash: FILLME
platform_macos: FILLME
platform_linux: FILLME
platform_windows: FILLME
submitted_set_transcripts_only: FILLME
submissions_withdrawn: FILLME
quarantined_found: FILLME
quarantined_resolved: FILLME
update_channel_macos_brew: FILLME
update_channel_macos_dmg: FILLME
update_channel_linux_flatpak: FILLME
update_channel_windows_appinstaller: FILLME
defects_filed: FILLME
```

Accepted values:

| Key | Accepted |
|---|---|
| `version` | The three-part version being tagged, matching the argument the gate is called with |
| `date` | ISO date the campaign closed |
| `operator` | Who ran it. A name or handle, not an email address |
| `artifact_sha256_*` | 64 hex characters. The hash of the artifact actually installed, not of a build you have locally |
| `invite_hash` | 64 hex characters. The verification invite's hash, never the code itself |
| `platform_*` | `pass`, `fail`, or `not-run`. The gate requires `pass` on all three |
| `submitted_set_transcripts_only` | `pass` or `fail`. Every submission was a session transcript, and no memory file or prompt history was collected. The gate requires `pass` |
| `submissions_withdrawn` | Integer. How many submissions step 13 withdrew |
| `quarantined_found` | Integer. Recorded even when zero |
| `quarantined_resolved` | Integer. Must equal `quarantined_found` |
| `update_channel_*` | `current`, `stale`, or `not-run`. The gate requires `current` on all four |
| `defects_filed` | Issue or PR references, comma separated, or `none` |

## Step outcomes

One row per step of the path under test, per platform. `n/a` where a platform
does not have the surface.

| # | Step | macOS | Linux | Windows |
|---|---|---|---|---|
| 1 | Install the real artifact | | | |
| 2 | First launch from the platform launcher | | | |
| 3 | Declare roots | | | |
| 4 | Enroll | | | |
| 5 | Consent scopes | | | |
| 6 | Discover projects | | | |
| 7 | Watch a session | | | |
| 8 | Preview | | | |
| 9 | Redaction and privacy scan | | | |
| 10 | Consent and submit | | | |
| 11 | Server outcome | | | |
| 12 | Read back | | | |
| 13 | Withdraw | | | |
| 14 | Update channel | | | |

## Update channel detail

Which channel each install method actually exercised, and what it offered.
A channel that exists but does not carry the user forward is `stale`, not
`current`.

| Install method | Channel exercised | Version offered | Result |
|---|---|---|---|
| macOS, Homebrew cask | `brew upgrade --cask trace-commons` | | |
| macOS, direct DMG | Sparkle appcast | | |
| Linux, flatpak | `flatpak update` | | |
| Windows, appinstaller / MSIX | `.appinstaller` feed | | |

## Defects found

One row per defect. These belong to new slices, not to this campaign; the
campaign's job is to find them and file them.

| Defect | Platform | Step | Filed as |
|---|---|---|---|

## Visual checks

One row per screenshot kept. "Not performed" is an acceptable outcome and does
not fail the campaign; a full-screen capture is never acceptable. See the
runbook's screenshot rules.

| Surface | Window located at capture time | Owning process asserted | Result |
|---|---|---|---|
| Roots screen, empty state | | | |
| Refusal notice | | | |
| macOS Dock icon and menu-bar mark | | | |
| Done screen | | | |

## Collected-set confirmation

- [ ] Every submission in the campaign was a session transcript.
- [ ] No `memory/` file was collected.
- [ ] No `history.jsonl` content was collected.
- [ ] The count of submitted items equals the count of transcripts seeded.

## Cleanup confirmation

- [ ] Every verification submission withdrawn; count recorded above.
- [ ] Verification tenant's quarantined submissions enumerated; count recorded
      above whether or not any were found.
- [ ] Every quarantined submission resolved; counts reconcile.
- [ ] Verification invite revoked.
- [ ] Throwaway accounts, containers and VM snapshots removed or reset.

## Evidence

Per-step observations. Written assertions, not transcripts. See the runbook's
evidence rules for what may and may not be captured, and for the screenshot
check that every image passes before it lands here.
