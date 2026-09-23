import { invokeTauri } from "./core-api";

type RecordValue = Record<string, unknown>;

function record(value: unknown, label: string): RecordValue {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`Invalid ${label} response`);
  }
  return value as RecordValue;
}

function string(value: RecordValue, key: string): string {
  if (typeof value[key] !== "string" || value[key].length === 0) {
    throw new Error(`Invalid contributor copy field: ${key}`);
  }
  return value[key] as string;
}

export type WitnessReviewCopy = {
  heading: string;
  disclosure: string;
  action: string;
  confirm: string;
  cancel: string;
  working: string;
  failed: string;
  failed_host_not_allowed: string;
  failed_measurement_unpinned: string;
  failed_unreachable: string;
  failed_unproven: string;
  failed_bodies_returned: string;
  failed_too_large: string;
  failed_not_connected: string;
  failed_receipt_declined: string;
  immutable: string;
};

export type EligibilityCopy = {
  state_line: string;
  reason_line: string;
  can_contribute: boolean;
};

export type EligibilityGroupCopy = {
  can_contribute: boolean;
  eligible_count: number;
  withheld_line: string;
};

export type OutcomeCopy = {
  verdict_question: string;
  worked: string;
  partly: string;
  failed: string;
  verdict_caption: string;
  correction_question: string;
  correction_placeholder: string;
  correction_caption: string;
  correction_credential_headline: string;
  correction_credential_body: string;
  submit_all_as: string;
  submit_all_as_tooltip: string;
  max_correction_chars: number;
};

export type ProjectIgnoreCopy = {
  title: string;
  body: string;
  button: string;
  tooltip: string;
};

export type RedactionSummaryRow = {
  family: string;
  display: string;
  description: string;
  occurrences: number;
  distinct: number;
  detail: string[];
};

export type RedactionSummary = {
  removed: RedactionSummaryRow[];
  still_present: RedactionSummaryRow[];
};

export type ArmingOfferCopy = {
  evidence: string;
  question: string;
  confirm: string;
  decline: string;
  body: string;
};

export type ContributorDisclosureCopy = {
  wallet: { heading: string; disclosure: string; commons: string; account: string };
  admission: {
    heading: string;
    disclosure: string;
    prerequisite: string;
    backend: string;
    confirm: string;
    cancel: string;
    permission: string;
    working: string;
    ready: string;
    failed: string;
  };
  private_inference: {
    destination: string;
    subtitle: string;
    settings_title: string;
    write_unconfirmed: string;
    offer_title: string;
    offer_what: string;
    offer_exposure: string;
    offer_no_repoint: string;
    offer_accept: string;
    offer_decline: string;
    offer_asked_once: string;
  };
  credential_cost: string;
  credential_wallet_notice: string;
  near_ai_enroll: {
    title: string;
    what: string;
    action: string;
    needs_login: string;
  };
  onboarding: {
    heading: string;
    start: string;
    review: string;
    follow_up: string;
    agent_setup: string;
  };
  onboarding_shell: {
    welcome_body: string;
    done_body: string;
    notification_purpose: string;
    notification_heading: string;
    notification_offer: string;
    notification_allowed: string;
    notification_denied: string;
    notification_unknown: string;
    notification_not_asked: string;
    notification_allow: string;
    not_now: string;
    system_settings: string;
  };
  source_settings: {
    heading: string;
    explanation: string;
    save_failed: string;
    consent_save_failed: string;
    unavailable: string;
    selected_folder: string;
    no_candidate: string;
    watch_candidate: string;
    choose_folder: string;
    retry: string;
    opencode_version_title: string;
    opencode_version_detail: string;
    tools: Record<
      string,
      {
        key: string;
        decline: string;
        unset_scans_conventional: boolean;
        explanation?: string;
        choose_folder?: string;
      }
    >;
  };
  source_check_lines: Record<
    string,
    { watch: string; unset: string; off: string }
  >;
  insights_ui: {
    delete: string;
    delete_confirm: string;
    cancel: string;
    working: string;
  };
  mission_drafts_ui: {
    delete: string;
    delete_confirm: string;
    delete_confirm_title: string;
    cancel: string;
    working: string;
  };
  history_ui: {
    held_row_body: string;
    status_awaiting_pii_backstop: string;
  };
  outcome: OutcomeCopy;
};

