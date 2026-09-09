# Enabling attested inference

Attested inference is the path that lets a trace carry evidence that the
inference it records **actually happened** — a per-request receipt from the
provider, verified inside the redaction witness enclave against the verbatim
request and response bodies, before redaction destroys them.

It shipped **dormant** in 0.9.0, on purpose. This runbook is
how an operator takes it from dormant to enforced, what each switch costs,
and how to get back.

> **Read before enabling.** The path has been exercised in part, not end to
> end. A run on 2026-09-06 confirmed live: a proxy serving a hosted model
> unsubstituted, session correlation, body capture, and a receipt whose
> signer verifies against the key the attestation report actually attests —
> for both receipt kinds. What has **not** been run is the remainder: the
> submit, the witness call, and the check that the stored envelope carries
> no request or response body. That last check is the whole privacy
> argument and has never been made directly, so treat the first enablement
> as an experiment with predicted failure modes, not as a rollout.
>
> The run record, including the four corrections it forced to receipt
> verification, is
> [`../superpowers/reports/2026-09-06-attested-inference-first-run.md`](../superpowers/reports/2026-09-06-attested-inference-first-run.md).

Related runbooks, all of which this one assumes rather than repeats:

- [`../../deploy/witness/README.md`](../../deploy/witness/README.md) — how the
  witness CVM is built, deployed and read back. Authoritative for the
  deployment's recorded values.
- [`./pii-backstop.md`](./pii-backstop.md) — the witness *bypass*: what a
  verified certificate changes about the PII backstop hold, and the four
  ingest variables in their own right.
- [`./near-attestation-drill.md`](./near-attestation-drill.md) — proving the
  NEAR AI endpoint is the enclave you pinned.

---

## The three switches, and who owns each

Attested inference is off unless **three independent parties** each turn
something on. None of them defaults on, and a contributor who flips only the
middle one gets a silent, correct refusal.

| Switch | Owner | Where | Effect when off |
|---|---|---|---|
| `capture.bodies` | the IronWire proxy operator | IronWire's own config, `[capture]` section | the ledger row carries no `body_ref`; the contributor refuses with `attested::CaptureOff` and submits an honestly unattested trace |
| `ironwire_attested_bodies` | the contributor | daemon settings, over local IPC (`set_settings`) | no prompt bodies leave the machine, so nothing is attestable |
| the witness pin and receipt endpoint | the deployment | contributor config / env, plus the witness deployment itself | the client has no witness to send raw bytes to |

The middle switch is the consequential one: **it is the switch that sends
prompt and completion bodies off the contributor's machine**, to the witness.
There is deliberately no UI for it in 0.9.0. It is set over the daemon's
authenticated local IPC:

```
set_settings { "ironwire_attested_bodies": true }
```

`set_settings` accepts it alongside the other settings keys; see
[`../contributor-daemon-ipc-v1_1.md`](../contributor-daemon-ipc-v1_1.md).

A contributor who sets only that switch, against a proxy running with
`capture.bodies` off, gets `attested::CaptureOff` — no bodies, no attestation,
submission proceeds unattested. That is correct behaviour and it looks exactly
like the feature being broken. Say so when you brief contributors.

---

## 1. Prerequisites

Before any of this is worth attempting:

1. **0.9.0 binaries on both ends.** `app-v0.9.0` / `contributor-v0.9.0` are
   the first releases that carry the attested path at all. Both workspaces are
   at `0.9.0` in tree.
2. **A deployed witness CVM**, upgraded rather than recreated, whose signing
   address and measurement have been read back **from the running instance**.
   The procedure is Tasks 4 and 5 of
   `docs/superpowers/plans/2026-09-04-attested-inference-release.md`, and the
   values it produced are recorded in
   [`../../deploy/witness/README.md`](../../deploy/witness/README.md) under
   "The production deployment, as of 2026-09-05".
