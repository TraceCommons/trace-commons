# First-run daemon replies

The contributor daemon's replies to the calls the first run makes, read by
the Windows `FirstRunDaemonTests` so the shell is tested against what the
daemon sends rather than replies typed by hand (checklist B4,
`docs/superpowers/specs/2026-11-18-windows-one-pass-checklist.md`).

Each file is one whole response frame (`id`, and `result` or `error`), as
the contributor crate's `daemon::first_run_fixtures_tests` prints it between
its `BEGIN first-run/<name>` and `END first-run/<name>` lines under
`--nocapture`. Each request goes through the real dispatcher
(`ipc::handle_request_async`) on a temporary store. The tests also compare
each reply to the file here, so a wire change fails there until the file is
re-recorded:

```
cargo test -p trace-commons-contributor --lib first_run_fixtures_tests -- --nocapture
```

Nothing reaches a real service: an axum server on `127.0.0.1` stands in for
the issuer, the sign-in ceremony binds only its own loopback listener, and
the one host that is refused is under `.invalid`.

## Values replaced before comparing

These change on every run, so the test replaces them by field name before
it prints or compares:

- `attempt_id`: `00000000-0000-4000-8000-000000000001`.
- `browser_url`: the loopback port becomes `49152` and the `state` nonce
  becomes `fixture-state`.
- `granted_at`: `2026-11-19T00:00:00Z`.

## How each was produced

`first_run_join_replies_for_shell_fixtures`, on one store, in this order:

- `enroll.json`: `enroll` with an invite on the loopback issuer, whose
  `/v1/onboard` answers tenant `tenant-mock` and device key
  `sha256:mockdevice`.
- `set_consent_scopes.json`: `set_consent_scopes` with
  `debugging_evaluation` and `model_training`.
- `grant_automatic_including_existing.json`: the first run's grant
  (`confirmed`, witness `null`, `include_existing`, `keep_folder_asks`),
  given.
- `grant_automatic_including_existing_witness_changed.json`: the same grant
  naming witness `0xshown` while none is configured, refused with
  `automatic-grant-witness-changed`.
- `withdraw_automatic_grant.json`: `withdraw_automatic_grant` with
  `return_folders_to_ask`.
- `grant_automatic_from_now.json`: the older `grant_automatic` method, which
  grants from now on only. It is the daemon's real granted answer with
  `include_existing: false`. `grant_automatic_including_existing` never
  sends that answer; the shell refuses it if it does, and this is the reply
  its test uses.
- `set_settings.json`: `set_settings` with `{"claude_source":{"mode":"off"}}`.
  The echo holds the redacted settings: no path, key or session.
- `terms_accept.json`: `terms_accept` naming version `2026-11-01`. No terms
  are cached on the temporary store, so the async path makes no network
  check.

`first_run_invite_lookup_replies_for_shell_fixtures`:

- `invite_lookup_valid.json`: the loopback issuer answers a valid invite
  with a display name and a credit range.
- `invite_lookup_not_found.json`: the issuer answers `valid: false` with
  `reason_label: not_found`. The daemon never sends `valid: false` without a
  reason; it turns that answer into `invite-lookup-unavailable`.
- `invite_lookup_malformed.json`: the code `x`, which is not an invite,
  refused with `invite-invalid` before any request.
- `invite_lookup_unavailable.json`: the issuer answers HTTP 500.
- `invite_lookup_host_not_allowed.json`: a saved configuration whose
  `allowed_hosts` is `issuer.invalid`, asked about an invite on
  `other.invalid`, refused before any request.

`first_run_near_ai_sign_in_replies_for_shell_fixtures` (production build
only: a `near-ai-staging` build starts NEAR AI's Google login instead):

- `near_ai_credential_status_absent.json`: status on a new store.
- `near_ai_credential_start.json`: `near_ai_credential_start` with no
  provider, which opens the daemon's own login chooser page.
- `near_ai_credential_status_waiting.json`: status naming that attempt.
- `near_ai_credential_cancel.json`: cancel naming that attempt.
- `near_ai_credential_status_present.json`: status on a second new store
  that keeps a NEAR AI session. A finished ceremony writes that session
  from NEAR AI's service, which a test cannot reach, so the test writes the
  session into the settings directly (a placeholder refresh token, which the
  reply does not carry). The reply is the real handler's.

## Not here

`status` replies come from the macOS recorded samples
(`macos/Sources/TCShellCore/DataContract/RecordedSamples/normalDay/status.json`),
and `set_project_mode` from `RecordedSamples/shared/set_project_mode.json`.