function parseWitnessReview(value: unknown): WitnessReviewCopy {
  const item = record(value, "witness review copy");
  return {
    heading: string(item, "heading"),
    disclosure: string(item, "disclosure"),
    action: string(item, "action"),
    confirm: string(item, "confirm"),
    cancel: string(item, "cancel"),
    working: string(item, "working"),
    failed: string(item, "failed"),
    failed_host_not_allowed: string(item, "failed_host_not_allowed"),
    failed_measurement_unpinned: string(item, "failed_measurement_unpinned"),
    failed_unreachable: string(item, "failed_unreachable"),
    failed_unproven: string(item, "failed_unproven"),
    failed_bodies_returned: string(item, "failed_bodies_returned"),
    failed_too_large: string(item, "failed_too_large"),
    failed_not_connected: string(item, "failed_not_connected"),
    failed_receipt_declined: string(item, "failed_receipt_declined"),
    immutable: string(item, "immutable"),
  };
}

export async function getWitnessReviewCopy(): Promise<WitnessReviewCopy> {
  return parseWitnessReview(await invokeTauri("witness_review_copy"));
}

export async function getContributorDisclosureCopy(): Promise<ContributorDisclosureCopy> {
  const value = record(
    await invokeTauri("contributor_disclosure_copy"),
    "contributor disclosure copy",
  );
  const wallet = record(value.wallet, "wallet copy");
  const admission = record(value.admission, "admission copy");
  const privateInference = record(value.private_inference, "Private AI copy");
  const onboarding = record(value.onboarding, "first contribution copy");
  const onboardingShell = record(value.onboarding_shell, "onboarding copy");
  const sourceSettings = record(value.source_settings, "source settings copy");
  const sourceTools = record(sourceSettings.tools, "source settings tools");
  const sourceCheckPayload = record(value.source_check_lines, "source check lines");
  const insightsUi = record(value.insights_ui, "Insights UI copy");
  const missionDraftsUi = record(value.mission_drafts_ui, "mission drafts UI copy");
  const historyUi = record(value.history_ui, "history UI copy");
  const outcome = record(value.outcome, "outcome and correction copy");
  if (typeof outcome.max_correction_chars !== "number") {
    throw new Error("Invalid maximum correction length");
  }
  return {
    wallet: {
      heading: string(wallet, "heading"),
      disclosure: string(wallet, "disclosure"),
      commons: string(wallet, "commons"),
      account: string(wallet, "account"),
    },
    admission: {
      heading: string(admission, "heading"),
      disclosure: string(admission, "disclosure"),
      prerequisite: string(admission, "prerequisite"),
      backend: string(admission, "backend"),
      confirm: string(admission, "confirm"),
      cancel: string(admission, "cancel"),
      permission: string(admission, "permission"),
      working: string(admission, "working"),
      ready: string(admission, "ready"),
      failed: string(admission, "failed"),
    },
    private_inference: {
      destination: string(privateInference, "destination"),
      subtitle: string(privateInference, "subtitle"),
      settings_title: string(privateInference, "settings_title"),
      write_unconfirmed: string(privateInference, "write_unconfirmed"),
      offer_title: string(privateInference, "offer_title"),
      offer_what: string(privateInference, "offer_what"),
      offer_exposure: string(privateInference, "offer_exposure"),
      offer_no_repoint: string(privateInference, "offer_no_repoint"),
      offer_accept: string(privateInference, "offer_accept"),
      offer_decline: string(privateInference, "offer_decline"),
      offer_asked_once: string(privateInference, "offer_asked_once"),
    },
    credential_cost: string(value, "credential_cost"),
    credential_wallet_notice: string(value, "credential_wallet_notice"),
    near_ai_enroll: {
      title: string(value, "near_ai_enroll_title"),
      what: string(value, "near_ai_enroll_what"),
      action: string(value, "near_ai_enroll_action"),
      needs_login: string(value, "near_ai_enroll_needs_login"),
    },
    onboarding: {
      heading: string(onboarding, "heading"),
      start: string(onboarding, "start"),
      review: string(onboarding, "review"),
      follow_up: string(onboarding, "follow_up"),
      agent_setup: string(onboarding, "agent_setup"),
    },
    onboarding_shell: {
      welcome_body: string(onboardingShell, "welcome_body"),
      done_body: string(onboardingShell, "done_body"),
      notification_purpose: string(onboardingShell, "notification_purpose"),
      notification_heading: string(onboardingShell, "notification_heading"),
      notification_offer: string(onboardingShell, "notification_offer"),
      notification_allowed: string(onboardingShell, "notification_allowed"),
      notification_denied: string(onboardingShell, "notification_denied"),
      notification_unknown: string(onboardingShell, "notification_unknown"),
      notification_not_asked: string(onboardingShell, "notification_not_asked"),
      notification_allow: string(onboardingShell, "notification_allow"),
      not_now: string(onboardingShell, "not_now"),
      system_settings: string(onboardingShell, "system_settings"),
    },
    source_settings: {
      heading: string(sourceSettings, "heading"),
      explanation: string(sourceSettings, "explanation"),
      save_failed: string(sourceSettings, "save_failed"),
      consent_save_failed: string(sourceSettings, "consent_save_failed"),
      unavailable: string(sourceSettings, "unavailable"),
      selected_folder: string(sourceSettings, "selected_folder"),
      no_candidate: string(sourceSettings, "no_candidate"),
      watch_candidate: string(sourceSettings, "watch_candidate"),
      choose_folder: string(sourceSettings, "choose_folder"),
      retry: string(sourceSettings, "retry"),
      opencode_version_title: string(sourceSettings, "opencode_version_title"),
      opencode_version_detail: string(sourceSettings, "opencode_version_detail"),
      tools: Object.fromEntries(
        Object.entries(sourceTools).map(([adapter, rawTool]) => {
          const tool = record(rawTool, `source tool ${adapter}`);
          if (typeof tool.unset_scans_conventional !== "boolean") {
            throw new Error(`Invalid source tool policy: ${adapter}`);
          }
          return [
            adapter,
            {
              key: string(tool, "key"),
              decline: string(tool, "decline"),
              unset_scans_conventional: tool.unset_scans_conventional,
              ...(typeof tool.explanation === "string"
                ? { explanation: tool.explanation }
                : {}),
              ...(typeof tool.choose_folder === "string"
                ? { choose_folder: tool.choose_folder }
                : {}),
            },
          ];
        }),
      ),
    },
    source_check_lines: Object.fromEntries(
      Object.entries(sourceCheckPayload).map(([source, rawLines]) => {
        const lines = record(rawLines, `source check lines for ${source}`);
        return [
          source,
          {
            watch: string(lines, "watch"),
            unset: string(lines, "unset"),
            off: string(lines, "off"),
          },
        ];
      }),
    ),
    insights_ui: {
      delete: string(insightsUi, "delete"),
      delete_confirm: string(insightsUi, "delete_confirm"),
      cancel: string(insightsUi, "cancel"),
      working: string(insightsUi, "working"),
    },
    mission_drafts_ui: {
      delete: string(missionDraftsUi, "delete"),
      delete_confirm: string(missionDraftsUi, "delete_confirm"),
      delete_confirm_title: string(missionDraftsUi, "delete_confirm_title"),
      cancel: string(missionDraftsUi, "cancel"),
      working: string(missionDraftsUi, "working"),
    },
    history_ui: {
      held_row_body: string(historyUi, "held_row_body"),
      status_awaiting_pii_backstop: string(historyUi, "status_awaiting_pii_backstop"),
    },
    outcome: {
      verdict_question: string(outcome, "verdict_question"),
      worked: string(outcome, "worked"),
      partly: string(outcome, "partly"),
      failed: string(outcome, "failed"),
      verdict_caption: string(outcome, "verdict_caption"),
      correction_question: string(outcome, "correction_question"),
      correction_placeholder: string(outcome, "correction_placeholder"),
      correction_caption: string(outcome, "correction_caption"),
      correction_credential_headline: string(outcome, "correction_credential_headline"),
      correction_credential_body: string(outcome, "correction_credential_body"),
      submit_all_as: string(outcome, "submit_all_as"),
      submit_all_as_tooltip: string(outcome, "submit_all_as_tooltip"),
      max_correction_chars: outcome.max_correction_chars,
    },
  };
}

