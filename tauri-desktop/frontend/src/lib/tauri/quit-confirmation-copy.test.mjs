import assert from "node:assert/strict";
import { test } from "node:test";
import { parseQuitConfirmationCopy } from "./quit-confirmation-copy.ts";

const prompt = (body) => ({
  title: "Quit Near.ai?",
  body,
  confirm: "Quit",
  cancel: "Cancel",
});

test("null means quit without asking", () => {
  assert.equal(parseQuitConfirmationCopy(null), null);
});

test("a prompt keeps the sentence Rust chose", () => {
  const parsed = parseQuitConfirmationCopy(
    prompt("Quitting stops compute on this computer."),
  );
  assert.deepEqual(parsed, prompt("Quitting stops compute on this computer."));
});

test("a missing sentence is refused rather than guessed", () => {
  assert.throws(() => parseQuitConfirmationCopy(prompt("")));
  assert.throws(() => parseQuitConfirmationCopy({ title: "Quit Near.ai?" }));
  assert.throws(() => parseQuitConfirmationCopy(undefined));
  assert.throws(() => parseQuitConfirmationCopy([]));
});
