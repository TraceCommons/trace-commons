// Current source projection at 8218cc8; promises are pending owner selection.
module client_providers {
  type Provider = Claude | Codex
  type Account = Personal | Work
  type Session = FirstSession | SecondSession
  type Generation = G0 | G1 | G2 | G3 | G4
  type Saved = {identity: Generation, present: bool, provider: Provider, connection: str, auth: str, secret: bool, renamed: bool}
  type Prefs = {model: str, background: bool, collect: bool}
  type Run = {credentialCaptured: bool, settings: str, prefs: Prefs, folder: str, keepSaved: bool, terminal: str, routeStarting: bool, phase: str, account: Account, provider: Provider, connection: str, purpose: str, ticket: bool, redeemed: bool, stop: bool, route: bool}
  val providers = Set(Claude, Codex)
  val accounts = Set(Personal, Work)
  val sessions = Set(FirstSession, SecondSession)
  val generations = Set(G0, G1, G2, G3, G4)
  var s: {
    screen: str, accounts: Account -> Saved, sessions: Session -> Run,
    selected: Account, provider: Provider, connection: str,
    generation: Provider -> Generation, shownGeneration: Provider -> Generation,
    defaults: Provider -> {present: bool, account: Account},
    signedIn: bool, keyGeneration: Generation, operation: str, operationAccount: Account,
    reply: str, error: str, prepare: str, prepareAccount: Account, prepareProvider: Provider,
    preparePrefs: Prefs, prepareFolder: str, prepareKeepSaved: bool, folderStored: Provider -> (str -> Prefs),
    prepareConnection: str, prepareIdentity: Generation, prepareGeneration: Generation, prepareKey: Generation,
    folder: str, folderToken: Generation, folderReplyToken: Generation, folderRead: str,
    catalogue: str, catalogueRequested: bool, catalogueHasTool: bool, catalogueTool: Provider, catalogueToken: Generation, catalogueReplyToken: Generation, catalogueRead: bool,
    model: str, background: bool, collect: bool, keepSavedModel: bool, favorite: bool,
    optionsDraft: bool, optionsSent: bool, optionsStored: bool, optionsAcknowledged: bool,
    usageEnabled: Provider -> bool, usageGeneration: Generation, usageClaim: Generation,
    usageAccount: Account, usageNative: Generation, usageClaimNative: Generation, usage: str, usageRead: bool,
    listener: bool, harness: str, harnessAction: str, harnessOperation: str,
    plan: str, fileGeneration: Generation, planGeneration: Generation
  }
  val vars = s
  def held(a: Account): bool = sessions.exists(i => s.sessions.get(i).account == a and s.sessions.get(i).connection != "near_ai_signin" and Set("starting", "running", "unknown").contains(s.sessions.get(i).phase))
  def panel: bool = s.screen != "providers_closed"
  pure def controls(frame: str): Set[str] =
    if (frame == "providers_home") Set("Add account", "Launch", "Check sign-in", "Sign in", "Rename account", "Make default", "Remove", "Refresh", "Inspect runtime", "Select saved account", "Select NEAR AI sign-in", "Sign out", "Close")
    else if (frame == "providers_add") Set("Tool", "Sign-in type", "Name", "Sign in", "Save", "Cancel", "Close")
    else if (frame == "providers_launch") Set("Account", "Model", "Background model", "Pick a folder as context", "Use home", "Collect this session", "Add to favorites", "Remove from favorites", "Launch", "Retry", "Cancel", "Close")
    else if (frame == "providers_remove") Set("Remove", "Cancel", "Close")
    else if (frame == "providers_plan") Set("Apply", "Cancel", "Close")
    else if (frame == "providers_exposure") Set("Turn on", "Not now", "Close")
    else if (frame == "providers_runtime") Set("Accounts", "Select saved account", "Select NEAR AI sign-in", "Sign out", "Edit options", "Save options", "Usage permission", "Connect", "Disconnect", "Refresh usage", "Stop", "Dismiss", "Close")
    else if (frame == "providers_closed") Set("Open") else Set()