export async function getProjectIgnoreCopy(
  projectLabel: string,
  pending: number,
): Promise<ProjectIgnoreCopy> {
  const value = record(
    await invokeTauri("project_ignore_copy", { projectLabel, pending }),
    "project ignore copy",
  );
  return {
    title: string(value, "title"),
    body: string(value, "body"),
    button: string(value, "button"),
    tooltip: string(value, "tooltip"),
  };
}

export async function getArmingOfferCopy(
  projectLabel: string,
  count: number,
): Promise<ArmingOfferCopy> {
  const value = record(
    await invokeTauri("arming_offer_copy", { projectLabel, count }),
    "arming offer copy",
  );
  return {
    evidence: string(value, "evidence"),
    question: string(value, "question"),
    confirm: string(value, "confirm"),
    decline: string(value, "decline"),
    body: string(value, "body"),
  };
}

export async function getResidualSecretLine(
  count: number,
  sites: string[],
): Promise<string> {
  const value = await invokeTauri("residual_secret_line", { count, sites });
  if (typeof value !== "string" || value.length === 0) {
    throw new Error("Invalid residual secret disclosure");
  }
  return value;
}

export async function getRedactionSummary(
  redactions: Record<string, number>,
  distinct: Record<string, number>,
): Promise<RedactionSummary> {
  const value = record(
    await invokeTauri("redaction_summary_copy", { redactions, distinct }),
    "redaction summary copy",
  );
  const rows = (input: unknown, label: string): RedactionSummaryRow[] => {
    if (!Array.isArray(input)) throw new Error(`Invalid ${label}`);
    return input.map((raw) => {
      const row = record(raw, label);
      if (
        typeof row.occurrences !== "number" ||
        typeof row.distinct !== "number" ||
        !Array.isArray(row.detail) ||
        !row.detail.every((part) => typeof part === "string")
      ) {
        throw new Error(`Invalid ${label} row`);
      }
      return {
        family: string(row, "family"),
        display: string(row, "display"),
        description: string(row, "description"),
        occurrences: row.occurrences,
        distinct: row.distinct,
        detail: row.detail as string[],
      };
    });
  };
  return {
    removed: rows(value.removed, "removed redaction rows"),
    still_present: rows(value.still_present, "surviving redaction rows"),
  };
}

