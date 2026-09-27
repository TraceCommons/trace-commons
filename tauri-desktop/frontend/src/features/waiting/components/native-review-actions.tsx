import { useState } from "react";
import { Button } from "@/components/ui/button";
import { useNativeReviewActions } from "../hooks/use-native-review-actions";
import { AdmissionPreparationOverlay } from "./admission-preparation-overlay";
import { WitnessReviewOverlay } from "./witness-review-overlay";

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
    <div className="mt-4 grid gap-2 border-t border-border pt-4">
      <span className="font-mono text-[10px] font-extrabold tracking-[.16em] text-primary">
        NATIVE REVIEW
      </span>
      <p className="m-0 text-[11px] leading-[1.5] text-muted-foreground">
        Optional daemon-backed checks stay local until you explicitly confirm.
      </p>
      <div className="flex flex-wrap gap-2.5">
        {actions.admissionRequired && (
          <Button
            type="button"
            variant="outline"
            onClick={() => {
              actions.admission.reset();
              setAdmissionOpen(true);
            }}
          >
            Prepare admission
          </Button>
        )}
        {actions.canWitnessReview && (
          <Button
            type="button"
            variant="outline"
            onClick={() => {
              actions.review.reset();
              setWitnessOpen(true);
            }}
          >
            Request witness review
          </Button>
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
