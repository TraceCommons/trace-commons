// Bounded design model; see README for abstraction and implementation gaps.
module FLOW {
  var featureOn: bool
  var allowed: bool
  var analysis: bool
  var inventory: str
  var usage: str
  var scan: str
  var extract: str
  var detail: str
  var preview: bool
  var fileCurrent: bool
  var evidence: str
  var startedDisabled: bool
  var restoredStale: bool

  pure def controls(on: bool): Set[str] = if (on) Set("Refresh", "Read definition") else Set()
  pure def shows(c: str, on: bool): bool = c.in(controls(on))

  action init = all { featureOn' = true, allowed' = true, analysis' = false, inventory' = "none", usage' = "off", scan' = "idle", extract' = "idle", detail' = "idle", preview' = false, fileCurrent' = true, evidence' = "none", startedDisabled' = false, restoredStale' = false }

  action TheSkillsFlagTurnsOff = all {
    featureOn,
    featureOn' = false, allowed' = allowed, analysis' = analysis, inventory' = "none", usage' = "off", scan' = if (scan == "idle") "idle" else "stale", extract' = if (extract == "idle") "idle" else "stale", detail' = if (detail == "idle") "idle" else "stale", preview' = false, fileCurrent' = fileCurrent, evidence' = "none", startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action TheSkillsFlagTurnsOn = all {
    not(featureOn),
    featureOn' = true, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = if (analysis) "pending" else "off", scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action YouExcludeTheFolder = all {
    allowed,
    featureOn' = featureOn, allowed' = false, analysis' = analysis, inventory' = "none", usage' = if (analysis and featureOn) "pending" else "off", scan' = if (scan == "idle") "idle" else "stale", extract' = if (extract == "idle") "idle" else "stale", detail' = if (detail == "idle") "idle" else "stale", preview' = false, fileCurrent' = fileCurrent, evidence' = "none", startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action YouPermitTheFolder = all {
    not(allowed),
    featureOn' = featureOn, allowed' = true, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action YouEnableLocalAnalysis = all {
    not(analysis),
    featureOn' = featureOn, allowed' = allowed, analysis' = true, inventory' = inventory, usage' = if (featureOn) "pending" else "off", scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action YouDisableLocalAnalysis = all {
    analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = false, inventory' = inventory, usage' = "off", scan' = scan, extract' = if (extract == "idle") "idle" else "stale", detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = "none", startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action YouClearLocalAnalysis = all {
    true,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = if (analysis and featureOn) "pending" else "off", scan' = scan, extract' = if (extract == "idle") "idle" else "stale", detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = "none", startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action YouPressRefresh = all {
    shows("Refresh", featureOn) and allowed and scan == "idle",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = "current", extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = not(featureOn), restoredStale' = restoredStale
  }
  action ThePeriodicScanStarts = all {
    featureOn and allowed and scan == "idle",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = "current", extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = not(featureOn), restoredStale' = restoredStale
  }
  action TheScanFindsSkills = all {
    scan == "current" and featureOn and allowed,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = "ready", usage' = usage, scan' = "idle", extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = (restoredStale or scan == "stale")
  }
  action TheScanFindsNoSkills = all {
    scan == "current" and featureOn and allowed,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = "empty", usage' = usage, scan' = "idle", extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = (restoredStale or scan == "stale")
  }
  action TheScanIsIncomplete = all {
    scan == "current" and featureOn and allowed,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = "partial", usage' = usage, scan' = "idle", extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = (restoredStale or scan == "stale")
  }
  action TheOldScanReturns = all {
    scan == "stale",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = "idle", extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action TheAnalysisStarts = all {
    featureOn and allowed and analysis and extract == "idle",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = "current", detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = not(featureOn), restoredStale' = restoredStale
  }
  action TheAnalysisCompletes = all {
    extract == "current" and featureOn and allowed and analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = "complete", scan' = scan, extract' = "idle", detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = (restoredStale or extract == "stale")
  }
  action TheAnalysisIsPartial = all {
    extract == "current" and featureOn and allowed and analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = "partial", scan' = scan, extract' = "idle", detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = (restoredStale or extract == "stale")
  }
  action TheFormatIsUnsupported = all {
    extract == "current" and featureOn and allowed and analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = "unsupported", scan' = scan, extract' = "idle", detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = "none", startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action TheOldAnalysisReturns = all {
    extract == "stale",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = "idle", detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action AnInvocationIsObserved = all {
    extract == "current" and featureOn and allowed and analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = "invocation", startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action ASuccessfulLoadIsObserved = all {
    extract == "current" and featureOn and allowed and analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = "load", startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action AFailedReadIsObserved = all {
    extract == "current" and featureOn and allowed and analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action ACatalogListingIsObserved = all {
    extract == "current" and featureOn and allowed and analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action YouReadTheDefinition = all {
    shows("Read definition", featureOn) and allowed and inventory != "none" and inventory != "empty" and detail == "idle",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = "current", preview' = false, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = not(featureOn), restoredStale' = restoredStale
  }
  action TheDefinitionReturns = all {
    detail == "current" and featureOn and allowed and fileCurrent,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = "idle", preview' = true, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = (restoredStale or detail == "stale")
  }
  action TheDefinitionIsUnavailable = all {
    detail == "current",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = "idle", preview' = false, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action TheOldDefinitionReturns = all {
    detail == "stale",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = "idle", preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action TheDefinitionChanges = all {
    true,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = if (detail == "idle") "idle" else "stale", preview' = false, fileCurrent' = false, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action TheNewFingerprintIsObserved = all {
    not(fileCurrent),
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = true, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action YouLeaveTheDetail = all {
    detail != "idle" or preview,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = if (detail == "idle") "idle" else "stale", preview' = false, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action TheWorldWaits = all {
    true,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale
  }
  action step = any { TheSkillsFlagTurnsOff, TheSkillsFlagTurnsOn, YouExcludeTheFolder, YouPermitTheFolder, YouEnableLocalAnalysis, YouDisableLocalAnalysis, YouClearLocalAnalysis, YouPressRefresh, ThePeriodicScanStarts, TheScanFindsSkills, TheScanFindsNoSkills, TheScanIsIncomplete, TheOldScanReturns, TheAnalysisStarts, TheAnalysisCompletes, TheAnalysisIsPartial, TheFormatIsUnsupported, TheOldAnalysisReturns, AnInvocationIsObserved, ASuccessfulLoadIsObserved, AFailedReadIsObserved, ACatalogListingIsObserved, YouReadTheDefinition, TheDefinitionReturns, TheDefinitionIsUnavailable, TheOldDefinitionReturns, TheDefinitionChanges, TheNewFingerprintIsObserved, YouLeaveTheDetail, TheWorldWaits }

  /// Disabling Skills hides its UI and prevents new work.
  val SkillsDisabled = not(featureOn) implies (inventory == "none" and not(preview) and not(startedDisabled))
  /// Excluded folders never appear in results.
  val ExcludedFolders = not(allowed) implies (inventory == "none" and not(preview) and evidence == "none")
  /// Results from before a clear or permission change cannot restore removed data.
  val OldResultsStayRemoved = not(restoredStale)
  /// Analysis off still permits inventory but produces no usage.
  val AnalysisOff = not(analysis) implies (usage == "off" and evidence == "none")
  /// Failed reads and catalog listings never count as use.
  val OnlyObservedUse = evidence.in(Set("none", "invocation", "load"))
}
