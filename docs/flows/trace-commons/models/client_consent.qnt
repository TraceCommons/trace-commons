// Source baseline 8218cc8. Current behavior only; no promises selected.
module client_consent {
  type Folder = { mode: str, askUnder: int, byGrant: bool, terms: bool, fromNow: bool, recorded: bool, heldPath: bool, listingCurrent: bool, projectAtGrant: bool }
  type Entry = { state: str, unknown: bool, onDisk: bool, origin: str, scopeOk: bool, inputOk: bool, pin: bool, hold: str, candidate: bool, overrideHeld: bool }
  var s: {
    screen: str, selected: int, displayedSharing: str, sharingRead: str, sourceRoot: int, folders: int -> Folder, entries: int -> Entry,
    grant: bool, grantId: int, generation: int, includeExisting: bool,
    sourceRecorded: bool, sourceJob: str, jobId: int, jobCurrent: bool,
    grantTerms: bool, override: str, overrideTerms: bool, overrideRecorded: bool, overrideJobCurrent: bool,
    scopeChosen: bool, scopeCall: str, inputsChanged: bool,
    request: str, requestInclude: bool, keepAsks: bool, shownWitnessCurrent: bool,
    audit: bool, build: str, buildEntry: int, buildCurrent: bool,
    check: str, checkEntry: int, checkDisposition: str, notice: bool,
    firstRun: str, firstRunAsk: bool
  }
  val vars = s
  pure def controls(frame: str): Set[str] =
    if (frame == "consent_choice") Set("Automatic", "Ask each time")
    else if (frame == "consent_request") Set("Cancel")
    else if (frame == "consent_internal") Set("Select other folder", "Folder Ask", "Folder Never", "Folder Automatic", "Folder from now", "Global Never", "Clear global rule", "Global Ask", "Global Automatic", "Choose scopes", "Submit", "Grant from now", "Include existing and keep Ask", "Include existing for all", "Withdraw grant only", "Ask each time", "Start first run")
    else Set()
  pure def own(f: Folder): str = f.mode
  pure def resolved(f: Folder, e: Entry, o: str): str =
    if (f.mode == "never") "never"
    else if (o == "never") "never"
    else if (o == "ask") "ask"
    else if (e.unknown) "ask"
    else if (o == "auto") "auto" else own(f)
  pure def waiting(e: Entry): Entry = { ...e, state: "pending", origin: "none", scopeOk: false, inputOk: false, pin: false, hold: "none", candidate: false }
  pure def held(e: Entry, why: str): Entry = { ...waiting(e), hold: why }
  pure def restrict(e: Entry, mode: str): Entry =
    if (mode == "never" and e.state == "pending") { ...e, state: "refused", hold: "project_ignored" }
    else if (e.state == "approved" and e.origin == "automatic")
      if (mode == "never") { ...e, state: "refused", hold: "project_ignored" } else waiting(e)
    else e
  pure def blankFolder: Folder = { mode: "ask", askUnder: -1, byGrant: false, terms: true, fromNow: false, recorded: false, heldPath: false, listingCurrent: false, projectAtGrant: false }
  pure def blankEntry(unknown: bool): Entry = { state: "pending", unknown: unknown, onDisk: true, origin: "none", scopeOk: false, inputOk: false, pin: false, hold: "none", candidate: false, overrideHeld: false }
  action init = {
    nondet otherUnknown = Set(false, true).oneOf()
    s' = {
    screen: "consent_choice", selected: 0, displayedSharing: "ask", sharingRead: "none", sourceRoot: 0,
    folders: Map(0 -> blankFolder, 1 -> blankFolder), entries: Map(0 -> blankEntry(false), 1 -> { ...blankEntry(otherUnknown), state: "absent", onDisk: false }),
    grant: false, grantId: 1, generation: 1, includeExisting: false,
    sourceRecorded: false, sourceJob: "none", jobId: 1, jobCurrent: false,
    grantTerms: true, override: "none", overrideTerms: true, overrideRecorded: false, overrideJobCurrent: false,
    scopeChosen: false, scopeCall: "none", inputsChanged: false,
    request: "none", requestInclude: true, keepAsks: true, shownWitnessCurrent: true,
    audit: false, build: "none", buildEntry: 0, buildCurrent: true,
    check: "none", checkEntry: 0, checkDisposition: "none", notice: false,
    firstRun: "none", firstRunAsk: false
    }
  }
  // SharingChoiceCard.swift:16-21,202-249. Picking Automatic opens disclosures.
  action YouOpenAutomaticDisclosures = all {
    controls(s.screen).contains("Automatic"),
    s' = { ...s, screen: "consent_request", request: "disclosure", shownWitnessCurrent: true }
  }
  action YouCancelTheDisclosures = all {
    controls(s.screen).contains("Cancel") and s.request == "disclosure",
    s' = { ...s, screen: "consent_choice", request: "none" }
  }
  // Remaining controls are explicitly internal input selectors, not product UI.
  action YouInspectConsentInternals = all {
    s.screen == "consent_choice", s' = { ...s, screen: "consent_internal" }
  }
  action YouSelectTheOtherFolder = all {
    controls(s.screen).contains("Select other folder"), s' = { ...s, selected: 1 - s.selected }
  }
  // enroll.rs:216-252: policy adoption precedes audit and config replacement.
  action YouChooseConsentScopes = all {
    controls(s.screen).contains("Choose scopes") and s.scopeCall == "none",
    s' = { ...s, scopeCall: "adopt" }
  }
  action TheStandingGrantsAdoptChosenScopes = all {
    s.scopeCall == "adopt", s' = { ...s, scopeCall: "config" }
  }
  action TheScopeChoiceIsWritten = all {
    s.scopeCall == "config",
    s' = { ...s, scopeCall: "none", scopeChosen: true, buildCurrent: false,
      entries: Set(0, 1).mapBy(i => { val e = s.entries.get(i)
        { ...e, scopeOk: false } }) }
  }
  action TheScopeWriteOrAuditFails = all {
    s.scopeCall != "none", s' = { ...s, scopeCall: "none" }
  }
  // ipc.rs:5330-5398. Audit precedes policy save; callback is later.
  action YouConfirmAnAutomaticGrant = {
    nondet include = Set(false, true).oneOf()
    nondet keep = Set(false, true).oneOf()
    all {
      (s.request == "disclosure" or (s.screen == "consent_internal" and s.request == "none")),
      s' = { ...s, request: "validate", screen: "consent_saving", requestInclude: include, keepAsks: keep, audit: false }
    }
  }
  action TheGrantPreconditionsRefuseTheRequest = all {
    s.request == "validate" and (not(s.scopeChosen) or not(s.shownWitnessCurrent)),
    s' = { ...s, request: "refused", screen: "consent_internal" }
  }
  action TheGrantAuditIsRecorded = all {
    s.request == "validate" and s.scopeChosen and s.shownWitnessCurrent,
    s' = { ...s, request: "save", audit: true }
  }
  action TheGrantAuditOrPolicyWriteFails = all {
    s.request == "validate" or s.request == "save",
    s' = { ...s, request: "refused", screen: "consent_internal" }
  }
  // Canonical fresh identities: invalidate every prior equality reference.
  // Include-existing increments generation twice in source; only final equality matters.
  action TheFreshGrantIsSaved = all {
    s.request == "save" and s.audit,
    s' = { ...s, grant: true, grantId: 3 - s.grantId, generation: 3 - s.generation,
      grantTerms: true, includeExisting: s.requestInclude, sourceRecorded: false,
      jobCurrent: false, request: "reply", override: if(s.requestInclude) "none" else s.override,
      folders: Set(0, 1).mapBy(i => { val f = s.folders.get(i)
        { ...f,
        askUnder: if(s.requestInclude and s.keepAsks and f.mode == "ask" and f.askUnder != -1) 3 - s.generation else if(s.requestInclude and not(s.keepAsks) and f.mode == "ask") -1 else if(f.askUnder != -1) -2 else -1,
        projectAtGrant: false,
        fromNow: if(s.requestInclude and f.mode == "auto") false else f.fromNow } }) }
  }
  action TheGrantReplyArrives = all {
    s.request == "reply", s' = { ...s, request: "none", screen: "consent_internal" }
  }
  // ipc.rs:4170,5258-5273; SharingChoiceCard.swift:16-21. Read and UI callback differ.
  action TheSharingChoiceRequestsTheSavedGrant = all {
    s.sharingRead == "none",
    s' = { ...s, sharingRead: if(s.grant and s.includeExisting) "automatic" else "ask" }
  }
  action TheSharingChoiceReceivesTheSavedGrant = all {
    s.sharingRead != "none",
    s' = { ...s, displayedSharing: s.sharingRead, sharingRead: "none" }
  }
  action TheRefusedGrantIsAcknowledged = all {
    s.request == "refused", s' = { ...s, request: "none", firstRun: if(s.firstRun == "grant") "fallback" else s.firstRun }
  }
  // policy.rs:1125-1198. Legacy withdrawal retains grant-armed folders.
  action YouWithdrawTheGrant = {
    nondet returnToAsk = Set(false, true).oneOf()
    all {
      s.screen == "consent_internal" and s.grant,
      s' = { ...s, grant: false, jobCurrent: false, displayedSharing: "ask",
        folders: Set(0, 1).mapBy(i => { val f = s.folders.get(i)
        if(returnToAsk and f.byGrant and f.mode == "auto") blankFolder else f }) }
    }
  }
  // watcher.rs:197-228,523-595: captured identity before listing, then record.
  action TheWatcherStartsAFullSourceListing = all {
    s.sourceJob == "none",
    s' = { ...s, sourceJob: "listing", jobId: s.grantId, jobCurrent: s.grant, overrideJobCurrent: s.override == "auto",
      folders: Set(0, 1).mapBy(i => { val f = s.folders.get(i)
        { ...f, listingCurrent: f.fromNow } }) }
  }
  action TheSourceListingIsRecorded = all {
    s.sourceJob == "listing",
    s' = { ...s, sourceJob: "none", overrideRecorded: s.overrideRecorded or s.overrideJobCurrent,
      sourceRecorded: s.sourceRecorded or (s.grant and s.jobCurrent and s.jobId == s.grantId),
      entries: Set(0, 1).mapBy(i => { val e = s.entries.get(i)
        { ...e, onDisk: e.onDisk or (s.grant and s.jobCurrent and s.jobId == s.grantId and not(s.sourceRecorded) and e.state != "absent"),
          overrideHeld: if(s.overrideJobCurrent and not(s.overrideRecorded)) e.state != "absent" else e.overrideHeld } }),
      folders: Set(0, 1).mapBy(i => { val f = s.folders.get(i)
        { ...f, projectAtGrant: if(s.grant and s.jobCurrent and s.jobId == s.grantId and not(s.sourceRecorded)) s.entries.get(i).state != "absent" else f.projectAtGrant,
          recorded: f.recorded or (f.fromNow and f.listingCurrent), heldPath: if(f.fromNow and f.listingCurrent and not(f.recorded)) s.entries.get(i).state != "absent" else f.heldPath } }) }
  }
  action TheSourceListingFails = all {
    s.sourceJob == "listing", s' = { ...s, sourceJob: "none" }
  }
  // Distinct incoming session in same folder; retains recorded backlog relation.
  action ANewSessionArrives = {
    nondet i = Set(0, 1).oneOf()
    all { s.entries.get(i).state == "absent" and s.folders.get(i).mode != "never" and s.override != "never",
      s' = { ...s, entries: s.entries.set(i, { ...s.entries.get(i), state: "pending", onDisk: false }), folders: s.folders.set(i, { ...s.folders.get(i), heldPath: false, listingCurrent: false, projectAtGrant: false }) } }
  }
  // policy.rs:1271-1352. Unknown bucket is never grant-armed.
  action TheGrantArmsAnEligibleFolder = {
    nondet i = Set(0, 1).oneOf()
    all {
      s.grant and s.sourceRecorded and s.grantTerms and s.entries.get(i).state != "absent" and not(s.entries.get(i).unknown) and s.folders.get(i).mode == "ask" and
      (if(s.includeExisting) s.folders.get(i).askUnder != s.generation else s.folders.get(i).askUnder == -1 and not(s.folders.get(i).projectAtGrant) and not(s.entries.get(i).onDisk)),
      s' = { ...s, folders: s.folders.set(i, { ...s.folders.get(i), mode: "auto", byGrant: true, terms: true }) }
    }
  }
  // ipc.rs:5795-5978: policy-before-queue, only queued unattended retracted.
  action YouChangeTheFolderRule = {
    nondet mode = Set("ask", "never", "auto").oneOf()
    nondet fromNow = Set(false, true).oneOf()
    val i = s.selected
    all {
      s.screen == "consent_internal" and (mode != "auto" or (s.scopeChosen and not(s.entries.get(i).unknown))) and
      not(mode == "auto" and fromNow and s.grant and s.includeExisting),
      s' = { ...s, folders: s.folders.set(i, { ...s.folders.get(i), mode: mode,
        askUnder: if(s.grant) s.generation else -2, byGrant: false, terms: true,
        fromNow: mode == "auto" and fromNow, recorded: if(mode == "auto" and fromNow and s.folders.get(i).fromNow) s.folders.get(i).recorded else false,
        heldPath: if(mode == "auto" and fromNow and s.folders.get(i).fromNow) s.folders.get(i).heldPath else false, listingCurrent: false }),
        entries: s.entries.set(i, if(mode != "auto") restrict(s.entries.get(i), mode) else if(fromNow and not(s.folders.get(i).fromNow)) restrict(s.entries.get(i), "ask") else if(s.entries.get(i).hold == "project_ignored") waiting(s.entries.get(i)) else s.entries.get(i)) }
    }
  }
  // policy.rs:1662-1675; ipc.rs:6072-6270. Manual approvals retained by global Never.
  action YouChangeTheGlobalRule = {
    nondet mode = Set("none", "ask", "never", "auto").oneOf()
    all { s.screen == "consent_internal" and mode != s.override and (mode != "auto" or s.scopeChosen),
      s' = { ...s, override: mode, overrideTerms: true, overrideRecorded: false, overrideJobCurrent: false,
        entries: Set(0, 1).mapBy(i => { val e = s.entries.get(i)
        if(e.state == "approved" and e.origin == "automatic" and resolved(s.folders.get(i), e, mode) != "auto") waiting(e) else e }) } }
  }
  // Settings/root/config changes are world inputs; terms comparisons are symbolic.
  action TheEnvelopeTermsWiden = all {
    not(s.inputsChanged),
    s' = { ...s, inputsChanged: true, grantTerms: false, overrideTerms: false, buildCurrent: false,
      folders: Set(0, 1).mapBy(i => { val f = s.folders.get(i)
        { ...f, terms: false } }), entries: Set(0, 1).mapBy(i => { val e = s.entries.get(i)
        { ...e, inputOk: false } }) }
  }
  // settings.rs:1730-1762; watcher.rs:205-228. A root is a new source identity,
  // not a GrantTerms widening. Its new disk snapshot must be recorded separately.
  action TheSourceRootChanges = all {
    s' = { ...s, sourceRoot: 1 - s.sourceRoot, sourceRecorded: false, jobCurrent: false,
      overrideRecorded: false, overrideJobCurrent: false,
      folders: Set(0, 1).mapBy(i => { val f = s.folders.get(i)
        { ...f, recorded: false, listingCurrent: false } }) }
  }
  // watcher.rs:1157-1237. Void policy before per-row approval drift is processed.
  action TheGrantSweepVoidsChangedTerms = all {
    (s.grant and not(s.grantTerms)) or (s.override == "auto" and not(s.overrideTerms)) or Set(0, 1).exists(i => s.folders.get(i).mode == "auto" and not(s.folders.get(i).terms)),
    s' = { ...s, grant: s.grant and s.grantTerms, jobCurrent: s.jobCurrent and s.grantTerms, notice: true,
      override: if(s.override == "auto" and not(s.overrideTerms)) "none" else s.override,
      folders: Set(0, 1).mapBy(i => { val f = s.folders.get(i)
        if(f.mode == "auto" and not(f.terms)) { ...f, mode: "ask", fromNow: false } else f }) }
  }
  // Source input may change again; validity is equality-to-current, never wrapping revision.
  action TheNextEnvelopeTermsChangeBecomesPossible = all {
    s.inputsChanged and not(s.grant) and Set(0, 1).forall(i => s.folders.get(i).mode != "auto"),
    s' = { ...s, inputsChanged: false }
  }
  action TheWitnessDisclosureChanges = all {
    s.request == "disclosure" or s.request == "validate", s' = { ...s, shownWitnessCurrent: not(s.shownWitnessCurrent) }
  }
  // queue.rs:1757-1825; automatic_gate result is external validated evidence.
  action TheWatcherApprovesAnEligibleEntry = {
    nondet i = Set(0, 1).oneOf()
    all {
      s.scopeChosen and s.entries.get(i).state == "pending" and s.entries.get(i).hold == "none" and resolved(s.folders.get(i), s.entries.get(i), s.override) == "auto" and
      (not(s.folders.get(i).fromNow) or (s.folders.get(i).recorded and not(s.folders.get(i).heldPath))) and
      (s.override != "auto" or s.folders.get(i).mode == "auto" or (s.overrideRecorded and not(s.entries.get(i).overrideHeld))) and
      not(s.folders.get(i).byGrant and s.entries.get(i).onDisk and not(s.grant and s.includeExisting)),
      s' = { ...s, entries: s.entries.set(i, { ...s.entries.get(i), state: "approved", origin: "automatic", scopeOk: true, inputOk: true, pin: true }) }
    }
  }
  // ipc.rs:7286-7440,7867-7947: captured build can finish after settings change.
  action YouSubmitTheSelectedEntry = all {
    s.screen == "consent_internal" and s.override != "never" and s.scopeChosen and s.build == "none" and s.entries.get(s.selected).state == "pending",
    s' = { ...s, build: "building", buildEntry: s.selected, buildCurrent: true }
  }
  action TheManualEnvelopeBuildFinishes = all {
    s.build == "building" and s.entries.get(s.buildEntry).state == "pending" and s.override != "never" and s.folders.get(s.buildEntry).mode != "never",
    s' = { ...s, build: "none", entries: s.entries.set(s.buildEntry, { ...s.entries.get(s.buildEntry), state: "approved", origin: "manual", scopeOk: s.buildCurrent, inputOk: s.buildCurrent, pin: true, hold: "none" }) }
  }
  action TheManualEnvelopeBuildFailsOrIsOvertaken = all {
    s.build == "building", s' = { ...s, build: "none" }
  }
  // mod.rs:1136-1207: snapshot then queue recheck; current manual origin wins.
  action TheUploaderChoosesAPolicyRetraction = {
    nondet i = Set(0, 1).oneOf()
    all { s.check == "none" and s.entries.get(i).state == "approved" and s.entries.get(i).origin == "automatic" and
      (resolved(s.folders.get(i), s.entries.get(i), s.override) != "auto" or (s.folders.get(i).fromNow and (not(s.folders.get(i).recorded) or s.folders.get(i).heldPath))),
      s' = { ...s, check: "queue", checkEntry: i, checkDisposition: if(s.folders.get(i).mode == "never") "refuse" else "return" } }
  }
  action TheQueueRechecksTheApprovalOrigin = all {
    s.check == "queue",
    s' = { ...s, check: "none", entries: s.entries.set(s.checkEntry,
      if(s.entries.get(s.checkEntry).state == "approved" and s.entries.get(s.checkEntry).origin == "automatic")
        if(s.checkDisposition == "refuse") { ...s.entries.get(s.checkEntry), state: "refused", hold: "project_ignored" } else waiting(s.entries.get(s.checkEntry))
      else s.entries.get(s.checkEntry)) }
  }
  // mod.rs:1340-1385; uploader.rs:475-652: global check/claim, then input check.
  action TheUploaderSnapshotsAnEligibleApproval = {
    nondet i = Set(0, 1).oneOf()
    all { s.entries.get(i).state == "approved" and not(s.entries.get(i).candidate) and
      (s.entries.get(i).origin == "manual" or (resolved(s.folders.get(i), s.entries.get(i), s.override) == "auto" and
        (not(s.folders.get(i).fromNow) or (s.folders.get(i).recorded and not(s.folders.get(i).heldPath))) and
        (s.override != "auto" or s.folders.get(i).mode == "auto" or (s.overrideRecorded and not(s.entries.get(i).overrideHeld))) and
        not(s.folders.get(i).byGrant and s.entries.get(i).onDisk and not(s.grant and s.includeExisting)))),
      s' = { ...s, entries: s.entries.set(i, { ...s.entries.get(i), candidate: true }) } }
  }
  action TheUploaderClaimsAnApproval = {
    nondet i = Set(0, 1).oneOf()
    all { s.check == "none" and s.override != "never" and s.scopeChosen and s.entries.get(i).state == "approved" and s.entries.get(i).scopeOk and s.entries.get(i).candidate,
      s' = { ...s, entries: s.entries.set(i, { ...s.entries.get(i), state: "uploading", candidate: false }) } }
  }
  action TheUploaderHoldsChangedInputs = {
    nondet i = Set(0, 1).oneOf()
    all { (s.entries.get(i).state == "approved" and not(s.entries.get(i).scopeOk)) or (s.entries.get(i).state == "uploading" and not(s.entries.get(i).inputOk)),
      s' = { ...s, entries: s.entries.set(i, held(s.entries.get(i), "terms_changed")) } }
  }
  action TheInFlightSendFinishes = {
    nondet i = Set(0, 1).oneOf()
    all { s.entries.get(i).state == "uploading" and s.entries.get(i).inputOk,
      s' = { ...s, entries: s.entries.set(i, { ...s.entries.get(i), state: "uploaded" }) } }
  }
  action ThePassOrRestartReleasesAnInFlightEntry = {
    nondet i = Set(0, 1).oneOf()
    all { s.entries.get(i).state == "uploading", s' = { ...s, entries: s.entries.set(i, { ...s.entries.get(i), state: "approved" }) } }
  }
  // FirstRunPlan.swift:234-281; FirstRunRunner.swift:304-324: scopes, grant, then Ask.
  action YouStartTheFirstRunSharingCommit = all {
    s.firstRun == "none" and s.screen == "consent_internal",
    s' = { ...s, firstRun: "scopes", firstRunAsk: true, scopeCall: "adopt" }
  }
  action TheFirstRunReachesItsGrant = all {
    s.firstRun == "scopes" and s.scopeChosen and s.scopeCall == "none" and s.request == "none",
    s' = { ...s, firstRun: "grant", request: "validate", requestInclude: true, keepAsks: true, audit: false }
  }
  action TheFirstRunFallsBackToAsk = all {
    s.firstRun == "fallback",
    s' = { ...s, firstRun: "complete_refused", grant: false, jobCurrent: false, displayedSharing: "ask",
      folders: Set(0, 1).mapBy(i => { val f = s.folders.get(i)
        if(f.byGrant and f.mode == "auto") blankFolder else f }) }
  }
  action TheFirstRunAppliesItsAskRuleAfterTheGrant = all {
    s.firstRun == "grant" and s.request == "none" and s.grant and s.firstRunAsk,
    s' = { ...s, firstRun: "complete", folders: s.folders.set(0, { ...s.folders.get(0), mode: "ask", askUnder: s.generation, byGrant: false }), entries: s.entries.set(0, restrict(s.entries.get(0), "ask")) }
  }
  action step = any {
    YouOpenAutomaticDisclosures, YouCancelTheDisclosures, YouInspectConsentInternals, YouSelectTheOtherFolder,
    YouChooseConsentScopes, TheStandingGrantsAdoptChosenScopes, TheScopeChoiceIsWritten, TheScopeWriteOrAuditFails,
    YouConfirmAnAutomaticGrant, TheGrantPreconditionsRefuseTheRequest, TheGrantAuditIsRecorded, TheGrantAuditOrPolicyWriteFails,
    TheFreshGrantIsSaved, TheGrantReplyArrives, TheSharingChoiceRequestsTheSavedGrant, TheSharingChoiceReceivesTheSavedGrant, TheRefusedGrantIsAcknowledged, YouWithdrawTheGrant,
    TheWatcherStartsAFullSourceListing, TheSourceListingIsRecorded, TheSourceListingFails, ANewSessionArrives,
    TheGrantArmsAnEligibleFolder, YouChangeTheFolderRule, YouChangeTheGlobalRule, TheEnvelopeTermsWiden, TheSourceRootChanges,
    TheGrantSweepVoidsChangedTerms, TheNextEnvelopeTermsChangeBecomesPossible, TheWitnessDisclosureChanges,
    TheWatcherApprovesAnEligibleEntry, YouSubmitTheSelectedEntry, TheManualEnvelopeBuildFinishes, TheManualEnvelopeBuildFailsOrIsOvertaken,
    TheUploaderChoosesAPolicyRetraction, TheQueueRechecksTheApprovalOrigin, TheUploaderSnapshotsAnEligibleApproval, TheUploaderClaimsAnApproval,
    TheUploaderHoldsChangedInputs, TheInFlightSendFinishes, ThePassOrRestartReleasesAnInFlightEntry,
    YouStartTheFirstRunSharingCommit, TheFirstRunReachesItsGrant, TheFirstRunFallsBackToAsk, TheFirstRunAppliesItsAskRuleAfterTheGrant
  }
}
