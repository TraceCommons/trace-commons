import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Button } from "@/components/ui/button";
import { useFormContext } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import type { ProfileFormValues } from "../forms";

export function ProfileEditor({
  saved,
  published,
  actionState,
  actionError,
  actionNotice,
  onSave,
  onPublish,
  onWithdraw,
}: {
  saved: boolean;
  published: boolean;
  actionState: "idle" | "publishing" | "withdrawing" | "error";
  actionError: string | null;
  actionNotice: string | null;
  onSave: (values: ProfileFormValues) => void;
  onPublish: () => void;
  onWithdraw: () => void;
}) {
  const form = useFormContext<ProfileFormValues>();
  const errors = form.formState.errors;
  return (
    <form
      className="tc-card mt-4"
      onSubmit={form.handleSubmit(onSave)}
    >
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            LOCAL DRAFT
          </span>
          <h2>Shape your profile</h2>
        </div>
        <span className="tc-card whitespace-nowrap font-mono text-[10px] font-extrabold tracking-[.08em] text-primary">
          {published ? "Published" : saved ? "Saved in memory" : "Not published"}
        </span>
      </div>
      <div className="mt-6 grid grid-cols-2 gap-4">
        <label>
          Handle
          <Input
            {...form.register("handle")}
            placeholder="your-handle"
            maxLength={32}
            aria-invalid={Boolean(errors.handle)}
            aria-describedby={
              errors.handle ? "profile-handle-error" : undefined
            }
          />
          <FormFieldError
            id="profile-handle-error"
            message={errors.handle?.message}
          />
        </label>
        <label>
          Bio
          <Textarea
            {...form.register("bio")}
            placeholder="A short context for your contribution"
            rows={4}
            aria-invalid={Boolean(errors.bio)}
            aria-describedby={errors.bio ? "profile-bio-error" : undefined}
          />
          <FormFieldError
            id="profile-bio-error"
            message={errors.bio?.message}
          />
        </label>
      </div>
      <div className="flex items-center justify-between gap-[18px] pt-[17px]">
        <div aria-live="polite" className="grid gap-1">
          {actionError ? (
            <p role="alert" className="text-destructive">
              {actionError}
            </p>
          ) : actionNotice ? (
            <p role="status">{actionNotice}</p>
          ) : (
            <p>
              Publishing sends handle and bio to community roster. It does not
              publish session content.
            </p>
          )}
        </div>
        <div className="flex flex-wrap justify-end gap-[9px]">
          <Button
            className="tc-btn tc-btn--glass"
            type="submit"
            disabled={
              actionState === "publishing" || actionState === "withdrawing"
            }
          >
            Save draft
          </Button>
          <Button
            className="tc-btn tc-btn--primary tc-btn--sm"
            type="button"
            onClick={() => void form.handleSubmit(onPublish)()}
            disabled={
              actionState === "publishing" || actionState === "withdrawing"
            }
          >
            {actionState === "publishing"
              ? "Publishing…"
              : published
                ? "Update profile"
                : "Go public"}
          </Button>
          {published && (
            <Button
              className="tc-link text-destructive"
              type="button"
              onClick={onWithdraw}
              disabled={actionState !== "idle"}
            >
              {actionState === "withdrawing" ? "Withdrawing…" : "Withdraw"}
            </Button>
          )}
        </div>
      </div>
    </form>
  );
}
