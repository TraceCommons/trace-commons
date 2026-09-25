# Contributor inference connection selection

The versioned inference-connection capability offers operator-configured
descriptors. An authenticated contributor must explicitly select the exact
offer revision, configuration digest, and disclosure version. The selection is
recorded against a live account with a verified NEAR anchor. A device bearer
must not be allowed to select or disconnect. Cookie writes require the existing
same-origin and CSRF controls; native account tokens retain their rotation
response. An offer list or account login never creates a selection.

The offer list contains identifiers and digests only. The selection response is
the only response with witness installation configuration. The operator owns
the URL, signing address, measurement pins, and optional inference receipt
endpoint. The server has no provider credential exchange or provisioning API.
An absent receipt endpoint carries no inference provenance claim.
Do not advertise the new capability until the operator catalog is loaded and
validated; there is no fallback endpoint or inferred provider credential.
At startup, `TRACE_COMMONS_INFERENCE_CONNECTION_CATALOG_JSON` accepts at most
16 operator-reviewed descriptors. The value is a JSON array; each entry has
`offer_id`, `provider_id`, `disclosure_version`, `witness` (`url`,
`signing_address`, `expected_measurements`), and optional
`inference_receipt_endpoint`. `disclosure_version` must be
`inference-connection-disclosure-v1`. Identifiers are unique and the server
refuses invalid URLs, pins, and duplicate IDs at startup. An unset value means
there are no selectable offers. Replacing an entry changes its revision and
requires contributor reselection.

Selection is an account record, not permission for a new device to install or
send to the witness. Every device must ask for explicit installation. A
selection also grants no folder, trace, raw-session, or standing contribution
consent. A retired revision must surface `connection_reselection_required`;
the current operator configuration must not silently replace what was selected.
Disconnect stops new cooperating-client use after it is observed. It does not
recall a transcript already disclosed or revoke an external provider credential.
An offline device may continue to hold prior settings until it reconnects and
applies the revocation; witness authorization also follows its existing
credential lifetime.

Keep the existing enrollment capability and response shape for old clients.
`GET /v1/account/near/provision/capabilities/v2` is the opt-in contract. It
reports login readiness and the authenticated connection route paths without
returning installable witness fields. Route clients to this contract only after
they implement explicit selection and installation. K12 must
prove login alone writes no witness, decline leaves it disabled, a selected
response is installed exactly, existing local explicit configuration survives,
and a new device asks again. K8 must require re-consent after provider or pin
changes. Connection success with no verified inference receipt remains
`unknown/unattested`; a redaction witness and inference receipt are distinct.
Client eligibility must use Z1's exact full-pipeline allowlist. Production
activation depends on K12/K8 migration, the settled account-trust eligibility
policy, and Z1/Z2 provenance verification. Provider provisioning needs its own
credential and revocation contract before any `connected` claim.
