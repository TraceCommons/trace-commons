# Staging tester guide

This guide is for people testing the Near.ai Staging app. Staging is a test
environment. It is not the released app, and it is not the real service.

Read "What works and what does not yet" before you start. Some parts are not
built yet.

## What you need

- A Mac.
- A NEAR AI staging account (see below).
- The staging build. Ask the owner for the link, or find it on the
  [releases page](https://github.com/TraceCommons/trace-commons/releases).
  Staging builds are marked "Pre-release". Names look like
  `staging-<date>-<commit>-<run>`. The file is called
  `Near.ai-Staging-<version>-staging.<run>.dmg`.

## Make a NEAR AI staging account

1. Go to https://cloud-stg.near.ai.
2. Sign up with Google. The staging app signs in with Google only, so an
   account made with GitHub cannot sign in.

This account is separate from your production NEAR AI account, even if you
use the same Google login. A production account does not work in
staging, and a staging account does not work in production.

You do not need an invite. Anyone with a staging NEAR AI account can sign up
to the staging service.

## Install

1. Download the `.dmg` from the pre-release.
2. Optional: check the download (see below).
3. Open the `.dmg` and drag Near.ai Staging to Applications.
4. Open it.

The build is signed with a Developer ID and notarized by Apple.

The staging app installs beside the released Near.ai app. They do not share
anything: not the sign-in, not the keychain items, not the data folder, not
the updates, and not the `tracecommons-staging://` link scheme. You can keep
both.

The staging app shows "Staging" in its windows, so you can tell it apart.

## Check the download (optional)

Each pre-release has a `.sha256` file next to the `.dmg`. Download both into
the same folder and run:

```bash
shasum -a 256 -c Near.ai-Staging-*.dmg.sha256
```

It should print `OK`. If it does not, delete the download and tell us.

You can also check that macOS accepts the app. Drag it to Applications, then:

```bash
spctl --assess --type execute --verbose "/Applications/Near.ai Staging.app"
```

It should say `accepted` and name the Notarized Developer ID source.

## First run

1. Sign in with your staging NEAR AI account. The browser opens a Google
   sign-in; use the Google account you made it with.
2. Add a passkey if the app asks. A staging passkey works for the staging
   site only (`staging.tracecommons.ai`).
3. Choose which session folders the app may watch.

Do not add a folder with a session you would not want a test server to see.
Staging is run with the same care as the real service, but it is a place for
testing, and sign-up is open to anyone with a staging NEAR AI account.

## Updates

The staging app updates itself. It checks a staging-only update feed and
accepts only builds signed with a staging-only key. It never takes an update
meant for the released app, and the released app never takes a staging one.

New staging builds can arrive often.

## What works and what does not yet

As of 2026-10-22.

Works:

- Installing the staging app beside the released app.
- Updating the staging app through its own feed.
- The "Staging" label, the separate keychain, state folder and link scheme.
- Making a NEAR AI staging account.
- The staging app refuses production hosts and production state folders.

Does not work yet:

- **Our staging server is not deployed yet.** Until it is, signing in to our
  service, uploads and credit settlement do not work. You may be able to open
  the app, but you cannot finish sign-in. We will say here when this changes.
- **Private AI is off.** Attested NEAR AI inference stays off until we serve a
  signed set of measurements to check it against. That needs two things that
  are not done yet.

If something not listed here breaks, report it (see below).

## What to expect when NEAR AI staging changes

NEAR AI staging redeploys after each change to its code. When it does:

- You may be signed out for a moment. Sign in again.
- Private AI proof marks may stop until we re-sign the measurements. If they
  stay off, the app's status says why.

## Report a problem

Where to report is not decided yet (TBD). It is open item 6 in the
[staging design](../superpowers/specs/2026-10-21-staging-environment-design.md).
Until it is decided, send reports to the person who gave you the link.

Include:

- The app's status label.
- The time it happened.
- If you were signed out, or Sign in again did not work: the account-session
  renewal log, below.

### The account-session renewal log

The app renews its 12-hour commons session by itself, and the person's Sign
in again runs the same step. Each attempt adds one line to
`account-session-renewal.jsonl` in the data folder (the last 200 are kept):

```bash
cat ~/Library/Application\ Support/trace-commons-staging/account-session-renewal.jsonl
```

(`~/Library/Application Support/trace-commons/` for the production app.)
Each line has `at`, `trigger` (`automatic` or `sign_in`), `outcome`
(`renewed` or `failed`) and, on a failure, `failure`, one of:

| `failure` | Meaning |
| --- | --- |
| `near_ai_sign_in_required` | NEAR AI refused the stored login. Sign in again signs in to NEAR AI first. |
| `near_ai_unavailable` | NEAR AI could not be reached or answered something else. |
| `commons_start_failed` | The commons refused or did not answer the ceremony's start. |
| `commons_refused` | The commons refused the ceremony's finish. |
| `commons_answer_mismatch` | The commons answered for another account or tenant; nothing was stored. |
| `commons_address_refused` | The enrolled commons address failed the signup allowlist. |
| `local_check_failed` | A local check refused the ceremony or its answer (for example a clock far off). |
| `storage_unavailable` | The session could not be stored. |
| `unavailable` | Anything else. |

The lines carry no token, URL, account id or tenant, so the file is safe to
send. `account_session_status` reports the same class as `renewal_failure`
while the 10-minute retry wait holds.

Do not send a trace body, a key, an organization id or a screenshot of a real
session.

## Privacy

- Staging is a test environment. Do not treat it as private storage.
- Sign-up is open to any staging NEAR AI account. We cannot vet who else is
  using it.
- If you let the staging app watch a folder, a session in it can be sent to
  the staging server. If the released app watches the same folder, the same
  session can go to both. Pick your folders with that in mind.
- Staging credit is worth nothing in the released service.
- Staging logs follow the same hash-only rules as the real service.

## Remove it

1. Quit Near.ai Staging.
2. Delete Near.ai Staging from Applications.
3. Delete the data folder `~/Library/Application Support/trace-commons-staging`,
   if you want your staging state gone. The released app's folder,
   `trace-commons` beside it, is separate; leave it alone.
4. Delete the Near.ai Staging items from Keychain Access, if you want your
   staging keys gone.

None of this touches the released app.
