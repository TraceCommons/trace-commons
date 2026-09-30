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
      className="mt-4 p-7 rounded-2xl border border-border bg-card/80"
      onSubmit={form.handleSubmit(onSave)}
    >
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            LOCAL DRAFT
          </span>
          <h2>Shape your profile</h2>
        </div>
        <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary">
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
            className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
            type="submit"
            disabled={
              actionState === "publishing" || actionState === "withdrawing"
            }
          >
            Save draft
          </Button>
          <Button
            className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
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
              className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary text-destructive"
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
