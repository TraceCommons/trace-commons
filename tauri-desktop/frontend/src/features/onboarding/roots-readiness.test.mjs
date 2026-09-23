import assert from "node:assert/strict";
import { test } from "node:test";
import {
  missingRequiredRoots,
  rootsContinueError,
  rootsReadiness,
} from "./roots-readiness.ts";

const answered = {
  claude_source_mode: "watch",
  codex_source_mode: "off",
  gemini_source_mode: "unset",
  cline_source_mode: "unset",
  opencode_source_mode: "unset",
};

test("roots are undeclared until both Claude Code and Codex are answered", () => {
  assert.deepEqual(missingRequiredRoots({}), ["claude", "codex"]);
  assert.deepEqual(
    missingRequiredRoots({ ...answered, codex_source_mode: "unset" }),
    ["codex"],
  );
  assert.deepEqual(
    missingRequiredRoots({ ...answered, claude_source_mode: "bogus" }),
    ["claude"],
  );
  assert.equal(rootsReadiness({ claude_source_mode: "watch" }, "needs_roots"), "undeclared");
  assert.equal(rootsReadiness({ codex_source_mode: "off" }, "running"), "undeclared");
});

test("optional sources never gate the roots step", () => {
  assert.deepEqual(missingRequiredRoots(answered), []);
  assert.equal(rootsReadiness(answered, "needs_roots"), "needs_start");
});

test("a declared roots step advances only once the daemon reports running", () => {
  assert.equal(rootsReadiness(answered, "running"), "running");
  assert.equal(rootsReadiness(answered, "daemon_unavailable"), "needs_start");
  assert.equal(rootsReadiness(answered, undefined), "needs_start");
});

test("a failed start is reported as a start failure, not an enrollment failure", () => {
  assert.equal(rootsContinueError(new Error("roots-not-declared")), "roots_required");
  assert.equal(
    rootsContinueError(new Error("starting embedded contributor daemon failed")),
    "start_failed",
  );
  assert.equal(rootsContinueError(null), "start_failed");
});
