// Proof-only symbolic induction harness. Imports every original transition.
module client_learning_p13_induction {
  import client_learning.* from "../models/client_learning"
  type State = {
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
    skills: int -> Skill,
    selected: int,
    requestEntry: int,
    evaluationBusy: bool,
    publicationOperation: str,
    error: str,
    installViolation: bool
  }
  pure val ints = Set(-1, 0, 1, 2)
  pure val texts = Set("Approve & test", "Choose file", "Close", "Delete", "Edit skill", "Enable", "Import", "Insights", "Install skill", "Keep", "Learn", "Mission drafts", "Open", "Open file", "Publish", "Refresh", "Retry install", "Review install", "Review skill", "Roll back", "Select trace", "Select week", "Unpublish", "absent", "candidate", "confirmed", "counter", "delete", "error", "evaluate", "evaluated", "file_selected", "idle", "import", "insights-operation-failed", "install", "install-status-unresolved", "installed", "learning_closed", "learning_insights", "learning_insights_busy", "learning_insights_empty", "learning_insights_error", "learning_mission_busy", "learning_missions", "learning_missions_empty", "learning_publication", "learning_rollback_confirm", "learning_skill_busy", "learning_skill_candidate", "learning_skill_empty", "learning_skill_error", "learning_skill_installed", "learning_skill_plan", "learning_skill_results", "learning_skill_review", "loading", "mission-mutation-failed", "none", "old-week-result-discarded", "plan", "planned", "publish", "ready", "reconcile", "refreshed", "review", "reviewed", "rollback", "rollback-incomplete", "saved", "skill-operation-failed", "skill-owner-changed", "status", "unavailable", "unknown", "unpublish", "unread")
  pure val booleans = Set(false, true)
  pure def skillStrong(e: Skill, account: int, generation: int): bool = all {
    Set("reviewed", "evaluated", "planned").contains(e.phase) implies e.package == e.reviewPackage,
    Set("evaluated", "planned").contains(e.phase) implies e.reviewPackage == e.evaluatedPackage,
    e.phase == "planned" implies (e.passed and e.planPackage == e.evaluatedPackage),
    e.owner == account and e.ownerGeneration == generation,
    Set(1, 2).contains(e.requestOwner) and e.requestOwner <= account and e.requestGeneration == e.requestOwner - 1,
    (e.operation == "evaluate" and e.requestOwner == account and e.requestGeneration == generation)
      implies e.phase == "reviewed",
    (e.operation == "plan" and e.requestOwner == account and e.requestGeneration == generation)
      implies (e.phase == "evaluated" and e.passed),
    (e.operation == "install" and e.requestOwner == account and e.requestGeneration == generation and e.planLive)
      implies installMatchesConfirmation(e, account, generation),
    e.installedStatus == "installed" implies (e.reconciledPublication and e.filesystem),
    e.operation == "install" implies e.installedStatus != "installed"
  }
  pure def strongFor(t: State): bool = not(t.installViolation) and t.ownerGeneration == t.account - 1 and
    Set(0, 1).forall(i => skillStrong(t.skills.get(i), t.account, t.ownerGeneration))
  val strengthened = strongFor(s) and domainFor(s)
  val implication = not(strengthened) or P13

