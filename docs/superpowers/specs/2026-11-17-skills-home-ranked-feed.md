# Skills Home: personal, mixed stories and creator follows

The user expects Home to mix creator updates and locally generated insights, requests a smarter selection algorithm, and wants Follow to feel richer than a checkbox. They explicitly chose a mixed discovery feed favoring followed creators, while Following remains limited to their choices. The installed staging 113 snapshot contains 46 creator events, 24 usage rows and current explicit Claude activity. Default profile cards currently precede all stories; creator generation cuts to five events before selection and can crowd out another creator.

## Feed policy

Run deterministically on-device, using the existing local snapshot; no model calls, new telemetry, remote ranking requests or inferred user traits. Keep eligibility/evidence checks for real updates, explicit usage, complete comparison windows and verified public activity signals. Do not invent events, first-use claims, reliable totals from partial history or public popularity facts.

Build a bounded diverse candidate pool before ranking. Prefer a verified available update for a recently used skill, then meaningful local changes, relevant creator news and a useful recent recap. Boost exact installed/used skill identity and followed creators. Freshness decays by observed age, not assumed publication time. Cap activity/count influence: volume is not relevance. Rank greedily with recent source/publisher/topic repetition penalties, stable tie-breaking and a bounded result count. Avoid noisy one-event changes. Deduplicate the same skill update and batch a creator's changes before ranking. Missing sources do not cause filler; never force an empty or unsupported story family.

Home is one ranked story column; cards retain their date/period context. Creator introductions are secondary after real stories, and remain a baseline fallback. No strict alternation, engagement tracking, extra view selector or raw score UI. Following customizes Home rather than replacing default discovery. Explicit Following feed remains separate and chronological; combined explicit/default Home event metadata remains within 48KiB with per-publisher opportunity before bounding. Catalog remains 180KiB.

## Follow interaction

Reuse the glass design system. A shared creator Follow pill with plus icon becomes a confirmed Following/checkmark state, not a checkbox. Following offers a deliberate Unfollow action. Only the active creator shows pending progress; other writes stay serialized. Never show saved success before daemon confirmation. Failure retains the confirmed state, provides contextual retry feedback, and cancellation clears pending state. Keyboard, accessibility and Reduce Motion behavior are preserved. Shared short copy, no new settings or feature-flag controls.

## Validation

Tests prove eligibility, source/topic diversity, relevance over volume, stable ordering, duplicate suppression, age/weak-signal filtering and no forced unsupported stories. Follow tests cover in-flight, confirmed, failed and cancelled writes and state preservation. Replay the retained current snapshot locally and inspect native renders at 440/900 points; do not commit private snapshots or modify app state. Preserve primary checkout, source licenses, build/test locks and Kache flags. No dependencies. No automatic merge, release or machine installation in this task.
