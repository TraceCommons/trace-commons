# Scope ceilings and historical ownership

Policy/grant targets use the same empty-list semantics as their consumers:
empty consent scopes or allowed uses mean unrestricted in that dimension. A
restricted administrator or tenant policy cannot create an unrestricted target.
An unrestricted caller under unrestricted tenant ceilings may still do so.
Each dimension is checked independently against both ceilings.

Historical records with no persisted owner still deserialize to
`principal_legacy`, but that sentinel means unknown ownership. It grants no
ordinary contributor access to status, credit, self-revocation or quarantine
remediation. Canonical principal references and exact per-credential historical
aliases retain their existing access. Account expansion retains explicit active
principal membership; it does not infer an owner from attribution fields.

Same-tenant reviewers and administrators retain their explicit recovery routes.
Recovery revocations require the existing privileged action scope/policy checks
and a nonempty reason. A contributor must not regain access by guessing a
submission ID or by matching tenant attribution. Recover historical ownership
only from independently established, attributable evidence through an explicit
reviewed recovery process; this change neither invents ownership nor migrates
stored owner references. Cross-tenant recovery remains denied.
