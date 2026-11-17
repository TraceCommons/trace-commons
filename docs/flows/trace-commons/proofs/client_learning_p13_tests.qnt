module client_learning_p13_tests {
  import client_learning.* from "../models/client_learning"

  action reviewedInstall = init
    .then(YouStartLearningFromATrace)
    .then(TheInstallStatusShowsNoSkill)
    .then(TheSkillCandidateReturns)
    .then(YouReviewTheExactSkillPackage)
    .then(TheSkillReviewBindsPackageBytes)
    .then(YouApproveSkillEvaluation)
    .then(TheSkillEvaluationPasses)
    .then(YouReviewInstallingTheSkill)
    .then(TheSkillInstallPlanIsHeld)
    .then(YouInstallTheReviewedSkill)

  run confirmedPublicationTest = reviewedInstall
    .then(TheFilesystemPublishesTheSkill)
    .then(TheInstallStatusConfirmsPublication)
    .expect(P13)

  run laterAccountChangeDoesNotRewritePublicationHistoryTest = reviewedInstall
    .then(TheFilesystemPublishesTheSkill)
    .then(TheInstallStatusConfirmsPublication)
    .then(TheInstalledPackageIsChangedExternally)
    .then(TheLearningAccountChanges)
    .expect(P13)

  run laterExternalEditDoesNotRewritePublicationHistoryTest = reviewedInstall
    .then(TheFilesystemPublishesTheSkill)
    .then(TheInstallStatusConfirmsPublication)
    .then(TheInstalledPackageIsChangedExternally)
    .expect(P13)

  action claimDaemonSuccessWithoutStatus = s' = { ...s,
    skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), installedStatus: "installed" }) }

  run publicationObserverDetectsUnreconciledSuccessTest = reviewedInstall
    .then(TheFilesystemPublishesTheSkill)
    .then(claimDaemonSuccessWithoutStatus)
    .expect(not(P13))

  // Fault injection is test-only: this is not an allowed product transition.
  action loseConfirmation = s' = { ...s,
    skills: s.skills.set(s.requestEntry, { ...s.skills.get(s.requestEntry), confirmationPresent: false }) }

  run publicationObserverDetectsMissingConfirmationTest = reviewedInstall
    .then(loseConfirmation)
    .then(TheFilesystemPublishesTheSkill)
    .expect(not(P13))
}
