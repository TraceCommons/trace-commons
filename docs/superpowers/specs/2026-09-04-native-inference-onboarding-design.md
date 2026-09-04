# Native onboarding toward NEAR inference

Date: 2026-09-04

Status: implementation started with separate captured-inference consent in
macOS, Windows, and GTK. The broader flow below is a proposed target, not a
claim that account bootstrap, funded inference, or open admission exists.

Companions:

- [Admission policy](2026-09-04-admission-invite-or-attestation-design.md)
- [Execution plan](../plans/2026-09-04-native-inference-onboarding.md)
- [Attested-inference release](2026-09-04-attested-inference-release-design.md)

## User outcome

Help someone turn useful agent work into more inference. The activation
milestone is a useful NEAR-powered session followed by an accepted
contribution. Demonstrating the economic loop additionally requires earned
credit to fund another successful inference call.

Finishing setup is a separate milestone. A user must be able to leave the
wizard while a contribution is processing, return later, inspect a refusal,
and change permissions. Do not label pending points as available inference.

## What exists in this checkout

All three native shells use invite enrollment, contribution-use consent, an
optional third-party privacy scan, project selection, and a completion screen.
Tenant-keyed completion markers keep interrupted enrollment from skipping
consent. Source-root declaration can be required before the daemon starts.

The daemon can discover an existing IronWire proxy and observe its ledger.
That does not install the proxy, route an agent, enable body capture, fund
inference, or verify a receipt. The separate `ironwire_attested_bodies`
setting defaults to false and participates in approval invalidation.

NEAR login authenticates an already-linked identity. It does not bootstrap
a contributor tenant/account or enroll a new device. Native account auth
and device upload authorization are separate credentials and must remain so.

The account credit summary carries points, optional currency estimates,
settlement posture, and pending-review information. It does not expose an
available inference balance or implement redemption. The credit-numbers spec
references a redemption design that is absent from this checkout; that
missing contract is a dependency, not an implementation detail to guess.

Desktop preview currently returns `witness_claim_unavailable` for an enrolled
contributor with a witness configured (`daemon/preview.rs`). It cannot build
a witnessed envelope without the server-granted upload scopes. Direct
submission has a different path. Exposing consent does not remove this
refusal or make the native witness loop operational.

## Target journey

| Stage | User decision | Evidence needed before advancing |
|---|---|---|
| Discover | Choose a supported agent/project, or start without history | Local discovery only; no upload or body capture |
| Connect | Continue with NEAR or use an invite | Verified identity plus server-issued account/device enrollment |
| Start | Contribute existing work or use funded inference | Admission entitlement for submission; actual funding for inference |
| Review | Choose contribution uses and approve off-device processing | Separately persisted consent; inspectable contribution |
| Activate | Connect one agent and do a useful task | Observed provider call; routing configuration alone is insufficient |
| Continue | Review the result and next action | Processing/accepted/refused evidence; actual inference balance if available |

Discovering sessions must not require uploading them. For a fresh install
with no sessions, avoid an empty projects screen with no useful next action.
Offer funded inference if supported, or explain that an invite/funding is
needed. Do not fabricate starter credit.

An invite remains usable without making existing contributors adopt NEAR
inference. Linking a payout/funding identity can be a later step for that
path. The self-service window requires the verified NEAR identity anchor.

Wallet creation needs an explicit browser return/retry path. Account
creation sponsorship, if offered, is its own bounded subsidy; it does not
create evidence of a unique human.

## Keep four states separate

| State | Authority | What it must not imply |
|---|---|---|
| Identity and device enrollment | Authenticated server ceremony | Inference funding or permission to submit any trace |
| Admission entitlement | Server policy and atomic ledger | Quality acceptance or earned credit |
| Inference funding | Confirmed provider/redemption balance | A successful call or verified receipt |
| Setup and activation progress | Local persisted setup plus observed/server outcomes | Completing setup is not an accepted contribution |

Future capability discovery must be versioned and fail closed: absent or
unknown capabilities hide the associated action. Native clients must not
infer support from a provider name, a configured witness, a dollar estimate,
or a previous successful login. Refresh server state after browser return,
restart, device linking, and submission completion.

The planned first-contribution checklist belongs after setup and remains
available while processing. Use real local discovery, routing observations,
receipt counts, and server acceptance. A status error must render as unknown
or unavailable rather than a completed step. Include retry and recovery
without restarting consent or silently broadening it.

## First implemented slice: captured-inference consent

Add a separate section alongside witness settings in all three shells. It
uses the existing daemon setting and shared `witness_copy` text; no new
dependency, admission endpoint, certificate profile, or C ABI function.

The first action opens a disclosure. Only explicit confirmation saves
`ironwire_attested_bodies: true`. Cancel writes nothing. Disabling is always
available without a functioning witness or proxy. Each mutation contains
only that setting and must return a boolean matching the requested value
before the app confirms success. Missing/malformed responses and ignored
settings are failures, including a missing field when disabling.

Older settings decode as false. A missing setting key is not evidence that
the daemon supports enabling the feature. Unknown/stale state must not be
shown as a newly confirmed permission. Failed writes are visible; they do
not optimistically change the displayed saved setting.

The disclosure states:

- The final inference request and response can include prompts, history,
  tools, and secrets; they go to a remote witness before redaction.
- The witness checks the evidence and strips the attached bodies from its
  returned contribution. One call does not authenticate the whole session.
- Receipt lookup can reveal to NEAR AI which call is being contributed.
- IronWire capture is separately configured and stores bodies locally.
- Revoking this permission does not stop capture or delete existing bodies.
  Work already in progress may still finish.
- This setting does not fund inference, connect an agent, establish receipt
  verification, or make the currently blocked desktop witness review work.

Keep this consent independent of the optional privacy scanner, project
watching, automatic upload, witness configuration, and proxy observation.
Do not enable it during enrollment or migration. Its persisted setting is
not an attestation badge. Existing trust checks and upload refusals remain
authoritative.

## Dependencies before the target journey is offered

1. Native witness preview must obtain a scoped claim, build the witnessed
   envelope, and pin exactly the bytes and scopes later uploaded. Preserve
   fail-closed pin checks and approval invalidation across settings changes.
2. A real invited native user must complete capture, receipt retrieval,
   witness verification, and accepted submission with attached bodies absent
   from queued/stored envelopes. The release spec records this live chain as
   unvalidated; merged mocks are not deployment evidence.
3. Define the redemption/funding bridge: provisioning, scoped credentials,
   secure native storage, actual available balance, reservation, reconciliation,
   refunds/failures, and the credit-to-inference rate. Never embed an operator
   key in a desktop build or equate pending points with provider funds.
4. Implement explicit self-service account bootstrap and authenticated native
   browser return/device linkage, with replay and account-switch protection.
5. Implement account-bound provider-hashed challenges, signed admission
   evidence, receipt replay protection, and bounded window accounting from
   the admission spec. Redaction certificates alone cannot authorize entry.
6. Select window/subsidy values using measured processing costs and quality
   yield. Require both per-account and global exposure limits. Public
   self-service remains off until these dependencies are verified.

## Product decisions still open

The window sizes, cost ceiling, credit treatment, funding conversion, and
sponsorship source require concrete economic terms. This slice chooses none
of them. Users with no historical traces need existing funds or explicit
sponsorship to start inference. A free-submission window alone cannot help
them. A service outage must not silently extend economic limits or discard
already-earned credit; show the exact next available action.
