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

  var child: str
  var childAllowed: bool
  var childEvidence: str
  var call: str
  var accepted: int

  pure def controls(on: bool): Set[str] = if (on) Set("Refresh", "Read definition") else Set()
  pure def shows(c: str, on: bool): bool = c.in(controls(on))

  action init = all { featureOn' = true, allowed' = true, analysis' = false, inventory' = "none", usage' = "off", scan' = "idle", extract' = "idle", detail' = "idle", preview' = false, fileCurrent' = true, evidence' = "none", startedDisabled' = false, restoredStale' = false, child' = "missing", childAllowed' = true, childEvidence' = "none", call' = "absent", accepted' = 0 }

  action TheSkillsFlagTurnsOff = all {
    featureOn,
    featureOn' = false, allowed' = allowed, analysis' = analysis, inventory' = "none", usage' = "off", scan' = if (scan == "idle") "idle" else "stale", extract' = if (extract == "idle") "idle" else "stale", detail' = if (detail == "idle") "idle" else "stale", preview' = false, fileCurrent' = fileCurrent, evidence' = "none", startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = if (child == "current" or child == "stale") "stale" else "missing", childAllowed' = childAllowed, childEvidence' = "none", call' = "absent", accepted' = 0
  }
  action TheSkillsFlagTurnsOn = all {
    not(featureOn),
    featureOn' = true, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = if (analysis) "pending" else "off", scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action YouExcludeTheFolder = all {
    allowed,
    featureOn' = featureOn, allowed' = false, analysis' = analysis, inventory' = "none", usage' = if (analysis and featureOn) "pending" else "off", scan' = if (scan == "idle") "idle" else "stale", extract' = if (extract == "idle") "idle" else "stale", detail' = if (detail == "idle") "idle" else "stale", preview' = false, fileCurrent' = fileCurrent, evidence' = "none", startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = if (child == "current" or child == "stale") "stale" else "missing", childAllowed' = childAllowed, childEvidence' = "none", call' = "absent", accepted' = 0
  }
  action YouPermitTheFolder = all {
    not(allowed),
    featureOn' = featureOn, allowed' = true, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action YouEnableLocalAnalysis = all {
    not(analysis),
    featureOn' = featureOn, allowed' = allowed, analysis' = true, inventory' = inventory, usage' = if (featureOn) "pending" else "off", scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action YouDisableLocalAnalysis = all {
    analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = false, inventory' = inventory, usage' = "off", scan' = scan, extract' = if (extract == "idle") "idle" else "stale", detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = "none", startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = if (child == "current" or child == "stale") "stale" else "missing", childAllowed' = childAllowed, childEvidence' = "none", call' = "absent", accepted' = 0
  }
  action YouClearLocalAnalysis = all {
    true,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = if (analysis and featureOn) "pending" else "off", scan' = scan, extract' = if (extract == "idle") "idle" else "stale", detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = "none", startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = if (child == "current" or child == "stale") "stale" else "missing", childAllowed' = childAllowed, childEvidence' = "none", call' = "absent", accepted' = 0
  }
  action YouPressRefresh = all {
    shows("Refresh", featureOn) and allowed and scan == "idle",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = "current", extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = not(featureOn), restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action ThePeriodicScanStarts = all {
    featureOn and allowed and scan == "idle",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = "current", extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = not(featureOn), restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheScanFindsSkills = all {
    scan == "current" and featureOn and allowed,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = "ready", usage' = usage, scan' = "idle", extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = (restoredStale or scan == "stale"), child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheScanFindsNoSkills = all {
    scan == "current" and featureOn and allowed,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = "empty", usage' = usage, scan' = "idle", extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = (restoredStale or scan == "stale"), child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheScanIsIncomplete = all {
    scan == "current" and featureOn and allowed,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = "partial", usage' = usage, scan' = "idle", extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = (restoredStale or scan == "stale"), child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheOldScanReturns = all {
    scan == "stale",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = "idle", extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheAnalysisStarts = all {
    featureOn and allowed and analysis and extract == "idle",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = "current", detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = not(featureOn), restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheAnalysisCompletes = all {
    extract == "current" and featureOn and allowed and analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = "complete", scan' = scan, extract' = "idle", detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = (restoredStale or extract == "stale"), child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheAnalysisIsPartial = all {
    extract == "current" and featureOn and allowed and analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = "partial", scan' = scan, extract' = "idle", detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = (restoredStale or extract == "stale"), child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheFormatIsUnsupported = all {
    extract == "current" and featureOn and allowed and analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = "unsupported", scan' = scan, extract' = "idle", detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = "none", startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheOldAnalysisReturns = all {
    extract == "stale",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = "idle", detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action AnInvocationIsObserved = all {
    extract == "current" and featureOn and allowed and analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = "invocation", startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action ASuccessfulLoadIsObserved = all {
    extract == "current" and featureOn and allowed and analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = "load", startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action AFailedReadIsObserved = all {
    extract == "current" and featureOn and allowed and analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action ACatalogListingIsObserved = all {
    extract == "current" and featureOn and allowed and analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action YouReadTheDefinition = all {
    shows("Read definition", featureOn) and allowed and inventory != "none" and inventory != "empty" and detail == "idle",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = "current", preview' = false, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = not(featureOn), restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheDefinitionReturns = all {
    detail == "current" and featureOn and allowed and fileCurrent,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = "idle", preview' = true, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = (restoredStale or detail == "stale"), child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheDefinitionIsUnavailable = all {
    detail == "current",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = "idle", preview' = false, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheOldDefinitionReturns = all {
    detail == "stale",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = "idle", preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheDefinitionChanges = all {
    true,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = if (detail == "idle") "idle" else "stale", preview' = false, fileCurrent' = false, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheNewFingerprintIsObserved = all {
    not(fileCurrent),
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = true, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action YouLeaveTheDetail = all {
    detail != "idle" or preview,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = if (detail == "idle") "idle" else "stale", preview' = false, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheWorldWaits = all {
    true,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action AChildTraceBecomesAvailable = all {
    child == "missing" and childAllowed and allowed,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = "available", childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheChildAnalysisStarts = all {
    featureOn and allowed and childAllowed and analysis and child.in(Set("available", "ready", "partial")),
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = not(featureOn), restoredStale' = restoredStale, child' = "current", childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheChildInvocationIsObserved = all {
    featureOn and allowed and childAllowed and analysis and child == "current",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = "invocation", call' = call, accepted' = accepted
  }
  action TheChildDefinitionLoadIsObserved = all {
    featureOn and allowed and childAllowed and analysis and child == "current",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = "load", call' = call, accepted' = accepted
  }
  action TheChildAnalysisCompletes = all {
    featureOn and allowed and childAllowed and analysis and child == "current",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = (restoredStale or child == "stale"), child' = "ready", childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheChildAnalysisIsPartial = all {
    featureOn and allowed and childAllowed and analysis and child == "current",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = (restoredStale or child == "stale"), child' = "partial", childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheOldChildAnalysisReturns = all {
    child == "stale",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = "missing", childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action YouExcludeTheChildFolder = all {
    childAllowed,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = if (child == "current" or child == "stale") "stale" else "missing", childAllowed' = false, childEvidence' = "none", call' = call, accepted' = accepted
  }
  action YouPermitTheChildFolder = all {
    not(childAllowed),
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = true, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action TheChildTraceDisappears = all {
    child != "missing",
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = if (child == "current" or child == "stale") "stale" else "missing", childAllowed' = childAllowed, childEvidence' = "none", call' = call, accepted' = accepted
  }
  action TheOpenCodeCallIsRunning = all {
    extract == "current" and featureOn and allowed and analysis and accepted == 0,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = "running", accepted' = accepted
  }
  action TheOpenCodeCallFails = all {
    extract == "current" and featureOn and allowed and analysis and accepted == 0,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = "failed", accepted' = accepted
  }
  action TheDatabaseReportsSuccessfulInvocation = all {
    extract == "current" and featureOn and allowed and analysis and call.in(Set("absent", "running", "invocation")),
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = "invocation", accepted' = 1
  }
  action TheDatabaseReportsSuccessfulLoad = all {
    extract == "current" and featureOn and allowed and analysis and call.in(Set("absent", "running", "load")),
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = "load", accepted' = 1
  }
  action TheExportReportsSuccessfulInvocation = all {
    extract == "current" and featureOn and allowed and analysis and call.in(Set("absent", "running", "invocation")),
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = "invocation", accepted' = 1
  }
  action TheExportReportsSuccessfulLoad = all {
    extract == "current" and featureOn and allowed and analysis and call.in(Set("absent", "running", "load")),
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = "load", accepted' = 1
  }
  action TheOpenCodeCatalogIsObserved = all {
    extract == "current" and featureOn and allowed and analysis,
    featureOn' = featureOn, allowed' = allowed, analysis' = analysis, inventory' = inventory, usage' = usage, scan' = scan, extract' = extract, detail' = detail, preview' = preview, fileCurrent' = fileCurrent, evidence' = evidence, startedDisabled' = startedDisabled, restoredStale' = restoredStale, child' = child, childAllowed' = childAllowed, childEvidence' = childEvidence, call' = call, accepted' = accepted
  }
  action step = any { AChildTraceBecomesAvailable, TheChildAnalysisStarts, TheChildInvocationIsObserved, TheChildDefinitionLoadIsObserved, TheChildAnalysisCompletes, TheChildAnalysisIsPartial, TheOldChildAnalysisReturns, YouExcludeTheChildFolder, YouPermitTheChildFolder, TheChildTraceDisappears, TheOpenCodeCallIsRunning, TheOpenCodeCallFails, TheDatabaseReportsSuccessfulInvocation, TheDatabaseReportsSuccessfulLoad, TheExportReportsSuccessfulInvocation, TheExportReportsSuccessfulLoad, TheOpenCodeCatalogIsObserved, TheSkillsFlagTurnsOff, TheSkillsFlagTurnsOn, YouExcludeTheFolder, YouPermitTheFolder, YouEnableLocalAnalysis, YouDisableLocalAnalysis, YouClearLocalAnalysis, YouPressRefresh, ThePeriodicScanStarts, TheScanFindsSkills, TheScanFindsNoSkills, TheScanIsIncomplete, TheOldScanReturns, TheAnalysisStarts, TheAnalysisCompletes, TheAnalysisIsPartial, TheFormatIsUnsupported, TheOldAnalysisReturns, AnInvocationIsObserved, ASuccessfulLoadIsObserved, AFailedReadIsObserved, ACatalogListingIsObserved, YouReadTheDefinition, TheDefinitionReturns, TheDefinitionIsUnavailable, TheOldDefinitionReturns, TheDefinitionChanges, TheNewFingerprintIsObserved, YouLeaveTheDetail, TheWorldWaits }

  /// Disabling Skills hides its UI and prevents new work.
  val SkillsDisabled = not(featureOn) implies (inventory == "none" and not(preview) and not(startedDisabled) and childEvidence == "none" and accepted == 0)
  /// Excluded folders never appear in results.
  val ExcludedFolders = (not(allowed) implies (inventory == "none" and not(preview) and evidence == "none" and childEvidence == "none" and accepted == 0)) and (not(childAllowed) implies childEvidence == "none")
  /// Results from before a clear or permission change cannot restore removed data.
  val OldResultsStayRemoved = not(restoredStale)
  /// Analysis off still permits inventory but produces no usage.
  val AnalysisOff = not(analysis) implies (usage == "off" and evidence == "none" and childEvidence == "none" and accepted == 0)
  /// Failed reads and catalog listings never count as use.
  val OnlyObservedUse = evidence.in(Set("none", "invocation", "load")) and childEvidence.in(Set("none", "invocation", "load")) and (accepted > 0 implies call.in(Set("invocation", "load")))
}
