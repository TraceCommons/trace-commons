# Client End-to-End Verification — Operator Runbook

Sub-project D. Design:
[`../superpowers/specs/2026-08-19-client-end-to-end-verification-design.md`](../superpowers/specs/2026-08-19-client-end-to-end-verification-design.md).
This is the GUI-client counterpart to
[`./pilot-contributor-onboarding.md`](./pilot-contributor-onboarding.md),
which covers the CLI path.

Every other check in this repository looks at code. This one looks at the
artifact a contributor downloads. On 2026-08-18 the difference was total: CI
was green, the DMG was notarized, and the installed macOS app could not start,
because `AppModel.start()`
(`macos/Sources/TraceCommonsApp/AppModel.swift:112`) resolved its state
directory from an environment variable that a Finder launch cannot set. All 32
daemon methods (`crates/trace-commons-contributor/src/daemon/ipc.rs:193`) sit
behind that construction, so `enroll`, `preview`, `approve` and the rest had
never executed in any contributor's hands.

## Goal

For each of macOS, Linux and Windows: install the released artifact, and drive
it from first launch to an accepted submission and back out again. Produce a
committed pass record.

The campaign verifies:

- The installed bundle launches from the platform's own launcher.
- The fail-closed roots refusal is escapable.
- A trace can be enrolled, watched, previewed, redacted, consented and
  submitted.
- The server's outcome is visible to the contributor, and withdrawal works.
- The update channel that *this install method actually uses* offers the
  current version.

A completed record is the release gate for the next `app-v*` tag.

---

## Pre-flight

1. **Candidate artifacts**, downloaded exactly as a contributor would, and
   their SHA-256 recorded for the pass record. Never a local build: a local
   build is a different artifact and tests nothing about the release pipeline.

2. **Verification invite.** Mint through the admin API, not the
   `--mint-invites` CLI path:

   ```bash
   curl -sS -X POST \
     -H "Authorization: Bearer $ADMIN_JWT" \
     -H "Content-Type: application/json" \
     -d '{"tenant_mode":"fixed","fixed_tenant_id":"<verification-tenant-uuid>","max_uses":25}' \
     "$INGEST_BASE/v1/admin/invites"
   ```

   `tenant_mode: "fixed"` with `fixed_tenant_id` is one of exactly two
   accepted shapes (`crates/trace-commons-server/src/trace_invite_admin.rs:209-217`);
   `max_uses: 0` is rejected (`:219-224`).

   The admin API rather than the CLI **because a CLI mint is not immediately
   redeemable** — it writes the database directly and the running issuer picks
   it up only on its next allowlist refresh, default 60 seconds
   ([`./pilot-allowlist.md`](./pilot-allowlist.md), lines 103-111). A
   verification run that opens with a spurious sixty-second failure teaches
   the operator to ignore failures.

   Record the invite's **hash**. The code itself never lands in a file.

3. **Clean user accounts.** Not clean directories — see "Why a whole account"
   below. One throwaway local account on macOS, one fresh container or VM for
   the flatpak, one Windows VM snapshot.

4. **Synthetic fixtures staged**, in a scratch project inside the root you
   will declare in step 3. Never the operator's real work. The fixtures seed
   known synthetic secrets so step 9 has something to find; take their shapes
   from the detector's own corpus rather than inventing them, so the fixture
   and the code under test are not authored to match.

5. **A copy of the pass-record template**, at
   `docs/operator/verification-records/app-v<version>.md`.

---

## Why the launcher matters

**Every macOS pass launches from Finder.** Not from a terminal.

A shell launch inherits the operator's environment, including
`TRACE_COMMONS_CONTRIBUTOR_DIR` — the exact variable whose absence is the
defect. `macos/scripts/run-demo.sh` exports it, which is precisely how a
non-starting app shipped. A shell launch will pass while the artifact fails.

The same reasoning applies more weakly on Linux and Windows: launch from the
desktop environment's launcher or the Start menu, not a terminal.

**Launch by explicit path or launcher, never by bundle identifier.** On one
development machine, five bundles were registered under
`ai.tracecommons.shell`, three of which no longer existed on disk; a bundle-id
launch started a stale development build instead of the installed one. Before
trusting any observation, confirm which executable is actually running.

---

## Why a whole account

`/v1/onboard` is not idempotent: an invite code is redeemable exactly once.
[`../release-runbook.md`](../release-runbook.md) (lines 45-56) records why the
Homebrew cask deliberately does not zap
`contributor.json` — deleting it strands a contributor whose invite is spent
and cannot be reissued.

So the obvious reset, deleting the state directory, burns an invite per run
and on the wrong machine destroys a real identity. And the one override that
could redirect the state directory is `TRACE_COMMONS_CONTRIBUTOR_DIR`, which
a Finder launch cannot set and which this runbook forbids.

