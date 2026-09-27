import assert from "node:assert/strict";
import { test } from "node:test";
import {
  acknowledgeWitnessDisclosure,
  decideLater,
  goBack,
  grantBlockers,
  initialFlow1Progress,
  initialScopeSelection,
  requestGrant,
  scopeChoice,
  withdrawAndConfirm,
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
  assert.equal(
    scopeChoice(options, ["debugging_evaluation"]).canContinue,
    true,
  );
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
  witnessShown: null,
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

test("the grant is given under the witness the disclosure screen showed", async () => {
  const shown = [];
  await requestGrant({ ...complete, witnessShown: "0xshown" }, async (w) => {
    shown.push(w);
  });
  await requestGrant({ ...complete, witnessShown: null }, async (w) => {
    shown.push(w);
  });
  assert.deepEqual(shown, ["0xshown", null]);
});

test("reading the witness screen records the witness it showed", () => {
  const read = acknowledgeWitnessDisclosure(
    { ...complete, witnessDisclosureSeen: false, witnessShown: null },
    "0xabc",
  );
  assert.equal(read.witnessDisclosureSeen, true);
  assert.equal(read.witnessShown, "0xabc");
  assert.equal(acknowledgeWitnessDisclosure(complete, null).witnessShown, null);
});

test("Decide later saves no scope, gives no grant and lands on Flow 2", async () => {
  for (const showPrivacy of [true, false]) {
    const next = decideLater(showPrivacy);
    assert.equal(next.progress.scopesSaved, null);
    assert.equal(next.progress.path, "ask_first");
    assert.equal(next.privacyIncluded, showPrivacy);
    assert.equal(next.step, showPrivacy ? "privacy" : "projects");
    // Even a connected contributor who read everything gets no grant.
    const progress = {
      ...next.progress,
      connected: true,
      scrubDisclosureSeen: true,
      witnessDisclosureSeen: true,
    };
    assert.ok(grantBlockers(progress).includes("scope"));
    let calls = 0;
    await assert.rejects(
      requestGrant(progress, async () => {
        calls += 1;
      }),
      /grant-steps-incomplete/,
    );
    assert.equal(calls, 0);
  }
});

test("Back resets the disclosures read, so the grant needs them again", async () => {
  for (const from of ["grant", "disclosure_witness", "disclosure_scrub"]) {
    const back = goBack({ ...complete, witnessShown: "0xabc" }, from, false);
    assert.equal(back.progress.scrubDisclosureSeen, false);
    assert.equal(back.progress.witnessDisclosureSeen, false);
    assert.equal(back.progress.witnessShown, null);
    assert.deepEqual(back.progress.scopesSaved, complete.scopesSaved);
    assert.ok(grantBlockers(back.progress).includes("scrub_disclosure"));
    let calls = 0;
    await assert.rejects(
      requestGrant(back.progress, async () => {
        calls += 1;
      }),
      /grant-steps-incomplete/,
    );
    assert.equal(calls, 0);
  }
  assert.equal(goBack(complete, "grant", false).step, "disclosure_witness");
  assert.equal(
    goBack(complete, "disclosure_witness", false).step,
    "disclosure_scrub",
  );
  assert.equal(goBack(complete, "disclosure_scrub", false).step, "path");
  assert.equal(goBack(complete, "disclosure_scrub", true).step, "privacy");
  assert.equal(goBack(complete, "path", false).step, "consent");
});

test("a withdraw is confirmed only by re-reading the grant status", async () => {
  const order = [];
  const outcome = await withdrawAndConfirm(
    async () => {
      order.push("withdraw");
      return true;
    },
    async () => {
      order.push("read");
      return { granted: false };
    },
  );
  assert.equal(outcome, "withdrawn");
  assert.deepEqual(order, ["withdraw", "read"]);

  // The daemon said withdrawn, but a grant is still in force: not confirmed.
  assert.equal(
    await withdrawAndConfirm(
      async () => true,
      async () => ({ granted: true }),
    ),
    "still_granted",
  );
  // Nothing was in force to withdraw: none is in force now.
  assert.equal(
    await withdrawAndConfirm(
      async () => false,
      async () => ({ granted: false }),
    ),
    "withdrawn",
  );
  // A failed withdraw is never reported as one.
  let reads = 0;
  await assert.rejects(
    withdrawAndConfirm(
      async () => {
        throw new Error("daemon-unavailable");
      },
      async () => {
        reads += 1;
        return { granted: false };
      },
    ),
    /daemon-unavailable/,
  );
  assert.equal(reads, 0);
});
