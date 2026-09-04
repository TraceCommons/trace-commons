# Attested-Inference Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deploy the redaction witness in production configuration, cut the 0.9.0 release, and prove the attested-inference path works end to end for the first time.

**Architecture:** Ordering A from the spec — witness deployment first (the settings card is inert without a published measurement), then the release shipping attestation dormant behind three switches, then a live validation run on the owner's own traffic into pilot ingest. Most tasks are operational rather than code; those carry verification steps in place of tests, and the verification is always a *read-back of live state*, never an assumption that a written setting took effect.

**Tech Stack:** Rust (workspace + separate GTK workspace), dstack/Phala TEE, GitHub Actions, Swift/SwiftUI, WinUI/C#, GTK4, Flatpak, Homebrew.

**Spec:** `docs/superpowers/specs/2026-09-04-attested-inference-release-design.md`

## Global Constraints

- Verification uses `RUSTFLAGS='-D warnings'`. Plain `cargo check` does not apply it; CI does.
- `cargo --workspace` misses two configurations that CI gates. Both broke CI on 2026-09-04. Always also run: the four permissive crates with `--no-default-features`, and the GTK workspace with `--manifest-path crates/trace-commons-contributor-gtk/Cargo.toml`.
- Clippy allow-list, verbatim: `-A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`. Do not widen it.
- No emojis in commits, PRs, code, or reports. Commit subjects are short and imperative with no `feat:`/`fix:` prefix.
- Hash-only logging. Never log raw URLs, tokens, bodies, contributor identity, or trace content.
- License boundary: `-protocol`, `-contributor`, `-contributor-ffi`, `-attestation` are MIT OR Apache-2.0; `-server` and the gate crates are AGPL-3.0-or-later. Permissive may flow into AGPL, never the reverse. Never edit the expected sets in `crates/trace-commons-server/tests/license_boundary.rs`.
- Add no new dependencies. A new dependency invalidates `crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json`, which nothing in PR CI validates — the first failure is the `linux-flatpak` job on an `app-v*` tag.
- The C ABI header exists in two copies that CI requires to be byte-identical: `crates/trace-commons-contributor-ffi/include/trace_commons.h` and `macos/Sources/CTraceCommons/include/trace_commons.h`.
- Current versions: both workspaces at `0.8.0`; latest tags `app-v0.8.0` and `contributor-v0.8.0`.
- Live witness CVM: `8b8e6543-9743-41fc-ac05-a6b414888d5e`, app `f1654b0beac2ac2afae4235ee3d907096cd8f3de`, signing address `0x655a17fcf6d0b9069e1b1dd07a7f5535d0c76798`.

---

### Task 1: Correct the attestation refusal label

The client reports a refusal label the server never emits. It is in a shipped ABI, so it is cheaper to fix before 0.9.0 than after.

