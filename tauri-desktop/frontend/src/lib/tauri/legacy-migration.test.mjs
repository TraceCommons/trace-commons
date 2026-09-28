import assert from "node:assert/strict";
import { test } from "node:test";
import {
  parseLegacyMigrationNotice,
  parseLegacyMigrationOffer,
  parseLegacyMigrationResult,
  parseLegacyMigrationStatus,
} from "./legacy-migration.ts";

test("a core too old to report the move offers nothing and shows nothing", () => {
  assert.deepEqual(parseLegacyMigrationStatus(undefined), {
    offered: false,
    notice: null,
  });
});

test("the offer and the notice are read as sent", () => {
  assert.deepEqual(
    parseLegacyMigrationStatus({ offered: true, notice: null }),
    { offered: true, notice: null },
  );
  const notice = { folders_kept: 2, automatic_grant_kept: false };
  assert.deepEqual(parseLegacyMigrationStatus({ offered: false, notice }), {
    offered: false,
    notice,
  });
});

test("a malformed status is refused rather than read as nothing to show", () => {
  assert.throws(() => parseLegacyMigrationStatus({ offered: "yes" }));
  assert.throws(() =>
    parseLegacyMigrationStatus({ offered: false, notice: 3 }),
  );
  assert.throws(() => parseLegacyMigrationStatus([]));
});

test("the core's words are required in full", () => {
  const offer = {
    title: "t",
    body: "b",
    action: "a",
    working: "w",
    invite_prompt: "p",
  };
  assert.deepEqual(parseLegacyMigrationOffer(offer), offer);
  assert.throws(() => parseLegacyMigrationOffer({ ...offer, action: "" }));
  const notice = { title: "t", body: "b", folders: "f", acknowledge: "k" };
  assert.deepEqual(parseLegacyMigrationNotice(notice), notice);
  assert.equal(parseLegacyMigrationNotice(null), null);
  assert.throws(() => parseLegacyMigrationNotice({ title: "t" }));
});

test("a refusal is an answer with the core's sentence, and asking for the invite is one of them", () => {
  assert.deepEqual(
    parseLegacyMigrationResult({ migrated: true, folders_kept: 1 }),
    {
      kind: "migrated",
    },
  );
  assert.deepEqual(
    parseLegacyMigrationResult({
      refused: "legacy_migration_invite_needed",
      line: "Paste your invite link.",
    }),
    { kind: "invite_needed", line: "Paste your invite link." },
  );
  assert.deepEqual(
    parseLegacyMigrationResult({
      refused: "legacy_migration_tenant_pooled",
      line: "It keeps working.",
    }),
    {
      kind: "refused",
      label: "legacy_migration_tenant_pooled",
      line: "It keeps working.",
    },
  );
  assert.throws(() => parseLegacyMigrationResult({ refused: "x" }));
  assert.throws(() => parseLegacyMigrationResult({}));
});
