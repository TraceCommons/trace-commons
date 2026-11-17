# Server identity source mapping

Current tracked baseline: `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`.
This is an executable finite abstraction for reviewing current server identity
behavior. Owner promise selection is pending. No invariant, property, TLC run,
state exploration, security proof or application behavior change is included.
The panels are explanatory server states, not application UI proposals.
Primary-checkout untracked passkey and binding additions are excluded from
this baseline and were not read as implementation evidence.

## State and boundaries

`challenges` keeps the start-time verifier identity, device, and wallet account
separate from finish-request values. Native starts have no account authority;
a successful browser redeem supplies the stored code's account and tenant.
NEAR AI account authority instead comes from the verified introspection result,
never a finish-request account field. `links` and `codes` represent distinct durable
browser-login and headless-device code records. `links.native` is the browser
request's optional association, folded into the link slot for this finite model;
it is not a persisted login-link database column. Challenge phase `taken` means
the stored handle has already been removed: even a verifier mismatch cannot
retry that same handle. Expired and replayed handles receive refusal.

`sessions` keeps account, tenant, optional device attribution, transport kind,
current token generation, previous token/grace and clock boundary flags.
`auth` holds the request's validated account/tenant snapshot separately from
an arbitrary claimed account and the body's independently supplied minter key. The validation snapshot also stores the canonical current-token identity and
rotation eligibility read by the database SELECT. Crossing the rotation-age
boundary after that SELECT does not newly qualify the in-flight request. A
request validated before a concurrent revoke can finish with its saved context;
a planned rotation still checks the current row's token and liveness when its
UPDATE runs, and a losing rotation does not mint an unstored token. `responsesResolved` marks a delivery attempt and `received` marks actual
symbolic delivery. Neither proves client credential persistence, and a lost
response does not roll back a session. The client identity model owns local
storage/generation behavior.

`deviceOwner`, `deviceOrigin`, `revoked`, `grants`, and `renewable` remain
separate. Provisioning never revives a revoked key or moves an existing key to
another account/origin. Native sessions are weak account sessions regardless
of the browser ceremony that approved them. Transport validation is asymmetric:
a native bearer reaches database session validation before the native-kind
check. Validation may update last-seen or rotate a non-native row, after which
the resolver refuses account authority and sends no rotated token. Mixed
credentials and device upload bearers are refused before session validation.
A cookie may name
any live supported session row, including a native-kind row. Cookie use of a
native row remains SessionCookie transport with weak native-kind authority;
operations requiring NativeToken transport, such as device-code mint, refuse
that cookie request. An upload claim does not become an
account session. Current logout revokes its entire session row, including its
rotated predecessor. Account-wide logout revokes account sessions while keeping
keys and grants. Device revoke changes the key, grants, keyed sessions and
other live unattributed native sessions; its caller's own unattributed session
is spared, but its caller's keyed session is revoked when it names that key.

Native browser-login-link mint is a coarse authenticated-principal interface.
The implementation does not look up device-key revocation at mint: it checks
the bearer and, when configured, the access grant. The model allows the grant
gate to be optional rather than inventing an extra live-key check there.

## Source correspondence

All anchors below are in the pinned worktree, not the primary checkout's
untracked files. Paths are repository-relative; line numbers name the inspected
function or check, not an entire subsystem's proof boundary.

