# Letting a witness certificate affect the PII backstop

The redaction witness issues certificates (#548). Nothing on the server builds
a pin from configuration, so today the deployment produces certificates the
server can verify **in principle and not in practice**. This design closes that
gap and states exactly what a verified certificate is allowed to change.

The short version, because the rest of this document is qualifications: a
verified certificate may let ingest **not enter the PII-backstop hold**. It may
not skip anything the server already does synchronously, it may not substitute
the witness's verdict for the server's own, and on this pilot **it will not
drain the held backlog, because that backlog contains no witnessed traces and
no client emits one.**

Verified against the tree on 2026-09-02, at `e2660e04`.

---

## The ceiling, and why the obvious reading of it is wrong

The binding constraint is stated in three places already --
`deploy/witness/README.md` ("What a certificate attests"),
`witness_service/mod.rs`'s module doc, and the comment block at
`crates/trace-commons-server/src/redaction_witness/verification.rs:246-283`,
which was written for this document. It is:

> A witness certificate cannot license skipping the PII backstop wholesale.
> At most it can license skipping the backstop's *classifier* stage; the
> trailing deterministic sweep must still run.

The reason is the classifier's training set. `rescrub_envelope_prose_pii_with`
says it in a comment at `trace_contribution.rs:4617-4626`:

> The classifier is trained on prose PII, not credential formats, so an AWS
> key, a bearer token or a PEM block produces no span and would be written
> straight back into the field. The residual scan then finds it and
> quarantines the trace [...] That is the whole of the pilot's quarantine
> backlog.

The obvious reading of the ceiling is that a bypass must build a new
classifier-free path through the backstop that keeps the sweep. **That reading
is wrong, and the tree says so.** The three stages the ceiling requires already
run on every submission, synchronously, before the hold is decided.

### What actually runs where

`rescrub_trace_envelope` (`trace_contribution.rs:4246`), called from the submit
handler at `trace-commons-ingest.rs:12911`, before `status_for_risk` at 12934
and before `corpus_status_with_pii_backstop_hold` at 12954:

1. `reconcile_consent_declarations` -- corrects under-reported consent flags;
2. the deterministic pass over `events[*].redacted_content` **and**
   `events[*].structured_payload` (`trace_contribution.rs:4265-4281`) -- this
   is the structured pass;
3. `detect_correction_credentials` over `outcome.human_correction`;
4. `redact_envelope_side_channels`;
5. `residual_envelope_scan` (`trace_contribution.rs:4301`) -- the residual scan,
   detection-only, forcing `High` on any survivor;
6. `resolve_post_scrub_risk` with `useful_classifier_result: false`, so this
   pass can only **raise** the risk, never lower it.

`rescrub_envelope_prose_pii_with` (`trace_contribution.rs:4583`), which the
backstop driver runs asynchronously, does:

1. the same consent reconciliation;
2. **the classifier** over each event's `redacted_content`, then the
   deterministic sweep over *the classifier's output*
   (`trace_contribution.rs:4628-4631`);
3. **the classifier** over every string leaf and object key of
   `structured_payload`;
4. `residual_envelope_scan` again;
5. `resolve_post_scrub_risk`, this time able to lower risk when the classifier
   produced evidence.

Subtract one from the other. **The only thing the backstop adds over the
synchronous submit pass is the prose-PII classifier.** Its deterministic sweep
exists to cover the classifier's own output; with no classifier there is no
output to cover.

### So the sweep the ceiling protects is already there

A witness running `full-pipeline` emits classifier output into the artifact the
contributor uploads. The credential-emission hazard therefore happens *at the
witness*, upstream, and the bytes carrying it arrive at ingest as ordinary
submission bytes. `rescrub_trace_envelope` sweeps those bytes deterministically
and scans them residually **before the hold is decided**. A witness-emitted AWS
key is caught there, forces `High`, and `status_for_risk` sends the trace to
`Quarantined` -- before any bypass has a chance to act.

That is the whole safety argument for this design, and it is the reason the
bypass can be a single early return rather than a second pipeline. It is also
the one thing a reviewer must check: **if `rescrub_trace_envelope` ever stops
running before the hold decision, this design is unsound.** A test pins the
ordering.

### What the bypass may not do

The witness verdict is a **pass** verdict over the witness's own redaction
report -- `witness_service/mod.rs` says so explicitly, and warns that it is
"*not* the post-residual-scan verdict `rescrub_trace_envelope` resolves on the
server, which may raise it".

So: the server's own `residual_pii_risk`, as resolved by its own synchronous
pass, remains authoritative. `status_for_risk` is untouched. The certificate is
consulted **only** at `corpus_status_with_pii_backstop_hold`, and only in the
one direction of not entering the hold. A certificate can never turn a
`Quarantined` into an `Accepted`, and can never lower a risk tier.

---

## What a bypass saves, stated plainly

This section exists because the brief asked for it, and the answer is worth
more than the plan.

**For a witnessed trace: everything.** The classifier is the entire cost. The
pilot's self-hosted `openai/privacy-filter` measures around 58 characters per
second; the deterministic sweep and residual scan are local regex work over the
same bytes. A witnessed trace that skips the hold is Accepted at submit and
never enters the queue.

**For this pilot today: nothing at all.** Four independent reasons, any one of
which is sufficient:

1. **No client emits a certificate.** The witness service's own plan lists
   "the client half" under "Not in this plan", and the spec's sequencing puts
   client-side measurement verification at step 6. Nothing in
   `trace-commons-contributor` mentions a witness.
2. **The witness has never run.** `deploy/witness/README.md`'s "What in this
   document is unverified" opens with "No part of this has run on a real CVM",
   and "Nothing in this project has spoken to a live dstack guest agent."
3. **The bypass is a submit-path decision and the backlog is already past it.**
   `corpus_status_with_pii_backstop_hold` runs once, at submission. The 248
   held traces are already on `AwaitingPiiBackstop`; nothing this design adds
   re-examines them.
4. **The queue's drain rate is not gated on per-trace classifier work anyway.**
   `run_pii_backstop_driver_tick` (`trace-commons-ingest.rs:40624`) gates the
   **whole tick** on a live classifier canary round-trip
   (`trace-commons-ingest.rs:40645-40651`): if the classifier is unhealthy the
   tick aborts before a single submission is enumerated. A per-trace classifier
   skip inside the driver would never be reached. The pilot's measured
   constraint is host CPU -- the local embedder saturating two vCPUs -- and a
   batch size of 5 per 45-second tick, neither of which this change touches.

**The finding:** this slice is a correctness and trust change, not a throughput
change. It should be built and shipped disabled because it is the payoff the
witness exists for, and because building it later against a live contributor
population is worse. It should **not** be presented to anyone as the fix for
the held backlog. The backlog needs the drain-rate work and the risk-verdict
policy decision, and both are elsewhere.

---

## The binding problem, which the witness service did not solve

This is the largest piece of real work in the slice, and the witness service
plan did not deliver it.

`verify_witness_certificate(certificate, signature_hex, pin, redacted_bytes)`
compares the certificate's `redacted_sha256` against the digest of
`redacted_bytes`. The server must therefore hold, byte for byte, the artifact
the witness signed.

It does not. `WitnessResponse::redacted_artifact` is a `String` -- redacted
**transcript text** (`witness_service/mod.rs:124`). The submit handler receives
`Json(mut envelope): Json<TraceContributionEnvelope>`
(`trace-commons-ingest.rs:12836`) -- a structured envelope whose
`events[*].redacted_content` are fragments the contributor derived from that
text. There is no function anywhere in the tree that maps one to the other, and
in general the mapping is not invertible: the envelope splits, canonicalises
and re-encodes.

Three ways out were considered.

- **Recompute a canonical projection of the envelope and digest that.** Fragile
  by construction: the digest then depends on a projection function that both
  sides must implement identically forever, which is the drift the certificate
  exists to remove. Rejected.
- **Carry the witnessed artifact inside the envelope.** The digest cannot cover
  a structure that contains it. Rejected.
- **Make the witnessed artifact the submitted bytes.** The witness builds the
  envelope itself and returns the exact bytes the contributor will POST; the
  server digests the raw request body. Chosen.

The third is the only one where the certificate binds what the server actually
holds, and it is what the certificate's own doc already assumes -- "Any
re-encoding, wrapper or added trailing newline between here and the server
fails closed."

### The witness builds the envelope; the client is a courier

Settled jointly with the client plan. The witness takes a
`RawTraceContribution` (`trace_contribution.rs:1568`) plus the grant lists,
calls `DeterministicTraceRedactor::redact_trace`
(`trace_contribution.rs:4102`) -- the originating pass, which is what the
certificate is supposed to describe -- serialises the resulting
`TraceContributionEnvelope` **once**, digests those bytes, and returns them
verbatim. The contributor forwards them unmodified.

**So the server does not receive a client-assembled envelope**, and nothing in
this design may assume it does. Two consequences reach the server side:

- **The client stamps nothing after witnessing.** The grant-derived fields a
  client would ordinarily write onto the envelope after redaction travel in the
  witness request instead, so they are inside the digest. A re-mint on the
  witnessed path refuses with `witness_claim_expired` rather than rewriting the
  body -- any post-witness rewrite is a digest mismatch and would fail closed on
  a perfectly honest submission. The server's existing grant enforcement is
  unchanged; it reads the same fields, which now arrived witnessed.
- **The witness service changes.** `witness()` currently runs
  `DeterministicTraceRedactor` over its input as flat text. It must call
  `redact_trace` instead. That is a prerequisite task in the plan, not an
  assumption.

### The digest must be taken at receipt, before anything mutates the envelope

**This is the constraint that most easily gets built wrong.**
`rescrub_trace_envelope` takes `&mut envelope` and rewrites
`privacy.residual_pii_risk` (`trace_contribution.rs:4350`),
`redaction_counts`, `pii_labels_present`, `redaction_pipeline_version`,
`warnings` and `redaction_hash`. **The stored bytes are therefore never the
received bytes.** A certificate verified against a stored or re-serialised
envelope fails on a perfectly honest submission.

So verification happens at receipt: digest the raw body, verify the
certificate, and carry the resulting `VerifiedWitnessCertificate` forward as a
value. It must run **before** `trace-commons-ingest.rs:12911`, and never off
storage or off any later serialisation. Taking the body as `Bytes` rather than
`Json` is necessary for this and is not sufficient on its own -- a handler that
took `Bytes` and then verified after the rescrub would be just as wrong.


---

## Where the certificate travels

Headers, not the body. The body must stay byte-identical to what the witness
signed, so nothing may be added to it.

- `X-Trace-Commons-Witness-Certificate` -- the certificate, as compact JSON,
  base64url without padding.
- `X-Trace-Commons-Witness-Signature` -- the EIP-191 signature, `0x`-prefixed
  hex, 65 bytes.

Both absent is the ordinary un-witnessed submission and changes nothing. One
present without the other is a refusal, not a fallback: a submission that meant
to be witnessed and half-arrived must not silently be treated as unwitnessed.

The certificate has no `Serialize` impl, deliberately -- a
`serde_json/preserve_order` change moved every untyped-JSON digest in this
workspace on 2026-09-01. Header parsing therefore needs an explicit
field-by-field decoder, not `serde_json::from_slice`, and the signing bytes are
recomputed from the decoded fields through the existing length-prefixed
encoder. A decoder that reordered fields cannot change what is verified.

---

## Configuration, following the house pattern

The precedent is `crates/trace-commons-server/src/near_attestation/measurements.rs`:
`EXPECTED_MEASUREMENTS_ENV` at line 28, `EXPECTED_MEASUREMENTS_CONTROL` at 31,
`expected_measurements_from_env()` at 39 returning `Ok(None)` for unset -- and
the doc comment stating that `Ok(None)` "is *not* an acceptance". The refusal
names the control.

`WitnessPin::new` already enforces the fail-closed half: an empty measurement
set is `WitnessPinError::NoMeasurements`, a blank entry is
`MeasurementBlank`, a malformed address is `SigningAddressMalformed`. There is
no half-configured pin.

New variables, all of them AGPL-side server config:

| Variable | Meaning |
|---|---|
| `TRACE_COMMONS_WITNESS_BYPASS_ENABLED` | Master switch. **Default false.** Off, every certificate is ignored and every content-bearing trace holds exactly as today. |
| `TRACE_COMMONS_WITNESS_SIGNING_ADDRESS` | The pinned signing address. |
| `TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS` | Comma-separated `mrtd:...+mrconfigid:...` strings. |
| `TRACE_COMMONS_WITNESS_ALLOWED_POLICY_VERSIONS` | Comma-separated allowlist of `redaction_policy_version` aliases. |

Fail-closed shape, exactly as the NEAR AI precedent:

- Bypass **enabled** with no address, no measurements, or no policy allowlist
  is a **boot refusal** naming the missing control. It is not a silent fallback
  to "hold everything", because an operator who set the switch and got the old
  behaviour would conclude the feature does not work and look in the wrong
  place.
- Bypass **disabled** with a pin configured boots fine and ignores the pin.
  That is the staging posture: pin first, verify the measurement matches a real
  deployment, then flip the switch.
- A certificate that arrives while the bypass is disabled is not verified and
  not an error. It is ignored, and a hash-only counter records that one
  arrived.

The missing-control name for the measurement half is already spelled:
`EXPECTED_MEASUREMENT_CONTROL = "witness_expected_measurement"`
(`verification.rs:60`).

### The policy allowlist is load-bearing, not decoration

A `deterministic-only` witness never runs a classifier at all. Its
`redaction_policy_version` carries the deterministic alias, and
`deploy/witness/README.md` says a server requiring the classifier "can and
should refuse the certificate."

This is the sharpest edge in the design. Skipping the server's classifier for a
trace whose witness also skipped the classifier means **no classifier ever sees
that trace's prose**. Prose PII -- a name, an address, a phone number -- is
exactly what the deterministic pass cannot detect. So:

**The allowlist must be configured to contain only `full-pipeline` aliases.**
The plan ships a test that a deterministic-only alias is refused by name, and
the operator doc states the rule. Putting a deterministic-only alias in the
allowlist is a way to configure a real hole, and nothing in the code can tell
it apart from a deliberate one, so it is called out loudly instead.

---

## The decision, precisely

At `corpus_status_with_pii_backstop_hold`
(`trace-commons-ingest.rs:56685`), the hold is skipped when **all** hold:

1. the bypass is enabled;
2. a certificate and a signature both arrived;
3. `verify_witness_certificate` returned `Ok` -- signature against the pinned
   address, measurement in the pinned set, digest against the exact request
   body **as received, before `rescrub_trace_envelope` mutated it**;
4. the certificate's `residual_risk_verdict` is `Low`;
5. the certificate's `redaction_policy_version` is in the allowlist;
6. **and the server's own post-rescrub `risk_status` is already `Accepted`** --
   which is the existing precondition of the hold and is what keeps the
   residual scan authoritative.

Any of 2-5 failing on an otherwise valid submission is a **refusal of the
bypass, not of the submission**: the trace holds exactly as it does today. A
malformed or non-verifying certificate on a submission that would otherwise be
accepted must not reject the trace, because that turns a witness outage into a
submission outage. It is logged hash-only and counted.

Condition 6 is what makes this design's safety argument load-bearing rather
than decorative, and it is free: `corpus_status_with_pii_backstop_hold` already
takes `risk_status` and already returns it unchanged unless it is `Accepted`.

### What the certificate buys, stated as narrowly as it is true

Two things, and no third.

**Classifier evidence the synchronous pass structurally cannot have.**
`rescrub_trace_envelope` runs `resolve_post_scrub_risk` with
`useful_classifier_result: false`, so it can only ever raise risk -- it has no
classifier and therefore no evidence that could license lowering a floor. The
hold exists to go and get that evidence asynchronously. A `full-pipeline`
witness already produced it, over the originating pass, and signed the verdict.
That is the whole of what the hold is being excused from.

**Attribution to a known program.** The verdict comes from an image whose
measurement the operator pinned and the contributor verified before sending,
rather than from an unauthenticated field on a submission.

It buys **no** licence to skip the trailing deterministic sweep, the structured
pass or the residual scan -- and does not need to, because all three already
ran synchronously on these bytes before the decision is reached. Any wording
broader than the two paragraphs above is over-claiming.

---

## The surfaces that read the hold state

The witness service plan says "four surfaces read the hold state, and a bypass
must account for all of them." **It is not four.** Fourteen distinct sites read
`AwaitingPiiBackstop` outside tests, in five groups. They are listed because
the count being wrong by three-and-a-half times is itself worth recording, and
because the group that matters is not the one the plan named.

**Group A -- the writers (2).**
`trace-commons-ingest.rs:12954` (submit) and `:19703` (quarantine re-scrub).
Both call `corpus_status_with_pii_backstop_hold` at `:56685`. **This slice
changes the first only.** The re-scrub path has no request-borne certificate
and is left alone.

**Group B -- counters that fold the hold into a coarse tally (4).**
`:15448` `pending_review`; `:71981` contributor credit summary;
`:74226` corpus summary (precise count preserved in
`by_status["awaiting_pii_backstop"]`); `:69944`
`TraceRankerTrainingLabel::from_status`. All fold `Quarantined |
AwaitingPiiBackstop` together. A bypassed trace is simply never in these
tallies; none needs a change.

**Group C -- the contributor receipt (1).**
`receipt_from_record` at `:56738` -- "Held pending an automated privacy
backstop verdict; not yet in the corpus." A bypassed trace takes the `Accepted`
arm. **This one needs a change**: the accepted receipt currently cannot say
whether the trace was admitted on the server's own pass or on a witness's, and
a contributor is entitled to know which. One additional sentence, on the
`Accepted` arm, when the record was admitted under a verified certificate.

**Group D -- credit (1).**
`:19728-19730` exempts `AwaitingPiiBackstop` from credit zeroing so a held
trace keeps its pending credit. A bypassed trace is `Accepted`, which is the
non-zeroing branch a fortiori. No change; a test pins it, because "the bypass
quietly zeroed the credit" is a silent failure.

**Group E -- the queue machinery (6).**
`list_submissions_awaiting_pii_backstop` (`db/mod.rs:1241`,
`db/postgres.rs:5052`, SQL predicate at `:5099`);
`release_pii_backstop_hold` and `requeue_quarantined_for_pii_backstop`
(`db/trace_corpus_pg.rs:1954`); and three admin routes at
`trace-commons-ingest.rs:7706` `/v1/admin/requeue-pii-backstop`, `:7719`
`/v1/admin/pii-backstop-requeue-quarantined`, `:7723`
`/v1/admin/pii-backstop-clear-stale-prior-risk`. A bypassed trace never enters
this machinery. **The requeue routes are the one real interaction and it is a
deliberate non-change:** `pii_backstop_requeue_quarantined_handler` at `:51887`
moves quarantined submissions **back to `AwaitingPiiBackstop`** for
re-assessment. A requeued trace has no request context and no certificate, so
it re-enters the classifier queue exactly as it does today. A bypass that
survived a requeue would be a bypass an admin cannot undo, which is worse.

Also touched incidentally and needing no change:
`storage_derived_status` (`:60062`, held rows stay `Current`),
`audit_action_for_status` (`db/trace_corpus_common.rs:37`), and the
withdrawal object-key sweep at `:15877` which enumerates every status.

Two further reads are a class rather than a site: roughly forty
`status == Accepted` consumer gates enforce the hold off stored status. A
bypassed trace is `Accepted`, so it passes them -- which is the entire point of
the feature and the reason conditions 1-6 above are what they are.

---

## What I falsified from the brief

- **"Four surfaces."** Fourteen, in five groups. The group the plan named --
  the tallies -- is the one that needs no change at all; the receipt and the
  requeue interaction are the ones that do.
- **"`residual_pii_risk` is a client-computed field the server trusts."**
  Half right, and the half that is wrong changes the design. It arrives
  client-computed, but the submit handler runs `rescrub_trace_envelope`
  (`:12911`) before reading it, which **overwrites** it
  (`trace_contribution.rs:4350`). What survives of the self-report is narrower
  and more precise: `resolve_post_scrub_risk` is called with
  `useful_classifier_result: false` on that path, so the synchronous pass can
  only ever **raise** the risk. The client's value is a **floor**, not the
  value. A client asserting `Low` is believed exactly when the server's own
  deterministic pass and residual scan find nothing to contradict it. That is
  still authorization by self-report and the witness still replaces it -- but a
  design that assumed the field was taken verbatim would have over-claimed what
  the certificate buys.
- **"A classifier-stage skip would relieve the queue."** No. See "What a bypass
  saves". Four independent reasons, of which the sharpest is that the driver's
  canary gates the whole tick before enumeration, so a per-trace skip inside
  the driver is unreachable.
- **Implied: the bypass belongs in the backstop driver.** It belongs at submit.
  The driver is behind a whole-tick canary gate and only ever sees traces that
  are already held.

## What this design does not do

- **It does not touch the held backlog.** Nothing here re-examines a trace that
  is already on `AwaitingPiiBackstop`.
- **It does not change `status_for_risk`,** any risk tier, or any quarantine
  decision.
- **It does not verify a certificate on the `approved_envelope` path.** That
  path reuses a previously built envelope and never re-redacts; the witness
  spec already records it as permanently unwitnessable without build-time
  witnessing.
- **It does not add a client.** No contributor shell emits a certificate after
  this slice, so the feature is unreachable in production until that lands.
- **It does not claim a witnessed trace is clean.** It claims a known program
  in a pinned enclave reached a `Low` pass verdict, and that the server's own
  synchronous pass agreed.
