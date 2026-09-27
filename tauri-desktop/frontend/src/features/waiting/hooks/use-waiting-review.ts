import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import {
  useContributorDisclosureCopy,
  useEligibilityCopy,
} from "../../../lib/tauri/use-contributor-copy";
import { waitingKeys } from "../api/query-keys";
import type { UndoScope } from "../api/undo-api";
import {
  approveWaitingEntry,
  dismissWaitingEntry,
  previewWaitingEntry,
} from "../api/waiting-api";
import type { OutcomeVerdict, WaitingPreview } from "../types";

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Review hook coordinates preview, approval, dismissal, and undo handoff states.
export function useWaitingReview(onApproved?: (scope: UndoScope) => void) {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [credentialRefusal, setCredentialRefusal] = useState(false);
  const [verdict, setVerdict] = useState<OutcomeVerdict | null>(null);
  const [correction, setCorrection] = useState("");
  const disclosures = useContributorDisclosureCopy();
  const previewQuery = useQuery<WaitingPreview>({
    queryKey: waitingKeys.preview(core.scope, selectedId ?? "none"),
    queryFn: () => previewWaitingEntry(selectedId as string),
    enabled: core.isSuccess && selectedId !== null,
  });
  const eligibilityQuery = useEligibilityCopy(
    previewQuery.data?.entry.eligibility,
    previewQuery.data?.entry.eligibility_reason,
  );
  const dismissMutation = useMutation({
    mutationFn: (entryId: string) => dismissWaitingEntry(entryId),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: waitingKeys.list(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: waitingKeys.arming(core.scope),
        }),
        queryClient.invalidateQueries({
          queryKey: waitingKeys.privateInference(core.scope),
        }),
      ]);
    },
  });
  const approveMutation = useMutation({
    mutationFn: (input: {
      entryId: string;
      outcome: OutcomeVerdict | undefined;
      correction: string | undefined;
    }) => approveWaitingEntry(input.entryId, input.outcome, input.correction),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: waitingKeys.list(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: waitingKeys.arming(core.scope),
        }),
        queryClient.invalidateQueries({
          queryKey: waitingKeys.privateInference(core.scope),
        }),
      ]);
    },
  });

  const review = (entryId: string) => {
    setSelectedId(entryId);
    setActionError(null);
    setCredentialRefusal(false);
    setVerdict(null);
    setCorrection("");
  };

  const dismiss = async () => {
    if (!selectedId) return;
    setActionError(null);
    try {
      await dismissMutation.mutateAsync(selectedId);
      setSelectedId(null);
      setVerdict(null);
      setCorrection("");
    } catch {
      setActionError("Could not dismiss session.");
    }
  };

  const approve = async () => {
    const preview = previewQuery.data;
    if (!selectedId || !preview?.enrolled) return;
    if (
      preview.entry.eligibility &&
      eligibilityQuery.data?.can_contribute !== true
    ) {
      return;
    }
    const outcomeCopy = disclosures.data?.outcome;
    if (!outcomeCopy || correction.length > outcomeCopy.max_correction_chars) return;
    const correctionAllowed = verdict === "partly" || verdict === "failed";
    setActionError(null);
    setCredentialRefusal(false);
    try {
      const result = await approveMutation.mutateAsync({
        entryId: selectedId,
        outcome: verdict ?? undefined,
        correction:
          correctionAllowed && correction.length > 0 ? correction : undefined,
      });
      if (result.approved !== 1) throw new Error("Approval was not accepted");
      onApproved?.({
        kind: "entry",
        id: selectedId,
        hold_until: result.hold_until,
        label: preview.entry.project_label,
      });
      setSelectedId(null);
      setVerdict(null);
      setCorrection("");
    } catch (error) {
      const isCredentialRefusal =
        error instanceof Error &&
        error.message.includes("correction-credential-detected");
      setCredentialRefusal(isCredentialRefusal);
      setActionError(
        isCredentialRefusal
          ? null
          : "Could not approve session. Nothing is reported as sent.",
      );
    }
  };

  const clear = () => {
    setSelectedId(null);
    setActionError(null);
    setCredentialRefusal(false);
    setVerdict(null);
    setCorrection("");
  };

  const state =
    dismissMutation.isPending || approveMutation.isPending
      ? "acting"
      : selectedId === null
        ? "idle"
        : previewQuery.isPending
          ? "loading"
          : previewQuery.isError
            ? "error"
            : "ready";
  const error = actionError
    ? actionError
    : previewQuery.isError
      ? "The session file changed while it was being read. Nothing has been sent, and nothing will be until it can be shown to you."
      : null;

  return {
    ...previewQuery,
    selectedId,
    preview: previewQuery.data ?? null,
    eligibilityCopy: eligibilityQuery.data ?? null,
    eligibilityPending: eligibilityQuery.isPending,
    eligibilityError: eligibilityQuery.isError,
    outcomeCopy: disclosures.data?.outcome ?? null,
    outcomeCopyPending: disclosures.isPending,
    outcomeCopyError: disclosures.isError,
    verdict,
    correction,
    credentialRefusal,
    state: state as "idle" | "loading" | "ready" | "error" | "acting",
    error: actionError ?? error,
    errorKind: (actionError
      ? "action"
      : previewQuery.isError
        ? "preview"
        : null) as "action" | "preview" | null,
    review,
    dismiss,
    approve,
    setVerdict: (value: OutcomeVerdict | null) => {
      setVerdict(value);
      if (value !== "partly" && value !== "failed") setCorrection("");
    },
    setCorrection,
    clear,
    previewQuery,
    dismissMutation,
    approveMutation,
  };
}
