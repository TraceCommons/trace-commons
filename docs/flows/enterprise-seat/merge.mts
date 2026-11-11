// Merged chart for a model above the 500-state chart budget (flow-promises SKILL.md step 5).
// Reads TLC's full state dump, labels every state with the render view, and draws it with the
// checker's own passes. Every model state and step stays in; the view only merges them.
// Usage: node merge.mts GRAPH.dot VIEW.mts OUT.mmd
import { Buffer } from "node:buffer";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { inspect } from "node:util";

// FLOW_PROMISES is the flow-promises skill folder.
const skill = process.env.FLOW_PROMISES;
if (!skill) throw new Error("Set FLOW_PROMISES to the flow-promises skill folder.");
const S = `${skill}/statechart/scripts`;
const { parseDot } = await import(`${S}/frontends/dot.ts`);
const { checkView } = await import(`${S}/frontends/view.ts`);
const { walk } = await import(`${S}/passes/reach.ts`);
const { project } = await import(`${S}/pipeline.ts`);
const { mermaidLines } = await import(`${S}/backends/mermaid.ts`);

const [dot, viewFile, out] = process.argv.slice(2);
function* lines(file: string) {
  const fd = fs.openSync(file, "r");
  const buf = Buffer.alloc(1 << 24);
  let rest = "";
  for (let n; (n = fs.readSync(fd, buf, 0, buf.length, null)) > 0; ) {
    const parts = (rest + buf.toString("utf8", 0, n)).split("\n");
    rest = parts.pop() ?? "";
    yield* parts;
  }
  if (rest) yield rest;
  fs.closeSync(fd);
}
const graph = parseDot(lines(dot));
const { hidden: _hidden, ...exports } = await import(pathToFileURL(path.resolve(viewFile)).href);
const view = checkView(exports, inspect);
const { chart } = project(graph, walk(graph), view, []);
fs.writeFileSync(out, [...mermaidLines(chart)].join("\n") + "\n");
console.log(`Wrote ${out}`);
