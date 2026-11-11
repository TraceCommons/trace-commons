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
