# TraceCommons current-behavior flow models

Model seed baseline: `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`.
Inventory and selected source seams reviewed at `a2e0aa56608f8c403deff679f8b51e76ac2f5ca7`.
Workflow: [bkase flow-promises](https://github.com/bkase/plugins/blob/6f35f6ae6b3581c499bdfcae080932b4ffa7e281/pstack/skills/flow-promises/SKILL.md),
pinned at `6f35f6ae6b3581c499bdfcae080932b4ffa7e281`.

Open [the review surface](mocks/index.html) to inspect the projected client
states and internal pipeline panels. HTML uses the app's flat dark design
tokens; native Liquid Glass rendering and every platform's exact layout are
outside this surface. Each domain note identifies detailed behavior and
coarse abstractions. [Coverage](coverage.md), [platform differences](platforms.md)
and [composition correspondence](composition.md) delimit the model's scope.

## Status

The owner selected all 13 [candidate guarantees](promise-candidates.md). They
are encoded as 15 safety predicates across seven model families. See the
[proof results](proofs/results.md) for counterexamples, source replays and
checks that remain incomplete. No production behavior changes are included.

[Structural validation](validation.md) covers typechecking, frame/control
agreement and random simulation. Those checks are separate from safety
verification. Neither establishes automatic equivalence with executable code.
This work does not describe a live deployment configuration.

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

The refreshed source index has 1,013 declaration/boundary rows: 59 detailed
projections, 29 coarse interfaces, and 925 unresolved correspondence rows.
Changed source fingerprints invalidate previous mappings until reviewed again.
These counts describe mapping granularity, not verified behavior. Platform
notes outside the selected seams retain their original seed baseline.

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

The recorded owner choice is in `promise-selection.json`. The checker refuses
unselected definitions, stale output folders and exploration constraints.
Only safety invariants were selected; no progress or fairness theorem is
claimed. Observers record transition events without granting new behavior.

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
Java 21 is installed locally but must be placed on PATH. Set `XDG_CACHE_HOME`
to a writable tool-cache directory when running in a restricted sandbox.
`run_proofs.py` runs the selected families with an explicit wall-clock budget;
interruption never becomes a passing result. See the result manifest for exact
commands and hashes.

Report each selected promise as holds/breaks/not checked, with source and
tool versions, finite bounds, state counts, exact command and fairness.
For models beyond the chart budget, follow flow-promises' render-only
merging workflow; disclose each merge without weakening the checked model.
Replay every bug path against the real implementation before reporting a
product defect. Product changes and promise revisions remain owner decisions.
