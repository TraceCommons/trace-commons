import { useEffect, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { Button } from "@/components/ui/button";
import { NativeSelect } from "@/components/ui/native-select";
import { ProjectAutoUploadDisclosure } from "../../../components/project-auto-upload-disclosure";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import type { Project, ProjectMode } from "../api/projects-api";
import { type ProjectModeFormValues, projectModeFormSchema } from "../forms";
import { useController, useForm } from "react-hook-form";

const labels: Record<ProjectMode, string> = {
  notify_only: "Ask me first",
  auto_upload: "Contribute automatically",
  ignore: "Never offer this one",
};

type Confirmation = "auto_upload" | "ignore" | null;

export function ProjectModeField({
  project,
  allowAutoUpload,
  disabled,
  onSetMode,
}: {
  project: Project;
  allowAutoUpload: boolean;
  disabled: boolean;
  onSetMode: (id: string, mode: ProjectMode) => Promise<unknown>;
}) {
  const visibleMode =
    allowAutoUpload || project.mode !== "auto_upload"
      ? project.mode
      : "notify_only";
  const [confirmation, setConfirmation] = useState<Confirmation>(null);
  const form = useForm<ProjectModeFormValues>({
    resolver: zodResolver(projectModeFormSchema),
    defaultValues: { mode: visibleMode },
  });
  const mode = useController({ control: form.control, name: "mode" });
  useEffect(() => {
    if (!form.formState.isDirty) form.reset({ mode: visibleMode });
  }, [form, visibleMode]);

  const save = async (next: ProjectMode) => {
    try {
      await onSetMode(project.project_id, next);
      form.reset({ mode: next });
    } catch {
      form.reset({ mode: visibleMode });
    }
  };
  const cancel = () => {
    setConfirmation(null);
    form.reset({ mode: visibleMode });
  };
  const confirm = (next: ProjectMode) => {
    setConfirmation(null);
    void save(next);
  };
  const change = (next: ProjectMode) => {
    mode.field.onChange(next);
    if (next === "auto_upload" && allowAutoUpload) {
      setConfirmation("auto_upload");
    } else if (next === "ignore") {
      setConfirmation("ignore");
    } else {
      void save(next);
    }
  };

  return (
    <>
      <NativeSelect
        ref={mode.field.ref}
        name={mode.field.name}
        value={mode.field.value}
        onBlur={mode.field.onBlur}
        onChange={(event) => change(event.target.value as ProjectMode)}
        disabled={disabled || confirmation !== null}
      >
        <option value="notify_only">{labels.notify_only}</option>
        {allowAutoUpload && !project.is_unresolved_bucket && (
          <option value="auto_upload">{labels.auto_upload}</option>
        )}
        <option value="ignore">{labels.ignore}</option>
      </NativeSelect>
      <ResponsiveOverlay
        open={confirmation === "auto_upload"}
        onOpenChange={(open) => {
          if (!open) cancel();
        }}
        title={`Enable automatic contribution for ${project.project_label}?`}
        description="Confirm project-wide automatic contribution."
        footer={
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" onClick={cancel} disabled={disabled}>
              Keep asking first
            </Button>
            <Button
              type="button"
              onClick={() => confirm("auto_upload")}
              disabled={disabled}
            >
              Enable for this project
            </Button>
          </div>
        }
      >
        <ProjectAutoUploadDisclosure />
      </ResponsiveOverlay>
      <ResponsiveOverlay
        open={confirmation === "ignore"}
        onOpenChange={(open) => {
          if (!open) cancel();
        }}
        title={`Ignore ${project.project_label}?`}
        description="Review the sessions this setting will remove from the queue."
        footer={
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" onClick={cancel} disabled={disabled}>
              Keep project
            </Button>
            <Button
              type="button"
              variant="destructive"
              onClick={() => confirm("ignore")}
              disabled={disabled}
            >
              Ignore project
            </Button>
          </div>
        }
      >
        <p className="text-[12px] leading-[1.55] text-muted-foreground">
          {project.pending_count === undefined
            ? "Pending sessions"
            : `${project.pending_count} pending session${project.pending_count === 1 ? "" : "s"}`}{" "}
          will leave the review queue. Future sessions from this project will
          not be offered. Session files stay on this device. You can switch this
          project back to Ask me first in Project settings.
        </p>
      </ResponsiveOverlay>
    </>
  );
}
