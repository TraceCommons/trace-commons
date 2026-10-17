import { useState } from "react";
import { useNativeReviewActions } from "../hooks/use-native-review-actions";
import { AdmissionPreparationOverlay } from "./admission-preparation-overlay";
import { WitnessReviewOverlay } from "./witness-review-overlay";
import { GlassButton } from "@/design-system";

export function NativeReviewActions({
  entryId,
  hasCertificate,
  onReviewed,
}: {
  entryId: string;
  hasCertificate: boolean;
  onReviewed: () => void;
}) {
  const [admissionOpen, setAdmissionOpen] = useState(false);
  const [witnessOpen, setWitnessOpen] = useState(false);
  const [backend, setBackend] = useState("");
  const actions = useNativeReviewActions(
    entryId,
    hasCertificate,
    onReviewed,
    backend,
  );
  if (
    hasCertificate ||
    (!actions.admissionRequired && !actions.canWitnessReview)
  ) {
    return null;
  }
  return (
    <div className="mt-4 grid gap-2 border-t border-tc-hairline pt-4">
      <span className="font-mono text-[10px] font-extrabold tracking-[.16em] text-tc-accent">
        NATIVE REVIEW
      </span>
      <p className="m-0 tc-caption tc-text-tertiary">
        Optional daemon-backed checks stay local until you explicitly confirm.
      </p>
      <div className="flex flex-wrap gap-2.5">
        {actions.admissionRequired && (
          <GlassButton
            type="button"
            onClick={() => {
              actions.admission.reset();
              setAdmissionOpen(true);
            }}
          >
            Prepare admission
          </GlassButton>
        )}
        {actions.canWitnessReview && (
          <GlassButton
            type="button"
            onClick={() => {
              actions.review.reset();
              setWitnessOpen(true);
            }}
          >
            Request witness review
          </GlassButton>
        )}
      </div>
      <AdmissionPreparationOverlay
        open={admissionOpen}
        onOpenChange={setAdmissionOpen}
        backend={backend}
        onBackendChange={setBackend}
        mutation={actions.admission}
      />
      <WitnessReviewOverlay
        open={witnessOpen}
        onOpenChange={setWitnessOpen}
        mutation={actions.review}
      />
    </div>
  );
}
