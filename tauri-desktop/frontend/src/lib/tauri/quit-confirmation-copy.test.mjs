import assert from "node:assert/strict";
import { test } from "node:test";
import { parseQuitConfirmationCopy } from "./quit-confirmation-copy.ts";

const prompt = (role, body) => ({
  role,
  title: "Quit Trace Commons?",
  body,
  confirm: "Quit",
  cancel: "Cancel",
});

test("each watcher role keeps the sentence Rust chose for it", () => {
  for (const role of ["hosting", "attached", "unavailable"]) {
    const parsed = parseQuitConfirmationCopy(prompt(role, `${role} body`));
    assert.equal(parsed.role, role);
    assert.equal(parsed.body, `${role} body`);
    assert.equal(parsed.title, "Quit Trace Commons?");
    assert.equal(parsed.confirm, "Quit");
    assert.equal(parsed.cancel, "Cancel");
  }
});

test("an unknown role or a missing sentence is refused rather than guessed", () => {
  assert.throws(() => parseQuitConfirmationCopy(prompt("embedded", "x")));
  assert.throws(() => parseQuitConfirmationCopy(prompt("hosting", "")));
  assert.throws(() => parseQuitConfirmationCopy({ role: "hosting" }));
  assert.throws(() => parseQuitConfirmationCopy(null));
  assert.throws(() => parseQuitConfirmationCopy([]));
});
