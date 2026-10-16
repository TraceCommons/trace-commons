import assert from "node:assert/strict";
import { test } from "node:test";
import { isNavigablePath, visibleNavItems } from "./routes.ts";

test("the Mission drafts row is hidden while missions are off", () => {
  const ids = visibleNavItems(false).map((item) => item.id);
  assert.ok(!ids.includes("mission-drafts"));
  assert.ok(ids.includes("insights") && ids.includes("settings"));
});

test("the Mission drafts row returns when missions are on", () => {
  const ids = visibleNavItems(true).map((item) => item.id);
  assert.ok(ids.includes("mission-drafts"));
});

test("a desktop event cannot open Mission drafts while missions are off", () => {
  assert.equal(isNavigablePath("/mission-drafts", false), false);
  assert.equal(isNavigablePath("/mission-drafts", true), true);
  assert.equal(isNavigablePath("/settings", false), true);
  assert.equal(isNavigablePath("/nope", true), false);
  assert.equal(isNavigablePath(3, true), false);
});
