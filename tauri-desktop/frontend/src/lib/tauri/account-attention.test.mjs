import assert from "node:assert/strict";
import { test } from "node:test";
import {
  accountAttentionWire,
  parseAccountAttentionCopy,
  parseSetUpAgainAnswer,
} from "./account-attention.ts";

const copy = {
  title: "t",
  body: "b",
  private_ai: null,
  action: "a",
  confirm_title: "ct",
  confirm_body: "cb",
  confirm_action: "ca",
  confirm_cancel: "cc",
  failed: "f",
};

test("only a needs_attention status is handed to the core", () => {
  assert.equal(accountAttentionWire({ account_state: "ok" }), null);
  assert.equal(accountAttentionWire({}), null);
  assert.equal(accountAttentionWire(undefined), null);
  assert.deepEqual(
    accountAttentionWire({
      account_state: "needs_attention",
      account_state_reason: "account_record_unreadable",
      near_ai_login_unconfirmed: true,
      logged_in: true,
    }),
    {
      account_state: "needs_attention",
      account_state_reason: "account_record_unreadable",
      near_ai_login_unconfirmed: true,
    },
  );
});

test("the core answering null means nothing to show", () => {
  assert.equal(parseAccountAttentionCopy(null), null);
});

test("the core's copy is read as given, with optional sentences", () => {
  assert.deepEqual(parseAccountAttentionCopy(copy), copy);
  const unknownReason = { ...copy, body: null, private_ai: "p" };
  assert.deepEqual(parseAccountAttentionCopy(unknownReason), unknownReason);
});

test("a malformed copy is refused, not drawn with blanks", () => {
  assert.throws(() => parseAccountAttentionCopy({ ...copy, title: "" }));
  assert.throws(() => parseAccountAttentionCopy({ ...copy, action: 1 }));
  assert.throws(() => parseAccountAttentionCopy("x"));
});

test("the set-up-again answer carries the core's left-behind sentence", () => {
  assert.deepEqual(parseSetUpAgainAnswer({ left_behind: null }), {
    leftBehind: null,
  });
  assert.deepEqual(parseSetUpAgainAnswer({ left_behind: "line" }), {
    leftBehind: "line",
  });
  assert.throws(() => parseSetUpAgainAnswer({}));
  assert.throws(() => parseSetUpAgainAnswer({ left_behind: 3 }));
});
