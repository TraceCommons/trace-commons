// Current Windows/GTK/Tauri shell differences at 8218cc8. No selected promises.
module platform_clients {
  var s: {
    platform: str, screen: str, page: str, first: str, owner: str,
    alive: bool, visible: bool, roots: bool, external: bool, tray: bool,
    missions: bool, scan: bool, installed: bool, setup: bool, enrolled: bool,
    session: bool, scopes: bool, path: str, scrub: bool, witness: bool,
    grant: bool, req: str, answer: str, subscription: str, quit: bool,
    update: str, auth: str, change: str, changeSent: str, changeOpen: bool, fallback: bool, fallbackUsed: bool,
    generation: int, authGeneration: int, revision: int, savedRevision: int
  }
  val vars = s
  pure def controls(frame: str): Set[str] =
    if (frame == "platform_starting") Set("Quit")
    else if (frame == "platform_roots") Set("Declare sources", "Quit")
    else if (frame == "platform_local") Set("Open contributor", "Close")
    else if (frame == "platform_first_run") Set("Continue", "Back", "Enroll", "Choose scopes", "Decide later", "Automatic", "Ask each time", "Grant", "Close setup", "Quit")
    else if (frame == "platform_settings") Set("Navigate", "Save settings", "Check update", "Apply update", "Set up again", "Automatic contribution setup", "Change account", "Close", "Quit")
    else if (frame == "platform_workspace") Set("Navigate", "Sign in to withdraw", "Close", "Quit")
    else if (frame == "platform_change") Set("Continue", "Close account change", "Navigate", "Close", "Quit")
    else if (frame == "platform_auth") Set("Navigate", "Open sign-in page again", "Close", "Quit")
    else if (frame == "platform_update") Set("Navigate", "Apply update", "Close", "Quit")
    else if (frame == "platform_hidden") Set("Open", "Quit")
    else if (frame == "platform_quit") Set("Cancel", "Confirm quit", "Stop watcher too")
    else if (frame == "platform_offline") Set("Navigate", "Retry read", "Close", "Quit")
    else if (frame == "platform_refused") Set("Navigate", "Close", "Quit")
    else if (frame == "platform_not_found") Set("Navigate", "Close", "Quit")
    else if (frame == "platform_stopped") Set("Launch") else Set()

