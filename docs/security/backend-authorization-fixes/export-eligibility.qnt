// Authorization decision after source selection and awaited provenance work.
// Cryptography, all body/consent checks and last-byte delivery are outside this model.
module export_eligibility {
  var s: {tenantMatches: bool, exportRole: bool, allowedUse: bool,
    fileAccepted: bool, fileExpired: bool, dbAccepted: bool, dbExpired: bool,
    dbAuthority: bool, mirrorPresent: bool, kind: str,
    completed: bool, completedWhileIneligible: bool}
  val vars = s
  val authoritativeAccepted = if (s.dbAuthority) s.dbAccepted else s.fileAccepted
  val authoritativeExpired = if (s.dbAuthority) s.dbExpired else s.fileExpired
  val sourceEligible = authoritativeAccepted and not(authoritativeExpired)
    and (not(s.dbAuthority) or s.mirrorPresent)
  val provenanceEligible = s.kind != "derived" or not(s.mirrorPresent)
    or (s.dbAccepted and not(s.dbExpired))
  val canExport = s.tenantMatches and s.exportRole and s.allowedUse
    and sourceEligible and provenanceEligible
  val NoIneligibleExport = not(s.completedWhileIneligible)
  action configure(db: bool, mirror: bool, kind: str): bool = s' = {
    tenantMatches: true, exportRole: true, allowedUse: true,
    fileAccepted: true, fileExpired: false, dbAccepted: true, dbExpired: false,
    dbAuthority: db, mirrorPresent: mirror, kind: kind,
    completed: false, completedWhileIneligible: false}
  action init = {
    nondet db = Set(false, true).oneOf()
    nondet mirror = Set(false, true).oneOf()
    nondet kind = Set("replay", "derived").oneOf()
    configure(db, mirror, kind)
  }
  // Maintenance/mirror lag can leave accepted status or the other copy unchanged.
  action TheFileDeadlinePasses = s' = {...s, fileExpired: true}
  action TheDbDeadlinePasses = s' = {...s, dbExpired: true}
  action TheFileIsRevoked = s' = {...s, fileAccepted: false}
  action TheDbIsRevoked = s' = {...s, dbAccepted: false}
  action TheAuthorizedExporterDownloads = all {canExport, not(s.completed),
    s' = {...s, completed: true, completedWhileIneligible: not(sourceEligible)} }
  action step = any {TheFileDeadlinePasses, TheDbDeadlinePasses,
    TheFileIsRevoked, TheDbIsRevoked, TheAuthorizedExporterDownloads}

  // Proof-only historical controls: excluded from the repaired full step.
  // They ensure this model can expose both original and review-found defects.
  action OldMirrorPresenceChoosesAuthority = all {not(s.completed),
    if (s.mirrorPresent) s.dbAccepted and not(s.dbExpired)
      else s.fileAccepted and not(s.fileExpired),
    s' = {...s, completed: true, completedWhileIneligible: not(sourceEligible)} }
  action OldAcceptedStatusIgnoresDeadline = all {not(s.completed), authoritativeAccepted,
    s' = {...s, completed: true, completedWhileIneligible: not(sourceEligible)} }

  run fileExpiryCannotHideBehindMirrorTest = configure(false, true, "derived")
    .then(TheFileDeadlinePasses).expect(not(canExport))
  run fileRevocationCannotHideBehindMirrorTest = configure(false, true, "derived")
    .then(TheFileIsRevoked).expect(not(canExport))
  run replayUsesFileAuthorityTest = configure(false, true, "replay")
    .then(TheDbDeadlinePasses).expect(canExport)
    .then(TheFileDeadlinePasses).expect(not(canExport))
  run databaseAuthorityRemainsDatabaseTest = configure(true, true, "derived")
    .then(TheFileDeadlinePasses).expect(canExport)
    .then(TheDbDeadlinePasses).expect(not(canExport))
  run missingDatabaseAuthorityRefusesTest = configure(true, false, "derived")
    .expect(not(canExport))
  run deliveredBeforeExpiryRemainsHistoricalTest = configure(false, false, "replay")
    .then(TheAuthorizedExporterDownloads).then(TheFileDeadlinePasses)
    .expect(NoIneligibleExport)
  run historicalWrongAuthorityCounterexampleTest = configure(false, true, "derived")
    .then(TheFileDeadlinePasses).then(OldMirrorPresenceChoosesAuthority)
    .expect(not(NoIneligibleExport))
  run historicalMissingDeadlineCounterexampleTest = configure(false, false, "replay")
    .then(TheFileDeadlinePasses).then(OldAcceptedStatusIgnoresDeadline)
    .expect(not(NoIneligibleExport))
}
