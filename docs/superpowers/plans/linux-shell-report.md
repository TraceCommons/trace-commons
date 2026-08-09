# Linux contributor shell — first pass

Date: 2026-08-09
Branch: `linux-shell`
Crate: `crates/trace-commons-contributor-gtk`
Reads with: `docs/superpowers/specs/2026-08-08-contributor-shell-linux-design.md`,
`docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md`,
`docs/contributor-daemon-ipc-v1_1.md`.

## What exists

A GTK 4 / libadwaita application, in Rust, linking `trace-commons-contributor`
directly rather than through the C ABI — on this platform both sides are Rust,
so the FFI boundary would buy nothing. It builds and runs against Debian
bookworm's GTK 4.8.3 and libadwaita 1.2.2, which is deliberately an
old-but-widely-deployed pair.

Built in this order, as the brief asked:

1. **The crate and the probe.** `trace-commons-shell-probe` opens the same
   backend the window does, against a throwaway state directory holding one
   real Claude Code session, and prints `hello`, `status`, `list_pending` and
   `preview` — raw JSON and the typed rendering of both — from a real running
   daemon.
2. **A typed client layer** (`model.rs`, `backend.rs`, `worker.rs`) over
   `trace_commons.daemon.v1_1`. No type here has a field for anything the
   contract keeps off the wire, so a rendering mistake cannot put a path or a
   token on screen.
3. **The window**: Queue with the shared spec's row, and the preview sheet
   with its four tabs, Search first and focused.
4. **Notifications** with exactly `Review` and `Not now`.
5. **History and Settings.**

### The process model, both halves

The Linux spec inverts macOS: the separate daemon under the systemd user unit
is the primary deployment and this application is an optional client. Both
halves work, and the existing exclusive lock is the whole of the arbitration —
`Backend::open` checks for a running daemon, tries the lock, and falls back to
attaching if it loses the race. Nothing fails on the lock.

Which half this process is decides two user-visible things: whether the
preview sheet can show the transcript at all (see the contract gap below), and
which of the shared spec's two quit warnings is the true one. Getting the
second wrong would be a lie about whether the machine is still watching, so it
is decided at runtime rather than hardcoded per platform.

### Rules held

- **Preview-then-approve only.** The queue row has no approve button;
  `Contribute` exists only in the preview sheet. It is disabled until a real,
  pinned preview arrives, so an unenrolled illustration can never be approved.
- **The undo counts against the daemon's clock.** `approve`'s `hold_until` is
  what the countdown runs against; `hold_until: null` means no undo is offered
  at all rather than one being invented.
- **No notification action uploads anything.** `notify::Action` has two
  variants and every unrecognized D-Bus action id maps to `NotNow`.
- **No filesystem path is ever rendered.** Projects are named by
  `project_id` on the wire and `project_label` on screen.
- **Credit is a record**: no symbol, estimate, projection, date, or
  gamification; `last_refreshed_at: null` renders as "Not synced yet".
- **Quarantine reads as held, never rejected, with no turnaround time.**
- Copy lives in one module with tests asserting the rules above — that no
  health sentence names an internal mechanism, that credit copy carries no
  currency, that the quarantine text uses the word "rejected" only to deny it.

## How it was verified

Everything below was run in the container, not reasoned about. `scripts/linux-build.sh`
builds the image and mounts named volumes for the cargo registry and target
directory; `crates/trace-commons-contributor-gtk/scripts/` holds the fixture and
the three runs.

| Check | Command | Result |
|---|---|---|
| Builds | `scripts/linux-build.sh` | clean, no warnings |
| Formatting | `scripts/linux-build.sh "cargo fmt --check"` | clean |
| Lints | `RUSTFLAGS='-D warnings' cargo clippy --all-targets` | clean, no allow-list |
| Unit tests | `RUSTFLAGS='-D warnings' cargo test` | 11 passed |
| Talks to a real daemon | `scripts/linux-build.sh --probe` | real `status` / `list_pending` / `preview` JSON |
| Starts, attached | `scripts/linux-build.sh --run-headless` | window realized, queue populated, screenshot taken |
| Starts, hosting | `scripts/hosted-run.sh` | took the lock, served the body, search found a planted string: `1 match` |
| Host workspace unaffected | `RUSTFLAGS='-D warnings' cargo check -p trace-commons-contributor --bins` on macOS | clean |

**What the headless run proves, and what it does not.** Under `xvfb-run` with
a private session bus, the run proves the process starts, the widget tree is
realized on a display, the backend reaches the daemon over the socket, and the
views are populated with real daemon data — the screenshots show the queue row
with its redaction receipt, the preview sheet's four tabs with the search
result, and Settings with the real project list. It proves **nothing about
whether the layout looks right**: there is no theme beyond stock Adwaita, no
icon theme, a fixed 1280x900 frame, and nobody looking at it. Visual judgment
still needs a person on a real desktop.

Two paths could not be exercised headlessly and are untested end to end: the
**approve → undo → cancel** flow, because `Contribute` is correctly disabled
without an enrollment and enrolling needs a live issuer; and **notification
actions**, because the container has no notification daemon. Both are wired;
neither has been watched working.

## New dependencies

