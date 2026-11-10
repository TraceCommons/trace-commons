// Current behavior projection at 8218cc8; no product promises selected.
module client_services {
  var s: {
    screen: str,
    account: int,
    credentialGeneration: int,
    inferenceKey: int,
    cloudSession: bool,
    attempt: int,
    attemptState: str,
    callbackAttempt: int,
    organization: int,
    orgTarget: int,
    orgRequestGeneration: int,
    orgPhase: str,
    revocationPending: bool,
    listener: bool,
    provider: str,
    backend: str,
    harness: str,
    harnessPlan: str,
    harnessOperation: str,
    harnessRevision: int,
    planRevision: int,
    managedGeneration: int,
    prepareGeneration: int,
    prepareKey: int,
    prepare: str,
    sessions: int -> {phase: str, owner: int, generation: int, connection: str, ticket: str, terminal: str, redeemed: bool, stop: bool, collection: str, nativeId: bool, hidden: bool},
    selectedSession: int,
    prepareSession: int,
    balance: str,
    profile: str,
    profileOperation: str,
    profileRequestOwner: int,
    profileOwner: int,
    profileCached: bool,
    creditRequestOwner: int,
    credit: str,
    referral: str,
    payoutLink: str,
    linkOperation: str,
    device: str,
    error: str
  }
  val vars = s

  pure def controls(frame: str): Set[str] =
    if (frame == "services_home") Set("Sign in", "Connect", "Disconnect", "Launch", "Refresh", "Spending organization", "Private AI", "Account", "Close")
    else     if (frame == "services_login") Set("Cancel", "Private AI", "Account", "Close")
    else     if (frame == "services_org_empty") Set("Refresh", "Private AI", "Account", "Close")
    else     if (frame == "services_org") Set("Select organization", "Cancel", "Private AI", "Account", "Close")
    else     if (frame == "services_org_busy") Set("Cancel", "Private AI", "Account", "Close")
    else     if (frame == "services_busy") Set("Private AI", "Account", "Close")
    else     if (frame == "services_error") Set("Retry", "Refresh", "Private AI", "Account", "Close")
    else     if (frame == "services_offline") Set("Refresh", "Private AI", "Account", "Close")
    else     if (frame == "services_exposure") Set("Continue", "Cancel", "Private AI", "Account", "Close")
    else     if (frame == "services_plan") Set("Apply", "Cancel", "Private AI", "Account", "Close")
    else     if (frame == "services_launch") Set("Launch", "Cancel", "Private AI", "Account", "Close")
    else     if (frame == "services_sessions_empty") Set("Launch", "Private AI", "Account", "Close")
    else     if (frame == "services_session") Set("Stop", "Start again", "Resume", "Dismiss", "Select session", "Launch", "Refresh", "Private AI", "Account", "Close")
    else     if (frame == "services_account") Set("Update profile", "Withdraw", "Refresh", "Link", "Unlink", "Confirm", "Billing", "Devices", "Sign out", "Private AI", "Account", "Close")
    else     if (frame == "services_profile_busy") Set("Private AI", "Account", "Close")
    else     if (frame == "services_credit_empty") Set("Refresh", "Private AI", "Account", "Close")
    else     if (frame == "services_payout_confirm") Set("Confirm", "Cancel", "Private AI", "Account", "Close")
    else     if (frame == "services_devices") Set("Revoke device", "Refresh", "Private AI", "Account", "Close")
    else     if (frame == "services_closed") Set("Open")
    else Set()

