// Current behavior projection at 8218cc8; no product promises selected.
module client_learning {
  var s: {
    screen: str,
    account: int,
    ownerGeneration: int,
    active: bool,
    presentation: int,
    insightsRequestPresentation: int,
    insights: str,
    analysisEnabled: bool,
    savedSnapshot: bool,
    counterFeed: str,
    feed: str,
    figures: bool,
    week: int,
    weekRequest: int,
    weekGeneration: int,
    weekRequestGeneration: int,
    missionActive: bool,
    missionPresentation: int,
    missionRequestPresentation: int,
    missionMutation: str,
    draftExists: bool,
    draftDetail: str,
    draftNotice: str,
    skills: int -> {phase: str, operation: str, owner: int, requestOwner: int, ownerGeneration: int, requestGeneration: int, package: int, reviewPackage: int, evaluatedPackage: int, passed: bool, planPackage: int, marker: int, planLive: bool, filesystem: bool, filesystemOwner: int, installedStatus: str, editedOnDisk: bool, published: bool},
    selected: int,
    requestEntry: int,
    evaluationBusy: bool,
    publicationOperation: str,
    error: str
  }
  val vars = s

  pure def controls(frame: str): Set[str] =
    if (frame == "learning_insights_empty") Set("Enable", "Open file", "Refresh", "Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_insights_busy") Set("Refresh", "Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_insights") Set("Refresh", "Open file", "Select week", "Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_insights_error") Set("Refresh", "Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_missions_empty") Set("Choose file", "Import", "Refresh", "Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_missions") Set("Choose file", "Import", "Delete", "Refresh", "Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_mission_busy") Set("Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_skill_empty") Set("Learn", "Select trace", "Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_skill_busy") Set("Select trace", "Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_skill_candidate") Set("Review skill", "Edit skill", "Select trace", "Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_skill_review") Set("Approve & test", "Edit skill", "Select trace", "Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_skill_results") Set("Review install", "Edit skill", "Select trace", "Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_skill_plan") Set("Install skill", "Edit skill", "Select trace", "Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_skill_installed") Set("Roll back", "Select trace", "Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_rollback_confirm") Set("Roll back", "Keep", "Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_skill_error") Set("Retry install", "Learn", "Edit skill", "Select trace", "Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_publication") Set("Publish", "Unpublish", "Insights", "Mission drafts", "Learn", "Close")
    else     if (frame == "learning_closed") Set("Open")
    else Set()