**Files:**
- Modify: `crates/trace-commons-contributor/src/witness/status.rs:165`
- Test: `crates/trace-commons-contributor/src/witness/status.rs` (in-crate `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `WitnessTrustState::refusal_label()` returns `Some("witness_inference_attestation_missing")` for `RefusingInferenceReceiptsMissing`. Task 8 (version bump) and the release depend on this landing first.

- [ ] **Step 1: Write the failing test**

Add to the test module in `crates/trace-commons-contributor/src/witness/status.rs`:

```rust
/// The client's refusal label must be the one the server actually emits.
///
/// `witness_service/http.rs` answers `witness_inference_attestation_missing`
/// when a requiring witness gets no receipt. A client reporting a different
/// spelling sends a contributor grepping for a string that appears nowhere
/// in any server response.
#[test]
fn the_attestation_refusal_label_is_the_one_the_server_emits() {
    assert_eq!(
        WitnessTrustState::RefusingInferenceReceiptsMissing.refusal_label(),
        Some("witness_inference_attestation_missing"),
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

```bash
cargo test -p trace-commons-contributor --lib the_attestation_refusal_label_is_the_one_the_server_emits
```

Expected: FAIL, `left: Some("witness_inference_receipts_missing")`, `right: Some("witness_inference_attestation_missing")`.

- [ ] **Step 3: Correct the label**

In `crates/trace-commons-contributor/src/witness/status.rs`, change the `RefusingInferenceReceiptsMissing` arm of `refusal_label`:

```rust
            Self::RefusingInferenceReceiptsMissing => {
                Some("witness_inference_attestation_missing")
            }
```

- [ ] **Step 4: Run the test and the suites that consume the label**

```bash
cargo test -p trace-commons-contributor --lib the_attestation_refusal_label_is_the_one_the_server_emits
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor -p trace-commons-contributor-ffi
```

Expected: the new test PASSES; both suites report `0 failed`. If an FFI ABI test asserts the old string, it encoded the bug — update it and say so in the commit.

- [ ] **Step 5: Run the two configurations a workspace build cannot see**

```bash
RUSTFLAGS='-D warnings' cargo check -p trace-commons-contributor --no-default-features
RUSTFLAGS='-D warnings' cargo check -p trace-commons-contributor-ffi --no-default-features
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
```

Expected: all `Finished` / `0 failed`. GTK re-exports this label through `copy.rs`, so it must compile.

- [ ] **Step 6: Commit and open a PR**

```bash
cargo fmt --all
git add -A
git commit -m "Report the attestation refusal label the server sends

WitnessTrustState::RefusingInferenceReceiptsMissing reported
witness_inference_receipts_missing. No server response uses that string;
witness_service/http.rs answers witness_inference_attestation_missing. A
contributor greps for a label that appears nowhere, and it is in a shipped
ABI, so the cost of leaving it grows."
git push -u origin fix-attestation-refusal-label
gh pr create --repo TraceCommons/trace-commons --base main \
  --title "Report the attestation refusal label the server sends" --body-file -
```

---

### Task 2: Land the wiring PR

PR #596 is the link that makes attested inference reachable at all. Without it 0.9.0 ships a feature that cannot activate.

**Files:** none — this is a merge gate.

**Interfaces:**
- Consumes: nothing.
- Produces: `DaemonSettings::ironwire_attested_bodies` and `settings::attested_bodies_dir_for` on `main`. Task 11 sets that switch.

- [ ] **Step 1: Check its state**

```bash
gh pr view 596 --repo TraceCommons/trace-commons --json mergeable,mergeStateStatus --jq '"\(.mergeable)/\(.mergeStateStatus)"'
gh pr checks 596 --repo TraceCommons/trace-commons --json state --jq '[.[].state]|group_by(.)|map("\(.[0])=\(length)")|join(" ")'
```

- [ ] **Step 2: If BEHIND, update it from main**

```bash
gh pr update-branch 596 --repo TraceCommons/trace-commons
```

The repo requires branches up to date with base. Expect all checks to re-run.

- [ ] **Step 3: Merge once every check passes**

```bash
gh pr merge 596 --repo TraceCommons/trace-commons --squash --delete-branch
```

Do not merge with a failing check. If `linux-shell desktop integration` fails, a shared type gained a field and the GTK workspace needs it — that job is the only thing that compiles that crate.

- [ ] **Step 4: Merge Task 1's PR the same way, then confirm main**

```bash
git checkout main && git pull --ff-only
git log --oneline -3
```

---

### Task 3: Build the witness image from current main

**Files:**
- Modify: `deploy/witness/docker-compose.yml`
- Modify: `deploy/witness/app-compose.json` (generated)

**Interfaces:**
- Consumes: `main` with Tasks 1 and 2 merged.
- Produces: an image digest `ghcr.io/tracecommons/trace-commons-witness@sha256:...` pinned in `docker-compose.yml`. Task 4 deploys it.

- [ ] **Step 1: Dispatch the image build on current main**

```bash
gh workflow run witness-image.yml --repo TraceCommons/trace-commons --ref main
gh run list --repo TraceCommons/trace-commons --workflow witness-image.yml --limit 1
```

The workflow builds natively amd64 (TDX is Intel), runs the three pre-push checks, and asserts `SOURCE_DATE_EPOCH`.

- [ ] **Step 2: Take the digest it reports**

```bash
gh run view <run-id> --repo TraceCommons/trace-commons --log | grep -o "sha256:[0-9a-f]\{64\}" | tail -1
```

Record it. This is the only artifact that a measurement can pin.

- [ ] **Step 3: Pin the digest and set the production redaction mode**

In `deploy/witness/docker-compose.yml`, set the `image:` line to the new digest, and set:

```yaml
      TRACE_COMMONS_WITNESS_REDACTION: "full-pipeline"
```

Leave `TRACE_NEAR_AI_PRIVACY_BASE_URL` and `TRACE_NEAR_AI_PRIVACY_MODEL` as they are — they are measured on purpose, so the destination and model are part of the enclave's identity.

- [ ] **Step 4: Regenerate the manifest and confirm it matches**

```bash
./deploy/witness/build-app-compose.sh
./deploy/witness/build-app-compose.sh --check
```

Expected: `app-compose.json matches docker-compose.yml`. Note the SHA-256 it prints is **not** the value to pin — see Task 5.

- [ ] **Step 5: Commit both files**

```bash
git add deploy/witness/docker-compose.yml deploy/witness/app-compose.json
git commit -m "Pin the witness image the production deployment runs

Built from main by the witness-image workflow, and switched to
full-pipeline so certificates carry a classifier-backed verdict rather
than a deterministic-only one."
git push origin main
```

---

### Task 4: Upgrade the existing CVM to the production configuration

The live witness runs `deterministic-only` with `public_logs: true` — container logs served publicly by a service whose premise is that raw transcripts do not leave it.

That was previously blamed on Phala overriding the manifest. It is not what happens: **`phala deploy` builds its own manifest and never reads `app-compose.json`**, and `--public-logs` / `--public-sysinfo` default to `true`. The setting was written in a file nothing reads.

**Files:** none in the repo — this changes live infrastructure.

**Interfaces:**
- Consumes: the digest pinned in Task 3.
- Produces: an upgraded CVM keeping signing address `0x655a17fc…`, with a new measurement. Task 5 reads and records it.

- [ ] **Step 1: Confirm the CLI is current and the target CVM is the right one**

```bash
phala --version
phala cvms get 8b8e6543-9743-41fc-ac05-a6b414888d5e
```

Expected: status `running`, App ID `app_f1654b0beac2ac2afae4235ee3d907096cd8f3de`.

- [ ] **Step 2: Clear any stale pin that would retarget the deploy**

```bash
rm -f .phala/config
```

A stale `.phala/config` pointing at a deleted CVM has previously turned a deploy into an attempted upgrade of the wrong thing.

- [ ] **Step 3: Upgrade, with the visibility flags explicit**

```bash
cd deploy/witness
phala deploy \
  --cvm-id 8b8e6543-9743-41fc-ac05-a6b414888d5e \
  --no-public-logs \
  --no-public-sysinfo \
  --public-tcbinfo \
  -e TRACE_NEAR_AI_PRIVACY_API_KEY="$TRACE_NEAR_AI_PRIVACY_API_KEY"
```

**Upgrade rather than create.** The signing key is KMS-derived from a stable app id, so upgrading keeps `0x655a17fc…` and moves only the measurement. A new CVM gets a new app id and a new signing address, invalidating anything pinned to the old one.

`--public-tcbinfo` stays on deliberately: it is how an operator reads `mrtd` and `compose_hash` without shelling into the guest, and the measurement is published on purpose.

- [ ] **Step 4: Wait for it to come up and confirm it is not crash-looping**

```bash
phala cvms get 8b8e6543-9743-41fc-ac05-a6b414888d5e --json | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['status'], d.get('in_progress'))"
```

Expected: `running`. A `running` CVM is not proof the container is up — Task 5 proves that.

---

### Task 5: Read the deployed manifest back and record what clients pin

A written setting is not a deployed one. This step exists because the last deployment's visibility flags were wrong and nothing noticed.

**Files:**
- Modify: `deploy/witness/README.md` (record the deployed values)

**Interfaces:**
- Consumes: the upgraded CVM from Task 4.
- Produces: `WITNESS_SIGNING_ADDRESS` and `WITNESS_MEASUREMENT` (an `mrtd:...+mrconfigid:...` string), and the instance `compose_hash`. Tasks 7, 9 and 11 all consume these.

- [ ] **Step 1: Read the manifest dstack actually stored**

```bash
phala cvms get 8b8e6543-9743-41fc-ac05-a6b414888d5e --json > /tmp/cvm.json
python3 - <<'PY'
import json
d = json.load(open('/tmp/cvm.json'))
c = d['compose_file']
for k in ('public_logs','public_sysinfo','public_tcbinfo','allowed_envs','kms_enabled'):
    print(f"{k} = {c.get(k)}")
print("compose_hash =", d['compose_hash'])
PY
```

Expected: `public_logs = False`, `public_sysinfo = False`, `public_tcbinfo = True`, `allowed_envs` containing the classifier key (Phala also adds `DSTACK_AUTHORIZED_KEYS`; note it, it is theirs).

**If `public_logs` is `True`, stop.** The witness is serving raw-transcript logs publicly and must not take traffic. Re-run Task 4 with the flags and investigate why they did not apply.

- [ ] **Step 2: Confirm the deployed image is the one built in Task 3**

```bash
python3 -c "
import json
c = json.load(open('/tmp/cvm.json'))['compose_file']
print([l.strip() for l in c['docker_compose_file'].splitlines() if 'image:' in l or 'WITNESS_REDACTION' in l])
"
```

Expected: the Task 3 digest, and `full-pipeline`.

- [ ] **Step 3: Fetch the attestation and record the two values clients pin**

```bash
U=https://f1654b0beac2ac2afae4235ee3d907096cd8f3de-8088.dstack-pha-prod9.phala.network
N=$(python3 -c "import secrets;print(secrets.token_hex(32))")
curl -sS --max-time 40 "$U/v1/attestation?nonce=$N" | python3 -m json.tool
```

Expected: `signing_address` equal to `0x655a17fcf6d0b9069e1b1dd07a7f5535d0c76798` (proving the upgrade preserved the KMS-derived key) and a `quote_hex`. **If the address changed, a new CVM was created rather than an upgrade** — stop and reconcile before anyone pins anything.

- [ ] **Step 4: Smoke the full pipeline**

```bash
curl -sS --max-time 300 -X POST "$U/v1/witness" \
  -H 'content-type: application/json' \
  -d '{"raw_transcript":"user: deploy the thing\nassistant: using AWS_SECRET_ACCESS_KEY=AKIAIOSFODNN7EXAMPLE and my home dir /Users/example/src\n","consent":{"include_tool_payloads":false}}' \
  > /tmp/wresp.json
python3 - <<'PY'
import json, hashlib
d = json.load(open('/tmp/wresp.json'))
a = d['redacted_artifact'].encode()
print("redaction ok:", b'AKIA' not in a and b'/Users/example' not in a)
print("digest match:", hashlib.sha256(a).hexdigest() == d['certificate']['redacted_sha256'])
print("verdict:", d['certificate']['residual_risk_verdict'])
print("policy:", d['certificate']['redaction_policy_version'])
print("measurement:", d['certificate']['witness_measurement'])
PY
```

Expected: redaction ok, digest match, and a `redaction_policy_version` naming the full pipeline rather than the deterministic alias. **Record the `witness_measurement` string verbatim** — it is what clients pin.

- [ ] **Step 5: Write the deployed values into the README and commit**

Add a dated block to `deploy/witness/README.md` recording the image digest, the instance `compose_hash` (not the script's), the signing address, the measurement string, and the confirmed visibility flags.

```bash
git add deploy/witness/README.md
git commit -m "Record the values the production witness deployment reports

The instance compose_hash rather than build-app-compose.sh's, because the
two differ and only the instance's is what MRCONFIGID carries."
git push origin main
```

---

### Task 6: Configure pilot ingest to accept the witness

Ingest refuses every certificate until it is told which witness to trust. Doing this before the release means the measurement in the release notes is one a real deployment already honours.

**Files:** none in the repo — pilot host configuration.

**Interfaces:**
- Consumes: the signing address, measurement, and policy version from Task 5.
- Produces: a pilot that admits certificates from this witness. Task 11 depends on it.

- [ ] **Step 1: Read the pilot's live configuration, not its files**

```bash
gcloud auth list && gcloud config get-value project
# on tc-pilot-host:
sudo tr '\0' '\n' < /proc/$(pgrep -f trace-commons-ingest | head -1)/environ | grep TRACE_COMMONS_WITNESS
```

The env files and systemd drop-ins have disagreed with the running process before. `/proc/<pid>/environ` is the only honest answer.

- [ ] **Step 2: Set the four witness variables**

```
TRACE_COMMONS_WITNESS_BYPASS_ENABLED=true
TRACE_COMMONS_WITNESS_SIGNING_ADDRESS=<from Task 5 step 3>
TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS=<measurement from Task 5 step 4>
TRACE_COMMONS_WITNESS_ALLOWED_POLICY_VERSIONS=<policy version from Task 5 step 4>
```

All four are required together. A configured witness with no pinned measurement refuses everything — that is `WitnessVerificationError::Unpinned`, fail-closed and correct, but it is not a working deployment.

- [ ] **Step 3: Restart and confirm the running process actually has them**

```bash
sudo systemctl restart trace-commons-ingest
sudo tr '\0' '\n' < /proc/$(pgrep -f trace-commons-ingest | head -1)/environ | grep -c TRACE_COMMONS_WITNESS
```

Expected: `4`. Application logs go to `/var/log/tracecommons/ingest.log`, not the journal — a clean `journalctl` proves nothing.

---

### Task 7: Bump both workspaces to 0.9.0

**Files:**
- Modify: `crates/trace-commons-contributor/Cargo.toml:3`
- Modify: `crates/trace-commons-contributor-ffi/Cargo.toml:3`
- Modify: `crates/trace-commons-contributor-gtk/Cargo.toml:12`

**Interfaces:**
- Consumes: Tasks 1 and 2 merged.
- Produces: `0.9.0` in both workspaces and both lockfiles. Task 9 tags it.

- [ ] **Step 1: Set the version in all three manifests**

```bash
sed -i '' 's/^version = "0.8.0"/version = "0.9.0"/' \
  crates/trace-commons-contributor/Cargo.toml \
  crates/trace-commons-contributor-ffi/Cargo.toml
sed -i '' 's/^version = "0.8.0"/version = "0.9.0"/' \
  crates/trace-commons-contributor-gtk/Cargo.toml
```

- [ ] **Step 2: Refresh both lockfiles — they are separate**

```bash
cargo check --workspace
cargo check --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
git diff --stat -- '*Cargo.lock'
```

Expected: both `Cargo.lock` files show only the version change. The GTK lockfile is invisible to a root build and drifts silently.

- [ ] **Step 3: Verify everything, including the two hidden configurations**

```bash
RUSTFLAGS='-D warnings' cargo test --workspace
for c in trace-commons-protocol trace-commons-attestation trace-commons-contributor trace-commons-contributor-ffi; do
  RUSTFLAGS='-D warnings' cargo check -p $c --no-default-features
done
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
```

Expected: zero failures everywhere.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Release 0.9.0"
git push origin main
```

---

### Task 8: Check the two artifact risks that only fire at tag time

Neither is validated by any PR check. The first failure for both is during the release build.

**Files:**
- Possibly modify: `crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json`

**Interfaces:**
- Consumes: the 0.9.0 lockfiles from Task 7.
- Produces: confidence that Task 9's tag build will not fail on vendored sources or a signing mismatch.

- [ ] **Step 1: Regenerate the flatpak vendored source set and diff it**

```bash
git status --porcelain crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json
```

No dependencies were added in this cycle (the `receipt` feature adds zero packages), so this should be unchanged apart from any version string. If it differs, regenerate it per `crates/trace-commons-contributor-gtk/flatpak/` and commit — an invalidated source set fails the `linux-flatpak` job in `.github/workflows/release-apps.yml`, which is the first thing to run on an `app-v*` tag.

- [ ] **Step 2: Confirm the MSIX publisher matches the signing certificate**

```bash
grep -i "Publisher" windows/packaging/Package.appxmanifest
```

The `Publisher` must equal the signing certificate subject exactly. A pwsh preflight fails the job before any build starts if it does not.

- [ ] **Step 3: Commit anything that changed**

```bash
git add -A && git commit -m "Refresh the vendored flatpak sources for 0.9.0"
git push origin main
```

If nothing changed, skip the commit and record that both checks passed clean.

---

### Task 9: Tag and publish the release

**Files:** none — tags and release notes.

**Interfaces:**
- Consumes: Task 5's measurement and signing address; Task 7's version.
- Produces: `app-v0.9.0`, `contributor-v0.9.0`, and published artifacts.

- [ ] **Step 1: Write the release notes**

They must carry two things ordinary notes would not:

- **The witness measurement and signing address** from Task 5. Users pin these; it is the point of the settings card, and without them the card's only reachable states are off and broken.
- **An explicit statement that attested inference ships dormant** — three independent switches, none defaulted on, and it has never run end to end.

The working, user-facing content is the redaction fail-closed fix and the witness settings card in three shells. Lead with those; describe the attestation machinery as infrastructure.

- [ ] **Step 2: Tag both series**

```bash
git tag app-v0.9.0
git tag contributor-v0.9.0
git push origin app-v0.9.0 contributor-v0.9.0
```

- [ ] **Step 3: Watch the release workflows**

```bash
gh run list --repo TraceCommons/trace-commons --workflow release-apps.yml --limit 3
```

Expected: macOS DMG, Windows MSIX, GTK flatpak, CLI. The flatpak job is the one most likely to fail; Task 8 step 1 is what protects it.

- [ ] **Step 4: Merge the Homebrew tap bump when it opens**

Confirm the formula's URL points at the canonical repository rather than a fork before merging.

---

### Task 10: Run the system end to end for the first time

Every leg is verified against mocks. Nothing has run as a system. Treat this as an experiment with predicted failure modes.

**Files:** none — a live run.

**Interfaces:**
- Consumes: the deployed witness (Task 5), pilot configuration (Task 6), and `ironwire_attested_bodies` (Task 2).
- Produces: either a working end-to-end path, or a list of real defects.

- [ ] **Step 1: Run IronWire locally with body capture on**

Build IronWire from current `main` (it carries #24). In its config set:

```toml
[capture]
enabled = true
bodies = true
```

`bodies` is off by default and holds complete prompts and completions on disk while on — one exchange per session, rolling.

- [ ] **Step 2: Configure the contributor**

Declare IronWire in daemon settings, then set the attested-bodies switch. There is deliberately no UI, so use IPC:

```
set_settings { "ironwire_attested_bodies": true }
```

Set `TRACE_COMMONS_INFERENCE_RECEIPT_ENDPOINT` to the NEAR AI base URL, and pin the witness with `TRACE_COMMONS_WITNESS_URL`, `TRACE_COMMONS_WITNESS_SIGNING_ADDRESS`, and `TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS` from Task 5. All three witness variables are required together; a partial configuration is a refusal to configure, not a partial witness.

- [ ] **Step 3: Generate one real session through IronWire, then contribute it**

Use a genuine session of your own. Confirm first that the ledger row carries what the path needs:

```bash
# the row must have a body_ref, both digests, and an upstream_id
```

If `body_ref` is null, `capture.bodies` did not take effect and the contributor will refuse with `attested::CaptureOff` — silent and correct.

- [ ] **Step 4: Diagnose by symptom**

| Symptom | Diagnosis |
|---|---|
| `RequestHashMismatch` | Capture not verbatim, or re-serialisation between ledger and witness. NOT tampering, despite the name. |
| Receipt fetch fails | Endpoint, the unsigned `model` query param, or `chat_id` is not `upstream_id` in practice |
| `attested::CaptureOff` | `capture.bodies` off at the proxy |
| Witness refuses | Policy version, or verdict is not `Low` |
| `WitnessBodyNotStripped` | The client's own guard fired — the witness returned bodies it should have removed |
| Ingest refuses a valid certificate | Task 6 incomplete |

**Expect the receipt fetch to fail first.** It is the only leg with no successful live execution anywhere.

- [ ] **Step 5: Check the success criterion explicitly**

A trace in the pilot with a verified certificate, admitted on the fast path, **whose stored envelope contains no request or response body.** Query the stored envelope and confirm the absence directly. Do not infer it from a passing certificate — the absence is the whole privacy argument and a certificate would not tell you.

- [ ] **Step 6: Write up what happened**

Record the outcome in `docs/operator/` including every failure and its cause. If the run failed, that list is the next plan; do not enable anything further until it is empty.

---

### Task 11: Document enablement and its limits

**Files:**
- Create: `docs/operator/attested-inference.md`

**Interfaces:**
- Consumes: Task 10's outcome.
- Produces: the operator-facing description of what the system claims.

- [ ] **Step 1: Write the three switches and their owners**

`capture.bodies` belongs to the proxy operator; `ironwire_attested_bodies` belongs to the contributor and is the one that sends prompt bodies off the machine; the witness pin and receipt endpoint are deployment configuration. A contributor flipping only the middle one gets `CaptureOff` and silence — correct, and worth saying so it does not read as broken.

- [ ] **Step 2: State that the server's `required` mode stays off**

Requiring attested inference scopes the corpus to traffic that went through NEAR AI, in practice only via IronWire. Claude Code, Codex, Gemini and Cline sessions have no receipt to offer. Switching it on does not tighten a control; it deletes most of the corpus. Available per-deployment, default off.

- [ ] **Step 3: Write the four limits where an operator reads them**

- A server cannot distinguish a requiring witness from a permissive one at the same measurement. Measurement pins the image, not the environment.
- Receipt replay is deduped nowhere. The witness holds no state by design.
- The attested body is the upstream document, not what the harness sent. Say "the bytes the provider hashed".
- Compaction breaks the history argument, and the witness cannot tell which case it received.

- [ ] **Step 4: State the honest claim**

A certificate says a specific enclave redacted specific bytes and reached a verdict, and — where a receipt verified — that the final inference call happened on NEAR AI's hardware over exactly those bytes. It does not say the trace is genuine, complete, or that unattested turns did not occur.

- [ ] **Step 5: Commit**

```bash
git add docs/operator/attested-inference.md
git commit -m "Describe what attested inference does and does not claim"
git push origin main
```
