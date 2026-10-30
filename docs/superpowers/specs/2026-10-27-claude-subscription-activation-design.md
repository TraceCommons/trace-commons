# Claude subscription usage activation

## Owner decision

On 2026-10-27 the owner requested activation of the Claude system from PR #1666, using CodexBar as precedent, then clarified: “I don't think it needs opt in.” This supersedes the earlier feature gate and separate Claude opt-in. Claude usage is available by default for detected native profiles. An explicit Refresh is the acquisition action; there is no extra app consent switch or confirmation sheet. macOS retains its Keychain permission controls.

CodexBar revision `5f93dc3068b3bee3f8753f3772614cc2949c335d` is the implementation reference for native credential selection, OAuth usage/profile endpoints and profile mapping. This owner decision enables the implementation; it is not a statement of Anthropic approval.

## Approach

Restore direct OAuth reads from the selected Claude Code credential file or exactly scoped macOS Keychain item. Each login has its own Refresh action and visible acquisition disclosure. The provider response supplies session, weekly and model-specific quotas, including Fable when reported. Values that are absent remain unknown.

This is the original #1666 acquisition approach with the review safeguards retained. Merely lifting the gate would restore its old background-access, identity and timeout defects. A native CLI-only implementation would require a separate PTY/trust design. Browser-cookie sources are a separate feature.

## Boundaries

- Claude needs no separate opt-in. Codex's existing permission and file-storage policy remain independent.
- Daemon startup, status reads and ticks only inspect file metadata and expire readings. Keychain APIs, including attribute queries, run only within a selected-login Refresh. Opening Insights starts a silent Refresh for each login with no current figures (amended 2026-10-30, below); only a pressed Refresh may show a Keychain dialog or start the native CLI.
- One Refresh targets one known local login. Codex actions cannot acquire Claude credentials. Native reads are serialized; interactive authority belongs to the exact selected request.
- OAuth profile identity uses comparable account and organization evidence, not an access-token hash. Known credential/binding changes invalidate results. Secrets and raw provider errors never enter IPC, logs or contributed traces.
- Fixed HTTPS endpoints, an app user agent, bounded responses and network deadlines constrain the provider request. Native credential calls run on leased threads with a longer interactive deadline; a late return retains its concurrency lease and wipes unconsumed secret bytes.
- Claude values are explicitly captured on Refresh. File changes invalidate known state, but the app does not silently track Keychain account changes. The UI instructs users to Refresh after changing Claude accounts and never presents the captured account as continuously verified.
- Unenroll clears all held readings and cancels pending generations. Late Keychain permission results cannot start provider HTTP after cancellation. A future explicit Refresh remains available independently of Commons enrollment.
- Exact native profile service selection respects `CLAUDE_CONFIG_DIR` and `CLAUDE_SECURESTORAGE_CONFIG_DIR`; custom profiles never borrow the global item. Managed launches clear an inherited secure-storage override so their collector and native CLI use the same profile.

## Amendment 2026-10-30: read on open

The owner asked why Claude and Codex limits appeared only after pressing Refresh. Readings live only in daemon memory and expire after an hour, so every daemon restart and every open after an hour showed "Not checked". Opening Insights now reads each login that has no figures, or stale ones, without interaction: Codex reads its saved sign-in and never starts its app-server (which can renew and rewrite auth.json); Claude uses the daemon's own cached token or a Keychain read that refuses to show a dialog. A login that needs permission still shows the Refresh instruction. Each login is read silently at most once per open and again only after 20 minutes. A silent read that fails does not hold the 60-second Refresh floor, so the person's Refresh right after it still runs.

## Verification

Use fixture HTTP and credential readers for protocol, expiry, scope, identity, account-change, bounded-I/O and cancellation coverage. Use only a private disposable Keychain for native storage proof, without changing the user's search list or reading real Claude credentials. Verify default visibility, per-login scope, prompt context, capture labels and 408pt layout with the native suite. Run fresh FFI, Rust/Swift regression suites, Clippy, formatting and license-boundary checks before opening a PR. No release or merge is authorized by this request.
