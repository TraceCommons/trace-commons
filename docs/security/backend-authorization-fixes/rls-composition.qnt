// PostgreSQL composition seam, corresponding to V18, V142 and PgBackend.
// No HTTP input/credential derivation is modeled here. Tenant arguments are
// application-authorized; SQL-capable runtime code can choose another tenant.
module rls_composition {
  val Tenants = Set(0, 1)
  type State = {inheritedResolver: bool, extraPermissive: bool,
    sessionTenant: int, transaction: bool, localTenant: int, localResolver: bool,
    read: bool, readForeign: bool, readSecret: bool, readAsResolver: bool}
  var s: State
  action initialize(inheritRole: bool, extraPolicy: bool): bool = s' = {
    inheritedResolver: inheritRole, extraPermissive: extraPolicy,
    sessionTenant: -1, transaction: false, localTenant: -1, localResolver: false,
    read: false, readForeign: false, readSecret: false, readAsResolver: false}
  action init = initialize(false, false)
  action inheritedInit = initialize(true, false)
  action extraPolicyInit = initialize(false, true)
  val tenant = if (s.transaction) s.localTenant else s.sessionTenant
  // Named tenant policy, enabled/FORCE RLS, direct non-owner/NOBYPASS role
  // all remain good in every configuration. The repaired startup checker also
  // rejects applicable permissive alternatives and inherited resolver authority.
  val CatalogAccepts = not(s.inheritedResolver) and not(s.extraPermissive)
  action setSessionTenant(t: int): bool = all { not(s.transaction),
    s' = {...s, sessionTenant: t, read: false} }
  action beginTenant(t: int): bool = all { not(s.transaction),
    s' = {...s, transaction: true, localTenant: t, localResolver: false, read: false} }
  // SET LOCAL ROLE replaces effective runtime privileges with the resolver's
  // six-column grant. INHERIT combines grants while retaining runtime identity.
  action assumeResolver = all { s.transaction,
    s' = {...s, localResolver: true, read: false} }
  action finish = all { s.transaction,
    s' = {...s, transaction: false, localTenant: -1, localResolver: false, read: false} }
  action query(rowTenant: int, secretColumn: bool): bool = {
    val columnAllowed = not(s.localResolver) or not(secretColumn)
    val rolePolicy = s.localResolver or s.inheritedResolver
    val rowAllowed = rowTenant == tenant or rolePolicy or s.extraPermissive
    // This is application access after startup, not a change to PostgreSQL OR.
    val permitted = CatalogAccepts and columnAllowed and rowAllowed
    s' = {...s, read: permitted, readForeign: rowTenant != tenant,
      readSecret: secretColumn, readAsResolver: s.localResolver}
  }
  action step = any {
    nondet t = Tenants.oneOf()
    setSessionTenant(t),
    nondet t = Tenants.oneOf()
    beginTenant(t),
    assumeResolver, finish,
    nondet t = Tenants.oneOf()
    nondet secret = Set(false, true).oneOf()
    query(t, secret)
  }
  val RuntimeTenantIsolation = not(s.read and not(s.readAsResolver)) or not(s.readForeign)
  val ResolverColumnRestriction = not(s.read and s.readAsResolver) or not(s.readSecret)
  val NoForeignSecret = not(s.read and s.readForeign and s.readSecret)
  val GoodConfiguration = not(s.inheritedResolver) and not(s.extraPermissive)
  val Preserved = (not(GoodConfiguration) or RuntimeTenantIsolation) and ResolverColumnRestriction

  run SetLocalOverridesAndRestores = init.then(setSessionTenant(1)).then(beginTenant(0))
    .then(query(1, true)).then(all { not(s.read), tenant == 0, s' = s })
    .then(finish).then(query(1, true)).then(all { s.read, tenant == 1, s' = s })
  run ExplicitResolverIsNarrow = init.then(beginTenant(0)).then(assumeResolver)
    .then(query(1, false)).then(all { s.read, ResolverColumnRestriction, s' = s })
    .then(query(1, true)).then(all { not(s.read), s' = s })
    .then(finish).then(beginTenant(0)).then(query(1, false))
    .then(all { not(s.read), s' = s })
  run InheritedResolverRefusesStartup = inheritedInit.then(beginTenant(0)).then(query(1, true))
    .then(all { not(CatalogAccepts), NoForeignSecret, not(s.read), s' = s })
  run ExtraPolicyRefusesStartup = extraPolicyInit.then(beginTenant(0)).then(query(1, true))
    .then(all { not(CatalogAccepts), NoForeignSecret, not(s.read), s' = s })
}
