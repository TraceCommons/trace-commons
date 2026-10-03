import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import { SourceRootsPanel } from "../../settings/public";
import { missingRequiredRoots } from "../roots-readiness";
import type { OnboardingStepProps } from "./onboarding-step-types";
import { ButtonPrimary, GlassButton } from "@/design-system";

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
        <p className="mt-4 mb-0 text-[12px] text-tc-secondary">
          {shell.roots_required}
        </p>
      )}
      {starting && shell && (
        <p className="mt-4 mb-0 text-[12px] text-tc-secondary" role="status">
          {shell.watcher_starting}
        </p>
      )}
      {!starting && startError && (
        <p
          className="tc-card tc-card--quiet mt-4 mb-0 border-tc-outside/30 text-[12px] text-tc-outside"
          role="alert"
        >
          {startError}
        </p>
      )}
      <div className="mt-3 flex flex-wrap gap-2">
        <GlassButton
          type="button"
          onClick={onboarding.back}
          disabled={busy || roots.busy || starting}
        >
          Back
        </GlassButton>
        <ButtonPrimary size="sm"
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
        </ButtonPrimary>
      </div>
    </>
  );
}
