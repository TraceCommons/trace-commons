import assert from "node:assert/strict";
import { test } from "node:test";
import {
  parseGrantVoidNotice,
  parseGrantVoids,
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
};

test("the core's notice is taken whole", () => {
  assert.deepEqual(parseGrantVoidNotice(notice), notice);
});

test("a notice missing a sentence is refused rather than shown in part", () => {
  for (const key of Object.keys(notice)) {
    const partial = { ...notice };
    delete partial[key];
    assert.throws(() => parseGrantVoidNotice(partial), key);
  }
  assert.throws(() => parseGrantVoidNotice({ ...notice, reasons: [] }));
  assert.throws(() => parseGrantVoidNotice({ ...notice, title: "" }));
  assert.throws(() => parseGrantVoidNotice(null));
});
