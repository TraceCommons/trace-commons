# Recorded sample data in `TCShellCore`

`Sources/TCShellCore/DataContract/SampleDaemonData.swift` answers `empty`,
`normalDay`, `busyQueue`, `heldSessions`, `armedFolder` and `unknownCounts`
(and the four methods every sample set answers alike -- `approve`, `keep`,
`undo_keep`, `set_project_mode`) from `RecordedSamples/`, real replies the
real daemon sent against a throwaway store, bundled into `TCShellCore` as a
resource (`Package.swift`) and read through `Bundle.module`. `coreDown` and
the network methods (C3, #1187: `inference_summary`, `inference_call_proof`,
`model_spend`, `private_ai`, `mission_catalogue`, `invite_lookup`,
`account_session_status`, `activity_missions_catalogue`,
marked `"_sample":"hand-written"` in the JSON) stay hand-written, the first
because it answers `nil` for every method by construction and the second
because they answer from the network and the recorder's temp store has none
behind it. The network samples follow the shapes the daemon serves, are
synthetic rather than pilot observations, and are a candidate for
Rust-emitted fixtures in a follow-up. The PROVISIONAL shapes the Inference
and Missions screens still read (`SampleDaemonData.provisional`, marked
`"_sample":"no source yet"`) are hand-written too. To
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
`private_inference_state` (running on 8463) and `routing` (`rows_seen`),
and `harness_list.json`'s Claude Code row
(connected, `answering`), `activity`, `spend` (known) and
`destination_port`; `empty/harness_list.json` overrides `spend` to a known
zero.

## Dev-only dry run on your own sessions (K2)

The other half of K2: rather than the fixtures above, `DaemonDataWiring.live`
can run against your own real Claude Code and Codex sessions while you build
screens, with a guarantee enforced by the daemon itself, not by this app --
**nothing it does can reach the network.**

Set `TC_DEV_DRY_RUN=1` (or `true`) in the environment of the bundled
debug app: build it with `macos/scripts/make-app-bundle.sh`, then run
`open -n .build/Near.ai.app --env TC_DEV_DRY_RUN=1`, as `run-demo.sh` does.
An unbundled launch (`.build/debug/TraceCommonsApp` or an Xcode scheme) has
no Info.plist channel, so it refuses to start by design (staging design,
section 4.3). The Rust library must be a
debug build too: the switch is compiled out of release builds
(`cfg(debug_assertions)`), so a release dylib ignores the variable.

**Its own state store.** The daemon does not use your real state directory.
It runs against `~/Library/Caches/TraceCommons/dev-dry-run/`, seeded the
first time with a copy of your real `contributor.json` and
`daemon-settings.json` (so it reads the same session folders and builds the
same envelopes), and nothing else: no device key, account session, queue,
policy or history. It reads your real session files, and never writes your
real queue, policy, history or settings -- an entry you approved for real
stays approved there. Delete the folder to start over.

**Nothing is sent.** While the variable is set:

- The IPC dispatcher answers only an allowlist of local methods
  (`DEV_DRY_RUN_LOCAL_METHODS` in `daemon/ipc.rs`). Every other method --
  anything that reaches ingest, the issuer, the witness or near.ai
  (`enroll`, `near_ai_account_enroll`, `native_wallet_flow`,
  `publish_public_run`, `set_public_profile`, `withdraw`, and the rest), and
  `approve` and `grant_automatic` -- is refused with the fixed label
  `dev-dry-run` (`ERR_DEV_DRY_RUN`). It is an allowlist, so a network method
  added later is refused until someone allowlists it.
- `set_project_mode` and `set_contribution_override` refuse `auto_upload`.
- The supervisor runs as a dry run: no upload, history or community pass,
  and `drain_approved` returns before it looks at the queue, so an approved
  entry stays `Approved` -- never `Refused` -- and nothing is retried.
- IronWire is never hosted, however your settings read.

**Telling which mode you are in.** A debug daemon reports
`status.dev_dry_run`, and `AppModel` logs a console notice when it is
`true`. The app never parses the variable itself, so the notice always
matches the daemon. The flag is set once, by `daemon::start_embedded`,
before the daemon's shared state is shared; no IPC method sets or clears it.

`macos/scripts/check-dev-dry-run-release.sh` checks the release Rust dylib
and the release app binary for the variable's name, the same method C1 used
for `SampleDaemonClient`.

## The Linux shell reads the same sets, and what they lack (spike S3)

Debug builds of the GTK shell answer from these files too
(`crates/trace-commons-contributor-gtk/src/sample.rs`), under the same
`TRACE_COMMONS_SAMPLE` names and fallback: a method is answered from its
set's file or from `shared/`, and anything else is refused with the fixed
label `sample-unrecorded`. The Swift hand-written replies above are not
ported, so on Linux those methods are refused.

What each set holds (read from the files on 2026-11-16):

| Set | Waiting | Held (reason) | Kept | History | Folder modes | Unknown counts |
|---|---|---|---|---|---|---|
| `empty` | 0 | 0 | 0 | none | none | no |
| `normalDay` | 3 | 0 | 1 | 1 submitted, 2 accepted, 1 quarantined, 1 withdrawn | 2 Ask | no |
| `busyQueue` | 14 | 0 | 1 | as `normalDay` | 3 Ask | no |
| `heldSessions` | 4 | 4 (2 `second-look-review-required`, 2 `scrub-check-manual`) | 1 | as `normalDay` | 2 Automatic (armed) | no |
| `armedFolder` | 3 | 0 | 0 | as `normalDay` | 1 Automatic (armed) | no |
| `unknownCounts` | 3 | 0 | 0 | as `normalDay` | 2 Ask | `decisions_owed` absent |

So every state the window draws has a set: empty, populated, unknown, held,
armed, kept and withdrawn. Withdrawn is only ever one history row, and no
set has a Never folder.

Reads the map, tree, graph and inspector need that no set records:

- `account_devices_list`: the Traces map's other computers. The macOS
  window reads it through the live client even on sample data, and the
  recorder cannot capture it, because it answers from the account service
  and the recorder's temp store has none. Recording it means a hand-written
  override in the recorder, like `inference_calls`; not done here.
- The Private AI map view's network reads (`private_ai`,
  `inference_summary`, `model_spend`, `inference_call_proof`): hand-written
  in Swift, so a Linux sample has none until they are emitted as files.
- `preview` and `preview_body`: no transcript is recorded, by design.
- Reads only the GTK shell makes today (`hello`, `get_setup_status`,
  `consent_options`, `managed_snapshot`, `subscription_usage`,
  `queue_outcome_counts`, `list_past_sessions`, `insights_week` and the
  rest). They are refused on sample data. Slice 2 moves the GTK window onto
  the macOS window's read set, after which the recorder should cover
  whichever of these remain.
