// Explicitly coarse service boundaries. This is not an implementation model of each algorithm.
module server_interfaces {
  pure val Actors = Set(0, 1)
  pure val Keys = Set(0, 1)
  pure val Families = Set("legacy_gate", "replay_export", "benchmark", "ranker", "active_learning", "utility", "vector", "access_grant", "community_read", "community_profile", "reward_catalog", "reward_account", "activity_catalog", "activity_account", "token_bundle", "token_research", "analytics", "drill", "public_source", "public_stats", "public_estimate", "public_usage", "collateral", "process_evaluation", "credit_admin", "account_trust", "inference_account", "audit")
  pure val Anonymous = Set("public_source", "public_stats", "public_estimate", "public_usage", "collateral", "reward_catalog", "activity_catalog", "community_read")
  pure val ReadOnly = Set("public_source", "public_stats", "public_estimate", "collateral", "reward_catalog", "activity_catalog", "activity_account", "community_read", "active_learning")
  pure val Panels = Set("interfaces-idle", "interface-authorizing", "interface-running", "interface-returned", "interface-refused", "token-staging", "community-snapshot", "witness-running", "witness-returned")
  pure def controls(frame: str): Set[str] = if (Panels.contains(frame)) Set("YouCreateAnAccessGrant", "YouRevokeAnAccessGrant", "YouPublishYourCommunityProfile", "YouWithdrawYourCommunityProfile") else Set()
  var s: {
    panel: str, family: str, stage: str, actor: int, tenant: int, requestKey: int,
    authenticated: bool, familyCredential: bool, roleAllowed: bool, policyAllowed: bool,
    controlsPresent: bool, requestValid: bool, enabled: str -> bool,
    grant: int -> bool, grantOwner: int -> int, effects: Set[(str, int, int)],
    response: str, inFlight: bool, currentGeneration: int, requestGeneration: int,
    coarseOutcome: str -> str, token: int -> str, attachment: Set[int], tokenSubmission: Set[int], tokenReceipt: Set[int],
    profile: int -> str, snapshotDirty: bool, snapshotInvalidated: bool, snapshotFresh: bool, snapshotProfiles: Set[int], rosterAllowed: bool,
    analyticsAllowed: bool, snapshotHasAnalytics: bool, returnedAnalytics: bool,
    witnessBusy: bool, witnessSlots: int, witnessRequest: int, witnessPending: Set[int], witnessKind: int -> str, witnessNonceValid: int -> bool,
    witnessInferenceValid: int -> bool, witnessCertificate: Set[int], witnessQuotes: Set[int], error: str
  }
  val vars = { s: s }
  action init = s' = {
    panel: "interfaces-idle", family: "public_source", stage: "idle", actor: 0, tenant: 0, requestKey: 0,
    authenticated: false, familyCredential: false, roleAllowed: false, policyAllowed: false,
    controlsPresent: true, requestValid: true, enabled: Families.mapBy(_ => true),
    grant: Keys.mapBy(_ => true), grantOwner: Keys.mapBy(i => i), effects: Set(), response: "none", inFlight: false,
    currentGeneration: 0, requestGeneration: 0, coarseOutcome: Families.mapBy(_ => "unobserved"),
    token: Keys.mapBy(_ => "none"), attachment: Set(), tokenSubmission: Set(), tokenReceipt: Set(),
    profile: Actors.mapBy(_ => "private"), snapshotDirty: false, snapshotInvalidated: false, snapshotFresh: false, snapshotProfiles: Set(), rosterAllowed: true,
    analyticsAllowed: false, snapshotHasAnalytics: false, returnedAnalytics: false,
    witnessBusy: false, witnessSlots: 2, witnessRequest: -1, witnessPending: Set(), witnessKind: Keys.mapBy(_ => "attestation"), witnessNonceValid: Keys.mapBy(_ => true),
    witnessInferenceValid: Keys.mapBy(_ => true), witnessCertificate: Set(), witnessQuotes: Set(), error: "none"
  }
  // Environment supplies auth/ABAC/credential validation results. No cross-family credential is inferred.
  action TheWorldSendsAServiceRequest = {
    nondet f = Families.oneOf()
    nondet a = Actors.oneOf()
    nondet t = Actors.oneOf()
    nondet k = Keys.oneOf()
    nondet auth = Set(true, false).oneOf()
    nondet credential = Set(true, false).oneOf()
    nondet role = Set(true, false).oneOf()
    nondet policy = Set(true, false).oneOf()
    nondet valid = Set(true, false).oneOf()
    all { not(s.inFlight),
      s' = { ...s, family: f, actor: a, tenant: t, requestKey: k,
        authenticated: auth, familyCredential: credential, roleAllowed: role, policyAllowed: policy,
        requestValid: valid, requestGeneration: s.currentGeneration, stage: "received", inFlight: true,
        response: "none", panel: "interface-authorizing", error: "none" } }
  }
  action TheDeploymentFeatureConfigurationChanges = {
    nondet f = Families.oneOf()
    nondet featureOn = Set(true, false).oneOf()
    nondet controlsAvailable = Set(true, false).oneOf()
    s' = { ...s, enabled: s.enabled.set(f, featureOn), controlsPresent: controlsAvailable }
  }
  action ThePublicSourceOfferReturnsWithoutTenantAuthority = all { s.inFlight, s.stage == "received", s.family == "public_source",
    s' = { ...s, stage: "returned", inFlight: false, response: "source_offer", panel: "interface-returned" } }
  action TheAnonymousBoundaryAcceptsTheRequest = all { s.inFlight, s.stage == "received", s.family != "public_source", Anonymous.contains(s.family),
    s.enabled.get(s.family), s.controlsPresent, s.requestValid,
    s' = { ...s, stage: "authorized", panel: "interface-running" } }
  action TheProtectedBoundaryAcceptsTheRequest = all { s.inFlight, s.stage == "received", not(Anonymous.contains(s.family)),
    s.enabled.get(s.family), s.controlsPresent, s.requestValid, s.authenticated, s.familyCredential, s.roleAllowed, s.policyAllowed,
    s' = { ...s, stage: "authorized", panel: "interface-running" } }
  action TheBoundaryRefusesTheRequest = all { s.inFlight, s.stage == "received", s.family != "public_source",
    not(s.enabled.get(s.family)) or not(s.controlsPresent) or not(s.requestValid) or
      (not(Anonymous.contains(s.family)) and (not(s.authenticated) or not(s.familyCredential) or not(s.roleAllowed) or not(s.policyAllowed))),
    s' = { ...s, stage: "refused", inFlight: false, response: "refusal", error: "route_specific_refusal", panel: "interface-refused" } }
  // Coarse work has no invented shared queue/lease semantics: it merely crosses an accepted handler boundary.
  action TheServiceStartsItsCoarseWork = all { s.inFlight, s.stage == "authorized", not(Set("access_grant", "community_profile", "token_bundle").contains(s.family)),
    s' = { ...s, stage: "effect_pending", coarseOutcome: s.coarseOutcome.set(s.family, "running"), panel: "interface-running" } }
  action TheServiceReturnsACoarseReadResult = all { s.inFlight, Set("authorized", "effect_pending").contains(s.stage),
    s' = { ...s, stage: "returned", inFlight: false, response: "coarse_read_result", coarseOutcome: s.coarseOutcome.set(s.family, "read_returned"), panel: "interface-returned" } }
  action TheServiceCommitsACoarseDomainEffect = all { s.inFlight, s.stage == "effect_pending", not(ReadOnly.contains(s.family)),
    s' = { ...s, effects: s.effects.union(Set((s.family, s.tenant, s.requestKey))), stage: "effect_committed",
      coarseOutcome: s.coarseOutcome.set(s.family, "effect_committed"), panel: "interface-running" } }
  action TheServiceReturnsAfterItsEffect = all { s.inFlight, s.stage == "effect_committed",
    s' = { ...s, inFlight: false, stage: "returned", response: "domain_result", panel: "interface-returned" } }
  action TheServiceFailsDuringCoarseWork = all { s.inFlight, Set("authorized", "effect_pending", "effect_committed").contains(s.stage),
    s' = { ...s, inFlight: false, stage: "failed", coarseOutcome: s.coarseOutcome.set(s.family, "failed"), error: "service_or_store_unavailable", panel: "interface-refused" } }
  action TheCallerDisconnectsOrChangesAccount = all { s.inFlight,
    s' = { ...s, currentGeneration: 1 - s.currentGeneration } }
  action TheResponseIsLostAfterTheEffect = all { s.inFlight, s.stage == "effect_committed",
    s' = { ...s, inFlight: false, stage: "response_lost", response: "missing", panel: "interface-refused" } }
  action YouCreateAnAccessGrant = all { controls(s.panel).contains("YouCreateAnAccessGrant"), s.family == "access_grant", s.stage == "authorized",
    s' = { ...s, grant: s.grant.set(s.requestKey, true), grantOwner: s.grantOwner.set(s.requestKey, s.tenant), stage: "effect_committed", panel: "interface-running" } }
  action YouRevokeAnAccessGrant = all { controls(s.panel).contains("YouRevokeAnAccessGrant"), s.family == "access_grant", s.stage == "authorized", s.grantOwner.get(s.requestKey) == s.tenant,
    s' = { ...s, grant: s.grant.set(s.requestKey, false), stage: "effect_committed", panel: "interface-running" } }
  action YouPublishYourCommunityProfile = all { controls(s.panel).contains("YouPublishYourCommunityProfile"), s.family == "community_profile", s.stage == "authorized",
    s' = { ...s, profile: s.profile.set(s.actor, "public"), snapshotDirty: true, stage: "effect_committed", panel: "community-snapshot" } }
  action YouWithdrawYourCommunityProfile = all { controls(s.panel).contains("YouWithdrawYourCommunityProfile"), s.family == "community_profile", s.stage == "authorized",
    s' = { ...s, profile: s.profile.set(s.actor, "withdrawn"), snapshotDirty: true, snapshotInvalidated: true, stage: "effect_committed", panel: "community-snapshot" } }
  action TheCommunitySnapshotIsRecomputed = s' = { ...s, snapshotProfiles: Actors.filter(a => s.profile.get(a) == "public"),
    snapshotDirty: false, snapshotInvalidated: false, snapshotFresh: true, snapshotHasAnalytics: s.analyticsAllowed, panel: "community-snapshot" }
  action TheCommunityPublicationControlsChange = {
    nondet roster = Set(true, false).oneOf()
    nondet analytics = Set(true, false).oneOf()
    s' = { ...s, rosterAllowed: roster, analyticsAllowed: analytics }
  }
  action TheCommunityRosterReadRechecksAnalytics = all { s.rosterAllowed, s.snapshotFresh, not(s.snapshotInvalidated),
    s' = { ...s, returnedAnalytics: s.analyticsAllowed and s.snapshotHasAnalytics, response: "gated_snapshot", panel: "community-snapshot" } }
  action TheCommunitySnapshotBecomesStale = s' = { ...s, snapshotFresh: false }
  action TheCommunityReadIsWithheldForFreshness = all { not(s.snapshotFresh) or s.snapshotInvalidated,
    s' = { ...s, error: "community_snapshot_stale_or_pending_invalidation", returnedAnalytics: false, panel: "interface-refused" } }
  action TheCommunityAnalyticsReadNeedsARecompute = all { s.analyticsAllowed, not(s.snapshotHasAnalytics),
    s' = { ...s, returnedAnalytics: false, error: "snapshot_computed_without_analytics", panel: "interface-refused" } }
  action TheTokenBundleManifestBeginsStaging = all { s.inFlight, s.family == "token_bundle", s.stage == "authorized", s.token.get(s.requestKey) == "none",
    s' = { ...s, token: s.token.set(s.requestKey, "staging"), inFlight: false, stage: "returned", panel: "token-staging" } }
  action TheTokenBundleAttachmentsBecomeReady = {
    nondet k = Keys.oneOf()
    all { s.token.get(k) == "staging", s' = { ...s, attachment: s.attachment.union(Set(k)), panel: "token-staging" } }
  }
  // Submit acceptance is not the durable bundle receipt; rescrub/digest checks and receipt commit follow it.
  action TheTokenBundleFinalizeAcceptsTheSubmission = all { s.inFlight, s.family == "token_bundle", s.stage == "authorized",
    s.token.get(s.requestKey) == "staging", s.attachment.contains(s.requestKey),
    s' = { ...s, tokenSubmission: s.tokenSubmission.union(Set(s.requestKey)), stage: "effect_pending", panel: "token-staging" } }
  action TheTokenBundleDurableReceiptCommits = all { s.inFlight, s.family == "token_bundle", s.stage == "effect_pending", s.tokenSubmission.contains(s.requestKey),
    s' = { ...s, token: s.token.set(s.requestKey, "committed"), tokenReceipt: s.tokenReceipt.union(Set(s.requestKey)), stage: "returned", inFlight: false, panel: "interface-returned" } }
  action TheTokenBundlePostSubmitRevalidationFails = all { s.inFlight, s.family == "token_bundle", s.stage == "effect_pending", s.tokenSubmission.contains(s.requestKey),
    s' = { ...s, inFlight: false, stage: "failed", error: "token_bundle_rescrub_changed_or_store_failed", panel: "interface-refused" } }
  action TheTokenBundleIsRevokedOrExpires = {
    nondet k = Keys.oneOf()
    all { s.token.get(k) != "none", s' = { ...s, token: s.token.set(k, "revoked_or_expired"), attachment: s.attachment.exclude(Set(k)), panel: "token-staging" } }
  }
  // Separate unauthenticated witness service. Inputs are validation results, never asserted cryptographic proofs.
  action TheWorldRequestsWitnessEvidence = {
    nondet k = Keys.oneOf()
    nondet kind = Set("attestation", "contribution", "token_bundle", "admission").oneOf()
    nondet nonceValid = Set(true, false).oneOf()
    nondet inferenceValid = Set(true, false).oneOf()
    all { not(s.witnessPending.contains(k)), (kind == "attestation" or s.witnessSlots > 0),
      s' = { ...s, witnessBusy: true, witnessSlots: if (kind == "attestation") s.witnessSlots else s.witnessSlots - 1, witnessRequest: k,
        witnessPending: s.witnessPending.union(Set(k)), witnessKind: s.witnessKind.set(k, kind),
        witnessNonceValid: s.witnessNonceValid.set(k, nonceValid), witnessInferenceValid: s.witnessInferenceValid.set(k, inferenceValid), panel: "witness-running" } }
  }
  action TheWitnessReturnsValidatedEvidence = {
    nondet k = Keys.oneOf()
    all { s.witnessPending.contains(k),
      (if (s.witnessKind.get(k) == "attestation") s.witnessNonceValid.get(k) else s.witnessInferenceValid.get(k)),
      s' = { ...s, witnessBusy: s.witnessPending.size() > 1, witnessSlots: if (s.witnessKind.get(k) == "attestation") s.witnessSlots else s.witnessSlots + 1,
        witnessPending: s.witnessPending.exclude(Set(k)),
        witnessCertificate: if (s.witnessKind.get(k) != "attestation") s.witnessCertificate.union(Set(k)) else s.witnessCertificate,
        witnessQuotes: if (s.witnessKind.get(k) == "attestation") s.witnessQuotes.union(Set(k)) else s.witnessQuotes, panel: "witness-returned" } }
  }
  action TheWitnessRejectsEvidenceOrTimesOut = {
    nondet k = Keys.oneOf()
    all { s.witnessPending.contains(k),
      s' = { ...s, witnessBusy: s.witnessPending.size() > 1, witnessSlots: if (s.witnessKind.get(k) == "attestation") s.witnessSlots else s.witnessSlots + 1,
        witnessPending: s.witnessPending.exclude(Set(k)), error: "witness_validation_or_timeout", panel: "interface-refused" } }
  }
  action TheWitnessLoadBoundRefusesAnotherRequest = all { s.witnessSlots == 0,
    s' = { ...s, error: "witness_saturated", panel: "interface-refused" } }
  action step = any {
    TheWorldSendsAServiceRequest, TheDeploymentFeatureConfigurationChanges, ThePublicSourceOfferReturnsWithoutTenantAuthority,
    TheAnonymousBoundaryAcceptsTheRequest, TheProtectedBoundaryAcceptsTheRequest, TheBoundaryRefusesTheRequest,
    TheServiceStartsItsCoarseWork, TheServiceReturnsACoarseReadResult, TheServiceCommitsACoarseDomainEffect,
    TheServiceReturnsAfterItsEffect, TheServiceFailsDuringCoarseWork, TheCallerDisconnectsOrChangesAccount,
    TheResponseIsLostAfterTheEffect, YouCreateAnAccessGrant, YouRevokeAnAccessGrant,
    YouPublishYourCommunityProfile, YouWithdrawYourCommunityProfile, TheCommunitySnapshotIsRecomputed,
    TheCommunityPublicationControlsChange, TheCommunityRosterReadRechecksAnalytics, TheCommunitySnapshotBecomesStale,
    TheCommunityReadIsWithheldForFreshness, TheCommunityAnalyticsReadNeedsARecompute,
    TheTokenBundleManifestBeginsStaging, TheTokenBundleAttachmentsBecomeReady, TheTokenBundleFinalizeAcceptsTheSubmission,
    TheTokenBundleDurableReceiptCommits, TheTokenBundlePostSubmitRevalidationFails, TheTokenBundleIsRevokedOrExpires,
    TheWorldRequestsWitnessEvidence, TheWitnessReturnsValidatedEvidence, TheWitnessRejectsEvidenceOrTimesOut,
    TheWitnessLoadBoundRefusesAnotherRequest
  }
}
