# First-run glass flows (FTUX) — implementation report

Date: 2026-09-25 (updated 2026-09-28 and 2026-09-29 after review; 2026-09-30 moved onto the #1146 design system)
Branch: `ftux`
Spec: `docs/superpowers/specs/2026-09-25-ftux-glass-flows-design.md`
Scope: `tauri-desktop/frontend` only. No daemon, server, Rust or CI change.

## Summary

The Quick setup and Custom setup flows (the design's "Quick setup" and
"Custom setup", renamed; see the spec) and the Create a passkey flow
from the WYSIWYG design are built as a new feature module,
`src/features/ftux/`, reachable at `#/ftux`. Every backend call is mocked in
one file. The existing onboarding and its gate are unchanged.

## How to see it

The flow opens at its own address, `#/ftux`, in the Tauri app (`tauri dev`) or `pnpm dev`. The route is registered only in development builds or with `VITE_FTUX_PREVIEW=1`, so a release build does not contain it.
Two options: `#/ftux?path=custom` starts on Custom setup, and
`#/ftux?returning=1` opens the returning-user "Welcome back" card. The
existing onboarding is untouched and still decides who sees setup. Finishing
the new flow saves nothing and just returns to the app, so it can't mark anyone
as set up for real.

```bash
pnpm --dir tauri-desktop/frontend dev
```

Then open `http://localhost:5173/#/ftux`. Storybook (`pnpm storybook`) has one
story per screen and popup under `Features/FTUX/FtuxPage`.

## Files

All paths are under `tauri-desktop/frontend/`.

| File | Role |
|---|---|
| `src/features/ftux/ftux-model.ts` | Pure logic: step order per tier, tier switching, the "every tool answered" gate, optional-uses label and group toggles, past-session counts and the Never rule, the passkey popup transition table, name validation, date and duration formatting. No imports, so `node --test` loads it directly. |
| `src/features/ftux/ftux-model.test.mjs` | 12 tests for the above, including the setup guards. Added to the `test` script. |
| `src/features/ftux/types.ts` | Shapes for detected tools, repos, sessions, join state and the final settings. |
| `src/features/ftux/api/ftux-api.ts` | The only doorway to the outside world. Mocks with short delays; header lists the real command for each. |
| `src/features/ftux/api/ftux-mock-data.ts` | Mock tools, repos and sessions, seeded with the design's own examples. |
| `src/features/ftux/ftux.css` | Flow layout and the imitation macOS sheets only, on `--tc-*` tokens. Everything else comes from `src/design-system/` (#1146). |
| `src/features/ftux/components/ftux-frame.tsx` | The window (`Window` + `Pane` + `StepProgress`), title, scroll body, footer and status line. |
| `src/features/ftux/components/icons.tsx` | Icons the design system does not carry, and the spinner. |
| `src/features/ftux/components/join-screen.tsx` | W-1. |
| `src/features/ftux/components/tool-row.tsx`, `tool-screens.tsx` | W-2 Folders and W-4 Tools. |
| `src/features/ftux/components/rules-screen.tsx` | W-5. |
| `src/features/ftux/components/uses-screen.tsx` | W-3 and W-6. |
| `src/features/ftux/components/private-ai-card.tsx` | The Private AI card, rendered from the shared `private_inference` copy. |
| `src/features/ftux/components/passkey-flow.tsx` | P-1 to P-6 as one component per popup, plus P-7 `WelcomeBack`. |
| `src/features/ftux/ftux-page.tsx` | Holds the flow state and wires the screens to the API. |
| `src/features/ftux/ftux-preview-route.tsx`, `index.ts` | The `#/ftux` route and its query options. |
| `src/features/ftux/ftux-page.stories.tsx` | Storybook stories. |
| `src/app/main.tsx` | Adds the `/ftux` route beside the app shell, in development builds or with `VITE_FTUX_PREVIEW=1` only. |
| `package.json` | Adds the model test to `pnpm test`. |

## What works

- **Quick setup:** Join, Folders, Uses. Continue stays off until every
  tool found on the Mac has an answer, and "Custom setup instead" switches to the
  other flow.
- **Custom setup:** Join, Tools, Rules, Uses.
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

Everything that needs the backend goes through one file, `ftux-api.ts`. Its
header lists the real command to swap in for each. The mocked parts are:

- checking an invite with its issuer and the pay range it returns (reading the
  link itself is real)
- finding tools on the Mac, and the repos and past sessions
- creating, verifying and signing in with a passkey
- near.ai sign-in
- saving the choices at the end

*Choose a different folder* (`pick_directory`) and *Get Codex*
(`open_external_url`) already use real commands inside Tauri.

## Design decisions

- **Separate route, not a replacement.** Replacing the onboarding gate with a
  mocked flow would let someone finish setup without enrolling. `#/ftux` sits
  outside the gate until `finishSetup` is real.
- **The #1146 design system, not a private copy.** The first version carried
  its own glass stylesheet transcribed from the design. Since #1146 lands the
  same design as `src/design-system/`, the screens now use its components and
  tokens; `ftux.css` keeps only layout and the imitation macOS sheets. See the
  spec's "Design system" table.
- **System font** (`-apple-system`, SF Pro) inside `.ftux`, as the design uses,
  rather than the app's IBM Plex.
- **Passkey steps as a table** (`PASSKEY_TABLE` in the model): each step lists
  where each event leads and where anything else (Cancel, Close, Escape) leads.
  The component only renders the current step.
- **Accessibility:** popups are `role="dialog"` with `aria-modal`, move focus
  to their first control, and close on Escape. The checkbox and switch are
  buttons with `role="checkbox"` / `role="switch"` and `aria-checked`, because
  a native checkbox cannot draw the glass box or the mixed state. Selects are
  native `<select>` elements. Motion is removed under
  `prefers-reduced-motion`.

## Review fixes (2026-09-28)

Review on #1030 found one failing test and nine other issues. All ten are
addressed:

| # | Finding | Fix |
|---|---|---|
| 1 | "Private Inference" on the Uses screen fails `tauri_never_says_private_inference_to_a_contributor` | Sentence removed; the test passes |
| 2 | Private AI text written in the file and contradicting `OFFER_NO_REPOINT`; `OFFER_EXPOSURE` missing | `PrivateAiCard` renders `destination`, `offer_what`, `offer_exposure`, `offer_no_repoint` from the shared copy; switch disabled until it loads |
| 3 | Defaults arm automatic contribution | Sharing starts at Ask me, optional uses off, no repo on Share automatically, no session ticked; `finishSetup` refuses `automaticChoices` |
| 4 | Quick setup sends unseen repos; repos not filtered by tool answers | Quick setup sends no repos; `reposForWatchedTools` filters Rules; empty state when nothing is watched |
| 5 | `aria-modal` without a real modal | Background `inert`, Tab trapped, focus restored to the opener |
| 6 | Imitation macOS sheets unmarked and shipped in release | Each sheet carries a Simulated mark; preview tag drawn above popups; route dev-only or `VITE_FTUX_PREVIEW=1` |
| 7 | Verify cancel leaves other sign-ins shown | Sign-out clears invite, near.ai and passkey |
| 8 | P-6 names a passkey that may not exist | P-6 names the stored passkey when known, otherwise asks generically |
| 9 | Drop records a bare name and sets it to watch | A drop opens the picker; added tools start unanswered |
| 10 | No `prefers-reduced-transparency` / `prefers-contrast` | Solid surfaces without blur, and brighter text with firmer edges, respectively |

See the spec for the deliberate differences from the design and the open
questions, the first of which (the *Share automatically* default against #507
and #991) must be settled before `finishSetup` is wired.

## Consent-spec decisions (2026-09-29)

The reviewer's second comment on #1030 set how the flows line up with #991
rev 8. Applied here:

| # | Decision | Change |
|---|---|---|
| 1 | "Connect and forget" names the consent path | Tiers renamed Quick setup / Custom setup (`FtuxPath` is now `"quick" \| "custom"`, `?path=custom`) |
| 2 | Sharing is a moment of consent | Picker kept; *Share automatically* still refused, with a message saying the disclosures and grant are not connected yet. Wiring deferred to the stacked PR |
| 3 | The core chooses the sharing copy | Sharing card renders `useAutomaticGrantCopy`; Start disabled until it loads |
| 4 | Uses are the scope and come first; the floor use is required, unticked | `baseUse` starts unticked; Start disabled until ticked; `finishSetup` refuses without it |
| 5 | Private AI is a consent event | Deferred to the stacked PR |
| 6 | near.ai is the account; Skip leads to watching only; passkey card may be hidden | *Skip: watch only* with a note; passkey card hidden behind `showPasskey` (story `JoinWithPasskeyCard` shows it) |

## Moving onto the design system (2026-09-30)

`ftux` merged #1146's branch (`claude/tc-monitor-frontend-refactor-8ecf22`)
and #1030 is stacked on it, so the diff against that branch is the FTUX alone.
`components/glass.tsx` is gone. Every screen now uses the design system's
components. `ftux.css` went from about 1,100 lines to about 400.

Checked in the browser against the previous screens:
- Join, Folders, Rules and Uses on Quick and Custom setup.
- The returning-user card, the Sign In sheet with its Simulated mark, and Create new passkey.

Fixed during that check:
- The design system's `.tc-root h2` rule was shrinking the popup headings.
- The loss warning's icon was stacking above its text.

On 2026-09-30 the flow also dropped `tc-alert` and its coloured left edge.
Notices and errors are now `Notice`: a quiet design-system card with a
status dot or icon and a label. The same cleanup for the rest of the app is
being done in #1146.

## Verification

Run in `tauri-desktop/frontend`:

```
$ pnpm test            # after merging main on 2026-09-28
ℹ pass 96
ℹ fail 0

$ cargo test -p trace-commons-contributor-ffi --test tauri_copy_surface_is_central
test result: ok. 4 passed; 0 failed

$ pnpm build
✓ built in 371ms

$ pnpm build-storybook
Storybook build completed successfully
```

`pnpm build` runs `tsc --noEmit` first, which passes. `npx biome check
src/features/ftux` reports no errors and 4 CSS specificity warnings. Biome is
not run in CI, and `main` already has Biome errors elsewhere.

Each screen and popup was walked in the browser at `#/ftux`,
`#/ftux?path=custom` and `#/ftux?returning=1` and compared with the design:
the full Create new passkey path (P-1 to P-5) back to Join, invite lookup,
Continue gating on Folders and Tools, adding a tool, the Never rule changing the
count from "7 of 43" to "4 of 21", the Private AI switch, and *Start sharing*
returning to `#/insights`. No console errors.

A production `pnpm build` was searched for the preview's strings and does not
contain them; a build with `VITE_FTUX_PREVIEW=1` does.

After the review fixes, the browser check confirmed:
- the window is inert while a popup is open;
- Tab wraps inside the popup;
- focus returns to "Create passkey";
- P-6 asks generically, and the Simulated mark is present;
- Rules shows its empty state when no tool is watched;
- Uses starts at Ask me with all optional uses off, and the Private AI switch is disabled without the copy;
- choosing Share automatically is refused with a message.

The Rust side of the Tauri app is unchanged.