Clean therefore means a clean **user account**. This is the largest
operational cost in this procedure, and it is stated here rather than
discovered halfway through. Precedent: the `windows named-pipe ACL` CI job
(`.github/workflows/ci.yml:225`) already creates a second non-administrator
local account, because the property it tests is not observable any other way.

Per campaign: one invite minted and revoked, up to 25 uses, one throwaway
macOS account, one Linux VM or container, one Windows VM snapshot per pass.

---

## The run

Fourteen steps, in order, per platform. Each names the daemon method or
surface it exercises so a failure lands somewhere specific. Record pass or
fail for each in the record's step table.

| # | Step | Exercises | Expected observation |
|---|---|---|---|
| 1 | Install the real artifact | packaging | Installs without a signature override |
| 2 | First launch from the platform launcher | bundle, activation | The app appears; on macOS confirm the Dock icon and the menu-bar item are both present and the mark is *visible* |
| 3 | Declare roots | the roots step A adds | The fail-closed refusal is escapable; the app proceeds |
| 4 | Enroll | `enroll`, deep link | Enrollment succeeds against the verification invite |
| 5 | Consent scopes | `consent_options`, `set_consent_scopes` | Scopes render and persist |
| 6 | Discover projects | `list_projects`, `set_project_mode` | The scratch project appears; mode changes stick |
| 7 | Watch a session | `list_pending`, `status` | The seeded session appears as pending |
| 8 | Preview | `preview`, `preview_body`, `preview_turns` | The preview renders the session |
| 9 | Redaction and privacy scan | redaction pipeline | Every seeded synthetic secret is found and replaced; the residual risk label matches what the fixture was built to produce |
| 10 | Consent and submit | `approve` | Submission accepted by the client |
| 11 | Server outcome | ingest | Accepted, or quarantined with a stated reason |
| 12 | Read back | `list_history`, `history_rollup` | The submission and its credit surface appear |
| 13 | Withdraw | `withdraw` | The submission is withdrawn; also the cleanup step |
| 14 | Update channel | per install method | The channel this install method uses offers the current version |

### Step 1, on Linux specifically

Install from the published flatpakref, exactly as the release notes instruct
(`.github/workflows/release-apps.yml:1083`):

```bash
flatpak install --from \
  https://storage.googleapis.com/tracecommons-flatpak/ai.tracecommons.Contributor.flatpakref
```

This is not a wrapper around a local build. The flatpakref points at the
OSTree repo under the `tracecommons-flatpak` bucket, published by
`.github/workflows/release-apps.yml:987` (bucket at `:986`). Note the repo
lives under `/repo`, not at the bucket root.

**GPG verification is part of what this step proves.** The flatpakref carries
an inline signing key, and `scripts/flatpak/publish-repo.sh` refuses to
publish a repo with no signed summary (lines 46-49), so a successful install
exercises the signature chain rather than merely fetching bytes. An install
that needed `--no-gpg-verify` is a **failure**, not a workaround.

### Step 14, per install method

The channel differs by install method, and a check that merely asks "is there
an update mechanism" finds two working ones while the contributor is stranded.
Both macOS channels are currently dead for a Homebrew install: Sparkle is
correctly disabled under Homebrew
(`macos/Sources/TraceCommonsApp/UpdateController.swift:48-49` gates on
`mode.startsUpdater`; `macos/Sources/TCUpdates/UpdatePolicy.swift:34-37`
returns `.managedByHomebrew` when Homebrew manages the bundle; `:18-21` makes
`startsUpdater` true only for `.selfUpdating`), and the cask it points the
user at has not been bumped in three releases.

| Install method | Channel under test | Pass condition |
|---|---|---|
| macOS, Homebrew cask | `brew upgrade --cask trace-commons` | The tap's cask version equals the current release and the upgrade lands it. Confirm Sparkle does *not* run. |
| macOS, direct DMG | Sparkle appcast | The updater constructs, checks, and offers the current version |
| Linux, flatpak | `flatpak update` | Resolves and verifies its GPG signature |
| Windows, appinstaller / MSIX | the `.appinstaller` feed | Advertises the current version |

A stale channel is a release blocker, not a note.

---

## Evidence rules

This repository is hash-only, label-only
([`./hash-only-logging.md`](./hash-only-logging.md)). A GUI pass strains that,
because the surfaces under test are the ones that legitimately display exactly
the material the rule excludes: the connect screen shows an invite code, the
projects screen shows filesystem paths, the preview sheet shows trace content
by design.

**Evidence is a per-step pass/fail record with hashes, labels and written
observations. It is not a transcript and not a screenshot album.**

Screenshots are permitted only for chrome-level states containing no trace
content, no path, no code and no identity:

- the roots-declaration screen in its empty state
- the refusal notice
- the Dock icon and the menu-bar item
- the done screen

For the preview, consent and connect steps the evidence is a written
assertion of what was observed, not an image.

**The screenshot check.** Before any image is committed, tick this box for it:

