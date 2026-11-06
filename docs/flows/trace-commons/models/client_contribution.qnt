// Current behavior projection at 8218cc8; no product promises selected.
module client_contribution {
  var s: {
    screen: str,
    entries: int -> { state: str, sourceRev: int, offeredRev: int, pin: int, pinnedRev: int, artifact: bool, approvedScopes: int, approvedInputs: int, unattended: bool, hold: str, undo: bool, retry: bool, attempts: int, submission: int, receipt: str, account: int, history: str, hidden: bool, withdrawal: str },
    selected: int,
    uploadEntry: int,
    previewEntry: int,
    preview: str,
    upload: str,
    enrolled: bool,
    account: int,
    sourceEnabled: bool,
    sourceAccessible: bool,
    projectMode: str,
    globalHold: bool,
    grant: bool,
    scopes: int,
    inputs: int,
    scrub: str,
    healthy: bool,
    capAvailable: bool,
    network: bool,
    backoff: str,
    withdrawalEntry: int,
    withdrawalCall: str,
    historyRead: str
  }
  val vars = s

  pure def controls(frame: str): Set[str] =
    if (frame == "contribution_empty") Set("Choose other trace", "Policy", "History")
    else     if (frame == "contribution_pending") Set("Review", "Submit", "Keep", "Not this one", "Choose other trace", "Policy", "History")
    else     if (frame == "contribution_preview_busy") Set("Close preview", "Choose other trace", "Policy", "History")
    else     if (frame == "contribution_preview") Set("Submit", "Close preview", "Keep", "Choose other trace", "Policy", "History")
    else     if (frame == "contribution_approved") Set("Undo", "Choose other trace", "Policy", "History")
    else     if (frame == "contribution_uploading") Set("Choose other trace", "Policy", "History")
    else     if (frame == "contribution_held") Set("Review", "Submit", "Keep", "Not this one", "Choose other trace", "Policy", "History")
    else     if (frame == "contribution_offline") Set("Undo", "Choose other trace", "Policy", "History")
    else     if (frame == "contribution_failed") Set("Retry", "Choose other trace", "Policy", "History")
    else     if (frame == "contribution_refused") Set("Choose other trace", "Policy", "History")
    else     if (frame == "contribution_kept") Set("Undo", "Choose other trace", "Policy", "History")
    else     if (frame == "contribution_expired") Set("Choose other trace", "Policy", "History")
    else     if (frame == "contribution_superseded") Set("Choose other trace", "Policy", "History")
    else     if (frame == "contribution_uploaded") Set("Choose other trace", "Policy", "History")
    else     if (frame == "contribution_policy") Set("Ask", "Automatic", "Never", "Manual Scrub check", "Change scopes", "Change inputs", "Disable source", "Enable source", "Hold all sends", "Resume all sends", "Back to trace", "Choose other trace", "Policy", "History")
    else     if (frame == "contribution_history") Set("Withdraw", "Refresh history", "Back to trace", "Choose other trace", "Policy", "History")
    else Set()