  action init = s' = {
    screen: "providers_home",
    accounts: accounts.mapBy(a => {identity: G0, present: false, provider: Claude, connection: "subscription", auth: "sign_in_required", secret: false, renamed: false}),
    sessions: sessions.mapBy(i => {credentialCaptured: false, settings: "saved", prefs: {model: "default", background: false, collect: false}, folder: "home", keepSaved: false, terminal: "idle", routeStarting: false, phase: "absent", account: Personal, provider: Claude, connection: "subscription", purpose: "coding", ticket: false, redeemed: false, stop: false, route: false}),
    selected: Personal, provider: Claude, connection: "subscription",
    generation: providers.mapBy(p => G0), shownGeneration: providers.mapBy(p => G0),
    defaults: providers.mapBy(p => {present: false, account: Personal}),
    signedIn: false, keyGeneration: G0, operation: "none", operationAccount: Personal,
    reply: "none", error: "none", prepare: "none", prepareAccount: Personal, prepareProvider: Claude,
    preparePrefs: {model: "default", background: false, collect: false}, prepareFolder: "home", prepareKeepSaved: false,
    folderStored: providers.mapBy(p => Map("home" -> {model: "default", background: false, collect: false}, "project" -> {model: "default", background: false, collect: false})),
    prepareConnection: "subscription", prepareIdentity: G0, prepareGeneration: G0, prepareKey: G0,
    folder: "home", folderToken: G0, folderReplyToken: G0, folderRead: "idle",
    catalogue: "unread", catalogueRequested: false, catalogueHasTool: false, catalogueTool: Claude, catalogueToken: G0, catalogueReplyToken: G0, catalogueRead: false,
    model: "default", background: false, collect: false, keepSavedModel: false, favorite: false,
    optionsDraft: false, optionsSent: false, optionsStored: false, optionsAcknowledged: false,
    usageEnabled: providers.mapBy(p => false), usageGeneration: G0, usageClaim: G0,
    usageAccount: Personal, usageNative: G0, usageClaimNative: G0, usage: "not_checked", usageRead: false,
    listener: false, harness: "disconnected", harnessAction: "connect", harnessOperation: "none",
    plan: "none", fileGeneration: G0, planGeneration: G0
  }
  // Native navigation/window lifetime does not stop daemon operations.
  action YouOpenAddAccount = all { controls(s.screen).contains("Add account") and s.operation == "none", s' = {...s, screen: "providers_add"} }
  action YouOpenLaunch = all { controls(s.screen).contains("Launch") and s.screen == "providers_home" and s.operation == "none", s' = {...s, screen: "providers_launch", folderRead: "idle", catalogueRequested: false} }
  action YouInspectRuntime = all { controls(s.screen).contains("Inspect runtime"), s' = {...s, screen: "providers_runtime"} }
  action YouReturnToAccounts = all { controls(s.screen).contains("Accounts"), s' = {...s, screen: "providers_home"} }
  action YouCloseWindow = all { controls(s.screen).contains("Close"), s' = {...s, screen: "providers_closed"} }
  action YouOpenWindow = all { controls(s.screen).contains("Open"), s' = {...s, screen: "providers_home"} }
  action YouCancelSheet = all { controls(s.screen).contains("Cancel") and s.harnessOperation == "none", s' = {...s, screen: "providers_home", plan: "none"} }
  action YouChooseSavedAccount = { nondet a = accounts.oneOf(); all {
    controls(s.screen).contains("Select saved account") and s.accounts.get(a).present and (a != s.selected or s.connection == "near_ai_signin"),
    s' = {...s, selected: a, provider: s.accounts.get(a).provider, connection: s.accounts.get(a).connection, folderRead: "idle"}
  } }
  action YouChooseDerivedSignIn = { nondet p = providers.oneOf(); all {
    controls(s.screen).contains("Select NEAR AI sign-in") and s.signedIn and s.connection != "near_ai_signin",
    s' = {...s, provider: p, connection: "near_ai_signin", folderRead: "idle"}
  } }
  // Account add commits metadata first. The app then sends set_key or reconnect.
  action YouSaveNewAccount = { nondet a = accounts.oneOf(); nondet p = providers.oneOf(); nondet c = Set("subscription", "api_key", "near_ai").oneOf(); nondet g = generations.exclude(Set(s.accounts.get(a).identity, s.prepareIdentity, s.usageGeneration, s.usageClaim)).oneOf(); all {
    controls(s.screen).contains(if(c == "subscription") "Sign in" else "Save") and s.screen == "providers_add" and s.operation == "none" and not(s.accounts.get(a).present) and not(held(a)),
    s' = {...s, accounts: s.accounts.set(a, {identity: g, present: true, provider: p, connection: c, auth: "sign_in_required", secret: false, renamed: false}), selected: a, provider: p, connection: c, operationAccount: a, operation: if(c == "subscription") "login" else "key", usageGeneration: g, screen: "providers_home"}
  } }
  action TheCredentialStoreAcceptsTheKey = all {
    s.operation == "key" and s.accounts.get(s.operationAccount).present and not(held(s.operationAccount)),
    s' = {...s, accounts: s.accounts.set(s.operationAccount, {...s.accounts.get(s.operationAccount), secret: true, auth: "ready"}), operation: "callback", reply: "success"}
  }
  action TheCredentialStoreOrNativeOperationFails = all {
    Set("key", "login", "verify").contains(s.operation),
    s' = {...s, operation: "callback", reply: "unavailable"}
  }
  action YouCheckSignIn = all {
    controls(s.screen).contains("Check sign-in") and s.operation == "none" and s.accounts.get(s.selected).present,
    s' = {...s, operation: "verify", operationAccount: s.selected}
  }
  action TheNativeOrKeyCheckAnswers = { nondet auth = Set("ready", "sign_in_required", "unavailable").oneOf(); all {
    s.operation == "verify" and s.accounts.get(s.operationAccount).present and (s.accounts.get(s.operationAccount).connection == "subscription" or s.accounts.get(s.operationAccount).secret),
    s' = {...s, accounts: s.accounts.set(s.operationAccount, {...s.accounts.get(s.operationAccount), auth: if(s.accounts.get(s.operationAccount).connection == "subscription") auth else "ready"}), operation: "callback", reply: "success"}
  } }
  action YouReconnectSubscription = all {
    controls(s.screen).contains("Sign in") and s.screen == "providers_home" and s.operation == "none" and s.accounts.get(s.selected).present and s.accounts.get(s.selected).connection == "subscription" and not(held(s.selected)),
    s' = {...s, operation: "login", operationAccount: s.selected}
  }
  action TheDaemonPreparesNativeLogin = { nondet i = sessions.oneOf(); all {
    s.operation == "login" and s.accounts.get(s.operationAccount).present and not(held(s.operationAccount)) and s.sessions.get(i).phase == "absent" and not(s.sessions.get(i).routeStarting) and not(s.sessions.get(i).route) and s.sessions.get(i).terminal != "held",
    s' = {...s, sessions: s.sessions.set(i, {credentialCaptured: false, settings: "saved", prefs: {model: "default", background: false, collect: false}, folder: "home", keepSaved: false, terminal: "idle", routeStarting: false, phase: "starting", account: s.operationAccount, provider: s.accounts.get(s.operationAccount).provider, connection: "subscription", purpose: "login", ticket: true, redeemed: false, stop: false, route: false}), operation: "callback", reply: "success"}
  } }
  // AppModel.managedAction has no account/window epoch guard on this reply.
  action TheManagedActionCallbackArrives = all { s.operation == "callback", s' = {...s, operation: "none", error: if(s.reply == "success") "none" else s.reply, reply: "none"} }
  action YouRenameAccount = all {
    controls(s.screen).contains("Rename account") and s.operation == "none" and s.accounts.get(s.selected).present,
    s' = {...s, accounts: s.accounts.set(s.selected, {...s.accounts.get(s.selected), renamed: not(s.accounts.get(s.selected).renamed)})}
  }
  action YouAskToRemoveAccount = all { controls(s.screen).contains("Remove") and s.screen == "providers_home" and s.accounts.get(s.selected).present, s' = {...s, screen: "providers_remove"} }
  action YouConfirmAccountRemoval = { nondet g = generations.exclude(Set(s.generation.get(s.provider), s.prepareGeneration, s.usageGeneration, s.usageClaim)).oneOf(); all {
    controls(s.screen).contains("Remove") and s.screen == "providers_remove" and s.operation == "none" and s.accounts.get(s.selected).present and not(held(s.selected)),
    s' = {...s, accounts: s.accounts.set(s.selected, {...s.accounts.get(s.selected), present: false, secret: false}), defaults: s.defaults.set(s.provider, if(s.defaults.get(s.provider).account == s.selected) {...s.defaults.get(s.provider), present: false} else s.defaults.get(s.provider)), generation: s.generation.set(s.provider, if(s.defaults.get(s.provider).present and s.defaults.get(s.provider).account == s.selected) g else s.generation.get(s.provider)), screen: "providers_home", usageGeneration: g, usage: "not_checked"}
  } }
  action TheHeldProfileRefusesRemoval = all { s.screen == "providers_remove" and held(s.selected), s' = {...s, screen: "providers_home", error: "conflict"} }
  action YouMakeAccountDefault = { nondet g = generations.exclude(Set(s.generation.get(s.provider), s.prepareGeneration)).oneOf(); all {
    controls(s.screen).contains("Make default") and s.accounts.get(s.selected).present and s.shownGeneration.get(s.provider) == s.generation.get(s.provider),
    s' = {...s, defaults: s.defaults.set(s.provider, {present: true, account: s.selected}), generation: s.generation.set(s.provider, g)}
  } }
  action AnotherClientChangesDefault = { nondet a = accounts.oneOf(); nondet g = generations.exclude(Set(s.generation.get(s.accounts.get(a).provider), s.prepareGeneration)).oneOf(); all {
    s.accounts.get(a).present,
    s' = {...s, defaults: s.defaults.set(s.accounts.get(a).provider, {present: true, account: a}), generation: s.generation.set(s.accounts.get(a).provider, g)}
  } }
  action TheStaleSelectionIsRefused = all { controls(s.screen).contains("Make default") and s.shownGeneration.get(s.provider) != s.generation.get(s.provider), s' = {...s, error: "conflict", shownGeneration: s.generation} }
  action YouRefreshSnapshot = all { controls(s.screen).contains("Refresh"), s' = {...s, shownGeneration: s.generation, error: "none"} }
  action TheExternalSignInOrOrganizationChanges = { nondet g = generations.exclude(Set(s.keyGeneration, s.prepareKey)).oneOf(); all {
    s' = {...s, signedIn: true, keyGeneration: g}
  } }
  action YouSignOut = { nondet g = generations.exclude(Set(s.keyGeneration, s.prepareKey)).oneOf(); all { controls(s.screen).contains("Sign out") and s.signedIn, s' = {...s, signedIn: false, keyGeneration: g} } }
  action TheSignInCredentialDisappears = { nondet g = generations.exclude(Set(s.keyGeneration, s.prepareKey)).oneOf(); all { s.signedIn, s' = {...s, signedIn: false, keyGeneration: g} } }
  action TheExternalSavedCredentialDisappears = { nondet a = accounts.oneOf(); nondet g = generations.exclude(Set(s.usageNative, s.usageClaimNative)).oneOf(); all {
    s.accounts.get(a).present and s.accounts.get(a).auth == "ready",
    s' = {...s, accounts: s.accounts.set(a, {...s.accounts.get(a), secret: false, auth: "sign_in_required"}), usageNative: g, usage: "not_checked"}
  } }
  // Folder and catalogue callbacks retain separate generations, unlike managedAction.
  action YouChooseFolder = { nondet f = Set("home", "project").oneOf(); nondet g = generations.exclude(Set(s.folderToken, s.folderReplyToken)).oneOf(); all { controls(s.screen).contains(if(f == "home") "Use home" else "Pick a folder as context") and f != s.folder, s' = {...s, folder: f, folderToken: g, folderRead: "idle"} } }
  action YouRetryFolderRead = all { controls(s.screen).contains("Retry") and s.folderRead == "failed", s' = {...s, folderRead: "idle"} }
  action TheLaunchSheetStartsFolderAndCatalogueReads = { nondet g = generations.exclude(Set(s.folderToken, s.folderReplyToken, s.catalogueToken, s.catalogueReplyToken)).oneOf(); val readCatalogue = Set("near_ai", "near_ai_signin").contains(s.connection) and not(s.catalogueRequested) and (not(s.catalogueHasTool) or s.catalogueTool != s.provider); all {
    s.screen == "providers_launch" and s.folderRead == "idle" and not(s.catalogueRead),
    s' = {...s, folderToken: g, folderReplyToken: g, folderRead: "loading", catalogueToken: if(readCatalogue) g else s.catalogueToken, catalogueReplyToken: if(readCatalogue) g else s.catalogueReplyToken, catalogueRead: readCatalogue, catalogueRequested: true, catalogueTool: if(readCatalogue) s.provider else s.catalogueTool}
  } }
  action YouChangeAccountDuringReads = { nondet a = accounts.oneOf(); nondet g = generations.exclude(Set(s.folderToken, s.folderReplyToken, s.catalogueToken, s.catalogueReplyToken)).oneOf(); all {
    controls(s.screen).contains("Account") and s.accounts.get(a).present and a != s.selected,
    s' = {...s, selected: a, provider: s.accounts.get(a).provider, connection: s.accounts.get(a).connection, folderToken: g, catalogueToken: if(s.accounts.get(a).provider == s.provider) s.catalogueToken else g, catalogueRequested: false, folderRead: "idle"}
  } }
  action TheFolderReadAnswers = { nondet outcome = Set("ready", "failed").oneOf(); all {
    s.folderRead == "loading" and s.folderToken == s.folderReplyToken,
    s' = {...s, folderRead: outcome, model: if(outcome == "ready") s.folderStored.get(s.provider).get(s.folder).model else s.model, background: if(outcome == "ready") s.folderStored.get(s.provider).get(s.folder).background else s.background, collect: if(outcome == "ready") s.folderStored.get(s.provider).get(s.folder).collect else s.collect}
  } }
  action TheOldFolderReadIsDiscarded = all { s.folderToken != s.folderReplyToken, s' = {...s, folderReplyToken: s.folderToken} }
  action TheCatalogueReadAnswers = { nondet result = Set("listed", "removed", "offline").oneOf(); all {
    s.catalogueRead, s' = {...s, catalogueRead: false, catalogueHasTool: if(s.catalogueToken == s.catalogueReplyToken) result != "offline" else s.catalogueHasTool, catalogue: if(s.catalogueToken == s.catalogueReplyToken) result else s.catalogue, model: if(s.catalogueToken == s.catalogueReplyToken and result == "removed") "default" else s.model, keepSavedModel: s.catalogueToken == s.catalogueReplyToken and result == "removed", background: if(s.catalogueToken == s.catalogueReplyToken and result == "removed") false else s.background}
  } }
  action YouChooseModelsAndCollection = { nondet model = Set("default", "listed").oneOf(); nondet b = Set(false, true).oneOf(); nondet c = Set(false, true).oneOf(); all {
    controls(s.screen).contains("Model") and controls(s.screen).contains("Collect this session") and s.folderRead == "ready" and (model == "default" or (Set("near_ai", "near_ai_signin").contains(s.connection) and s.catalogue == "listed")),
    s' = {...s, model: model, background: b and s.provider == Claude and Set("near_ai", "near_ai_signin").contains(s.connection), collect: c, keepSavedModel: false}
  } }
  action YouToggleFavorite = all { controls(s.screen).contains(if(s.favorite) "Remove from favorites" else "Add to favorites") and (s.catalogue == "listed" or s.favorite), s' = {...s, favorite: not(s.favorite)} }
  action YouEditCLIOptions = all { controls(s.screen).contains("Edit options"), s' = {...s, optionsDraft: not(s.optionsDraft), optionsAcknowledged: false} }
  action YouSaveCLIOptions = all { controls(s.screen).contains("Save options") and s.operation == "none" and s.optionsDraft != s.optionsStored, s' = {...s, operation: "options", optionsSent: s.optionsDraft, reply: "none"} }
  action TheOptionsWriteIsAcknowledged = all { s.operation == "options", s' = {...s, optionsStored: s.optionsSent, optionsAcknowledged: true, operation: "callback", reply: "success"} }
  action TheOptionsAcknowledgmentIsMissing = all { s.operation == "options", s' = {...s, optionsStored: s.optionsSent, optionsAcknowledged: false, operation: "callback", reply: "unconfirmed"} }
  // Valid version/folder/options/project checks are explicit environment outcomes below.
  action YouRequestManagedLaunch = all {
    controls(s.screen).contains("Launch") and s.screen == "providers_launch" and s.operation == "none" and s.prepare == "none" and s.folderRead == "ready" and (if(s.connection == "near_ai_signin") s.signedIn else s.accounts.get(s.selected).present) and (s.model == "default" or s.catalogue == "listed" or s.catalogue == "offline"),
    s' = {...s, prepare: if(s.connection == "near_ai_signin") "balance" else "ready", prepareAccount: s.selected, prepareProvider: s.provider, preparePrefs: {model: s.model, background: s.background, collect: s.collect}, prepareFolder: s.folder, prepareKeepSaved: s.keepSavedModel, prepareConnection: s.connection, prepareIdentity: s.accounts.get(s.selected).identity, prepareGeneration: s.shownGeneration.get(s.provider), prepareKey: s.keyGeneration, screen: "providers_home"}
  }
  action TheBalanceReadAnswers = { nondet ok = Set(false, true).oneOf(); all {
    s.prepare == "balance", s' = {...s, prepare: if(ok and s.signedIn and s.prepareKey == s.keyGeneration) "ready" else "none", error: if(ok and s.signedIn and s.prepareKey == s.keyGeneration) "none" else "balance_or_sign_in"}
  } }
  action TheDaemonPreparesCodingSession = { nondet i = sessions.oneOf(); all {
    s.prepare == "ready" and s.sessions.get(i).phase == "absent" and not(s.sessions.get(i).routeStarting) and not(s.sessions.get(i).route) and s.sessions.get(i).terminal != "held" and s.prepareGeneration == s.generation.get(s.prepareProvider) and (if(s.prepareConnection == "near_ai_signin") s.signedIn else s.accounts.get(s.prepareAccount).present and s.accounts.get(s.prepareAccount).identity == s.prepareIdentity and s.accounts.get(s.prepareAccount).provider == s.prepareProvider and s.accounts.get(s.prepareAccount).connection == s.prepareConnection) and (s.preparePrefs.model == "default" or Set("near_ai", "near_ai_signin").contains(s.prepareConnection)) and not(sessions.exists(j => s.sessions.get(j).account == s.prepareAccount and s.sessions.get(j).connection == s.prepareConnection and s.sessions.get(j).purpose == "login" and Set("starting", "running", "unknown").contains(s.sessions.get(j).phase))),
    s' = {...s, sessions: s.sessions.set(i, {credentialCaptured: false, settings: "pending", prefs: s.preparePrefs, folder: s.prepareFolder, keepSaved: s.prepareKeepSaved, terminal: "idle", routeStarting: false, phase: "starting", account: s.prepareAccount, provider: s.prepareProvider, connection: s.prepareConnection, purpose: "coding", ticket: true, redeemed: false, stop: false, route: false}), prepare: "none"}
  } }
  action TheLaunchPrecheckRefuses = all { s.prepare == "ready", s' = {...s, prepare: "none", error: "version_folder_options_model_or_conflict"} }
  action ThePreparedFolderSettingsAreSaved = { nondet i = sessions.oneOf(); val r = s.sessions.get(i); val old = s.folderStored.get(r.provider).get(r.folder); all {
    r.phase == "starting" and r.settings == "pending",
    s' = {...s, sessions: s.sessions.set(i, {...r, settings: "saved"}), folderStored: s.folderStored.set(r.provider, s.folderStored.get(r.provider).set(r.folder, if(r.keepSaved) {...r.prefs, model: old.model, background: old.background} else r.prefs))}
  } }
  action TheFolderWriteRollsBackPreparedSession = { nondet i = sessions.oneOf(); all { s.sessions.get(i).settings == "pending" and s.sessions.get(i).ticket, s' = {...s, sessions: s.sessions.set(i, {...s.sessions.get(i), phase: "absent", ticket: false, settings: "saved"}), error: "folder_save_failed"} } }
  action TheTerminalLaunchIsHeld = { nondet i = sessions.oneOf(); all { s.sessions.get(i).ticket and s.sessions.get(i).settings == "saved" and s.sessions.get(i).terminal == "idle", s' = {...s, sessions: s.sessions.set(i, {...s.sessions.get(i), terminal: "held"})} } }
  action TheTerminalOpeningAnswers = { nondet i = sessions.oneOf(); nondet result = Set("opened", "unknown", "idle").oneOf(); all { s.sessions.get(i).terminal == "held", s' = {...s, sessions: s.sessions.set(i, {...s.sessions.get(i), terminal: result})} } }
  action TheLauncherRedeemsTicket = { nondet i = sessions.oneOf(); all {
    s.sessions.get(i).phase == "starting" and s.sessions.get(i).settings == "saved" and s.sessions.get(i).ticket and (s.sessions.get(i).connection != "near_ai_signin" or s.signedIn),
    s' = {...s, sessions: s.sessions.set(i, {...s.sessions.get(i), ticket: false, redeemed: true, credentialCaptured: s.sessions.get(i).connection == "near_ai_signin", routeStarting: Set("near_ai", "near_ai_signin").contains(s.sessions.get(i).connection)})}
  } }
  action TheSessionProxyStarts = { nondet i = sessions.oneOf(); all {
    s.sessions.get(i).routeStarting and s.sessions.get(i).credentialCaptured,
    s' = {...s, sessions: s.sessions.set(i, {...s.sessions.get(i), routeStarting: false, route: true, credentialCaptured: true})}
  } }
  // redeem_async starts with a captured key; its success has no new epoch check.
  action TheDaemonCollectsEndedProxy = { nondet i = sessions.oneOf(); all { s.sessions.get(i).route and Set("absent", "failed", "exited").contains(s.sessions.get(i).phase), s' = {...s, sessions: s.sessions.set(i, {...s.sessions.get(i), route: false})} } }
  action TheSessionProxyCannotStart = { nondet i = sessions.oneOf(); all { s.sessions.get(i).routeStarting, s' = {...s, sessions: s.sessions.set(i, {...s.sessions.get(i), routeStarting: false, phase: if(Set("starting", "running", "unknown").contains(s.sessions.get(i).phase)) "failed" else s.sessions.get(i).phase, route: false})} } }
  action TheLauncherCapturesSavedKey = { nondet i = sessions.oneOf(); all { Set("starting", "unknown").contains(s.sessions.get(i).phase) and s.sessions.get(i).redeemed and Set("api_key", "near_ai").contains(s.sessions.get(i).connection) and not(s.sessions.get(i).credentialCaptured) and s.accounts.get(s.sessions.get(i).account).secret, s' = {...s, sessions: s.sessions.set(i, {...s.sessions.get(i), credentialCaptured: true})} } }
  action TheToolReportsRunning = { nondet i = sessions.oneOf(); all { Set("starting", "unknown").contains(s.sessions.get(i).phase) and s.sessions.get(i).redeemed and not(s.sessions.get(i).routeStarting) and (s.sessions.get(i).connection == "subscription" or s.sessions.get(i).credentialCaptured), s' = {...s, sessions: s.sessions.set(i, {...s.sessions.get(i), phase: "running"})} } }
  action TheSavedCredentialPreventsToolStartup = { nondet i = sessions.oneOf(); all {
    s.sessions.get(i).phase == "starting" and s.sessions.get(i).redeemed and Set("api_key", "near_ai").contains(s.sessions.get(i).connection) and not(s.sessions.get(i).credentialCaptured) and not(s.accounts.get(s.sessions.get(i).account).secret),
    s' = {...s, sessions: s.sessions.set(i, {...s.sessions.get(i), phase: "failed", route: false})}
  } }
  action TheTicketExpires = { nondet i = sessions.oneOf(); all { s.sessions.get(i).ticket, s' = {...s, sessions: s.sessions.set(i, {...s.sessions.get(i), phase: "failed", ticket: false})} } }
  action TheLeaseExpires = { nondet i = sessions.oneOf(); all {
    Set("starting", "running").contains(s.sessions.get(i).phase) and s.sessions.get(i).redeemed,
    s' = {...s, sessions: s.sessions.set(i, {...s.sessions.get(i), phase: if(s.sessions.get(i).connection == "near_ai_signin") "failed" else "unknown", route: if(s.sessions.get(i).connection == "near_ai_signin") false else s.sessions.get(i).route})}
  } }
  action TheLostSignInEndsItsSessions = { nondet i = sessions.oneOf(); all {
    not(s.signedIn) and s.sessions.get(i).connection == "near_ai_signin" and Set("starting", "running", "unknown").contains(s.sessions.get(i).phase),
    s' = {...s, sessions: s.sessions.set(i, {...s.sessions.get(i), phase: "failed", ticket: false, route: false})}
  } }
  action YouRequestStop = { nondet i = sessions.oneOf(); all { controls(s.screen).contains("Stop") and s.sessions.get(i).redeemed and Set("starting", "running", "unknown").contains(s.sessions.get(i).phase) and not(s.sessions.get(i).stop), s' = {...s, sessions: s.sessions.set(i, {...s.sessions.get(i), stop: true})} } }
  action TheToolReportsExit = { nondet i = sessions.oneOf(); all { s.sessions.get(i).redeemed and Set("starting", "running", "unknown").contains(s.sessions.get(i).phase), s' = {...s, sessions: s.sessions.set(i, {...s.sessions.get(i), phase: "exited", route: false})} } }
  action YouDismissEndedSession = { nondet i = sessions.oneOf(); all { controls(s.screen).contains("Dismiss") and Set("exited", "failed").contains(s.sessions.get(i).phase), s' = {...s, sessions: s.sessions.set(i, {...s.sessions.get(i), phase: "absent"})} } }
  // Usage consent/binding invalidates a captured collection claim.
  action YouChangeUsagePermission = { nondet p = providers.oneOf(); nondet g = generations.exclude(Set(s.usageGeneration, s.usageClaim)).oneOf(); all { controls(s.screen).contains("Usage permission"), s' = {...s, usageEnabled: s.usageEnabled.set(p, not(s.usageEnabled.get(p))), usageGeneration: g, usage: "not_checked"} } }
  action YouRefreshSubscriptionUsage = all { controls(s.screen).contains("Refresh usage") and not(s.usageRead) and s.accounts.get(s.selected).present and s.accounts.get(s.selected).connection == "subscription" and s.usageEnabled.get(s.accounts.get(s.selected).provider), s' = {...s, usageRead: true, usageAccount: s.selected, usageClaim: s.usageGeneration, usageClaimNative: s.usageNative} }
  action TheUsageProbeAnswers = { nondet result = Set("ready", "auth_changed", "unavailable").oneOf(); nondet witness = Set(s.usageNative, s.usageClaimNative).oneOf(); all { s.usageRead, s' = {...s, usageRead: false, usage: if(s.usageClaim != s.usageGeneration or not(s.accounts.get(s.usageAccount).present)) "not_checked" else if(witness != s.usageNative) "auth_changed" else result} } }
  // Harness planning writes no config; enabling the listener can precede a failure.
  action YouRequestHarnessChange = { nondet a = Set("connect", "disconnect").oneOf(); all { controls(s.screen).contains(if(a == "connect") "Connect" else "Disconnect") and s.harnessOperation == "none", s' = {...s, harnessAction: a, harnessOperation: if(a == "connect" and not(s.listener)) "exposure" else "planning", screen: if(a == "connect" and not(s.listener)) "providers_exposure" else s.screen} } }
  action YouAcceptListenerExposure = all { controls(s.screen).contains("Turn on") and s.harnessOperation == "exposure", s' = {...s, listener: true, harnessOperation: "planning"} }
  action YouDeclineListenerExposure = all { controls(s.screen).contains("Not now") and s.harnessOperation == "exposure", s' = {...s, harnessOperation: "none", screen: "providers_runtime"} }
  action TheHarnessPlanAnswers = all { s.harnessOperation == "planning", s' = {...s, harnessOperation: "none", plan: "held", planGeneration: s.fileGeneration, screen: if(panel) "providers_plan" else s.screen} }
  action TheHarnessPlanFails = all { s.harnessOperation == "planning", s' = {...s, harnessOperation: "none", error: "unconfirmed"} }
  action TheToolConfigurationChanges = { nondet g = generations.exclude(Set(s.fileGeneration, s.planGeneration)).oneOf(); all { s.plan == "held", s' = {...s, fileGeneration: g} } }
  action TheHarnessPlanExpires = all { s.plan == "held", s' = {...s, plan: "expired"} }
  action YouConfirmHarnessPlan = all { controls(s.screen).contains("Apply") and s.plan != "none" and s.harnessOperation == "none", s' = {...s, harnessOperation: "committing"} }
  action TheHeldPlanCommits = all { s.harnessOperation == "committing" and s.plan == "held" and s.planGeneration == s.fileGeneration, s' = {...s, harness: if(s.harnessAction == "connect") "connected" else "disconnected", plan: "none", harnessOperation: "none", screen: if(panel) "providers_runtime" else s.screen} }
  action TheHarnessCommitIsRefused = all { s.harnessOperation == "committing", s' = {...s, plan: "none", harnessOperation: "none", error: "unconfirmed", screen: if(panel) "providers_runtime" else s.screen} }

  action step = any {
    YouOpenAddAccount, YouOpenLaunch, YouInspectRuntime, YouReturnToAccounts, YouCloseWindow, YouOpenWindow, YouCancelSheet,
    YouChooseSavedAccount, YouChooseDerivedSignIn, YouSaveNewAccount, TheCredentialStoreAcceptsTheKey,
    TheCredentialStoreOrNativeOperationFails, YouCheckSignIn, TheNativeOrKeyCheckAnswers, YouReconnectSubscription,
    TheDaemonPreparesNativeLogin, TheManagedActionCallbackArrives, YouRenameAccount, YouAskToRemoveAccount,
    YouConfirmAccountRemoval, TheHeldProfileRefusesRemoval, YouMakeAccountDefault, TheStaleSelectionIsRefused,
    AnotherClientChangesDefault, YouRefreshSnapshot, TheExternalSignInOrOrganizationChanges, YouSignOut, TheSignInCredentialDisappears, TheExternalSavedCredentialDisappears,
    YouChooseFolder, YouRetryFolderRead, TheLaunchSheetStartsFolderAndCatalogueReads, YouChangeAccountDuringReads, TheFolderReadAnswers, TheOldFolderReadIsDiscarded,
    TheCatalogueReadAnswers, YouChooseModelsAndCollection, YouToggleFavorite, YouEditCLIOptions, YouSaveCLIOptions,
    TheOptionsWriteIsAcknowledged, TheOptionsAcknowledgmentIsMissing, YouRequestManagedLaunch, TheBalanceReadAnswers,
    TheDaemonPreparesCodingSession, TheLaunchPrecheckRefuses, ThePreparedFolderSettingsAreSaved,
    TheFolderWriteRollsBackPreparedSession, TheTerminalLaunchIsHeld, TheTerminalOpeningAnswers, TheLauncherRedeemsTicket,
    TheSessionProxyStarts, TheDaemonCollectsEndedProxy, TheSessionProxyCannotStart, TheLauncherCapturesSavedKey, TheToolReportsRunning,
    TheSavedCredentialPreventsToolStartup, TheTicketExpires, TheLeaseExpires, TheLostSignInEndsItsSessions, YouRequestStop, TheToolReportsExit,
    YouDismissEndedSession, YouChangeUsagePermission, YouRefreshSubscriptionUsage, TheUsageProbeAnswers,
    YouRequestHarnessChange, YouAcceptListenerExposure, YouDeclineListenerExposure, TheHarnessPlanAnswers,
    TheHarnessPlanFails, TheToolConfigurationChanges, TheHarnessPlanExpires, YouConfirmHarnessPlan,
    TheHeldPlanCommits, TheHarnessCommitIsRefused
  }
}
