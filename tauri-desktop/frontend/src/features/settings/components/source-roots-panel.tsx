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
    <section className="rounded-2xl border border-border bg-card/80 p-[26px] block">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            {copy?.heading ?? "Source settings"}
          </span>
          <h2>Session folders</h2>
        </div>
        <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start bg-muted text-muted-foreground">
          Explicit
        </span>
      </div>
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        {copy?.explanation ?? "Loading source settings disclosure…"}
      </p>
      {!copy && (
        <p
          className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive"
          role="alert"
        >
          Source settings copy unavailable. Saving is disabled until it loads.
        </p>
      )}
      {error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {error}
        </p>
      )}
      <div className="mt-5 grid gap-px border-t border-border">
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
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
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
              className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
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
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
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