  action init = s' = {
    screen: "learning_insights_empty",
    account: 1,
    ownerGeneration: 0,
    active: true,
    presentation: 0,
    insightsRequestPresentation: 0,
    insights: "none",
    analysisEnabled: false,
    savedSnapshot: false,
    counterFeed: "unread",
    feed: "saved",
    figures: false,
    week: 0,
    weekRequest: 0,
    weekGeneration: 0,
    weekRequestGeneration: 0,
    missionActive: false,
    missionPresentation: 0,
    missionRequestPresentation: 0,
    missionMutation: "none",
    draftExists: false,
    draftDetail: "none",
    draftNotice: "none",
    skills: Map(0 -> {phase: "idle", operation: "none", owner: 1, requestOwner: 1, ownerGeneration: 0, requestGeneration: 0, package: 0, reviewPackage: -1, evaluatedPackage: -1, passed: false, planPackage: -1, marker: 0, planLive: false, filesystem: false, filesystemOwner: 1, installedStatus: "unknown", editedOnDisk: false, published: false}, 1 -> {phase: "idle", operation: "none", owner: 1, requestOwner: 1, ownerGeneration: 0, requestGeneration: 0, package: 0, reviewPackage: -1, evaluatedPackage: -1, passed: false, planPackage: -1, marker: 0, planLive: false, filesystem: false, filesystemOwner: 1, installedStatus: "unknown", editedOnDisk: false, published: false}),
    selected: 0,
    requestEntry: 0,
    evaluationBusy: false,
    publicationOperation: "none",
    error: "none"
  }

  // macos/Sources/TraceCommonsApp/InsightsModel.swift:76-112
  action YouOpenInsights = all {
    controls(s.screen).contains("Insights") and (true),
    s' = { ...s, active: true, insights: "loading", insightsRequestPresentation: s.presentation, counterFeed: "loading", weekRequest: s.week, weekRequestGeneration: s.weekGeneration, screen: "learning_insights_busy" }
  }

  // macos/Sources/TraceCommonsApp/Views/InsightsGetStarted.swift:1-32 [validated settings result abstracted]
  action YouEnableLocalTraceAnalysis = all {
    controls(s.screen).contains("Enable") and (not(s.analysisEnabled)),
    s' = { ...s, analysisEnabled: true }
  }

  // crates/trace-commons-contributor-insights/src/insights/service.rs:297-318 [local import transaction coarse]
  action YouImportASavedInsight = all {
    controls(s.screen).contains("Open file") and (s.active),
    s' = { ...s, savedSnapshot: true, screen: "learning_insights" }
  }

  // macos/Sources/TraceCommonsApp/InsightsModel.swift:87-119
  action YouRefreshInsights = all {
    controls(s.screen).contains("Refresh") and (s.active and not(s.missionActive)),
    s' = { ...s, insights: "loading", insightsRequestPresentation: s.presentation, screen: "learning_insights_busy" }
  }

  // macos/Sources/TraceCommonsApp/InsightsModel.swift:97-119
  action TheInsightSnapshotReadReturns = all {
    s.insights == "loading" and s.active and s.insightsRequestPresentation == s.presentation,
    s' = { ...s, insights: "ready", figures: s.savedSnapshot, screen: if(s.savedSnapshot) "learning_insights" else "learning_insights_empty" }
  }

  // macos/Sources/TraceCommonsApp/InsightsModel.swift:97-119 [overview failure clears figures separately]
  action TheInsightSnapshotReadFails = all {
    s.insights == "loading" and s.active and s.insightsRequestPresentation == s.presentation,
    s' = { ...s, insights: "error", figures: false, error: "insights-operation-failed", screen: "learning_insights_error" }
  }

  // macos/Sources/TraceCommonsApp/InsightsModel.swift:52-70
  action TheReadableCounterWeekReturns = all {
    s.counterFeed == "loading" and s.active and s.weekRequestGeneration == s.weekGeneration and s.analysisEnabled,
    s' = { ...s, counterFeed: "ready", feed: "counter", figures: true, screen: "learning_insights" }
  }

  // macos/Sources/TraceCommonsApp/InsightsModel.swift:52-73 [later failure drops earlier counter week whole]
  action TheCounterWeekIsDisabledOrUnreadable = all {
    s.counterFeed == "loading" and s.active and s.weekRequestGeneration == s.weekGeneration,
    s' = { ...s, counterFeed: "unavailable", feed: "saved", figures: s.savedSnapshot, screen: if(s.savedSnapshot) "learning_insights" else "learning_insights_empty" }
  }

  // macos/Sources/TraceCommonsApp/InsightsModel.swift:174-188
  action YouSelectAnotherInsightWeek = {
    nondet freshWeekGeneration = Set(0, 1, 2).exclude(Set(s.weekGeneration, s.weekRequestGeneration)).oneOf()
    all {
      controls(s.screen).contains("Select week") and (true),
      s' = { ...s, week: 1 - s.week, weekGeneration: freshWeekGeneration, counterFeed: "loading", weekRequest: 1 - s.week, weekRequestGeneration: freshWeekGeneration, figures: false, screen: "learning_insights_busy" }
    }
  }

  // macos/Sources/TraceCommonsApp/InsightsModel.swift:53-59
  action TheOldWeekReadIsDiscarded = all {
    s.weekRequestGeneration != s.weekGeneration,
    s' = { ...s, error: "old-week-result-discarded" }
  }

  // macos/Sources/TraceCommonsApp/MissionDraftsModel.swift:79-83
  action YouOpenMissionDrafts = all {
    controls(s.screen).contains("Mission drafts") and (true),
    s' = { ...s, missionActive: true, draftDetail: "none", screen: if(s.draftExists) "learning_missions" else "learning_missions_empty" }
  }

  // macos/Sources/TraceCommonsApp/Views/MissionDraftsView.swift:61-67,91-98
  action YouChooseAMissionDraftFile = all {
    controls(s.screen).contains("Choose file") and (s.missionActive and s.missionMutation == "none"),
    s' = { ...s, draftDetail: "file_selected" }
  }

  // macos/Sources/TraceCommonsApp/MissionDraftsModel.swift:199-218
  action YouImportTheMissionDraft = all {
    controls(s.screen).contains("Import") and (s.missionActive and s.missionMutation == "none" and s.draftDetail == "file_selected"),
    s' = { ...s, missionMutation: "import", missionRequestPresentation: s.missionPresentation, screen: "learning_mission_busy" }
  }

  // macos/Sources/TraceCommonsApp/MissionDraftsModel.swift:178-198 [detail identity/presentation confirmation coarse]
  action YouDeleteTheMissionDraft = all {
    controls(s.screen).contains("Delete") and (s.missionActive and s.missionMutation == "none" and s.draftExists),
    s' = { ...s, missionMutation: "delete", missionRequestPresentation: s.missionPresentation, screen: "learning_mission_busy" }
  }

  // macos/Sources/TraceCommonsApp/MissionDraftsModel.swift:221-262 [reopened view refreshes after late native result]
  action TheMissionMutationCommits = all {
    s.missionMutation != "none",
    s' = { ...s, draftExists: s.missionMutation == "import", draftNotice: if(s.missionActive and s.missionRequestPresentation == s.missionPresentation) "confirmed" else "none", missionMutation: "none", draftDetail: "none", screen: if(s.screen == "learning_closed") s.screen else if(s.missionActive) "learning_missions" else s.screen }
  }

  // macos/Sources/TraceCommonsApp/MissionDraftsModel.swift:263-279
  action TheMissionMutationFails = all {
    s.missionMutation != "none",
    s' = { ...s, missionMutation: "none", draftNotice: "none", error: if(s.missionActive) "mission-mutation-failed" else s.error, screen: if(s.screen == "learning_closed") s.screen else "learning_missions" }
  }

  // macos/Sources/TraceCommonsApp/MissionDraftsModel.swift:122-147 [list read/result coarse]
  action YouRefreshMissionDrafts = all {
    controls(s.screen).contains("Refresh") and (s.missionActive and s.missionMutation == "none"),
    s' = { ...s, draftNotice: "refreshed" }
  }

  // macos/Sources/TraceCommonsApp/AppModel+SkillLearning.swift:9-38; SkillLearningStore.swift:152-177
  action YouStartLearningFromATrace = all {
    controls(s.screen).contains("Learn") and (s.skills.get(s.selected).phase == "idle" and s.skills.get(s.selected).operation == "none"),
    s' = { ...s, skills: s.skills.set(s.selected, { ...s.skills.get(s.selected), operation: if(s.skills.get(s.selected).installedStatus == "absent") "candidate" else "status", requestOwner: s.account, requestGeneration: s.ownerGeneration }), requestEntry: s.selected, screen: "learning_skill_busy" }
  }

  // macos/Sources/TraceCommonsApp/AppModel+SkillLearning.swift:52-108
  action TheInstallStatusShowsNoSkill = all {
    s.skills.get(s.requestEntry).operation == "status" and not(s.skills.get(s.requestEntry).filesystem),
    s' = { ...s, skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), installedStatus: "absent", operation: "candidate" }) }
  }

  // crates/trace-commons-contributor/src/daemon/skill_loop/handlers.rs:691-735; macos/Sources/TraceCommonsApp/SkillLearningStore.swift:192-205
  action TheInstallStatusRecoversAnOwnedSkill = all {
    s.skills.get(s.requestEntry).operation == "status" and s.skills.get(s.requestEntry).filesystem and s.skills.get(s.requestEntry).filesystemOwner == s.account,
    s' = { ...s, skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), installedStatus: "installed", operation: "none" }), screen: if(s.screen == "learning_closed") s.screen else "learning_skill_installed" }
  }

  // crates/trace-commons-contributor/src/daemon/skill_loop/handlers.rs:304-344; macos/Sources/TraceCommonsApp/SkillLearningStore.swift:160-177
  action TheSkillCandidateReturns = all {
    s.skills.get(s.requestEntry).operation == "candidate" and s.skills.get(s.requestEntry).requestOwner == s.account and s.skills.get(s.requestEntry).requestGeneration == s.ownerGeneration,
    s' = { ...s, skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), phase: "candidate", operation: "none", owner: s.account, ownerGeneration: s.ownerGeneration }), screen: if(s.screen == "learning_closed") s.screen else "learning_skill_candidate" }
  }

  // macos/Sources/TraceCommonsApp/AppModel+SkillLearning.swift:111-140; SkillLearningStore.swift:226-258
  action YouReviewTheExactSkillPackage = all {
    controls(s.screen).contains("Review skill") and (s.skills.get(s.selected).phase == "candidate" and s.skills.get(s.selected).operation == "none"),
    s' = { ...s, skills: s.skills.set(s.selected, { ...s.skills.get(s.selected), operation: "review", requestOwner: s.account, requestGeneration: s.ownerGeneration }), requestEntry: s.selected, screen: "learning_skill_busy" }
  }

  // crates/trace-commons-contributor/src/daemon/skill_loop/handlers.rs:346-383
  action TheSkillReviewBindsPackageBytes = all {
    s.skills.get(s.requestEntry).operation == "review" and s.skills.get(s.requestEntry).requestOwner == s.account and s.skills.get(s.requestEntry).requestGeneration == s.ownerGeneration,
    s' = { ...s, skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), phase: "reviewed", reviewPackage: s.skills.get(s.requestEntry).package, operation: "none" }), screen: if(s.screen == "learning_closed") s.screen else "learning_skill_review" }
  }

  // macos/Sources/TraceCommonsApp/AppModel+SkillLearning.swift:142-169; crates/trace-commons-contributor/src/daemon/skill_loop/handlers.rs:387-469
  action YouApproveSkillEvaluation = all {
    controls(s.screen).contains("Approve & test") and (s.skills.get(s.selected).phase == "reviewed" and s.skills.get(s.selected).operation == "none" and not(s.evaluationBusy)),
    s' = { ...s, skills: s.skills.set(s.selected, { ...s.skills.get(s.selected), operation: "evaluate", requestOwner: s.account, requestGeneration: s.ownerGeneration }), requestEntry: s.selected, evaluationBusy: true, screen: "learning_skill_busy" }
  }

  // macos/Sources/TraceCommonsApp/SkillLearningStore.swift:272-283; crates/trace-commons-contributor/src/daemon/skill_loop/handlers.rs:449-467
  action TheSkillEvaluationPasses = all {
    s.skills.get(s.requestEntry).operation == "evaluate" and s.skills.get(s.requestEntry).requestOwner == s.account and s.skills.get(s.requestEntry).requestGeneration == s.ownerGeneration,
    s' = { ...s, skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), phase: "evaluated", evaluatedPackage: s.skills.get(s.requestEntry).reviewPackage, passed: true, operation: "none" }), evaluationBusy: false, screen: if(s.screen == "learning_closed") s.screen else "learning_skill_results" }
  }

  // macos/Sources/TraceCommonsApp/SkillLearningStore.swift:272-283
  action TheSkillEvaluationNeedsRevision = all {
    s.skills.get(s.requestEntry).operation == "evaluate" and s.skills.get(s.requestEntry).requestOwner == s.account and s.skills.get(s.requestEntry).requestGeneration == s.ownerGeneration,
    s' = { ...s, skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), phase: "evaluated", evaluatedPackage: s.skills.get(s.requestEntry).reviewPackage, passed: false, operation: "none" }), evaluationBusy: false, screen: if(s.screen == "learning_closed") s.screen else "learning_skill_results" }
  }

  // macos/Sources/TraceCommonsApp/SkillLearningStore.swift:423-443
  action YouEditTheSkill = all {
    controls(s.screen).contains("Edit skill") and (Set("candidate", "reviewed", "evaluated", "planned").contains(s.skills.get(s.selected).phase) and s.skills.get(s.selected).operation == "none" and s.skills.get(s.selected).package == 0),
    s' = { ...s, skills: s.skills.set(s.selected, { ...s.skills.get(s.selected), phase: "candidate", package: 1, passed: false, planLive: false }), screen: "learning_skill_candidate" }
  }

  // macos/Sources/TraceCommonsApp/SkillLearningStore.swift:286-325
  action YouReviewInstallingTheSkill = all {
    controls(s.screen).contains("Review install") and (s.skills.get(s.selected).phase == "evaluated" and s.skills.get(s.selected).passed and s.skills.get(s.selected).operation == "none"),
    s' = { ...s, skills: s.skills.set(s.selected, { ...s.skills.get(s.selected), operation: "plan", requestOwner: s.account, requestGeneration: s.ownerGeneration }), requestEntry: s.selected, screen: "learning_skill_busy" }
  }

  // crates/trace-commons-contributor/src/daemon/skill_loop/handlers.rs:510-599
  action TheSkillInstallPlanIsHeld = all {
    s.skills.get(s.requestEntry).operation == "plan" and s.skills.get(s.requestEntry).requestOwner == s.account and s.skills.get(s.requestEntry).requestGeneration == s.ownerGeneration,
    s' = { ...s, skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), phase: "planned", planPackage: s.skills.get(s.requestEntry).evaluatedPackage, marker: s.requestEntry + 1, planLive: true, operation: "none" }), screen: if(s.screen == "learning_closed") s.screen else "learning_skill_plan" }
  }

  // crates/trace-commons-contributor/src/daemon/skill_loop.rs:30-31,172-185 [10 minute TTL relation]
  action TheHeldInstallPlanExpires = all {
    s.skills.get(s.selected).phase == "planned" and s.skills.get(s.selected).planLive,
    s' = { ...s, skills: s.skills.set(s.selected, { ...s.skills.get(s.selected), planLive: false }) }
  }

  // macos/Sources/TraceCommonsApp/AppModel+SkillLearning.swift:193-219; crates/trace-commons-contributor/src/daemon/skill_loop/handlers.rs:602-647
  action YouInstallTheReviewedSkill = all {
    controls(s.screen).contains("Install skill") and (s.skills.get(s.selected).phase == "planned" and s.skills.get(s.selected).operation == "none" and s.skills.get(s.selected).planLive and not(s.skills.get(s.selected).filesystem)),
    s' = { ...s, skills: s.skills.set(s.selected, { ...s.skills.get(s.selected), operation: "install", requestOwner: s.account, requestGeneration: s.ownerGeneration }), requestEntry: s.selected, screen: "learning_skill_busy" }
  }

  // crates/trace-commons-contributor/src/daemon/skill_loop/handlers.rs:637-687 [exclusive filesystem publication then owner recheck]
  action TheFilesystemPublishesTheSkill = all {
    s.skills.get(s.requestEntry).operation == "install" and s.skills.get(s.requestEntry).requestOwner == s.account and s.skills.get(s.requestEntry).requestGeneration == s.ownerGeneration and s.skills.get(s.requestEntry).planLive,
    s' = { ...s, skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), filesystem: true, filesystemOwner: s.account, operation: "reconcile" }) }
  }

  // macos/Sources/TraceCommonsApp/AppModel+SkillLearning.swift:211-249
  action TheInstallStatusConfirmsPublication = all {
    s.skills.get(s.requestEntry).operation == "reconcile" and s.skills.get(s.requestEntry).filesystem and s.skills.get(s.requestEntry).filesystemOwner == s.account,
    s' = { ...s, skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), installedStatus: "installed", operation: "none" }), screen: if(s.screen == "learning_closed") s.screen else "learning_skill_installed" }
  }

  // macos/Sources/TraceCommonsApp/AppModel+SkillLearning.swift:250-260; SkillLearningStore.swift:356-376
  action TheInstallStatusReadFailsAfterPublication = all {
    s.skills.get(s.requestEntry).operation == "reconcile" and s.skills.get(s.requestEntry).filesystem,
    s' = { ...s, skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), installedStatus: "unknown", operation: "none" }), error: "install-status-unresolved", screen: if(s.screen == "learning_closed") s.screen else "learning_skill_error" }
  }

  // macos/Sources/TraceCommonsApp/AppModel+SkillLearning.swift:52-108
  action YouRetryInstallStatus = all {
    controls(s.screen).contains("Retry install") and (s.skills.get(s.selected).filesystem and s.skills.get(s.selected).installedStatus == "unknown" and s.skills.get(s.selected).operation == "none"),
    s' = { ...s, skills: s.skills.set(s.selected, { ...s.skills.get(s.selected), operation: "status" }), requestEntry: s.selected, screen: "learning_skill_busy" }
  }

  // crates/trace-commons-contributor/src/skill_loop/install.rs:375-393 [file identity input]
  action TheInstalledPackageIsChangedExternally = all {
    s.skills.get(s.selected).filesystem and not(s.skills.get(s.selected).editedOnDisk),
    s' = { ...s, skills: s.skills.set(s.selected, { ...s.skills.get(s.selected), editedOnDisk: true }) }
  }

  // macos/Sources/TraceCommonsApp/Views/SkillLearningView.swift:1-35 [confirmation layout, exact copy in skill_loop/copy.rs:148-158]
  action YouAskToRollBackTheSkill = all {
    controls(s.screen).contains("Roll back") and (s.skills.get(s.selected).installedStatus == "installed" and s.skills.get(s.selected).operation == "none" and s.screen != "learning_rollback_confirm"),
    s' = { ...s, screen: "learning_rollback_confirm" }
  }

  // macos/Sources/TraceCommonsApp/AppModel+SkillLearning.swift:266-312
  action YouConfirmSkillRollback = all {
    controls(s.screen).contains("Roll back") and (s.screen == "learning_rollback_confirm" and s.skills.get(s.selected).operation == "none"),
    s' = { ...s, skills: s.skills.set(s.selected, { ...s.skills.get(s.selected), operation: "rollback", requestOwner: s.account, requestGeneration: s.ownerGeneration }), requestEntry: s.selected, screen: "learning_skill_busy" }
  }

  // crates/trace-commons-contributor/src/skill_loop/copy.rs:148-158
  action YouKeepTheInstalledSkill = all {
    controls(s.screen).contains("Keep") and (s.screen == "learning_rollback_confirm"),
    s' = { ...s, screen: "learning_skill_installed" }
  }

  // macos/Sources/TraceCommonsApp/AppModel+SkillLearning.swift:266-312
  action TheUnchangedSkillIsRolledBack = all {
    s.skills.get(s.requestEntry).operation == "rollback" and not(s.skills.get(s.requestEntry).editedOnDisk) and s.skills.get(s.requestEntry).filesystemOwner == s.account,
    s' = { ...s, skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), filesystem: false, installedStatus: "absent", operation: "none" }), screen: if(s.screen == "learning_closed") s.screen else "learning_skill_results" }
  }

  // macos/Sources/TraceCommonsApp/SkillLearningStore.swift:379-402; crates/trace-commons-contributor/src/skill_loop/copy.rs:143-156
  action TheChangedSkillCannotBeRolledBack = all {
    s.skills.get(s.requestEntry).operation == "rollback" and s.skills.get(s.requestEntry).editedOnDisk,
    s' = { ...s, skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), operation: "none" }), error: "rollback-incomplete", screen: if(s.screen == "learning_closed") s.screen else "learning_skill_installed" }
  }

  // macos/Sources/TraceCommonsApp/SkillLearningStore.swift:148-150 [two submission selection abstraction]
  action YouSelectAnotherLearningTrace = all {
    controls(s.screen).contains("Select trace") and (true),
    s' = { ...s, selected: 1 - s.selected, screen: "learning_skill_empty" }
  }

  // crates/trace-commons-contributor/src/daemon/skill_loop.rs:132-158; macos/Sources/TraceCommonsApp/AppModel.swift:3989 [UI store reset modeled below]
  action TheLearningAccountChanges = all {
    s.account == 1,
    s' = { ...s, account: 2, ownerGeneration: 1, evaluationBusy: false, skills: s.skills.set(0, { ...s.skills.get(0), phase: "idle", owner: 2, ownerGeneration: 1, installedStatus: "unknown", planLive: false }).set(1, { ...s.skills.get(1), phase: "idle", owner: 2, ownerGeneration: 1, installedStatus: "unknown", planLive: false }) }
  }

  // crates/trace-commons-contributor/src/daemon/skill_loop/handlers.rs:184-225 [post-publication compensation is separate]
  action TheOldSkillOperationLosesItsOwnerLease = all {
    s.skills.get(s.requestEntry).operation != "none" and (s.skills.get(s.requestEntry).requestOwner != s.account or s.skills.get(s.requestEntry).requestGeneration != s.ownerGeneration),
    s' = { ...s, skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), operation: "none", phase: "idle", installedStatus: "unknown", planLive: false }), evaluationBusy: false, error: "skill-owner-changed", screen: if(s.screen == "learning_closed") s.screen else "learning_skill_error" }
  }

  // crates/trace-commons-contributor/src/daemon/skill_loop/handlers.rs:239-270,648-687
  action TheOwnerChangedPublishedInstallIsCompensated = all {
    s.skills.get(s.requestEntry).filesystem and s.skills.get(s.requestEntry).filesystemOwner != s.account and not(s.skills.get(s.requestEntry).editedOnDisk),
    s' = { ...s, skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), filesystem: false, installedStatus: "unknown" }) }
  }

  // macos/Sources/TraceCommonsApp/SkillLearningStore.swift:406-421 [phase-specific error recovery coarse]
  action TheSkillOperationFails = all {
    s.skills.get(s.requestEntry).operation != "none",
    s' = { ...s, skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), operation: "none" }), evaluationBusy: false, error: "skill-operation-failed", screen: if(s.screen == "learning_closed") s.screen else "learning_skill_error" }
  }

  // crates/trace-commons-contributor/src/daemon/skill_loop.rs:41-53 [held workflows in memory, filesystem marker durable]
  action TheDaemonRestartsAndDropsHeldSkillWorkflow = all {
    s.skills.get(s.selected).phase != "idle",
    s' = { ...s, skills: s.skills.set(s.selected, { ...s.skills.get(s.selected), phase: "idle", operation: "none", installedStatus: "unknown", planLive: false }), evaluationBusy: false }
  }

  // crates/trace-commons-contributor/src/daemon/public_run.rs:25-27,269-294 [reviewed detail input]
  action ThePublishedRunReviewIsOpened = all {
    s.publicationOperation == "none" and s.screen != "learning_closed",
    s' = { ...s, screen: "learning_publication" }
  }

  // crates/trace-commons-contributor/src/daemon/public_run.rs:269-318 [approved digest/version/request scope inputs coarse]
  action YouPublishTheReviewedRun = all {
    controls(s.screen).contains("Publish") and (s.publicationOperation == "none"),
    s' = { ...s, publicationOperation: "publish" }
  }

  // crates/trace-commons-contributor/src/daemon/public_run.rs:320-349
  action YouUnpublishTheRun = all {
    controls(s.screen).contains("Unpublish") and (s.skills.get(s.selected).published and s.publicationOperation == "none"),
    s' = { ...s, publicationOperation: "unpublish" }
  }

  // crates/trace-commons-contributor/src/daemon/public_run.rs:300-349 [independent of skill install]
  action TheRunPublicationReturns = all {
    s.publicationOperation != "none",
    s' = { ...s, skills: s.skills.set(s.selected, { ...s.skills.get(s.selected), published: s.publicationOperation == "publish" }), publicationOperation: "none" }
  }

  // macos/Sources/TraceCommonsApp/InsightsModel.swift:81-85; MissionDraftsModel.swift:86-100 [mutation remains alive]
  action YouCloseLearning = {
    nondet freshPresentation = Set(0, 1, 2).exclude(Set(s.presentation, s.insightsRequestPresentation)).oneOf()
    nondet freshMissionPresentation = Set(0, 1, 2).exclude(Set(s.missionPresentation, s.missionRequestPresentation)).oneOf()
    nondet freshWeekGeneration = Set(0, 1, 2).exclude(Set(s.weekGeneration, s.weekRequestGeneration)).oneOf()
    all {
      controls(s.screen).contains("Close") and (true),
      s' = { ...s, active: false, missionActive: false, presentation: freshPresentation, missionPresentation: freshMissionPresentation, weekGeneration: freshWeekGeneration, screen: "learning_closed", figures: false }
    }
  }

  // macos/Sources/TraceCommonsApp/MissionDraftsModel.swift:79-100
  action YouReopenLearning = all {
    controls(s.screen).contains("Open") and (s.screen == "learning_closed"),
    s' = { ...s, active: true, missionActive: true, screen: if(s.missionMutation != "none") "learning_mission_busy" else "learning_insights_empty" }
  }

  action step = any {
    YouOpenInsights,
    YouEnableLocalTraceAnalysis,
    YouImportASavedInsight,
    YouRefreshInsights,
    TheInsightSnapshotReadReturns,
    TheInsightSnapshotReadFails,
    TheReadableCounterWeekReturns,
    TheCounterWeekIsDisabledOrUnreadable,
    YouSelectAnotherInsightWeek,
    TheOldWeekReadIsDiscarded,
    YouOpenMissionDrafts,
    YouChooseAMissionDraftFile,
    YouImportTheMissionDraft,
    YouDeleteTheMissionDraft,
    TheMissionMutationCommits,
    TheMissionMutationFails,
    YouRefreshMissionDrafts,
    YouStartLearningFromATrace,
    TheInstallStatusShowsNoSkill,
    TheInstallStatusRecoversAnOwnedSkill,
    TheSkillCandidateReturns,
    YouReviewTheExactSkillPackage,
    TheSkillReviewBindsPackageBytes,
    YouApproveSkillEvaluation,
    TheSkillEvaluationPasses,
    TheSkillEvaluationNeedsRevision,
    YouEditTheSkill,
    YouReviewInstallingTheSkill,
    TheSkillInstallPlanIsHeld,
    TheHeldInstallPlanExpires,
    YouInstallTheReviewedSkill,
    TheFilesystemPublishesTheSkill,
    TheInstallStatusConfirmsPublication,
    TheInstallStatusReadFailsAfterPublication,
    YouRetryInstallStatus,
    TheInstalledPackageIsChangedExternally,
    YouAskToRollBackTheSkill,
    YouConfirmSkillRollback,
    YouKeepTheInstalledSkill,
    TheUnchangedSkillIsRolledBack,
    TheChangedSkillCannotBeRolledBack,
    YouSelectAnotherLearningTrace,
    TheLearningAccountChanges,
    TheOldSkillOperationLosesItsOwnerLease,
    TheOwnerChangedPublishedInstallIsCompensated,
    TheSkillOperationFails,
    TheDaemonRestartsAndDropsHeldSkillWorkflow,
    ThePublishedRunReviewIsOpened,
    YouPublishTheReviewedRun,
    YouUnpublishTheRun,
    TheRunPublicationReturns,
    YouCloseLearning,
    YouReopenLearning
  }
}