| Source anchor | Actions / represented boundary |
| --- | --- |
| `crates/trace-commons-server/src/account_native_auth.rs:54` (request/code TTL), `:170` (loopback URI), `:197` (verifier shape), `:257` (saved request/code) | `YouStartASignInCeremony`, expiry; native start shape/redirect/S256 validation is an admitted-input prerequisite |
| `crates/trace-commons-server/src/account_ceremony.rs:1`, `:86` (`put`), `:100` (`take`) | `TheIngestProcessRestarts`, single-use native stores; in-process native state is lost while persisted provisioning rows survive |
| `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs:21706` (`mint_login_link_handler`) | `YouMintABrowserLoginLink`; authority/account creation and grant configuration are coarse inputs; no direct live-device-row check |
| same ingest file `:21916` (`native_authorize_start_handler`), `:22012` (`issue_native_authorization_code`), `:22060` (`native_token_inner`) | Start, browser code issuance, `TheFinishRequestTakesTheStoredCodeOrCeremony`, saved request/verifier checks, session issue; code take precedes mismatch checks |
| same ingest file `:23330` (confirm cookie/form evidence), `:23398` (atomic redeem), `:23425` (native fallback) | `YouConfirmTheBrowserLogin`; browser success persists when native request is absent/expired; origin/cookie/form security is one external evidence predicate |
| `crates/trace-commons-server/src/db/postgres.rs:4397`, `:4857` | Atomic browser link consume/session/audit; native session insert/audit; `TheSessionIssueTransactionCommitsOrRefuses` |
| `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/near_provisioning.rs:601`, `:691`, `:744` | Wallet start/take/verification/issue; saved verifier, account, device and callback binding, wallet/device signatures then ownership RPC; unavailable after take consumes ceremony |
| same provisioning file `:1371`, `:1622`, `:1655`, `:1466` | NEAR AI start/take, saved device/PKCE proof, then introspection, account/device/session transaction |
| `crates/trace-commons-server/src/db/postgres_account_onboarding.rs:42`, `:124`, `:316`, `:561`, `:604` | Key origin/revocation/conflict checks, account/device link, persisted single-use take, NEAR AI identity/account resolution; provisioned device versus admission grant kept distinct |
| `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/account_routes.rs:142`; main ingest `:19835`, `:19957`, `:20017`, `:20081` | `TheAccountRequestPresentsCredentials`, `TheRotationUpdateAndMiddlewareFinish`; middleware dispatch/rotated response, cookie plus bearer refusal, device-bearer refusal, database validation effects before native-bearer client-kind restriction, cookie acceptance of native-kind rows, weak authority |
| `crates/trace-commons-server/src/db/postgres.rs:4475`, `:4550`, `:4590`, `:4699`, `:4733` | Saved validation snapshot, current-only rotation, previous-token grace, race-losing update, idle refusal, current/all-session revoke |
| main ingest file `:23497`, `:23574` | `YouSignOutTheCurrentSession`, `YouSignOutEverywhere`; source comments above logout include an obsolete bearer-no-op description, but the actual native branch revokes the row |
| `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/account_devices.rs:1`, `:164`; `crates/trace-commons-server/src/db/postgres_account_devices.rs:127` | `YouRevokeAnAccountDevice`; transaction serializes keyed issue/revoke; foreign, unknown and already-revoked devices receive uniform not-found |
| `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/account_device_codes.rs:146`, `:305`, `:365`; `crates/trace-commons-server/src/db/postgres_account_device_codes.rs:41`, `:142`, `:251` | `YouMintAHeadlessDeviceCode`, `TheHeadlessDeviceCodeIsRedeemed`, `TheHeadlessDeviceRenewsWithItsKey`; native-only mint, independently signed body minter linked to auth-derived account, non-delegation for code-linked minters (`:58–64`), active minter, successful-only atomic code consume, code-linked renewal eligibility and per-tenant nonce replay |
| `crates/trace-commons-contributor/src/account_auth.rs:275`, `:290` | Local cancel/timeout abandons loopback wait; `YouCancelTheLocalCeremony` does not claim a nonexistent server cancel endpoint |

## Finite bounds and abstraction limits

- Two account identities, two tenant identities, and two devices. Account `a`
  belongs to tenant `a`; same-tenant multiple-account attacks are outside this
  seed. Initial device owners are independently unregistered or linked to
  either account; linked devices begin with a symbolic grant. Accounts already
  exist; account creation/resolution is an atomic identity boundary rather than
  a separately interleaved SQL process. Initial device origin is `near` or
  `near_ai`; an unregistered key's initial origin is overwritten on registration.