  action init = s' = {
    screen: "contribution_empty",
    entries: Map(0 -> {state: "absent", sourceRev: 0, offeredRev: 0, pin: 0, pinnedRev: 0, artifact: false, approvedScopes: -1, approvedInputs: -1, unattended: false, hold: "none", undo: false, retry: false, attempts: 0, submission: 0, receipt: "none", account: 1, history: "none", hidden: false, withdrawal: "none"}, 1 -> {state: "absent", sourceRev: 0, offeredRev: 0, pin: 0, pinnedRev: 0, artifact: false, approvedScopes: -1, approvedInputs: -1, unattended: false, hold: "none", undo: false, retry: false, attempts: 0, submission: 0, receipt: "none", account: 1, history: "none", hidden: false, withdrawal: "none"}),
    selected: 0,
    uploadEntry: 0,
    previewEntry: 0,
    preview: "none",
    upload: "none",
    enrolled: true,
    account: 1,
    sourceEnabled: true,
    sourceAccessible: true,
    projectMode: "ask",
    globalHold: false,
    grant: false,
    scopes: 0,
    inputs: 0,
    scrub: "automatic",
    healthy: true,
    capAvailable: true,
    network: true,
    backoff: "none",
    withdrawalEntry: 0,
    withdrawalCall: "none",
    historyRead: "unread"
  }

  // crates/trace-commons-contributor/src/daemon/watcher.rs:1576-1778,2068-2123
  action TheWatcherDiscoversATrace = {
    nondet daemonEntry = Set(0, 1).oneOf()
    all {
      s.entries.get(daemonEntry).state == "absent" and s.sourceEnabled and s.sourceAccessible and s.projectMode != "never" and not(s.globalHold),
      s' = { ...s, entries: s.entries.set(daemonEntry, { ...s.entries.get(daemonEntry), state: "pending", offeredRev: s.entries.get(daemonEntry).sourceRev, account: s.account }), screen: if(s.selected == daemonEntry) "contribution_pending" else s.screen }
    }
  }

  // crates/trace-commons-contributor/src/daemon/watcher.rs:130-178 [source discovery failure abstracted]
  action TheSourcePermissionIsLost = all {
    s.sourceAccessible,
    s' = { ...s, sourceAccessible: false }
  }

  // crates/trace-commons-contributor/src/daemon/watcher.rs:130-178
  action TheSourcePermissionReturns = all {
    not(s.sourceAccessible),
    s' = { ...s, sourceAccessible: true }
  }

  // crates/trace-commons-contributor/src/daemon/uploader.rs:527-555; watcher.rs:953-1091
  action TheSourceSessionChanges = {
    nondet daemonEntry = Set(0, 1).oneOf()
    all {
      s.entries.get(daemonEntry).sourceRev == 0 and s.entries.get(daemonEntry).state != "absent",
      s' = { ...s, entries: s.entries.set(daemonEntry, { ...s.entries.get(daemonEntry), sourceRev: 1 }) }
    }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:1212-1214 [two trace UI selection]
  action YouSelectTheOtherTrace = all {
    controls(s.screen).contains("Choose other trace") and (true),
    s' = { ...s, selected: 1 - s.selected, screen: "contribution_empty" }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:33-66,752-769 [render projection only]
  action TheSelectedTraceRowIsRendered = all {
    s.screen == "contribution_empty" and s.entries.get(s.selected).state != "absent",
    s' = { ...s, screen: if(s.entries.get(s.selected).hold != "none" and s.entries.get(s.selected).state == "pending") "contribution_held" else if(s.entries.get(s.selected).state == "pending") "contribution_pending" else if(s.entries.get(s.selected).state == "approved") "contribution_approved" else if(s.entries.get(s.selected).state == "uploading") "contribution_uploading" else if(s.entries.get(s.selected).state == "uploaded") "contribution_uploaded" else if(s.entries.get(s.selected).state == "failed") "contribution_failed" else if(s.entries.get(s.selected).state == "expired") "contribution_expired" else if(s.entries.get(s.selected).state == "superseded") "contribution_superseded" else if(s.entries.get(s.selected).hold == "kept") "contribution_kept" else "contribution_refused" }
  }

  // crates/trace-commons-contributor/src/daemon/ipc.rs:8342-8459
  action YouOpenThePreview = all {
    controls(s.screen).contains("Review") and (s.entries.get(s.selected).state == "pending" and s.preview == "none"),
    s' = { ...s, previewEntry: s.selected, preview: "building", screen: "contribution_preview_busy" }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:1860-1882; ipc.rs:8342-8459
  action ThePreviewPinsAnEnvelope = all {
    s.preview == "building" and s.entries.get(s.previewEntry).state == "pending",
    s' = { ...s, entries: s.entries.set(s.previewEntry, { ...s.entries.get(s.previewEntry), pin: s.previewEntry + 1, pinnedRev: s.entries.get(s.previewEntry).sourceRev, artifact: true }), preview: "ready", screen: if(s.selected == s.previewEntry) "contribution_preview" else s.screen }
  }

  // crates/trace-commons-contributor/src/daemon/ipc.rs:8342-8459 [privacy/canary/unavailable reasons coarse]
  action ThePreviewFails = all {
    s.preview == "building",
    s' = { ...s, preview: "none", screen: if(s.selected == s.previewEntry) "contribution_pending" else s.screen, healthy: false }
  }

  // crates/trace-commons-contributor/src/daemon/ipc.rs:8510-8590 [visibility/cancellation, pin retained]
  action YouCloseThePreview = all {
    controls(s.screen).contains("Close preview") and (s.preview != "none"),
    s' = { ...s, preview: "none", screen: "contribution_pending" }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:1936-1953 [age/pressure abstraction]
  action TheUnreferencedPreviewPinIsReleased = {
    nondet daemonEntry = Set(0, 1).oneOf()
    all {
      s.entries.get(daemonEntry).state == "pending" and s.entries.get(daemonEntry).pin != 0 and s.preview == "none",
      s' = { ...s, entries: s.entries.set(daemonEntry, { ...s.entries.get(daemonEntry), pin: 0, artifact: false }) }
    }
  }

  // crates/trace-commons-contributor/src/daemon/ipc.rs:7286-7440
  action YouSubmitWithoutOpeningPreview = all {
    controls(s.screen).contains("Submit") and (s.entries.get(s.selected).state == "pending" and s.enrolled and not(s.globalHold) and s.preview == "none"),
    s' = { ...s, previewEntry: s.selected, preview: "approve_build", screen: "contribution_preview_busy" }
  }

  // crates/trace-commons-contributor/src/daemon/ipc.rs:7286-7440; queue.rs:1757-1795
  action TheApprovalBuildPinsAndApproves = all {
    s.preview == "approve_build" and s.entries.get(s.previewEntry).state == "pending" and s.enrolled and not(s.globalHold),
    s' = { ...s, entries: s.entries.set(s.previewEntry, { ...s.entries.get(s.previewEntry), state: "approved", pin: s.previewEntry + 1, artifact: true, pinnedRev: s.entries.get(s.previewEntry).sourceRev, approvedScopes: s.scopes, approvedInputs: s.inputs, unattended: false, undo: true, hold: "none", retry: false }), preview: "none", screen: if(s.selected == s.previewEntry) "contribution_approved" else s.screen }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:1757-1795; ipc.rs:7286-7440
  action YouApproveThePinnedPreview = all {
    controls(s.screen).contains("Submit") and (s.entries.get(s.selected).state == "pending" and s.entries.get(s.selected).artifact and s.enrolled and not(s.globalHold)),
    s' = { ...s, entries: s.entries.set(s.selected, { ...s.entries.get(s.selected), state: "approved", approvedScopes: s.scopes, approvedInputs: s.inputs, unattended: false, undo: true, hold: "none", retry: false }), screen: "contribution_approved", preview: "none" }
  }

  // crates/trace-commons-contributor/src/daemon/watcher.rs:1741-1778,2025-2039; queue.rs:1802-1825
  action TheStandingGrantApprovesATrace = {
    nondet daemonEntry = Set(0, 1).oneOf()
    all {
      s.entries.get(daemonEntry).state == "pending" and s.entries.get(daemonEntry).hold == "none" and s.enrolled and s.projectMode == "automatic" and s.grant and not(s.globalHold) and s.scrub == "automatic",
      s' = { ...s, entries: s.entries.set(daemonEntry, { ...s.entries.get(daemonEntry), state: "approved", approvedScopes: s.scopes, approvedInputs: s.inputs, unattended: true, undo: false, retry: false }), screen: if(s.selected == daemonEntry) "contribution_approved" else s.screen }
    }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:2519-2560
  action YouUndoTheApproval = all {
    controls(s.screen).contains("Undo") and (s.entries.get(s.selected).state == "approved"),
    s' = { ...s, entries: s.entries.set(s.selected, { ...s.entries.get(s.selected), state: "pending", approvedScopes: -1, approvedInputs: -1, pin: 0, artifact: false, unattended: false, undo: false }), screen: "contribution_pending" }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:844-857,1961-1971
  action TheUndoWindowPasses = {
    nondet daemonEntry = Set(0, 1).oneOf()
    all {
      s.entries.get(daemonEntry).state == "approved" and s.entries.get(daemonEntry).undo,
      s' = { ...s, entries: s.entries.set(daemonEntry, { ...s.entries.get(daemonEntry), undo: false }) }
    }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:1298-1331 [original hold restoration coarsened]
  action YouKeepTheTraceLocally = all {
    controls(s.screen).contains("Keep") and (s.entries.get(s.selected).state == "pending" or (s.entries.get(s.selected).state == "approved" and s.entries.get(s.selected).unattended)),
    s' = { ...s, entries: s.entries.set(s.selected, { ...s.entries.get(s.selected), state: "refused", hold: "kept", pin: 0, artifact: false, approvedScopes: -1, approvedInputs: -1, unattended: false }), screen: "contribution_kept" }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:1341-1381 [cap represented as capacity input]
  action YouUndoKeepingTheTrace = all {
    controls(s.screen).contains("Undo") and (s.entries.get(s.selected).state == "refused" and s.entries.get(s.selected).hold == "kept"),
    s' = { ...s, entries: s.entries.set(s.selected, { ...s.entries.get(s.selected), state: "pending", hold: "returned_from_keep" }), screen: "contribution_held" }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:894; ipc.rs:4280-4388 [dismiss route]
  action YouDismissTheTrace = all {
    controls(s.screen).contains("Not this one") and (s.entries.get(s.selected).state == "pending"),
    s' = { ...s, entries: s.entries.set(s.selected, { ...s.entries.get(s.selected), state: "refused", hold: "dismissed", pin: 0, artifact: false }), screen: "contribution_refused" }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:2570-2590 [clock relation older-than-TTL input]
  action TheOfferExpires = {
    nondet daemonEntry = Set(0, 1).oneOf()
    all {
      s.entries.get(daemonEntry).state == "pending" and s.healthy,
      s' = { ...s, entries: s.entries.set(daemonEntry, { ...s.entries.get(daemonEntry), state: "expired", hold: "expired" }), screen: if(s.selected == daemonEntry) "contribution_expired" else s.screen }
    }
  }

  // crates/trace-commons-contributor/src/daemon/policy.rs:69-93; settings.rs:350-374
  action YouOpenSharingPolicy = all {
    controls(s.screen).contains("Policy") and (true),
    s' = { ...s, screen: "contribution_policy" }
  }

  // crates/trace-commons-contributor/src/daemon/policy.rs:69-93 [NotifyOnly]
  action YouChooseProjectAsk = all {
    controls(s.screen).contains("Ask") and (s.screen == "contribution_policy"),
    s' = { ...s, projectMode: "ask", entries: s.entries.set(0, if(s.entries.get(0).state == "approved" and s.entries.get(0).unattended) { ...s.entries.get(0), state: "pending", hold: "none", retry: false, pin: 0, artifact: false, approvedScopes: -1, approvedInputs: -1, unattended: false, undo: false } else s.entries.get(0)).set(1, if(s.entries.get(1).state == "approved" and s.entries.get(1).unattended) { ...s.entries.get(1), state: "pending", hold: "none", retry: false, pin: 0, artifact: false, approvedScopes: -1, approvedInputs: -1, unattended: false, undo: false } else s.entries.get(1)) }
  }

  // crates/trace-commons-contributor/src/daemon/automatic_gate.rs:1-53 [validated disclosure/grant result coarse]
  action YouChooseProjectAutomatic = all {
    controls(s.screen).contains("Automatic") and (s.screen == "contribution_policy" and s.enrolled),
    s' = { ...s, projectMode: "automatic", grant: true }
  }

  // crates/trace-commons-contributor/src/daemon/ipc.rs:5910-5968; queue.rs:1480-1532 [synchronous pending/unattended retraction; Uploading unchanged]
  action YouChooseProjectNever = all {
    controls(s.screen).contains("Never") and (s.screen == "contribution_policy"),
    s' = { ...s, projectMode: "never", entries: s.entries.set(0, if(s.entries.get(0).state == "pending" or (s.entries.get(0).state == "approved" and s.entries.get(0).unattended)) { ...s.entries.get(0), state: "refused", hold: "project_ignored", retry: false } else s.entries.get(0)).set(1, if(s.entries.get(1).state == "pending" or (s.entries.get(1).state == "approved" and s.entries.get(1).unattended)) { ...s.entries.get(1), state: "refused", hold: "project_ignored", retry: false } else s.entries.get(1)) }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:2018-2039; settings.rs:350-374
  action YouSwitchToManualScrubCheck = all {
    controls(s.screen).contains("Manual Scrub check") and (s.screen == "contribution_policy"),
    s' = { ...s, scrub: "manual" }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:879-882; mod.rs:1260-1300 [scope change and re-offer separated]
  action YouChangeConsentScopes = all {
    controls(s.screen).contains("Change scopes") and (s.scopes == 0),
    s' = { ...s, scopes: 1 }
  }

  // crates/trace-commons-contributor/src/daemon/uploader.rs:558-575
  action YouChangeEnvelopeInputs = all {
    controls(s.screen).contains("Change inputs") and (s.inputs == 0),
    s' = { ...s, inputs: 1 }
  }

  // crates/trace-commons-contributor/src/daemon/watcher.rs:130-178 [declaration update coarse]
  action YouDisableTheSource = all {
    controls(s.screen).contains("Disable source") and (s.sourceEnabled),
    s' = { ...s, sourceEnabled: false }
  }

  // crates/trace-commons-contributor/src/daemon/watcher.rs:130-178
  action YouEnableTheSource = all {
    controls(s.screen).contains("Enable source") and (not(s.sourceEnabled)),
    s' = { ...s, sourceEnabled: true }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:1212-1214 [UI selection]
  action YouReturnToTheTrace = all {
    controls(s.screen).contains("Back to trace") and (true),
    s' = { ...s, screen: "contribution_empty" }
  }

  // crates/trace-commons-contributor/src/daemon/mod.rs:1260-1300; queue.rs:2078-2126
  action TheChangedScopeRevokesApproval = {
    nondet daemonEntry = Set(0, 1).oneOf()
    all {
      s.entries.get(daemonEntry).state == "approved" and s.entries.get(daemonEntry).approvedScopes != s.scopes,
      s' = { ...s, entries: s.entries.set(daemonEntry, { ...s.entries.get(daemonEntry), state: "pending", pin: 0, artifact: false, approvedScopes: -1, approvedInputs: -1, unattended: false, undo: false }), screen: if(s.selected == daemonEntry) "contribution_pending" else s.screen }
    }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:2018-2039
  action TheManualScrubSettingRetractsAnUnattendedApproval = {
    nondet daemonEntry = Set(0, 1).oneOf()
    all {
      s.entries.get(daemonEntry).state == "approved" and s.entries.get(daemonEntry).unattended and s.scrub == "manual",
      s' = { ...s, entries: s.entries.set(daemonEntry, { ...s.entries.get(daemonEntry), state: "pending", hold: "manual_scrub", approvedScopes: -1, approvedInputs: -1, unattended: false }), screen: if(s.selected == daemonEntry) "contribution_held" else s.screen }
    }
  }

  // crates/trace-commons-contributor/src/daemon/uploader.rs:401-410 [disk/storage fault input]
  action ThePinnedArtifactIsLost = {
    nondet daemonEntry = Set(0, 1).oneOf()
    all {
      s.entries.get(daemonEntry).pin != 0 and s.entries.get(daemonEntry).artifact,
      s' = { ...s, entries: s.entries.set(daemonEntry, { ...s.entries.get(daemonEntry), artifact: false }) }
    }
  }

  // crates/trace-commons-contributor/src/daemon/mod.rs:1310-1385; queue.rs:1961-1972
  action TheUploaderClaimsTheApproval = {
    nondet daemonEntry = Set(0, 1).oneOf()
    all {
      s.upload == "none" and s.entries.get(daemonEntry).state == "approved" and not(s.entries.get(daemonEntry).undo) and not(s.entries.get(daemonEntry).retry) and s.entries.get(daemonEntry).approvedScopes == s.scopes and (not(s.entries.get(daemonEntry).unattended) or s.projectMode == "automatic") and not(s.globalHold) and s.capAvailable,
      s' = { ...s, entries: s.entries.set(daemonEntry, { ...s.entries.get(daemonEntry), state: "uploading" }), uploadEntry: daemonEntry, upload: "checking", screen: if(s.selected == daemonEntry) "contribution_uploading" else s.screen }
    }
  }

  // crates/trace-commons-contributor/src/daemon/uploader.rs:380-389,494-503 [unenroll races settled before clearing in full runtime]
  action TheEnrollmentDisappears = all {
    s.enrolled,
    s' = { ...s, enrolled: false }
  }

  // crates/trace-commons-contributor/src/daemon/account_onboarding.rs:669-688 [enrollment boundary]
  action TheEnrollmentReturns = all {
    not(s.enrolled),
    s' = { ...s, enrolled: true }
  }

  // crates/trace-commons-contributor/src/daemon/mod.rs:1210-1310 [numeric volumes coarse]
  action TheDailyCapIsReached = all {
    s.capAvailable,
    s' = { ...s, capAvailable: false }
  }

  // crates/trace-commons-contributor/src/daemon/mod.rs:1210-1310
  action TheDailyCapResets = all {
    not(s.capAvailable),
    s' = { ...s, capAvailable: true }
  }

  // crates/trace-commons-contributor/src/daemon/uploader.rs:494-503
  action TheUploaderRefusesMissingEnrollment = all {
    s.upload == "checking" and not(s.enrolled),
    s' = { ...s, entries: s.entries.set(s.uploadEntry, { ...s.entries.get(s.uploadEntry), state: "refused", hold: "not_logged_in" }), upload: "none", screen: if(s.selected == s.uploadEntry) "contribution_refused" else s.screen }
  }

  // crates/trace-commons-contributor/src/daemon/uploader.rs:527-555; mod.rs:1539-1554 [replacement allocation coarse]
  action TheUploaderSupersedesChangedSource = all {
    s.upload == "checking" and s.entries.get(s.uploadEntry).sourceRev != s.entries.get(s.uploadEntry).offeredRev,
    s' = { ...s, entries: s.entries.set(s.uploadEntry, { ...s.entries.get(s.uploadEntry), state: "superseded" }), upload: "none", screen: if(s.selected == s.uploadEntry) "contribution_superseded" else s.screen }
  }

  // crates/trace-commons-contributor/src/daemon/uploader.rs:558-652; mod.rs:1579-1594
  action TheUploaderReoffersChangedInputsOrMissingPin = all {
    s.upload == "checking" and (s.entries.get(s.uploadEntry).approvedInputs != s.inputs or (s.entries.get(s.uploadEntry).pin != 0 and not(s.entries.get(s.uploadEntry).artifact))),
    s' = { ...s, entries: s.entries.set(s.uploadEntry, { ...s.entries.get(s.uploadEntry), state: "pending", approvedScopes: -1, approvedInputs: -1, pin: 0, artifact: false, unattended: false }), upload: "none", screen: if(s.selected == s.uploadEntry) "contribution_pending" else s.screen }
  }

  // crates/trace-commons-contributor/src/daemon/uploader.rs:586-654,724-746; mod.rs:1596-1635
  action ThePrivacyOrWitnessReviewHoldsTheTrace = all {
    s.upload == "checking" and s.entries.get(s.uploadEntry).unattended,
    s' = { ...s, entries: s.entries.set(s.uploadEntry, { ...s.entries.get(s.uploadEntry), state: "pending", hold: "needs_person", pin: s.uploadEntry + 1, artifact: true, approvedScopes: -1, approvedInputs: -1, unattended: false }), upload: "none", screen: if(s.selected == s.uploadEntry) "contribution_held" else s.screen }
  }

  // crates/trace-commons-contributor/src/daemon/uploader.rs:663-778; mod.rs:1557-1577
  action ThePrivacyFilterRefusesTheTrace = all {
    s.upload == "checking",
    s' = { ...s, entries: s.entries.set(s.uploadEntry, { ...s.entries.get(s.uploadEntry), state: "refused", hold: "privacy_refused" }), upload: "none", screen: if(s.selected == s.uploadEntry) "contribution_refused" else s.screen }
  }

  // crates/trace-commons-contributor/src/daemon/mod.rs:1727-1749 [no per-session attempt consumed]
  action TheWitnessIsAtCapacity = all {
    s.upload == "checking",
    s' = { ...s, entries: s.entries.set(s.uploadEntry, { ...s.entries.get(s.uploadEntry), state: "approved", retry: true }), upload: "none", backoff: "witness", screen: if(s.selected == s.uploadEntry) "contribution_offline" else s.screen }
  }

  // crates/trace-commons-contributor/src/daemon/mod.rs:1750-1783 [same abstraction for local burst pacing]
  action TheIngestRateLimitDefersTheSend = all {
    s.upload == "checking",
    s' = { ...s, entries: s.entries.set(s.uploadEntry, { ...s.entries.get(s.uploadEntry), state: "approved", retry: true }), upload: "none", backoff: "rate_limit", screen: if(s.selected == s.uploadEntry) "contribution_offline" else s.screen }
  }

  // crates/trace-commons-contributor/src/daemon/uploader.rs:475-778 [privacy/witness valid evidence input]
  action TheUploadPassesItsPreconditions = all {
    s.upload == "checking" and s.enrolled and s.entries.get(s.uploadEntry).sourceRev == s.entries.get(s.uploadEntry).offeredRev and s.entries.get(s.uploadEntry).approvedInputs == s.inputs and (s.entries.get(s.uploadEntry).pin == 0 or s.entries.get(s.uploadEntry).artifact) and (not(s.entries.get(s.uploadEntry).unattended) or s.scrub == "automatic"),
    s' = { ...s, upload: "sending" }
  }

  // crates/trace-commons-contributor/src/daemon/mod.rs:1682-1725 [one retained retry plus one exhausted retry]
  action TheNetworkFailsDuringSend = all {
    s.upload == "sending" and s.entries.get(s.uploadEntry).attempts < 1,
    s' = { ...s, entries: s.entries.set(s.uploadEntry, { ...s.entries.get(s.uploadEntry), state: "approved", retry: true, attempts: s.entries.get(s.uploadEntry).attempts + 1 }), upload: "none", backoff: "network", network: false, screen: if(s.selected == s.uploadEntry) "contribution_offline" else s.screen }
  }

  // crates/trace-commons-contributor/src/daemon/mod.rs:1682-1725
  action TheRetryBudgetIsSpent = all {
    s.upload == "sending" and s.entries.get(s.uploadEntry).attempts == 1,
    s' = { ...s, entries: s.entries.set(s.uploadEntry, { ...s.entries.get(s.uploadEntry), state: "failed", attempts: 2 }), upload: "none", screen: if(s.selected == s.uploadEntry) "contribution_failed" else s.screen }
  }

  // crates/trace-commons-contributor/src/daemon/mod.rs:1650-1680 [retry ladder coarsened]
  action TheRedactionRetryBudgetIsSpent = all {
    s.upload == "checking" and s.entries.get(s.uploadEntry).unattended,
    s' = { ...s, entries: s.entries.set(s.uploadEntry, { ...s.entries.get(s.uploadEntry), state: "pending", hold: "classifier_exhausted", approvedScopes: -1, approvedInputs: -1, unattended: false }), upload: "none", screen: if(s.selected == s.uploadEntry) "contribution_held" else s.screen }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:2225-2290 [time relation, no fairness]
  action TheBackoffPasses = {
    nondet daemonEntry = Set(0, 1).oneOf()
    all {
      s.entries.get(daemonEntry).retry,
      s' = { ...s, entries: s.entries.set(daemonEntry, { ...s.entries.get(daemonEntry), retry: false }), backoff: "none", network: true }
    }
  }

  // crates/trace-commons-contributor/src/daemon/mod.rs:1518-1532,1836-1880
  action TheSendSucceedsWithProofReceipt = all {
    s.upload == "sending" and s.network,
    s' = { ...s, entries: s.entries.set(s.uploadEntry, { ...s.entries.get(s.uploadEntry), state: "uploaded", submission: s.uploadEntry + 1, receipt: "present", history: "submitted" }), upload: "none", screen: if(s.selected == s.uploadEntry) "contribution_uploaded" else s.screen }
  }

  // crates/trace-commons-contributor/src/daemon/mod.rs:1836-1880 [ReceiptShipped missing is separate from local submission receipt]
  action TheSendSucceedsWithoutProofReceipt = all {
    s.upload == "sending" and s.network,
    s' = { ...s, entries: s.entries.set(s.uploadEntry, { ...s.entries.get(s.uploadEntry), state: "uploaded", submission: s.uploadEntry + 1, receipt: "missing", history: "submitted" }), upload: "none", screen: if(s.selected == s.uploadEntry) "contribution_uploaded" else s.screen }
  }

  // crates/trace-commons-contributor/src/daemon/mod.rs:1682-1725 [environment input]
  action TheNetworkReturns = all {
    not(s.network),
    s' = { ...s, network: true }
  }

  // crates/trace-commons-contributor/src/daemon/ipc.rs:7150-7156; queue.rs:1670-1681 [explicit reoffer coarse]
  action YouRetryAFailedTrace = all {
    controls(s.screen).contains("Retry") and (s.entries.get(s.selected).state == "failed"),
    s' = { ...s, entries: s.entries.set(s.selected, { ...s.entries.get(s.selected), state: "pending", approvedScopes: -1, approvedInputs: -1, pin: 0, artifact: false, attempts: 0 }), screen: "contribution_pending" }
  }

  // crates/trace-commons-contributor/src/daemon/queue.rs:1981-1991; mod.rs:1450-1469 [release in flight at load/pass exit]
  action TheDaemonRestartsDuringAnUpload = all {
    s.upload != "none",
    s' = { ...s, entries: s.entries.set(s.uploadEntry, { ...s.entries.get(s.uploadEntry), state: "approved" }), upload: "none", screen: if(s.selected == s.uploadEntry) "contribution_approved" else s.screen }
  }

  // crates/trace-commons-contributor/src/daemon/history.rs:312-349
  action YouOpenHistory = all {
    controls(s.screen).contains("History") and (true),
    s' = { ...s, screen: "contribution_history" }
  }

  // crates/trace-commons-contributor/src/daemon/history.rs:312-349
  action YouRefreshHistory = all {
    controls(s.screen).contains("Refresh history") and (s.historyRead != "loading"),
    s' = { ...s, historyRead: "loading" }
  }

  // crates/trace-commons-contributor/src/daemon/history.rs:90-105,349-492
  action TheHistoryReadFindsProcessing = all {
    s.historyRead == "loading" and s.entries.get(s.selected).withdrawal != "confirmed" and s.entries.get(s.selected).submission != 0,
    s' = { ...s, entries: s.entries.set(s.selected, { ...s.entries.get(s.selected), history: "processing" }), historyRead: "answered" }
  }

  // crates/trace-commons-contributor/src/daemon/history.rs:90-105,349-492 [credit/rejection alternatives owned by server models]
  action TheHistoryReadFindsASettledVerdict = all {
    s.historyRead == "loading" and s.entries.get(s.selected).withdrawal != "confirmed" and s.entries.get(s.selected).submission != 0,
    s' = { ...s, entries: s.entries.set(s.selected, { ...s.entries.get(s.selected), history: "accepted" }), historyRead: "answered" }
  }

  // crates/trace-commons-contributor/src/daemon/history.rs:329-349 [age/current-account receipt-scope conditions coarse]
  action TheHistoryReadFindsPreviousAccount = all {
    s.historyRead == "loading" and s.entries.get(s.selected).submission != 0 and s.entries.get(s.selected).account != s.account,
    s' = { ...s, entries: s.entries.set(s.selected, { ...s.entries.get(s.selected), history: "previous_account" }), historyRead: "answered" }
  }

  // crates/trace-commons-contributor/src/daemon/history.rs:312-349 [cached history retained]
  action TheHistoryReadFails = all {
    s.historyRead == "loading",
    s' = { ...s, historyRead: "failed" }
  }

  // crates/trace-commons-contributor/src/daemon/history.rs:329-349 [authority boundary input]
  action TheCurrentAccountChanges = all {
    s.account == 1,
    s' = { ...s, account: 2, historyRead: "unread" }
  }



  // macos/Sources/TraceCommonsApp/AppModel.swift:3652-3690: success then explicit history reread.
  action YouWithdrawTheContributedTrace = all {
    controls(s.screen).contains("Withdraw") and s.entries.get(s.selected).submission != 0 and s.withdrawalCall == "none",
    s' = { ...s, withdrawalEntry: s.selected, withdrawalCall: "request" }
  }
  // macos/Sources/TraceCommonsApp/AppModel.swift:3673-3679
  action TheWithdrawalRequestSucceeds = all {
    s.withdrawalCall == "request",
    s' = { ...s, entries: s.entries.set(s.withdrawalEntry, { ...s.entries.get(s.withdrawalEntry), withdrawal: "confirmed" }), withdrawalCall: "none", historyRead: "loading" }
  }
  // macos/Sources/TraceCommonsApp/AppModel.swift:3680-3688
  action TheWithdrawalNeedsAnAccountSession = all {
    s.withdrawalCall == "request",
    s' = { ...s, entries: s.entries.set(s.withdrawalEntry, { ...s.entries.get(s.withdrawalEntry), withdrawal: "sign_in_required" }), withdrawalCall: "none" }
  }
  // macos/Sources/TraceCommonsApp/AppModel.swift:3680-3685
  action TheWithdrawalRequestFails = all {
    s.withdrawalCall == "request",
    s' = { ...s, entries: s.entries.set(s.withdrawalEntry, { ...s.entries.get(s.withdrawalEntry), withdrawal: "failed" }), withdrawalCall: "none" }
  }
  // macos/Sources/TraceCommonsApp/AppModel.swift:3664-3665,3678-3679
  action TheHistoryReadReflectsWithdrawal = all {
    s.historyRead == "loading" and s.entries.get(s.selected).withdrawal == "confirmed",
    s' = { ...s, entries: s.entries.set(s.selected, { ...s.entries.get(s.selected), history: "withdrawn" }), historyRead: "answered" }
  }


  // crates/trace-commons-contributor/src/daemon/ipc.rs:6072-6220; policy.rs:678-686.
  // Global Never is separate from a folder Never: preserve manual approval/pin,
  // return queued unattended approvals to waiting, and do not retract Uploading.
  action YouHoldEverySend = all {
    controls(s.screen).contains("Hold all sends") and not(s.globalHold),
    s' = { ...s, globalHold: true, entries: s.entries.set(0, if(s.entries.get(0).state == "approved" and s.entries.get(0).unattended) { ...s.entries.get(0), state: "pending", hold: "none", retry: false, pin: 0, artifact: false, approvedScopes: -1, approvedInputs: -1, unattended: false, undo: false } else s.entries.get(0)).set(1, if(s.entries.get(1).state == "approved" and s.entries.get(1).unattended) { ...s.entries.get(1), state: "pending", hold: "none", retry: false, pin: 0, artifact: false, approvedScopes: -1, approvedInputs: -1, unattended: false, undo: false } else s.entries.get(1)) }
  }
  // crates/trace-commons-contributor/src/daemon/ipc.rs:6222-6270; policy.rs:689-699.
  action YouResumeEverySend = all {
    controls(s.screen).contains("Resume all sends") and s.globalHold,
    s' = { ...s, globalHold: false }
  }
  // crates/trace-commons-contributor/src/daemon/mod.rs:1091-1208.
  // A previously Uploading unattended row can return Approved after restart or pass exit.
  action TheAskPolicyReturnsAnUnattendedApprovalToWaiting = {
    nondet daemonEntry = Set(0, 1).oneOf()
    all {
      s.entries.get(daemonEntry).state == "approved" and s.entries.get(daemonEntry).unattended and (s.projectMode == "ask" or (s.globalHold and s.projectMode != "never")),
      s' = { ...s, entries: s.entries.set(daemonEntry, { ...s.entries.get(daemonEntry), state: "pending", hold: "none", retry: false, pin: 0, artifact: false, approvedScopes: -1, approvedInputs: -1, unattended: false, undo: false }), screen: if(s.selected == daemonEntry) "contribution_pending" else s.screen }
    }
  }
  // crates/trace-commons-contributor/src/daemon/mod.rs:1136-1185; queue.rs:1512-1532.
  action TheNeverFolderRefusesAnUnattendedApproval = {
    nondet daemonEntry = Set(0, 1).oneOf()
    all {
      s.entries.get(daemonEntry).state == "approved" and s.entries.get(daemonEntry).unattended and s.projectMode == "never",
      s' = { ...s, entries: s.entries.set(daemonEntry, { ...s.entries.get(daemonEntry), state: "refused", hold: "project_ignored", retry: false }), screen: if(s.selected == daemonEntry) "contribution_refused" else s.screen }
    }
  }

  action step = any {
    YouHoldEverySend,
    YouResumeEverySend,
    TheAskPolicyReturnsAnUnattendedApprovalToWaiting,
    TheNeverFolderRefusesAnUnattendedApproval,
    YouWithdrawTheContributedTrace,
    TheWithdrawalRequestSucceeds,
    TheWithdrawalNeedsAnAccountSession,
    TheWithdrawalRequestFails,
    TheHistoryReadReflectsWithdrawal,
    TheWatcherDiscoversATrace,
    TheSourcePermissionIsLost,
    TheSourcePermissionReturns,
    TheSourceSessionChanges,
    YouSelectTheOtherTrace,
    TheSelectedTraceRowIsRendered,
    YouOpenThePreview,
    ThePreviewPinsAnEnvelope,
    ThePreviewFails,
    YouCloseThePreview,
    TheUnreferencedPreviewPinIsReleased,
    YouSubmitWithoutOpeningPreview,
    TheApprovalBuildPinsAndApproves,
    YouApproveThePinnedPreview,
    TheStandingGrantApprovesATrace,
    YouUndoTheApproval,
    TheUndoWindowPasses,
    YouKeepTheTraceLocally,
    YouUndoKeepingTheTrace,
    YouDismissTheTrace,
    TheOfferExpires,
    YouOpenSharingPolicy,
    YouChooseProjectAsk,
    YouChooseProjectAutomatic,
    YouChooseProjectNever,
    YouSwitchToManualScrubCheck,
    YouChangeConsentScopes,
    YouChangeEnvelopeInputs,
    YouDisableTheSource,
    YouEnableTheSource,
    YouReturnToTheTrace,
    TheChangedScopeRevokesApproval,
    TheManualScrubSettingRetractsAnUnattendedApproval,
    ThePinnedArtifactIsLost,
    TheUploaderClaimsTheApproval,
    TheEnrollmentDisappears,
    TheEnrollmentReturns,
    TheDailyCapIsReached,
    TheDailyCapResets,
    TheUploaderRefusesMissingEnrollment,
    TheUploaderSupersedesChangedSource,
    TheUploaderReoffersChangedInputsOrMissingPin,
    ThePrivacyOrWitnessReviewHoldsTheTrace,
    ThePrivacyFilterRefusesTheTrace,
    TheWitnessIsAtCapacity,
    TheIngestRateLimitDefersTheSend,
    TheUploadPassesItsPreconditions,
    TheNetworkFailsDuringSend,
    TheRetryBudgetIsSpent,
    TheRedactionRetryBudgetIsSpent,
    TheBackoffPasses,
    TheSendSucceedsWithProofReceipt,
    TheSendSucceedsWithoutProofReceipt,
    TheNetworkReturns,
    YouRetryAFailedTrace,
    TheDaemonRestartsDuringAnUpload,
    YouOpenHistory,
    YouRefreshHistory,
    TheHistoryReadFindsProcessing,
    TheHistoryReadFindsASettledVerdict,
    TheHistoryReadFindsPreviousAccount,
    TheHistoryReadFails,
    TheCurrentAccountChanges
  }
}
