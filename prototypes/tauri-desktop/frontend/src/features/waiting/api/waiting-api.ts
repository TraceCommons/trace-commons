import { daemonCall, invokeTauri } from "../../../lib/tauri/core-api";
import type {
  ApprovalResult,
  QueueOutcomeCounts,
  WaitingData,
  WaitingEntry,
  WaitingPreview,
} from "../types";

function asRecord(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null)
    throw new Error("Invalid waiting response");
  return value as Record<string, unknown>;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function asString(record: Record<string, unknown>, key: string): string {
  if (typeof record[key] !== "string")
    throw new Error(`Invalid waiting field: ${key}`);
  return record[key] as string;
}

function asNumber(record: Record<string, unknown>, key: string): number {
  if (typeof record[key] !== "number")
    throw new Error(`Invalid waiting field: ${key}`);
  return record[key] as number;
}

function optionalString(record: Record<string, unknown>, key: string) {
  if (record[key] === undefined || record[key] === null) return null;
  return typeof record[key] === "string"
    ? record[key]
    : (() => {
        throw new Error(`Invalid waiting field: ${key}`);
      })();
}

function optionalNumber(record: Record<string, unknown>, key: string) {
  if (record[key] === undefined || record[key] === null) return undefined;
  if (typeof record[key] !== "number")
    throw new Error(`Invalid waiting field: ${key}`);
  return record[key];
}

function numberMap(value: unknown, label: string): Record<string, number> {
  if (
    !isRecord(value) ||
    Array.isArray(value) ||
    Object.values(value).some((item) => typeof item !== "number")
  )
    throw new Error(`Invalid waiting ${label}`);
  return value as Record<string, number>;
}

function parseEntry(value: unknown): WaitingEntry {
  const record = asRecord(value);
  return {
    entry_id: asString(record, "entry_id"),
    project_id: asString(record, "project_id"),
    project_label: asString(record, "project_label"),
    project_path: asString(record, "project_path"),
    source: asString(record, "source"),
    state: asString(record, "state"),
    size_bytes: asNumber(record, "size_bytes"),
    discovered_at: asString(record, "discovered_at"),
    subagent_count: asNumber(record, "subagent_count"),
    subagents_dropped: asNumber(record, "subagents_dropped"),
    declared_source: optionalString(record, "declared_source"),
    session_path: optionalString(record, "session_path"),
    reason_label: optionalString(record, "reason_label"),
    attempts: optionalNumber(record, "attempts"),
    eligibility: optionalString(record, "eligibility"),
    eligibility_reason: optionalString(record, "eligibility_reason"),
    attestation: optionalString(record, "attestation"),
    attestation_reason: optionalString(record, "attestation_reason"),
    holds_certificate:
      record.holds_certificate === undefined
        ? undefined
        : typeof record.holds_certificate === "boolean"
          ? record.holds_certificate
          : (() => {
              throw new Error("Invalid waiting field: holds_certificate");
            })(),
    attested_inference:
      record.attested_inference === undefined ||
      record.attested_inference === null
        ? null
        : parseAttestedInference(record.attested_inference),
  };
}

function parseAttestedInference(value: unknown) {
  const item = asRecord(value);
  if (typeof item.state !== "string")
    throw new Error("Invalid waiting attested inference");
  return { state: item.state, reason: optionalString(item, "reason") };
}

export async function getWaitingData(): Promise<WaitingData> {
  const response = asRecord(await daemonCall<unknown>("list_pending"));
  if (!Array.isArray(response.pending))
    throw new Error("Invalid waiting response");
  return { pending: response.pending.map(parseEntry) };
}

function parsePreview(value: unknown): WaitingPreview {
  const record = asRecord(value);
  const entry = parseEntry(record.entry);
  if (
    typeof record.would_send_bytes !== "number" ||
    typeof record.raw_session_bytes !== "number" ||
    typeof record.event_count !== "number" ||
    typeof record.opening_prompt !== "string" ||
    typeof record.residual_risk !== "string" ||
    typeof record.input_fingerprint !== "string" ||
    typeof record.enrolled !== "boolean"
  ) {
    throw new Error("Invalid preview response");
  }
  if (
    !Array.isArray(record.pii_labels_present) ||
    !record.pii_labels_present.every((item) => typeof item === "string")
  )
    throw new Error("Invalid preview labels");
  if (
    !Array.isArray(record.consent_scopes) ||
    !record.consent_scopes.every((item) => typeof item === "string")
  )
    throw new Error("Invalid preview scopes");
  if (
    !isRecord(record.redactions) ||
    Object.values(record.redactions).some((item) => typeof item !== "number")
  )
    throw new Error("Invalid preview redactions");
  return {
    would_send_bytes: record.would_send_bytes,
    raw_session_bytes: record.raw_session_bytes,
    event_count: record.event_count,
    opening_prompt: record.opening_prompt,
    redactions: record.redactions as Record<string, number>,
    redactions_distinct:
      record.redactions_distinct === undefined
        ? {}
        : numberMap(record.redactions_distinct, "redactions"),
    pii_labels_present: record.pii_labels_present as string[],
    consent_scopes: record.consent_scopes as string[],
    residual_risk: record.residual_risk,
    input_fingerprint: record.input_fingerprint,
    enrolled: record.enrolled,
    subagent_count:
      optionalNumber(record, "subagent_count") ?? entry.subagent_count,
    subagents_dropped:
      optionalNumber(record, "subagents_dropped") ?? entry.subagents_dropped,
    entry,
  } as WaitingPreview;
}

export async function previewWaitingEntry(entryId: string) {
  return parsePreview(await invokeTauri<unknown>("preview_entry", { entryId }));
}

export async function dismissWaitingEntry(entryId: string) {
  return invokeTauri<{ ok: boolean }>("dismiss_entry", { entryId });
}

function parseApproval(value: unknown) {
  const response = asRecord(value);
  if (typeof response.approved !== "number" || !Array.isArray(response.skipped))
    throw new Error("Invalid approval response");
  if (
    response.excluded_ineligible !== undefined &&
    typeof response.excluded_ineligible !== "number"
  )
    throw new Error("Invalid approval response");
  return response as unknown as ApprovalResult;
}

export async function approveWaitingEntry(entryId: string) {
  return parseApproval(
    await invokeTauri<unknown>("approve_entry", { entryId }),
  );
}

export async function approveWaitingProject(projectId: string) {
  return parseApproval(
    await invokeTauri<unknown>("approve_project", { projectId }),
  );
}

export async function getQueueOutcomeCounts(): Promise<QueueOutcomeCounts> {
  const response = asRecord(await daemonCall<unknown>("queue_outcome_counts"));
  const reasons = numberMap(response.reasons, "outcome counts");
  const lines = Object.fromEntries(
    await Promise.all(
      Object.keys(reasons).map(async (label) => {
        const value = await invokeTauri<unknown>("queue_outcome_line", {
          label,
        });
        if (typeof value !== "string")
          throw new Error("Invalid queue outcome line");
        return [label, value] as const;
      }),
    ),
  );
  return { reasons, lines };
}
