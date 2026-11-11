from pathlib import Path
import subprocess, sys
skill, scratch = map(Path, sys.argv[1:])
src=Path(__file__).with_name('FLOW.qnt').read_text()
mutations=[('SkillsDisabled','ThePeriodicScanStarts','featureOn and allowed','allowed'),('ExcludedFolders','YouExcludeTheFolder',"inventory' = \"none\"","inventory' = inventory"),('OldResultsStayRemoved','TheOldScanReturns',"restoredStale' = restoredStale","restoredStale' = (scan == \"stale\")"),('AnalysisOff','YouDisableLocalAnalysis',"usage' = \"off\"","usage' = usage"),('OnlyObservedUse','AFailedReadIsObserved',"evidence' = evidence","evidence' = \"failed_read\"")]
for name,action,old,new in mutations:
 p=scratch/name;p.mkdir(parents=True,exist_ok=True)
 start=src.index('  action '+action+' =');end=src.index('\n  }',start);block=src[start:end]
 assert old in block,(name,old)
 s=src[:start]+block.replace(old,new)+src[end:]
 if name=='OldResultsStayRemoved':
  start=s.index('  action '+action+' =');end=s.index('\n  }',start);block=s[start:end].replace("inventory' = inventory","inventory' = \"ready\"");s=s[:start]+block+s[end:]
 (p/'FLOW.qnt').write_text(s);(p/'FLOW.cfg').write_text('INIT q_init\nNEXT q_step\nINVARIANT '+name+'\n')

for name, *_ in mutations:
 p=scratch/name
 result=subprocess.run([str(skill/'scripts/check'),str(p/'FLOW.qnt'),'--out',str(p/'check')],capture_output=True,text=True)
 (p/'check.log').write_text(result.stdout+result.stderr)
 report=p/'check/report.md'
 assert result.returncode == 1 and report.exists() and ('## '+name+' breaks') in report.read_text(), (name,result.returncode)
 print(name+': expected counterexample detected')
