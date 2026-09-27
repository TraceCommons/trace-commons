import { Navigate, Route, Routes } from "react-router-dom";
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
import { routePaths } from "./routes";

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
        element={<SettingsPage key={core.scope} />}
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
      <Route path="*" element={<NotFoundPage />} />
    </Routes>
  );
}
