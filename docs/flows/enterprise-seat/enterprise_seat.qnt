// Enterprise Near AI Gateway: one member's Mac and one server seat through the life of an org.
// Source: docs/superpowers/specs/2026-11-11-enterprise-gateway-design.md (sections 3.1, 4.4,
// 5.3, 5.6, 6.1, 7.1, 7.2, 9). Where the spec leaves a choice open, the model takes the spec's
// recommendation; the README lists each one.
module enterprise_seat {
  // ---- The org, as the server sees it -------------------------------------------------------
  type OrgStatus = Active | Suspended | Closing | Closed

  // ---- The member's Mac -----------------------------------------------------------------------
  // Personal: enrolled on its own, as today. Joining: the org admit request is in flight.
  // Managed: a member seat in the org. Window: removed by the admin, may still withdraw.
  // Removed: the window is over.
  type MacScreen = Personal | Joining | Managed | Window | Removed
  type Mode = Ask | Automatic
  // A trace the agent wrote. Waiting: held on the Mac. Claiming: the Mac asked the issuer for an
  // upload claim; nothing has left the Mac. Sending: the claim was granted and the upload is in
  // flight. Credited: the ledger holds it. Withdrawn: the member took it back.
  type Trace = NoTrace | Waiting | Claiming | Sending | Credited | Withdrawn
  // Why the trace left the Mac: the member pressed Submit, or an Automatic grant sent it.
  type Basis = YouApproved | YourAutomatic
  // Whose account the claim was minted for.
  type Holder = PersonalAccount | OrgAccount

  // ---- The server seat ------------------------------------------------------------------------
  type ServerSeat = NotEnrolled | Enrolled | Revoked
  type Job = NoJob | JobSending | JobCredited

  var org: { status: OrgStatus, sharing: bool, automaticAllowed: bool, serverTerms: bool, unsettled: bool }
  type Mac = {
    screen: MacScreen,
    session: bool,       // an org session with a fresh NEAR AI membership proof (spec 5.3)
    nearMember: bool,    // still in the bound NEAR AI organization
    mode: Mode,
    seenSharing: bool,   // the Mac's copy of org policy, fetched after the admin changes it
    seenAutomatic: bool,
    trace: Trace,
    basis: Basis,
    holder: Holder,
    // Bookkeeping for the promises, not shown to anyone:
    autoMine: bool,         // you turned Automatic on yourself since joining
    sentAutoMine: bool,     // the last automatic send ran on an Automatic you turned on in the org
    sentAutoAllowed: bool,  // the org allowed Automatic when the issuer granted the last claim
  }
  var mac: Mac

  // What held at the moment of the last credit, for the promises about credit. Each step that
  // credits nothing sets it to noCredit.
  type CreditKind = NoCredit | PersonalCredit | MemberCredit | ServerCredit
  type Credit = {
    kind: CreditKind, sharing: bool, orgActive: bool, seatActive: bool, session: bool,
    nearMember: bool, serverTerms: bool,
  }
  pure val noCredit: Credit = {
    kind: NoCredit, sharing: false, orgActive: false, seatActive: false, session: false,
    nearMember: false, serverTerms: false,
  }
  var credit: Credit

  var srv: { seat: ServerSeat, job: Job }

  // The controls each mock frame shows, in the mock's words.
  pure def controls(m: Mac): Set[str] = match m.screen {
    | Personal => Set("Join your organization")
        .union(if (m.mode == Ask) Set("Automatic") else Set("Ask"))
        .union(if (m.trace == Waiting and m.mode == Ask) Set("Submit") else Set())
    | Joining => Set()
    | Managed =>
        (if (not(m.session)) Set("Sign in again") else Set())
        .union(if (m.seenSharing and m.seenAutomatic and m.mode == Ask) Set("Automatic") else Set())
        .union(if (m.mode == Automatic) Set("Ask") else Set())
        .union(if (m.session and m.seenSharing and m.trace == Waiting and m.mode == Ask) Set("Submit") else Set())
        .union(if (m.session and m.trace == Credited and m.holder == OrgAccount) Set("Withdraw") else Set())
    | Window => if (m.trace == Credited and m.holder == OrgAccount) Set("Withdraw") else Set()
    | Removed => Set()
  }.union(
    // "Before you joined": a withdraw-only personal credential stays on the Mac (fix for promise 8).
    if (m.trace == Credited and m.holder == PersonalAccount) Set("Withdraw") else Set()
  )

