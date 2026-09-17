import { Button } from "@/components/ui/button";
import { SourceRootsPanel } from "../../settings/components/source-roots-panel";
import type { OnboardingStepProps } from "./onboarding-step-types";

export function OnboardingRootsStep({
  onboarding,
  settings,
  roots,
  busy,
}: Pick<OnboardingStepProps, "onboarding" | "settings" | "roots" | "busy">) {
  return (
    <>
      <SourceRootsPanel
        snapshot={settings.data ?? {}}
        busy={roots.busy || settings.state === "loading"}
        error={
          roots.error ||
          (settings.state === "error"
            ? "Source declarations unavailable. Refresh after Rust core starts."
            : null)
        }
        onSave={(source, mode, path) => roots.save(source, mode, path)}
      />
      <div className="mt-6 flex gap-2.5">
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={onboarding.back}
          disabled={busy || roots.busy}
        >
          Back
        </Button>
        <Button
          className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
          type="button"
          onClick={onboarding.continueRoots}
          disabled={busy || roots.busy || settings.state !== "ready"}
        >
          Continue
        </Button>
      </div>
    </>
  );
}
