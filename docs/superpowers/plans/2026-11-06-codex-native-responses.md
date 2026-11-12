# Codex native Responses simplification

**Goal:** Remove obsolete Codex compatibility code now NEAR supports stateless Responses, client tools and web search.

**Architecture:** Use the native Responses route and preserve Codex's known client-tool settings. Advance the pinned IronWire revision to a companion fix removing its obsolete NEAR assistant-history rewrite and namespace rejection. Keep credential isolation, model catalogue/context caps, consent/capture gates, secondary-model selection, and basic settings for unknown models. Keep the existing preferred model order.

**Live evidence:** With explicit owner approval, a signed isolated probe used the existing OS-backed inference key without exporting it or sending project data. Native function calls plus result replay passed on openai/gpt-6-sol, openai/gpt-6.1-sol and openai/gpt-6-astra; native web search returned a web_search_call. A real Codex 0.167.0 turn executed its harmless shell tool, then failed because IronWire changed assistant output_text to input_text. Direct requests confirmed output_text works on both the native OpenAI route and Qwen/Qwen3.8-27B; native OpenAI rejects the obsolete input_text rewrite.

- [x] Replace the Chat-specific roundtrip fixture with native Responses SSE coverage, zero Chat calls, no-store, named choice, reasoning, exact result replay and streamed text. Demonstrate failure under old Chat behavior.
- [x] Remove the forced Chat detour and obsolete model exclusions/hidden-model fallback after live validation. Preserve default model ordering.
- [x] Remove the forced hosted-search disable after a successful live search.
- [x] Remove the obsolete replay rewrite/module and NEAR namespace rejection upstream; 917 tests and clippy passed. IronWire #79 merged as c314b05; all PR CI checks passed.
- [x] Pin c314b05f19f916c584ecd1eaf76a765961a34527 consistently in all three locks and Flatpak sources; only eleven IronWire source entries change in each lock. All four license checks passed.
- [x] Restore known Codex client-tool settings and cover custom/namespace tools, Lite additional_tools and unchanged assistant history. Keep broader model_messages policy templates excluded.
- [x] Codex 0.167.0 completed synthetic shell-tool turns on GPT-6 Sol, GPT-6.1 Sol and GPT-6 Astra with native client settings, zero error events, and the expected command output. Native tool/result checks on three models and hosted web search passed. The signed probe bundle was removed.
- [x] Run final contributor/lint/format/license/FFI/macOS checks and independent review. Contributor: 5,546 tests passed; license boundary: 4 passed; all license feature sets passed; FFI and native macOS suites passed.

Delivery target: draft PR from `codex-native-responses` to `main`.

No new dependencies. No consent, account budget, credential storage or installed-app changes. The separate receipt/witness repair in PR #2013 is not part of this branch. Remove the privileged probe bundle after live validation.

**Mixed-version review:** Add reciprocal native-route acknowledgement/capability
checks. New launchers refuse old daemon-hosted Codex routes before constructing
the native catalogue; new daemons refuse old unacknowledged Codex sign-in
launchers before proxy startup and mark the redeemed row failed. Legacy request
retry is limited to one fixed parse-error response and still requires the
returned native-route capability. Other tools and account types retain their
existing behavior. Independent review found no remaining material issues.
