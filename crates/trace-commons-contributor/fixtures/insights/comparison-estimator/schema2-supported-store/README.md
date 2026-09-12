# Qualified exact comparison store fixture

`index.json` is a synthetic schema-10 local Insights store used to exercise the
saved qualified-estimator path through the CLI and C ABI. It contains four
user-confirmed refactor tasks built from the repository's minimal Claude agent
branch fixture: two distinct root sessions per declared model cohort, complete
matching context, current independence confirmations, and categorical outcomes.
No source bodies, source files, private paths, usage claims, or production pilot
data are included.

The saved specification remains frozen to
`exact_binomial_components_bonferroni_v1`. Evaluation must include all four
tasks and return a schema-2 supported payload with six component intervals and
three ordered second-cohort-minus-first contrasts. The fixture proves the
cross-boundary evaluator wiring; it does not activate the method for newly
created specifications or establish product admission.

## Capture recipe

The fixture was captured with the contributor's real store APIs. Starting from
`claude-task-attribution/agent-alpha.jsonl`, the capture test made four copies,
replacing only the canonical record UUID chain, root session ID, safe agent ID,
and both native model declarations. It imported each copy as `claude-code`,
created a one-member episode and task, recorded the same complete synthetic
context, recorded the outcomes accepted/partial for `model-a` and
rejected/accepted for `model-b`, and reconfirmed each current material digest.
It then saved the immutable cutoff evidence, rebuilt that same specification
with the reviewed qualified estimator state so both specification digests were
recomputed by production code, and serialized the store through the ordinary
store encoder.

The committed byte capture has SHA-256
`c1bbb6b4e7b14a078971af78ad80324842cec7fd92657b2162b3e1c072e0c17b`.
The CLI and ABI tests validate the store on read and recompute the qualified
result rather than accepting a cached result payload.
