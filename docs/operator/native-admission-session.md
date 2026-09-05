# Prepare an admission-backed inference session

Native `prepare_admission_session {entry_id, backend, confirmed:true}` registers
an account-bound challenge for the selected session's **next** inference request.
A `ready_for_next_inference` response means registration succeeded. It does not
mean an inference ran, a receipt was verified, or a submission was admitted.
Existing inference calls cannot be retroactively bound.

Required configuration:

- A verified NEAR device enrollment with pinned witness settings and
  `admission_evidence=true`, plus an explicitly selected contribution purpose.
- Separate contributor body-export consent (`ironwire_attested_bodies=true`).
- An explicitly declared local IronWire proxy and its trustworthy control token.
- IronWire capability `supported=true`, `protocol="openai.chat"`, and
  `body_capture_ready=true`. Old proxies missing the capability are refused.
- A configured, funded backend with a compatible OpenAI Chat target and an
  existing agent route through that proxy. Translation to that target is allowed
  where IronWire supports it. Subscription-only or unsupported backend modes
  cannot be used. This operation changes no route, credentials, or capture flag.
- An enforcing Commons/issuer host allowlist and configured admission service.

The daemon resolves the queue entry back to a declared native source and reads
its exact session identifier from metadata: Codex `session_meta.payload.id` or
Claude Code `sessionId`. An opaque queue ID, rollout filename, or imported
trajectory name is never used as the proxy session key. The running agent must
continue that exact session and preserve its supported session header through
the local route.

The daemon checks consent before any network request, reads proxy capability,
mints a scoped claim, and requests `/v1/admission/challenge`. It checks canonical
encoding, the enrolled account anchor, matching expiry, and a lifetime of at most
15 minutes before authenticated loopback registration. It checks the config,
settings and session metadata again before that write. Only the session ID,
backend ID and canonical binding go to the local proxy; no transcript body
leaves during setup. The remote service receives no session ID or transcript.

The contributor must then continue the selected session. Future routing failure
or an expired binding refuses the inference path rather than silently emitting
an unbound request. The normal witness/preview/submission flow independently
checks evidence, redaction and admission limits afterward. This setup does not
create inference funds or consume an upload entitlement.

The required proxy implementation is in the separate IronWire repository; it
must be built and configured before this capability can become available.
