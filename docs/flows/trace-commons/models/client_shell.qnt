// Current behavior projection at 8218cc8; no product promises selected.
module client_shell {
  var s: {
    screen: str,
    startup: str,
    daemonOwner: str,
    window: bool,
    monitorWindow: bool,
    settingsWindow: bool,
    firstRunWindow: bool,
    status: str,
    subscription: str,
    needsOnboarding: bool,
    pendingDestination: str,
    destination: str,
    firstStep: str,
    privateOnly: bool,
    watches: bool,
    rootsSaved: bool,
    sharing: str,
    scopesSaved: bool,
    rulesSaved: bool,
    pastIncluded: bool,
    grantReady: bool,
    grant: str,
    enrollment: int,
    session: str,
    nearAi: bool,
    accountAnswer: str,
    ceremony: str,
    call: str,
    resume: str,
    completion: str,
    error: str,
    settings: int,
    settingsDraft: int,
    updateMode: str,
    update: str,
    startupCancelled: bool
  }
  val vars = s

  pure def controls(frame: str): Set[str] =
    if (frame == "shell_starting") Set("Quit")
    else     if (frame == "shell_refused") Set("Settings", "Quit")
    else     if (frame == "welcome") Set("Continue", "Close", "Quit")
    else     if (frame == "tools") Set("Watch", "Customize", "Use Private AI only", "Continue", "Back", "Close", "Quit")
    else     if (frame == "uses") Set("Ask each time", "Automatic", "Continue", "Back", "Close", "Quit")
    else     if (frame == "traces") Set("Continue", "Back", "Close", "Quit")
    else     if (frame == "private_ai") Set("Continue", "Back", "Close", "Quit")
    else     if (frame == "join") Set("Sign in", "Use invite", "Start sharing", "Back", "Close", "Quit")
    else     if (frame == "shell_busy") Set("Cancel", "Close", "Quit")
    else     if (frame == "shell_error") Set("Retry", "Back", "Close", "Quit")
    else     if (frame == "shell_home") Set("Private AI", "Traces", "Insights", "Settings", "Sign out", "Close", "Quit")
    else     if (frame == "shell_offline") Set("Settings", "Retry", "Close", "Quit")
    else     if (frame == "shell_settings") Set("Save settings", "Check for updates", "Sign in again", "Sign out", "Set up again", "Close", "Quit")
    else     if (frame == "shell_settings_saving") Set("Close", "Quit")
    else     if (frame == "shell_update_busy") Set("Close", "Quit")
    else     if (frame == "shell_update_external") Set("Settings", "Close", "Quit")
    else     if (frame == "shell_empty") Set("Open", "Quit")
    else     if (frame == "shell_stopped") Set("Launch")
    else Set()

  action init = s' = {
    screen: "shell_starting",
    startup: "starting",
    daemonOwner: "none",
    window: true,
    monitorWindow: true,
    settingsWindow: false,
    firstRunWindow: false,
    status: "unread",
    subscription: "none",
    needsOnboarding: true,
    pendingDestination: "none",
    destination: "home",
    firstStep: "welcome",
    privateOnly: false,
    watches: false,
    rootsSaved: false,
    sharing: "unchosen",
    scopesSaved: false,
    rulesSaved: false,
    pastIncluded: false,
    grantReady: false,
    grant: "none",
    enrollment: 0,
    session: "none",
    nearAi: false,
    accountAnswer: "none",
    ceremony: "none",
    call: "none",
    resume: "tools",
    completion: "none",
    error: "none",
    settings: 0,
    settingsDraft: 0,
    updateMode: "unknown",
    update: "idle",
    startupCancelled: false
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:1730-1805
  action TheDaemonStarts = all {
    s.startup == "starting" and not(s.startupCancelled),
    s' = { ...s, startup: "running", daemonOwner: "local", subscription: "subscribed", status: "checking" }
  }

