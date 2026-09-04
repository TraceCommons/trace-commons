# Releasing and deploying the attested-inference system

**Status:** approved design. Ordering A, agreed 2026-09-04.

Twelve PRs landed on 2026-09-04 across `trace-commons` and `nearai/ironwire`
building an attested-inference path. Every leg is verified in isolation
against mocks. **The system has never run once end to end** — no real trace
has been captured by a real IronWire, carried to the real witness, with a
real receipt fetched from NEAR AI, returning a certificate real ingest
accepts. The receipt leg has never made a successful live fetch anywhere.

This spec sequences the deployment, the release, and the validation that
turns that sentence into a different one.

## The decision that shapes everything

**Ship dormant, validate after.** The release carries all the code with
attested inference off — it already is, behind three independent switches,
none defaulted on. What ships working is the redaction fail-closed fix and
the witness settings card in three shells. Attestation stays dark until a
live run proves it.

**Ordering A: witness deployment, then release, then validation.** The
witness settings card is inert without a published measurement — all three
shells let a contributor pin a witness, and a configured-but-unpinned
witness *refuses every submission*, so shipping the UI with nothing to pin
offers a control whose only reachable states are "off" and "broken". The
deployment is independent of the release; the release's usefulness depends
on the deployment. Ordering A also puts the one irreversible outward-facing
step — publishing a measurement people pin — before the artifacts that
reference it.

## 1. Witness production deployment

The live witness runs a **diagnostic** configuration: `deterministic-only`
on an image predating this work, with `public_logs: true` on a service whose
premise is that raw transcripts do not leave it.

That last fact was misdiagnosed once and the correction matters:
`phala deploy` **builds its own manifest and never reads
`app-compose.json`**. `--public-logs` and `--public-sysinfo` default to
`true`. So the manifest's `false` was not overridden by Phala — it was set
in a file nothing reads.

**Build.** Dispatch the `witness-image` workflow on current `main`; it
builds natively amd64, runs the three pre-push checks and asserts
`SOURCE_DATE_EPOCH`. Take the printed digest.

**Configure.** Pin that digest in `deploy/witness/docker-compose.yml`, set
`TRACE_COMMONS_WITNESS_REDACTION: full-pipeline` against the already-measured
classifier base URL and model, regenerate `app-compose.json`, commit both.

**Deploy, with the flags explicit:**

```
phala deploy --no-public-logs --no-public-sysinfo --public-tcbinfo \
             -e TRACE_NEAR_AI_PRIVACY_API_KEY=...
```

`public_tcbinfo` stays on deliberately: it is how an operator reads `mrtd`
and `compose_hash` without shelling into the guest, and the measurement is a
value published on purpose.

**Upgrade the existing CVM `8b8e6543-9743-41fc-ac05-a6b414888d5e`,** do not
create a new one. The signing key is KMS-derived from a stable app id, so an
upgrade keeps the signing address `0x655a17fc…` and moves only the
measurement. A new CVM gets a new app id and therefore a new signing
address, invalidating anything pinned to the old one.

**Read the manifest back.** `phala cvms get <id> --json` reports
`compose_file` — the manifest dstack actually stored. Diff it against
intent: `public_logs: false`, `public_sysinfo: false`, `allowed_envs`
holding only the classifier key. This is a permanent step, not a one-off. It
is the only thing between a written setting and a deployed one.

**Pin from the instance, never from the script.** `build-app-compose.sh`
printed `a12e930e…` while the deployed `compose_hash` was `c2511a8b…` — the
value inside the live certificate's MRCONFIGID. Take the hash from the
instance. Then `GET /v1/attestation?nonce=…` and record the signing address
and the `mrtd`+`mrconfigid` measurement. **Those two values are what clients
pin and what the release notes carry.**

**Smoke test:** `POST /v1/witness` with a known secret and a home path.
Confirm redaction, a `Low` verdict from the full pipeline, and a certificate
whose `redacted_sha256` matches the artifact.

**Two risks, stated rather than buried.** The classifier makes the witness
dependent on NEAR AI availability *inside* the enclave, and that adapter is
measured as flaky under load. And `full-pipeline` changes
`redaction_policy_version`, so ingest's allowlist must accept the new value
or every certificate is refused on arrival.

## 2. The 0.9.0 release

**Gates:** #596 merges (without it 0.9.0 ships a feature that cannot
activate), and the `RefusingInferenceReceiptsMissing` label mismatch is
fixed — the client reports `witness_inference_receipts_missing` where the
server says `witness_inference_attestation_missing`, and it is in a shipped
ABI.

A visual check of the three witness cards was considered as a gate and
**deliberately declined**. The consequence, recorded rather than argued: the
first person to see these cards will be a user. The tests cover binding
correctness and state distinctness, not layout.

**The bump.** 0.9.0 in both the root workspace and the GTK workspace —
separate manifests, separate lockfiles, and only the GTK one is invisible to
a root build. Tags `app-v0.9.0` and `contributor-v0.9.0`.

