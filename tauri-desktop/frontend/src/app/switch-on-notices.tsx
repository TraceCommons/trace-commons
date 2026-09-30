import { useMutation, useQueryClient } from "@tanstack/react-query";
import { changeProjectMode, settingsKeys } from "../features/settings/public";
import { acknowledgeArmingRewordings } from "../lib/tauri/core-api";
import { coreKeys } from "../lib/tauri/query-keys";
import {
  type ArmingRewording,
  askFirstTarget,
  type GateHeld,
  type GateHeldProject,
  parseArmingRewordings,
  parseGateHeld,
} from "../lib/tauri/switch-on-notices";
import { useCoreStatus } from "../lib/tauri/use-core-status";
import {
  useArmingRewordedNotice,
  useGateHeldNotice,
} from "../lib/tauri/use-contributor-copy";
import { GlassButton, Notice } from "@/design-system";

/**
 * "Ask me first" is the Settings call, unchanged: `set_project_mode` with
 * the project's id and `notify_only`. The daemon answers the rewording
 * notice itself, and a refusal changes nothing.
 */
function useAskFirst() {
  const queryClient = useQueryClient();
  const core = useCoreStatus();
  return useMutation({
    mutationFn: (id: string) => changeProjectMode(id, "notify_only"),
    onSettled: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.projects(core.scope),
        }),
      ]),
  });
}

/**
 * Every armed folder whose arming wording no longer claims a model scrubs
 * its sessions, not yet shown by any shell (K5). Rendered above every page,
 * like a void: what the contributor agreed to has been reworded, and they
 * are told wherever they are. The words come from the contributor core.
 */
export function ArmingRewordingNotices({
  rewordings,
}: {
  rewordings: unknown;
}) {
  let list: ArmingRewording[];
  try {
    list = parseArmingRewordings(rewordings);
  } catch {
    return (
      <p className="tc-alert" role="alert">
        What automatic contributing means for some of your projects may have
        changed, but the notice that says which could not be read. Check your
        projects' settings.
      </p>
    );
  }
  if (list.length === 0) return null;
  return (
    <>
      {list.map((rewording) => (
        <ArmingRewordingCard key={rewording.id} rewording={rewording} />
      ))}
    </>
  );
}

function ArmingRewordingCard({ rewording }: { rewording: ArmingRewording }) {
  const queryClient = useQueryClient();
  const copy = useArmingRewordedNotice(rewording.id, rewording.wire);
  const acknowledge = useMutation({
    mutationFn: () => acknowledgeArmingRewordings([rewording.id]),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: coreKeys.status }),
  });
  const askFirst = useAskFirst();
  const projectId = copy.data
    ? askFirstTarget(rewording.wire, copy.data)
    : null;
  const busy = acknowledge.isPending || askFirst.isPending;

  return (
    <Notice tone="ask" title={copy.data ? copy.data.title : undefined}>
      {copy.data ? (
        <div className="grid gap-2">
          <span>{copy.data.body}</span>
          <span className="font-semibold">{copy.data.now_heading}</span>
          <span>{copy.data.scope}</span>
          <span>{copy.data.limit}</span>
          <span>{copy.data.no_review}</span>
          <div className="flex flex-wrap gap-2">
            {/* Offered only once the notice is on screen. */}
            {projectId !== null && copy.data.ask_first_action && (
              <GlassButton
                type="button"
                disabled={busy || !copy.data}
                onClick={() => askFirst.mutate(projectId)}
              >
                {copy.data.ask_first_action}
              </GlassButton>
            )}
            <GlassButton
              type="button"
              disabled={busy || !copy.data}
              onClick={() => acknowledge.mutate()}
            >
              {copy.data.acknowledge}
            </GlassButton>
          </div>
          {askFirst.isError && copy.data.ask_first_failed && (
            <span className="text-tc-outside" role="alert">
              {copy.data.ask_first_failed}
            </span>
          )}
          {acknowledge.isError && (
            <span className="text-tc-outside" role="alert">
              This notice could not be dismissed. It will show again.
            </span>
          )}
        </div>
      ) : (
        <div>
          {copy.isError
            ? "What automatic contributing means for one of your projects has changed, but this build could not read the notice that says how. Check your projects' settings."
            : "Loading…"}
        </div>
      )}
    </Notice>
  );
}

/**
 * Armed folders the automatic-contribution gate is holding, from
 * `status.automatic_contribution_held`. Beside the health slot rather than
 * in it, because a higher label can mask `automatic-contribution-held`. It
 * has no dismiss button: it goes when the hold does.
 */
export function GateHeldNotice({ held }: { held: unknown }) {
  let parsed: GateHeld | null;
  try {
    parsed = parseGateHeld(held);
  } catch {
    return (
      <p className="tc-alert" role="alert">
        Some projects set to contribute automatically may be on hold, but this
        build could not read which or why.
      </p>
    );
  }
  if (parsed === null) return null;
  return <GateHeldCard held={parsed} />;
}

function GateHeldCard({ held }: { held: GateHeld }) {
  const copy = useGateHeldNotice(held.wire);
  return (
    <Notice tone="ask" title={copy.data ? copy.data.title : undefined}>
      {copy.data ? (
        <div className="grid gap-2">
          <span>{copy.data.body}</span>
          <ul className="m-0 list-disc pl-5">
            {copy.data.reasons.map((reason) => (
              <li key={reason}>{reason}</li>
            ))}
          </ul>
          <span>{copy.data.release}</span>
          {copy.data.projects.length > 0 && (
            <>
              <span>{copy.data.ask_first}</span>
              <ul className="m-0 grid list-none gap-2 p-0">
                {copy.data.projects.map((project) => (
                  <GateHeldProjectRow
                    key={project.project_id ?? project.line}
                    project={project}
                  />
                ))}
              </ul>
            </>
          )}
        </div>
      ) : (
        <div>
          <span role={copy.isError ? "alert" : undefined}>
            {copy.isError
              ? "Some projects set to contribute automatically are on hold and nothing from them is being sent, but this build could not read the notice that says why."
              : "Loading…"}
          </span>
        </div>
      )}
    </Notice>
  );
}

function GateHeldProjectRow({ project }: { project: GateHeldProject }) {
  const askFirst = useAskFirst();
  return (
    <li className="flex flex-wrap items-center gap-2">
      <span>{project.line}</span>
      {project.project_id !== null && project.ask_first_action && (
        <GlassButton
          type="button"
          disabled={askFirst.isPending}
          onClick={() => askFirst.mutate(project.project_id as string)}
        >
          {project.ask_first_action}
        </GlassButton>
      )}
      {askFirst.isError && project.ask_first_failed && (
        <span className="text-tc-outside" role="alert">
          {project.ask_first_failed}
        </span>
      )}
    </li>
  );
}
