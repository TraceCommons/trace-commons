import type { UseMutationResult } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import type { AdmissionPreparation } from "../api/native-review-api";

type AdmissionMutation = UseMutationResult<AdmissionPreparation, Error, boolean, unknown>;

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
  const copy = useContributorDisclosureCopy();
  const [confirmed, setConfirmed] = useState(false);
  useEffect(() => {
    if (!open) setConfirmed(false);
  }, [open]);
  const disclosure = copy.data?.admission;
  const error = mutation.isError
    ? (disclosure?.failed ??
      "Admission preparation could not be completed.")
    : mutation.isSuccess && !mutation.data.ready
      ? mutation.data.message
      : null;
  return (
    <ResponsiveOverlay
      open={open}
      onOpenChange={onOpenChange}
      title={disclosure?.heading ?? "Prepare admission"}
      description={disclosure?.disclosure}
      footer={
        <div className="flex justify-end gap-2">
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            {disclosure?.cancel ?? "Cancel"}
          </Button>
          <Button
            type="button"
            onClick={() => mutation.mutate(true)}
            disabled={
              mutation.isPending ||
              mutation.isSuccess ||
              !backend.trim() ||
              !disclosure ||
              !confirmed
            }
          >
            {mutation.isPending
              ? "Preparing…"
              : (disclosure?.confirm ?? "Confirm preparation")}
          </Button>
        </div>
      }
    >
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="admission-backend">
            {disclosure?.backend ?? "Inference backend"}
          </FieldLabel>
          <Input
            id="admission-backend"
            value={backend}
            onChange={(event) => onBackendChange(event.target.value)}
            disabled={mutation.isPending}
          />
          <FieldDescription>
            {disclosure?.prerequisite ??
              "Backend label is bound into admission evidence."}
          </FieldDescription>
        </Field>
      </FieldGroup>
      {!disclosure && (
        <p className="mt-4 text-[12px] text-muted-foreground">
          {copy.isError
            ? "Admission disclosure unavailable. Preparation is disabled."
            : "Loading admission disclosure…"}
        </p>
      )}
      {disclosure && (
        <label className="mt-4 flex items-start gap-2.5 rounded-[9px] border border-border p-3.5 text-[12px] text-foreground">
          <Checkbox
            checked={confirmed}
            onCheckedChange={(value) => setConfirmed(value === true)}
            disabled={mutation.isPending || mutation.isSuccess}
            aria-label="Confirm admission preparation"
          />
          <span>I understand and want to prepare this session.</span>
        </label>
      )}
      {disclosure && (
        <p className="mt-3 text-[11px] text-muted-foreground">
          {disclosure.permission}
        </p>
      )}
      {error && (
        <p className="mt-4 rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {error}
        </p>
      )}
      {mutation.isPending && disclosure && (
        <p className="mt-3 text-[12px] text-muted-foreground">
          {disclosure.working}
        </p>
      )}
      {mutation.isSuccess && mutation.data.ready && disclosure && (
        <p className="mt-3 text-[12px] text-muted-foreground">
          {disclosure.ready}
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
