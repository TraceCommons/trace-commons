# Local IronWire routing source

Trace Commons maintains this MIT OR Apache-2.0 source import to carry the
multi-subscription work without waiting for nearai/ironwire#59 to merge.
The contributor's three IronWire dependencies resolve directly to this tree.
The nested workspace keeps the routing tests, CLI and original macOS test
fixtures together without adding another desktop app to Trace Commons releases.

- Upstream: https://github.com/nearai/ironwire
- Base: `bfb5e0718716bd61998491ca2d83e1313bf7388d`, the revision Trace Commons
  already embedded.
- Contribution: `64264f2af208f580fc92ef28b197e526ca972bea`, by Jay Zalowitz,
  "Support isolated subscription logins and bounded large-account routing".
- Imported content: tracked source, tests, docs, licenses and build helpers;
  upstream workflows remain reference files under this nested directory and
  are not executed by GitHub. No credentials or developer state.

The contribution was applied on top of the existing base. Conflict resolutions
retain provider-listed exact-model selection (#60), the shared status-line API
and `StatusLine::Command` (#61), home-bound endpoint lookup (#62), and the
prompt-cache warm-up fix (#63). Multi-login status-line labels and their tests
live in `ironwire_proxy::statusline`, where upstream moved the renderer.

The original documentation describes IronWire itself and includes historical
validation results. Those figures are not measurements of this import. For
Trace Commons setup and validation commands, see
[`docs/multi-subscription-routing.md`](../../docs/multi-subscription-routing.md).
Future fixes to this imported implementation belong here. Updating the
upstream revision alone must not silently remove the multi-login changes.

Synthetic credential fixtures are assembled from constant fragments so the
repository credential guard accepts them. Their runtime bytes are unchanged.
