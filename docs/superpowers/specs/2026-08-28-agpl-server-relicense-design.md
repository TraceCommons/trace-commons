# AGPL relicensing of the server components

Date: 2026-08-28
Status: design, approved
Copyright holder: K&Z Partners LLC

## Problem

The whole workspace is published under `MIT OR Apache-2.0`. That permits a third
party to run a closed-source fork of the hosted control plane -- the ingest API,
the gate pipeline, the credit scoring, the dedup clustering -- without returning
anything. The server is the part of Trace Commons that has accumulated the
non-obvious work, and it is exactly the part a permissive license gives away.

The clients are the opposite case. Adoption of the contributor CLI, the desktop
apps, and the envelope protocol depends on them being trivially embeddable,
including inside proprietary agent harnesses. Ironclaw is the first trace source
and consumes `trace-commons-protocol` directly.

## Decision

Split the license along the client/server seam.

**AGPL-3.0-or-later**

- `trace-commons-server`
- `trace-commons-gate-api`
- `trace-commons-gate-enclave`

**MIT OR Apache-2.0 (unchanged)**

- `trace-commons-protocol`
- `trace-commons-contributor`
- `trace-commons-contributor-ffi`
- `trace-commons-contributor-gtk`
- `trace-commons-operator-client`
- `trace-commons-mark`
- `trace-commons-build-info`

### Why the boundary holds

Every cross-boundary dependency edge points permissive into copyleft, which is
the legal direction. Verified against the manifests:

- `trace-commons-server` depends on `-build-info`, `-gate-enclave`,
  `-operator-client`, `-protocol`.
- `trace-commons-gate-enclave` depends on `-gate-api`.
- Nothing depends on `trace-commons-server`.
- No permissive crate depends on `-gate-api` or `-gate-enclave`.

So no client crate is contaminated, and the AGPL crates may freely absorb the
permissive ones.

### Accepted cost of putting the gate crates under AGPL

`crates/trace-commons-gate-api/README.md` and CLAUDE.md describe the gate traits
as a seam where a proprietary scoring backend substitutes. Under AGPL, a *third
party's* proprietary backend linking those traits is a derivative work and cannot
stay closed. K&Z Partners LLC can still ship one, because it holds the copyright
and can license itself.

This is deliberate: the seam should be available to the copyright holder and
closed to everyone else. The consequence, accepted, is that with no CLA the first
outside contribution to `-gate-api` or `-gate-enclave` permanently removes the
ability to grant a proprietary exception on that crate. Revisit only if a
commercial exception track becomes real.

### Contributor consent

Three people other than the copyright holder appear in the history. Two touched
files inside the AGPL crates:

- **abbyshekit** (15 commits) -- `gate_calibrate/bakeoff_report.rs`,
  `gate_calibrate/run_candidate_eval.rs`.
- **Sean Braithwaite** (4 commits) -- `pilot_bootstrap/hf_dataset.rs`,
  `pilot_bootstrap/mod.rs`, `trace-commons-ingest.rs`,
  `gate_calibrate/run_candidate_eval.rs`, `tests/run_candidate_eval.rs`.

No CLA exists. We proceed permissive-in / copyleft-out: MIT and Apache-2.0 are
both one-way compatible with AGPL-3.0, so future combined versions ship under
AGPL while those contributions retain their original grants on the historical
code. Their MIT/Apache grants on already-published commits are irrevocable and
are not being claimed otherwise. No consent request is sent.

## Work

### 1. Manifests

Root `[workspace.package] license` stays `"MIT OR Apache-2.0"`. The three AGPL
crates replace `license.workspace = true` with
`license = "AGPL-3.0-or-later"`.

`crates/trace-commons-contributor-gtk/Cargo.toml` already sets its license
literally (it is excluded from the workspace); it stays `"MIT OR Apache-2.0"`.

### 2. License files

- Add `LICENSE-AGPL` containing the full AGPL-3.0 text.
- Rewrite the root `LICENSE` to name the split crate by crate, replacing its
  current two-bullet form.
- `LICENSE-MIT` and `LICENSE-APACHE` are untouched.

### 3. Per-file headers

Prepend to every `.rs` file in the three AGPL crates (80 + 6 + 12 = 98 files):

```
// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
```

Applied by a script that inserts at line 1 only. The repository is not
rustfmt-clean and the post-edit hook rewrites whole files, so verify with
`git show --stat`: the expected shape is ~98 files at +2/-0 each. Anything
larger means the hook reformatted and the commit must be redone.

Files that begin with an inner attribute (`#![...]`) still take the header
above it -- comments before inner attributes are legal.

### 4. AGPL section 13 source offer

AGPL-3.0 section 13 requires that users interacting with the program remotely be
offered the Corresponding Source. Pilot contributors interact with
`trace-commons-ingest` over the network and never see the repository, so a
documentation-only notice is not enough.

Add `GET /v1/source` to `trace-commons-ingest`:

- Unauthenticated. Outside the auth middleware, outside every fail-closed gate,
  and it must not consult `trace_current_tenant_id()` -- it carries no tenant
  context by design.
- Returns JSON: license identifier, source URL, build commit, build version.
- Reveals nothing beyond what a published release already reveals, so it does
  not violate the hash-only logging convention.

Prerequisite: confirm `trace-commons-build-info` exposes a commit SHA. If it
does not, adding one is part of this work.

### 5. Dependency license audit

`cargo-deny` is installed; there is no `deny.toml`. Add one and run
`cargo deny check licenses` over the default, `near-ai-scorer`, and
`local-gpu-models` feature sets.

A dependency reachable from the three AGPL crates under a license incompatible
with AGPL-3.0 -- GPL-2.0-only is the realistic case -- is a hard blocker. Run
this audit and report before editing any license text; a hit changes the plan.

### 6. Boundary guard test

The durable risk is not this change, it is someone later adding `-gate-api` to
the contributor CLI and silently making a client copyleft. Add a test that reads
`cargo metadata` and asserts no crate declaring `MIT OR Apache-2.0` has an
AGPL-licensed workspace crate anywhere in its dependency closure.

Note this test must reach `trace-commons-contributor-gtk`, which is excluded
from the workspace and so absent from the default `cargo metadata` output.

### 7. Endpoint test

`GET /v1/source` returns 200 with no credentials presented, and reports the
build commit.

### 8. Documentation

- `README.md` has no license section at all today. Add one stating the split.
- `CLAUDE.md`: add a Licensing section pinning the boundary and the rule that
  code does not move from an AGPL crate into a permissive one.
- `docs/operator/`: note the section 13 endpoint so operators do not firewall it.

## Out of scope

- No CLA, DCO, or CONTRIBUTING file.
- No commercial-exception or dual-licensing offer.
- No relicensing of the client crates, now or as part of this work.
- No attempt to alter the license of already-published history.