  action init = {
    nondet platform = Set("windows", "gtk", "tauri").oneOf()
    nondet roots = Set(false, true).oneOf()
    nondet external = Set(false, true).oneOf()
    nondet tray = Set(false, true).oneOf()
    nondet missions = Set(false, true).oneOf()
    nondet scan = Set(false, true).oneOf()
    nondet installed = Set(false, true).oneOf()
    nondet setup = Set(false, true).oneOf()
    s' = { platform: platform, screen: if(platform == "gtk") "local" else "starting",
      page: if(platform == "tauri" or platform == "gtk") "insights" else "waiting", first: "welcome",
      owner: "none", alive: true, visible: true, roots: roots, external: external,
      tray: tray, missions: missions, scan: scan, installed: installed, setup: setup,
      enrolled: setup, session: false, scopes: false, path: "none", scrub: false,
      witness: false, grant: false, req: "none", answer: "none", subscription: "none",
      quit: false, update: "idle", auth: "none", change: "none", changeSent: "none", changeOpen: false, fallback: false, fallbackUsed: false,
      generation: 0, authGeneration: 0, revision: 0, savedRevision: 0 } }
  // GTK main.rs:142-185: ordinary launch is local Insights, not daemon startup.
  action YouOpenTheGtkContributor = all {
    s.alive and s.screen == "local",
    s' = { ...s, screen: "starting", page: "waiting" } }
  // Backend::open attaches before roots; Windows FFI construction has no attach fallback.
  action TheExistingWatcherAttaches = all {
    s.alive and s.screen == "starting" and s.external and s.platform != "windows"
      and (s.platform == "gtk" or s.roots),
    s' = { ...s, owner: "attached", screen: "checking", req: "status", subscription: "live" } }
  action TheShellHostsTheWatcher = all {
    s.alive and s.screen == "starting" and s.roots and not(s.external),
    s' = { ...s, owner: "hosted", screen: "checking", req: "status", subscription: "live" } }
  action TheStartNeedsSourceDeclarations = all {
    s.alive and s.screen == "starting" and not(s.roots) and not(s.platform == "gtk" and s.external),
    s' = { ...s, screen: if(s.platform == "tauri") "first_run" else "roots", answer: "roots-not-declared" } }
  action YouDeclareTheSources = all {
    s.alive and s.screen == "roots",
    s' = { ...s, roots: true, screen: "starting", answer: "none" } }
  action WindowsFindsAnotherOwner = all {
    s.alive and s.screen == "starting" and s.platform == "windows" and s.roots and s.external,
    s' = { ...s, screen: "refused", answer: "already-running" } }
  action TheStartFails = all {
    s.alive and s.screen == "starting" and s.roots,
    s' = { ...s, screen: if(s.platform == "gtk") "stopped" else "offline",
      alive: s.platform != "gtk", subscription: if(s.platform == "tauri") "closed" else "none", answer: "startup-failed" } }
  // Per-tenant completion/CLI proof is separate from logged_in on Windows/GTK.
  action TheStatusRequiresSetup = all {
    s.alive and s.req == "status" and s.screen == "checking" and not(s.setup and s.enrolled),
    s' = { ...s, screen: "first_run", first: if(s.first == "connect") "connect" else "welcome", req: "none", setup: false } }
  action TheStatusRestoresCompletedSetup = all {
    s.alive and s.req == "status" and s.screen == "checking" and s.setup and s.enrolled,
    s' = { ...s, screen: "workspace", setup: true, enrolled: true, req: "none" } }
  action TheReadFails = all {
    s.alive and s.req == "status",
    s' = { ...s, req: "none", answer: "read-failed", screen: "offline" } }
  // Native Windows/GTK six steps differ from the twelve-step Tauri reducer.
  action YouContinueFirstRun = all {
    s.alive and s.screen == "first_run" and s.req == "none",
    Set("welcome", "roots", "scan", "privacy", "inference", "disclosure_scrub", "disclosure_witness", "projects", "done").contains(s.first),
    s' = { ...s,
      first: if(s.first == "welcome") (if(s.platform == "tauri") "roots" else "connect")
        else if(s.first == "roots") "connect" else if(s.first == "scan") "watch"
        else if(s.first == "projects") "done"
        else if(s.first == "privacy") "inference"
        else if(s.first == "inference") (if(s.path == "automatic") "disclosure_scrub" else "projects")
        else if(s.first == "disclosure_scrub") "disclosure_witness"
        else if(s.first == "disclosure_witness") "grant" else "done",
      roots: s.roots or s.first == "roots",
      scrub: s.scrub or s.first == "disclosure_scrub",
      witness: s.witness or s.first == "disclosure_witness",
      setup: s.setup or s.first == "projects",
      screen: if(s.first == "done") "workspace" else if(s.first == "roots" and s.owner == "none") "starting" else s.screen } }
  action YouFinishNativeWatching = all {
    s.alive and s.platform != "tauri" and s.screen == "first_run" and s.first == "watch" and s.req == "none",
    s' = { ...s, req: "completion" } }
  action TheNativeCompletionReadAnswers = {
    nondet persisted = Set(false, true).oneOf()
    all {
      s.alive and s.req == "completion",
      s' = { ...s, req: "none", first: "done", setup: persisted,
        answer: if(persisted) "completion-saved" else "completion-not-saved" }
    } }
  action YouEnrollFromConnect = all {
    s.alive and s.screen == "first_run" and s.first == "connect" and s.req == "none",
    s' = { ...s, req: "enroll", answer: "none" } }
  action EnrollmentAnswers = {
    nondet accepted = Set(false, true).oneOf()
    all {
    s.alive and s.req == "enroll",
    s' = { ...s, req: "none", enrolled: s.enrolled or accepted,
      first: if(accepted) "consent" else s.first, answer: if(accepted) "enrolled" else "invite-failed" }
    } }
  action YouChooseAndSaveScopes = all {
    s.alive and s.screen == "first_run" and s.first == "consent" and s.req == "none",
    s' = { ...s, req: "scopes", scopes: true } }
  // Both native shells advance after scope replies even when the daemon refused them.
  action ScopeSavingAnswers = {
    nondet saved = Set(false, true).oneOf()
    all {
    s.alive and s.req == "scopes",
    s' = { ...s, req: "none", answer: if(saved) "scopes-saved" else "scopes-failed",
      scopes: saved,
      first: if(saved or s.platform != "tauri") (if(s.platform == "tauri") "path" else if(s.scan) "scan" else "watch") else s.first }
    } }
  action YouChooseTheTauriPath = {
    nondet path = Set("automatic", "ask_first").oneOf()
    all {
    s.alive and s.platform == "tauri" and s.screen == "first_run" and s.first == "path" and s.req == "none",
    s' = { ...s, path: path, first: if(s.scan) "privacy" else "inference" }
    } }
  action YouDecideScopesLaterInTauri = all {
    s.alive and s.platform == "tauri" and s.screen == "first_run" and s.first == "consent" and s.req == "none",
    s' = { ...s, scopes: false, path: "ask_first", scrub: false, witness: false,
      first: if(s.scan) "privacy" else "inference" } }
  action YouGoBackInTauri = all {
    s.alive and s.platform == "tauri" and s.screen == "first_run" and s.req == "none",
    s.first != "welcome" and s.first != "done",
    s' = { ...s, scrub: false, witness: false,
      first: if(s.first == "roots") "welcome" else if(s.first == "connect") "roots"
        else if(s.first == "consent") "connect" else if(s.first == "path") "consent"
        else if(s.first == "privacy") (if(s.scopes) "path" else "consent")
        else if(s.first == "inference") (if(s.scan) "privacy" else if(s.scopes and s.path != "none") "path" else "consent")
        else if(s.first == "disclosure_scrub") "inference"
        else if(s.first == "disclosure_witness") "disclosure_scrub"
        else if(s.first == "grant") "disclosure_witness" else "inference" } }
  action YouRequestTheTauriGrant = all {
    s.alive and s.platform == "tauri" and s.screen == "first_run" and s.first == "grant"
      and s.enrolled and s.scopes and s.path == "automatic" and s.scrub and s.witness and s.req == "none",
    s' = { ...s, req: "grant" } }
  action TheGrantAnswers = {
    nondet granted = Set(false, true).oneOf()
    all {
    s.alive and s.req == "grant",
    s' = { ...s, req: "none", grant: granted, first: if(granted) "projects" else s.first,
      answer: if(granted) "granted" else "grant-refused" }
    } }
  action YouCloseNativeSetup = all {
    s.alive and s.platform != "tauri" and s.screen == "first_run",
    s' = { ...s, screen: "workspace" } }
  action YouOpenTauriAutomaticConsent = all {
    s.alive and s.platform == "tauri" and s.enrolled and s.screen == "workspace" and s.page == "settings",
    s' = { ...s, screen: "first_run", first: if(s.platform == "tauri" and s.enrolled) "consent" else "welcome", scrub: false, witness: false } }
  // Navigation does not cancel requests: callbacks/events can arrive on any destination.
  action YouNavigate = {
    nondet page = Set("waiting", "history", "private_ai", "insights", "mission_drafts", "settings", "compute", "profile", "unknown").oneOf()
    all {
    s.alive and Set("workspace", "offline", "not_found", "refused").contains(s.screen) and s.visible and not(s.quit) and not(s.changeOpen),
    (s.platform == "tauri" or not(Set("compute", "profile", "unknown").contains(page)))
      and (s.missions or page != "mission_drafts" or s.platform == "tauri") and page != s.page,
    s' = { ...s, page: if(page == "mission_drafts" and not(s.missions)) "insights" else page,
      screen: if(page == "unknown") "not_found" else "workspace" }
    } }
  action TheMissionSettingChanges = all {
    s.alive and s.platform != "gtk",
    s' = { ...s, missions: not(s.missions),
      page: if(s.platform == "tauri" and s.missions and s.page == "mission_drafts") "insights" else s.page } }
  action YouSaveSettings = all {
    s.alive and s.screen == "workspace" and s.page == "settings" and s.req == "none",
    s' = { ...s, req: "settings", revision: 1 - s.revision } }
  action SettingsSavingAnswers = {
    nondet saved = Set(false, true).oneOf()
    all {
    s.alive and s.req == "settings",
    s' = { ...s, req: "none", savedRevision: if(saved) s.revision else s.savedRevision,
      answer: if(saved) "settings-saved" else "settings-failed" }
    } }
  action TheSubscriptionNeedsResync = all {
    s.alive and s.subscription == "live",
    s' = { ...s, subscription: "resync", answer: "refresh-required" } }
  action TheSnapshotArrives = all {
    s.alive and s.subscription == "resync",
    s' = { ...s, subscription: "live", answer: "refreshed" } }
  action TheExternalWatcherDisappears = all {
    s.alive and s.owner == "attached" and s.external,
    s' = { ...s, external: false, subscription: "closed", answer: "daemon-not-running" } }
  action TauriRecoversItsConnection = {
    nondet external = Set(false, true).oneOf()
    all {
    s.alive and s.platform == "tauri" and s.subscription == "closed" and s.roots,
    s' = { ...s, external: external, owner: if(external) "attached" else "hosted", subscription: "resync",
      req: if(s.owner == "none") "status" else s.req, screen: if(s.owner == "none") "checking" else s.screen }
    } }
  action YouRetryTheRead = all {
    s.alive and s.screen == "offline" and s.req == "none" and s.owner != "none",
    s' = { ...s, req: "status", screen: "checking" } }
  // Tauri withdrawal uses account_auth, not daemon enrollment or native_wallet_flow.
  action YouSignInForTauriWithdrawal = all {
    s.alive and s.platform == "tauri" and s.page == "history" and s.enrolled
      and not(Set("opening", "browser", "verifying").contains(s.auth)),
    s' = { ...s, auth: "opening", authGeneration: s.generation, fallback: false, fallbackUsed: false } }
  action TheBrowserUrlArrives = all {
    s.alive and s.auth == "opening",
    s' = { ...s, auth: "browser", fallback: true } }
  action YouReopenTheAuthorizedBrowserUrl = all {
    s.alive and s.auth == "browser" and s.fallback and not(s.fallbackUsed),
    s' = { ...s, fallbackUsed: true, answer: "browser-reopened" } }
  action TheAccountCallbackAnswers = {
    nondet valid = Set(false, true).oneOf()
    all {
    s.alive and s.auth == "browser",
    s' = { ...s, auth: if(valid) "verifying" else "failed", fallback: false,
      session: s.session or valid, answer: if(valid) "callback-accepted" else "callback-failed" }
    } }
  action TheAccountSessionIsReread = {
    nondet active = Set(false, true).oneOf()
    all {
    s.alive and s.auth == "verifying",
    s' = { ...s, auth: if(active) "complete" else "failed", session: active,
      answer: if(active) "session-active" else "session-unverified" }
    } }
  action YouConfirmSetUpAgain = all {
    s.alive and s.screen == "workspace" and s.page == "settings" and s.enrolled and s.req == "none",
    s' = { ...s, req: "unenroll" } }
  action SetUpAgainAnswers = {
    nondet removed = Set(false, true).oneOf()
    all {
      s.alive and s.req == "unenroll",
      s' = { ...s, req: "none", enrolled: s.enrolled and not(removed), setup: s.setup and not(removed),
        screen: if(removed) "first_run" else s.screen, first: if(removed) "welcome" else s.first,
        answer: if(removed) "signed-out" else "sign-out-refused" }
    } }
  // Native F10 dialogs: Windows discards old generations; GTK reread has no such guard.
  action YouBeginNativeAccountChange = all {
    s.alive and s.platform != "tauri" and s.screen == "workspace" and s.page == "settings" and not(s.changeOpen) and s.req == "none",
    s' = { ...s, changeOpen: true, change: "current", generation: 1 - s.generation } }
  action YouStartTheChangeLeg = all {
    s.alive and s.changeOpen and Set("current", "new", "confirm").contains(s.change) and s.req == "none",
    s' = { ...s, req: "change", authGeneration: s.generation, changeSent: s.change } }
  action TheChangeLegAnswers = {
    nondet accepted = Set(false, true).oneOf()
    all {
      s.alive and s.req == "change",
      s' = { ...s, req: "none", change: if(s.platform == "windows" and s.authGeneration != s.generation) s.change
        else if(not(accepted)) "failed" else if(s.changeSent == "current") "new" else if(s.changeSent == "new") "confirm" else "done",
        answer: if(accepted) "change-reply" else "change-refused" }
    } }
  action YouCloseAccountChange = all {
    s.alive and s.changeOpen,
    s' = { ...s, changeOpen: false, change: "none", generation: 1 - s.generation } }
  action YouCheckPlatformUpdates = all {
    s.alive and s.page == "settings" and s.update == "idle" and s.platform != "gtk",
    s' = { ...s, update: if(s.platform == "tauri") "guidance" else if(not(s.installed)) "unmanaged" else "checking" } }
  action TheGtkPortalReportsItsState = {
    nondet result = Set("available", "unavailable").oneOf()
    all {
      s.alive and s.platform == "gtk" and s.owner != "none" and s.update == "idle",
      s' = { ...s, update: if(s.installed) result else "unmanaged" }
    } }
  action ThePlatformUpdateCheckAnswers = {
    nondet result = Set("available", "idle", "unavailable").oneOf()
    all {
    s.alive and s.update == "checking",
    s' = { ...s, update: result }
    } }
  action YouApplyThePlatformUpdate = all {
    s.alive and s.update == "available",
    s' = { ...s, update: if(s.platform == "windows") "quiescing" else "installing" } }
  action WindowsQuiesceAnswers = {
    nondet drained = Set(false, true).oneOf()
    all {
    s.alive and s.update == "quiescing",
    s' = { ...s, update: if(drained) "handoff" else "refused", owner: if(drained) "none" else s.owner,
      subscription: if(drained) "closed" else s.subscription }
    } }
  action TheInstallerAnswers = {
    nondet accepted = Set(false, true).oneOf()
    all {
    s.alive and Set("installing", "handoff").contains(s.update),
    s' = { ...s, update: if(accepted) "ready" else "failed",
      alive: not(accepted and s.platform == "windows"),
      screen: if(accepted and s.platform == "windows") "stopped" else s.screen }
    } }
  action YouCloseTheWindow = all {
    s.alive and s.visible and not(s.quit),
    s' = { ...s, visible: not(s.platform == "windows" and s.tray),
      alive: s.screen != "local", screen: if(s.screen == "local") "stopped" else s.screen,
      quit: s.screen != "local" and not(s.platform == "windows" and s.tray) } }
  action YouRaiseTheTrayWindow = all {
    s.alive and not(s.visible), s' = { ...s, visible: true } }
  action YouRequestQuit = all {
    s.alive and not(s.quit), s' = { ...s, quit: true } }
  action YouCancelQuit = all {
    s.alive and s.quit, s' = { ...s, quit: false } }
  action YouConfirmQuit = {
    nondet stopExternal = Set(false, true).oneOf()
    all {
    s.alive and s.quit,
    not(stopExternal) or (s.platform == "gtk" and s.owner == "attached"),
    s' = { ...s, alive: false, visible: false, quit: false, screen: "stopped",
      owner: "none", external: s.external and not(stopExternal), subscription: "closed",
      generation: 1 - s.generation }
    } }
  action YouLaunchAgain = all {
    not(s.alive), s' = { ...s, alive: true, visible: true, screen: if(s.platform == "gtk") "local" else "starting",
      req: "none", auth: "none", quit: false, update: "idle", answer: "none" } }
  action step = any {
    YouOpenTheGtkContributor, TheExistingWatcherAttaches, TheShellHostsTheWatcher,
    TheStartNeedsSourceDeclarations, YouDeclareTheSources, WindowsFindsAnotherOwner,
    TheStartFails, TheStatusRequiresSetup, TheStatusRestoresCompletedSetup, TheReadFails,
    YouContinueFirstRun, YouFinishNativeWatching, TheNativeCompletionReadAnswers, YouEnrollFromConnect, EnrollmentAnswers, YouChooseAndSaveScopes,
    ScopeSavingAnswers, YouChooseTheTauriPath, YouDecideScopesLaterInTauri, YouGoBackInTauri,
    YouRequestTheTauriGrant, TheGrantAnswers, YouCloseNativeSetup, YouOpenTauriAutomaticConsent,
    YouNavigate, TheMissionSettingChanges, YouSaveSettings, SettingsSavingAnswers, TheSubscriptionNeedsResync,
    TheSnapshotArrives, TheExternalWatcherDisappears, TauriRecoversItsConnection, YouRetryTheRead,
    YouSignInForTauriWithdrawal, TheBrowserUrlArrives, YouReopenTheAuthorizedBrowserUrl,
    TheAccountCallbackAnswers, TheAccountSessionIsReread, YouConfirmSetUpAgain, SetUpAgainAnswers,
    YouBeginNativeAccountChange, YouStartTheChangeLeg, TheChangeLegAnswers, YouCloseAccountChange,
    YouCheckPlatformUpdates, TheGtkPortalReportsItsState, ThePlatformUpdateCheckAnswers, YouApplyThePlatformUpdate,
    WindowsQuiesceAnswers, TheInstallerAnswers, YouCloseTheWindow, YouRaiseTheTrayWindow,
    YouRequestQuit, YouCancelQuit, YouConfirmQuit, YouLaunchAgain
  }
}