  // macos/Sources/TraceCommonsApp/DaemonStartup.swift:32-52
  action TheExistingDaemonAttaches = all {
    s.startup == "starting" and not(s.startupCancelled),
    s' = { ...s, startup: "running", daemonOwner: "attached", subscription: "subscribed", status: "checking" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:1757-1761
  action TheDaemonNeedsDeclaredRoots = all {
    s.startup == "starting" and not(s.startupCancelled),
    s' = { ...s, startup: "needs_roots", screen: "welcome", status: "known", firstRunWindow: true }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:1696-1714,1759-1761
  action TheDaemonRefusesStartup = all {
    s.startup == "starting" and not(s.startupCancelled),
    s' = { ...s, startup: "refused", screen: "shell_refused", error: "startup-refused" }
  }

  // macos/Sources/TraceCommonsApp/MonitorNavigation.swift:225-283
  action TheStatusRequiresFirstRun = all {
    s.startup == "running" and s.status == "checking",
    s' = { ...s, status: "answered", needsOnboarding: true, screen: "welcome", firstRunWindow: true }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:2005-2010; MonitorNavigation.swift:225-235
  action TheStatusRestoresAnEnrolledInstall = all {
    s.startup == "running" and s.status == "checking",
    s' = { ...s, status: "answered", needsOnboarding: false, enrollment: 1, session: "active", completion: "tenant", screen: "shell_home" }
  }

  // macos/Sources/TraceCommonsApp/MonitorNavigation.swift:268-283
  action TheStatusReadFails = all {
    s.startup == "running" and s.status == "checking",
    s' = { ...s, status: "failed", screen: "shell_offline" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunNavigation.swift:12,79-81
  action YouContinueFromWelcome = all {
    controls(s.screen).contains("Continue") and (s.screen == "welcome"),
    s' = { ...s, screen: "tools", firstStep: "tools" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunNavigation.swift:58-64; FirstRunState.swift:80-100
  action YouChooseToolsToWatch = all {
    controls(s.screen).contains("Watch") and (s.screen == "tools"),
    s' = { ...s, watches: true, privateOnly: false }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunNavigation.swift:35-49
  action YouChoosePrivateAIOnly = all {
    controls(s.screen).contains("Use Private AI only") and (s.screen == "tools"),
    s' = { ...s, privateOnly: true, watches: false, sharing: "unchosen", scopesSaved: false, rulesSaved: false, pastIncluded: false }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift:107-112,139-146
  action YouCustomizeFolders = all {
    controls(s.screen).contains("Customize") and (s.screen == "tools" and s.watches),
    s' = { ...s, screen: "shell_busy", call: "declare_roots", resume: "tools" }
  }

  // macos/Sources/TraceCommonsApp/FirstRunRunner.swift:182-211
  action YouCommitTools = all {
    controls(s.screen).contains("Continue") and (s.screen == "tools" and (s.watches or s.privateOnly)),
    s' = { ...s, screen: "shell_busy", call: "leave_tools", resume: "tools" }
  }

  // macos/Sources/TraceCommonsApp/FirstRunRunner.swift:238-248; macos/Sources/TCShellCore/FirstRun/FirstRunNavigation.swift:18-20
  action TheRootsDeclarationIsSaved = all {
    Set("declare_roots", "leave_tools").contains(s.call),
    s' = { ...s, rootsSaved: true, startup: "running", daemonOwner: "local", subscription: "subscribed", call: "none", screen: if(s.call == "declare_roots") "tools" else if(s.privateOnly) "private_ai" else "uses", firstStep: if(s.call == "declare_roots") "tools" else if(s.privateOnly) "private_ai" else "uses" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunState.swift:104-117
  action YouChooseAsk = all {
    controls(s.screen).contains("Ask each time") and (s.screen == "uses"),
    s' = { ...s, sharing: "ask", grantReady: false }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunNavigation.swift:185-191
  action YouChooseAutomatic = all {
    controls(s.screen).contains("Automatic") and (s.screen == "uses"),
    s' = { ...s, sharing: "automatic", grantReady: false }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunNavigation.swift:155-158
  action YouContinueFromUses = all {
    controls(s.screen).contains("Continue") and (s.screen == "uses" and s.sharing != "unchosen"),
    s' = { ...s, screen: "traces", firstStep: "traces" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunNavigation.swift:12,79-81
  action YouContinueFromTraces = all {
    controls(s.screen).contains("Continue") and (s.screen == "traces"),
    s' = { ...s, screen: "private_ai", firstStep: "private_ai" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunNavigation.swift:12,79-81
  action YouContinueFromPrivateAI = all {
    controls(s.screen).contains("Continue") and (s.screen == "private_ai"),
    s' = { ...s, screen: "join", firstStep: "join" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift:149-168
  action YouStartNearAISignIn = all {
    controls(s.screen).contains("Sign in") and (s.screen == "join" and s.rootsSaved),
    s' = { ...s, screen: "shell_busy", call: "browser", resume: "join", accountAnswer: "near_ai", ceremony: "waiting" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift:151-162
  action YouUseAnInvite = all {
    controls(s.screen).contains("Use invite") and (s.screen == "join" and s.rootsSaved),
    s' = { ...s, screen: "shell_busy", call: "invite_lookup", resume: "join", accountAnswer: "near_ai" }
  }

  // macos/Sources/TraceCommonsApp/FirstRunRunner.swift:249-264
  action TheInviteIsFound = all {
    s.call == "invite_lookup",
    s' = { ...s, call: "invite_enroll" }
  }

  // macos/Sources/TraceCommonsApp/FirstRunRunner.swift:265-274
  action TheInviteEnrollsThisMac = all {
    s.call == "invite_enroll",
    s' = { ...s, enrollment: 1, call: "browser", ceremony: "waiting" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift:27-94; macos/Sources/TraceCommonsApp/FirstRunRunner.swift:275-294
  action TheBrowserLoginCompletes = all {
    s.call == "browser" and s.ceremony == "waiting",
    s' = { ...s, nearAi: true, ceremony: "complete", call: if(s.enrollment == 0) "near_ai_enroll" else "account_sign_in" }
  }

  // macos/Sources/TraceCommonsApp/FirstRunRunner.swift:295-307
  action TheAccountEnrollmentCompletes = all {
    s.call == "near_ai_enroll",
    s' = { ...s, enrollment: 1, session: "active", call: "none", screen: "join" }
  }

  // macos/Sources/TraceCommonsApp/FirstRunRunner.swift:275-282
  action TheEnrolledAccountSignInCompletes = all {
    s.call == "account_sign_in",
    s' = { ...s, session: "active", call: "none", screen: "join" }
  }

  // macos/Sources/TraceCommonsApp/FirstRunRunner.swift:118-131; macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift:42-61
  action YouCancelTheBrowserLogin = all {
    controls(s.screen).contains("Cancel") and (s.call == "browser" and s.ceremony == "waiting"),
    s' = { ...s, call: "cancel_browser", ceremony: "cancelling" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift:42-61
  action TheBrowserCancellationCompletes = all {
    s.call == "cancel_browser",
    s' = { ...s, call: "none", ceremony: "cancelled", screen: "join" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunNavigation.swift:113-125
  action TheLoginFails = all {
    Set("browser", "invite_lookup", "invite_enroll", "near_ai_enroll", "account_sign_in").contains(s.call),
    s' = { ...s, call: "none", screen: "join", error: "sign-in-failed", accountAnswer: "none", ceremony: "failed" }
  }


  // macos/Sources/TCShellCore/FirstRun/FirstRunState.swift:122-128; FirstRunPlan.swift:238-242 [disclosure UI is coarse input]
  action TheSharingDisclosuresAreAccepted = all {
    s.screen == "join" and s.sharing == "automatic" and s.enrollment != 0 and not(s.grantReady),
    s' = { ...s, grantReady: true }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift:175-178,228-291
  action YouStartSetupCompletion = all {
    controls(s.screen).contains("Start sharing") and (s.screen == "join" and ((s.privateOnly and s.enrollment != 0 and not(s.watches)) or (not(s.privateOnly) and ((s.accountAnswer == "watch_only" and s.enrollment == 0) or (s.enrollment != 0 and s.sharing != "unchosen"))))),
    s' = { ...s, screen: "shell_busy", call: if(s.privateOnly) "marker" else if(s.accountAnswer == "watch_only") "rules" else if(s.sharing == "ask") "ask" else "scopes", resume: "join" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift:235-238
  action TheGlobalAskIsSaved = all {
    s.call == "ask",
    s' = { ...s, call: "scopes", grant: "none" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift:237-238
  action TheConsentScopesAreSaved = all {
    s.call == "scopes",
    s' = { ...s, scopesSaved: true, call: "rules" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift:247-259
  action TheEarlyFolderRulesAreSaved = all {
    s.call == "rules",
    s' = { ...s, rulesSaved: true, call: "past" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift:262-274 [two folder orders coarsened]
  action ThePastSessionsAreIncluded = all {
    s.call == "past",
    s' = { ...s, pastIncluded: true, call: if(s.sharing == "automatic" and s.grantReady and s.enrollment != 0) "grant" else "marker" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift:280-284
  action TheAutomaticGrantIsSaved = all {
    s.call == "grant",
    s' = { ...s, grant: "automatic", call: "late_ask_rules" }
  }

  // macos/Sources/TraceCommonsApp/FirstRunRunner.swift:88-93,171-180
  action TheAutomaticGrantIsRefused = all {
    s.call == "grant",
    s' = { ...s, grant: "refused", sharing: "ask", error: "grant-refused", call: "late_ask_rules" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift:241-244,284-285
  action TheExplicitAskRulesAreSavedAfterGrant = all {
    s.call == "late_ask_rules",
    s' = { ...s, call: "marker" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift:286; macos/Sources/TraceCommonsApp/FirstRunRunner.swift:69-77
  action TheCompletionMarkerIsWritten = all {
    s.call == "marker",
    s' = { ...s, completion: if(s.enrollment == 0) "watch_only" else "tenant", needsOnboarding: false, firstRunWindow: false, monitorWindow: true, call: "none", screen: "shell_home", destination: if(s.pendingDestination == "none") "home" else s.pendingDestination, pendingDestination: "none" }
  }

  // macos/Sources/TraceCommonsApp/FirstRunRunner.swift:182-210,88-93
  action TheCurrentSetupCallFails = all {
    Set("declare_roots", "leave_tools", "ask", "scopes", "rules", "past", "late_ask_rules", "marker").contains(s.call),
    s' = { ...s, screen: "shell_error", error: "daemon-call-failed" }
  }

  // macos/Sources/TraceCommonsApp/FirstRunRunner.swift:117-122 [retry plan is abstracted to failed call]
  action YouRetryTheFailedSetupCall = all {
    controls(s.screen).contains("Retry") and (s.screen == "shell_error"),
    s' = { ...s, screen: "shell_busy", error: "none" }
  }

  // macos/Sources/TCShellCore/FirstRun/FirstRunNavigation.swift:87-94
  action YouReturnToThePreviousFirstRunStage = all {
    controls(s.screen).contains("Back") and (Set("tools", "uses", "traces", "private_ai", "join").contains(s.screen) and s.call == "none"),
    s' = { ...s, screen: if(s.screen == "join") "private_ai" else if(s.screen == "private_ai") (if(s.privateOnly) "tools" else "traces") else if(s.screen == "traces") "uses" else if(s.screen == "uses") "tools" else "welcome", firstStep: if(s.screen == "join") "private_ai" else if(s.screen == "private_ai") (if(s.privateOnly) "tools" else "traces") else if(s.screen == "traces") "uses" else if(s.screen == "uses") "tools" else "welcome", grantReady: false }
  }

  // macos/Sources/TraceCommonsApp/MonitorNavigation.swift:239-265
  action YouOpenSettings = all {
    controls(s.screen).contains("Settings") and (s.startup != "stopped"),
    s' = { ...s, screen: "shell_settings", window: true, settingsWindow: true }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:2120-2264 [settings schema coarse]
  action YouSaveSettings = all {
    controls(s.screen).contains("Save settings") and (s.screen == "shell_settings" and s.startup == "running" and s.settings < 1),
    s' = { ...s, settingsDraft: 1, screen: "shell_settings_saving" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:2120-2264
  action TheSettingsSaveCompletes = all {
    s.screen == "shell_settings_saving",
    s' = { ...s, settings: s.settingsDraft, screen: "shell_settings" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:2120-2264
  action TheSettingsSaveFails = all {
    s.screen == "shell_settings_saving",
    s' = { ...s, screen: "shell_settings", error: "settings-save-failed" }
  }

  // macos/Sources/TraceCommonsApp/MonitorNavigation.swift:263-265
  action YouRequestPrivateAI = all {
    controls(s.screen).contains("Private AI") and (s.screen == "shell_home"),
    s' = { ...s, destination: "inference" }
  }

  // macos/Sources/TraceCommonsApp/MonitorNavigation.swift:32-42
  action YouRequestTraces = all {
    controls(s.screen).contains("Traces") and (s.screen == "shell_home"),
    s' = { ...s, destination: "traces" }
  }

  // macos/Sources/TraceCommonsApp/MonitorNavigation.swift:32-42
  action YouRequestInsights = all {
    controls(s.screen).contains("Insights") and (s.screen == "shell_home"),
    s' = { ...s, destination: "insights" }
  }

  // macos/Sources/TraceCommonsApp/MonitorNavigation.swift:89-125,153-158
  action ADeepLinkWaitsForOnboarding = all {
    s.needsOnboarding and s.pendingDestination == "none",
    s' = { ...s, pendingDestination: "traces" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:1892-1895
  action TheSubscriptionFallsBehind = all {
    s.subscription == "subscribed",
    s' = { ...s, subscription: "lagged", status: "checking" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:1892-1895 [managed/queue/status read outcomes coarse]
  action TheExplicitResyncCompletes = all {
    s.subscription == "lagged",
    s' = { ...s, subscription: "subscribed", status: "answered" }
  }

  // crates/trace-commons-contributor/src/daemon/account_session_renewal.rs:330-446
  action TheAccountSessionExpires = all {
    s.session == "active",
    s' = { ...s, session: "expired" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:3765-3798
  action YouSignInAgain = all {
    controls(s.screen).contains("Sign in again") and (s.enrollment != 0 and s.call == "none"),
    s' = { ...s, call: "account_sign_in", screen: "shell_busy", resume: "shell_settings" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:4755-4792
  action YouSignOut = all {
    controls(s.screen).contains("Sign out") and (s.startup == "running" and s.call == "none"),
    s' = { ...s, call: "sign_out", screen: "shell_busy", resume: "shell_settings" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:4763-4792 [enrollment persists]
  action TheLocalSignOutCompletes = all {
    s.call == "sign_out",
    s' = { ...s, session: "none", nearAi: false, call: "none", screen: "shell_settings", error: "none" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:4774-4782
  action TheSignOutRevocationFailsRemotely = all {
    s.call == "sign_out",
    s' = { ...s, session: "none", nearAi: false, call: "none", screen: "shell_settings", error: "remote-key-not-revoked" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:4777-4780
  action TheSignOutCannotClearTheLocalLogin = all {
    s.call == "sign_out",
    s' = { ...s, session: "none", call: "none", screen: "shell_settings", error: "near-ai-still-signed-in" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:4746-4753
  action YouSetUpAgain = all {
    controls(s.screen).contains("Set up again") and (s.screen == "shell_settings" and s.call == "none"),
    s' = { ...s, call: "unenroll", screen: "shell_busy", resume: "shell_settings" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:4746-4753 [account reset coarse]
  action TheUnenrollmentCompletes = all {
    s.call == "unenroll",
    s' = { ...s, enrollment: 0, session: "none", nearAi: false, needsOnboarding: true, completion: "none", call: "none", screen: "welcome" }
  }

  // macos/Sources/TraceCommonsApp/UpdateController.swift:31-40
  action TheInstallerOwnsUpdates = all {
    s.updateMode == "unknown",
    s' = { ...s, updateMode: "external" }
  }

  // macos/Sources/TraceCommonsApp/UpdateController.swift:31-52
  action TheAppOwnsUpdates = all {
    s.updateMode == "unknown",
    s' = { ...s, updateMode: "self" }
  }

  // macos/Sources/TraceCommonsApp/UpdateController.swift:72-81
  action YouCheckForUpdates = all {
    controls(s.screen).contains("Check for updates") and (s.updateMode != "unknown"),
    s' = { ...s, screen: if(s.updateMode == "self") "shell_update_busy" else "shell_update_external", update: if(s.updateMode == "self") "checking" else "external" }
  }

  // macos/Sources/TraceCommonsApp/UpdateController.swift:60-69,76-80 [Sparkle outcomes external]
  action TheUpdateCheckFinishes = all {
    s.update == "checking",
    s' = { ...s, update: "checked", screen: if(s.window) "shell_settings" else "shell_empty" }
  }

  // macos/Sources/TraceCommonsApp/MainWindowNavigation.swift:24-30 [window lifetime differs from app]
  action YouCloseTheWindow = all {
    controls(s.screen).contains("Close") and (s.window),
    s' = { ...s, settingsWindow: if(Set("shell_settings", "shell_settings_saving", "shell_update_busy", "shell_update_external").contains(s.screen)) false else s.settingsWindow, monitorWindow: if(Set("shell_settings", "shell_settings_saving", "shell_update_busy", "shell_update_external").contains(s.screen)) s.monitorWindow else false, firstRunWindow: if(Set("shell_settings", "shell_settings_saving", "shell_update_busy", "shell_update_external").contains(s.screen)) s.firstRunWindow else false, window: if(Set("shell_settings", "shell_settings_saving", "shell_update_busy", "shell_update_external").contains(s.screen)) (s.monitorWindow or s.firstRunWindow) else s.settingsWindow, screen: if(Set("shell_settings", "shell_settings_saving", "shell_update_busy", "shell_update_external").contains(s.screen) and (s.monitorWindow or s.firstRunWindow)) (if(s.needsOnboarding) s.firstStep else "shell_home") else if(not(Set("shell_settings", "shell_settings_saving", "shell_update_busy", "shell_update_external").contains(s.screen)) and s.settingsWindow) "shell_settings" else "shell_empty" }
  }

  // macos/Sources/TraceCommonsApp/MainWindowNavigation.swift:43-55; MonitorNavigation.swift:89-100
  action YouReopenTheWindow = all {
    controls(s.screen).contains("Open") and (not(s.window) and s.startup != "stopped"),
    s' = { ...s, window: true, monitorWindow: not(s.needsOnboarding), firstRunWindow: s.needsOnboarding, screen: if(s.call != "none") "shell_busy" else if(s.needsOnboarding) s.firstStep else "shell_home" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:1924-1948
  action YouQuit = all {
    controls(s.screen).contains("Quit") and (s.startup != "stopped"),
    s' = { ...s, startup: "stopped", monitorWindow: false, settingsWindow: false, firstRunWindow: false, startupCancelled: true, daemonOwner: "none", subscription: "none", window: false, screen: "shell_stopped", call: "none" }
  }

  // macos/Sources/TraceCommonsApp/DaemonStartup.swift:72-96,157-163
  action TheCancelledStartupResultIsDisposed = all {
    s.startupCancelled and s.startup == "stopped",
    s' = { ...s, error: "late-start-disposed" }
  }

  // macos/Sources/TraceCommonsApp/AppModel.swift:1687-1730 [persistent roots/marker retained]
  action YouLaunchAgain = all {
    controls(s.screen).contains("Launch") and (s.startup == "stopped"),
    s' = { ...s, startup: "starting", monitorWindow: true, startupCancelled: false, screen: "shell_starting", window: true, status: "unread" }
  }



  // macos/Sources/TraceCommonsApp/AppModel.swift:2005-2010; MonitorNavigation.swift:268-283
  action YouRetryTheStatusRead = all {
    controls(s.screen).contains("Retry") and s.screen == "shell_offline",
    s' = { ...s, status: "checking", screen: "shell_starting" }
  }

  action step = any {
    YouRetryTheStatusRead,
    TheDaemonStarts,
    TheExistingDaemonAttaches,
    TheDaemonNeedsDeclaredRoots,
    TheDaemonRefusesStartup,
    TheStatusRequiresFirstRun,
    TheStatusRestoresAnEnrolledInstall,
    TheStatusReadFails,
    YouContinueFromWelcome,
    YouChooseToolsToWatch,
    YouChoosePrivateAIOnly,
    YouCustomizeFolders,
    YouCommitTools,
    TheRootsDeclarationIsSaved,
    YouChooseAsk,
    YouChooseAutomatic,
    YouContinueFromUses,
    YouContinueFromTraces,
    YouContinueFromPrivateAI,
    YouStartNearAISignIn,
    YouUseAnInvite,
    TheInviteIsFound,
    TheInviteEnrollsThisMac,
    TheBrowserLoginCompletes,
    TheAccountEnrollmentCompletes,
    TheEnrolledAccountSignInCompletes,
    YouCancelTheBrowserLogin,
    TheBrowserCancellationCompletes,
    TheLoginFails,
    TheSharingDisclosuresAreAccepted,
    YouStartSetupCompletion,
    TheGlobalAskIsSaved,
    TheConsentScopesAreSaved,
    TheEarlyFolderRulesAreSaved,
    ThePastSessionsAreIncluded,
    TheAutomaticGrantIsSaved,
    TheAutomaticGrantIsRefused,
    TheExplicitAskRulesAreSavedAfterGrant,
    TheCompletionMarkerIsWritten,
    TheCurrentSetupCallFails,
    YouRetryTheFailedSetupCall,
    YouReturnToThePreviousFirstRunStage,
    YouOpenSettings,
    YouSaveSettings,
    TheSettingsSaveCompletes,
    TheSettingsSaveFails,
    YouRequestPrivateAI,
    YouRequestTraces,
    YouRequestInsights,
    ADeepLinkWaitsForOnboarding,
    TheSubscriptionFallsBehind,
    TheExplicitResyncCompletes,
    TheAccountSessionExpires,
    YouSignInAgain,
    YouSignOut,
    TheLocalSignOutCompletes,
    TheSignOutRevocationFailsRemotely,
    TheSignOutCannotClearTheLocalLogin,
    YouSetUpAgain,
    TheUnenrollmentCompletes,
    TheInstallerOwnsUpdates,
    TheAppOwnsUpdates,
    YouCheckForUpdates,
    TheUpdateCheckFinishes,
    YouCloseTheWindow,
    YouReopenTheWindow,
    YouQuit,
    TheCancelledStartupResultIsDisposed,
    YouLaunchAgain
  }
}
