import assert from "node:assert/strict";
import { test } from "node:test";
import {
  askFirstTarget,
  GATE_HELD_LABEL,
  parseArmingRewordedNotice,
  parseArmingRewordings,
  parseGateHeld,
  parseGateHeldNotice,
} from "./switch-on-notices.ts";

const rewording = {
  id: 2,
  reworded_at: "2026-09-27T01:00:00Z",
  project_id: "3f1c",
  project_label: "api",
  was: "model_scrubbed",
  now: "patterns_only",
};

test("a status without arming_rewordings has nothing to show", () => {
  assert.deepEqual(parseArmingRewordings(undefined), []);
  assert.deepEqual(parseArmingRewordings([]), []);
});

test("each rewording keeps the wire element, to hand back to the core", () => {
  const [parsed] = parseArmingRewordings([rewording]);
  assert.equal(parsed.id, 2);
  assert.deepEqual(parsed.wire, rewording);
});

// A malformed list read as empty would reword a folder in silence.
test("a malformed rewording list is refused, not read as empty", () => {
  assert.throws(() => parseArmingRewordings("none"));
  assert.throws(() => parseArmingRewordings([{ project_label: "api" }]));
  assert.throws(() => parseArmingRewordings([{ id: -1 }]));
});

const notice = {
  title: "What automatic contributing from api now means",
  body: "b",
  now_heading: "What happens to its sessions",
  scope: "s",
  limit: "l",
  no_review: "n",
  acknowledge: "Got it",
  ask_first_action: "Ask me first",
  ask_first_failed: "It could not be switched.",
};

test("the core's rewording notice is taken whole", () => {
  assert.deepEqual(parseArmingRewordedNotice(notice), notice);
  const noButton = { ...notice, ask_first_action: null, ask_first_failed: null };
  assert.deepEqual(parseArmingRewordedNotice(noButton), noButton);
});

test("a rewording notice missing a sentence is refused", () => {
  for (const key of ["title", "body", "now_heading", "scope", "limit", "no_review", "acknowledge"]) {
    const partial = { ...notice };
    delete partial[key];
    assert.throws(() => parseArmingRewordedNotice(partial), key);
  }
  assert.throws(() => parseArmingRewordedNotice({ ...notice, ask_first_failed: null }));
  assert.throws(() => parseArmingRewordedNotice(null));
});

test("Ask me first acts on the element's project, only when offered", () => {
  const [parsed] = parseArmingRewordings([rewording]);
  assert.equal(askFirstTarget(parsed.wire, notice), "3f1c");
  assert.equal(
    askFirstTarget(parsed.wire, { ...notice, ask_first_action: null, ask_first_failed: null }),
    null,
  );
  assert.equal(askFirstTarget({ id: 1 }, notice), null);
});

const held = {
  held_sessions: 3,
  reasons: ["admission-evidence-is-per-session"],
  projects: [{ project_id: "3f1c", project_label: "api", held_sessions: 3 }],
};

test("the held label is the daemon's", () => {
  assert.equal(GATE_HELD_LABEL, "automatic-contribution-held");
});

test("nothing held, or a core too old to say, has nothing to show", () => {
  assert.equal(parseGateHeld(undefined), null);
  assert.equal(
    parseGateHeld({ held_sessions: 0, reasons: [], projects: [] }),
    null,
  );
});

test("a held object keeps the wire, to hand back to the core", () => {
  const parsed = parseGateHeld(held);
  assert.equal(parsed.held_sessions, 3);
  assert.deepEqual(parsed.wire, held);
});

// Read as "nothing held", a malformed object would hide armed folders the
// gate is holding.
test("a malformed held object is refused", () => {
  assert.throws(() => parseGateHeld("held"));
  assert.throws(() => parseGateHeld({ held_sessions: -1 }));
  assert.throws(() => parseGateHeld({ held_sessions: "3" }));
});

const heldNotice = {
  title: "Automatic contributing is on hold",
  body: "3 sessions ...",
  reasons: ["This commons does not yet accept automatic contributions from your account."],
  release: "r",
  ask_first: "a",
  projects: [
    {
      project_id: "3f1c",
      line: "api: 3 sessions waiting",
      ask_first_action: "Ask me first",
      ask_first_failed: "f",
    },
  ],
};

test("the core's held notice is taken whole", () => {
  assert.deepEqual(parseGateHeldNotice(heldNotice), heldNotice);
});

test("a held notice missing a sentence is refused", () => {
  for (const key of ["title", "body", "release", "ask_first"]) {
    const partial = { ...heldNotice };
    delete partial[key];
    assert.throws(() => parseGateHeldNotice(partial), key);
  }
  assert.throws(() => parseGateHeldNotice({ ...heldNotice, reasons: [] }));
  assert.throws(() =>
    parseGateHeldNotice({
      ...heldNotice,
      projects: [{ ...heldNotice.projects[0], ask_first_failed: null }],
    }),
  );
});
