# Extended-model negative-control checker reports

These deliberate mutations violate the named selected promise; they are not reproduced product defects.

## SkillsDisabled

```text
# FLOW

TLC reached 75,264 states, more than the chart budget of 500, so the chart draws only the bug paths: 3 states and 2 arrows.

## SkillsDisabled breaks
The shortest path has 2 steps:
0. featureOn=TRUE usage="off" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=FALSE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
1. TheSkillsFlagTurnsOff: featureOn=FALSE usage="off" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=FALSE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
2. ThePeriodicScanStarts: featureOn=FALSE usage="off" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="current" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=FALSE startedDisabled=TRUE call="absent" preview=FALSE childAllowed=TRUE

## Rules that hold
NoDeadlock
```

## ExcludedFolders

```text
# FLOW

TLC reached 99,296 states, more than the chart budget of 500, so the chart draws only the bug paths: 4 states and 3 arrows.

## ExcludedFolders breaks
The shortest path has 3 steps:
0. featureOn=TRUE usage="off" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=FALSE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
1. YouPressRefresh: featureOn=TRUE usage="off" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="current" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=FALSE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
2. TheScanFindsSkills: featureOn=TRUE usage="off" extract="idle" accepted=0 inventory="ready" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=FALSE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
3. YouExcludeTheFolder: featureOn=TRUE usage="off" extract="idle" accepted=0 inventory="ready" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=FALSE evidence="none" child="missing" analysis=FALSE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE

## Rules that hold
NoDeadlock
```

## OldResultsStayRemoved

```text
# FLOW

TLC reached more states than the chart budget of 500, so the chart draws only the bug paths: 4 states and 3 arrows.

## OldResultsStayRemoved breaks
The shortest path has 3 steps:
0. featureOn=TRUE usage="off" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=FALSE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
1. YouPressRefresh: featureOn=TRUE usage="off" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="current" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=FALSE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
2. TheSkillsFlagTurnsOff: featureOn=FALSE usage="off" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="stale" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=FALSE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
3. TheOldScanReturns: featureOn=FALSE usage="off" extract="idle" accepted=0 inventory="ready" restoredStale=TRUE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=FALSE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE

## Rules not checked
NoDeadlock
After the first break, each TLC run stops at 100,000 distinct states. A run reached that bound before it checked this rule in every state.
```

## AnalysisOff

```text
# FLOW

TLC reached 76,512 states, more than the chart budget of 500, so the chart draws only the bug paths: 3 states and 2 arrows.

## AnalysisOff breaks
The shortest path has 2 steps:
0. featureOn=TRUE usage="off" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=FALSE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
1. YouEnableLocalAnalysis: featureOn=TRUE usage="pending" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=TRUE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
2. YouDisableLocalAnalysis: featureOn=TRUE usage="pending" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=FALSE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE

## Rules that hold
NoDeadlock
```

## OnlyObservedUse

```text
# FLOW

TLC reached 96,968 states, more than the chart budget of 500, so the chart draws only the bug paths: 4 states and 3 arrows.

## OnlyObservedUse breaks
The shortest path has 3 steps:
0. featureOn=TRUE usage="off" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=FALSE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
1. YouEnableLocalAnalysis: featureOn=TRUE usage="pending" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=TRUE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
2. TheAnalysisStarts: featureOn=TRUE usage="pending" extract="current" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=TRUE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
3. AFailedReadIsObserved: featureOn=TRUE usage="pending" extract="current" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="failed_read" child="missing" analysis=TRUE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE

## Rules that hold
NoDeadlock
```

## OnlyObservedUse_OpenCodeRunning

```text
# FLOW

TLC reached 74,288 states, more than the chart budget of 500, so the chart draws only the bug paths: 4 states and 3 arrows.

## OnlyObservedUse breaks
The shortest path has 3 steps:
0. featureOn=TRUE usage="off" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=FALSE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
1. YouEnableLocalAnalysis: featureOn=TRUE usage="pending" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=TRUE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
2. TheAnalysisStarts: featureOn=TRUE usage="pending" extract="current" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=TRUE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
3. TheOpenCodeCallIsRunning: featureOn=TRUE usage="pending" extract="current" accepted=1 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=TRUE startedDisabled=FALSE call="running" preview=FALSE childAllowed=TRUE

## Rules that hold
NoDeadlock
```

## ExcludedFolders_Child

```text
# FLOW

TLC reached more states than the chart budget of 500, so the chart draws only the bug paths: 6 states and 5 arrows.

## ExcludedFolders breaks
The shortest path has 5 steps:
0. featureOn=TRUE usage="off" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="missing" analysis=FALSE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
1. AChildTraceBecomesAvailable: featureOn=TRUE usage="off" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="available" analysis=FALSE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
2. YouEnableLocalAnalysis: featureOn=TRUE usage="pending" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="available" analysis=TRUE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
3. TheChildAnalysisStarts: featureOn=TRUE usage="pending" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="none" detail="idle" allowed=TRUE evidence="none" child="current" analysis=TRUE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
4. TheChildInvocationIsObserved: featureOn=TRUE usage="pending" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="invocation" detail="idle" allowed=TRUE evidence="none" child="current" analysis=TRUE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE
5. YouExcludeTheFolder: featureOn=TRUE usage="pending" extract="idle" accepted=0 inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" childEvidence="invocation" detail="idle" allowed=FALSE evidence="none" child="stale" analysis=TRUE startedDisabled=FALSE call="absent" preview=FALSE childAllowed=TRUE

## Rules not checked
NoDeadlock
After the first break, each TLC run stops at 100,000 distinct states. A run reached that bound before it checked this rule in every state.
```
