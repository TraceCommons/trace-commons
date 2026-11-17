// Fixed effective-permission guard for empty-as-unrestricted allowlists.
// The property compares effective permissions independently of the subset guard.
module scope_escalation {
  pure val scopes = Set("evaluation", "training")
  pure val uses = Set("evaluation", "training")
  var s: { policyScopes: Set[str], policyUses: Set[str], grantScopes: Set[str], grantUses: Set[str] }
  val vars = s
  pure val callerScopes = Set("evaluation")
  pure val callerUses = Set("evaluation")

  // ingest target_subset_of_allowlist / target_subset_of_use_allowlist.
  pure def currentAllows(target: Set[str], ceiling: Set[str]): bool =
    ceiling == Set() or (target != Set() and target.forall(x => ceiling.contains(x)))
  // Runtime interprets the empty stored allowlist as unrestricted.
  pure def effective(target: Set[str], universe: Set[str]): Set[str] =
    if (target == Set()) universe else target
  pure def semanticSubset(target: Set[str], ceiling: Set[str], universe: Set[str]): bool =
    effective(target, universe).forall(x => effective(ceiling, universe).contains(x))

  val noEscalation = all {
    semanticSubset(s.policyScopes, callerScopes, scopes),
    semanticSubset(s.policyUses, callerUses, uses),
    semanticSubset(s.grantScopes, callerScopes, scopes),
    semanticSubset(s.grantUses, callerUses, uses)
  }
  action init = s' = { policyScopes: callerScopes, policyUses: callerUses,
    grantScopes: callerScopes, grantUses: callerUses }

  action currentPolicy = {
    nondet targetScopes = scopes.powerset().oneOf()
    nondet targetUses = uses.powerset().oneOf()
    all {
      currentAllows(targetScopes, callerScopes), currentAllows(targetUses, callerUses),
      currentAllows(targetScopes, s.policyScopes), currentAllows(targetUses, s.policyUses),
      s' = { ...s, policyScopes: targetScopes, policyUses: targetUses }
    }
  }
  action currentGrant = {
    nondet targetScopes = scopes.powerset().oneOf()
    nondet targetUses = uses.powerset().oneOf()
    all {
      currentAllows(targetScopes, callerScopes), currentAllows(targetUses, callerUses),
      currentAllows(targetScopes, s.policyScopes), currentAllows(targetUses, s.policyUses),
      s' = { ...s, grantScopes: targetScopes, grantUses: targetUses }
    }
  }
  action currentStep = any { currentPolicy, currentGrant }
  action step = currentStep

  run policyWideningRejectedTest = init.expect(not(currentAllows(Set(), callerScopes)))
  run grantWideningRejectedTest = init.expect(not(currentAllows(Set(), callerUses)))
  run permittedPolicyRemainsBoundedTest = init.then(currentPolicy).expect(noEscalation)
  run permittedGrantRemainsBoundedTest = init.then(currentGrant).expect(noEscalation)
}
