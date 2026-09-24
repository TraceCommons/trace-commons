import { Button } from "@/components/ui/button";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import { SourceRootsPanel } from "../../settings/public";
import { missingRequiredRoots } from "../roots-readiness";
import type { OnboardingStepProps } from "./onboarding-step-types";

export function OnboardingRootsStep({
  onboarding,
  settings,
  roots,
  busy,
}: Pick<OnboardingStepProps, "onboarding" | "settings" | "roots" | "busy">) {
  const disclosure = useContributorDisclosureCopy();
  const shell = disclosure.data?.onboarding_shell;
  const copyReady = Boolean(disclosure.data?.source_settings && shell);
  const snapshot = settings.data ?? {};
  const rootsAnswered =
    settings.state === "ready" && missingRequiredRoots(snapshot).length === 0;
  const starting = onboarding.startingDaemon;
  const startError =
    onboarding.rootsStartError === "roots_required"
      ? shell?.roots_required
      : onboarding.rootsStartError === "start_failed"
        ? shell?.watcher_start_failed
        : null;
  return (
    <>
      <SourceRootsPanel
        snapshot={snapshot}
        busy={roots.busy || settings.state === "loading" || starting}
        error={
          roots.error ||
          (settings.state === "error"
            ? "Source declarations unavailable. Refresh after Rust core starts."
            : null)
        }
        onSave={(source, mode, path) => roots.save(source, mode, path)}
      />
      {settings.state === "ready" && !rootsAnswered && shell && (
        <p className="mt-4 mb-0 text-[12px] text-muted-foreground">
          {shell.roots_required}
        </p>
      )}
      {starting && shell && (
        <p className="mt-4 mb-0 text-[12px] text-muted-foreground" role="status">
          {shell.watcher_starting}
        </p>
      )}
      {!starting && startError && (
        <p
          className="mt-4 mb-0 rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive"
          role="alert"
        >
          {startError}
        </p>
      )}
      <div className="mt-6 flex gap-2.5">
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={onboarding.back}
          disabled={busy || roots.busy || starting}
        >
          Back
        </Button>
        <Button
          className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
          type="button"
          onClick={() => void onboarding.continueRoots(snapshot)}
          disabled={
            busy ||
            roots.busy ||
            starting ||
            settings.state !== "ready" ||
            !copyReady ||
            !rootsAnswered
          }
        >
          {starting && shell ? shell.watcher_starting : "Continue"}
        </Button>
      </div>
    </>
  );
}
