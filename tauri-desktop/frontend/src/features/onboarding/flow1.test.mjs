import assert from "node:assert/strict";
import { test } from "node:test";
import {
  grantBlockers,
  initialFlow1Progress,
  initialScopeSelection,
  requestGrant,
  scopeChoice,
} from "./flow1.ts";

const options = [
  { name: "debugging_evaluation", always_on: true },
  { name: "benchmark_only", always_on: false },
  { name: "public_attribution", always_on: false },
];

test("the scope picker starts with nothing selected, the floor scope included", () => {
  assert.deepEqual(initialScopeSelection(), []);
  const choice = scopeChoice(options, initialScopeSelection());
  assert.equal(choice.canContinue, false);
  assert.deepEqual(choice.missingRequired, ["debugging_evaluation"]);
});

test("the scope picker blocks until the always-on scope is ticked by hand", () => {
  assert.equal(scopeChoice(options, ["benchmark_only"]).canContinue, false);
  assert.equal(scopeChoice(options, ["debugging_evaluation"]).canContinue, true);
  assert.equal(
    scopeChoice(options, ["debugging_evaluation", "public_attribution"])
      .canContinue,
    true,
  );
});

test("the scope picker blocks with no options and refuses unknown scopes", () => {
  assert.equal(scopeChoice([], []).canContinue, false);
  assert.equal(scopeChoice([], ["anything"]).canContinue, false);
  assert.equal(
    scopeChoice(options, ["debugging_evaluation", "invented"]).canContinue,
    false,
  );
});

const complete = {
  connected: true,
  scopesSaved: ["debugging_evaluation"],
  path: "automatic",
  scrubDisclosureSeen: true,
  witnessDisclosureSeen: true,
};

test("the grant is refused, uncalled, before every step is done", async () => {
  assert.deepEqual(grantBlockers(initialFlow1Progress), [
    "connect",
    "scope",
    "path",
    "scrub_disclosure",
    "witness_disclosure",
  ]);
  const missing = [
    ["connect", { connected: false }],
    ["scope", { scopesSaved: null }],
    ["scope", { scopesSaved: [] }],
    ["path", { path: null }],
    ["path", { path: "ask_first" }],
    ["scrub_disclosure", { scrubDisclosureSeen: false }],
    ["witness_disclosure", { witnessDisclosureSeen: false }],
  ];
  for (const [blocker, change] of missing) {
    const progress = { ...complete, ...change };
    assert.deepEqual(grantBlockers(progress), [blocker]);
    let calls = 0;
    await assert.rejects(
      requestGrant(progress, async () => {
        calls += 1;
      }),
      /grant-steps-incomplete/,
    );
    assert.equal(calls, 0, `grant called with ${blocker} missing`);
  }
});

test("the grant is called once every step is done", async () => {
  assert.deepEqual(grantBlockers(complete), []);
  let calls = 0;
  const result = await requestGrant(complete, async () => {
    calls += 1;
    return { granted: true };
  });
  assert.equal(calls, 1);
  assert.deepEqual(result, { granted: true });
});
