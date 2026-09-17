import { useState } from "react";
import { PageHeader } from "../../components/page-header";
import { useProjects } from "../settings/hooks/use-projects";
import { useSettings } from "../settings/hooks/use-settings";
import { useSourceRoots } from "../settings/hooks/use-source-roots";
import { OnboardingStepContent } from "./components/onboarding-step-content";
import { ScrubDisclosure } from "./components/scrub-disclosure";
import { useOnboarding } from "./hooks/use-onboarding";
import { useScrubberPatterns } from "./hooks/use-scrubber-patterns";

export function OnboardingPage({
  onComplete,
  alreadyEnrolled = false,
  initialInvite = null,
}: {
  onComplete: () => void;
  alreadyEnrolled?: boolean;
  initialInvite?: string | null;
}) {
  const onboarding = useOnboarding(alreadyEnrolled);
  const settings = useSettings();
  const projects = useProjects();
  const roots = useSourceRoots();
  const [showScrubDisclosure, setShowScrubDisclosure] = useState(false);
  const busy = onboarding.state === "loading" || onboarding.state === "busy";
  const showPrivacy = settings.data?.near_ai_configured === true;
  const scrubber = useScrubberPatterns(showScrubDisclosure);
  const pageTitles = {
    welcome: "Welcome to Trace Commons",
    roots: "Choose source roots",
    connect: "Connect a commons",
    consent: "Choose your consent",
    privacy: "Extra scrub before sending?",
    projects: "What to watch",
    done: "You're set up",
  } as const;
  const pageDescriptions = {
    welcome: "A local contributor for sessions you choose to share.",
    roots:
      "Declare which local folders may be watched. Nothing is auto-discovered.",
    connect: "Enrollment happens only when you press Connect.",
    consent:
      "Always-on scope is required. Optional scopes stay off until selected.",
    privacy: "Local scrubbing runs either way. A second scanner is optional.",
    projects:
      "Every project starts at ask-first. Ignore a project to leave it out entirely.",
    done: "Nothing has been sent yet.",
  } as const;
  return (
    <div className="mx-auto max-w-[1080px] px-4 pb-12 pt-8 sm:px-8 sm:pb-16 sm:pt-10 lg:px-16 lg:pt-14 block">
      <PageHeader
        eyebrow="SETUP / CONTRIBUTION"
        title={pageTitles[onboarding.step]}
        description={pageDescriptions[onboarding.step]}
        phase={`${onboarding.step.toUpperCase()} · SETUP`}
      />
      {onboarding.error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {onboarding.error}
        </p>
      )}
      <OnboardingStepContent
        step={onboarding.step}
        onboarding={onboarding}
        settings={settings}
        projects={projects}
        roots={roots}
        alreadyEnrolled={alreadyEnrolled}
        initialInvite={initialInvite}
        busy={busy}
        showPrivacy={showPrivacy}
        onOpenScrubDisclosure={() => setShowScrubDisclosure(true)}
        onComplete={onComplete}
      />
      {onboarding.step !== "welcome" && onboarding.step !== "done" && (
        <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
          This flow is resumable. A failed daemon call does not advance the
          step.
        </p>
      )}
      <ScrubDisclosure
        open={showScrubDisclosure}
        names={scrubber.names}
        state={scrubber.state}
        error={scrubber.error}
        onRetry={() => void scrubber.refresh()}
        onClose={() => setShowScrubDisclosure(false)}
      />
    </div>
  );
}
