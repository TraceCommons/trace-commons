import assert from "node:assert/strict";
import { test } from "node:test";
import { WORDING_UNREADABLE } from "../copy-unreadable.ts";
import {
  parseShellStatusCopy,
  shellStatusLines,
} from "./shell-status-copy.ts";

const payload = () => ({
  core_down: {
    title: "core down title",
    detail: "core down detail",
    action: null,
    action_kind: null,
    severity: "actionable",
  },
  read_unavailable: "read unavailable",
  request_failed: "request failed",
  retry_startup: "retry",
  retrying_startup: "retrying",
});

test("the core's status lines are kept as the core words them", () => {
  const copy = parseShellStatusCopy(payload());
  assert.deepEqual(copy, {
    core_down: { title: "core down title", detail: "core down detail" },
    read_unavailable: "read unavailable",
    request_failed: "request failed",
    retry_startup: "retry",
    retrying_startup: "retrying",
  });
});

test("a missing or empty line is refused rather than filled in", () => {
  for (const key of [
    "read_unavailable",
    "request_failed",
    "retry_startup",
    "retrying_startup",
  ]) {
    assert.throws(() => parseShellStatusCopy({ ...payload(), [key]: "" }));
    const without = payload();
    delete without[key];
    assert.throws(() => parseShellStatusCopy(without));
  }
  assert.throws(() =>
    parseShellStatusCopy({ ...payload(), core_down: { title: "t" } }),
  );
  assert.throws(() => parseShellStatusCopy(null));
  assert.throws(() => parseShellStatusCopy([]));
});

test("lines resolve to the core's words, the shell's one sentence, or nothing", () => {
  const copy = parseShellStatusCopy(payload());
  const ready = shellStatusLines(copy, false, WORDING_UNREADABLE);
  assert.equal(ready.readUnavailable, "read unavailable");
  assert.equal(ready.requestFailed, "request failed");
  assert.equal(ready.coreDown.title, "core down title");
  assert.equal(ready.coreDown.detail, "core down detail");
  assert.equal(ready.retryStartup, "retry");
  assert.equal(ready.retrying, "retrying");
  assert.equal(ready.unreadable, false);

  // The core's words did not arrive: every line is the shell's one
  // sentence, which claims nothing about what happened.
  const failed = shellStatusLines(undefined, true, WORDING_UNREADABLE);
  assert.equal(failed.unreadable, true);
  for (const line of [
    failed.readUnavailable,
    failed.requestFailed,
    failed.coreDown.title,
  ]) {
    assert.equal(line, WORDING_UNREADABLE);
  }
  assert.equal(failed.coreDown.detail, "");
  assert.equal(failed.retryStartup, "");

  // Still loading: nothing is said yet.
  const loading = shellStatusLines(undefined, false, WORDING_UNREADABLE);
  assert.equal(loading.readUnavailable, "");
  assert.equal(loading.requestFailed, "");
  assert.equal(loading.coreDown.title, "");
});
