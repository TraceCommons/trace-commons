# Structural validation evidence

Finished UTC: `2026-11-06T02:29:14.673179+00:00`.

Quint `0.33.0`, Node `v26.13.0`, Python `3.12.8`.
Source baseline: `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`.

**14 models typechecked; 155 frame/control projections matched; 1,792 sampled traces and 180,992 sampled states passed view validation.**

This is structural validation and random simulation, not exhaustive state exploration or a product-promise verdict. No selected promise, invariant, temporal property or TLC result exists. Many actions and frames were not observed; see the exact lists and content hashes in [validation.json](validation.json).

| Model | Sampled traces | Sampled states | Observed frames / declared frames | Unobserved actions |
| --- | ---: | ---: | ---: | ---: |
| client_consent | 128 | 12,928 | 4 / 4 | 3 |
| client_contribution | 128 | 12,928 | 13 / 16 | 23 |
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

Additional checks:

- inventory extractor self-tests passed.
- inventory --check passed against pinned sources.
- ITF decoding negative control failed before decoder correction and passes after.
- deliberate frame/control drift rejected.
- unselected-promise checker refused execution.
- surface embedded JSON/JS valid and local HTTP 200.

The surface has 14 families and 155 frames. Browser visual QA could not run because the CUA browser access host was unavailable; HTML/JSON/JavaScript and localhost HTTP checks do not establish visual fidelity.

Source-fidelity reviews corrected model-only mismatches involving approval origin, background row selection, recyclable generations, stale operability reads and lease ownership, verdict versus processing state, identity validation/transport ordering, modal controls, captured credentials and heartbeat recovery. They did not change production code or prove source equivalence.

The earlier model-author validation paragraphs describe the writing stage. This integrated report supersedes their typecheck-only status; product promises remain awaiting owner selection.

Reproduce with a new output directory:

```sh
python3 docs/flows/trace-commons/scripts/verify.py --out /private/tmp/tc-quint-validation-NEW
python3 docs/flows/trace-commons/scripts/inventory.py --self-test
python3 docs/flows/trace-commons/scripts/inventory.py --check
python3 docs/flows/trace-commons/scripts/build_surface.py
```

A subset can be rechecked after a scoped change using repeated `--model NAME` arguments. Evidence for an unchanged model is reusable only when all recorded model/view/frame hashes still match. Raw traces and logs stay outside the committed seed.

A final whitespace-only cleanup in `end_to_end.qnt` was separately typechecked, control-checked and simulated. `validation.json` records the exact per-model commands and final hashes for both runs.
