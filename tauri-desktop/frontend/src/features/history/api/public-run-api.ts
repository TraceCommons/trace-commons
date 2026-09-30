import { invokeTauri, invokeTauriVoid } from "../../../lib/tauri/core-api";
import type {
  PublicRunDraft,
  PublicRunEditorInput,
  PublicRunPage,
  PublicRunValidation,
} from "../types";

function record(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error(`Invalid public run ${label}`);
  return value as Record<string, unknown>;
}

function stringField(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "string")
    throw new Error(`Invalid public run field: ${key}`);
  return value[key] as string;
}

function nullableString(value: Record<string, unknown>, key: string) {
  if (value[key] === undefined || value[key] === null) return null;
  return stringField(value, key);
}

function numberField(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "number" || !Number.isInteger(value[key]))
    throw new Error(`Invalid public run field: ${key}`);
  return value[key] as number;
}

function permission(value: unknown) {
  if (value === "cc_by_4_0" || value === "cc0_1_0") return value;
  throw new Error("Invalid public run permission");
}

function parsePage(value: unknown): PublicRunPage {
  const item = record(value, "page");
  const evidence = Array.isArray(item.evidence)
    ? item.evidence.map((entry) => ({
        excerpt: stringField(record(entry, "evidence"), "excerpt"),
      }))
    : [];
  const sourceValue = item.source;
  const source =
    sourceValue === undefined || sourceValue === null
      ? null
      : (() => {
          const sourceItem = record(sourceValue, "source");
          return {
            slug: stringField(sourceItem, "slug"),
            title: stringField(sourceItem, "title"),
          };
        })();
  const variations = Array.isArray(item.variations)
    ? item.variations.map((entry) => {
        const variation = record(entry, "variation");
        return {
          slug: stringField(variation, "slug"),
          title: stringField(variation, "title"),
        };
      })
    : [];
  if (
    item.source_unavailable !== undefined &&
    typeof item.source_unavailable !== "boolean"
  )
    throw new Error("Invalid public run source state");
  return {
    slug: stringField(item, "slug"),
    title: stringField(item, "title"),
    outcome_summary: stringField(item, "outcome_summary"),
    correction_excerpt: nullableString(item, "correction_excerpt"),
    workflow: stringField(item, "workflow"),
    reuse_permission: permission(item.reuse_permission),
    evidence,
    task_success: stringField(item, "task_success"),
    contributed_version: stringField(item, "contributed_version"),
    version: numberField(item, "version"),
    published_at: stringField(item, "published_at"),
    public_url: nullableString(item, "public_url"),
    source,
    source_unavailable: item.source_unavailable === true,
    variations,
    credential_warning: nullableString(item, "credential_warning"),
  };
}

export async function validatePublicRunEditor(
  input: PublicRunEditorInput,
): Promise<PublicRunValidation> {
  const value = record(
    await invokeTauri("validate_public_run_editor", { input }),
    "validation",
  );
  const draftValue = value.draft;
  let draft: PublicRunDraft | null = null;
  if (draftValue !== undefined && draftValue !== null) {
    const item = record(draftValue, "draft");
    const evidence = Array.isArray(item.evidence)
      ? item.evidence.map((entry) => {
          const evidenceItem = record(entry, "draft evidence");
          return {
            event_id: stringField(evidenceItem, "event_id"),
            excerpt: stringField(evidenceItem, "excerpt"),
          };
        })
      : [];
    draft = {
      title: stringField(item, "title"),
      outcome_summary: stringField(item, "outcome_summary"),
      correction_excerpt: nullableString(item, "correction_excerpt"),
      workflow: stringField(item, "workflow"),
      reuse_permission: permission(item.reuse_permission),
      evidence,
      source_slug: nullableString(item, "source_slug"),
    };
  }
  return { draft, error: nullableString(value, "error") };
}

export async function publishPublicRun(
  submissionId: string,
  draft: PublicRunDraft,
  taskSuccess: string,
  contributedVersion: string,
  expectedPublicationVersion: number,
) {
  return parsePage(
    await invokeTauri("publish_public_run", {
      submissionId,
      draft,
      taskSuccess,
      contributedVersion,
      expectedPublicationVersion,
    }),
  );
}

export async function unpublishPublicRun(submissionId: string) {
  const value = record(
    await invokeTauri("unpublish_public_run", { submissionId }),
    "unpublish",
  );
  if (typeof value.unpublished !== "boolean")
    throw new Error("Invalid public run unpublish response");
  return {
    unpublished: value.unpublished,
    expected_publication_version: numberField(
      value,
      "expected_publication_version",
    ),
    credential_warning: nullableString(value, "credential_warning"),
  };
}

export async function openExternalUrl(url: string) {
  return invokeTauriVoid("open_external_url", { url });
}

export { parsePage as parsePublicRunPage };
