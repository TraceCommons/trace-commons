import {
  listTrajectories,
  normalizeTranscript,
  NormalizationError,
} from "@letta-ai/trajectory";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { join } from "node:path";
import { SOURCES } from "./sources.mjs";

/**
 * The suffix matters. The Rust CLI auto-discovers `*.trajectory.json` in the
 * working directory and nothing else, precisely so a stray `session.json`
 * never joins a submission. Writing any other name means the contributor has
 * to pass --trajectory by hand, which is the step this tool exists to remove.
 */
export const SUFFIX = ".trajectory.json";

/** Filenames are ours to choose but ids are not, so keep them filesystem-safe. */
export function safeId(id) {
  return String(id).replace(/[^A-Za-z0-9._-]/g, "-").slice(0, 100);
}

/**
 * Probe one source's local store. A missing store yields an empty listing
 * rather than an error upstream, so every source can be probed uniformly and a
 * machine without a given harness simply reports zero.
 *
 * Several sources are export-only: upstream can normalize their transcripts
 * but cannot enumerate their local store, and says so with a
 * `listing_unavailable` code. That is a fact about the source, not a failure,
 * and it must not be reported as one -- it tells the contributor to export
 * from the harness and pass --input, which is a different instruction from
 * "something broke".
 */
export async function probe(source, limit = 50) {
  try {
    const { items } = await listTrajectories({ source, limit });
    return { source, items, listingUnsupported: false, error: null };
  } catch (err) {
    if (err instanceof NormalizationError && err.code === "listing_unavailable") {
      return { source, items: [], listingUnsupported: true, error: null };
    }
    // A store that exists but cannot be read is worth reporting, and not worth
    // aborting the whole probe over: the contributor may have sessions in the
    // other eleven.
    return {
      source,
      items: [],
      listingUnsupported: false,
      error: err?.message ?? String(err),
    };
  }
}

export async function probeAll(limit = 50) {
  return Promise.all(SOURCES.map((s) => probe(s, limit)));
}

/**
 * Normalize, converting upstream's refusals into a skip rather than a throw.
 *
 * `normalizeTranscript` throws a NormalizationError for perfectly ordinary
 * sessions -- `missing_assistant_records` fires on a transcript where the user
 * typed and then quit, which is common. Letting that propagate would abort a
 * whole `--all` run over one unremarkable session, so a session that cannot be
 * normalized is skipped by name and the rest continue.
 */
function normalize(source, transcript) {
  try {
    const { records, diagnostics } = normalizeTranscript({ source, transcript });
    if (!records.length) {
      return { path: null, records: 0, diagnostics, skipped: "empty" };
    }
    return { records, diagnostics, skipped: null };
  } catch (err) {
    if (err instanceof NormalizationError) {
      return { path: null, records: 0, diagnostics: [], skipped: err.code };
    }
    throw err;
  }
}

/**
 * Convert one listed session to a trajectory file. Returns the path written
 * and the upstream diagnostics, which are surfaced rather than swallowed --
 * they say what the normalizer dropped or synthesized, and a contributor
 * deciding whether to submit deserves to see that.
 */
export async function exportOne(source, item, outDir) {
  const transcript = await readFile(item.path, "utf8");
  const normalized = normalize(source, transcript);
  if (normalized.skipped) return normalized;
  const { records, diagnostics } = normalized;
  await mkdir(outDir, { recursive: true });
  const out = join(outDir, `${source}-${safeId(item.id)}${SUFFIX}`);
  // JSON Lines. The reader accepts a top-level array too, but one record per
  // line keeps a partially written file diagnosable instead of unparseable.
  await writeFile(out, records.map((r) => JSON.stringify(r)).join("\n") + "\n");
  return { path: out, records: records.length, diagnostics, skipped: null };
}

/** Normalize a transcript the caller names, for stores this tool cannot enumerate. */
export async function exportInput(source, inputPath, outDir) {
  const transcript = await readFile(inputPath, "utf8");
  const normalized = normalize(source, transcript);
  if (normalized.skipped) return normalized;
  const { records, diagnostics } = normalized;
  await mkdir(outDir, { recursive: true });
  const stem = inputPath.split("/").pop().replace(/\.[^.]+$/, "");
  const out = join(outDir, `${source}-${safeId(stem)}${SUFFIX}`);
  await writeFile(out, records.map((r) => JSON.stringify(r)).join("\n") + "\n");
  return { path: out, records: records.length, diagnostics, skipped: null };
}
