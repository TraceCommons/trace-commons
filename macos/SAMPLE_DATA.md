# Recorded sample data in `TCShellCore`

`Sources/TCShellCore/DataContract/SampleDaemonData.swift` answers `empty`,
`normalDay`, `busyQueue`, `heldSessions`, `armedFolder` and `unknownCounts`
(and the four methods every sample set answers alike -- `approve`, `keep`,
`undo_keep`, `set_project_mode`) from `RecordedSamples/`, real replies the
real daemon sent against a throwaway store, bundled into `TCShellCore` as a
resource (`Package.swift`) and read through `Bundle.module`. `coreDown` and
Zaki's provisional network methods (`inference_summary`, `model_spend`,
`private_ai`, `mission_catalogue`, `invite_lookup`, `passkey_state`,
`account_session_status`, marked `"_sample":"no source yet"` in the JSON) stay
hand-written, the first because it answers `nil` for every method by
construction and the second because none has a source on `main` yet. To
re-record after a daemon change: from the repo root, run
`cargo test -p trace-commons-contributor --test k2_sample_recorder -- --ignored record_samples_to_disk`
(see that file's module doc for exactly how each sample set is built), then
`swift test` here, and commit both the regenerated `RecordedSamples/` files
and whatever Rust change prompted the re-recording --
`drift_sample_data_matches_the_real_daemon`, an ordinary (non-ignored) test
in the same file, re-records every sample set in memory on every run and
fails, naming the method, the sample set and the first differing field path,
the moment a committed file stops matching what the daemon now sends.

Some recorded files are hand-adjusted after the real capture, each marked
with its own `"_sample"` reason and listed in `HAND_WRITTEN_OVERRIDES`. The
drift test still compares every field an override leaves alone
(`override_touched_keys`); only a file replaced whole is skipped.
`unknownCounts/status.json` has
`decisions_owed` removed by hand, because the badge draws "—" for an older
or unreachable daemon and the real daemon always sends a concrete count;
`normalDay/inference_calls.json` and `busyQueue/inference_calls.json` are a
hand-written `readable: true` page, shaped exactly like the real reply
`calls_page` builds in `daemon/inference_map.rs`, because `readable` needs a
live IronWire proxy answering and no temp store runs one. For the same
reason, `normalDay` and `busyQueue` override `status.json`'s
`private_inference_state` (running on 8463), `routing` (`rows_seen`) and
`daily_budget` (4 uploads today), and `harness_list.json`'s Claude Code row
(connected, `answering`), `activity`, `spend` (known) and
`destination_port`; `empty/harness_list.json` overrides `spend` to a known
zero.

## Dev-only dry run on your own sessions (K2)

The other half of K2: rather than the fixtures above, `DaemonDataWiring.live`
can run against your own real Claude Code and Codex sessions while you build
screens, with a guarantee enforced by the daemon itself, not by this app --
**nothing it does can reach the network.**

Set `TC_DEV_DRY_RUN=1` in the process environment before launching a Debug
build (an Xcode scheme's environment variables, or
`TC_DEV_DRY_RUN=1 .build/debug/TraceCommonsApp`), then go through the
ordinary first-run flow and point it at your own real session folders, same
as any contributor would. `AppModel` logs a console notice once the daemon
starts, so you can confirm the mode took.

What the daemon refuses, unconditionally, while that variable is set:

- `approve`, including a whole project/folder
- `set_project_mode` arming a project for `auto_upload`
- `set_contribution_override` set to `auto_upload`
- `grant_automatic`
- `enroll`
- a witness network preview (`witness_preview_request`), which otherwise
  sends a session's redacted body to a witness service independent of
  `approve`
- hosting IronWire at all (`private_inference`'s proxy never binds, however
  your settings read)
- the one call that actually puts bytes on the wire for an approved upload
  (`SubmitContext::submit_loaded`), checked by `Uploader::upload_entry`
  immediately before it runs

Every one of those is refused with the same fixed label (`dev-dry-run`,
`ERR_DEV_DRY_RUN` / `uploader::REASON_DEV_DRY_RUN` in the Rust source), so a
refusal is never confused with the account, scope, or terms refusals the UI
otherwise handles.

The switch is a single `DaemonShared.dev_dry_run: bool`
(`crates/trace-commons-contributor/src/daemon/ipc.rs`), set exactly once --
by `daemon::start_embedded`, from `TC_DEV_DRY_RUN` in the process
environment -- before the daemon's shared state is ever cloned across
threads. No IPC handler ever reads or writes it, and no wire method names it,
which is what makes it impossible to set, clear, or discover over the
socket: a native app embeds this library in-process, so the only way to
change it is to restart the daemon with a different environment.

On the Swift side, `DaemonDataWiring.devDryRunActive` is the only place the
variable's name appears, and it is compiled `#if DEBUG`: a Release build
never offers the switch at all. `macos/scripts/check-dev-dry-run-release.sh`
builds the app `-c release` and greps the product for the string, the same
method C1 used for `SampleDaemonClient`.

This mode adds no capture or dump helper of its own. The sessions it reads
are whatever real folders you declare, exactly as in ordinary use, and
nothing about this mode writes anywhere new; a tool that did add one would
have to write outside this repository (`~/Library/Caches/TraceCommons/dev-dry-run/`),
never into it.
