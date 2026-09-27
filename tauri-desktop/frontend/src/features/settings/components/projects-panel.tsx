import { Button } from "@/components/ui/button";
import { ProjectModeField } from "./project-mode-field";
import type { Project, ProjectMode } from "../api/projects-api";

export function ProjectsPanel({
  projects,
  state,
  error,
  onRefresh,
  onSetMode,
  allowAutoUpload = true,
}: {
  projects: Project[];
  state: "loading" | "ready" | "error" | "busy";
  error: string | null;
  onRefresh: () => Promise<void>;
  onSetMode: (id: string, mode: ProjectMode) => Promise<unknown>;
  allowAutoUpload?: boolean;
}) {
  return (
    <section className="rounded-2xl border border-border bg-card/80 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            PROJECT POLICY
          </span>
          <h2>{allowAutoUpload ? "Projects" : "What to watch"}</h2>
        </div>
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={() => void onRefresh()}
          disabled={state === "loading" || state === "busy"}
        >
          Refresh
        </Button>
      </div>
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        Every project starts at ask-first. Ignore a project to leave it out
        entirely.
      </p>
      {error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {error}
        </p>
      )}
      {state === "loading" && (
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          Reading projects…
        </p>
      )}
      {state === "error" && (
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          Project policy unavailable.
        </p>
      )}
      {state === "ready" && projects.length === 0 && (
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          No projects seen yet. Sessions appear here after discovery.
        </p>
      )}
      {state === "ready" && projects.length > 0 && (
        <div className="mt-5 grid gap-px border-t border-border">
          {projects.map((project) => (
            <div
              className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-[18px] border-b border-border py-[15px]"
              key={project.project_id}
            >
              <div>
                <strong>
                  {project.is_unresolved_bucket
                    ? "Sessions with no project"
                    : project.project_label}
                </strong>
                {project.project_path && <span>{project.project_path}</span>}
                <small>
                  {project.pending_count === undefined
                    ? "No pending count"
                    : `${project.pending_count} pending`}
                  {project.contributable_count === undefined
                    ? ""
                    : ` · ${project.contributable_count} eligible`}
                </small>
                {project.is_unresolved_bucket && (
                  <small>
                    These sessions cannot be contributed automatically.
                  </small>
                )}
              </div>
              <ProjectModeField
                project={project}
                allowAutoUpload={allowAutoUpload}
                disabled={state !== "ready"}
                onSetMode={onSetMode}
              />
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
