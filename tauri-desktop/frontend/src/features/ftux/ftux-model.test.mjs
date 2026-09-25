import assert from "node:assert/strict";
import { test } from "node:test";
import {
  applyRule,
  canContinueFromFolders,
  formatDuration,
  formatSessionDate,
  groupState,
  nextScreen,
  optionalUsesLabel,
  passkeyNameError,
  passkeyTransition,
  pastSessionSummary,
  stepsFor,
  switchPath,
  toggleAt,
  toggleGroup,
  unansweredTools,
} from "./ftux-model.ts";

test("connect and forget is three screens, customize and tailor is four", () => {
  assert.deepEqual(
    stepsFor("connect").map((step) => step.label),
    ["Join", "Folders", "Uses"],
  );
  assert.deepEqual(
    stepsFor("customize").map((step) => step.label),
    ["Join", "Tools", "Rules", "Uses"],
  );
  assert.equal(nextScreen("connect", "join"), "folders");
  assert.equal(nextScreen("customize", "tools"), "rules");
  assert.equal(nextScreen("customize", "uses"), null);
});

test("switching tiers lands on the equivalent screen", () => {
  assert.deepEqual(switchPath("customize", "folders"), {
    path: "customize",
    screen: "tools",
  });
  assert.deepEqual(switchPath("connect", "rules"), {
    path: "connect",
    screen: "folders",
  });
  assert.deepEqual(switchPath("customize", "join"), {
    path: "customize",
    screen: "join",
  });
});

test("folders continue only once every found tool has an answer", () => {
  const tools = [
    { id: "claude", presence: "found" },
    { id: "codex", presence: "missing" },
    { id: "antigravity", presence: "found" },
  ];
  assert.deepEqual(unansweredTools(tools, { claude: "watch" }), [
    "antigravity",
  ]);
  assert.equal(canContinueFromFolders(tools, { claude: "watch" }), false);
  assert.equal(
    canContinueFromFolders(tools, { claude: "watch", antigravity: "ignore" }),
    true,
    "saying no is an answer, and a missing tool is not asked",
  );
  assert.equal(
    canContinueFromFolders(tools, {
      claude: "watch",
      antigravity: "unanswered",
    }),
    false,
  );
});

test("optional uses label and group toggle", () => {
  assert.equal(
    optionalUsesLabel([true, true, true]),
    "3 optional uses, all on",
  );
  assert.equal(
    optionalUsesLabel([false, false, false]),
    "3 optional uses, all off",
  );
  assert.equal(
    optionalUsesLabel([true, false, false]),
    "3 optional uses · 1 on",
  );
  assert.equal(groupState([true, false, true]), "some");
  assert.deepEqual(toggleGroup([true, false, true]), [true, true, true]);
  assert.deepEqual(toggleGroup([true, true, true]), [false, false, false]);
  assert.deepEqual(toggleAt([true, false], 1), [true, true]);
});

test("past sessions exclude folders whose rule is Never", () => {
  const repos = [
    {
      folder: "a",
      rule: "ask",
      sessionCount: 22,
      selected: [true, false, true],
    },
    { folder: "b", rule: "auto", sessionCount: 9, selected: [true, true] },
    { folder: "c", rule: "never", sessionCount: 6, selected: [true] },
  ];
  assert.deepEqual(pastSessionSummary(repos), {
    selected: 4,
    eligible: 31,
    label: "4 of 31 selected",
  });
  const neverB = applyRule(repos[1], "never");
  assert.deepEqual(neverB.selected, [false, false]);
  assert.equal(
    pastSessionSummary([repos[0], neverB]).label,
    "2 of 22 selected",
  );
});

test("passkey creation walks P-1 to P-5 and ends created", () => {
  let step = "choose";
  for (const [event, expected] of [
    ["create-new", "name"],
    ["named", "save-where"],
    ["store-chosen", "touch-id"],
    ["touched", "verify"],
  ]) {
    const next = passkeyTransition(step, { type: event });
    assert.deepEqual(next, { kind: "step", step: expected });
    step = next.step;
  }
  assert.deepEqual(passkeyTransition(step, { type: "verified" }), {
    kind: "done",
    outcome: "created",
  });
});

test("cancelling verify signs out, closing earlier returns to join", () => {
  assert.deepEqual(passkeyTransition("verify", { type: "cancel" }), {
    kind: "signed-out",
  });
  assert.deepEqual(passkeyTransition("choose", { type: "cancel" }), {
    kind: "closed",
  });
  assert.deepEqual(passkeyTransition("name", { type: "back" }), {
    kind: "step",
    step: "choose",
  });
  assert.deepEqual(passkeyTransition("touch-id", { type: "cancel" }), {
    kind: "step",
    step: "save-where",
  });
});

test("using an existing passkey goes through the system sign-in sheet", () => {
  assert.deepEqual(passkeyTransition("choose", { type: "use-existing" }), {
    kind: "step",
    step: "sign-in",
  });
  assert.deepEqual(passkeyTransition("sign-in", { type: "touched" }), {
    kind: "done",
    outcome: "signed-in",
  });
});

test("passkey names must be non-empty and short", () => {
  assert.equal(passkeyNameError("My trace passkey"), null);
  assert.notEqual(passkeyNameError("   "), null);
  assert.notEqual(passkeyNameError("x".repeat(65)), null);
});

test("session dates and durations read the way the design writes them", () => {
  assert.equal(formatSessionDate("2026-09-12T00:00:00.000Z"), "Sat 12 Sep");
  assert.equal(formatSessionDate("not a date"), "");
  assert.equal(formatDuration(52), "52 min");
  assert.equal(formatDuration(78), "1 h 18 min");
  assert.equal(formatDuration(124), "2 h 04 min");
});
