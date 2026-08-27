#!/usr/bin/env node
// Export local agent-harness sessions to Letta Trajectory v1 files, so the
// Trace Commons contributor CLI can read them.
//
// This is the command @letta-ai/trajectory does not ship. That package is a
// library with no bin, so `npx @letta-ai/trajectory` fails outright; this wraps
// its listTrajectories/normalizeTranscript pair in the CLI its users keep
// reaching for.
//
// It touches the network never, and writes only into --out-dir (default: the
// working directory).

import { probeAll, probe, exportOne, exportInput } from "../src/export.mjs";
import { SOURCES, NATIVE_SOURCES } from "../src/sources.mjs";

const USAGE = `trajectory-export -- export agent sessions for Trace Commons

  trajectory-export                     probe every harness and report
  trajectory-export --all               export the newest session from each
  trajectory-export --source <name>     export the newest session from one
  trajectory-export --source <name> --input <file>
                                        normalize a transcript you name

Options:
  --limit <n>     sessions per source (default 1; --all caps at 5)
  --out-dir <d>   where to write (default: the working directory)
  --quiet         only print the paths written
  -h, --help      this text

Sources: ${SOURCES.join(", ")}

claude-code and codex are absent on purpose: the contributor CLI reads both
natively, with no conversion step and no Node required.`;

function parseArgs(argv) {
  const opts = { limit: null, outDir: process.cwd(), quiet: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "-h" || a === "--help") return { help: true };
    else if (a === "--all") opts.all = true;
    else if (a === "--quiet") opts.quiet = true;
    else if (a === "--source") opts.source = argv[++i];
    else if (a === "--input") opts.input = argv[++i];
    else if (a === "--out-dir") opts.outDir = argv[++i];
    else if (a === "--limit") opts.limit = Number(argv[++i]);
    else return { error: `unknown option: ${a}` };
  }
  return opts;
}

// Everything human goes to stderr so `--quiet` stdout is a clean path list a
// script can consume.
const say = (...m) => console.error(...m);

/**
 * Export-only sources are not failures and must not read as failures. Upstream
 * can normalize their transcripts but cannot enumerate their stores, so the
 * contributor needs a different instruction, not an apology.
 */
function reportExportOnly(exportOnly) {
  if (!exportOnly.length) return;
  say("");
  say("These cannot be listed from disk; export from the harness, then pass --input:");
  for (const r of exportOnly) say(`  ${r.source}`);
}

function reportWritten(source, r, quiet) {
  if (r.skipped) {
    const why = r.skipped === "empty" ? "normalized to zero records" : r.skipped;
    say(`  ${source}: skipped (${why})`);
    return 0;
  }
  if (quiet) console.log(r.path);
  else {
    const d = r.diagnostics.length ? `, ${r.diagnostics.length} diagnostics` : "";
    say(`  wrote ${r.path} (${r.records} records${d})`);
  }
  return 1;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) return say(USAGE), 0;
  if (opts.error) return say(opts.error), say(""), say(USAGE), 2;

  if (opts.source && !SOURCES.includes(opts.source)) {
    if (NATIVE_SOURCES.includes(opts.source)) {
      say(`${opts.source} needs no export: the contributor CLI reads it natively.`);
      say(`Run: trace-commons-contributor submit`);
      return 2;
    }
    say(`unknown source: ${opts.source}`);
    say(`known: ${SOURCES.join(", ")}`);
    return 2;
  }

  if (opts.input) {
    if (!opts.source) return say("--input requires --source"), 2;
    const r = await exportInput(opts.source, opts.input, opts.outDir);
    return reportWritten(opts.source, r, opts.quiet) ? 0 : 1;
  }

  if (opts.source) {
    const { items, error, listingUnsupported } = await probe(opts.source, opts.limit ?? 1);
    if (listingUnsupported) {
      say(`${opts.source}: its sessions cannot be listed from disk.`);
      say(`Export a transcript from ${opts.source} itself, then:`);
      say(`  trajectory-export --source ${opts.source} --input <file>`);
      return 2;
    }
    if (error) return say(`${opts.source}: ${error}`), 1;
    if (!items.length) return say(`${opts.source}: no sessions found`), 1;
    let n = 0;
    for (const item of items.slice(0, opts.limit ?? 1)) {
      n += reportWritten(opts.source, await exportOne(opts.source, item, opts.outDir), opts.quiet);
    }
    return n ? 0 : 1;
  }

  // No source named: probe everything. Reporting is the default because
  // writing a file per harness unasked is a surprise, and a contributor who
  // has not looked yet does not know what is on their machine.
  const results = await probeAll(opts.limit ?? 50);
  const found = results.filter((r) => r.items.length);
  const exportOnly = results.filter((r) => r.listingUnsupported);
  for (const r of results.filter((x) => x.error)) say(`  ${r.source}: unreadable (${r.error})`);

  if (!found.length) {
    say("No sessions found for any harness this tool covers.");
    say(`Looked for: ${SOURCES.join(", ")}`);
    say("");
    say("If you use Claude Code or Codex, you need nothing from this tool:");
    say("the contributor CLI reads both natively. Run: trace-commons-contributor submit");
    reportExportOnly(exportOnly);
    return 1;
  }

  if (!opts.all) {
    say("Found sessions:");
    for (const r of found) say(`  ${r.source}: ${r.items.length}`);
    say("");
    say("Export the newest from each:  trajectory-export --all");
    say("Or just one harness:          trajectory-export --source <name>");
    reportExportOnly(exportOnly);
    return 0;
  }

  let n = 0;
  for (const r of found) {
    for (const item of r.items.slice(0, opts.limit ?? 1)) {
      n += reportWritten(r.source, await exportOne(r.source, item, opts.outDir), opts.quiet);
    }
  }
  if (n && !opts.quiet) {
    say("");
    say(`Wrote ${n} file(s). Submit them with: trace-commons-contributor submit`);
  }
  return n ? 0 : 1;
}

main().then(
  (code) => process.exit(code ?? 0),
  (err) => {
    console.error(`trajectory-export failed: ${err?.message ?? err}`);
    process.exit(1);
  },
);
