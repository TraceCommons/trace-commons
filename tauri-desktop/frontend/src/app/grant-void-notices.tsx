import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Alert, AlertDescription, AlertTitle } from "../components/ui/alert";
import { Button } from "../components/ui/button";
import { changeProjectMode, settingsKeys } from "../features/settings/public";
import { acknowledgeGrantVoids } from "../lib/tauri/core-api";
import {
  type GrantVoid,
  parseGrantVoids,
  rearmTarget,
} from "../lib/tauri/grant-void-notice";
import { coreKeys } from "../lib/tauri/query-keys";
import { useCoreStatus } from "../lib/tauri/use-core-status";
import { useGrantVoidNotice } from "../lib/tauri/use-contributor-copy";

/**
 * Every grant the core voided that no shell has shown yet.
 *
 * Rendered in the app shell, above every page, because a void is a change
 * to what the contributor agreed to: automatic contributing stopped, and
 * they are told wherever they are. The words come from the contributor core
 * (`consent_copy`); this component only lays them out.
 */
export function GrantVoidNotices({ grantVoids }: { grantVoids: unknown }) {
  let voids: GrantVoid[];
  try {
    voids = parseGrantVoids(grantVoids);
  } catch {
    return (
      <p
        className="mx-6 mt-4 rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
        role="alert"
      >
        Automatic contributing may have stopped, but the notice that says
        where and why could not be read. Check your projects' settings.
      </p>
    );
  }
  if (voids.length === 0) return null;
  return (
    <>
      {voids.map((grantVoid) => (
        <GrantVoidNoticeCard key={grantVoid.id} grantVoid={grantVoid} />
      ))}
    </>
  );
}

function GrantVoidNoticeCard({ grantVoid }: { grantVoid: GrantVoid }) {
  const queryClient = useQueryClient();
  const copy = useGrantVoidNotice(grantVoid.id, grantVoid.wire);
  const core = useCoreStatus();
  const acknowledge = useMutation({
    mutationFn: () => acknowledgeGrantVoids([grantVoid.id]),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: coreKeys.status }),
  });
  const projectId = copy.data ? rearmTarget(grantVoid, copy.data) : null;
  // "Turn back on" is the Settings arming call, unchanged: set_project_mode
  // with this project's id and auto_upload, so the daemon applies the same
  // refusals and writes the same armed-auto-upload row, and clears this
  // notice itself. A refusal changes nothing and the notice stays.
  const rearm = useMutation({
    mutationFn: (id: string) => changeProjectMode(id, "auto_upload"),
    onSettled: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.projects(core.scope),
        }),
      ]),
  });
  const busy = acknowledge.isPending || rearm.isPending;

  return (
    <Alert className="mx-6 mt-4 w-auto border-amber-500/40 bg-amber-500/10">
      {copy.data ? (
        <>
          <AlertTitle>{copy.data.title}</AlertTitle>
          <AlertDescription className="grid gap-2">
            <span>{copy.data.body}</span>
            <span className="font-semibold">{copy.data.reasons_heading}</span>
            <ul className="m-0 list-disc pl-5">
              {copy.data.reasons.map((reason) => (
                <li key={reason}>{reason}</li>
              ))}
            </ul>
            <span>{copy.data.rearm}</span>
            <div className="flex flex-wrap gap-2">
              {/* Both buttons are offered only once the notice is on screen.
                  "Turn back on" sits under the sentence that says pressing
                  it agrees to the new settings. */}
              {projectId !== null && copy.data.rearm_action && (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={busy || !copy.data}
                  onClick={() => rearm.mutate(projectId)}
                >
                  {copy.data.rearm_action}
                </Button>
              )}
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={busy || !copy.data}
                onClick={() => acknowledge.mutate()}
              >
                {copy.data.acknowledge}
              </Button>
            </div>
            {rearm.isError && copy.data.rearm_failed && (
              <span className="text-destructive" role="alert">
                {copy.data.rearm_failed}
              </span>
            )}
            {acknowledge.isError && (
              <span className="text-destructive" role="alert">
                This notice could not be dismissed. It will show again.
              </span>
            )}
          </AlertDescription>
        </>
      ) : (
        <AlertDescription>
          {copy.isError
            ? "Automatic contributing stopped, but this build could not read the notice that says where and why. Check your projects' settings."
            : "Loading…"}
        </AlertDescription>
      )}
    </Alert>
  );
}
