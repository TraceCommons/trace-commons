# Native network Zaki lane execution and scope matrix

**Goal:** Implement #1173's authorized Zaki lane behind the additive C3
contracts in [daemon IPC](../../contributor-daemon-ipc-v1_1.md#native-network-additions-c3-1173).
**Architecture:** Keep secrets, consent and network validation in the permissive
Rust contributor core. Swift runs the platform passkey UI and consumes the same
contracts; Ron owns screens and Kristi owns local matching/data.
**Baseline:** `ba8a87777`; C3 lands without Rust or Swift behavior.

| Work | Deliverable / evidence boundary |
|---|---|
| C3 | Contract names, DTOs, SAMPLE JSON and support status before behavior |
| Z1 | Anonymized pilot recordings are separate evidence; existing log fixture is real, synthetic samples are not recordings. Other recordings remain outstanding until captured safely. |
| Z2 | Bounded IronWire summary read preserving real upstream groups and totals |
| Z3 | Register proof lookup; return stored label, no fabricated check details |
| Z4 | Explicit billed-per-model unknown until actual provider source exists |
| Z5 | Extend existing map additively with owned hub/route facts; no invented provider or remote dev-server discovery; coordinate K14 |
| Z6 | Core disclosure plus read/write wrappers around existing hosting lifecycle |
| Z7 | Daily assignment/completion/levels/streak/badges/credit mechanism and ledger rules unresolved in #1174/#1118; no reward semantics invented |
| Z8 | IPC wrapper around existing anonymous skill-evaluation catalogue; local matching sends no profile/results |
| Z9 | Existing `history_rollup.community` is authoritative; preserve rank and absence behavior |
| Z10 | Existing non-redeeming issuer lookup over IPC; full invite URL, real unit, pending conditions |
| Z11 | Approved native passkey names, daemon ceremonies/bind/session lifecycle and Swift platform adapter; no screen redesign |
| Z12 | Add entitlement/profile verification; approved profile bytes and signed observed origin remain operational gates |
| Z13 | Extract existing browser sign-in/status into IPC; preserve enrollment prerequisite and support pre-enrollment status/sign-out |
| Z14 | Inspect and extend existing signed native update feed/release verification; signing/publishing remains separate operational evidence |
| Z15 | Review consent boundaries in affected core/adapter work; every future screen PR still needs its own consent review |

- [ ] Land the C3 docs-only commit and notify implementers of exact interfaces.
- [ ] Implement network, identity, Swift adapter, catalogue and release support in isolated branches; preserve existing licensing and no-new-dependency rule.
- [ ] Merge focused implementation commits with tests for unknown/error/empty states, invalid/replayed inputs, authority changes and consent separation.
- [ ] Run contributor/FFI checks with warnings denied, contract tests, relevant clippy, format, license boundary and Swift tests against freshly built FFI. Release script tests validate profile/domain/update gates without claiming signing or deployment.
- [ ] Review the integrated diff and report implemented, pre-existing and unresolved/external work separately.

All credit remains conditional/pending, proof means only verified model proof,
registry prices never mean billed spend, unknown never means zero/off, and
sample data never becomes a release fallback. No deployment, messages to
collaborators, production capture consent or proposed screen-copy approval
follows from these contracts. Native tokens remain daemon-side; sign-out/wipe
invalidates stale ceremony/account-owned work. The existing source-offer and
permissive-to-AGPL dependency boundary remain load-bearing.