export async function getEligibilityCopy(
  label: string,
  reason: string | null,
): Promise<EligibilityCopy> {
  const value = record(
    await invokeTauri("eligibility_copy", { label, reason }),
    "eligibility copy",
  );
  if (typeof value.can_contribute !== "boolean") {
    throw new Error("Invalid eligibility control");
  }
  return {
    state_line: string(value, "state_line"),
    reason_line:
      typeof value.reason_line === "string" ? value.reason_line : "",
    can_contribute: value.can_contribute,
  };
}

export async function getEligibilityGroupCopy(
  pending: number,
  contributable: number | null,
): Promise<EligibilityGroupCopy> {
  const value = record(
    await invokeTauri("eligibility_group_copy", { pending, contributable }),
    "eligibility group copy",
  );
  if (
    typeof value.can_contribute !== "boolean" ||
    typeof value.eligible_count !== "number" ||
    typeof value.withheld_line !== "string"
  ) {
    throw new Error("Invalid eligibility group response");
  }
  return {
    can_contribute: value.can_contribute,
    eligible_count: value.eligible_count,
    withheld_line: value.withheld_line,
  };
}

export async function getWithdrawalConfirmationPrompt(): Promise<string> {
  const value = await invokeTauri("withdrawal_confirmation_prompt");
  if (typeof value !== "string" || value.length === 0) {
    throw new Error("Invalid withdrawal confirmation copy");
  }
  return value;
}
