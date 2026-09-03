# Imported conversations reach the desktop apps — design

**Status:** proposed
**Date:** 2026-09-02
**Follows:** #516 (Antigravity import), #519 (`declared_source` in the CLI)

## The problem

`import-antigravity` works, and its output is invisible to every desktop app.

A contributor who runs the import gets Trajectory-v1 files staged in
`<state dir>/trajectories`. They can then `list` and `submit` them from the
CLI. If they instead open the macOS, GTK or Windows app, they see nothing:
not a queued entry, not an error, not an empty state that mentions
Antigravity. The conversations do not exist as far as the apps are concerned.

That is the same silent shape as the missing macOS Gemini row (#518): every
mechanism that would normally surface it is one that was correctly designed
to stay quiet.

### Three gaps, verified on `origin/main` at 41baf1bf

**1. The daemon never scans the staging directory.**
`DaemonSettings::source_roots()` declares three adapters and no trajectory
selection at all:

```rust
crate::source::SourceRoots::new()
    .declare(SOURCE_CLAUDE_CODE, self.claude_source.clone())
    .declare(SOURCE_CODEX, self.codex_source.clone())
    .declare(SOURCE_GEMINI_CLI, self.gemini_source.clone())
```

Every daemon call site — `watcher.rs`, `ipc.rs`, `preview_scheduler.rs` —
uses that method unmodified. The only caller that adds a trajectory scope is
`cli_source_roots`. So the CLI sees staged imports and the daemon does not.

**The stated reason covers only half of what it excludes.** The method's doc
says: *"a daemon's working directory is whatever a service manager handed it,
so auto-discovery would mean nothing there."* That is correct for the
**working-directory** half of `TrajectorySelection::Auto`. The other half is
the **staging directory**, which is not the working directory at all: it is a
fixed path under the contributor's own state directory, resolved through
`ConfigStore`, created at `0700`, and cleared by `logout`. Nothing in the
stated reason applies to it. This reads as one argument applied to two
things rather than a considered exclusion of both.

**2. Even if discovered, the apps would call it the wrong thing.**
`QueueEntry.source` is set from `session_ref.source` — the *adapter* —
in `watcher.rs:550`, and surfaces over IPC as `"source": e.source`
(`ipc.rs:660`). Each shell maps that string to a label:

| Shell | Site |
|---|---|
| GTK | `model.rs:130` |
| macOS | `Models.swift:84` |
| Windows | `QueueEntryViewModel.cs:146` |

An imported conversation would arrive as `trajectory` and be shown as
"Trajectory". The `declared_source` added in #519 stops at the CLI's own
tables; it never reaches the queue, so it never reaches an app.

**3. No app can trigger an import.** Grepping `macos/`, `windows/`,
`crates/trace-commons-contributor-gtk` and `crates/trace-commons-contributor-ffi`
for "antigravity" returns nothing. There is no FFI entry point.

## Decisions

**Scope: (1) and (2). Not (3).** The import stays a thing a person types.
This spec removes the dead end — imported conversations become visible,
previewable and submittable from the apps — without moving process
enumeration and IDE discovery behind the daemon/app boundary that #516
deliberately kept them out of.

**Consent: the import is the consent.** No new roots row, no settings
toggle. The contributor ran `import-antigravity`; the files are in their own
`0700` state directory; `logout` clears them. This matches the rule the
staging directory already documents — *"Placing a file there IS the opt-in,
which is why nothing in it needs a name suffix"* — and it is why the CLI
already reads staging with no declaration.

This does not weaken the fail-closed-roots rule. That rule governs scanning
locations the contributor did not name — a real `~/.claude`, a real
`~/.gemini`. The staging directory is created by this application, holds only
what this application put there on an explicit command, and is not a place
anybody else's traces can appear.

## Design

### 1. The daemon gains a staging-only trajectory scope

`DaemonSettings::source_roots` takes the `ConfigStore` and adds one scope:

```rust
pub fn source_roots(&self, store: &ConfigStore) -> crate::source::SourceRoots {
    crate::source::SourceRoots::new()
        .declare(SOURCE_CLAUDE_CODE, self.claude_source.clone())
        .declare(SOURCE_CODEX, self.codex_source.clone())
        .declare(SOURCE_GEMINI_CLI, self.gemini_source.clone())
        .with_trajectory(TrajectorySelection::Auto {
            // Still None, and for the reason this method has always given:
            // a service manager's working directory means nothing.
            working_dir: None,
            staging_dir: Some(store.dir().join(TRAJECTORY_STAGING_SUBDIR)),
        })
}
```

The signature changes rather than a second method being added, because this
method's own doc gives the reason to keep it: *"built from them here, in one
place, so adding an adapter does not touch the daemon, the watcher, the
preview scheduler or the CLI."* A parallel `source_roots_with_staging` would
be exactly the second place that doc exists to prevent.

Every daemon call site already has the store — `DaemonShared.store` is a
`ConfigStore` — so each becomes `s.source_roots(&shared.store)`.

**No watcher registration is required for correctness.** The daemon's tick
sweeps `source.discover()` across `all_sources(&source_roots)`
(`watcher.rs:122`), so a staged file is found on the next tick without any
filesystem-event plumbing. Discovery latency is therefore the poll interval
plus the existing two-sighting stability rule, not instant. That is
acceptable for a one-shot import a contributor just ran, and it keeps this
change to one function. Adding the staging directory as a watched root is a
possible later refinement, not a requirement here.

### 2. The declared source reaches the apps

`QueueEntry` gains `declared_source: Option<String>`, populated from
`session_ref.declared_source` where `source` is already set
(`watcher.rs:550`), serialized with `#[serde(default)]` so an existing
`daemon-queue.jsonl` parses unchanged.

`source` is untouched. Nothing resolves an adapter from it — `ipc.rs:623`
records that entry matching is on path shape, not source name — but it is
persisted state and a stable field, and #519 already settled that the origin
rides *beside* the adapter rather than replacing it.

IPC adds `"declared_source": e.declared_source` beside `"source"`.

Each shell prefers it when present:

- **GTK** `agent_label()` — already falls through to the raw slug, so an
  unmapped value degrades to `antigravity` rather than to a wrong label.
- **macOS** `Models.swift:84`
- **Windows** `QueueEntryViewModel.cs:146`

Each gains an `antigravity` → "Antigravity" mapping, and each reads
`declared_source ?? source`.

### 3. What does not change

- No new settings key, no roots row, no onboarding change, no re-onboarding.
- No FFI surface.
- The CLI is unaffected: `cli_source_roots` already includes staging, and
  its display already prefers `declared_source`.
- The daemon still never enumerates processes or talks to the IDE.

## Testing

**The gap itself, as a test.** A daemon started against a state directory
whose `trajectories/` folder holds one staged conversation must produce a
queue entry for it. This test fails on `main` today, which is the point:
without it the fix is unfalsifiable.

**The label end to end.** The same entry must carry
`declared_source = "antigravity"` over IPC — not merely on the `SessionRef`.
The seam that broke in #519 was exactly this kind of hand-off, so it is
asserted at the IPC boundary rather than one layer below it.

**Backward compatibility.** A `daemon-queue.jsonl` written before this change
must load, with `declared_source` absent rather than erroring.

**The working directory stays excluded.** A daemon whose process working
directory contains a `*.trajectory.json` file must NOT queue it. This is the
half of `TrajectorySelection::Auto` the original reason genuinely covers, and
the one thing this change must not quietly switch on.

**Per-shell label mapping.** Each of the three shells has a unit test for its
label map; each gains an `antigravity` case and a case asserting the
`declared_source ?? source` preference.

## Risks

**A contributor's staged files become visible without a fresh prompt.**
Someone who imported conversations before this change and never submitted
them will see them appear in the app's queue after upgrading. They are the
contributor's own files, placed by their own command, and they appear as
*pending* — requiring approval, not auto-uploaded — unless that project is
already armed for auto-upload. That last case is the one worth naming: a
project set to auto-upload would take a previously-invisible staged
conversation and upload it without a further prompt. Whether to force
pending state for entries from the staging scope regardless of project mode
is the one open question in this spec, and I would default to forcing
pending: the contributor armed auto-upload for a watched source, not for an
import they may have forgotten.

**Discovery is not instant.** A contributor who imports and immediately opens
the app may see nothing for a poll interval. Acceptable, and preferable to
plumbing filesystem events for a one-shot command, but it should be stated in
the README next to the import instructions.

## Open question for review

Should entries discovered in the staging scope always enter as `Pending`,
even for a project in auto-upload mode? See the first risk. My
recommendation is yes.
