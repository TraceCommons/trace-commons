import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { Navigate, Route, Routes, useLocation, useNavigate } from "react-router-dom";
import { AppNavbar } from "../components/app-navbar";
import { SidebarInset, SidebarProvider } from "../components/ui/sidebar";
import { ComputePage } from "../features/compute";
import { HistoryPage } from "../features/history";
import { InsightsPage } from "../features/insights";
import { MissionDraftsPage } from "../features/mission-drafts";
import { OnboardingPage } from "../features/onboarding";
import { PrivateAiPage } from "../features/private-ai";
import { ProfilePage } from "../features/profile";
import { usePublicProfile } from "../features/profile/hooks/use-public-profile";
import { SettingsPage } from "../features/settings";
import { WaitingPage } from "../features/waiting";
import {
  consumeDeepLink,
  openExternalUrl,
} from "../lib/tauri/platform-api";
import { listenTauri } from "../lib/tauri/core-api";
import { useCoreStatus } from "../lib/tauri/use-core-status";
import { useDaemonEvents } from "../lib/tauri/use-daemon-events";
import { routeIdFromPath, routePaths } from "./routes";
import { QuitConfirmation } from "./quit-confirmation";

export function AppShell() {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  useDaemonEvents();
  const route = routeIdFromPath(pathname);
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const previousScope = useRef(core.scope);
  const publicProfile = usePublicProfile();
  const tenantId = core.data?.daemon.tenant_id ?? null;
  const onboardingKey = tenantId
    ? `trace-commons-onboarding-complete:${tenantId}`
    : null;
  const [onboardingComplete, setOnboardingComplete] = useState(false);
  const [initialInvite, setInitialInvite] = useState<string | null>(null);
  const [quitRequested, setQuitRequested] = useState(false);
  useEffect(() => {
    setOnboardingComplete(
      onboardingKey !== null &&
        window.localStorage.getItem(onboardingKey) === "true",
    );
  }, [onboardingKey]);
  useEffect(() => {
    let active = true;
    const consume = async () => {
      try {
        const link = await consumeDeepLink();
        if (!active || !link) return;
        if (link.kind === "enroll") setInitialInvite(link.invite);
        if (link.kind === "credential") navigate("/private-ai");
        if (link.kind === "navigate") navigate(link.path);
        if (link.kind === "public_run") await openExternalUrl(link.url);
      } catch {
        // Browser preview and a cold daemon have no native deep-link surface.
      }
    };
    void consume();
    const timer = window.setInterval(() => void consume(), 1000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [navigate]);
  useEffect(() => {
    let cancelled = false;
    const cleanups: Array<() => void> = [];
    const listen = async () => {
      const unlistenNavigate = await listenTauri<unknown>(
        "navigate",
        (payload) => {
          if (
            typeof payload === "string" &&
            Object.values(routePaths).includes(payload as (typeof routePaths)[keyof typeof routePaths])
          ) {
            navigate(payload);
          }
        },
      );
      if (cancelled) unlistenNavigate();
      else cleanups.push(unlistenNavigate);
      const unlistenQuit = await listenTauri<unknown>("quit-requested", () => {
        if (!cancelled) setQuitRequested(true);
      });
      if (cancelled) unlistenQuit();
      else cleanups.push(unlistenQuit);
    };
    void listen();
    return () => {
      cancelled = true;
      for (const cleanup of cleanups) cleanup();
    };
  }, [navigate]);
  useEffect(() => {
    if (previousScope.current === core.scope) return;
    queryClient.removeQueries({
      predicate: (query) =>
        query.queryKey[0] === "account" && query.queryKey[1] !== core.scope,
    });
    previousScope.current = core.scope;
  }, [core.scope, queryClient]);
  const requiresOnboarding =
    (core.data?.daemon.logged_in === false ||
      (core.data?.daemon.logged_in === true && !onboardingComplete)) &&
    route !== "insights" &&
    route !== "mission-drafts";
  const completeOnboarding = () => {
    if (onboardingKey) window.localStorage.setItem(onboardingKey, "true");
    setOnboardingComplete(true);
  };

  return (
    <SidebarProvider>
      <AppNavbar
        queueCount={core.data?.daemon.queue_depth ?? 0}
        profile={publicProfile.data}
        profileState={publicProfile.state}
      />
      <SidebarInset>
        <main className="min-w-0 flex-1">
          {requiresOnboarding ? (
            <OnboardingPage
              key={core.scope}
              alreadyEnrolled={core.data?.daemon.logged_in === true}
              initialInvite={initialInvite}
              onComplete={completeOnboarding}
            />
          ) : (
            <Routes>
              <Route
                path="/"
                element={<Navigate to={routePaths.insights} replace />}
              />
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
              <Route
                path="*"
                element={<Navigate to={routePaths.insights} replace />}
              />
            </Routes>
          )}
        </main>
      </SidebarInset>
      <QuitConfirmation open={quitRequested} onOpenChange={setQuitRequested} />
    </SidebarProvider>
  );
}
