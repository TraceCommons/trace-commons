import { invokeTauri } from "../../../lib/tauri/core-api";
import type {
  MissionDraft,
  MissionDraftReview,
  MissionDraftSummary,
} from "../types";

function record(value: unknown, label: string) {
  if (typeof value !== "object" || value === null)
    throw new Error(`Invalid mission ${label}`);
  return value as Record<string, unknown>;
}
function string(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "string")
    throw new Error(`Invalid mission field: ${key}`);
  return value[key] as string;
}
function number(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "number")
    throw new Error(`Invalid mission field: ${key}`);
  return value[key] as number;
}
function strings(value: unknown, key: string) {
  if (!Array.isArray(value) || !value.every((item) => typeof item === "string"))
    throw new Error(`Invalid mission field: ${key}`);
  return value as string[];
}
function review(value: unknown): MissionDraftReview {
  const item = record(value, "review");
  return {
    proposal_sha256: string(item, "proposal_sha256"),
    status: string(item, "status"),
    publication_authorized: item.publication_authorized === true,
    external_sources_verified: item.external_sources_verified === true,
    required_reviews: strings(item.required_reviews, "required_reviews"),
  };
}
function parseSummary(value: unknown): MissionDraftSummary {
  const item = record(value, "draft summary");
  return {
    id: string(item, "id"),
    source_count: number(item, "source_count"),
    status: string(item, "status"),
  };
}
function parseDraft(value: unknown): MissionDraft {
  const item = record(value, "draft");
  const proposal = record(item.proposal, "proposal");
  const artifact = record(proposal.starting_artifact, "starting_artifact");
  const budget = record(proposal.budget, "budget");
  return {
    id: string(item, "id"),
    review: review(item.review),
    proposal: {
      author_id: string(proposal, "author_id"),
      title: string(proposal, "title"),
      source_urls: strings(proposal.source_urls, "source_urls"),
      claim_to_test: string(proposal, "claim_to_test"),
      task: string(proposal, "task"),
      starting_artifact: {
        url: string(artifact, "url"),
        sha256: string(artifact, "sha256"),
      },
      evaluator_id: string(proposal, "evaluator_id"),
      rubric_version: string(proposal, "rubric_version"),
      success_criteria: strings(proposal.success_criteria, "success_criteria"),
      required_evidence: strings(
        proposal.required_evidence,
        "required_evidence",
      ),
      allowed_models: strings(proposal.allowed_models, "allowed_models"),
      allowed_tools: strings(proposal.allowed_tools, "allowed_tools"),
      budget: {
        max_duration_seconds: number(budget, "max_duration_seconds"),
        max_input_tokens: number(budget, "max_input_tokens"),
        max_output_tokens: number(budget, "max_output_tokens"),
      },
    },
  };
}
export async function listMissionDrafts() {
  const response = record(
    await invokeTauri<unknown>("mission_draft_list"),
    "list",
  );
  if (!Array.isArray(response.drafts)) throw new Error("Invalid mission list");
  return response.drafts.map(parseSummary);
}
export async function showMissionDraft(id: string) {
  const response = record(
    await invokeTauri<unknown>("mission_draft_show", { id }),
    "show",
  );
  return parseDraft(response.draft);
}
export async function importMissionDraft(file: File) {
  const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
  const response = record(
    await invokeTauri<unknown>("mission_draft_import", { fileBytes: bytes }),
    "import",
  );
  return review(response.draft);
}
export async function deleteMissionDraft(id: string) {
  const response = record(
    await invokeTauri<unknown>("mission_draft_delete", { id }),
    "delete",
  );
  if (record(response.draft, "delete").deleted !== true)
    throw new Error("Mission draft was not deleted");
}
