import type { UseMutationResult } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import type { AdmissionPreparation } from "../api/native-review-api";

type AdmissionMutation = UseMutationResult<
  AdmissionPreparation,
  Error,
  void,
  unknown
>;

export function AdmissionPreparationOverlay({
  open,
  onOpenChange,
  backend,
  onBackendChange,
  mutation,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  backend: string;
  onBackendChange: (backend: string) => void;
  mutation: AdmissionMutation;
}) {
  const error = mutation.isError
    ? "Admission preparation could not be completed. Nothing was sent."
    : mutation.isSuccess && !mutation.data.ready
      ? mutation.data.message
      : null;
  return (
    <ResponsiveOverlay
      open={open}
      onOpenChange={onOpenChange}
      title="Prepare admission"
      description="This asks the enrolled commons to prepare evidence for this reviewed entry. It does not send the session."
      footer={
        <div className="flex justify-end gap-2">
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            type="button"
            onClick={() => mutation.mutate()}
            disabled={mutation.isPending || !backend.trim()}
          >
            {mutation.isPending ? "Preparing…" : "Confirm preparation"}
          </Button>
        </div>
      }
    >
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="admission-backend">Inference backend</FieldLabel>
          <Input
            id="admission-backend"
            value={backend}
            onChange={(event) => onBackendChange(event.target.value)}
            disabled={mutation.isPending}
          />
          <FieldDescription>
            Backend label is bound into admission evidence.
          </FieldDescription>
        </Field>
      </FieldGroup>
      {error && (
        <p className="mt-4 rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {error}
        </p>
      )}
      {mutation.isSuccess && mutation.data.ready && (
        <p className="mt-4 text-[12px] text-muted-foreground">
          {mutation.data.message}
        </p>
      )}
    </ResponsiveOverlay>
  );
}
