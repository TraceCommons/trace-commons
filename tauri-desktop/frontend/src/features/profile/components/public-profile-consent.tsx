import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect } from "react";
import { useForm } from "react-hook-form";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import { FormFieldError } from "../../../components/form-field-error";
import { type ProfileConsentValues, profileConsentSchema } from "../forms";
import { ButtonPrimary, Checkbox, GlassButton } from "@/design-system";

type PublicProfileConsentProps = {
  open: boolean;
  handle: string;
  bio: string;
  busy: boolean;
  error: string | null;
  onConfirm: () => void;
  onCancel: () => void;
};

export function PublicProfileConsent({
  open,
  handle,
  bio,
  busy,
  error,
  onConfirm,
  onCancel,
}: PublicProfileConsentProps) {
  const form = useForm<ProfileConsentValues>({
    resolver: zodResolver(profileConsentSchema),
    defaultValues: { acknowledged: false },
  });
  const acknowledged = form.watch("acknowledged");
  const acknowledgedError = form.formState.errors.acknowledged?.message;

  useEffect(() => {
    if (!open) form.reset({ acknowledged: false });
  }, [form, open]);

  return (
    <ResponsiveOverlay
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) onCancel();
      }}
      title="Put your handle on the public roster?"
      description="Public attribution changes identity metadata only. It grants no use of trace data."
      footer={
        <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
          <GlassButton type="button" onClick={onCancel} disabled={busy}>
            Not now
          </GlassButton>
          <ButtonPrimary size="sm"
            type="submit"
            form="public-profile-consent-form"
            disabled={busy || !handle.trim()}
          >
            {busy ? "Going public…" : "Go public"}
          </ButtonPrimary>
        </div>
      }
    >
      <form
        id="public-profile-consent-form"
        className="grid gap-4 pb-2"
        onSubmit={form.handleSubmit(() => onConfirm())}
      >
        <div className="tc-card tc-card--quiet grid gap-4 sm:grid-cols-2">
          <section className="grid gap-2">
            <h3>What gets published</h3>
            <p className="text-sm text-tc-secondary">
              Your handle, aggregate counts, public date, and bio if provided.
            </p>
            <code className="tc-card tc-card--quiet text-xs">
              {handle.trim() || "handle not set"}
              {bio.trim() ? ` · ${bio.trim().length} bio chars` : ""}
            </code>
          </section>
          <section className="grid gap-2">
            <h3>What never does</h3>
            <p className="text-sm text-tc-secondary">
              Traces, trace contents, per-trace data, or anything about traces
              you did not send.
            </p>
          </section>
        </div>
        <label
          htmlFor="profile-consent-acknowledged"
          className="tc-card tc-card--quiet flex items-start gap-3 text-sm"
        >
          <Checkbox
            id="profile-consent-acknowledged"
            checked={acknowledged}
            disabled={busy}
            aria-invalid={Boolean(acknowledgedError)}
            aria-describedby={
              acknowledgedError ? "profile-consent-error" : undefined
            }
            onChange={(checked) =>
              form.setValue("acknowledged", checked === true, {
                shouldDirty: true,
                shouldValidate: true,
              })
            }
          />
          <span>
            I understand my handle and aggregate counts become public. Leaving
            the roster removes me from future snapshots.
          </span>
        </label>
        <FormFieldError id="profile-consent-error" message={acknowledgedError} />
        {error && (
          <p className="tc-alert m-0" role="alert">
            {error}
          </p>
        )}
        <p className="m-0 text-xs text-tc-secondary">
          Nothing is pre-checked. Go public stays off until acknowledgement is
          enabled.
        </p>
      </form>
    </ResponsiveOverlay>
  );
}
