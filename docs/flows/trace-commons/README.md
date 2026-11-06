# TraceCommons current-behavior flow models

Source baseline: `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`.
Workflow: [bkase flow-promises](https://github.com/bkase/plugins/blob/6f35f6ae6b3581c499bdfcae080932b4ffa7e281/pstack/skills/flow-promises/SKILL.md),
pinned at `6f35f6ae6b3581c499bdfcae080932b4ffa7e281`.

Open [the review surface](mocks/index.html) to inspect the projected client
states and internal pipeline panels. HTML uses the app's flat dark design
tokens; native Liquid Glass rendering and every platform's exact layout are
outside this surface. Each domain note identifies detailed behavior and
coarse abstractions. [Coverage](coverage.md), [platform differences](platforms.md)
and [composition correspondence](composition.md) delimit the model's scope.

## Status

See [validation evidence](validation.md) for the completed structural checks.
The design is approved. Product promises have **not** been selected or encoded.
Typechecking, frame/control agreement and random simulation are structural
validation, not proofs of safety or progress. There are no TLC verdicts,
counterexamples or generated checked statecharts yet. The surface reserves
space for those charts rather than substituting hand-drawn diagrams.

The primary checkout's unrelated untracked files, including passkey,
account-binding and upload-allowance additions, are outside the pinned source
baseline. This work does not describe a live deployment configuration.

## Model families

| Model | Responsibility |
| --- | --- |
| [client_shell](models/client_shell.md) | Startup, navigation, first run, account and shell lifecycle |
| [client_contribution](models/client_contribution.md) | Consent, previews, queue, upload, receipt and history |
| [client_services](models/client_services.md) | Private AI, managed tools and account service projections |
| [client_learning](models/client_learning.md) | Local analysis, mission and skill workflow |
| [client_consent](models/client_consent.md) | Grant identity, scope/revocation, folder policy and automatic sharing |
| [client_providers](models/client_providers.md) | Multiple provider accounts, routing and non-NEAR managed sessions |
| [platform_clients](models/platform_clients.md) | Explicit Windows, GTK and Tauri client lifecycle differences |
| [server_identity](models/server_identity.md) | Server account/device authorization ceremonies, session and revocation boundaries |
| [server_receipt](models/server_receipt.md) | Authority, replay, staged encrypted storage and receipt routing |
| [server_pipeline](models/server_pipeline.md) | Immutable runs, leases, privacy, review, Score and Settle processing |
| [server_settlement](models/server_settlement.md) | Instruments, credit, pipeline NEAR outbox and separate account payouts |
| [server_lifecycle](models/server_lifecycle.md) | Withdrawal, revocation, retention, index and export eligibility |
| [server_interfaces](models/server_interfaces.md) | Coarse interfaces for other route and worker families |
| [end_to_end](models/end_to_end.md) | Abstract composition of the contribution journey |

Every model uses bounded symbolic identities and external outcomes. None
proves cryptography, redaction/model quality, database implementation,
memory safety, OS isolation, or deployed configuration. Detailed domain
checks do not imply a proof of the abstract composition, and the composition
does not establish correspondence with executable Rust/Swift automatically.

The source index currently has 993 declaration/boundary rows: 119 detailed
projections, 215 coarse interfaces, and 659 unresolved correspondence rows.
Of the unresolved rows, 304 are presentation declarations rather than
established independent flows. These counts describe mapping granularity,
not verified behavior. The suite does not claim that every route parameter,
component, provider branch or platform dialog is modeled in detail; the
remaining gaps are explicit in the coverage files.

## Reproduce structural validation

Prerequisites: Quint 0.33.0, Python 3.9+, Node with native `.mts` support
(validated with Node 26). These tools do not change application manifests.

From the repository root:

```sh
python3 docs/flows/trace-commons/scripts/inventory.py --help
python3 docs/flows/trace-commons/scripts/build_surface.py
node docs/flows/trace-commons/scripts/test_itf.mjs
python3 docs/flows/trace-commons/scripts/verify.py --out /private/tmp/tc-quint-validation-NEW
python3 -m http.server 8765 --directory docs/flows/trace-commons
```

Visit `http://localhost:8765/mocks/`. The verifier refuses stale output,
typechecks all models, evaluates each frame's controls in Quint, simulates
bounded traces without product invariants, and runs each view against those
traces. Its evidence lists unobserved frames and actions; random traces do
not establish exhaustive reachability or deadlock freedom. Retain temporary
logs outside this design seed.

## Select and check promises

First inspect the screen surface. Then choose product-language promises
from the separately presented candidates. Record the owner's choice in
`promise-selection.json`, encode only the selected definitions in the
relevant `.qnt` files, and list them individually in each `.cfg` as
`INVARIANT NAME` or `PROPERTY NAME`. Do not add constraints to shrink the
behavior being checked. Fairness must match the selected promise's stated
environment and user actions.

For a pinned tool checkout and a **new** output directory:

```sh
git clone https://github.com/bkase/plugins.git /private/tmp/tc-flow-tools-NEW
git -C /private/tmp/tc-flow-tools-NEW checkout 6f35f6ae6b3581c499bdfcae080932b4ffa7e281
python3 docs/flows/trace-commons/scripts/check_selected.py MODEL \
  --skill-root /private/tmp/tc-flow-tools-NEW --out /private/tmp/tc-tlc-MODEL-NEW
python3 docs/flows/trace-commons/scripts/build_surface.py
```

The skill compiles Quint to TLA+, checks through TLC/statechart and renders
with Pele. It requires Java 21+ (or mise to provision it), network access
for tool bootstrap, and the pinned skill's own runtime prerequisites.
Java was not available on PATH at the initial inspection. The selected
checker intentionally refuses to run before a recorded owner selection.

Report each selected promise as holds/breaks/not checked, with source and
tool versions, finite bounds, state counts, exact command and fairness.
For models beyond the chart budget, follow flow-promises' render-only
merging workflow; disclose each merge without weakening the checked model.
Replay every bug path against the real implementation before reporting a
product defect. Product changes and promise revisions remain owner decisions.