  pure def domainFor(t: State): bool = t.skills.keys() == Set(0, 1) and
    texts.contains(t.screen) and
    Set(1, 2).contains(t.account) and
    Set(0, 1).contains(t.ownerGeneration) and
    booleans.contains(t.active) and
    Set(0, 1, 2).contains(t.presentation) and
    Set(0, 1, 2).contains(t.insightsRequestPresentation) and
    texts.contains(t.insights) and
    booleans.contains(t.analysisEnabled) and
    booleans.contains(t.savedSnapshot) and
    texts.contains(t.counterFeed) and
    texts.contains(t.feed) and
    booleans.contains(t.figures) and
    Set(0, 1).contains(t.week) and
    Set(0, 1).contains(t.weekRequest) and
    Set(0, 1, 2).contains(t.weekGeneration) and
    Set(0, 1, 2).contains(t.weekRequestGeneration) and
    booleans.contains(t.missionActive) and
    Set(0, 1, 2).contains(t.missionPresentation) and
    Set(0, 1, 2).contains(t.missionRequestPresentation) and
    texts.contains(t.missionMutation) and
    booleans.contains(t.draftExists) and
    texts.contains(t.draftDetail) and
    texts.contains(t.draftNotice) and
    Set(0, 1).contains(t.selected) and
    Set(0, 1).contains(t.requestEntry) and
    booleans.contains(t.evaluationBusy) and
    texts.contains(t.publicationOperation) and
    texts.contains(t.error) and
    booleans.contains(t.installViolation) and
    Set(0, 1).forall(i => texts.contains(t.skills.get(i).phase)) and
    Set(0, 1).forall(i => texts.contains(t.skills.get(i).operation)) and
    Set(0, 1).forall(i => ints.contains(t.skills.get(i).owner)) and
    Set(0, 1).forall(i => ints.contains(t.skills.get(i).requestOwner)) and
    Set(0, 1).forall(i => ints.contains(t.skills.get(i).ownerGeneration)) and
    Set(0, 1).forall(i => ints.contains(t.skills.get(i).requestGeneration)) and
    Set(0, 1).forall(i => ints.contains(t.skills.get(i).package)) and
    Set(0, 1).forall(i => ints.contains(t.skills.get(i).reviewPackage)) and
    Set(0, 1).forall(i => ints.contains(t.skills.get(i).evaluatedPackage)) and
    Set(0, 1).forall(i => booleans.contains(t.skills.get(i).passed)) and
    Set(0, 1).forall(i => ints.contains(t.skills.get(i).planPackage)) and
    Set(0, 1).forall(i => ints.contains(t.skills.get(i).marker)) and
    Set(0, 1).forall(i => booleans.contains(t.skills.get(i).planLive)) and
    Set(0, 1).forall(i => booleans.contains(t.skills.get(i).filesystem)) and
    Set(0, 1).forall(i => ints.contains(t.skills.get(i).filesystemOwner)) and
    Set(0, 1).forall(i => texts.contains(t.skills.get(i).installedStatus)) and
    Set(0, 1).forall(i => booleans.contains(t.skills.get(i).editedOnDisk)) and
    Set(0, 1).forall(i => booleans.contains(t.skills.get(i).published)) and
    Set(0, 1).forall(i => ints.contains(t.skills.get(i).planId)) and
    Set(0, 1).forall(i => booleans.contains(t.skills.get(i).confirmationPresent)) and
    Set(0, 1).forall(i => ints.contains(t.skills.get(i).confirmationPlanId)) and
    Set(0, 1).forall(i => ints.contains(t.skills.get(i).confirmationPackage)) and
    Set(0, 1).forall(i => ints.contains(t.skills.get(i).confirmationMarker)) and
    Set(0, 1).forall(i => ints.contains(t.skills.get(i).confirmationOwner)) and
    Set(0, 1).forall(i => ints.contains(t.skills.get(i).confirmationGeneration)) and
    Set(0, 1).forall(i => booleans.contains(t.skills.get(i).confirmationTested)) and
    Set(0, 1).forall(i => booleans.contains(t.skills.get(i).reconciledPublication))
  action arbitrary = {
    nondet e0_phase = texts.oneOf()
    nondet e0_operation = texts.oneOf()
    nondet e0_owner = ints.oneOf()
    nondet e0_requestOwner = ints.oneOf()
    nondet e0_ownerGeneration = ints.oneOf()
    nondet e0_requestGeneration = ints.oneOf()
    nondet e0_package = ints.oneOf()
    nondet e0_reviewPackage = ints.oneOf()
    nondet e0_evaluatedPackage = ints.oneOf()
    nondet e0_passed = booleans.oneOf()
    nondet e0_planPackage = ints.oneOf()
    nondet e0_marker = ints.oneOf()
    nondet e0_planLive = booleans.oneOf()
    nondet e0_filesystem = booleans.oneOf()
    nondet e0_filesystemOwner = ints.oneOf()
    nondet e0_installedStatus = texts.oneOf()
    nondet e0_editedOnDisk = booleans.oneOf()
    nondet e0_published = booleans.oneOf()
    nondet e0_planId = ints.oneOf()
    nondet e0_confirmationPresent = booleans.oneOf()
    nondet e0_confirmationPlanId = ints.oneOf()
    nondet e0_confirmationPackage = ints.oneOf()
    nondet e0_confirmationMarker = ints.oneOf()
    nondet e0_confirmationOwner = ints.oneOf()
    nondet e0_confirmationGeneration = ints.oneOf()
    nondet e0_confirmationTested = booleans.oneOf()
    nondet e0_reconciledPublication = booleans.oneOf()
    val e0 = { phase: e0_phase, operation: e0_operation, owner: e0_owner, requestOwner: e0_requestOwner, ownerGeneration: e0_ownerGeneration, requestGeneration: e0_requestGeneration, package: e0_package, reviewPackage: e0_reviewPackage, evaluatedPackage: e0_evaluatedPackage, passed: e0_passed, planPackage: e0_planPackage, marker: e0_marker, planLive: e0_planLive, filesystem: e0_filesystem, filesystemOwner: e0_filesystemOwner, installedStatus: e0_installedStatus, editedOnDisk: e0_editedOnDisk, published: e0_published, planId: e0_planId, confirmationPresent: e0_confirmationPresent, confirmationPlanId: e0_confirmationPlanId, confirmationPackage: e0_confirmationPackage, confirmationMarker: e0_confirmationMarker, confirmationOwner: e0_confirmationOwner, confirmationGeneration: e0_confirmationGeneration, confirmationTested: e0_confirmationTested, reconciledPublication: e0_reconciledPublication }
    nondet e1_phase = texts.oneOf()
    nondet e1_operation = texts.oneOf()
    nondet e1_owner = ints.oneOf()
    nondet e1_requestOwner = ints.oneOf()
    nondet e1_ownerGeneration = ints.oneOf()
    nondet e1_requestGeneration = ints.oneOf()
    nondet e1_package = ints.oneOf()
    nondet e1_reviewPackage = ints.oneOf()
    nondet e1_evaluatedPackage = ints.oneOf()
    nondet e1_passed = booleans.oneOf()
    nondet e1_planPackage = ints.oneOf()
    nondet e1_marker = ints.oneOf()
    nondet e1_planLive = booleans.oneOf()
    nondet e1_filesystem = booleans.oneOf()
    nondet e1_filesystemOwner = ints.oneOf()
    nondet e1_installedStatus = texts.oneOf()
    nondet e1_editedOnDisk = booleans.oneOf()
    nondet e1_published = booleans.oneOf()
    nondet e1_planId = ints.oneOf()
    nondet e1_confirmationPresent = booleans.oneOf()
    nondet e1_confirmationPlanId = ints.oneOf()
    nondet e1_confirmationPackage = ints.oneOf()
    nondet e1_confirmationMarker = ints.oneOf()
    nondet e1_confirmationOwner = ints.oneOf()
    nondet e1_confirmationGeneration = ints.oneOf()
    nondet e1_confirmationTested = booleans.oneOf()
    nondet e1_reconciledPublication = booleans.oneOf()
    val e1 = { phase: e1_phase, operation: e1_operation, owner: e1_owner, requestOwner: e1_requestOwner, ownerGeneration: e1_ownerGeneration, requestGeneration: e1_requestGeneration, package: e1_package, reviewPackage: e1_reviewPackage, evaluatedPackage: e1_evaluatedPackage, passed: e1_passed, planPackage: e1_planPackage, marker: e1_marker, planLive: e1_planLive, filesystem: e1_filesystem, filesystemOwner: e1_filesystemOwner, installedStatus: e1_installedStatus, editedOnDisk: e1_editedOnDisk, published: e1_published, planId: e1_planId, confirmationPresent: e1_confirmationPresent, confirmationPlanId: e1_confirmationPlanId, confirmationPackage: e1_confirmationPackage, confirmationMarker: e1_confirmationMarker, confirmationOwner: e1_confirmationOwner, confirmationGeneration: e1_confirmationGeneration, confirmationTested: e1_confirmationTested, reconciledPublication: e1_reconciledPublication }
    nondet t_screen = texts.oneOf()
    nondet t_account = Set(1, 2).oneOf()
    nondet t_ownerGeneration = Set(0, 1).oneOf()
    nondet t_active = booleans.oneOf()
    nondet t_presentation = Set(0, 1, 2).oneOf()
    nondet t_insightsRequestPresentation = Set(0, 1, 2).oneOf()
    nondet t_insights = texts.oneOf()
    nondet t_analysisEnabled = booleans.oneOf()
    nondet t_savedSnapshot = booleans.oneOf()
    nondet t_counterFeed = texts.oneOf()
    nondet t_feed = texts.oneOf()
    nondet t_figures = booleans.oneOf()
    nondet t_week = Set(0, 1).oneOf()
    nondet t_weekRequest = Set(0, 1).oneOf()
    nondet t_weekGeneration = Set(0, 1, 2).oneOf()
    nondet t_weekRequestGeneration = Set(0, 1, 2).oneOf()
    nondet t_missionActive = booleans.oneOf()
    nondet t_missionPresentation = Set(0, 1, 2).oneOf()
    nondet t_missionRequestPresentation = Set(0, 1, 2).oneOf()
    nondet t_missionMutation = texts.oneOf()
    nondet t_draftExists = booleans.oneOf()
    nondet t_draftDetail = texts.oneOf()
    nondet t_draftNotice = texts.oneOf()
    nondet t_selected = Set(0, 1).oneOf()
    nondet t_requestEntry = Set(0, 1).oneOf()
    nondet t_evaluationBusy = booleans.oneOf()
    nondet t_publicationOperation = texts.oneOf()
    nondet t_error = texts.oneOf()
    nondet t_installViolation = booleans.oneOf()
    val t = { screen: t_screen, account: t_account, ownerGeneration: t_ownerGeneration, active: t_active, presentation: t_presentation, insightsRequestPresentation: t_insightsRequestPresentation, insights: t_insights, analysisEnabled: t_analysisEnabled, savedSnapshot: t_savedSnapshot, counterFeed: t_counterFeed, feed: t_feed, figures: t_figures, week: t_week, weekRequest: t_weekRequest, weekGeneration: t_weekGeneration, weekRequestGeneration: t_weekRequestGeneration, missionActive: t_missionActive, missionPresentation: t_missionPresentation, missionRequestPresentation: t_missionRequestPresentation, missionMutation: t_missionMutation, draftExists: t_draftExists, draftDetail: t_draftDetail, draftNotice: t_draftNotice, skills: Map(0 -> e0, 1 -> e1), selected: t_selected, requestEntry: t_requestEntry, evaluationBusy: t_evaluationBusy, publicationOperation: t_publicationOperation, error: t_error, installViolation: t_installViolation }
    all { strongFor(t), s' = t }
  }
}
