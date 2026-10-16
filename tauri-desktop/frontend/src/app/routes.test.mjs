import assert from "node:assert/strict";
import { test } from "node:test";
import { isNavigablePath, visibleMonitorView } from "./routes.ts";

test("the Missions view is unreachable while missions are off", () => {
  assert.equal(visibleMonitorView("/mission-drafts", false), "home");
  assert.equal(visibleMonitorView("/history", false), "history");
});

test("the Missions view returns when missions are on", () => {
  assert.equal(visibleMonitorView("/mission-drafts", true), "missions");
});

test("a desktop event cannot open Mission drafts while missions are off", () => {
  assert.equal(isNavigablePath("/mission-drafts", false), false);
  assert.equal(isNavigablePath("/mission-drafts", true), true);
  assert.equal(isNavigablePath("/settings", false), true);
  assert.equal(isNavigablePath("/nope", true), false);
  assert.equal(isNavigablePath(3, true), false);
});
