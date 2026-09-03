# Cline session fixtures

Transcribed from upstream source, not captured from an install: no Cline was
available on the machine these were written on. Shapes follow `cline/cline`
main (extension 4.1.17) as of 2026-09-03:

- `sdk/packages/core/src/services/session-data.ts` -- `buildMessagesFilePayload`
  is the `.messages.json` wrapper (`version`, `updated_at`, `agent`,
  `sessionId`, `origin`, `messages`, optional `system_prompt`).
- `sdk/packages/shared/src/llms/messages.ts` -- `MessageWithMetadata` and the
  `text` / `thinking` / `tool_use` / `tool_result` / `image` blocks.
- `sdk/packages/core/src/session/models/session-manifest.ts` -- the sibling
  `<id>.json` manifest.
- `sdk/packages/shared/src/storage/paths.ts` -- `~/.cline/data/sessions` and
  the environment variables that relocate it.

Until a session written by a real Cline is dropped in here and the tests
still pass, treat the reader as unverified against the wild.

Layout:

- `1756900000000_k3x9q/` -- a full session: manifest, thinking, a tool call
  and its result, per-message model and metrics.
- `1756900100000_p2m7z/` -- no manifest, string content, a failed tool
  result, an image block, and an unknown block type.
- `1756900200000_bad00/` -- a document with no `messages` array. Must be
  refused, not offered as an empty transcript.
- `not-a-session/` -- a directory with no messages file. Must be skipped.
