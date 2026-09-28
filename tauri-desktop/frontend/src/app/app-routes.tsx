import { Navigate, Route, Routes, useNavigate } from "react-router-dom";
import { ComputePage } from "../features/compute";
import { HistoryPage } from "../features/history";
import { InsightsPage } from "../features/insights";
import { MissionDraftsPage } from "../features/mission-drafts";
import { OnboardingPage } from "../features/onboarding";
import type { useOnboardingCompletion } from "../features/onboarding/public";
import { PrivateAiPage } from "../features/private-ai";
import { ProfilePage } from "../features/profile";
import type { usePublicProfile } from "../features/profile/public";
import { SettingsPage } from "../features/settings";
import { WaitingPage } from "../features/waiting";
import type { useCoreStatus } from "../lib/tauri/use-core-status";
import { NotFoundPage } from "./not-found-page";
import { flowPaths, routePaths } from "./routes";

export function AppRoutes({
  requiresOnboarding,
  core,
  publicProfile,
  initialInvite,
  onOnboardingComplete,
}: {
  requiresOnboarding: boolean;
  core: ReturnType<typeof useCoreStatus>;
  publicProfile: ReturnType<typeof usePublicProfile>;
  initialInvite: string | null;
  onOnboardingComplete: ReturnType<typeof useOnboardingCompletion>["complete"];
}) {
  return requiresOnboarding ? (
    <OnboardingPage
      key={core.scope}
      alreadyEnrolled={core.data?.daemon.logged_in === true}
      initialInvite={initialInvite}
      onComplete={onOnboardingComplete}
    />
  ) : (
    <Routes>
      <Route path="/" element={<Navigate to={routePaths.insights} replace />} />
      <Route path={routePaths.insights} element={<InsightsPage />} />
      <Route
        path={routePaths.waiting}
        element={<WaitingPage key={core.scope} status={core.data} />}
      />
      <Route
        path={routePaths.history}
        element={<HistoryPage key={core.scope} />}
      />
      <Route
        path={routePaths.settings}
        element={<SettingsWithGrantEntry key={core.scope} />}
      />
      <Route
        path={routePaths.compute}
        element={<ComputePage key={core.scope} />}
      />
      <Route
        path={routePaths["private-ai"]}
        element={<PrivateAiPage key={core.scope} />}
      />
      <Route
        path={routePaths["mission-drafts"]}
        element={<MissionDraftsPage />}
      />
      <Route
        path={routePaths.profile}
        element={
          <ProfilePage
            key={core.scope}
            coreStatus={core.data}
            coreStatusState={core.state}
            onRefresh={core.refresh}
            publicProfile={publicProfile.data}
            publicProfileState={publicProfile.state}
            onPublicProfileRefresh={publicProfile.refresh}
          />
        }
      />
      <Route
        path={flowPaths["automatic-contributing"]}
        element={
          core.data?.daemon.logged_in === true ? (
            <AutomaticContributingFlow key={core.scope} />
          ) : (
            <Navigate to={routePaths.settings} replace />
          )
        }
      />
      <Route path="*" element={<NotFoundPage />} />
    </Routes>
  );
}

/**
 * The Flow 1 grant screens again, for an enrolled contributor (K10): from
 * the scope picker on, through the path, both disclosures and the grant, so
 * a re-grant is fresh consent under the settings now in force. Giving the
 * grant clears the grant's void notice; finishing returns to Settings.
 */
function AutomaticContributingFlow() {
  const navigate = useNavigate();
  return (
    <OnboardingPage
      alreadyEnrolled
      onComplete={() => navigate(routePaths.settings)}
    />
  );
}

/** Settings, with its way into the grant screens. */
function SettingsWithGrantEntry() {
  const navigate = useNavigate();
  return (
    <SettingsPage
      onTurnOnAutomaticContributing={() =>
        navigate(flowPaths["automatic-contributing"])
      }
    />
  );
}
