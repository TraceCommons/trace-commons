# Package and promotion qualification

This specification defines package trust and Phase 7 promotion evidence.

## Package signature

The package signature uses Ed25519. The signature input is the ASCII package
hash from `BundlePackage::package_hash()`.

The canonical package bytes contain these fields:

1. The domain `trace-commons-bundle-package`.
2. The bundle identifier.
3. The canonical bundle manifest.
4. The sorted set of artifact hashes.

Package validation checks each artifact before signature validation. Thus, the
signature binds the policy and implementation identifiers and all configuration
and data artifacts.

The signature does not bind policy code. Policies are Rust code in the server
binary, and the package does not contain them. The qualification record below
binds the package to the code revision that it tested.

The trust store maps a bounded key identifier to one Ed25519 public key. An
unknown key, algorithm, implementation, or artifact causes a safe refusal.

## Registration and retention

`pipeline_bundle_packages` stores the immutable package. The
`pipeline_bundle_qualifications` table stores its immutable qualification
identity.

The qualification identity contains only hashes, a key identifier, and a
timestamp. Lab reports and drill details stay outside the ingest database.

Production activation uses `activate_qualified_bundle`. This operation requires
a current passing promotion decision. It also requires the deployed code
revision and the current production dependency profile.

The code revision must match the qualified revision. All production
dependencies must remain available.

This check runs only at activation. A later deploy of a different revision is
not checked again. A check after each deploy is a promotion requirement; the
[roadmap](../../trace-commons-roadmap.md) tracks it. The operation selects only a qualified
package with four runnable policies.

Foreign keys prevent package removal while a run, active selection, or
qualification refers to the package. Readers validate the package each time
that they load it.

## Production dependency profile

The promotion gate requires production implementations for these adapters:

- PostgreSQL metadata
- Encrypted object storage
- Key wrapping
- Authentication
- Scoring
- Embedding
- Vector index reads and writes
- Settlement

The gate refuses plaintext fallback, best-effort database mirrors, static
bearer authentication, HS256 bridges, and unversioned policy dependencies.

Tests and local corpus runs keep external payout disabled. Synthetic adapters
cannot satisfy a production dependency profile.

## Promotion evidence

Each drill result contains a safe drill identifier, status, time, maximum age,
and evidence hash. The result can also contain safe blocker labels.

Promotion requires current passing evidence for all drills in `OPS-004`.
Missing, failed, future-dated, or stale evidence blocks promotion.

Use a promotion decision within 15 minutes. The activation operation rejects
an older decision.

The report identifies the code revision, bundle, corpus, configuration,
contract manifest, inventory, restore evidence, tests, and drills. Reports do
not contain trace text, secrets, account identifiers, or transaction hashes.
