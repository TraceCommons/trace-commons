import { useEffect, useRef, useState } from "react";
import { PageHeader } from "../../components/page-header";
import { StepProgress } from "../../design-system";
import { useProjects, useSettings, useSourceRoots } from "../settings/public";
import { OnboardingStepContent } from "./components/onboarding-step-content";
import { ScrubDisclosure } from "./components/scrub-disclosure";
import { useOnboarding } from "./hooks/use-onboarding";
import { useScrubberPatterns } from "./hooks/use-scrubber-patterns";

/** Progress groups for the step indicator: sequential nodes, not tabs. */
const progressGroups = [
  { label: "Folders", steps: ["welcome", "roots"] },
  { label: "Join", steps: ["connect"] },
  { label: "Uses", steps: ["consent", "path", "privacy"] },
  { label: "Inference", steps: ["inference"] },
  { label: "Sharing", steps: ["disclosure_scrub", "disclosure_witness", "grant"] },
  { label: "Projects", steps: ["projects", "done"] },
] as const;

function progressIndex(step: string) {
  const index = progressGroups.findIndex((group) =>
    (group.steps as readonly string[]).includes(step),
  );
  return index === -1 ? 0 : index;
}

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
  const showPrivacy =
    settings.state === "ready"
      ? settings.data?.near_ai_configured === true
      : null;
  const scrubber = useScrubberPatterns(showScrubDisclosure);
  // On a step change, move focus to the new step's heading, so keyboard and
  // screen-reader users land on the step rather than on the page body. Not
  // on the first render: the page opening is not a step change.
  const stepRef = useRef<HTMLDivElement>(null);
  const renderedStep = useRef(onboarding.step);
  useEffect(() => {
    if (renderedStep.current === onboarding.step) return;
    renderedStep.current = onboarding.step;
    const heading = stepRef.current?.querySelector<HTMLElement>("h2");
    if (!heading) return;
    heading.tabIndex = -1;
    heading.focus();
  }, [onboarding.step]);
  const pageTitles = {
    welcome: "Welcome to Trace Commons",
    roots: "Choose source roots",
    connect: "Connect a commons",
    consent: "Choose your consent",
    path: "Choose how to contribute",
    privacy: "Extra scrub before sending?",
    inference: "Connect inference",
    disclosure_scrub: "What automatic contributing removes",
    disclosure_witness: "Where sessions go",
    grant: "Turn on automatic contributing",
    projects: "What to watch",
    done: "You're set up",
  } as const;
  const pageDescriptions = {
    welcome: "A local contributor for sessions you choose to share.",
    roots:
      "Declare which local folders may be watched. Nothing is auto-discovered.",
    connect: "Enrollment happens only when you press Connect.",
    consent:
      "Nothing is pre-selected. Tick the required scope yourself to continue.",
    path: "Neither option is chosen for you.",
    privacy: "Local scrubbing runs either way. A second scanner is optional.",
    inference: "Optional. Skipping changes nothing.",
    disclosure_scrub: "Read this before turning anything on.",
    disclosure_witness: "Read from this device's current settings.",
    grant: "Nothing is turned on until you press the button.",
    projects:
      "Every project starts at ask-first. Ignore a project to leave it out entirely.",
    done: "Nothing has been sent yet.",
  } as const;
  return (
    <div className="tc-page">
      <StepProgress
        labels={progressGroups.map((group) => group.label)}
        current={
          onboarding.step === "done"
            ? progressGroups.length
            : progressIndex(onboarding.step)
        }
      />
      <PageHeader
        title={pageTitles[onboarding.step]}
        description={pageDescriptions[onboarding.step]}
      />
      {onboarding.error && (
        <p className="tc-alert">
          {onboarding.error}
        </p>
      )}
      <div ref={stepRef}>
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
      </div>
      {onboarding.step !== "welcome" && onboarding.step !== "done" && (
        <p className="m-0 tc-caption tc-text-tertiary">
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
