import assert from "node:assert/strict";
import { test } from "node:test";
import {
  WITNESS_SATURATED_LABEL,
  nextRetryLine,
  parseWitnessCapacity,
  parseWitnessCapacityNotice,
} from "./witness-capacity.ts";

test("the label is the daemon's spelling", () => {
  assert.equal(WITNESS_SATURATED_LABEL, "witness-saturated");
});

test("nothing waiting, or a core too old to say, shows nothing", () => {
  assert.equal(parseWitnessCapacity(undefined), null);
  assert.equal(
    parseWitnessCapacity({ waiting_sessions: 0, next_retry_at: null }),
    null,
  );
});

test("sessions waiting keep the wire object, to hand back to the core", () => {
  const wire = { waiting_sessions: 2, next_retry_at: "2030-01-01T00:01:00Z" };
  assert.deepEqual(parseWitnessCapacity(wire), {
    waiting_sessions: 2,
    next_retry_at: "2030-01-01T00:01:00Z",
    wire,
  });
});

// A malformed object read as "nothing waiting" would hide sessions that are
// held, which is the one thing this surface exists to say.
test("a malformed object is refused, not read as nothing waiting", () => {
  for (const value of [
    null,
    "2",
    [],
    { next_retry_at: null },
    { waiting_sessions: -1, next_retry_at: null },
    { waiting_sessions: 1.5, next_retry_at: null },
    { waiting_sessions: "2", next_retry_at: null },
    { waiting_sessions: 2, next_retry_at: 7 },
  ]) {
    assert.throws(() => parseWitnessCapacity(value), JSON.stringify(value));
  }
});

const notice = {
  title: "Waiting for the privacy witness",
  body: "2 approved sessions are waiting because the privacy witness is busy.",
  next_check: "Next try",
};

test("the core's notice is taken whole", () => {
  assert.deepEqual(parseWitnessCapacityNotice(notice), notice);
});

test("a notice missing a sentence is refused rather than shown in part", () => {
  for (const key of Object.keys(notice)) {
    const partial = { ...notice };
    delete partial[key];
    assert.throws(() => parseWitnessCapacityNotice(partial), key);
    assert.throws(
      () => parseWitnessCapacityNotice({ ...notice, [key]: "" }),
      key,
    );
  }
  assert.throws(() => parseWitnessCapacityNotice(null));
});

test("the next try is the core's label and the time, or nothing", () => {
  const capacity = parseWitnessCapacity({
    waiting_sessions: 1,
    next_retry_at: "2030-01-01T00:01:00Z",
  });
  assert.equal(
    nextRetryLine(notice, capacity, (date) => date.toISOString()),
    "Next try: 2030-01-01T00:01:00.000Z",
  );
  const unscheduled = parseWitnessCapacity({
    waiting_sessions: 1,
    next_retry_at: null,
  });
  assert.equal(nextRetryLine(notice, unscheduled), null);
  const unreadable = parseWitnessCapacity({
    waiting_sessions: 1,
    next_retry_at: "soon",
  });
  assert.equal(nextRetryLine(notice, unreadable), null);
});
