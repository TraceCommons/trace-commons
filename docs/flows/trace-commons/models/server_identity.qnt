// Current tracked baseline. No owner-selected promises or properties.
module server_identity {
  val Accounts = Set(0, 1)
  val Devices = Set(0, 1)
  val Challenges = Set(0, 1)
  val Sessions = 0.to(3)
  val Requests = Set(0, 1)
  type Challenge = {kind: str, phase: str, expired: bool, cancelled: bool,
    verifier: int, device: int, account: int, tenant: int,
    suppliedVerifier: int, suppliedDevice: int, suppliedAccount: int, suppliedRequest: int,
    result: str, session: int}
  type Session = {phase: str, account: int, tenant: int, device: int, kind: str,
    token: int, previous: int, grace: bool, expired: bool, idle: bool, due: bool, capped: bool, seen: int}
  type Auth = {phase: str, session: int, token: int, tenant: int, account: int,
    claimedAccount: int, suppliedMinter: int, transport: str, canonicalToken: int, rotatePlanned: bool, authorityAllowed: bool, rotated: int}
  type DeviceCode = {phase: str, account: int, minter: int, expired: bool}
  type Link = {phase: str, account: int, tenant: int, native: int, expired: bool}
  pure val emptyChallenge: Challenge = {kind: "none", phase: "unused", expired: false, cancelled: false,
    verifier: 0, device: -1, account: -1, tenant: -1, suppliedVerifier: -1, suppliedDevice: -1,
    suppliedAccount: -1, suppliedRequest: -1, result: "none", session: -1}
  pure val emptySession: Session = {phase: "unused", account: -1, tenant: -1, device: -1, kind: "none",
    token: 0, previous: -1, grace: false, expired: false, idle: false, due: false, capped: false, seen: 0}
  pure val emptyAuth: Auth = {phase: "unused", session: -1, token: -1, tenant: -1,
    account: -1, claimedAccount: -1, suppliedMinter: -1, transport: "none", canonicalToken: -1, rotatePlanned: false, authorityAllowed: false, rotated: -1}
  pure val emptyLink: Link = {phase: "unused", account: -1, tenant: -1, native: -1, expired: false}
  pure def newSession(a: int, t: int, d: int, k: str): Session =
    {...emptySession, phase: "live", account: a, tenant: t, device: d, kind: k}
  pure val emptyDeviceCode: DeviceCode = {phase: "unused", account: -1, minter: -1, expired: false}
  var s: {codes: int -> DeviceCode, challenges: int -> Challenge, links: int -> Link, sessions: int -> Session, auth: int -> Auth,
    deviceOwner: int -> int, deviceOrigin: int -> str, renewable: Set[int], revoked: Set[int], grants: Set[int], closed: Set[int],
    received: Set[int], responsesResolved: Set[int], renewalNonces: Set[int], frame: str, outcome: str}
  val vars = s
  pure def controls(frame: str): Set[str] = Set("YouStartASignInCeremony", "YouCancelTheLocalCeremony",
    "YouMintABrowserLoginLink", "YouConfirmTheBrowserLogin", "YouSignOutTheCurrentSession",
    "YouSignOutEverywhere", "YouRevokeAnAccountDevice", "YouReadAnAccountSurface", "YouMintAHeadlessDeviceCode")
  action init = {
    nondet a0 = Set(-1, 0, 1).oneOf()
    nondet a1 = Set(-1, 0, 1).oneOf()
    nondet o0 = Set("near", "near_ai").oneOf()
    nondet o1 = Set("near", "near_ai").oneOf()
    s' = {codes: Challenges.mapBy(_ => emptyDeviceCode), challenges: Challenges.mapBy(_ => emptyChallenge), links: Challenges.mapBy(_ => emptyLink),
      sessions: Sessions.mapBy(_ => emptySession), auth: Requests.mapBy(_ => emptyAuth),
      deviceOwner: Map(0 -> a0, 1 -> a1), deviceOrigin: Map(0 -> o0, 1 -> o1), renewable: Set(), revoked: Set(), grants: Devices.filter(d => (if (d == 0) a0 else a1) >= 0),
      closed: Set(), received: Set(), responsesResolved: Set(), renewalNonces: Set(), frame: "identity_ready", outcome: "none"}
  }
  action YouStartASignInCeremony = {
    nondet c = Challenges.oneOf()
    nondet k = Set("native", "wallet", "near_ai").oneOf()
    nondet v = Set(0, 1).oneOf()
    nondet d = Devices.oneOf()
    nondet a = Accounts.oneOf()
    all { controls(s.frame).contains("YouStartASignInCeremony"), s.challenges.get(c).phase == "unused",
      s' = {...s, challenges: s.challenges.set(c, {...emptyChallenge, kind: k, phase: "pending", verifier: v,
        device: if (k == "native") -1 else d, account: if (k == "wallet") a else -1}),
        frame: "identity_challenge", outcome: "validated_start_stored_no_session"} }
  }
  action YouCancelTheLocalCeremony = {
    nondet c = Challenges.oneOf()
    val x = s.challenges.get(c)
    all { controls(s.frame).contains("YouCancelTheLocalCeremony"), x.phase != "unused", not(x.cancelled),
      s' = {...s, challenges: s.challenges.set(c, {...x, cancelled: true}),
        frame: "identity_challenge", outcome: "local_wait_abandoned_server_record_unchanged"} }
  }
  action YouMintABrowserLoginLink = {
    nondet l = Challenges.oneOf()
    nondet d = Devices.oneOf()
    nondet c = Set(-1, 0, 1).oneOf()
    nondet grantsRequired = Set(true, false).oneOf()
    val a = s.deviceOwner.get(d)
    all { controls(s.frame).contains("YouMintABrowserLoginLink"), s.links.get(l).phase == "unused",
      a >= 0, (not(grantsRequired) or s.grants.contains(d)), not(s.closed.contains(a)),
      s' = {...s, links: s.links.set(l, {phase: "live", account: a, tenant: a, native: c, expired: false}),
        frame: "identity_browser", outcome: "device_authenticated_link_mint_coarse"} }
  }
  action YouConfirmTheBrowserLogin = {
    nondet l = Challenges.oneOf()
    nondet j = Sessions.oneOf()
    nondet browserEvidence = Set(true, false).oneOf()
    val link = s.links.get(l)
    val c = if (link.native < 0) 0 else link.native
    val x = s.challenges.get(c)
    val ok = link.phase == "live" and not(link.expired) and browserEvidence
    val native = ok and link.native >= 0 and x.kind == "native" and x.phase == "pending" and not(x.expired)
    all { controls(s.frame).contains("YouConfirmTheBrowserLogin"), link.phase != "unused", s.sessions.get(j).phase == "unused",
      s' = {...s, links: if (ok) s.links.set(l, {...link, phase: "consumed"}) else s.links,
        sessions: if (ok) s.sessions.set(j, newSession(link.account, link.tenant, -1, "browser")) else s.sessions,
        challenges: if (native) s.challenges.set(c, {...x, phase: "code", account: link.account, tenant: link.tenant}) else s.challenges,
        frame: "identity_browser", outcome: if (native) "browser_session_and_loopback_code" else if (ok) "browser_session_native_missing_or_expired" else "uniform_deny"} }
  }
  action TheFinishRequestTakesTheStoredCodeOrCeremony = {
    nondet c = Challenges.oneOf()
    nondet v = Set(0, 1).oneOf()
    nondet d = Devices.oneOf()
    nondet a = Accounts.oneOf()
    nondet request = Challenges.oneOf()
    val x = s.challenges.get(c)
    all { (x.kind == "native" and x.phase == "code") or (x.kind != "native" and x.phase == "pending"),
      s' = {...s, challenges: s.challenges.set(c, {...x, phase: if (x.expired) "done" else "taken",
        suppliedVerifier: v, suppliedDevice: d, suppliedAccount: a, suppliedRequest: request,
        result: if (x.expired) "uniform_deny" else "consumed_before_verification"}),
        frame: "identity_verifying", outcome: if (x.expired) "uniform_deny" else "single_use_take"} }
  }
  action TheStoredBindingAndExternalEvidenceAreChecked = {
    nondet c = Challenges.oneOf()
    nondet evidence = Set("valid", "invalid", "unavailable").oneOf()
    nondet login = Accounts.oneOf()
    val x = s.challenges.get(c)
    val binding = x.suppliedVerifier == x.verifier and (if (x.kind == "native") x.suppliedRequest == c
      else x.suppliedDevice == x.device and (x.kind != "wallet" or x.suppliedAccount == x.account))
    val valid = binding and (x.kind == "native" or evidence == "valid")
    val account = if (x.kind == "near_ai") login else x.account
    all { x.phase == "taken",
      s' = {...s, challenges: s.challenges.set(c, {...x, phase: if (valid) "verified" else "done",
        account: account, tenant: account, result: if (valid) "verified" else if (binding and evidence == "unavailable") "unavailable_consumed" else "uniform_deny"}),
        frame: "identity_verifying", outcome: if (valid) "stored_binding_verified" else "consumed_no_session"} }
  }
  action TheSessionIssueTransactionCommitsOrRefuses = {
    nondet c = Challenges.oneOf()
    nondet j = Sessions.oneOf()
    nondet database = Set(true, false).oneOf()
    nondet signupGrant = Set(true, false).oneOf()
    val x = s.challenges.get(c)
    val native = x.kind == "native"
    val d = if (native) 0 else x.device
    val owner = s.deviceOwner.get(d)
    val origin = if (x.kind == "wallet") "near" else "near_ai"
    val deviceOK = native or (not(s.revoked.contains(d)) and (owner == -1 or (owner == x.account and s.deviceOrigin.get(d) == origin)))
    val ok = database and deviceOK and not(s.closed.contains(x.account))
    all { x.phase == "verified", s.sessions.get(j).phase == "unused",
      s' = {...s, challenges: s.challenges.set(c, {...x, phase: "done", session: if (ok) j else -1,
        result: if (ok) "committed" else if (not(native) and s.revoked.contains(d)) "device_key_revoked" else "issue_refused"}),
        sessions: if (ok) s.sessions.set(j, newSession(x.account, x.tenant, if (native) -1 else d, "native")) else s.sessions,
        deviceOwner: if (ok and not(native)) s.deviceOwner.set(d, x.account) else s.deviceOwner,
        deviceOrigin: if (ok and not(native)) s.deviceOrigin.set(d, origin) else s.deviceOrigin,
        grants: if (ok and x.kind == "near_ai" and signupGrant) s.grants.union(Set(d)) else s.grants,
        frame: "identity_result", outcome: if (ok) "session_committed_response_pending" else "issue_refused"} }
  }
  action TheSessionResponseArrivesOrIsLost = {
    nondet j = Sessions.oneOf()
    nondet delivered = Set(true, false).oneOf()
    all { s.sessions.get(j).phase != "unused", not(s.responsesResolved.contains(j)),
      s' = {...s, responsesResolved: s.responsesResolved.union(Set(j)), received: if (delivered) s.received.union(Set(j)) else s.received, frame: "identity_result",
        outcome: if (delivered) "response_delivered_saved_authority" else "committed_session_response_lost"} }
  }
  action TheExpiredOrConsumedHandleIsReplayed = {
    nondet c = Challenges.oneOf()
    val x = s.challenges.get(c)
    all { Set("taken", "verified", "done").contains(x.phase),
      s' = {...s, frame: "identity_result", outcome: if (s.outcome == "uniform_deny") "replay_uniform_deny" else "uniform_deny"} }
  }
  action TheChallengeOrLoginLinkExpires = {
    nondet c = Challenges.oneOf()
    nondet link = Set(true, false).oneOf()
    val x = s.challenges.get(c)
    val l = s.links.get(c)
    all { if (link) l.phase == "live" and not(l.expired) else Set("pending", "code").contains(x.phase) and not(x.expired),
      s' = {...s, links: if (link) s.links.set(c, {...l, expired: true}) else s.links,
        challenges: if (link) s.challenges else s.challenges.set(c, {...x, expired: true}), frame: "identity_challenge", outcome: "expiry_boundary"} }
  }
  action TheIngestProcessRestarts = {
    all { Challenges.exists(c => { val x = s.challenges.get(c)
      x.kind == "native" and Set("pending", "code").contains(x.phase) }),
      s' = {...s, challenges: Challenges.mapBy(c => { val x = s.challenges.get(c)
        if (x.kind == "native" and Set("pending", "code").contains(x.phase)) {...x, phase: "done", result: "process_store_lost"} else x }),
        frame: "identity_challenge", outcome: "native_memory_lost_provisioning_rows_preserved"} }
  }
  action TheAccountRequestPresentsCredentials = {
    nondet r = Requests.oneOf()
    nondet j = Sessions.oneOf()
    nondet token = 0.to(2).oneOf()
    nondet tenant = Accounts.oneOf()
    nondet claim = Accounts.oneOf()
    nondet transport = Set("native", "browser", "device", "mixed").oneOf()
    val row = s.sessions.get(j)
    // Native kind refusal occurs AFTER database validation and its mutations.
    val dispatch = transport == "native" or transport == "browser"
    val accepted = dispatch and row.phase == "live" and row.tenant == tenant and not(row.expired) and not(row.idle)
      and not(s.closed.contains(row.account)) and (row.token == token or (row.previous == token and row.grace))
    val authorityAllowed = transport == "browser" or (transport == "native" and row.kind == "native")
    all { s.auth.get(r).phase == "unused",
      s' = {...s, auth: s.auth.set(r, {...emptyAuth, phase: if (accepted) "validated" else "done", session: j,
        token: token, tenant: tenant, account: if (accepted) row.account else -1, claimedAccount: claim, transport: transport,
        canonicalToken: row.token, rotatePlanned: accepted and token == row.token and row.due, authorityAllowed: authorityAllowed}),
        sessions: if (dispatch and not(accepted) and row.phase == "live" and row.tenant == tenant and row.token == token and row.idle and not(row.expired)) s.sessions.set(j, {...row, phase: "revoked"}) else s.sessions,
        frame: "identity_authority", outcome: if (accepted) "session_database_validation_snapshot" else "credentials_refused"} }
  }
  action TheRotationUpdateAndMiddlewareFinish = {
    nondet r = Requests.oneOf()
    val a = s.auth.get(r)
    val j = if (a.session < 0) 0 else a.session
    val row = s.sessions.get(j)
    val rotate = a.rotatePlanned and row.phase == "live" and not(row.expired)
      and a.canonicalToken == row.token and row.token < 2
    all { a.phase == "validated",
      s' = {...s, auth: s.auth.set(r, {...a, phase: if (a.authorityAllowed) "authorized" else "done",
        account: if (a.authorityAllowed) a.account else -1, rotated: if (rotate and a.authorityAllowed) row.token + 1 else -1}),
        sessions: if (rotate) s.sessions.set(j, {...row, previous: row.token, token: row.token + 1, grace: true, due: false,
          seen: if (row.seen < 2) row.seen + 1 else 2})
          else if (not(a.rotatePlanned) and a.canonicalToken == row.token) s.sessions.set(j, {...row, seen: if (row.seen < 2) row.seen + 1 else 2}) else s.sessions,
        frame: "identity_authority", outcome: if (not(a.authorityAllowed)) (if (rotate) "session_rotated_then_native_kind_refused" else "session_validated_then_native_kind_refused")
          else if (rotate) (if (row.kind == "native" and not(row.capped)) "native_rotation_sliding_expiry" else "rotation_fixed_or_capped_expiry") else "validated_snapshot_no_new_token"} }
  }
  action YouReadAnAccountSurface = {
    nondet r = Requests.oneOf()
    val a = s.auth.get(r)
    all { controls(s.frame).contains("YouReadAnAccountSurface"), a.phase == "authorized",
      s' = {...s, auth: s.auth.set(r, {...a, phase: "done"}), frame: "identity_authority",
        outcome: "handler_uses_saved_tenant_account_ignores_claimed_account"} }
  }
  action YouSignOutTheCurrentSession = {
    nondet r = Requests.oneOf()
    val a = s.auth.get(r)
    val j = if (a.session < 0) 0 else a.session
    all { controls(s.frame).contains("YouSignOutTheCurrentSession"), a.phase == "authorized",
      s' = {...s, sessions: s.sessions.set(j, {...s.sessions.get(j), phase: "revoked"}),
        auth: s.auth.set(r, {...a, phase: "done"}), frame: "identity_revocation", outcome: "whole_session_row_revoked_current_or_previous_token"} }
  }
  action YouSignOutEverywhere = {
    nondet r = Requests.oneOf()
    val a = s.auth.get(r)
    all { controls(s.frame).contains("YouSignOutEverywhere"), a.phase == "authorized",
      s' = {...s, sessions: Sessions.mapBy(j => { val row = s.sessions.get(j)
        if (row.phase == "live" and row.tenant == a.tenant and row.account == a.account) {...row, phase: "revoked"} else row }),
        auth: s.auth.set(r, {...a, phase: "done"}), frame: "identity_revocation", outcome: "account_sessions_revoked_keys_and_grants_retained"} }
  }
  action YouRevokeAnAccountDevice = {
    nondet r = Requests.oneOf()
    nondet d = Devices.oneOf()
    val a = s.auth.get(r)
    val ok = s.deviceOwner.get(d) == a.account and not(s.revoked.contains(d))
    all { controls(s.frame).contains("YouRevokeAnAccountDevice"), a.phase == "authorized",
      s' = {...s, revoked: if (ok) s.revoked.union(Set(d)) else s.revoked, grants: if (ok) s.grants.exclude(Set(d)) else s.grants,
        sessions: if (ok) Sessions.mapBy(j => { val row = s.sessions.get(j)
          if (row.phase == "live" and row.account == a.account and row.tenant == a.tenant and
            (row.device == d or (row.device == -1 and row.kind == "native" and j != a.session and not(row.expired) and not(row.idle)))) {...row, phase: "revoked"} else row }) else s.sessions,
        auth: s.auth.set(r, {...a, phase: "done"}), frame: "identity_revocation", outcome: if (ok) "device_key_grants_keyed_and_other_unattributed_native_sessions_revoked" else "uniform_not_found"} }
  }
  action TheSessionClockCrossesABoundary = {
    nondet j = Sessions.oneOf()
    nondet clock = Set("rotation", "grace", "expiry", "idle", "cap").oneOf()
    val row = s.sessions.get(j)
    all { row.phase == "live", if (clock == "rotation") not(row.due) else if (clock == "grace") row.grace
      else if (clock == "expiry") not(row.expired) else if (clock == "idle") not(row.idle) else not(row.capped),
      s' = {...s, sessions: s.sessions.set(j, {...row, due: row.due or clock == "rotation", grace: row.grace and clock != "grace",
        expired: row.expired or clock == "expiry", idle: row.idle or clock == "idle", capped: row.capped or clock == "cap"}),
        frame: "identity_authority", outcome: clock} }
  }
  // Device-code mint and renewal rate/signature/skew checks are one coarse predicate.
  action YouMintAHeadlessDeviceCode = {
    nondet r = Requests.oneOf()
    nondet c = Challenges.oneOf()
    nondet evidence = Set(true, false).oneOf()
    nondet d = Devices.oneOf()
    val a = s.auth.get(r)
    // The body supplies a signed minter key independently of session attribution.
    val ok = a.transport == "native" and evidence and s.deviceOwner.get(d) == a.account
      and not(s.revoked.contains(d)) and not(s.renewable.contains(d))
    all { controls(s.frame).contains("YouMintAHeadlessDeviceCode"), a.phase == "authorized", s.codes.get(c).phase == "unused",
      s' = {...s, codes: if (ok) s.codes.set(c, {phase: "live", account: a.account, minter: d, expired: false}) else s.codes,
        auth: s.auth.set(r, {...a, phase: "done", suppliedMinter: d}), frame: "identity_headless", outcome: if (ok) "device_code_minted" else "device_code_mint_refused"} }
  }
  action TheHeadlessDeviceCodeIsRedeemed = {
    nondet c = Challenges.oneOf()
    nondet d = Devices.oneOf()
    nondet j = Sessions.oneOf()
    nondet signatureAndDatabase = Set(true, false).oneOf()
    nondet signupGrant = Set(true, false).oneOf()
    val code = s.codes.get(c)
    val minter = if (code.minter < 0) 0 else code.minter
    val owner = s.deviceOwner.get(d)
    val ok = signatureAndDatabase and code.phase == "live" and not(code.expired) and not(s.revoked.contains(minter))
      and not(s.closed.contains(code.account)) and not(s.revoked.contains(d))
      and (owner == -1 or (owner == code.account and s.deviceOrigin.get(d) == "near_ai"))
    all { code.phase != "unused", s.sessions.get(j).phase == "unused",
      s' = {...s, codes: if (ok) s.codes.set(c, {...code, phase: "redeemed"}) else s.codes,
        sessions: if (ok) s.sessions.set(j, newSession(code.account, code.account, d, "native")) else s.sessions,
        deviceOwner: if (ok) s.deviceOwner.set(d, code.account) else s.deviceOwner,
        deviceOrigin: if (ok) s.deviceOrigin.set(d, "near_ai") else s.deviceOrigin,
        renewable: if (ok) s.renewable.union(Set(d)) else s.renewable,
        grants: if (ok and signupGrant) s.grants.union(Set(d)) else s.grants,
        frame: "identity_headless", outcome: if (ok) "atomic_code_consume_device_link_session" else "device_code_refused_unconsumed"} }
  }
  action TheHeadlessDeviceRenewsWithItsKey = {
    nondet j = Sessions.oneOf()
    nondet d = Devices.oneOf()
    nondet n = Set(0, 1).oneOf()
    nondet signatureSkewAndInterval = Set(true, false).oneOf()
    val a = s.deviceOwner.get(d)
    val nonce = a * 2 + n
    val ok = signatureSkewAndInterval and a >= 0 and s.renewable.contains(d) and not(s.revoked.contains(d))
      and not(s.closed.contains(a)) and not(s.renewalNonces.contains(nonce))
    all { s.sessions.get(j).phase == "unused", s.renewable.contains(d),
      s' = {...s, renewalNonces: if (ok) s.renewalNonces.union(Set(nonce)) else s.renewalNonces,
        sessions: if (ok) s.sessions.set(j, newSession(a, a, d, "native")) else s.sessions,
        frame: "identity_headless", outcome: if (ok) "headless_key_renewal_committed" else "renewal_refused_nonce_skew_interval_or_authority"} }
  }
  action TheHeadlessDeviceCodeExpires = {
    nondet c = Challenges.oneOf()
    val code = s.codes.get(c)
    all { code.phase == "live", not(code.expired), s' = {...s, codes: s.codes.set(c, {...code, expired: true}),
      frame: "identity_headless", outcome: "device_code_expired"} }
  }
  action step = any { YouStartASignInCeremony, YouCancelTheLocalCeremony, YouMintABrowserLoginLink,
    YouConfirmTheBrowserLogin, TheFinishRequestTakesTheStoredCodeOrCeremony, TheStoredBindingAndExternalEvidenceAreChecked,
    TheSessionIssueTransactionCommitsOrRefuses, TheSessionResponseArrivesOrIsLost, TheExpiredOrConsumedHandleIsReplayed,
    TheChallengeOrLoginLinkExpires, TheIngestProcessRestarts, TheAccountRequestPresentsCredentials,
    TheRotationUpdateAndMiddlewareFinish, YouReadAnAccountSurface, YouSignOutTheCurrentSession, YouSignOutEverywhere,
    YouRevokeAnAccountDevice, TheSessionClockCrossesABoundary, YouMintAHeadlessDeviceCode, TheHeadlessDeviceCodeIsRedeemed, TheHeadlessDeviceRenewsWithItsKey, TheHeadlessDeviceCodeExpires }
}
