# Structural validation evidence

Finished UTC: `2026-11-06T22:44:33.073854+00:00` (includes the scoped contribution refresh).

Quint `0.33.0`, Node `v26.13.0`, Python `3.12.8`.

Model seed: `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`; selected source seams reviewed at `a2e0aa56608f8c403deff679f8b51e76ac2f5ca7`.

**14 models typechecked; 155 frame/control projections matched; 1,792 sampled traces and 180,992 sampled states passed view validation.**

These are structural checks and random simulation, not exhaustive safety verification. Selected safety checks are reported separately in [proof results](proofs/results.md). Unobserved actions and frames, model hashes and exact commands are in [validation.json](validation.json).

| Model | Sampled traces | Sampled states | Observed frames / declared frames | Unobserved actions |
| --- | ---: | ---: | ---: | ---: |
| client_consent | 128 | 12,928 | 4 / 4 | 3 |
| client_contribution | 128 | 12,928 | 13 / 16 | 24 |
| client_learning | 128 | 12,928 | 14 / 18 | 19 |
| client_providers | 128 | 12,928 | 8 / 8 | 12 |
| client_services | 128 | 12,928 | 16 / 19 | 18 |
| client_shell | 128 | 12,928 | 18 / 18 | 13 |
| end_to_end | 128 | 12,928 | 9 / 12 | 48 |
| platform_clients | 128 | 12,928 | 15 / 15 | 4 |
| server_identity | 128 | 12,928 | 7 / 8 | 7 |
| server_interfaces | 128 | 12,928 | 8 / 9 | 9 |
| server_lifecycle | 128 | 12,928 | 8 / 8 | 0 |
| server_pipeline | 128 | 12,928 | 5 / 9 | 15 |
| server_receipt | 128 | 12,928 | 5 / 5 | 4 |
| server_settlement | 128 | 12,928 | 5 / 6 | 19 |

The source inventory self-tests and pinned-tree comparison are independent checks. Changed source fingerprints are invalidated in coverage rather than silently retained. The current source index has 925 correspondence gaps.

Source-method replays use synthetic clients and exact extracted method bodies; they are not installed-app UI integration tests. HTML review frames do not establish native visual fidelity. No production Rust, Swift, application manifest or dependency lockfile changes are part of this work.
