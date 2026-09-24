import assert from "node:assert/strict";
import { test } from "node:test";
import {
  NEAR_AI_NOTICE_LABEL,
  nearAiNoticeRecovery,
} from "./health-recovery.ts";

const copy = {
  title: "t",
  local_always: "l",
  offer: "o",
  disclosure: "d",
  local_only: "lo",
  with_near: "w",
  recovery_title: "rt",
  recovery_detail: "rd",
  recovery_action: "ra",
  recovery_confirm: "rc",
  recovery_working: "rw",
  recovery_failed: "rf",
  recovery_cancel: "c",
};

test("only the NEAR AI notice label offers the notice recovery", () => {
  assert.equal(NEAR_AI_NOTICE_LABEL, "near-ai-notice-not-acknowledged");
  assert.deepEqual(nearAiNoticeRecovery(null, copy, false), { kind: "none" });
  assert.deepEqual(nearAiNoticeRecovery("ingest-unreachable", copy, false), {
    kind: "none",
  });
});

test("the confirmation is offered only with the shared notice loaded", () => {
  assert.deepEqual(
    nearAiNoticeRecovery(NEAR_AI_NOTICE_LABEL, copy, false),
    { kind: "ready", copy },
  );
  assert.deepEqual(
    nearAiNoticeRecovery(NEAR_AI_NOTICE_LABEL, undefined, false),
    { kind: "loading" },
  );
  assert.deepEqual(
    nearAiNoticeRecovery(NEAR_AI_NOTICE_LABEL, undefined, true),
    { kind: "unavailable" },
  );
});