All confined to this crate. No existing crate gained any, and the crate is its
own workspace (excluded from the root's) so the GTK tree stays out of every
other lockfile.

Direct: `gtk4` 0.7, `libadwaita` 0.5 (feature `v1_2`), `glib` 0.18,
`notify-rust` 4, `async-channel` 2, `anyhow`, `chrono`, `serde`,
`serde_json`, `tokio` (rt-multi-thread/sync/time), `uuid`, and
`trace-commons-contributor` by path.

That resolves to **218 packages** (normal edges, no dev-dependencies) against
**135** for the contributor crate alone — the GTK/gdk/pango/cairo `-sys` tree and `zbus` (via
`notify-rust`) account for the difference. `notify-rust` is the only one that
is not a direct consequence of "GTK 4 with libadwaita, as the spec calls for";
it is the practical route to notification **actions**, which on this platform
do the work the tray menu does elsewhere.

System packages needed to build: `libgtk-4-dev`, `libadwaita-1-dev`,
`libnotify-dev`, `pkg-config`.

## Contract gaps found

These are reported rather than worked around, per the brief. Nothing under
`trace-commons-contributor` or `-ffi` was modified.

### 1. On the primary Linux deployment, the preview body is unreachable

The shared spec's preview sheet has a Search tab ("the highest-value
affordance in the product") and an "Exactly what would be sent" tab, and says
both come from the in-process preview. The IPC contract deliberately serves
only a **summary** over the socket, on the rationale that "a native app that
wants the actual body should call the C ABI's local preview entry point
directly".

That rationale holds on macOS and Windows, where the app hosts the daemon. It
does not hold on Linux, where the Linux spec makes the separate systemd daemon
the *primary* deployment and the app a client. In that configuration the app
cannot obtain the body at all:

- `ipc::open_preview` needs a `&DaemonShared`, which only the lock-holder has.
- Building a second `DaemonShared` in the app is not a workaround:
  `DaemonShared::load` rewrites `daemon-queue.jsonl` (`release_in_flight`) and
  sweeps stored preview envelopes, which is precisely the two-processes-editing-
  the-same-files corruption `daemon::client` exists to prevent.
- The app cannot preview the file itself either — it never learns the path,
  by design.

So a GNOME contributor running the recommended systemd unit gets a preview
sheet whose two most valuable tabs cannot work. The shell currently says so
plainly instead of showing an empty box, and the search tab refuses to report
a reassuring `0 matches` it has not earned.

**What would close it:** a `preview_body`-style method on the socket, subject
to the same bounds the existing exemption already states (post-redaction only,
only for an entry the caller holds, never onward into a log or receipt) — the
exemption's reasoning already covers the content; only the transport is
missing. That is a contract change, so it is not made here.

### 2. The queue row's mandated line costs a full preview per entry

The row is specified to carry `Would send 84 KB · scrubbed: …`, the redacted
opening prompt, and a turn count. None of those are on the queue entry; all
come from `preview`, which runs the redaction pipeline, pins an envelope,
writes it to disk, and (under an external scanner) makes a network call. A
500-entry queue would mean 500 of those to draw a list.

The shell previews the first 12 pending rows and fills the rest when opened;
rows without one yet read "Checking what would be sent…". A cheap
summary-only variant, or these three fields on the queue entry, would remove
the compromise.

### 3. "What's in it" cannot be built from the contract

The tab is specified as files touched (redacted), tools invoked with counts,
model, and turn count. `PreviewSummary` carries none of the first three. The
tab currently shows what the contract does give — turns, would-send versus
raw size, the redaction receipt, PII categories present, residual risk.

### 4. History → Withdraw has no method

The shared spec makes withdraw first-class and always available, and names it
twice. `trace_commons.daemon.v1_1` has no withdraw method, and the CLI's only
`--withdraw` is on `profile`, which withdraws public attribution rather than a
trace. The quarantine section says where the capability is instead of drawing
a button that cannot work.

## Not done, and knowingly so

- **Onboarding** (six screens), **Flatpak packaging**, and **XDG Background
  portal registration** — out of scope for this pass, as the brief set it.
  The portal one matters more than it sounds: it is where a GNOME user looks
  for a background app, and it doubles as the pause/quit surface for anyone
  who never sees a tray.
- **The `StatusNotifierItem` tray.** A bonus by the spec's own framing, and
  the app is fully usable without it, which was the requirement. Nothing in
  the app mentions shell extensions.
- **Autostart.** Neither path (detecting the systemd unit, nor an XDG
  autostart entry) is wired. The spec's "never both" rule is the thing to
  hold when it is.
- **`acknowledge_near_ai_notice`.** The health banner renders the
  `near-ai-notice-not-acknowledged` sentence and its `Review and confirm`
  button, but the button is not yet wired to show the disclosure text and make
  the call. It must not be wired without showing that text first — the call
  asserts, on the caller's unverified word, that a human saw it.
- **Consent scope editing** from Settings (`consent_options` /
  `set_consent_scopes`). Scopes are shown at the moment of consent in the
  preview sheet; changing them is still CLI-only.
- **Recent searches do not persist across restarts.** They persist for the
  session, which delivers the stated benefit (the second trace is one click).
  A search term is the contributor's own sensitive string — usually a client
  name — and writing those to disk deserves a decision, not a default.
