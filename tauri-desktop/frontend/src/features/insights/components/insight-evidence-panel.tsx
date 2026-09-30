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
    <section className="mt-3 pt-3 tc-hairline-top">
      <span className="mb-1.5 block tc-eyebrow">
        OUTCOME EVIDENCE
      </span>
      <p className="m-0 tc-caption tc-text-tertiary">
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
        <div className="mt-3 flex flex-wrap gap-2">
          <Button
            className="tc-btn tc-btn--glass"
            type="button"
            onClick={() => void chooseRepository()}
            disabled={!saved || busy || pickerBusy}
          >
            {pickerBusy ? "Choosing…" : "Choose Git repository"}
          </Button>
          <Button
            className="tc-btn tc-btn--primary tc-btn--sm"
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
          <p className="tc-alert">
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
          className="tc-btn tc-btn--glass"
          type="button"
          onClick={() => reportInput.current?.click()}
          disabled={!saved || busy || pickerBusy}
        >
          Link test report
        </Button>
      </div>
      <div className="mt-3.5 grid gap-2.5">
        <span className="mb-1.5 block tc-eyebrow">
          SOURCE EVIDENCE
        </span>
        {insight.report.evidence.length === 0 ? (
          <p className="mt-3 mb-1 tc-body tc-text-tertiary">
            No source evidence retained.
          </p>
        ) : (
          insight.report.evidence.map((evidence) => (
            <div
              className="grid grid-cols-[150px_minmax(0,1fr)] gap-3 tc-hairline-top py-2 tc-caption tc-text-tertiary"
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
