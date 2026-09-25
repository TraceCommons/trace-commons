# First-run glass flows (FTUX) — implementation report

Date: 2026-09-25
Branch: `ftux`
Spec: `docs/superpowers/specs/2026-09-25-ftux-glass-flows-design.md`
Scope: `tauri-desktop/frontend` only. No daemon, server, Rust or CI change.

## Summary

The Connect and forget, Customize and tailor, and Create a passkey flows from
the WYSIWYG design are built as a new feature module,
`src/features/ftux/`, reachable at `#/ftux`. Every backend call is mocked in
one file. The existing onboarding and its gate are unchanged.

## How to see it

The flow opens at its own address, `#/ftux`, in the Tauri app or `pnpm dev`.
Two options: `#/ftux?path=customize` starts on Customize and tailor, and
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
| `src/features/ftux/ftux-model.test.mjs` | 10 tests for the above. Added to the `test` script. |
| `src/features/ftux/types.ts` | Shapes for detected tools, repos, sessions, join state and the final settings. |
| `src/features/ftux/api/ftux-api.ts` | The only doorway to the outside world. Mocks with short delays; header lists the real command for each. |
| `src/features/ftux/api/ftux-mock-data.ts` | Mock tools, repos and sessions, seeded with the design's own examples. |
| `src/features/ftux/ftux.css` | The glass style, transcribed from the design and scoped under `.ftux`. |
| `src/features/ftux/components/glass.tsx` | Window, stepper, title, pill select, checkbox (with mixed state), switch, spinner, and the design's icons. |
| `src/features/ftux/components/join-screen.tsx` | W-1. |
| `src/features/ftux/components/tool-row.tsx`, `tool-screens.tsx` | W-2 Folders and W-4 Tools. |
| `src/features/ftux/components/rules-screen.tsx` | W-5. |
| `src/features/ftux/components/uses-screen.tsx` | W-3 and W-6. |
| `src/features/ftux/components/passkey-flow.tsx` | P-1 to P-6 as one component per popup, plus P-7 `WelcomeBack`. |
| `src/features/ftux/ftux-page.tsx` | Holds the flow state and wires the screens to the API. |
| `src/features/ftux/ftux-preview-route.tsx`, `index.ts` | The `#/ftux` route and its query options. |
| `src/features/ftux/ftux-page.stories.tsx` | Storybook stories. |
| `src/app/main.tsx` | Adds the `/ftux` route beside the app shell. |
| `package.json` | Adds the model test to `pnpm test`. |

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
- **One scoped stylesheet** rather than Tailwind utilities. The design's
  layered gradients, inset shadows and backdrop filters are long and repeated;
  `.ftux-*` classes keep the components readable and cannot leak into the rest
  of the app.
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

See the spec for the deliberate differences from the design and the open
questions, the first of which (the *Share automatically* default against #507
and #991) must be settled before `finishSetup` is wired.

## Verification

Run in `tauri-desktop/frontend`:

```
$ pnpm test
ℹ tests 28
ℹ pass 28
ℹ fail 0

$ pnpm build
✓ built in 371ms

$ pnpm build-storybook
Storybook build completed successfully
```

`pnpm build` runs `tsc --noEmit` first, which passes. `npx biome check
src/features/ftux` reports no errors and 4 CSS specificity warnings. Biome is
not run in CI, and `main` already has Biome errors elsewhere.

Each screen and popup was walked in the browser at `#/ftux`,
`#/ftux?path=customize` and `#/ftux?returning=1` and compared with the design:
the full Create new passkey path (P-1 to P-5) back to Join, invite lookup,
Continue gating on Folders and Tools, adding a tool, the Never rule changing the
count from "7 of 43" to "4 of 21", the Private AI switch, and *Start sharing*
returning to `#/insights`. No console errors.

The Rust side of the Tauri app is unchanged, so `cargo` checks were not rerun.
