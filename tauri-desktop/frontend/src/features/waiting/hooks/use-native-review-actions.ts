import { useMutation, useQuery } from "@tanstack/react-query";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { useSettings, useWitness } from "../../settings";
import {
  type AdmissionPreparation,
  getWitnessReviewSupport,
  prepareAdmissionSession,
  requestWitnessReview,
  type WitnessReview,
} from "../api/native-review-api";
import { waitingKeys } from "../api/query-keys";

export function useNativeReviewActions(
  entryId: string,
  hasCertificate: boolean,
  onReviewed: () => void,
  backend: string,
) {
  const core = useCoreStatus();
  const settings = useSettings();
  const witness = useWitness();
  const support = useQuery({
    queryKey: waitingKeys.witnessSupport(core.scope),
    queryFn: getWitnessReviewSupport,
    enabled: core.isSuccess && !hasCertificate,
  });
  const admission = useMutation<AdmissionPreparation, Error, boolean>({
    mutationFn: (confirmed) =>
      prepareAdmissionSession(entryId, backend.trim(), confirmed),
  });
  const review = useMutation<WitnessReview, Error, boolean>({
    mutationFn: (rawSessionConfirmed) =>
      requestWitnessReview(entryId, rawSessionConfirmed),
    onSuccess: (result) => {
      if (result.ready) onReviewed();
    },
  });
  const admissionRequired = settings.data?.admission_evidence_required === true;
  const canWitnessReview =
    witness.data?.state === "pinned" && support.data === true;
  return {
    admission,
    review,
    admissionRequired,
    canWitnessReview,
  };
}
