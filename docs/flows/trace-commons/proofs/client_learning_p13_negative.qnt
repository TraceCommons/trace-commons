// Proof-only negative control. This fault is absent from the product step.
module client_learning_p13_negative {
  import client_learning.* from "../models/client_learning"
  import client_learning_p13_induction.* from "client_learning_p13_induction"

  // Erase a captured confirmation while leaving the held install request intact.
  // A subsequent original filesystem publication must expose the missing consent.
  action eraseConfirmation = all {
    s.skills.get(s.requestEntry).operation == "install",
    s.skills.get(s.requestEntry).planLive,
    s.skills.get(s.requestEntry).requestOwner == s.account,
    s.skills.get(s.requestEntry).requestGeneration == s.ownerGeneration,
    s' = { ...s, skills: s.skills.set(s.requestEntry,
      { ...s.skills.get(s.requestEntry), confirmationPresent: false }) }
  }
  action negativeStep = any { eraseConfirmation, TheFilesystemPublishesTheSkill }
}
