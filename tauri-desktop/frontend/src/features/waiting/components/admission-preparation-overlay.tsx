import type { UseMutationResult } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import type { AdmissionPreparation } from "../api/native-review-api";
import { ButtonPrimary, Checkbox, GlassButton, Input } from "@/design-system";

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
          <GlassButton
            type="button"
            onClick={() => onOpenChange(false)}
          >
            {disclosure?.cancel ?? "Cancel"}
          </GlassButton>
          <ButtonPrimary size="sm"
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
          </ButtonPrimary>
        </div>
      }
    >
      <div className="flex flex-col gap-4">
        <div className="tc-field">
          <label className="tc-label" htmlFor="admission-backend">
            {disclosure?.backend ?? "Inference backend"}
          </label>
          <Input
            id="admission-backend"
            value={backend}
            onChange={(event) => onBackendChange(event.target.value)}
            disabled={mutation.isPending}
          />
          <p className="m-0 tc-caption tc-text-tertiary">
            {disclosure?.prerequisite ??
              "Backend label is bound into admission evidence."}
          </p>
        </div>
      </div>
      {!disclosure && (
        <p className="mt-4 text-[12px] text-tc-secondary">
          {copy.isError
            ? "Admission disclosure unavailable. Preparation is disabled."
            : "Loading admission disclosure…"}
        </p>
      )}
      {disclosure && (
        <label className="tc-card tc-card--quiet mt-4 flex items-start gap-2.5 text-[12px] text-tc-primary">
          <Checkbox
            checked={confirmed}
            onChange={(value) => setConfirmed(value === true)}
            disabled={mutation.isPending || mutation.isSuccess}
            aria-label="Confirm admission preparation"
          />
          <span>I understand and want to prepare this trace.</span>
        </label>
      )}
      {disclosure && (
        <p className="mt-3 text-[11px] text-tc-secondary">
          {disclosure.permission}
        </p>
      )}
      {error && (
        <p className="tc-card tc-card--quiet mt-4 border-tc-outside/30 text-[12px] text-tc-outside">
          {error}
        </p>
      )}
      {mutation.isPending && disclosure && (
        <p className="mt-3 text-[12px] text-tc-secondary">
          {disclosure.working}
        </p>
      )}
      {mutation.isSuccess && mutation.data.ready && disclosure && (
        <p className="mt-3 text-[12px] text-tc-secondary">
          {disclosure.ready}
        </p>
      )}
      {mutation.isSuccess && mutation.data.ready && (
        <p className="mt-4 text-[12px] text-tc-secondary">
          {mutation.data.message}
        </p>
      )}
    </ResponsiveOverlay>
  );
}
