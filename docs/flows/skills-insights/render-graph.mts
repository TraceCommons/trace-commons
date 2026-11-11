import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
const [skill, dump, out] = process.argv.slice(2);
if (!skill || !dump || !out) throw Error('Usage: node render-graph.mts SKILL_DIR GRAPH.dot OUT_DIR');
const {parseDot} = await import(pathToFileURL(path.join(skill,'statechart/scripts/frontends/dot.ts')).href);
const graph = parseDot(fs.readFileSync(dump,'utf8'));
const inventory = (v:any) => !v.featureOn ? 'Skills disabled' : !v.allowed ? 'Folder excluded' : v.scan !== 'idle' ? 'Scanning inventory' : ({none:'Inventory pending',empty:'No skills found',ready:'Inventory available',partial:'Inventory incomplete'}[v.inventory]);
const usage = (v:any) => !v.featureOn ? 'Skills disabled' : !v.analysis ? 'Usage unavailable — analysis off' : !v.allowed ? 'Folder excluded' : v.extract !== 'idle' ? 'Analyzing local history' : ({off:'Usage pending',pending:'Usage pending',complete:'Analysis complete',partial:'Partial history',unsupported:'Usage unavailable — unsupported'}[v.usage]);
const detail = (v:any) => !v.featureOn ? 'Skills disabled' : !v.allowed ? 'Folder excluded' : v.preview ? 'Definition available' : v.detail === 'current' ? 'Reading definition' : v.detail === 'stale' ? 'Old detail request pending' : 'Detail closed or unavailable';
const initial = [...graph.states.entries()].find(([,s]:any)=>s.start)![0];
let cursor = initial; const witness = [];
for (const action of ['YouPressRefresh','TheScanFindsSkills']) {
 const edge = graph.steps.find((e:any)=>e.from===cursor && e.action===action);
 if (!edge) throw Error('Missing analysis-off inventory witness: '+action);
 cursor=edge.to; witness.push(action);
}
const reached=graph.states.get(cursor).vars;
if (reached.analysis || reached.inventory!=='ready' || reached.evidence!=='none') throw Error('Invalid witness');
const result:any={states:graph.states.size,steps:graph.steps.length,views:{},analysisOffInventoryWitness:witness};
for (const [name,view] of [['FLOW',inventory],['USAGE',usage],['DETAIL',detail]] as const) {
 const ids=new Map<string,string>();const id=(s:string)=>{if(!ids.has(s))ids.set(s,`s${ids.size}`);return ids.get(s)};
 const starts=new Set<string>();for(const s of graph.states.values()){const label=view(s.vars);id(label);if(s.start)starts.add(label)}
 const edges=new Map<string,Set<string>>();let internal=0;
 for(const e of graph.steps){const from=view(graph.states.get(e.from).vars),to=view(graph.states.get(e.to).vars); const key=JSON.stringify([from,to]);if(from===to){internal++;continue;} if(!edges.has(key))edges.set(key,new Set());edges.get(key)!.add(e.action.replace(/([a-z])([A-Z])/g,'$1 $2').toLowerCase());}
 let m='stateDiagram-v2\n  direction TB\n';for(const [label,node] of ids)m+=`  state "${label}" as ${node}\n`;
 for(const s of starts)m+=`  [*] --> ${id(s)}\n`;
 for(const [key,actions] of edges){const [from,to]=JSON.parse(key);m+=`  ${id(from)} --> ${id(to)} : ${[...actions].sort().join(' / ')}\n`;}
 for(const [label,node] of ids)m+=`  class ${node} ${label.includes('disabled')||label.includes('excluded')?'tan':label.includes('ing ')?'yellow':'teal'}\n`;
 fs.writeFileSync(path.join(out,name+'.mmd'),m);
 result.views[name]={labels:ids.size,arrows:edges.size,internalSteps:internal,projectedSteps:graph.steps.length-internal};
}
fs.writeFileSync(path.join(out,'projection.json'),JSON.stringify(result,null,2)+'\n');
console.log(JSON.stringify(result));
