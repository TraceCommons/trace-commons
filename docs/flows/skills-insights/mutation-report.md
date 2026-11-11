# Negative-control checker reports

Each mutation deliberately violates its named promise. These are not product defects.

## SkillsDisabled

```text
# FLOW

TLC reached 1,546 states, more than the chart budget of 500, so the chart draws only the bug paths: 3 states and 2 arrows.

## SkillsDisabled breaks
The shortest path has 2 steps:
0. featureOn=TRUE usage="off" extract="idle" inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" detail="idle" allowed=TRUE evidence="none" analysis=FALSE startedDisabled=FALSE preview=FALSE
1. TheSkillsFlagTurnsOff: featureOn=FALSE usage="off" extract="idle" inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" detail="idle" allowed=TRUE evidence="none" analysis=FALSE startedDisabled=FALSE preview=FALSE
2. ThePeriodicScanStarts: featureOn=FALSE usage="off" extract="idle" inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="current" detail="idle" allowed=TRUE evidence="none" analysis=FALSE startedDisabled=TRUE preview=FALSE

## Rules that hold
NoDeadlock
```

## ExcludedFolders

```text
# FLOW

TLC reached 1,848 states, more than the chart budget of 500, so the chart draws only the bug paths: 4 states and 3 arrows.

## ExcludedFolders breaks
The shortest path has 3 steps:
0. featureOn=TRUE usage="off" extract="idle" inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" detail="idle" allowed=TRUE evidence="none" analysis=FALSE startedDisabled=FALSE preview=FALSE
1. YouPressRefresh: featureOn=TRUE usage="off" extract="idle" inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="current" detail="idle" allowed=TRUE evidence="none" analysis=FALSE startedDisabled=FALSE preview=FALSE
2. TheScanFindsSkills: featureOn=TRUE usage="off" extract="idle" inventory="ready" restoredStale=FALSE fileCurrent=TRUE scan="idle" detail="idle" allowed=TRUE evidence="none" analysis=FALSE startedDisabled=FALSE preview=FALSE
3. YouExcludeTheFolder: featureOn=TRUE usage="off" extract="idle" inventory="ready" restoredStale=FALSE fileCurrent=TRUE scan="idle" detail="idle" allowed=FALSE evidence="none" analysis=FALSE startedDisabled=FALSE preview=FALSE

## Rules that hold
NoDeadlock
```

## OldResultsStayRemoved

```text
# FLOW

TLC reached 2,724 states, more than the chart budget of 500, so the chart draws only the bug paths: 4 states and 3 arrows.

## OldResultsStayRemoved breaks
The shortest path has 3 steps:
0. featureOn=TRUE usage="off" extract="idle" inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" detail="idle" allowed=TRUE evidence="none" analysis=FALSE startedDisabled=FALSE preview=FALSE
1. YouPressRefresh: featureOn=TRUE usage="off" extract="idle" inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="current" detail="idle" allowed=TRUE evidence="none" analysis=FALSE startedDisabled=FALSE preview=FALSE
2. TheSkillsFlagTurnsOff: featureOn=FALSE usage="off" extract="idle" inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="stale" detail="idle" allowed=TRUE evidence="none" analysis=FALSE startedDisabled=FALSE preview=FALSE
3. TheOldScanReturns: featureOn=FALSE usage="off" extract="idle" inventory="ready" restoredStale=TRUE fileCurrent=TRUE scan="idle" detail="idle" allowed=TRUE evidence="none" analysis=FALSE startedDisabled=FALSE preview=FALSE

## Rules that hold
NoDeadlock
```

## AnalysisOff

```text
# FLOW

TLC reached 1,786 states, more than the chart budget of 500, so the chart draws only the bug paths: 3 states and 2 arrows.

## AnalysisOff breaks
The shortest path has 2 steps:
0. featureOn=TRUE usage="off" extract="idle" inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" detail="idle" allowed=TRUE evidence="none" analysis=FALSE startedDisabled=FALSE preview=FALSE
1. YouEnableLocalAnalysis: featureOn=TRUE usage="pending" extract="idle" inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" detail="idle" allowed=TRUE evidence="none" analysis=TRUE startedDisabled=FALSE preview=FALSE
2. YouDisableLocalAnalysis: featureOn=TRUE usage="pending" extract="idle" inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" detail="idle" allowed=TRUE evidence="none" analysis=FALSE startedDisabled=FALSE preview=FALSE

## Rules that hold
NoDeadlock
```

## OnlyObservedUse

```text
# FLOW

TLC reached 1,662 states, more than the chart budget of 500, so the chart draws only the bug paths: 4 states and 3 arrows.

## OnlyObservedUse breaks
The shortest path has 3 steps:
0. featureOn=TRUE usage="off" extract="idle" inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" detail="idle" allowed=TRUE evidence="none" analysis=FALSE startedDisabled=FALSE preview=FALSE
1. YouEnableLocalAnalysis: featureOn=TRUE usage="pending" extract="idle" inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" detail="idle" allowed=TRUE evidence="none" analysis=TRUE startedDisabled=FALSE preview=FALSE
2. TheAnalysisStarts: featureOn=TRUE usage="pending" extract="current" inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" detail="idle" allowed=TRUE evidence="none" analysis=TRUE startedDisabled=FALSE preview=FALSE
3. AFailedReadIsObserved: featureOn=TRUE usage="pending" extract="current" inventory="none" restoredStale=FALSE fileCurrent=TRUE scan="idle" detail="idle" allowed=TRUE evidence="failed_read" analysis=TRUE startedDisabled=FALSE preview=FALSE

## Rules that hold
NoDeadlock
```
