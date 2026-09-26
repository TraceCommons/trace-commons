import assert from "node:assert/strict";
import { test } from "node:test";
import {
  parseGrantVoidNotice,
  parseGrantVoids,
  rearmTarget,
} from "./grant-void-notice.ts";

const projectVoid = {
  id: 4,
  kind: "project",
  voided_at: "2026-09-26T12:00:00Z",
  project_id: "3f1c",
  project_label: "api",
  reasons: ["witness-measurement-admitted"],
};

const grantVoid = {
  id: 5,
  kind: "automatic_grant",
  voided_at: "2026-09-26T12:00:00Z",
  project_id: null,
  project_label: null,
  reasons: ["destination-changed"],
};

test("a status without grant_voids has nothing to show", () => {
  assert.deepEqual(parseGrantVoids(undefined), []);
  assert.deepEqual(parseGrantVoids([]), []);
});

test("each void keeps the wire element, to hand back to the core verbatim", () => {
  const voids = parseGrantVoids([projectVoid, grantVoid]);
  assert.deepEqual(
    voids.map((v) => v.id),
    [4, 5],
  );
  assert.deepEqual(voids[0].wire, projectVoid);
  assert.deepEqual(voids[1].wire, grantVoid);
});

// A void this build cannot read is still a void: it is kept, so the shell
// can say a notice could not be shown, rather than dropped in silence.
test("an element with an id is kept even when the rest is unfamiliar", () => {
  const voids = parseGrantVoids([{ id: 9, kind: "folder" }]);
  assert.equal(voids.length, 1);
  assert.equal(voids[0].id, 9);
});

test("a malformed list is refused, not read as empty", () => {
  assert.throws(() => parseGrantVoids("none"));
  assert.throws(() => parseGrantVoids([{ kind: "project" }]));
  assert.throws(() => parseGrantVoids([{ id: -1 }]));
});

const notice = {
  title: "Automatic contributing stopped for api",
  body: "b",
  reasons_heading: "What changed",
  reasons: ["r1", "r2"],
  rearm: "re",
  acknowledge: "Got it",
  rearm_action: "Turn back on",
  rearm_failed: "It could not be turned back on.",
};

const noButton = { ...notice, rearm_action: null, rearm_failed: null };

test("the core's notice is taken whole", () => {
  assert.deepEqual(parseGrantVoidNotice(notice), notice);
});

test("a notice without a re-arm button is taken whole too", () => {
  assert.deepEqual(parseGrantVoidNotice(noButton), noButton);
});

// The button and its refusal line travel together: a button with nothing to
// say on refusal, or a refusal line with no button, is a payload out of step.
test("the re-arm button and its refusal line come as a pair", () => {
  assert.throws(() => parseGrantVoidNotice({ ...notice, rearm_failed: null }));
  assert.throws(() => parseGrantVoidNotice({ ...notice, rearm_action: null }));
  assert.throws(() => parseGrantVoidNotice({ ...notice, rearm_action: "" }));
});

// The button acts on the wire element's project_id, and only when the core
// offered it -- never on the grant's notice or an unplaced one.
test("the re-arm target is the element's project, only when offered", () => {
  const [projectVoidParsed, grantVoidParsed] = parseGrantVoids([
    projectVoid,
    grantVoid,
  ]);
  assert.equal(rearmTarget(projectVoidParsed, notice), "3f1c");
  assert.equal(rearmTarget(projectVoidParsed, noButton), null);
  assert.equal(rearmTarget(grantVoidParsed, notice), null);
});

test("a notice missing a sentence is refused rather than shown in part", () => {
  for (const key of Object.keys(noButton)) {
    const partial = { ...notice };
    delete partial[key];
    assert.throws(() => parseGrantVoidNotice(partial), key);
  }
  assert.throws(() => parseGrantVoidNotice({ ...notice, reasons: [] }));
  assert.throws(() => parseGrantVoidNotice({ ...notice, title: "" }));
  assert.throws(() => parseGrantVoidNotice(null));
});
