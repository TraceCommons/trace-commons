// Structural artifact validation, not a check of product guarantees.
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { decode } from './itf.mjs';

const [modelDir, traceDir, resultFile, ...selectedModels] = process.argv.slice(2);
if (!modelDir || !traceDir || !resultFile) throw Error('usage: inspect_views.mjs MODELS TRACES RESULT');
const results = [];
for (const filename of fs.readdirSync(modelDir).filter(f => f.endsWith('.frames.json')).sort()) {
  const name = filename.replace('.frames.json', '');
  if (selectedModels.length && !selectedModels.includes(name)) continue;
  const frames = JSON.parse(fs.readFileSync(path.join(modelDir, filename), 'utf8'));
  const labels = new Set(frames.map(f => f.label));
  if (labels.size !== frames.length) throw Error(`${name}: duplicate frame label`);
  const view = await import(pathToFileURL(path.resolve(modelDir, `${name}.view.mts`)).href);
  if (typeof view.state !== 'function' || typeof view.step !== 'function') throw Error(`${name}: missing view functions`);
  const seen = new Map();
  const actions = new Set();
  let states = 0;
  const traces = fs.readdirSync(traceDir).filter(f => f.startsWith(`${name}-`) && f.endsWith('.json')).sort();
  if (!traces.length) throw Error(`${name}: no simulation traces`);
  for (const file of traces) {
    const trace = JSON.parse(fs.readFileSync(path.join(traceDir, file), 'utf8'));
    let previous;
    for (const raw of trace.states) {
      const vars = decode(raw);
      const label = view.state(vars);
      if (typeof label !== 'string' || !labels.has(label)) throw Error(`${name}: view produced unknown frame ${JSON.stringify(label)}`);
      seen.set(label, (seen.get(label) ?? 0) + 1);
      const action = vars['mbt::actionTaken'];
      actions.add(action);
      if (previous) {
        const step = view.step(action, previous, vars);
        if (step !== null && typeof step !== 'string') throw Error(`${name}: invalid step label`);
      }
      previous = vars;
      states++;
    }
  }
  results.push({name, traces: traces.length, sampledStates: states, observedActions: [...actions].sort(),
    observedFrames: [...seen.keys()].sort(), unobservedFrames: [...labels].filter(l => !seen.has(l)).sort()});
}
fs.writeFileSync(resultFile, JSON.stringify(results, null, 2) + '\n');
process.stdout.write(`Validated views for ${results.length} models against simulation traces.\n`);
