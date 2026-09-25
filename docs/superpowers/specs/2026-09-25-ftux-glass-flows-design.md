# First-Run Flows in the Glass Style (FTUX) — Design

Date: 2026-09-25
Status: front end implemented on mock data; backend wiring open
Source design: claude.ai/design project `0935cc40-8e63-454f-848f-7f4bf5995b26`,
file `WYSIWYG.dc.html` (Flow 1 · First run, and "f1 · passkey"). Built from a
local export of that file dated 2026-09-25.
Related: [`2026-09-23-connect-and-forget-consent-design.md`](2026-09-23-connect-and-forget-consent-design.md) (#991),
[`2026-08-31-contributor-trust-by-default-design.md`](2026-08-31-contributor-trust-by-default-design.md) (#507),
[`2026-06-22-contributor-account-passkeys-slice2-design.md`](2026-06-22-contributor-account-passkeys-slice2-design.md)
Scope: `tauri-desktop/frontend` only. No daemon, server or Rust change.
Implementation report: [`../plans/ftux-glass-flows-report.md`](../plans/ftux-glass-flows-report.md)

## What this is

The WYSIWYG design describes Trace Commons as one glass window, and its
Flow 1 is first-run setup (Epic 1, Environment config) in two tiers:

- **Connect and forget**: Join, Folders, Uses. Three screens.
- **Customize and tailor**: Join, Tools, Rules, Uses. Four screens.

Both start from the same Join screen. A secondary flow, **Create a passkey**,
opens from Join as a stack of popups (P-1 to P-7) and returns to Join with the
passkey card marked done.

This spec records what the Tauri client now draws for those flows, the rules
the screens enforce, and what is still mocked.

## Rules carried from the design

1. **Saying no is an answer.** On Folders (and Tools), Continue stays off until
   every tool found on this machine has an answer: *Watch this folder* or
   *I don't use it*. A tool not found on this machine is not asked; its row
   says "Install it, then this row asks again."
2. **Joining never authorizes sharing.** Join states this in a fixed note.
   Invite, passkey and near.ai sign-in are all optional; the primary button
   reads *Skip* until one of them succeeds, then *Continue*.
3. **Finding bugs and measuring agents is always on.** It is shown checked and
   disabled. The three optional uses sit in a collapsed group so the always-on
   use stays visible.
4. **A folder set to Never contributes nothing,** including its past sessions:
   choosing Never clears that folder's picks and removes it from the
   "N of M selected" count.
5. **Private AI is off until turned on,** and appears only in Customize and
   tailor.
6. **Cancelling passkey verification signs out** rather than leaving a
   half-linked account. Closing the stack earlier returns to Join unchanged.

## Screens

| Id | Screen | Tier | Notes |
|---|---|---|---|
| W-1 | Join | both | Invite link + Look up, Create passkey, Sign in with near.ai |
| W-2 | Folders | connect | Per-tool answer, folder override, *Customize instead* |
| W-4 | Tools | customize | As W-2 plus "Not seeing your tool above?" (click or drag) |
| W-5 | Rules | customize | Rule per repo (Ask me / Share automatically / Never); past sessions by folder |
| W-3 / W-6 | Uses | both | Uses, handle listing, Sharing; W-6 adds Private AI |

*Customize instead* on Folders switches tiers and lands on Tools, the
equivalent screen. The stepper shows done, current and upcoming steps for the
active tier.

### Passkey popups

| Id | Popup | Drawn by |
|---|---|---|
| P-1 | Continue with passkey: Use existing / Create new | app |
| P-2 | Create new passkey: name it, loss warning | app |
| P-3 | Save a passkey? choose 1Password or Passwords | macOS (imitated) |
| P-4 | Touch ID to Save Passkey | macOS (imitated) |
| P-5 | Verify your passkey (binds to near.ai) | app |
| P-6 | Sign In with a stored passkey | macOS (imitated) |
| P-7 | Welcome back (returning user) | app |

Create new runs P-1, P-2, P-3, P-4, P-5. Use existing runs P-1, P-6. A
returning user starts at P-7, whose *Sign in with passkey* opens P-6.

P-3, P-4 and P-6 are macOS's own sheets in the shipped app, raised by the
WebAuthn request. Until WebAuthn exists the app draws imitations so the flow
can be walked end to end.

## How to see it

The flow opens at its own address, `#/ftux`, in the Tauri app or `pnpm dev`.
Two options: `#/ftux?path=customize` starts on Customize and tailor, and
`#/ftux?returning=1` opens the returning-user "Welcome back" card. The
existing onboarding is untouched and still decides who sees setup. Finishing
the new flow saves nothing and just returns to the app, so it can't mark anyone
as set up for real.

Storybook has one story per screen and per popup under
`Features/FTUX/FtuxPage`.

## What works

- **Connect and forget:** Join, Folders, Uses. Continue stays off until every
  tool found on the Mac has an answer, and "Customize instead" switches to the
  other flow.
- **Customize and tailor:** Join, Tools, Rules, Uses.
  - Tools adds the "add your tool" box, by click or drag.
  - Rules sets a rule per repo and lets you pick past sessions by folder, with
    the "7 of 43 selected" count. Setting a folder to Never clears its
    sessions.
  - Uses adds the Private AI switch, off by default.
- **Create a passkey:** the popups P-1 to P-7 in order.
  - Cancelling at Verify signs you out and shows a note.
  - A finished passkey marks the card done, and the Join button changes from
    Skip to Continue.
  - The "Save a passkey?" and "Sign In" macOS sheets are imitations. In the
    real app macOS draws its own.

## Mocked for later

Everything that needs the backend goes through one file,
`tauri-desktop/frontend/src/features/ftux/api/ftux-api.ts`. Its header lists
the real command to swap in for each. The mocked parts are:

- checking an invite with its issuer and the pay range it returns (reading the
  link itself is real)
- finding tools on the Mac, and the repos and past sessions
- creating, verifying and signing in with a passkey
- near.ai sign-in
- saving the choices at the end

| Mock | Real replacement | State of the backend |
|---|---|---|
| `lookupInvite` | `enroll_with_invite` | exists |
| `detectTools` | daemon source detection | not built |
| `listRepoCandidates` | `list_projects` plus a per-session listing | partly built |
| `createPasskey`, `verifyPasskey`, `signInWithPasskey` | WebAuthn for tracecommons.ai | not built (see the Slice 2 passkeys spec) |
| `signInWithNearAi` | `account_sign_in` | exists |
| `finishSetup` | `set_source_declaration`, `set_project_mode`, `set_consent_scopes`, Private AI | exists except per-session selection |

Two calls are already real: *Choose a different folder* uses `pick_directory`,
and *Get Codex* uses `open_external_url` (both fall back to browser behaviour
outside Tauri). The invite link is parsed with the existing `resolveInvite`.

## Deliberate differences from the design

- **Claude Code starts unanswered** rather than pre-set to *Watch this folder*.
  The design's own rule is that saying no is an answer; a pre-selected Watch
  leans toward sharing.
- **No traffic-light buttons** in the card. The native window already has them.
- **A folder with some sessions picked shows a half-filled (mixed) checkbox**
  rather than an empty one.
- Session dates and durations use the design's format ("Sat 12 Sep",
  "1 h 18 min", "2 h 04 min") and are read in UTC so a session never shifts a
  day with the time zone.

## Open questions before wiring the backend

1. **Default sharing mode.** The design defaults Sharing to *Share
   automatically* and offers *Share automatically* as a per-repo rule during
   setup. #507 keeps onboarding ask-first and does not offer `auto_upload`
   there, and #991 gates automatic contribution behind conditions that are
   still open. The mock saves nothing, so there is no conflict today, but
   `finishSetup` must not arm automatic contribution until this is settled.
2. **Replacing the current onboarding.** `#/ftux` sits beside the real
   onboarding gate. Swapping it in needs `finishSetup` wired and the
   completion flag (`useOnboardingCompletion`) set only after the real calls
   succeed.
3. **Past-session selection** has no backend. Rules would need a per-session
   listing and a way to contribute a chosen subset.
4. **Other sign-in options** from P-7 and *More Options* from P-6 currently
   return to Join / P-1. Where they should lead (near.ai login, another
   passkey) is not specified beyond the design's flow notes.
5. **Dropped tools.** A drag-and-drop in a Tauri webview does not expose file
   paths to the DOM. The mock uses the dropped item's name; the real version
   needs Tauri's drag-drop event.
