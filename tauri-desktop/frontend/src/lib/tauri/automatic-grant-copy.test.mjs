import assert from "node:assert/strict";
import { test } from "node:test";
import {
  parseAutomaticGrantCopy,
  scrubDisclosureLines,
} from "./automatic-grant-copy.ts";

// Stand-ins, not the core's sentences: the shell must render whichever block
// the core sent and nothing else, so the words themselves do not matter here.
const MODEL_SCOPE = "MODEL SCOPE: removed when a model recognises it.";
const MODEL_LIMIT = "MODEL LIMIT: the model is not reliable.";
const PATTERNS_SCOPE = "PATTERNS SCOPE";
const PATTERNS_LIMIT = "PATTERNS LIMIT";

const common = {
  no_review: "NO REVIEW",
  scope_required: "SCOPE REQUIRED",
  path_automatic: "PATH AUTOMATIC",
  path_ask_first: "PATH ASK FIRST",
  raw_send: "RAW SEND",
  witness_origin: "WITNESS ORIGIN",
};

const patternsOnly = {
  ...common,
  disclosure: "patterns_only",
  patterns_only: { scope: PATTERNS_SCOPE, limit: PATTERNS_LIMIT },
  model_scrubbed: null,
};

test("a patterns-only disclosure shows the patterns wording and never the model wording", () => {
  const copy = parseAutomaticGrantCopy(patternsOnly);
  assert.equal(copy.disclosure, "patterns_only");
  const lines = scrubDisclosureLines(copy);
  assert.deepEqual(lines, [PATTERNS_SCOPE, PATTERNS_LIMIT, "NO REVIEW"]);
  for (const line of lines) {
    assert.ok(!line.includes("MODEL"), line);
  }
  assert.ok(!JSON.stringify(copy).includes("MODEL"));
});

test("a patterns-only payload that also carries model wording is refused", () => {
  assert.throws(
    () =>
      parseAutomaticGrantCopy({
        ...patternsOnly,
        model_scrubbed: { scope: MODEL_SCOPE, limit: MODEL_LIMIT },
      }),
    /model-scrub/,
  );
});

test("a patterns-only payload without its own wording is refused, not filled from the model block", () => {
  assert.throws(() =>
    parseAutomaticGrantCopy({
      ...patternsOnly,
      patterns_only: null,
    }),
  );
});

test("an unknown or missing disclosure is refused", () => {
  assert.throws(() =>
    parseAutomaticGrantCopy({ ...patternsOnly, disclosure: "full_pipeline" }),
  );
  assert.throws(() =>
    parseAutomaticGrantCopy({ ...patternsOnly, disclosure: undefined }),
  );
});

test("the model wording is shown only when the core says a certified pipeline ran", () => {
  const copy = parseAutomaticGrantCopy({
    ...common,
    disclosure: "model_scrubbed",
    patterns_only: null,
    model_scrubbed: { scope: MODEL_SCOPE, limit: MODEL_LIMIT },
  });
  assert.deepEqual(scrubDisclosureLines(copy), [
    MODEL_SCOPE,
    MODEL_LIMIT,
    "NO REVIEW",
  ]);
});
