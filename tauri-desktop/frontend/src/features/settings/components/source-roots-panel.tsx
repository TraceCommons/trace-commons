import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { Button } from "@/components/ui/button";
import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect, useState } from "react";
import { useForm } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import type { ContributorDisclosureCopy } from "../../../lib/tauri/contributor-copy-api";
import { useDirectoryPicker } from "../../../lib/tauri/use-platform-actions";
import type { SourceMode, SourceName } from "../api/source-roots-api";
import { type SourceRootFormValues, sourceRootFormSchema } from "../forms";

type SourceRootsPanelProps = {
  snapshot: Record<string, unknown>;
  busy: boolean;
  error: string | null;
  onSave: (
    source: SourceName,
    mode: SourceMode,
    path: string,
  ) => Promise<unknown>;
};
const sources: Array<{ name: SourceName; label: string }> = [
  { name: "claude", label: "Claude Code" },
  { name: "codex", label: "Codex" },
  { name: "gemini", label: "Gemini CLI" },
  { name: "cline", label: "Cline" },
  { name: "opencode", label: "OpenCode" },
];

export function SourceRootsPanel({
  snapshot,
  busy,
  error,
  onSave,
}: SourceRootsPanelProps) {
  const disclosure = useContributorDisclosureCopy();
  const copy = disclosure.data?.source_settings;
  return (
    <section className="tc-card block">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            {copy?.heading ?? "Source settings"}
          </span>
          <h2>Session folders</h2>
        </div>
        <span className="tc-chip self-start">
          Explicit
        </span>
      </div>
      <p className="m-0 tc-caption tc-text-tertiary">
        {copy?.explanation ?? "Loading source settings disclosure…"}
      </p>
      {!copy && (
        <p
          className="tc-alert"
          role="alert"
        >
          Source settings copy unavailable. Saving is disabled until it loads.
        </p>
      )}
      {error && (
        <p className="tc-alert">
          {error}
        </p>
      )}
      <div className="mt-3 grid gap-px">
        {sources.map((source) => (
          <SourceRootRow
            key={source.name}
            source={source.name}
            label={source.label}
            snapshot={snapshot}
            busy={busy || !copy}
            copy={copy}
            tool={Object.values(copy?.tools ?? {}).find(
              (tool) => tool.key === source.name,
            )}
            statusLine={
              disclosure.data?.source_check_lines[source.name]?.[
                snapshot[`${source.name}_source_mode`] === "watch"
                  ? "watch"
                  : snapshot[`${source.name}_source_mode`] === "off"
                    ? "off"
                    : "unset"
              ] ?? copy?.unavailable ?? ""
            }
            onSave={onSave}
          />
        ))}
      </div>
      <p className="m-0 tc-caption tc-text-tertiary">
        Daemon does not return saved paths. Re-enter path when replacing a
        watched root. Rust validates selected directories before saving.
      </p>
    </section>
  );
}

function SourceRootRow({
  source,
  label,
  snapshot,
  busy,
  copy,
  tool,
  statusLine,
  onSave,
}: {
  source: SourceName;
  label: string;
  snapshot: Record<string, unknown>;
  busy: boolean;
  copy: ContributorDisclosureCopy["source_settings"] | undefined;
  tool:
    | ContributorDisclosureCopy["source_settings"]["tools"][string]
    | undefined;
  statusLine: string;
  onSave: (
    source: SourceName,
    mode: SourceMode,
    path: string,
  ) => Promise<unknown>;
}) {
  const authoritativeMode = mode(snapshot[`${source}_source_mode`]);
  const form = useForm<SourceRootFormValues>({
    resolver: zodResolver(sourceRootFormSchema),
    defaultValues: { mode: authoritativeMode, path: "" },
    mode: "onChange",
  });
  const picker = useDirectoryPicker();
  const [choosing, setChoosing] = useState(false);
  useEffect(() => {
    if (!form.formState.isDirty)
      form.reset({ mode: authoritativeMode, path: form.getValues("path") });
  }, [authoritativeMode, form]);
  const modeValue = form.watch("mode");
  const pathError = form.formState.errors.path?.message;
  const pickerError = picker.isError
    ? "Folder picker unavailable or cancelled."
    : null;
  const chooseRoot = async () => {
    setChoosing(true);
    try {
      const path = await picker.pick("source_root");
      form.setValue("path", path, { shouldDirty: true, shouldValidate: true });
    } catch {
      form.resetField("path", {
        defaultValue: form.getValues("path"),
        keepDirty: true,
      });
    } finally {
      setChoosing(false);
    }
  };
  const save = async (values: SourceRootFormValues) => {
    try {
      await onSave(source, values.mode, values.path);
      form.reset(values);
    } catch {
      form.reset({ mode: authoritativeMode, path: values.path });
    }
  };
  return (
    <form
      className="grid grid-cols-[170px_minmax(0,1fr)] gap-[18px] border-b border-border py-[15px]"
      onSubmit={form.handleSubmit(save)}
    >
      <div>
        <strong>{label}</strong>
        <span>{statusLine || copy?.unavailable}</span>
      </div>
      <div className="flex flex-wrap items-start gap-2">
        <NativeSelect
          {...form.register("mode")}
          disabled={busy}
          aria-label={`${label} source mode`}
          aria-invalid={Boolean(form.formState.errors.mode)}
          aria-describedby={
            form.formState.errors.mode ? `${source}-mode-error` : undefined
          }
        >
          <option value="watch">{copy?.watch_candidate ?? "Watch"}</option>
          <option value="off">
            {tool?.decline ?? copy?.no_candidate ?? "Off"}
          </option>
        </NativeSelect>
        {modeValue === "watch" && (
          <>
            <label>
              <span className="sr-only">{label} sessions folder</span>
              <Input
                {...form.register("path")}
                placeholder={
                  copy?.selected_folder ?? "/absolute/path/to/sessions"
                }
                disabled={busy || choosing}
                aria-invalid={Boolean(pathError)}
                aria-describedby={
                  pathError ? `${source}-path-error` : undefined
                }
              />
              <FormFieldError id={`${source}-path-error`} message={pathError} />
            </label>
            <Button
              className="tc-btn tc-btn--glass"
              type="button"
              onClick={() => void chooseRoot()}
              disabled={busy || choosing}
            >
              {choosing
                ? "Choosing…"
                : (tool?.choose_folder ?? copy?.choose_folder ?? "Choose folder")}
            </Button>
          </>
        )}
        <Button
          className="tc-btn tc-btn--glass"
          type="submit"
          disabled={busy || !form.formState.isValid}
        >
          Save
        </Button>
        <FormFieldError
          id={`${source}-mode-error`}
          message={form.formState.errors.mode?.message}
        />
        {pickerError && (
          <p className="m-0 text-[11px] text-destructive">{pickerError}</p>
        )}
      </div>
    </form>
  );
}

function mode(value: unknown): SourceMode {
  return value === "watch" ? "watch" : "off";
}