**Artifacts:** macOS DMG, Windows MSIX, GTK flatpak, Homebrew tap, CLI.

**The risk that only fires at tag time:** `flatpak/cargo-sources.json` is a
vendored offline source set that nothing in PR CI validates — the first
failure is the `linux-flatpak` job on the tag. No dependencies were added
today (the `receipt` feature adds zero packages), so it should be clean;
regenerate and diff before tagging rather than discover it mid-release. Same
for the MSIX `Publisher` matching the signing cert subject, which fails a
pwsh preflight before any build starts.

**Release notes must carry two things** ordinary notes would not: the
**witness measurement and signing address** from section 1, because users
pin them and that is the point of the settings card; and an explicit
statement that **attested inference ships dormant** behind three switches
and has never run end to end.

## 3. Live end-to-end validation

The first time the system runs as a system. Treat it as an experiment with
predicted failure modes, not a smoke test.

**Venue:** your own machine into pilot ingest. IronWire from current main
with `capture.bodies = true`; the contributor daemon with IronWire declared
*and* `ironwire_attested_bodies` on — set by IPC `set_settings`, since there
is deliberately no UI; `inference_receipt_endpoint` at NEAR AI; the witness
pinned to section 1's values.

**Pilot side, easily forgotten:** ingest needs the witness signing address,
the expected measurement, and an allowed `redaction_policy_version` matching
what `full-pipeline` reports. Any one missing and it refuses — correctly,
while you debug the wrong end. The pilot's live configuration is readable
only from `/proc/<pid>/environ`; env files and systemd drop-ins have lied
before.

**Failure modes and what each means:**

| Symptom | Diagnosis |
|---|---|
| `RequestHashMismatch` | Capture not verbatim, or something re-serialised between ledger and witness. NOT tampering, despite the name. |
| Receipt fetch fails | Endpoint, the unsigned `model` query param, or `chat_id` is not `upstream_id` in practice |
| `attested::CaptureOff` | `capture.bodies` off at the proxy — silent and correct |
| Witness refuses | Policy version, or verdict is not `Low` |
| `WitnessBodyNotStripped` | The client's own guard fired: the witness returned bodies it should have removed |
| Ingest refuses a valid certificate | Pin or policy allowlist not configured on the pilot |

**Expect the receipt fetch to fail first.** It is the only leg with no
successful live execution anywhere; every other hop has at least run against
a mock shaped like the real thing.

**Success criterion:** a trace in the pilot with a verified certificate,
admitted on the fast path, whose stored envelope contains **no request or
response body**. Check that last part explicitly rather than infer it — it
is the whole privacy argument, and a passing certificate would not tell you.

## 4. Enablement

Three switches with three different owners: `capture.bodies` is the proxy
operator's; `ironwire_attested_bodies` is the contributor's, and it is the
one that sends prompt bodies off the machine; the witness pin and receipt
endpoint are deployment configuration. A contributor flipping only the
middle one gets `CaptureOff` and silence — correct, and worth documenting so
it does not read as broken.

**Staging:** the owner first, on their own traffic (section 3). Then a small
invited set who know what they are contributing. Broad enablement waits on
two things that do not exist: a real UI for the switch — it turns on sending
prompt bodies to a third-party enclave and must not ship as a bare toggle —
and a plain statement of the retention change, that IronWire holds one
exchange's bodies on disk while enabled.

**Leave the server's `required` mode off.** Requiring attested inference
scopes the corpus to traffic that went through NEAR AI, in practice only via
IronWire. Claude Code, Codex, Gemini and Cline sessions run on Anthropic,
OpenAI and Google and have no receipt to offer. Switching it on does not
tighten a control; it deletes most of the corpus. That is a product decision
about the inference provider and should be taken as one. Available
per-deployment, default off.

**Four limits that belong where an operator reads them, not only in code:**

- A server **cannot distinguish a requiring witness from a permissive one**
  at the same measurement. Measurement pins the image, not the environment.
  Closing it needs the v2 certificate profile — a flag day across three
  implementations of the preimage, and the main thing between this and a
  strong end-to-end claim.
- **Receipt replay is not deduped.** The witness holds no state by design;
  this belongs to ingest and does not exist.
- **The attested body is the upstream document**, not what the harness sent.
  IronWire re-serialises on model swap, privacy filter and cross-family
  translation. Operator-facing wording must say "the bytes the provider
  hashed".
- **Compaction breaks the history argument.** The final request body covers
  the conversation only for an uncompacted linear session, and the witness
  cannot tell which it received.

## What the system honestly claims when this is done

A certificate says a specific enclave redacted specific bytes and reached a
verdict, and — where a receipt verified — that the final inference call in
that trace happened on NEAR AI's hardware over exactly those bytes.

It does not say the trace is genuine, complete, or that unattested turns did
not occur.

## Not in this spec

Admission control — requiring an invite **or** attested inference, and the
onboarding submission window. That is a new subsystem, none of it is built,
and it has its own design at
`2026-09-04-admission-invite-or-attestation-design.md`.
