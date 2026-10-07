import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { parseConsentOptions } from "./consent-options.ts";

// Stand-ins, not the core's titles: the shell must draw whichever title the
// core sent, so the words themselves do not matter here.
const scope = (name, extra = {}) => ({
  name,
  title: `TITLE OF ${name}`,
  description: `DESCRIPTION OF ${name}`,
  always_on: name === "debugging_evaluation",
  grants_data_use: name !== "public_attribution",
  ...extra,
});

test("each scope carries the core's title", () => {
  const options = parseConsentOptions({
    scopes: [scope("debugging_evaluation"), scope("public_attribution")],
  });
  assert.deepEqual(
    options.map((option) => [option.name, option.title]),
    [
      ["debugging_evaluation", "TITLE OF debugging_evaluation"],
      ["public_attribution", "TITLE OF public_attribution"],
    ],
  );
});

test("a list carrying one scope without a title is refused whole", () => {
  const untitled = scope("benchmark_only");
  delete untitled.title;
  assert.throws(
    () =>
      parseConsentOptions({
        scopes: [scope("debugging_evaluation"), untitled],
      }),
    /title/,
  );
});

test("an empty or non-string title is refused, never filled in", () => {
  for (const title of ["", "   ", null, 7]) {
    assert.throws(
      () =>
        parseConsentOptions({
          scopes: [scope("debugging_evaluation", { title })],
        }),
      /title/,
    );
  }
});

test("neither consent surface draws a scope's wire name as its label", () => {
  for (const path of [
    "src/features/settings/components/consent-settings-panel.tsx",
    "src/features/onboarding/components/onboarding-consent-step.tsx",
  ]) {
    const source = readFileSync(path, "utf8");
    assert.ok(source.includes("{option.title}"), path);
    assert.ok(!source.includes("option.name.replaceAll"), path);
  }
});
