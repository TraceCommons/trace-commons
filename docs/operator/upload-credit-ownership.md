# Upload identity and credit ownership

An account is the durable owner of a contribution's earned credit. A device
proves that it may upload; an upload claim identifies the authenticated source.
A payout organization is a destination explicitly chosen by the account.
These identifiers serve different purposes and must not be substituted for one
another.

## Ownership contract

- Preserve the signed-claim principal on submissions and credit ledger events.
  It remains the provenance and direct authorization identity.
- After verifying a provisioned personal device's signature, record an immutable,
  tenant-scoped association from the derived upload principal to the device
  principal before returning an upload claim. A conflicting association fails
  closed. The durable provisioning record determines this behavior even after
  account membership is unlinked.
- Legacy shared-instance devices without a personal provisioning record retain
  their existing distinct per-user login principals and account ownership. Do
  not reserve those principals as upload-only aliases or collapse their users
  into one device account.
- Account-wide contribution reads, credit summaries, starter-credit plausibility,
  settlement grouping, credit holds, and payout accrual resolution use the same
  ownership relation.
- An association resolves through the device principal's active account
  membership. Explicit account merges follow that membership; removing membership
  removes the association from account-wide expansion. Device revocation blocks
  new uploads without erasing previously earned credit.
- Ownership associations are not login credentials. Native account operations
  continue to require their existing account authentication and scopes.
- Issuing another claim, refreshing a session, and retrying payout must not create
  duplicate awards, settlement lines, or payments. A claimed accrual keeps its
  recorded owner; the payout worker's existing idempotency rules still apply.

## Identity privacy boundary

The upload-to-account association is private service data, not training or
dataset metadata. V135 grants access only to the ingest runtime; it grants no
access to dataset consumers or PUBLIC. An ownership alias does not authorize
identity lookup.

NEAR AI identity recovery continues through the existing database and KMS
boundary: login subjects have domain-separated HMAC blind indexes, and the
recoverable subject is AES-256-GCM encrypted under a per-row key wrapped by KMS.
Tenant IDs are random. The upload aliases derive from the device and tenant,
not from a NEAR AI subject or account name. Do not publish the ownership relation,
login blind index, sealed subject, payout destination, or identity decryption
capability with an export. Keep identity-service KMS access separate from data
consumer access.

These controls protect the service's identity linkage. They do not by themselves
remove identifying text a contributor might include inside a trace; content
redaction and export admission remain separate requirements.

## Historical credits

Existing submission and ledger identities must not be rewritten. Repair an
association only from independently verified device evidence and the exact actor
used to derive the stored signed-claim identity. Tenant coincidence, a matching
organization name, or an inference API key is not ownership proof.

The native client's historical subject is the protocol's domain-separated
`user_subject_hash(device_key_id)`. This permits an exact derivation from a
registered device for that client version; arbitrary subjects cannot be inferred.
A repair must match the derived principal to stored records and reject ambiguous
or conflicting ownership. Already paid accruals must not be reassigned or paid
again.

## Payment timing

Ownership, earning, settlement, and payment are separate states. An active payout
destination alone does not create an award. Scoring must produce eligible credit,
a settlement operation must finalize it, and a live payout worker must claim and
send it. The worker wakes on relevant changes and retries approximately every
minute. Automatic settlement also requires its own scheduler configuration.

Do not relax scoring gates or create synthetic awards to hide a missing ownership
association. Report an unassigned credit or a disabled settlement scheduler as
such rather than promising a payment time.

## Deployment order

Apply V135 with the new ingest before restarting the new upload-claim issuer.
The old issuer can continue to serve while the additive schema is installed.
The new issuer requires the ownership table; starting it against the old schema
correctly refuses claims. Keep the issuer signing key unchanged during this
rollout. With the existing installer, deploy `ingest <tag>` first, verify its
health and schema, then deploy `issuer <tag>`.

Verify the new association with an account-authenticated summary and the payout
worker's persisted accrual and line states. An activated destination or a
successful worker pass alone does not prove payment.

## Repair command

Build `trace-commons-repair-upload-ownership` from the same revision as the
ownership migration. Staging Cloud Build publishes the binary and its SHA-256
sidecar with the other staging artifacts; verify the sidecar before running it.
The command never runs migrations. Deploy the schema first.

Supply `DATABASE_URL` through the operator's protected environment. Select the
verified tenant and registered device using `TC_REPAIR_TENANT_ID` and
`TC_REPAIR_DEVICE_KEY_ID` (or `--tenant-id` and `--device-key-id`). Keep these
values out of shared logs. The command emits only a matched-submission count,
mode, and identity hashes. Its PostgreSQL role must have the ownership table's
write grants for apply and the existing tenant-scoped read grants; do not use
an unscoped administrator session as the application runtime.

```sh
# Read-only preview; checks the exact historical native-client identity.
trace-commons-repair-upload-ownership

# After checking the intended tenant/device and preview count:
trace-commons-repair-upload-ownership --apply
```

The count includes matching submissions whose association already exists, so
repeating apply is safe. Zero matches means no historical association was added.
The command refuses conflicting ownership or a device without the required live
provisioned account membership. It cannot repair arbitrary user subjects by
inference.

Applying the association does not create credit, rewrite a settlement, select a
payout organization, or call a payment API. **It can make already-settled credit
claimable by the running payout worker**, which may then send it to the account's
confirmed destination. Check existing accrual/payment rows before apply and the
worker's confirmed receipt afterward. A successful repair report is not evidence
that a payment completed.