  action init = s' = {
    screen: "services_home",
    account: 1,
    credentialGeneration: 0,
    inferenceKey: 0,
    cloudSession: false,
    attempt: 0,
    attemptState: "none",
    callbackAttempt: 0,
    organization: 0,
    orgTarget: 0,
    orgRequestGeneration: 0,
    orgPhase: "none",
    revocationPending: false,
    listener: false,
    provider: "near_ai",
    backend: "ready",
    harness: "disconnected",
    harnessPlan: "none",
    harnessOperation: "none",
    harnessRevision: 0,
    planRevision: 0,
    managedGeneration: 0,
    prepareGeneration: 0,
    prepareKey: 0,
    prepare: "none",
    sessions: Map(0 -> {phase: "absent", owner: 1, generation: 0, connection: "near_ai", ticket: "none", terminal: "none", redeemed: false, stop: false, collection: "off", nativeId: false, hidden: false}, 1 -> {phase: "absent", owner: 1, generation: 0, connection: "near_ai", ticket: "none", terminal: "none", redeemed: false, stop: false, collection: "off", nativeId: false, hidden: false}),
    selectedSession: 0,
    prepareSession: 0,
    balance: "unread",
    profile: "private",
    profileOperation: "none",
    profileRequestOwner: 1,
    profileOwner: 1,
    profileCached: true,
    creditRequestOwner: 1,
    credit: "unread",
    referral: "unread",
    payoutLink: "unlinked",
    linkOperation: "none",
    device: "active",
    error: "none"
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/ceremony.rs:1-35; macos/Sources/TraceCommonsApp/AppModel.swift:1129-1154
  action YouBeginBrowserSignIn = {
    nondet freshAttempt = Set(1, 2, 3).exclude(Set(s.attempt, s.callbackAttempt)).oneOf()
    all {
      controls(s.screen).contains("Sign in") and (s.attemptState != "waiting"),
      s' = { ...s, attempt: freshAttempt, callbackAttempt: freshAttempt, attemptState: "waiting", screen: "services_login" }
    }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/mod.rs:321-350
  action YouCancelBrowserSignIn = all {
    controls(s.screen).contains("Cancel") and (s.attemptState == "waiting"),
    s' = { ...s, attemptState: "cancelled", screen: "services_home" }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/mod.rs:107-169; session.rs:1-37 [validated callback transaction coarse]
  action TheBrowserLoginCommitsCurrentCredential = {
    nondet freshCredentialGeneration = Set(0, 1, 2).exclude(Set(s.credentialGeneration, s.orgRequestGeneration)).oneOf()
    all {
      s.attemptState == "waiting" and s.callbackAttempt == s.attempt,
      s' = { ...s, inferenceKey: s.attempt, cloudSession: true, attemptState: "complete", credentialGeneration: freshCredentialGeneration, organization: 1, screen: if(s.screen == "services_closed") s.screen else "services_home" }
    }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/ceremony.rs:1-35 [attempt/commit ownership projection]
  action TheOldBrowserCallbackIsIgnored = all {
    s.callbackAttempt < s.attempt or s.attemptState == "cancelled",
    s' = { ...s, error: "stale-browser-callback" }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/ceremony.rs:1-35 [named old attempt input]
  action AnOlderBrowserCallbackArrives = all {
    s.attempt == 2 and s.callbackAttempt == 2,
    s' = { ...s, callbackAttempt: 1 }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/mod.rs:107-169
  action TheBrowserLoginFails = all {
    s.attemptState == "waiting",
    s' = { ...s, attemptState: "failed", error: "sign-in-failed", screen: if(s.screen == "services_closed") s.screen else "services_error" }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/mod.rs:300-304 [inference key can outlive session]
  action TheCloudSessionExpires = all {
    s.cloudSession,
    s' = { ...s, cloudSession: false }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/mod.rs:624-637
  action YouOpenSpendingOrganizations = all {
    controls(s.screen).contains("Spending organization") and (s.cloudSession),
    s' = { ...s, orgPhase: "listing", screen: "services_busy" }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/spend_org.rs:1-35 [sole organization selected by shared logic]
  action TheOrganizationListHasOneChoice = all {
    s.orgPhase == "listing",
    s' = { ...s, organization: 1, orgPhase: "none", screen: if(s.screen == "services_closed") s.screen else "services_home" }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/mod.rs:624-637
  action TheOrganizationListHasMultipleChoices = all {
    s.orgPhase == "listing",
    s' = { ...s, orgPhase: "choosing", screen: if(s.screen == "services_closed") s.screen else "services_org" }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/mod.rs:624-637
  action TheOrganizationListIsEmpty = all {
    s.orgPhase == "listing",
    s' = { ...s, orgPhase: "none", screen: if(s.screen == "services_closed") s.screen else "services_org_empty" }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/mod.rs:639-662; spend_org.rs:192-211 [row selection is an abstract selector]
  action YouChooseAnotherSpendingOrganization = all {
    controls(s.screen).contains("Select organization") and (s.orgPhase == "choosing" and s.organization != 2),
    s' = { ...s, orgTarget: 2, orgRequestGeneration: s.credentialGeneration, orgPhase: "minting", screen: "services_org_busy" }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/spend_org.rs:192-201 [old key serves during propagation]
  action TheNewOrganizationKeyIsReady = all {
    s.orgPhase == "minting" and s.orgRequestGeneration == s.credentialGeneration and s.cloudSession,
    s' = { ...s, orgPhase: "ready" }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/mod.rs:639-662 [proxy move precedes old-key revoke]
  action TheOrganizationSwitchCommits = {
    nondet freshCredentialGeneration = Set(0, 1, 2).exclude(Set(s.credentialGeneration, s.orgRequestGeneration)).oneOf()
    all {
      s.orgPhase == "ready" and s.orgRequestGeneration == s.credentialGeneration,
      s' = { ...s, organization: s.orgTarget, inferenceKey: 2, credentialGeneration: freshCredentialGeneration, orgPhase: "revoking", revocationPending: true, screen: if(s.screen == "services_closed") s.screen else "services_home" }
    }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/mod.rs:639-662; spend_org.rs:234-249
  action TheOldOrganizationKeyIsRevoked = all {
    s.orgPhase == "revoking",
    s' = { ...s, revocationPending: false, orgPhase: "none" }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/mod.rs:643-660; spend_org.rs:192-194
  action TheOldKeyRevocationRemainsPending = all {
    s.orgPhase == "revoking",
    s' = { ...s, revocationPending: true, orgPhase: "none", error: "old-key-revoke-pending" }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/spend_org.rs:156-182,332-340 [guarded commit projection]
  action TheOrganizationSwitchLosesItsCredentialLease = all {
    Set("minting", "ready").contains(s.orgPhase) and (s.orgRequestGeneration != s.credentialGeneration or not(s.cloudSession)),
    s' = { ...s, orgPhase: "none", error: "organization-switch-stale", screen: if(s.screen == "services_closed") s.screen else "services_error" }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/mod.rs:624-637 [selection cancellation is presentation]
  action YouCancelOrganizationSelection = all {
    controls(s.screen).contains("Cancel") and (s.orgPhase == "choosing"),
    s' = { ...s, orgPhase: "none", screen: "services_home" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:633-645
  action YouConnectAHarness = all {
    controls(s.screen).contains("Connect") and (s.harnessOperation == "none" and s.harnessPlan == "none"),
    s' = { ...s, harnessOperation: if(s.listener) "planning" else "exposure", screen: if(s.listener) "services_busy" else "services_exposure" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:651-683 [settings write and plan remain separate]
  action YouAcceptLocalListenerExposure = all {
    controls(s.screen).contains("Continue") and (s.harnessOperation == "exposure"),
    s' = { ...s, listener: true, harnessOperation: "planning", screen: "services_busy" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:651-656
  action YouDeclineLocalListenerExposure = all {
    controls(s.screen).contains("Cancel") and (s.harnessOperation == "exposure"),
    s' = { ...s, harnessOperation: "none", screen: "services_home" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:633-645
  action YouDisconnectAHarness = all {
    controls(s.screen).contains("Disconnect") and (s.harnessOperation == "none" and s.harness == "connected"),
    s' = { ...s, harnessOperation: "planning_disconnect", screen: "services_busy" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:686-713
  action TheHarnessPlanIsReturned = all {
    Set("planning", "planning_disconnect").contains(s.harnessOperation),
    s' = { ...s, harnessPlan: if(s.harnessOperation == "planning_disconnect") "disconnect" else "connect", planRevision: s.harnessRevision, harnessOperation: "none", screen: if(s.screen == "services_closed") s.screen else "services_plan" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:731-749
  action YouApplyTheHarnessPlan = all {
    controls(s.screen).contains("Apply") and (s.harnessPlan != "none" and s.harnessOperation == "none"),
    s' = { ...s, harnessOperation: "committing", screen: "services_busy" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:718-721
  action YouCancelTheHarnessPlan = all {
    controls(s.screen).contains("Cancel") and (s.harnessPlan != "none" and s.harnessOperation == "none"),
    s' = { ...s, harnessPlan: "none", screen: "services_home" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:724-729 [external edit input]
  action TheHarnessFileChangesBehindThePlan = all {
    s.harnessPlan != "none" and s.harnessRevision == 0,
    s' = { ...s, harnessRevision: 1 }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:731-749
  action TheHarnessPlanCommits = all {
    s.harnessOperation == "committing" and s.planRevision == s.harnessRevision,
    s' = { ...s, harness: if(s.harnessPlan == "connect") "connected" else "disconnected", harnessPlan: "none", harnessOperation: "none", screen: if(s.screen == "services_closed") s.screen else "services_home" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:724-754
  action TheHarnessPlanIsRefusedOrExpired = all {
    s.harnessOperation == "committing",
    s' = { ...s, harnessPlan: "none", harnessOperation: "none", error: "write-unconfirmed", screen: if(s.screen == "services_closed") s.screen else "services_error" }
  }

  // crates/trace-commons-contributor/src/daemon/managed.rs:1040-1109 [external availability input]
  action TheInferenceBackendBecomesUnavailable = all {
    s.backend == "ready",
    s' = { ...s, backend: "unavailable", balance: "unavailable", screen: if(s.screen == "services_closed") s.screen else "services_offline" }
  }

  // crates/trace-commons-contributor/src/daemon/managed.rs:1040-1109
  action TheInferenceBackendRecovers = all {
    s.backend != "ready",
    s' = { ...s, backend: "ready" }
  }

  // crates/trace-commons-contributor/src/daemon/managed.rs:1060-1109
  action YouRequestAManagedLaunch = all {
    controls(s.screen).contains("Launch") and (s.prepare == "none" and s.sessions.get(s.selectedSession).phase == "absent"),
    s' = { ...s, prepare: "balance", prepareSession: s.selectedSession, prepareGeneration: s.managedGeneration, prepareKey: s.inferenceKey, screen: "services_busy" }
  }

  // crates/trace-commons-contributor/src/daemon/managed.rs:1081-1109
  action TheLaunchBalanceIsSufficient = all {
    s.prepare == "balance" and s.inferenceKey != 0 and s.inferenceKey == s.prepareKey and s.backend == "ready",
    s' = { ...s, balance: "sufficient", prepare: "prepare" }
  }

  // crates/trace-commons-contributor/src/daemon/managed.rs:1081-1109
  action TheLaunchBalanceIsInsufficient = all {
    s.prepare == "balance",
    s' = { ...s, balance: "insufficient", prepare: "none", error: "insufficient-balance", screen: if(s.screen == "services_closed") s.screen else "services_error" }
  }

  // crates/trace-commons-contributor/src/daemon/managed.rs:1094-1102
  action TheLaunchPrecheckIsInvalidated = all {
    s.prepare == "balance" and s.inferenceKey != s.prepareKey,
    s' = { ...s, prepare: "none", error: "near-ai-sign-in-required", screen: if(s.screen == "services_closed") s.screen else "services_error" }
  }

  // crates/trace-commons-contributor/src/managed/sessions.rs:903-979; daemon/managed.rs:324-325
  action TheManagedLaunchIsPrepared = all {
    s.prepare == "prepare" and s.prepareGeneration == s.managedGeneration and s.sessions.get(s.prepareSession).phase == "absent",
    s' = { ...s, sessions: s.sessions.set(s.prepareSession, { ...s.sessions.get(s.prepareSession), phase: "starting", owner: s.account, generation: s.managedGeneration, ticket: "live", collection: "pending" }), prepare: "none", screen: if(s.screen == "services_closed") s.screen else "services_session" }
  }

  // crates/trace-commons-contributor/src/daemon/managed.rs:600-608
  action TheAccountSelectionGenerationChanges = {
    nondet freshManagedGeneration = Set(0, 1, 2).exclude(Set(s.managedGeneration, s.prepareGeneration)).oneOf()
    all {
      true,
      s' = { ...s, managedGeneration: freshManagedGeneration }
    }
  }

  // crates/trace-commons-contributor/src/daemon/managed.rs:324-325
  action ThePreparedRequestGenerationIsStale = all {
    s.prepare == "prepare" and s.prepareGeneration != s.managedGeneration,
    s' = { ...s, prepare: "none", error: "selection-stale", screen: if(s.screen == "services_closed") s.screen else "services_error" }
  }

  // crates/trace-commons-contributor/src/daemon/managed.rs:155-208; managed/sessions.rs:552-571
  action TheTerminalLaunchBegins = all {
    s.sessions.get(s.selectedSession).phase == "starting" and s.sessions.get(s.selectedSession).ticket == "live" and s.sessions.get(s.selectedSession).terminal == "none",
    s' = { ...s, sessions: s.sessions.set(s.selectedSession, { ...s.sessions.get(s.selectedSession), terminal: "held" }) }
  }

  // crates/trace-commons-contributor/src/daemon/managed.rs:210-247
  action TheTerminalOpens = all {
    s.sessions.get(s.selectedSession).terminal == "held",
    s' = { ...s, sessions: s.sessions.set(s.selectedSession, { ...s.sessions.get(s.selectedSession), terminal: "opened" }) }
  }

  // crates/trace-commons-contributor/src/daemon/managed.rs:210-225 [held until report or ticket lapse]
  action TheTerminalLaunchResultIsUnknown = all {
    s.sessions.get(s.selectedSession).terminal == "held",
    s' = { ...s, sessions: s.sessions.set(s.selectedSession, { ...s.sessions.get(s.selectedSession), terminal: "unknown" }), error: "launch-unknown" }
  }

  // crates/trace-commons-contributor/src/daemon/managed.rs:210-225
  action TheTerminalLaunchFailsBeforeOpening = all {
    s.sessions.get(s.selectedSession).terminal == "held",
    s' = { ...s, sessions: s.sessions.set(s.selectedSession, { ...s.sessions.get(s.selectedSession), terminal: "none" }), error: "terminal-unavailable" }
  }

  // crates/trace-commons-contributor/src/managed/sessions.rs:1206-1244; daemon/managed.rs:1047-1050
  action TheLauncherRedeemsItsTicket = all {
    s.sessions.get(s.selectedSession).phase == "starting" and s.sessions.get(s.selectedSession).ticket == "live" and not(s.sessions.get(s.selectedSession).redeemed) and s.inferenceKey != 0,
    s' = { ...s, sessions: s.sessions.set(s.selectedSession, { ...s.sessions.get(s.selectedSession), redeemed: true, ticket: "redeemed" }) }
  }

  // crates/trace-commons-contributor/src/managed/sessions.rs:823-847
  action TheLaunchTicketExpires = all {
    s.sessions.get(s.selectedSession).phase == "starting" and not(s.sessions.get(s.selectedSession).redeemed),
    s' = { ...s, sessions: s.sessions.set(s.selectedSession, { ...s.sessions.get(s.selectedSession), phase: "failed", ticket: "expired", collection: "failed" }) }
  }

  // crates/trace-commons-contributor/src/managed/sessions.rs:1248-1367
  action TheToolReportsRunning = all {
    s.sessions.get(s.selectedSession).phase == "starting" and s.sessions.get(s.selectedSession).redeemed,
    s' = { ...s, sessions: s.sessions.set(s.selectedSession, { ...s.sessions.get(s.selectedSession), phase: "running", nativeId: true }) }
  }

  // crates/trace-commons-contributor/src/managed/sessions.rs:650-673
  action YouRequestTheManagedToolToStop = all {
    controls(s.screen).contains("Stop") and (s.sessions.get(s.selectedSession).phase == "running" and s.sessions.get(s.selectedSession).terminal != "none"),
    s' = { ...s, sessions: s.sessions.set(s.selectedSession, { ...s.sessions.get(s.selectedSession), stop: true }) }
  }

  // crates/trace-commons-contributor/src/managed/sessions.rs:1248-1367
  action TheToolReportsNormalExit = all {
    s.sessions.get(s.selectedSession).phase == "running",
    s' = { ...s, sessions: s.sessions.set(s.selectedSession, { ...s.sessions.get(s.selectedSession), phase: "exited" }) }
  }

  // crates/trace-commons-contributor/src/managed/sessions.rs:1248-1367
  action TheToolReportsFailure = all {
    s.sessions.get(s.selectedSession).phase == "running",
    s' = { ...s, sessions: s.sessions.set(s.selectedSession, { ...s.sessions.get(s.selectedSession), phase: "failed" }) }
  }

  // crates/trace-commons-contributor/src/managed/sessions.rs:30-36,823-860
  action TheManagedLeaseLapses = all {
    s.sessions.get(s.selectedSession).redeemed and Set("starting", "running").contains(s.sessions.get(s.selectedSession).phase),
    s' = { ...s, sessions: s.sessions.set(s.selectedSession, { ...s.sessions.get(s.selectedSession), phase: if(s.sessions.get(s.selectedSession).connection == "near_ai") "failed" else "unknown" }) }
  }

  // crates/trace-commons-contributor/src/managed/sessions.rs:1299-1308,1403-1435 [artifact capture interface coarse]
  action TheManagedCollectionCompletes = all {
    s.sessions.get(s.selectedSession).collection == "pending" and Set("exited", "failed").contains(s.sessions.get(s.selectedSession).phase),
    s' = { ...s, sessions: s.sessions.set(s.selectedSession, { ...s.sessions.get(s.selectedSession), collection: "collected" }) }
  }

  // crates/trace-commons-contributor/src/managed/sessions.rs:153-160,814-819 [actual can_dismiss conditions coarse]
  action YouDismissAnEndedManagedSession = all {
    controls(s.screen).contains("Dismiss") and (Set("exited", "failed", "unknown").contains(s.sessions.get(s.selectedSession).phase)),
    s' = { ...s, sessions: s.sessions.set(s.selectedSession, { ...s.sessions.get(s.selectedSession), hidden: true }) }
  }

  // crates/trace-commons-contributor/src/daemon/managed.rs:568-575 [two session selection abstraction]
  action YouSelectTheOtherManagedSession = all {
    controls(s.screen).contains("Select session") and (true),
    s' = { ...s, selectedSession: 1 - s.selectedSession }
  }

  // crates/trace-commons-contributor/src/managed/sessions.rs:676-708
  action YouStartTheManagedToolAgain = all {
    controls(s.screen).contains("Start again") and (Set("exited", "failed").contains(s.sessions.get(s.selectedSession).phase) and s.sessions.get(1 - s.selectedSession).phase == "absent"),
    s' = { ...s, prepareSession: 1 - s.selectedSession, prepare: "balance", prepareGeneration: s.managedGeneration, prepareKey: s.inferenceKey, screen: "services_busy" }
  }

  // crates/trace-commons-contributor/src/managed/sessions.rs:710-738 [root identity projection coarse]
  action YouResumeTheNativeConversation = all {
    controls(s.screen).contains("Resume") and (Set("exited", "failed").contains(s.sessions.get(s.selectedSession).phase) and s.sessions.get(s.selectedSession).nativeId and s.sessions.get(1 - s.selectedSession).phase == "absent"),
    s' = { ...s, prepareSession: 1 - s.selectedSession, prepare: "balance", prepareGeneration: s.managedGeneration, prepareKey: s.inferenceKey, screen: "services_busy" }
  }

  // macos/Sources/TraceCommonsApp/MonitorNavigation.swift:32-42
  action YouOpenPrivateAI = all {
    controls(s.screen).contains("Private AI") and (true),
    s' = { ...s, screen: "services_home" }
  }

  // crates/trace-commons-contributor/src/shell_words_copy.rs:1-30 [account destination abstracted]
  action YouOpenAccount = all {
    controls(s.screen).contains("Account") and (true),
    s' = { ...s, screen: "services_account" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:3540-3586
  action YouPublishOrUpdateAProfile = all {
    controls(s.screen).contains("Update profile") and (s.profileOperation == "none"),
    s' = { ...s, profileOperation: "publish", profileRequestOwner: s.account, screen: "services_profile_busy" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:3588-3611
  action YouWithdrawThePublicProfile = all {
    controls(s.screen).contains("Withdraw") and (s.profileOperation == "none" and s.profile == "public"),
    s' = { ...s, profileOperation: "withdraw", profileRequestOwner: s.account, screen: "services_profile_busy" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:3555-3576,3597-3606 [no account-generation guard in these callbacks]
  action TheProfileWriteSucceeds = all {
    s.profileOperation != "none",
    s' = { ...s, profile: if(s.profileOperation == "publish") "public" else "private", profileOwner: s.profileRequestOwner, profileOperation: "none", profileCached: true, screen: if(s.screen == "services_closed") s.screen else "services_account" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:3560-3566
  action TheProfilePublishesButLocalCacheWriteFails = all {
    s.profileOperation == "publish",
    s' = { ...s, profile: "public", profileOwner: s.profileRequestOwner, profileOperation: "none", profileCached: false, screen: if(s.screen == "services_closed") s.screen else "services_account" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:3577-3581,3607-3611
  action TheProfileWriteFails = all {
    s.profileOperation != "none",
    s' = { ...s, profileOperation: "none", error: "profile-write-refused", screen: if(s.screen == "services_closed") s.screen else "services_account" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:3509-3533
  action TheProfileRefreshFailsAndKeepsTheCache = all {
    s.profile == "public",
    s' = { ...s, error: "profile-refresh-unavailable" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:291-319 [contribution account read scoped to client/account]
  action YouReadAccountCreditAndReferrals = all {
    controls(s.screen).contains("Refresh") and (s.credit != "loading"),
    s' = { ...s, credit: "loading", referral: "loading", creditRequestOwner: s.account }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:308-315 [amounts/counts coarse; not a payout commit]
  action TheAccountCreditReadReturns = all {
    s.credit == "loading" and s.creditRequestOwner == s.account,
    s' = { ...s, credit: "pending", referral: "available" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:315-319
  action TheAccountCreditReadFails = all {
    s.credit == "loading" and s.creditRequestOwner == s.account,
    s' = { ...s, credit: "unavailable", referral: "unavailable" }
  }

  // crates/trace-commons-contributor/src/private_inference_copy.rs:1930-1955 [external browser funding interface]
  action YouOpenBilling = all {
    controls(s.screen).contains("Billing") and (true),
    s' = { ...s, error: "billing-browser-opened" }
  }

  // crates/trace-commons-contributor/src/daemon/payout_link.rs:660-706 [owned organizations and listing revision coarse]
  action YouStartLinkingAPayoutAccount = all {
    controls(s.screen).contains("Link") and (s.linkOperation == "none" and s.cloudSession),
    s' = { ...s, linkOperation: "link_listing", screen: "services_busy" }
  }

  // crates/trace-commons-contributor/src/daemon/payout_link.rs:706-785
  action ThePayoutLinkAwaitsConfirmation = all {
    s.linkOperation == "link_listing",
    s' = { ...s, payoutLink: "pending", linkOperation: "link_confirm", screen: if(s.screen == "services_closed") s.screen else "services_payout_confirm" }
  }

  // crates/trace-commons-contributor/src/daemon/payout_link.rs:787-835
  action YouStartUnlinkingAPayoutAccount = all {
    controls(s.screen).contains("Unlink") and (s.linkOperation == "none" and s.payoutLink == "linked"),
    s' = { ...s, linkOperation: "unlink_confirm", screen: "services_payout_confirm" }
  }

  // crates/trace-commons-contributor/src/daemon/payout_link.rs:837-894
  action YouConfirmThePayoutAccountChange = all {
    controls(s.screen).contains("Confirm") and (Set("link_confirm", "unlink_confirm").contains(s.linkOperation)),
    s' = { ...s, linkOperation: if(s.linkOperation == "link_confirm") "link_commit" else "unlink_commit", screen: "services_busy" }
  }

  // crates/trace-commons-contributor/src/daemon/payout_link.rs:837-894 [rail link, not credit settlement or issuance]
  action ThePayoutAccountChangeCompletes = all {
    Set("link_commit", "unlink_commit").contains(s.linkOperation),
    s' = { ...s, payoutLink: if(s.linkOperation == "link_commit") "linked" else "unlinked", linkOperation: "none", screen: if(s.screen == "services_closed") s.screen else "services_account" }
  }

  // crates/trace-commons-contributor/src/daemon/payout_link.rs:706-894 [pending handle/expiry/version guards coarse]
  action ThePayoutAccountChangeIsRefusedOrExpires = all {
    s.linkOperation != "none",
    s' = { ...s, linkOperation: "none", error: "payout-link-refused", screen: if(s.screen == "services_closed") s.screen else "services_account" }
  }

  // crates/trace-commons-contributor/src/daemon/ipc.rs:7092-7097; device_codes.rs:228-318 [device route family coarse]
  action YouOpenDeviceManagement = all {
    controls(s.screen).contains("Devices") and (true),
    s' = { ...s, screen: "services_devices" }
  }

  // crates/trace-commons-contributor/src/daemon/ipc.rs:7092-7097; device_codes.rs:228-318 [remote device authority interface, no device enumeration model]
  action YouRevokeAnotherDevice = all {
    controls(s.screen).contains("Revoke device") and (s.device == "active"),
    s' = { ...s, device: "revoke_pending" }
  }

  // crates/trace-commons-contributor/src/daemon/ipc.rs:7092-7097; device_codes.rs:228-318 [validated server result input]
  action TheDeviceRevocationReturns = all {
    s.device == "revoke_pending",
    s' = { ...s, device: "revoked" }
  }

  // crates/trace-commons-contributor/src/daemon/nearai_credential/mod.rs:446-518; macos/Sources/TraceCommonsApp/AppModel.swift:4763-4792
  action YouSignOutOfAccountServices = {
    nondet freshCredentialGeneration = Set(0, 1, 2).exclude(Set(s.credentialGeneration, s.orgRequestGeneration)).oneOf()
    all {
      controls(s.screen).contains("Sign out") and (true),
      s' = { ...s, account: 2, cloudSession: false, inferenceKey: 0, credentialGeneration: freshCredentialGeneration, organization: 0, credit: "unread", referral: "unread", error: "none" }
    }
  }

  // crates/trace-commons-contributor/src/managed/sessions.rs:865-881; daemon/managed.rs:1513-1532
  action TheLostLoginEndsManagedSignInSessions = all {
    s.sessions.get(s.selectedSession).connection == "near_ai" and s.inferenceKey == 0 and Set("starting", "running", "unknown").contains(s.sessions.get(s.selectedSession).phase),
    s' = { ...s, sessions: s.sessions.set(s.selectedSession, { ...s.sessions.get(s.selectedSession), phase: "failed", ticket: "none" }) }
  }

  // macos/Sources/TraceCommonsApp/MainWindowNavigation.swift:24-30
  action YouCloseTheServicesWindow = all {
    controls(s.screen).contains("Close") and (true),
    s' = { ...s, screen: "services_closed" }
  }

  // macos/Sources/TraceCommonsApp/MainWindowNavigation.swift:43-55
  action YouReopenTheServicesWindow = all {
    controls(s.screen).contains("Open") and (s.screen == "services_closed"),
    s' = { ...s, screen: "services_home" }
  }



  // macos/Sources/TraceCommonsApp/AppModel.swift:308-311: captured client/account scope guard.
  action TheOldAccountCreditReadIsDiscarded = all {
    s.credit == "loading" and s.creditRequestOwner != s.account,
    s' = { ...s, credit: "unread", referral: "unread" }
  }

  action step = any {
    TheOldAccountCreditReadIsDiscarded,
    YouBeginBrowserSignIn,
    YouCancelBrowserSignIn,
    TheBrowserLoginCommitsCurrentCredential,
    TheOldBrowserCallbackIsIgnored,
    AnOlderBrowserCallbackArrives,
    TheBrowserLoginFails,
    TheCloudSessionExpires,
    YouOpenSpendingOrganizations,
    TheOrganizationListHasOneChoice,
    TheOrganizationListHasMultipleChoices,
    TheOrganizationListIsEmpty,
    YouChooseAnotherSpendingOrganization,
    TheNewOrganizationKeyIsReady,
    TheOrganizationSwitchCommits,
    TheOldOrganizationKeyIsRevoked,
    TheOldKeyRevocationRemainsPending,
    TheOrganizationSwitchLosesItsCredentialLease,
    YouCancelOrganizationSelection,
    YouConnectAHarness,
    YouAcceptLocalListenerExposure,
    YouDeclineLocalListenerExposure,
    YouDisconnectAHarness,
    TheHarnessPlanIsReturned,
    YouApplyTheHarnessPlan,
    YouCancelTheHarnessPlan,
    TheHarnessFileChangesBehindThePlan,
    TheHarnessPlanCommits,
    TheHarnessPlanIsRefusedOrExpired,
    TheInferenceBackendBecomesUnavailable,
    TheInferenceBackendRecovers,
    YouRequestAManagedLaunch,
    TheLaunchBalanceIsSufficient,
    TheLaunchBalanceIsInsufficient,
    TheLaunchPrecheckIsInvalidated,
    TheManagedLaunchIsPrepared,
    TheAccountSelectionGenerationChanges,
    ThePreparedRequestGenerationIsStale,
    TheTerminalLaunchBegins,
    TheTerminalOpens,
    TheTerminalLaunchResultIsUnknown,
    TheTerminalLaunchFailsBeforeOpening,
    TheLauncherRedeemsItsTicket,
    TheLaunchTicketExpires,
    TheToolReportsRunning,
    YouRequestTheManagedToolToStop,
    TheToolReportsNormalExit,
    TheToolReportsFailure,
    TheManagedLeaseLapses,
    TheManagedCollectionCompletes,
    YouDismissAnEndedManagedSession,
    YouSelectTheOtherManagedSession,
    YouStartTheManagedToolAgain,
    YouResumeTheNativeConversation,
    YouOpenPrivateAI,
    YouOpenAccount,
    YouPublishOrUpdateAProfile,
    YouWithdrawThePublicProfile,
    TheProfileWriteSucceeds,
    TheProfilePublishesButLocalCacheWriteFails,
    TheProfileWriteFails,
    TheProfileRefreshFailsAndKeepsTheCache,
    YouReadAccountCreditAndReferrals,
    TheAccountCreditReadReturns,
    TheAccountCreditReadFails,
    YouOpenBilling,
    YouStartLinkingAPayoutAccount,
    ThePayoutLinkAwaitsConfirmation,
    YouStartUnlinkingAPayoutAccount,
    YouConfirmThePayoutAccountChange,
    ThePayoutAccountChangeCompletes,
    ThePayoutAccountChangeIsRefusedOrExpires,
    YouOpenDeviceManagement,
    YouRevokeAnotherDevice,
    TheDeviceRevocationReturns,
    YouSignOutOfAccountServices,
    TheLostLoginEndsManagedSignInSessions,
    YouCloseTheServicesWindow,
    YouReopenTheServicesWindow
  }
}
