export type AttestationCopy = {
  state_line: string;
  reason_line: string | null;
  tone: "neutral" | "held" | "clear" | "attention" | "refused";
};

export type CertificateCopy = {
  list_title: string;
  row_line: string;
  list_empty: string;
};

export type WaitingEntry = {
  entry_id: string;
  project_id: string;
  project_label: string;
  project_path: string;
  source: string;
  state: string;
  size_bytes: number;
  discovered_at: string;
  subagent_count: number;
  subagents_dropped: number;
  declared_source?: string | null;
  session_path?: string | null;
  reason_label?: string | null;
  attempts?: number;
  eligibility?: string | null;
  eligibility_reason?: string | null;
  attestation?: string | null;
  attestation_reason?: string | null;
  attestation_copy?: AttestationCopy | null;
  holds_certificate?: boolean;
  attested_inference?: { state: string; reason?: string | null } | null;
};

export type WaitingData = { pending: WaitingEntry[] };

export type OutcomeVerdict = "worked" | "partly" | "failed";

export type WaitingPreview = {
  would_send_bytes: number;
  raw_session_bytes: number;
  event_count: number;
  opening_prompt: string;
  redactions: Record<string, number>;
  redactions_distinct: Record<string, number>;
  pii_labels_present: string[];
  consent_scopes: string[];
  residual_risk: string;
  gate_statement: string;
  input_fingerprint: string;
  enrolled: boolean;
  subagent_count: number;
  subagents_dropped: number;
  entry: WaitingEntry;
};

export type QueueOutcomeCounts = {
  reasons: Record<string, number>;
  lines: Record<string, string>;
};

export type ApprovalResult = {
  approved: number;
  hold_until: string | null;
  flagged: number;
  redactions: Record<string, number>;
  skipped: Array<{ entry_id: string; reason_label: string }>;
  excluded_ineligible?: number;
};