  pure def shows(control: str, m: Mac): bool = control.in(controls(m))

  action init = all {
    org' = { status: Active, sharing: true, automaticAllowed: true, serverTerms: false, unsettled: false },
    // Before joining, the member used Gateway personally and had turned on Automatic.
    mac' = {
      screen: Personal, session: false, nearMember: true, mode: Automatic, seenSharing: true,
      seenAutomatic: true, trace: NoTrace, basis: YouApproved, holder: PersonalAccount,
      autoMine: false, sentAutoMine: false, sentAutoAllowed: true,
    },
    srv' = { seat: NotEnrolled, job: NoJob },
    credit' = noCredit,
  }

  // =============================== You, on your Mac ===========================================

  action YouPressJoin = all {
    shows("Join your organization", mac),
    mac' = { ...mac, screen: Joining },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  // The server checks NEAR AI org membership and the org's status (spec 5.3, path A).
  // Joining voids standing grants: GrantTerms now carry the org policy digest (spec 3.1).
  action TheJoinSucceeds = all {
    mac.screen == Joining, mac.nearMember, org.status == Active,
    mac' = {
      ...mac, screen: Managed, session: true, mode: Ask, autoMine: false,
      seenSharing: org.sharing, seenAutomatic: org.automaticAllowed,
    },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  action TheJoinFails = all {
    mac.screen == Joining,
    mac' = { ...mac, screen: Personal },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  action YouChooseAutomatic = all {
    shows("Automatic", mac),
    mac' = { ...mac, mode: Automatic, autoMine: mac.screen == Managed },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  action YouChooseAsk = all {
    shows("Ask", mac),
    mac' = { ...mac, mode: Ask },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  // The claim is minted from the device key, not the session (spec 2.2, 5.3).
  action YouPressSubmit = all {
    shows("Submit", mac),
    mac' = {
      ...mac, trace: Claiming, basis: YouApproved,
      holder: if (mac.screen == Personal) PersonalAccount else OrgAccount,
    },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  action YouPressWithdraw = all {
    shows("Withdraw", mac),
    mac' = { ...mac, trace: Withdrawn },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  action YouSignInAgain = all {
    shows("Sign in again", mac),
    // The org renewal route repeats the NEAR AI membership check (spec 5.3).
    mac' = { ...mac, session: mac.nearMember },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  // =============================== Your Mac, on its own =======================================

  action TheAgentWritesATrace = all {
    mac.screen.in(Set(Personal, Managed)),
    mac.trace.in(Set(NoTrace, Credited, Withdrawn)),
    mac' = { ...mac, trace: Waiting },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  // Automatic is enforced on the Mac, from the policy the Mac last fetched (spec 4.4).
  action YourMacSharesAutomatically = all {
    mac.trace == Waiting, mac.mode == Automatic,
    or {
      mac.screen == Personal,
      // Fix for promise 11: nothing is sent while the Mac is signed out of the org.
      mac.screen == Managed and mac.session and mac.seenSharing and mac.seenAutomatic,
    },
    mac' = {
      ...mac, trace: Claiming, basis: YourAutomatic, sentAutoMine: mac.autoMine,
      holder: if (mac.screen == Personal) PersonalAccount else OrgAccount,
    },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  // A widening change voids standing grants; a narrowing one takes effect without voiding
  // (spec 4.4). Forbidding Automatic turns it off on the Mac.
  action YourMacFetchesPolicy = all {
    mac.screen == Managed,
    mac.seenSharing != org.sharing or mac.seenAutomatic != org.automaticAllowed,
    val widened = (not(mac.seenSharing) and org.sharing) or (not(mac.seenAutomatic) and org.automaticAllowed)
    mac' = {
      ...mac, seenSharing: org.sharing, seenAutomatic: org.automaticAllowed,
      mode: if (widened or not(org.automaticAllowed)) Ask else mac.mode,
    },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  // Fix for promise 3: an org claim carries the policy digest the Mac acted on and how the trace
  // was sent. The issuer refuses a stale digest, and an Automatic claim the org forbids.
  pure def claimAllowed(m: Mac, o: { status: OrgStatus, sharing: bool, automaticAllowed: bool, serverTerms: bool, unsettled: bool }): bool = or {
    m.holder == PersonalAccount,
    and {
      m.holder == OrgAccount,
      m.seenSharing == o.sharing, m.seenAutomatic == o.automaticAllowed,
      m.basis == YourAutomatic implies o.automaticAllowed,
    },
  }

  action TheClaimIsGranted = all {
    mac.trace == Claiming, claimAllowed(mac, org),
    mac' = { ...mac, trace: Sending, sentAutoAllowed: org.automaticAllowed },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  // The issuer answers a claim it cannot grant with a refusal; the trace goes back to Held.
  action TheIssuerRefusesTheClaim = all {
    mac.trace == Claiming, not(claimAllowed(mac, org)),
    mac' = { ...mac, trace: Waiting },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  // The claim request is lost or times out.
  action TheClaimRequestFails = all {
    mac.trace == Claiming,
    mac' = { ...mac, trace: Waiting },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  // A personal claim is the personal account's business. An org claim needs an active org that
  // shares, an active seat, and a fresh membership proof (spec 4.4, 5.3, 5.6).
  action TheTraceIsCredited = all {
    mac.trace == Sending,
    or {
      mac.holder == PersonalAccount,
      and {
        mac.holder == OrgAccount, org.status == Active, org.sharing,
        mac.screen == Managed, mac.session,
      },
    },
    mac' = { ...mac, trace: Credited },
    org' = { ...org, unsettled: org.unsettled or mac.holder == OrgAccount },
    srv' = srv,
    credit' = {
      kind: if (mac.holder == OrgAccount) MemberCredit else PersonalCredit,
      sharing: org.sharing, orgActive: org.status == Active, seatActive: mac.screen == Managed,
      session: mac.session, nearMember: mac.nearMember, serverTerms: org.serverTerms,
    },
  }

  // The network fails, or the issuer or ingest refuses. The trace goes back to waiting.
  action TheTraceIsRefused = all {
    mac.trace == Sending,
    mac' = { ...mac, trace: Waiting },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  action YourSessionExpires = all {
    mac.screen == Managed, mac.session,
    mac' = { ...mac, session: false },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  action TheWindowEnds = all {
    mac.screen == Window,
    mac' = { ...mac, screen: Removed },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  // =============================== NEAR AI ====================================================

  action NearAiRemovesYouFromTheOrg = all {
    mac.nearMember,
    mac' = { ...mac, nearMember: false },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  action NearAiAddsYouBack = all {
    not(mac.nearMember),
    mac' = { ...mac, nearMember: true },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  // =============================== The org admin ==============================================

  // Removal revokes the device key, sessions and grant; a closing window follows (spec 5.6).
  action TheAdminRemovesYou = all {
    mac.screen == Managed,
    mac' = { ...mac, screen: Window, mode: Ask },
    org' = org, srv' = srv,
    credit' = noCredit,
  }

  action TheAdminTurnsSharingOff = all {
    org.sharing,
    org' = { ...org, sharing: false },
    mac' = mac, srv' = srv,
    credit' = noCredit,
  }

  action TheAdminTurnsSharingOn = all {
    not(org.sharing),
    org' = { ...org, sharing: true },
    mac' = mac, srv' = srv,
    credit' = noCredit,
  }

  action TheAdminForbidsAutomatic = all {
    org.automaticAllowed,
    org' = { ...org, automaticAllowed: false },
    mac' = mac, srv' = srv,
    credit' = noCredit,
  }

  action TheAdminAllowsAutomatic = all {
    not(org.automaticAllowed),
    org' = { ...org, automaticAllowed: true },
    mac' = mac, srv' = srv,
    credit' = noCredit,
  }

  // The org consents for its servers by accepting the server terms once (spec 7.2).
  action TheAdminAcceptsServerTerms = all {
    not(org.serverTerms),
    org' = { ...org, serverTerms: true },
    mac' = mac, srv' = srv,
    credit' = noCredit,
  }

  action TheAdminEnrolsTheServer = all {
    srv.seat == NotEnrolled, org.status == Active,
    srv' = { ...srv, seat: Enrolled },
    org' = org, mac' = mac,
    credit' = noCredit,
  }

  action TheAdminRevokesTheServer = all {
    srv.seat == Enrolled,
    srv' = { ...srv, seat: Revoked },
    org' = org, mac' = mac,
    credit' = noCredit,
  }

  action TheAdminClosesTheOrg = all {
    org.status == Active,
    org' = { ...org, status: Closing },
    mac' = mac, srv' = srv,
    credit' = noCredit,
  }

  // =============================== The operator and settlement ================================

  action TheOperatorSuspendsTheOrg = all {
    org.status == Active,
    org' = { ...org, status: Suspended },
    mac' = mac, srv' = srv,
    credit' = noCredit,
  }

  action TheOperatorResumesTheOrg = all {
    org.status == Suspended,
    org' = { ...org, status: Active },
    mac' = mac, srv' = srv,
    credit' = noCredit,
  }

  // Settlement credits the org's holder account (spec 6.1).
  action TheCreditSettles = all {
    org.unsettled, org.status != Closed,
    org' = { ...org, unsettled: false },
    mac' = mac, srv' = srv,
    credit' = noCredit,
  }

  action TheOrgCloses = all {
    org.status == Closing, not(org.unsettled),
    org' = { ...org, status: Closed },
    mac' = mac, srv' = srv,
    credit' = noCredit,
  }

  // =============================== The server =================================================

  // The server daemon's terms hold reads the org's acceptance (spec 7.2).
  action TheServerSharesATrace = all {
    srv.seat == Enrolled, srv.job.in(Set(NoJob, JobCredited)), org.serverTerms,
    srv' = { ...srv, job: JobSending },
    org' = org, mac' = mac,
    credit' = noCredit,
  }

  action TheServerTraceIsCredited = all {
    srv.job == JobSending, srv.seat == Enrolled, org.status == Active, org.sharing, org.serverTerms,
    srv' = { ...srv, job: JobCredited },
    org' = { ...org, unsettled: true },
    mac' = mac,
    credit' = {
      kind: ServerCredit, sharing: org.sharing, orgActive: org.status == Active,
      seatActive: srv.seat == Enrolled, session: false, nearMember: false, serverTerms: org.serverTerms,
    },
  }

  action TheServerTraceIsRefused = all {
    srv.job == JobSending,
    srv' = { ...srv, job: NoJob },
    org' = org, mac' = mac,
    credit' = noCredit,
  }

  action step = any {
    YouPressJoin, TheJoinSucceeds, TheJoinFails, YouChooseAutomatic, YouChooseAsk,
    YouPressSubmit, YouPressWithdraw, YouSignInAgain, TheClaimIsGranted, TheIssuerRefusesTheClaim, TheClaimRequestFails,
    TheAgentWritesATrace, YourMacSharesAutomatically, YourMacFetchesPolicy, TheTraceIsCredited,
    TheTraceIsRefused, YourSessionExpires, TheWindowEnds,
    NearAiRemovesYouFromTheOrg, NearAiAddsYouBack,
    TheAdminRemovesYou, TheAdminTurnsSharingOff, TheAdminTurnsSharingOn, TheAdminForbidsAutomatic,
    TheAdminAllowsAutomatic, TheAdminAcceptsServerTerms, TheAdminEnrolsTheServer,
    TheAdminRevokesTheServer, TheAdminClosesTheOrg,
    TheOperatorSuspendsTheOrg, TheOperatorResumesTheOrg, TheCreditSettles, TheOrgCloses,
    TheServerSharesATrace, TheServerTraceIsCredited, TheServerTraceIsRefused,
  }

  val vars = (org, mac, srv, credit)

  // =============================== Promises ===================================================

  /// 1. Joining never carries your Automatic over: when you join, every folder is back on Ask.
  val JoiningResetsAutomatic =
    (mac.screen == Managed and mac.mode == Automatic) implies mac.autoMine

  /// 2. Your traces leave only by your act: every trace your Mac sends to the org was submitted
  /// by you, or sent by an Automatic you turned on after joining.
  val OnlyYourActSends =
    (mac.holder == OrgAccount and mac.trace.in(Set(Sending, Credited)) and mac.basis == YourAutomatic)
      implies mac.sentAutoMine

  /// 3. If the org forbids Automatic, nothing of yours goes automatically.
  val ForbiddenAutomaticSendsNothing =
    (mac.holder == OrgAccount and mac.trace.in(Set(Sending, Credited)) and mac.basis == YourAutomatic)
      implies mac.sentAutoAllowed

  /// 4. Sharing off means no new credit: while sharing is off, nothing from you or a server is
  /// credited.
  val SharingOffMeansNoCredit =
    credit.kind.in(Set(MemberCredit, ServerCredit)) implies credit.sharing

  /// 5. Removal stops uploads: after the admin removes you, nothing more from your Mac is
  /// credited, including an upload already in flight.
  val RemovalStopsUploads = credit.kind == MemberCredit implies credit.seatActive

  /// 6. Leaving the NEAR AI org stops uploads by your next sign-in: once you are out of the NEAR AI
  /// org and your session has expired, nothing more is credited.
  val LeavingNearAiStopsUploads =
    credit.kind == MemberCredit implies (credit.nearMember or credit.session)

  /// 7. You can take back what you gave the org: a trace credited to the org can be withdrawn
  /// while you are signed in or during the removal window.
  val OrgCreditWithdrawable =
    (mac.trace == Credited and mac.holder == OrgAccount
      and ((mac.screen == Managed and mac.session) or mac.screen == Window))
      implies shows("Withdraw", mac)

  /// 8. What you earned personally stays withdrawable.
  val PersonalCreditWithdrawable =
    (mac.trace == Credited and mac.holder == PersonalAccount) implies shows("Withdraw", mac)

  /// 9. Servers share only under the org's consent: a server trace is credited only after the org
  /// accepted the server terms, and only while the server is enrolled.
  val ServersShareOnlyUnderOrgConsent =
    credit.kind == ServerCredit implies (credit.serverTerms and credit.seatActive)

  /// 10. The org closes cleanly: it closes only after its credit settles, and a closed org
  /// credits nothing new.
  val OrgClosesCleanly = and {
    org.status == Closed implies not(org.unsettled),
    credit.kind.in(Set(MemberCredit, ServerCredit)) implies credit.orgActive,
  }

  /// 11. A submitted trace eventually lands: if you stay a member, keep signing in, sharing stays
  /// on, the network works, the org stops changing policy and your sign-in eventually lasts long
  /// enough, a trace you submit is eventually credited.
  /// (Owner rewrites, 2026-11-11. "The org stops changing policy": without it, an admin who
  /// toggles Automatic forever makes every claim stale, and the issuer refuses each one; that
  /// refusal is the fix for promise 3. "Your sign-in eventually lasts long enough": without it,
  /// a session that expires during every upload gets each one refused on arrival; ingest keeps
  /// that check so that promise 6 stays strict.)
  temporal SubmittedTraceLands = (
    always(and {
      mac.nearMember, org.status == Active, org.sharing,
      mac.screen != Window, mac.screen != Removed,
    })
    and (eventually(always(org.automaticAllowed)) or eventually(always(not(org.automaticAllowed))))
    and eventually(always(mac.session))
    and strongFair(TheClaimIsGranted, vars)
    and weakFair(TheIssuerRefusesTheClaim, vars)
    and strongFair(TheTraceIsCredited, vars)
    and strongFair(YouPressSubmit, vars)
    and strongFair(YourMacSharesAutomatically, vars)
    and weakFair(YouSignInAgain, vars)
    and weakFair(YourMacFetchesPolicy, vars)
  ) implies always(
    (mac.trace.in(Set(Claiming, Sending)) and mac.holder == OrgAccount) implies eventually(mac.trace == Credited)
  )
}
