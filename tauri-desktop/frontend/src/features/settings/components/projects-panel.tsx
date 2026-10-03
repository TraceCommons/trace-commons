import { ProjectModeField } from "./project-mode-field";
import { ProjectAutomaticDisclosure } from "./project-automatic-disclosure";
import type { Project, ProjectMode } from "../api/projects-api";
import { TertiaryLink } from "@/design-system";

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
    <section className="tc-card">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            PROJECT POLICY
          </span>
          <h2>{allowAutoUpload ? "Projects" : "What to watch"}</h2>
        </div>
        <TertiaryLink
          type="button"
          onClick={() => void onRefresh()}
          disabled={state === "loading" || state === "busy"}
        >
          Refresh
        </TertiaryLink>
      </div>
      <p className="m-0 tc-caption tc-text-tertiary">
        Every project starts at ask-first. Ignore a project to leave it out
        entirely.
      </p>
      {error && (
        <p className="tc-alert">
          {error}
        </p>
      )}
      {state === "loading" && (
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          Reading projects…
        </p>
      )}
      {state === "error" && (
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          Project policy unavailable.
        </p>
      )}
      {state === "ready" && projects.length === 0 && (
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          No projects seen yet. Sessions appear here after discovery.
        </p>
      )}
      {state === "ready" && projects.length > 0 && (
        <div className="mt-3 grid gap-px">
          {projects.map((project) => (
            <div
              className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-[18px] border-b border-tc-hairline py-[15px]"
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
                {project.mode === "auto_upload" && (
                  <ProjectAutomaticDisclosure
                    projectId={project.project_id}
                    disclosure={project.automatic_disclosure}
                  />
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
