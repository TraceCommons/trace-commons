export type MissionDraftSummary = {
  id: string;
  source_count: number;
  status: string;
};
export type MissionDraftReview = {
  proposal_sha256: string;
  status: string;
  publication_authorized: boolean;
  external_sources_verified: boolean;
  required_reviews: string[];
};
export type MissionDraft = {
  id: string;
  proposal: {
    author_id: string;
    title: string;
    source_urls: string[];
    claim_to_test: string;
    task: string;
    starting_artifact: { url: string; sha256: string };
    evaluator_id: string;
    rubric_version: string;
    success_criteria: string[];
    required_evidence: string[];
    allowed_models: string[];
    allowed_tools: string[];
    budget: {
      max_duration_seconds: number;
      max_input_tokens: number;
      max_output_tokens: number;
    };
  };
  review: MissionDraftReview;
};
