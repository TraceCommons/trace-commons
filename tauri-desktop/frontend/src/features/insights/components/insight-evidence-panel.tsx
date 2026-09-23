import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { zodResolver } from "@hookform/resolvers/zod";
import { useRef } from "react";
import { useForm } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import { useDirectoryPicker } from "../../../lib/tauri/use-platform-actions";
import { type GitEvidenceFormValues, gitEvidenceFormSchema } from "../forms";
import type { Insight } from "../types";
import { OutcomeLinkList } from "./outcome-link-list";

type InsightEvidencePanelProps = {
  insight: Insight;
  saved: boolean;
  busy: boolean;
  onLinkTestReport: (file: File) => void;
  onLinkGit: (repository: string, commit: string) => Promise<boolean>;
  onUnlink: (id: string) => void;
};

export function InsightEvidencePanel({
  insight,
  saved,
  busy,
  onLinkTestReport,
  onLinkGit,
  onUnlink,
}: InsightEvidencePanelProps) {
  const reportInput = useRef<HTMLInputElement>(null);
  const picker = useDirectoryPicker();
  const form = useForm<GitEvidenceFormValues>({
    resolver: zodResolver(gitEvidenceFormSchema),
    defaultValues: { commit: "", repository: "", reportFile: undefined },
    mode: "onChange",
  });
  const reportField = form.register("reportFile");
  const errors = form.formState.errors;
  const chooseRepository = async () => {
    try {
      form.setValue("repository", await picker.pick("repository"), {
        shouldDirty: true,
        shouldValidate: true,
      });
    } catch {
      form.resetField("repository", {
        defaultValue: form.getValues("repository"),
        keepDirty: true,
      });
    }
  };
  const pickerBusy = picker.isPending;
  const pickerError = picker.isError
    ? "Repository picker unavailable or cancelled."
    : null;
  return (
    <section className="mt-6 border-t border-border pt-5">
      <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
        OUTCOME EVIDENCE
      </span>
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        Git inspection is local and read-only. Imported reports are producer
        assertions. Neither link verifies task success.
      </p>
      <form
        className="mt-3.5 grid gap-2.5"
        onSubmit={form.handleSubmit(async (values) => {
          if (await onLinkGit(values.repository.trim(), values.commit.trim()))
            form.reset(values);
        })}
      >
        <label>
          Commit
          <Input
            {...form.register("commit")}
            placeholder="40 or 64 lowercase hex characters"
            disabled={!saved || busy || pickerBusy}
            aria-invalid={Boolean(errors.commit)}
            aria-describedby={errors.commit ? "git-commit-error" : undefined}
          />
          <FormFieldError
            id="git-commit-error"
            message={errors.commit?.message}
          />
        </label>
        <div className="mt-6 flex gap-2.5">
          <Button
            className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
            type="button"
            onClick={() => void chooseRepository()}
            disabled={!saved || busy || pickerBusy}
          >
            {pickerBusy ? "Choosing…" : "Choose Git repository"}
          </Button>
          <Button
            className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
            type="submit"
            disabled={!saved || busy || pickerBusy || !form.formState.isValid}
          >
            Link commit
          </Button>
        </div>
        {errors.repository && (
          <FormFieldError
            id="git-repository-error"
            message={errors.repository.message}
          />
        )}
        {form.watch("repository") && (
          <code className="overflow-hidden text-[10px] text-muted-foreground text-ellipsis whitespace-nowrap">
            {form.watch("repository")}
          </code>
        )}
        {pickerError && (
          <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
            {pickerError}
          </p>
        )}
      </form>
      <div className="flex flex-wrap justify-end gap-[9px] mt-3">
        <Input
          {...reportField}
          ref={(element) => {
            reportField.ref(element);
            reportInput.current = element;
          }}
          className="absolute h-px w-px overflow-hidden whitespace-nowrap [clip:rect(0_0_0_0)] [clip-path:inset(50%)]"
          type="file"
          accept="application/json,.json"
          onChange={(event) => {
            const files = event.currentTarget.files;
            form.setValue("reportFile", files, { shouldDirty: true });
            const file = files?.[0];
            form.resetField("reportFile");
            event.currentTarget.value = "";
            if (file) onLinkTestReport(file);
          }}
        />
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={() => reportInput.current?.click()}
          disabled={!saved || busy || pickerBusy}
        >
          Link test report
        </Button>
      </div>
      <div className="mt-3.5 grid gap-2.5">
        <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
          SOURCE EVIDENCE
        </span>
        {insight.report.evidence.length === 0 ? (
          <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
            No source evidence retained.
          </p>
        ) : (
          insight.report.evidence.map((evidence) => (
            <div
              className="grid grid-cols-[150px_minmax(0,1fr)] gap-3 border-t border-border py-[9px] text-[11px] text-muted-foreground"
              key={evidence.id}
            >
              <span>{evidence.id}</span>
              <code>{evidence.source_digest}</code>
            </div>
          ))
        )}
      </div>
      <OutcomeLinkList
        links={insight.outcome_links ?? []}
        busy={busy}
        onUnlink={onUnlink}
      />
    </section>
  );
}
