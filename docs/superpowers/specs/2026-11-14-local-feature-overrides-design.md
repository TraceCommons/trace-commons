# Local feature flag overrides

Approved direction: per-machine default / on / off through CLI, config file and environment variables. User explicitly expanded scope to production as well as staging/testing, then excluded Settings controls and requested environment variables.

## Intent and scope
Enable Skills Insights on one installed app without changing other users or publishing a signed environment manifest. Generalize to registered product flags. No Settings controls or new UI copy. Persist by environment and immutable profile digest, across restarts of the same profile. Profile changes return to environment defaults. No dependency additions. Preserve the dirty primary checkout.

## Approaches
A local signed-cache injection requires operator signing and tangles revision watermarks; rejected in favor of a supported override layer. Environment-wide publication cannot satisfy per-machine scope. Implement daemon-owned persistent local choices and process-start environment choices above existing verified manifest/default evaluation. User chose the local layer, subsequently including production.

## Behavior
Only known product flags with all required build capabilities allow local overrides. Privacy-sensitive flags cannot be overridden; flags never grant consent, folder access, or upload permission. TRACE_COMMONS_FEATURE_FLAGS JSON choices override local file choices, which override remote product decisions and persist until cleared, without expiry. Signed manifests retain signature, expiry and rollback checks independently. Next-start flags preserve restart latching; live flags apply immediately. Clearing recomputes the current valid environment decision. Default is represented by absence, not false.

## Interfaces
Snapshot adds `local_overrides: {id: boolean}`, `local_override_allowed: {id: boolean}`, `environment_variable_overrides: {id: boolean}` and `environment_variable_error: string|null`; absent fields in older snapshots decode as empty. Existing source/revision/fresh fields describe the environment layer. `feature_flags_override_set` accepts `{id, value: boolean|null, environment, profile_digest}` and returns a verified-identity Snapshot. Identity validation precedes mutation. Mutations require the running daemon; no unsynchronized offline writer. CLI exposes `feature-flags set <id> on|off|default`. CLI prints effective state, local selection, environment-variable selection and pending restart. Config file edits require quitting the daemon then restarting; no live file watcher. Rust is the only evaluator. Existing native UI consumes effective flags without additional controls.

## Storage and failures
Atomic mode-0600 bounded local JSON separate from signed cache and rollback watermarks. Validate schema, profile identity, known IDs, class and capability on load and write. Invalid local data cannot enable flags or prevent app startup. Serialize mutations, persist before changing runtime state, publish feature_flags_changed after success. Environment values are read once on daemon startup and never persisted. Invalid environment JSON rejects the entire map and reports a stable diagnostic error without preventing startup. Environment refresh and local edits must not overwrite each other.

## Verification
Test precedence, clear, persistence, isolation, remote refresh/expiry, generation changes including same-effective-value changes, privacy/capability rejection, malformed storage, atomic failure, next-start latching, IPC identity rejection and notification, CLI parsing/mutation, environment-variable precedence and validation, and unchanged Swift decoding. Run contributor/FFI/Swift suites and relevant lint/build gates after all implementation tasks. No release publication or production deployment is part of this change.
