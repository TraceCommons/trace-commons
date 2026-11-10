// Source-correspondence model of selected backend guard families, not all routes.
module authorization {
  val Tokens = 0.to(8) // 8 is an unknown/forged credential, never authenticated
  val Objects = 0.to(4)
  val Tenants = Set(0, 1)
  val Actors = Set(0, 1, 2)
  val Operations = Set("status", "delete", "remediate", "download", "admin", "export", "worker")
  pure def tokenTenant(k: int): int = if (k == 2) 1 else 0
  pure def tokenActor(k: int): int = if (k == 0) 0 else if (k == 2) 2 else 1
  pure def tokenRole(k: int): str =
    if (k <= 2) "contributor" else if (k == 3) "reviewer"
    else if (k == 4) "admin" else if (k == 5) "export_worker"
    else if (k == 6) "revocation_worker" else "utility_worker"
  pure def review(k: int): bool = Set("reviewer", "admin").contains(tokenRole(k))
  pure def objectTenant(o: int): int = if (o == 2) 1 else 0
  // -1 represents persisted principal_legacy, NOT an attacker-owned principal.
  pure def storedOwner(o: int): int = if (o == 3) -1 else if (o == 4) 3 else o
  // A ghost attribution for the audit hypothesis. The old record lacks this fact.
  pure def originalOwner(o: int): int = if (o == 3 or o == 4) 0 else o
  pure def tenantReadOwn(k: int, o: int): bool = storedOwner(o) == tokenActor(k)
  pure def selfRevoke(k: int, o: int): bool = storedOwner(o) == tokenActor(k)
  // Principal 3 is a second device linked to account 0. Account reads exclude
  // the legacy wildcard and tenant reviewer shortcut. Membership can be revoked.
  pure def principalAccount(p: int): int = if (p == 3) 0 else p
  type Request = {pending: bool, token: int, object: int, claimedTenant: int,
    claimedActor: int, claimedRole: str, op: str, authenticated: bool}
  type Effect = {present: bool, token: int, object: int, tenant: int, actor: int,
    op: str, own: bool, originalOwn: bool, grantAtCheck: bool, abacAtCheck: bool}
  pure val noRequest: Request = {pending: false, token: 0, object: 0, claimedTenant: 0,
    claimedActor: 0, claimedRole: "contributor", op: "status", authenticated: false}
  pure val noEffect: Effect = {present: false, token: 0, object: 0, tenant: 0,
    actor: 0, op: "status", own: false, originalOwn: false, grantAtCheck: false, abacAtCheck: false}
  var s: {request: Request, effect: Effect, expired: Set[int], grants: Set[int],
    activeAccounts: Set[int], activePrincipals: Set[int], abacObjects: Set[int], revoked: Set[int], changed: Set[int]}
  val vars = s
  action init = s' = {request: noRequest, effect: noEffect, expired: Set(),
    grants: Tokens.exclude(Set(8)), activeAccounts: Actors, activePrincipals: Set(0, 1, 2, 3), abacObjects: Objects,
    revoked: Set(), changed: Set()}

  // All target IDs and identity/role assertions are adversarial request inputs.
  // Only a configured/verified credential fixture supplies tenant/actor/role.
  action begin(k: int, o: int, t: int, a: int, role: str, op: str): bool = all {
    not(s.request.pending),
    s' = {...s, effect: noEffect, request: {pending: true, token: k, object: o,
      claimedTenant: t, claimedActor: a, claimedRole: role, op: op,
      authenticated: k != 8 and not(s.expired.contains(k))}}
  }
  action BeginRequest = {
    nondet k = Tokens.oneOf()
    nondet o = Objects.oneOf()
    nondet t = Tenants.oneOf()
    nondet a = Actors.oneOf()
    nondet role = Set("contributor", "admin").oneOf()
    nondet op = Operations.oneOf()
    begin(k, o, t, a, role, op)
  }
  action ExpireCredential = {
    nondet k = Tokens.oneOf()
    s' = {...s, expired: s.expired.union(Set(k))}
  }
  action RevokeAccessGrant = {
    nondet k = Tokens.oneOf()
    s' = {...s, grants: s.grants.exclude(Set(k))}
  }
  action RevokeAccount = {
    nondet a = Actors.oneOf()
    s' = {...s, activeAccounts: s.activeAccounts.exclude(Set(a))}
  }
  action RestrictObjectPolicy = {
    nondet o = Objects.oneOf()
    s' = {...s, abacObjects: s.abacObjects.exclude(Set(o))}
  }
  action unlinkPrincipal(p: int): bool = s' = {...s, activePrincipals: s.activePrincipals.exclude(Set(p))}
  action UnlinkPrincipal = {
    nondet p = Set(0, 1, 2, 3).oneOf()
    unlinkPrincipal(p)
  }
  // Atomic guard-decision abstraction. The implementation may await between
  // validation, ownership expansion, authorization and publication; those
  // linearization races are outside this model (bootstrap has its own model).
  action AuthorizeSelectedOperation = {
    val r = s.request
    val k = r.token
    val o = r.object
    val accountOwn = s.activePrincipals.contains(storedOwner(o))
      and principalAccount(storedOwner(o)) == tokenActor(k)
    val statusAccountOwn = s.activePrincipals.contains(tokenActor(k)) and accountOwn
    val principalOwn = storedOwner(o) == tokenActor(k)
    val privateObject = Set("status", "delete", "remediate", "download", "export").contains(r.op)
    val tenantOK = not(privateObject) or objectTenant(o) == tokenTenant(k)
    // Grant management/admin recovery and self-revocation intentionally have
    // distinct active-grant semantics; they are not universal grant-gated routes.
    val needsGrant = Set("status", "remediate", "export").contains(r.op)
    val grantOK = not(needsGrant) or s.grants.contains(k)
    val objectOK = if (r.op == "status") review(k) or tenantReadOwn(k, o) or statusAccountOwn
      else if (r.op == "delete") selfRevoke(k, o) or (review(k) and s.abacObjects.contains(o))
      else if (r.op == "remediate") selfRevoke(k, o)
      else if (r.op == "download") accountOwn and s.activeAccounts.contains(tokenActor(k))
      else if (r.op == "admin") tokenRole(k) == "admin"
      else if (r.op == "export") Set("reviewer", "admin", "export_worker").contains(tokenRole(k)) and s.abacObjects.contains(o)
      else Set("admin", "revocation_worker").contains(tokenRole(k))
    // download uses the account-session surface, not the tenant bearer surface.
    // Fixture tokens 0..2 stand for matching account session identities there.
    val transportOK = r.op != "download" or k <= 2
    val ok = r.authenticated and tenantOK and grantOK and objectOK and transportOK
    all { r.pending,
      s' = {...s, request: {...r, pending: false},
        effect: if (ok) {present: true, token: k, object: o, tenant: tokenTenant(k),
          actor: tokenActor(k), op: r.op, own: if (r.op == "download") accountOwn else if (r.op == "status") principalOwn or statusAccountOwn else principalOwn,
          originalOwn: originalOwner(o) == tokenActor(k), grantAtCheck: s.grants.contains(k),
          abacAtCheck: s.abacObjects.contains(o)} else noEffect,
        revoked: if (ok and r.op == "delete") s.revoked.union(Set(o)) else s.revoked,
        changed: if (ok and r.op == "remediate") s.changed.union(Set(o)) else s.changed}
    }
  }
  action step = any { BeginRequest, AuthorizeSelectedOperation, ExpireCredential, RevokeAccessGrant,
    RevokeAccount, UnlinkPrincipal, RestrictObjectPolicy }

  val TenantIsolation = not(s.effect.present) or (
    s.effect.tenant == tokenTenant(s.effect.token)
    and (not(Set("status", "delete", "remediate", "download", "export").contains(s.effect.op))
      or objectTenant(s.effect.object) == s.effect.tenant))
  val AuthenticatedAtEntry = not(s.effect.present) or s.effect.token != 8
  val NoRolePromotion = not(s.effect.present) or (
    (s.effect.op != "admin" or tokenRole(s.effect.token) == "admin")
    and (s.effect.op != "worker" or Set("admin", "revocation_worker").contains(tokenRole(s.effect.token))))
  val AccountDownloadOwnership = not(s.effect.present and s.effect.op == "download") or s.effect.own
  val ModernContributorIsolation = not(s.effect.present
    and Set("status", "delete", "remediate").contains(s.effect.op)
    and tokenRole(s.effect.token) == "contributor" and storedOwner(s.effect.object) != -1) or s.effect.own
  val GrantRequiredWhereEnforced = not(s.effect.present
    and Set("status", "remediate", "export").contains(s.effect.op)) or s.effect.grantAtCheck
  val ScopedExport = not(s.effect.present and s.effect.op == "export") or (
    Set("reviewer", "admin", "export_worker").contains(tokenRole(s.effect.token)) and s.effect.abacAtCheck)
  val Preserved = TenantIsolation and AuthenticatedAtEntry and NoRolePromotion
    and AccountDownloadOwnership and ModernContributorIsolation and GrantRequiredWhereEnforced and ScopedExport
  // Contributor privacy hypotheses now hold for legacy records too: the sentinel
  // conveys no ordinary ownership. Privileged tenant recovery remains separate.
  val ContributorReadsOnlyAttributedData = not(s.effect.present and s.effect.op == "status"
    and tokenRole(s.effect.token) == "contributor") or s.effect.originalOwn
  val ContributorDeletesOnlyAttributedData = not(s.effect.present and s.effect.op == "delete"
    and tokenRole(s.effect.token) == "contributor") or s.effect.originalOwn
  val ContributorEditsOnlyAttributedData = not(s.effect.present and s.effect.op == "remediate"
    and tokenRole(s.effect.token) == "contributor") or s.effect.originalOwn

  val Safety = Preserved and ContributorReadsOnlyAttributedData
    and ContributorDeletesOnlyAttributedData and ContributorEditsOnlyAttributedData

  run LegacyReadDeniedTest = init.then(begin(1, 3, 0, 0, "admin", "status")).then(AuthorizeSelectedOperation)
    .then(all { not(s.effect.present), ContributorReadsOnlyAttributedData, Preserved, s' = s })
  run LegacyDeleteDeniedTest = init.then(begin(1, 3, 0, 0, "admin", "delete")).then(AuthorizeSelectedOperation)
    .then(all { not(s.effect.present), ContributorDeletesOnlyAttributedData, not(s.revoked.contains(3)), Preserved, s' = s })
  run LegacyEditDeniedTest = init.then(begin(1, 3, 0, 0, "admin", "remediate")).then(AuthorizeSelectedOperation)
    .then(all { not(s.effect.present), ContributorEditsOnlyAttributedData, not(s.changed.contains(3)), Preserved, s' = s })
  run ForeignTenantDeniedTest = init.then(begin(2, 0, 0, 0, "admin", "delete")).then(AuthorizeSelectedOperation)
    .then(all { not(s.effect.present), s' = s })
  run ForgedRoleDeniedTest = init.then(begin(1, 0, 0, 0, "admin", "admin")).then(AuthorizeSelectedOperation)
    .then(all { not(s.effect.present), s' = s })
  run ForeignExportDeniedTest = init.then(begin(5, 2, 1, 2, "admin", "export")).then(AuthorizeSelectedOperation)
    .then(all { not(s.effect.present), s' = s })
  run LinkedDeviceDownloadTest = init.then(begin(0, 4, 0, 0, "contributor", "download")).then(AuthorizeSelectedOperation)
    .then(all { s.effect.present, AccountDownloadOwnership, s' = s })
  run UnlinkedCallerCannotExpandStatusTest = init.then(unlinkPrincipal(0))
    .then(begin(0, 4, 0, 0, "contributor", "status")).then(AuthorizeSelectedOperation)
    .then(all { not(s.effect.present), s' = s })
  run UnlinkedCallerKeepsExactOwnStatusTest = init.then(unlinkPrincipal(0))
    .then(begin(0, 0, 0, 0, "contributor", "status")).then(AuthorizeSelectedOperation)
    .then(all { s.effect.present, s.effect.own, s' = s })
  run PeerDownloadDeniedTest = init.then(begin(1, 0, 0, 0, "admin", "download")).then(AuthorizeSelectedOperation)
    .then(all { not(s.effect.present), s' = s })
}
