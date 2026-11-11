# Linux Insights parity implementation plan

Goal: implement the approved visual foundation and saved-data Overview.
Spec: ../specs/2026-11-11-linux-aesthetic-parity-design.md
Architecture: generated GTK Insights CSS and Rust spacing constants, scoped to Insights; existing services and operations unchanged.

Constraints: no dependencies, no new copy or notices, preserve all four tabs and data semantics. Cargo uses RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0. Tests run once after all tasks per repository instructions.

- [x] Extend generate.py and brand_tokens.rs with INSIGHTS_LIGHT_CSS, INSIGHTS_DARK_CSS and insights::SECTION_GAP, HEADING_GAP, LINE_GAP, BODY_INSET Rust i32 constants from existing flat tokens. Generated CSS supplies tc-insights root and descendant typography, tc-insights-card, tc-insights-summary and navigation states. Guard contrast in generator tests.
- [x] Load generated Insights CSS with existing scheme provider. Scope root in insights.rs. Use generated spacing and card classes in insights_analytics.rs; replace rigid summary row with nonselectable FlowBox, preserving button actions and tab destinations. Verify responsive allocation and keyboard access with existing GTK display test infrastructure.
- [x] Run generator checks, GTK format/check/tests and applicable display tests. Inspect rendered light/dark screenshots when environment permits; report any limitation explicitly.
- [x] Fresh review of complete diff; fix substantive issues and repeat affected checks.

Review focus: narrow windows, enlarged text, long labels, empty/unknown values, stale results. No live-feed or new destination parity in this slice.

## Execution record

Implemented on branch linux-insights-parity. No dependency changes. Review caught and resolved module paths, a disabled GTK API, narrow tab minimum width, and fixed-pixel typography. Current native flat theme uses blue selection; Insights follows it while other GTK surfaces retain their palette.

Validation:
- Generator --check and six generator tests passed.
- GTK suite: 607 passed across library/binaries/integration tests, six Linux display tests ignored on macOS.
- Cargo fmt and Clippy with clients.yml allowances passed.
- Required server license_boundary: four passed.
- Actual GTK app account-free Insights launch/realize/exit succeeded on macOS with isolated temporary state. GDK reported a skipped-frame timing warning; no CSS parser errors.
- Ubuntu 24.04 component harness loaded the complete production stylesheet and passed light/dark at 440/980 pixels and dark at 150% text. Nonblank screenshots, sizing, focus and activation checked. Narrow dark, wide light and enlarged dark captures visually inspected.
- Independent final diff review: no remaining significant findings.

Qualification limit: synthetic Linux components are not the Rust application. Full Linux app, retained real-data states and Weston lifecycle qualification remain pending. The newly added ignored Rust display tests were compiled, not executed. No merge, PR, deployment or release performed.

Reproduce component capture from the GTK crate with Python GI, GTK4/libadwaita GIR, Xvfb, xauth, ImageMagick, dbus-x11 and DejaVu fonts installed:

```sh
INSIGHTS_TEXT_SCALE=1.5 GTK_A11Y=none xvfb-run -a -s '-screen 0 1100x1000x24' dbus-run-session -- python3 scripts/insights-component-shot.py src/ui/brand_tokens.rs /tmp/dark-440-large.png dark 440
```

## PR 2083 CI follow-up

Hosted Linux compiled and passed unit tests but failed the existing OCR gate: the frame was nonblank and contained Overview content, but OCR omitted Overview/Breakdown/Sessions tab labels. No screenshot artifact was retained by that run. A full production stylesheet plus mounted GTK hierarchy reproduced all four labels on Linux; an actual InsightsView probe on macOS also reported all four labels mapped at nonzero size. The cause remains unresolved; no speculative product fix or weakened OCR gate was applied.

Added an actual rooted InsightsView regression checking labels, visible allocation, focus and tab switching, and wired it plus the two existing style/layout regressions into the Weston runner using its nonzero-test-count helper. Local GTK library validation: 597 passed, seven display tests ignored; formatting and bash syntax passed. New Linux display cases are compiled but await CI execution.

Other hosted failures are untouched by this UI diff: rustix 1.1.4 advisory RUSTSEC-2026-0486 across lockfiles; root hickory-proto 0.25.2 advisories RUSTSEC-2026-0487/0489/0490/0491; two contributor fake-Codex process-survival assertions after their 30-second sleep lifetime in a 61-second suite. No review comments were present when checked.
