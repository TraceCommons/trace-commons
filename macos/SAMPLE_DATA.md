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

Three recorded files are hand-adjusted after the real capture, each marked
with its own `"_sample"` reason and listed in `HAND_WRITTEN_OVERRIDES`
(excluded from the drift test, since a fresh real capture can never equal a
value deliberately edited away from): `unknownCounts/status.json` has
`decisions_owed` removed by hand, because the badge draws "—" for an older
or unreachable daemon and the real daemon always sends a concrete count;
`normalDay/inference_calls.json` and `busyQueue/inference_calls.json` are a
hand-written `readable: true` page, shaped exactly like the real reply
`calls_page` builds in `daemon/inference_map.rs`, because `readable` needs a
live IronWire proxy answering and no temp store runs one.