- [ ] Opened at full size and inspected against the permitted list above:
      no invite code, no filesystem path, no trace content, no contributor
      identity, no token, no URL bearing a credential.

"Be careful with screenshots" is not a control; the checkbox is. Note that the
existing captures under `docs/images/` are demo images, not verification
evidence, and this rule does not retroactively govern them.

---

## Cleanup

Mandatory, and its results are recorded whether or not anything was found.

1. **Withdraw every verification submission** through the client's own
   `withdraw`, backed by
   `/v1/account/traces/{submission_id}/withdraw`
   (`crates/trace-commons-server/src/bin/trace-commons-ingest.rs:6897-6900`),
   with `/v1/account/traces` (`:6888`) enumerating what remains. Through the
   contributor path rather than an admin purge, deliberately: cleanup then
   exercises the withdrawal promise instead of verifying around it.

2. **Enumerate the verification tenant's quarantined submissions and resolve
   them.** A trace assessed HIGH residual risk becomes `quarantined` with
   credit held at 0.0, pending a human
   ([`./quarantine-review.md`](./quarantine-review.md), lines 7-9). That queue
   sat at 48 with zero reviews for 71 days (`:3-4`). A verification design
   that silently added to it would be creating the exact problem it exists to
   catch.

   Record the count even when it is zero. **A campaign with unresolved
   quarantined rows does not pass.**

3. **Revoke the verification invite.** Revocation is immediate; the cache
   entry drops in the same request
   ([`./pilot-allowlist.md`](./pilot-allowlist.md), lines 121-129):

   ```bash
   curl -sS -X POST \
     -H "Authorization: Bearer $ADMIN_JWT" \
     "$INGEST_BASE/v1/admin/invites/$INVITE_HASH/revoke"
   ```

4. **Reset the throwaway accounts, containers and VM snapshots.**

---

## Verification checklist

Each bullet is one independent check. The campaign is acceptable only if all
pass.

- **All three platforms completed all fourteen steps.** A partial campaign is
  recorded as a partial campaign; there is no "mostly passed".
- **Every macOS observation came from a Finder launch**, and the running
  executable path was confirmed before each observation.
- **The Linux install completed without `--no-gpg-verify`.**
- **Every seeded synthetic secret was found by step 9**, and the residual risk
  label matched the fixture's expectation.
- **Withdrawal count recorded**, and `/v1/account/traces` shows nothing left
  for the verification identity.
- **Quarantine count recorded and reconciled**, zero or otherwise.
- **All four update-channel rows are `current`.**
- **Every defect found is filed**, with its issue or PR reference in the
  record. Defects belong to new slices, not to this campaign.
- **The invite is revoked.**
- **The record passes the gate:**

  ```bash
  ./scripts/operator/check-verification-record.sh <version>
  ```

  Expect `VerificationRecordOK: ...`. This is the same check
  `.github/workflows/release-apps.yml` runs on the tag push, so running it
  locally first means the tag does not fail on a typo.

---

## Decision after the campaign

- **All checks pass:** commit the record and tag the release.
- **A defect is found in a client:** file it, fix it in its own slice, rebuild
  the candidate, and re-run at minimum the affected platform's steps from the
  first one that could be influenced. A rebuilt artifact has a new hash, so
  the record's `artifact_sha256_*` changes with it.
- **A defect is found in the server or the pipeline:** file it. Whether it
  blocks the tag is a judgement call; record the judgement in the record's
  defect table.
- **A platform cannot be run at all** (no hardware, no VM): mark it `not-run`.
  The gate refuses, deliberately. Overriding that is a decision someone makes
  explicitly, not one that happens by default.

---

## What this does not cover

- Load, soak, or performance. One pass, one trace, one contributor.
- Correctness of components; the unit and integration suites own that.
- The CLI path — see
  [`./pilot-contributor-onboarding.md`](./pilot-contributor-onboarding.md).

## Automation status

The daemon path is portable Rust and can be driven over IPC without a GUI; a
harness that enrolls, declares roots, generates a session, previews, approves
and withdraws belongs in CI on Linux and Windows. The client path — whether
the installed bundle launches, whether the roots screen appears, whether the
menu-bar mark is visible — is manual on all three platforms.

macOS is the hardest case and should not be papered over.
`.github/workflows/ci.yml` has nine `ubuntu-latest` jobs and four
`windows-latest` jobs, and no macOS job at all; the Swift compiles only in
`.github/workflows/release-apps.yml` on a tag. Even adding a macOS runner
would not help, because a hosted runner cannot perform the Finder launch that
makes the check meaningful.

---

## Hash-only / no-secrets reminder

The pass record, its tables, and any committed screenshot are hash-only and
label-only. Artifact provenance is a SHA-256; the invite is a hash. Invite
codes, bearer tokens, admin JWTs, operator filesystem paths, trace bodies and
contributor identity do not appear in anything this procedure commits.
