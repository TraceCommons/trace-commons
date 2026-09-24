import assert from "node:assert/strict";
import { test } from "node:test";
import {
  countHistory,
  contributorFacingExplanations,
  filterHistory,
  groupHistory,
  historyCreditLine,
  quarantineExplanations,
} from "./history-view-model.ts";

const records = [
  {
    project_id: "opaque-b",
    project_label: "Editor",
    status: "accepted",
    explanations: [],
  },
  {
    project_id: "opaque-a",
    project_label: "API",
    status: "quarantined",
    explanations: ["Privacy hold"],
  },
  {
    project_id: "opaque-b",
    project_label: "Editor",
    status: "submitted",
    explanations: [],
  },
  {
    project_id: "opaque-a",
    project_label: "API",
    status: "quarantined",
    explanations: ["Privacy hold", "Needs review"],
  },
];

test("history counts every record without losing first status", () => {
  assert.deepEqual(countHistory(records), {
    all: 4,
    accepted: 1,
    submitted: 1,
    quarantined: 2,
  });
  assert.deepEqual(countHistory([]), {});
  assert.equal(filterHistory(records, "accepted").length, 1);
});

test("history groups stable project IDs and deduplicates hold explanations", () => {
  assert.deepEqual(
    groupHistory(records).map(({ id, label, records }) => [
      id,
      label,
      records.length,
    ]),
    [
      ["opaque-b", "Editor", 2],
      ["opaque-a", "API", 2],
    ],
  );
  assert.deepEqual(quarantineExplanations(records), [
    "Privacy hold",
    "Needs review",
  ]);
});

test("history hides digest explanations and supplies held fallback only when needed", () => {
  const fallback =
    "Automated checks saw something that might be personal and couldn't decide on their own. It has not been rejected, and it has not been shared with anyone but the agent that inspects it.";
  const digestOnly = [
    {
      project_id: "opaque-a",
      project_label: "API",
      status: "quarantined",
      explanations: ["receipt sha256:abcdef"],
    },
  ];
  assert.deepEqual(
    contributorFacingExplanations([
      "Privacy hold",
      "tenant sha256:abcdef",
    ]),
    ["Privacy hold"],
  );
  assert.deepEqual(quarantineExplanations(digestOnly, fallback), [fallback]);
  assert.deepEqual(quarantineExplanations(records, fallback), [
    "Privacy hold",
    "Needs review",
  ]);
});

test("row credit states a settled figure alone", () => {
  assert.equal(historyCreditLine(4, 0), "credit 4.0");
  assert.equal(historyCreditLine(12.5, 3), "credit 12.5");
});

test("row credit still pending reads as still being scored, as the native shells do", () => {
  assert.equal(historyCreditLine(null, 3), "credit 3.0, still being scored");
  assert.equal(historyCreditLine(0, 3), "credit 3.0, still being scored");
  assert.equal(historyCreditLine(undefined, 0.25), "credit 0.3, still being scored");
});

test("a row with no credit states nothing rather than a zero", () => {
  assert.equal(historyCreditLine(null, 0), null);
  assert.equal(historyCreditLine(0, 0), null);
  assert.equal(historyCreditLine(null, undefined), null);
  assert.equal(historyCreditLine(null, Number.NaN), null);
});