3. **The published measurement.** As recorded there:

   | | |
   |---|---|
   | Signing address | `0x655a17fcf6d0b9069e1b1dd07a7f5535d0c76798` |
   | Instance `compose_hash` | `177eea9a58121613c91ef11c6bf0a7dbe7f00f7f2d8a5a492779896c2f258315` |
   | Measurement | `mrtd:f06dfda6dce1cf904d4e2bab1dc370634cf95cefa2ceb2de2eee127c9382698090d7a4a13e14c536ec6c9c3c8fa87077+mrconfigid:01177eea9a58121613c91ef11c6bf0a7dbe7f00f7f2d8a5a492779896c2f258315000000000000000000000000000000` |
   | Policy version | `ironclaw-deterministic-secret-path-v3+privacy-filter-near-ai-v1` |

   **Do not copy these values from here into a pin without re-reading them
   from the instance.** They move on every witness redeploy — see
   [The measurement-pinning contract](#3-the-measurement-pinning-contract).
   The `app-v0.9.0` release notes already carry a *different*, earlier
   `mrconfigid` (`0168ecca83…`) for the same signing address, because the
   witness was redeployed after those notes were written. That is the whole
   hazard in one example.

---

## 2. Configuring ingest to trust the witness

Ingest ignores every certificate until it is told which witness to trust. Four
variables, read at boot by
`crates/trace-commons-server/src/redaction_witness/config.rs`:

| Variable | Meaning |
|---|---|
| `TRACE_COMMONS_WITNESS_BYPASS_ENABLED` | the master switch. `1`, `true` or `yes` (trimmed, case-insensitive) is on; anything else, including absent, is off |
| `TRACE_COMMONS_WITNESS_SIGNING_ADDRESS` | the pinned witness signing address, `0x` + 20 bytes hex |
| `TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS` | comma-separated measurement strings, compared byte for byte |
| `TRACE_COMMONS_WITNESS_ALLOWED_POLICY_VERSIONS` | comma-separated redaction-policy aliases. Only `full-pipeline` aliases belong here — see [`./pii-backstop.md`](./pii-backstop.md) §3 |

A fifth, `TRACE_COMMONS_WITNESS_CERTIFICATE_MAX_AGE_SECONDS`, is optional and
defaults to 86400. Present-and-unparseable is a boot refusal, never a silent
fallback.

**All four required ones go together.** With the switch on and any of them
missing or blank, the binary refuses to boot naming the missing control
(`witness_signing_address`, `witness_expected_measurement`,
`witness_allowed_policy_versions`). There is no half-configured bypass. The
recommended order is the one in [`./pii-backstop.md`](./pii-backstop.md) §4:
set the three pins with the switch still off, confirm the measurement against
the live instance, then flip the switch.

### The systemd drop-in

The pilot unit
(`deploy/pilot-gcp/systemd/trace-commons-ingest.service`) reads
`EnvironmentFile=/etc/tracecommons/ingest.env`. Add the witness variables as a
drop-in rather than editing that file, so the witness pin can be moved and
reverted as one unit:

```bash
sudo install -d /etc/systemd/system/trace-commons-ingest.service.d
sudo tee /etc/systemd/system/trace-commons-ingest.service.d/witness.conf >/dev/null <<'EOF'
[Service]
Environment=TRACE_COMMONS_WITNESS_BYPASS_ENABLED=true
Environment=TRACE_COMMONS_WITNESS_SIGNING_ADDRESS=0x...
Environment=TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS=mrtd:...+mrconfigid:...
Environment=TRACE_COMMONS_WITNESS_ALLOWED_POLICY_VERSIONS=...
EOF
sudo systemctl daemon-reload
sudo systemctl restart trace-commons-ingest
```

### Verify the running process picked them up

**This step is not optional, and the obvious ways of doing it all answer
wrongly on this deployment.**

- The env file and the drop-in say what was *written*, not what the process
  read. They have disagreed before.
- `journalctl` is empty of application output: this unit appends to
  `/var/log/tracecommons/ingest.log` and `ingest.err`. A clean journal proves
  nothing.
- `pgrep -f trace-commons-ingest | head -1` can return a child rather than the
  service's main process, and reading a child's environment can answer for a
  process that was started before your change.

Ask systemd which PID is the service's, and read that process's environment:

```bash
PID=$(systemctl show -p MainPID --value trace-commons-ingest)
sudo tr '\0' '\n' < /proc/$PID/environ | grep -c '^TRACE_COMMONS_WITNESS_'
```

Expected: `4` (or `5` with the age window set). To see which four:

```bash
sudo tr '\0' '\n' < /proc/$PID/environ | grep '^TRACE_COMMONS_WITNESS_' | cut -d= -f1
```

Print the names, not the values, out of habit — nothing here is a secret, but
the surrounding environment on this host is full of things that are.

Note that ingest emits **no startup log line** naming the witness pin. The
only per-certificate logging is at `debug`, label-only, when a certificate is
refused. `/proc/<MainPID>/environ` is the honest answer about configuration,
and a submission that comes back `accepted` rather than
`awaiting_pii_backstop` is the honest answer about effect.

---

## 3. The measurement-pinning contract

**Every witness redeploy moves the measurement, and every pin has to move with
it.**

The mechanism, confirmed against the live instance and recorded in
[`../../deploy/witness/README.md`](../../deploy/witness/README.md):

```
MRCONFIGID = "01" + instance compose_hash + zero padding to 96 hex chars
```

`compose_hash` commits to the deployed manifest, which embeds
`deploy/witness/docker-compose.yml` verbatim. So it moves when **anything**
about the deployment moves: a new image digest, a changed redaction mode, a
changed body cap or concurrency bound, a changed classifier base URL or model
name — and also the `phala deploy` visibility flags, which are not in the
manifest at all but still land in the stored compose. Toggling `--public-logs`
alone changes the measurement.

That is the intended property. A client pinning the old value refuses the new
deployment until it is re-pinned, which is exactly what should happen when the
enclave's identity changes.

The consequence for an operator is a strict order of work on every redeploy:

1. Redeploy the witness (**upgrade** the existing CVM; a new CVM gets a new
   app id and therefore a new signing address, invalidating every pin).
2. Read the new measurement back from the instance.
3. Update the ingest pin and restart, verifying with
   `/proc/<MainPID>/environ` as above.
4. Update every client pin, and the published measurement wherever it is
   named — `deploy/witness/README.md`, release notes, contributor
   instructions.

Between steps 1 and 3 the deployment is fail-closed, not fail-open:
certificates from the new witness carry a measurement ingest has not pinned,
so they are refused and traces hold exactly as unwitnessed ones do. Nothing is
admitted that should not be. Plan the window anyway.

### The stale-container trap

After an upgrade completes, **the old container can keep answering for about a
minute**, and it answers with the *old* measurement. A measurement read in
that window is the one you just replaced: pin it and every client will refuse
the deployment that is actually running, with no signal beyond a refusal that
looks like a client bug.

`phala cvms get` reporting `running` does not close this window — it describes
the CVM, not the container.

Poll a real certificate until it carries the new hash. `/v1/attestation`
returns a quote and the signing address but no plaintext measurement, so the
value comes from a certificate:

```bash
U=https://<app-id>-8088.<dstack gateway host>
NEW_COMPOSE_HASH=<instance compose_hash from phala cvms get --json>
while :; do
  M=$(curl -sS --max-time 300 -X POST "$U/v1/witness" \
        -H 'content-type: application/json' \
        -d '{"raw_transcript":"user: hello\n","consent":{"include_tool_payloads":false}}' \
      | python3 -c 'import json,sys; print(json.load(sys.stdin)["certificate"]["witness_measurement"])')
  echo "$M"
  case "$M" in *"$NEW_COMPOSE_HASH"*) break ;; esac
  echo "still the old container"
done
```

where `NEW_COMPOSE_HASH` is the instance `compose_hash` from
`phala cvms get <cvm-id> --json`. The two are independent readings of the same
fact — the certificate's `mrconfigid` should be `01` + that hash + padding —
so agreement between them is the signal to pin, and disagreement means the
window has not closed.

---

## 4. Requiring attested inference at the witness

Enforcement lives in the witness, not in ingest. The witness binary reads
`TRACE_COMMONS_WITNESS_REQUIRE_ATTESTED_INFERENCE`
(`crates/trace-commons-server/src/bin/trace-commons-witness.rs`). Off by
default. When on, the witness refuses — `403`, by name — any contribution
whose last declared inference call does not carry a verified receipt:

| Refusal | Means |
|---|---|
| `witness_inference_attestation_missing` | a requiring witness got no receipt |
| `witness_inference_attestation_unavailable` | the receipt could not be checked |
| `witness_inference_call_absent` | the session declares no inference call |
| `witness_inference_call_unattestable` | the call cannot carry a faithful body pair |
| `witness_inference_body_not_in_session` | the offered bodies are not the session's |
| `witness_inference_receipt_unverified` | the receipt did not verify |

**Turning it on excludes most of the corpus.** A receipt exists only for
inference that went through NEAR AI. Claude Code, Codex, Gemini and Cline
sessions have none to offer, and neither does any session that withheld tool
payloads, because the bodies a receipt binds are carried under that consent
flag. Requiring attestation does not tighten a control on the same corpus; it
selects a much smaller one. That is a product decision, and the code refuses
to make it by inheriting a security default.

Two operational consequences:

- The requirement is a **witness deployment change**. Written into
  `deploy/witness/docker-compose.yml` it is measured, so it moves
  `compose_hash` and the measurement, and section 3 applies in full. Passed
  instead as a deploy-time `-e` value it is *not* measured — dstack measures
  an injected variable's **name**, not its value.
- A certificate carries no field saying attestation was required
  (`CertificateDetails` has four fields: verdict, policy version, measurement,
  timestamp). **A verifier holding a certificate cannot tell a requiring
  witness from a permissive one.** The witness logs its policy at boot,
  label-only, and that log line is the only place the answer appears.

---

## 5. Client side: what a contributor needs

1. **0.9.0 app.** macOS, Windows and GTK all gained a witness settings card in
   this release; before it, the witness could only be configured by editing a
   config file or setting environment variables.
2. **IronWire running with body capture on** — `[capture] enabled = true` and
   `bodies = true`. With `bodies` on, IronWire holds complete prompts and
   completions on disk, one exchange per session, rolling. That is a real
   local-disk exposure and the contributor should be told about it in those
   words.
3. **The witness pinned.** Three variables, required *together*
   (`crates/trace-commons-contributor/src/config.rs`):

   ```
   TRACE_COMMONS_WITNESS_URL
   TRACE_COMMONS_WITNESS_SIGNING_ADDRESS
   TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS
   ```

   A URL with no address or no measurements is a **refusal to configure**, not
   a partly configured witness. Note the separator difference from the server
   side: the client separates measurement *sets* with `;`, because a set's
   own keys are comma-separated. The server's list is comma-separated.
4. **A receipt endpoint.** Normally the commons publishes this, from
   `TRACE_COMMONS_NEAR_PROVISIONING_RECEIPT_ENDPOINT`, and a client with none
   saved adopts it at signup or the first time it prepares a bound session; see
   [near-native-provisioning.md](./near-native-provisioning.md).
   `TRACE_COMMONS_INFERENCE_RECEIPT_ENDPOINT` is the operator override on a host
   you control and outranks the published value. Either way it must be an
   allowlisted HTTPS origin with no query, fragment or userinfo, or the client
   refuses it as `inference_receipt_endpoint_invalid`.
   `TRACE_COMMONS_INFERENCE_RECEIPT_CHECK_ATTESTATION=true` (the literal word,
   nothing else counts) additionally **verifies the receipt** against the
   call's own request and response bytes — signature, both digests, and that
   the model the receipt *binds* is the model that served — and then fetches a
   freshly-nonced attestation report for that model —
   `GET {base}/attestation/report?model={model}&signing_algo=ed25519&nonce={64 hex}`
   — and refuses the receipt unless its signer is one of the keys that
   report attests **for that model**, bound as
   `report_data == signing_address || nonce`.

   `signing_algo=ed25519` is a query parameter of that endpoint and is not
   optional: without it the report comes back with ECDSA model attestations,
   whose keys sign no receipt this client verifies. Checking a receipt signer
   against `gateway_attestation.signing_address` — which is what this did
   before — refuses every real hosted-model receipt, because the gateway key
   signs none of them.

   The verification step is not decorative: without it the gate compares only
   the `signing_address` the endpoint *claimed* against the attested set, so a
   hostile receipt endpoint returning a genuinely attested address over a
   bogus signature would pass. The witness refuses such a submission either
   way — but the client would have reported it as attested.

   Note what the client gate still does not do: it does not verify the
   report's TDX quote. A key from it is a claim by the provider, checked for
   internal consistency and freshness, not a proof.
5. **`ironwire_attested_bodies` on**, over IPC, as above.

### The limitation to state plainly

**Receipts exist only for inference NEAR AI served.** The client fetches
`GET {base}/signature/{chat_id}?model={model}&signing_algo=ed25519`, where
`chat_id` is the provider's own identifier for the exchange, recorded in the
local proxy's ledger and nowhere else. A session routed to any other provider
has no such identifier and no receipt to fetch, so it is honestly unattested —
not refused, unless a requiring witness is in play.

Two further honesty notes that belong in any contributor-facing description:

- **The fetch itself discloses something.** A `GET` for a `chat_id` tells the
  provider that this client is preparing to contribute that specific exchange,
  now, from this address. The provider already knew the call happened; it now
  learns it is being contributed. Nothing mitigates this while the receipt has
  to arrive with the submission.
- **The `model` retrieval parameter is not signed** — but the receipt text
  is, when there is a model in it. A chat-completions receipt returns the
  three-part form, `{model}:{requestHash}:{responseHash}`, so it does bind a
  model, and that is the name to read. The query parameter the client sends
  when fetching establishes nothing on its own and no surface may quote it as
  if it did.

### Which key signs depends on the protocol, not on the model

NEAR AI issues **two** legitimate kinds of receipt for the **same hosted
model**. Which one you get is decided by the API the inference call used:

| your harness's inference call | receipt `signature_kind` | signer | receipt text |
|---|---|---|---|
| Chat Completions, `POST /v1/chat/completions` | `provider_tee` | the per-model key in the report's `model_attestations` | three-part, binds the model |
| Responses API, `POST /v1/responses` | `gateway` | the single key in the report's `gateway_attestation` | two-part, binds no model |

Both were captured live against `Qwen/Qwen3.6-35B-A3B-FP8` on the same day.
One attestation-report fetch — `GET /v1/attestation/report?model=..&
signing_algo=ed25519&nonce=..` — returns both objects, so verifying both kinds
costs no extra request.

**The Codex CLI, which this runbook recommends, speaks the Responses API
exclusively.** It dropped `wire_api = "chat"`. So Codex-driven traffic is
entirely gateway-signed, and a verifier that consults `model_attestations`
alone refuses every bit of it.

Retrieval detail worth pinning: a Responses exchange's receipt is looked up by
the **full `resp_`-prefixed identifier**. Stripping the prefix returns 404.

The receipt's own `signature_kind` selects **one** key source and never both.
A gateway receipt is not checked against a model key, nor the reverse; a kind
that is not recognised, or absent, names no source and the receipt is refused.

### A gateway receipt attests the bytes, not the model

Say this out loud before enabling the gateway pin, because it changes what a
certificate means.

A gateway receipt's signed text is the two-part `{requestHash}:{responseHash}`
— no model in it. So nothing compares the declared model against anything:
`verify_receipt` returns no bound model, and neither the client nor the witness
has a name to check. The gateway key is a **single key shared across every
hosted model**, so it cannot tell them apart either.

The consequence: a contributor holding a genuine gateway receipt for a cheap
model can declare that exchange as **any other model**, and both the client and
a gateway-pinned witness accept it. The signature is real and the bytes are
exactly the bytes; the model label beside them is an unattested claim.

Under model pins alone this could not arise, because a receipt binding no model
was always refused. **Enabling gateway pins is the first configuration in which
an attested receipt carries no model claim at all.**

This is inherent to NEAR AI's receipt format and is not fixable in our code.
What follows from it:

- A gateway-pinned witness attests the request and response **bytes**. It does
  **not** attest the model.
- **Any downstream consumer that keys credit, scoring, pricing or eligibility
  off the declared model must not treat a gateway receipt as evidence of that
  model.** Credit scoring, per-model rate tiers and anything reading
  `served_model` on an attested trace are all in scope here. A gateway receipt
  is evidence about bytes only.
- A deployment that genuinely needs the model attested has to require the
  chat-completions path, whose three-part receipt signs the model name — and
  that means not accepting Codex traffic, since Codex speaks only the Responses
  API. That is a product trade-off, not something the witness can decide.

### The witness side of the same keys

A witness pins the two kinds separately, and the receipt's `signature_kind`
decides which set applies:

| Variable | Pins | Needed by |
|---|---|---|
| `TRACE_COMMONS_WITNESS_MODEL_KEY_PINS` | `model=key[,model=key...]`, keyed by the model the receipt itself names | deployments whose contributors make chat-completions calls |
| `TRACE_COMMONS_WITNESS_GATEWAY_RECEIPT_KEY_PINS` | `key[,key...]`, bare 32-byte ed25519 keys | deployments whose contributors use Codex, or anything else on the Responses API |

**An operator whose contributors use Codex needs the gateway pin.** Without
it, on a witness that pins anything at all, every Codex submission is refused
under the folded `witness_inference_receipt_unverified` label — the kind you
did not pin is refused rather than admitted unchecked. A deployment seeing
both kinds of traffic sets both variables. A witness that pins nothing remains
dormant for both.

Both replace `TRACE_COMMONS_WITNESS_GATEWAY_KEY_PIN`, which is no longer read
and whose presence now **refuses the boot**: that variable applied the gateway
key to every receipt, including the `provider_tee` ones it could never match.
The gateway key is pinnable again only under the new name, deliberately, so a
value set under the old semantics cannot silently acquire new meaning. **Unset
the old variable on upgrade.** The derivation procedures for both pins, and
the fail-closed properties, are in `deploy/witness/README.md`.

### Reading a client-side refusal

Every reason a body pair cannot be carried is a distinct label, and each sends
an operator somewhere different:

| Label | Where to look |
|---|---|
| `CaptureOff` | `capture.bodies` at the proxy |
| `NoCall` | the session joined no inference hop |
| `DigestAbsent` | a restarted, cancelled or truncated stream — the proxy records no response digest for one |
| `UpstreamIdAbsent` | no provider identifier, so no receipt is reachable |
| `DigestMismatch` | the bytes on disk do not hash to the recorded digest. Refused rather than carried |
| `BodyTooLarge` | over the 8 MiB per-body client bound |
| `BodyNotUtf8` | no faithful representation in the carrier |

---

## 6. What the whole thing claims

Stated the way it should be repeated, because every shorter version of it is
wrong in the same direction:

> A certificate says that a specific enclave, reporting a measurement you
> pinned, redacted specific bytes and reached a residual-PII verdict over its
> own redaction pass — and, where a receipt verified, that the final inference
> call happened on NEAR AI's hardware over exactly those bytes.

It does **not** say the trace is genuine, complete, or that unattested turns
did not occur. Four specific limits an operator should hold onto:

1. **A server cannot tell a requiring witness from a permissive one at the
   same measurement.** The certificate carries no such field.
2. **Receipt replay is deduped nowhere.** The witness holds no state, by
   design. Nor does a certificate bind a submitter: the pair (envelope bytes,
   certificate) is a bearer token for the life of the age window.
3. **The attested body is the upstream document, not what the harness sent.**
   Say "the bytes the provider hashed", never "what the agent sent".
4. **Compaction breaks the history argument**, and the witness cannot tell
   which case it received.

---

## 7. Rollback

Return to dormant in the order that keeps the deployment fail-closed at every
step.

**Ingest, back to ignoring certificates:**

```bash
sudo rm /etc/systemd/system/trace-commons-ingest.service.d/witness.conf
sudo systemctl daemon-reload
sudo systemctl restart trace-commons-ingest
PID=$(systemctl show -p MainPID --value trace-commons-ingest)
sudo tr '\0' '\n' < /proc/$PID/environ | grep -c '^TRACE_COMMONS_WITNESS_'
```

Expected: `0`. With the switch off an arriving certificate is ignored
entirely and every content-bearing trace holds exactly as it did before. This
is safe to do at any moment: it can only make the server hold more, never
less. Setting `TRACE_COMMONS_WITNESS_BYPASS_ENABLED=false` while leaving the
pins in place is the same outcome and keeps the pins ready to re-enable.

**The witness requirement, back off:** redeploy the witness without
`TRACE_COMMONS_WITNESS_REQUIRE_ATTESTED_INFERENCE`. If it was set in the
measured compose, this is a redeploy and section 3 applies in full — the
measurement moves and every pin has to move with it, including the one you may
have just removed from ingest. Re-pin before telling contributors the witness
is usable again.

**Contributors, back off:** `set_settings { "ironwire_attested_bodies": false }`
stops bodies leaving the machine immediately. Submission continues, unattested.
Turning `capture.bodies` off at the proxy additionally stops bodies being held
on local disk, which is the change worth making if the reason for rolling back
was exposure rather than breakage.

Nothing in this rollback re-evaluates anything already decided. A submission
admitted on a verified certificate stays admitted; a trace held while the pin
was stale stays held. Both are submit-path decisions.
