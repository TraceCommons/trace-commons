import type { ReactNode } from "react";
import { Navigate, Route, Routes, useNavigate } from "react-router-dom";
import { Pane, Window } from "../design-system";
import { OnboardingPage } from "../features/onboarding";
import type { useOnboardingCompletion } from "../features/onboarding/public";
import type { usePublicProfile } from "../features/profile/public";
import type { useCoreStatus } from "../lib/tauri/use-core-status";
import { MonitorShell } from "./monitor/monitor-shell";
import { flowPaths, routePaths } from "./routes";

export function AppRoutes({
  requiresOnboarding,
  core,
  publicProfile,
  initialInvite,
  onOnboardingComplete,
  notices,
}: {
  requiresOnboarding: boolean;
  core: ReturnType<typeof useCoreStatus>;
  publicProfile: ReturnType<typeof usePublicProfile>;
  initialInvite: string | null;
  onOnboardingComplete: ReturnType<typeof useOnboardingCompletion>["complete"];
  notices: ReactNode;
}) {
  return requiresOnboarding ? (
    <FirstRunWindow notices={notices}>
      <OnboardingPage
        key={core.scope}
        alreadyEnrolled={core.data?.daemon.logged_in === true}
        initialInvite={initialInvite}
        onComplete={onOnboardingComplete}
      />
    </FirstRunWindow>
  ) : (
    <Routes>
      <Route
        path={flowPaths["automatic-contributing"]}
        element={
          core.data?.daemon.logged_in === true ? (
            <FirstRunWindow notices={notices}>
              <AutomaticContributingFlow key={core.scope} />
            </FirstRunWindow>
          ) : (
            <Navigate to={routePaths.settings} replace />
          )
        }
      />
      <Route
        path="*"
        element={
          <MonitorShell
            core={core}
            publicProfile={publicProfile}
            notices={notices}
          />
        }
      />
    </Routes>
  );
}

/**
 * First run, and the grant screens again: one pane over the scene, the
 * modal width the FTUX flows use, with the core notices above the flow.
 */
function FirstRunWindow({
  notices,
  children,
}: {
  notices: ReactNode;
  children: ReactNode;
}) {
  return (
    <Window className="h-screen justify-center">
      <Pane
        className="flex w-full max-w-[720px] flex-col overflow-hidden"
        data-tauri-drag-region
      >
        <div className="min-h-0 flex-1 overflow-auto px-6 pt-10 pb-6">
          <div className="tc-page mb-2.5">{notices}</div>
          {children}
        </div>
      </Pane>
    </Window>
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