- Two start handles, two browser links, two headless codes, four session rows,
  two account-request slots, two verifier identities, token generations 0–2,
  and two nonce identities per tenant. Slots are not recycled. Repeated requests
  beyond these bounds, UUID/hash collisions, unlimited retries, and more than
  two rotations of one row are omitted. No concurrency completeness is claimed. Session `seen` records the first
  two last-seen mutations and saturates at two; it makes validation effects
  visible even when the resolver subsequently refuses native transport for a
  non-native row. It is an observation counter, not elapsed time.
- Clock flags represent crossing expiry, idle, rotation-age, previous-grace and
  absolute-cap boundaries. They are independent finite environment events, not
  real elapsed durations. Native TTL is 12 hours, rotation age 1 hour and hard
  cap 30 days in source; browser fixed expiry differs. `capped` distinguishes
  sliding from capped response outcomes, but no numeric expiry arithmetic is
  proved. The third-day idle threshold and grace durations are not simulated.
- `closed` is a frozen empty boundary input in this seed: account closure,
  unlink/merge/rebind, retired logins, strong authenticator registration/removal,
  direct NEAR browser login and payout-authenticator changes are explicit coarse
  interfaces owned by the suite's client identity/server interfaces coverage.
  This module does not turn their names into detailed ceremony coverage.
- Device list/rename and device-code status are coarse projections of stored
  ownership/phase, with label validation, counts and response payloads omitted.
  Login-link mint's create-or-reuse and principal-alias resolver are coarse;
  upload-only principals are excluded by that interface's admission predicate.
  Device-code mint requires native account-session transport and a signed
  body-supplied minter key linked to the authenticated account. The minter
  need not be the session's attributed device; an unattributed native session
  can mint by proving a linked key. A key that ever joined through a redeemed
  device code cannot mint another code (`renewable` records this history).
  Signature/key bytes, mint nonce, cap and timestamp checks remain one evidence
  input. Renewal signature/skew/minimum-interval/database success is
  one evidence input; successful renewal consumes a nonce, refused attempts
  leave it reusable. Timing and rate-limit policy arithmetic are omitted.
- Valid start/body/readiness/rate-limit prechecks are prerequisites. A request
  rejected before take leaves its record unchanged. The modeled take begins
  after those checks; malformed-body and unavailable-before-take responses are
  coarse refusal interfaces. Wallet and NEAR AI legacy/v2 configuration choices
  are folded into these prerequisites; no deployed enablement is assumed.
- Browser evidence represents same-origin plus matching cookie/form ceremony,
  not a proof that a human acted. S256/secret equality, signatures, signed nonce
  preimages, NEAR ownership RPC, NEAR AI introspection, TLS, loopback binding,
  browser cookie enforcement, OS credential storage and entropy are explicit
  external predicates. No cryptographic, browser, operating-system or network
  security property is established. Wallet account identity is represented by
  the finite account token, not raw NEAR text; introspection returns the finite
  account token after a successful evidence check.
- Native ephemeral requests/codes, durable provisioning rows, session rows and
  response delivery are separated. Restart does not model an in-flight Rust
  future surviving process death. Provisioning grant-template choice is folded
  into an optional grant outcome; grant policy details belong to other models.
  Referral side effects occur after provisioning commit and belong to the
  account/community interfaces, not session issuance.

## Verification status

Only static typechecking was run:

```sh
quint typecheck docs/flows/trace-commons/models/server_identity.qnt
```

Quint 0.33.0 accepted the module. Configuration contains only `INIT q_init` and
`NEXT q_step`. No generated statechart, invariant, property, fairness condition
or TLC result exists; owner selection remains pending. Frame/control labels
are supplied for the suite's correspondence validator.

## Integrated validation

Earlier typecheck-only status notes describe the model-writing stage. The later
[integrated validation report](../validation.md) records the suite-level
frame/control comparisons and sampled traces. Those checks do not establish
product guarantees; owner promise selection and TLC checks remain pending.
