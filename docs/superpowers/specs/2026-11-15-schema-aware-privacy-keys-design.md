# Schema-aware privacy keys

The user approved building the mitigation discussed in the current session. Reproduced public inputs: standalone `fork_turns` and `explanation` are falsely labeled private persons; fixed JSON context is clean. Preserve all existing privacy, identity, consent and approval gates.

## Design

The permissive protocol crate owns a small versioned compiled catalog of exact public tool-property literals at fixed ToolCall/arguments paths with validated value kinds. It recognizes public syntax, not tool origin or authorization. Caller schemas, manifest hashes, tool-name substrings, nested maps and tool results cannot extend the catalog. Use reviewed parameter tables for spawn_agent, update_plan, exec_command and write_stdin; the generic contextual path handles every other tool and key. Catalog selection does not authenticate origin. The user explicitly requires held-out and adversarial generalization tests rather than an examples-only fix.

Unknown keys are inspected as serde-serialized synthetic JSON `{key:null}` without values. Only the serialized key token is eligible for a finding. Preserve the key and separately scan its value using the existing traversal. Invalid evidence, unavailable context inspection, credentials, out-of-token findings and exhausted budgets remain unresolved. Legacy ordinary scrub policy stays unchanged; the new behavior applies to manual review rechecks.

Retain raw classifier byte spans, canonical categories and optional confidence in private, nonserialized working data before overlap collapse. Fresh trusted review evidence records bounded findings with domain-separated length-prefixed hashed locations, fixed reasons, catalog version and optional bounded confidence. No raw keys, values, paths or custom labels enter diagnostics. Existing stored evidence deserializes with absent diagnostics. Diagnostics do not grant approval or replace complete coverage, useful classifier evidence or the final residual scan.

## Validation

Regression coverage: reproduced false positives; real PII in values and dynamic keys; exact tool/path/type matching; forged schemas/hashes; tool results; nested maps; escaped and Unicode keys; missing/malformed span provenance; out-of-token findings; missing confidence; bounded diagnostics; credential preflight and budget failure; atomic errors; private-span serialization. Run protocol feature/default/self-hosted tests, touched server/ingest compatibility checks, license boundary and focused lint/format gates. No new dependencies, production calls, deployment or trace reuploads are part of this implementation.
