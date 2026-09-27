import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { useLocation } from "react-router-dom";
import { AppNavbar } from "../components/app-navbar";
import { SidebarInset, SidebarProvider } from "../components/ui/sidebar";
import { useOnboardingCompletion } from "../features/onboarding/public";
import { privateAiKeys } from "../features/private-ai/public";
import { usePublicProfile } from "../features/profile/public";
import { useCoreStatus } from "../lib/tauri/use-core-status";
import { AppRoutes } from "./app-routes";
import { useAccountQueryLifecycle } from "./hooks/use-account-query-lifecycle";
import { useDaemonQueryEvents } from "./hooks/use-daemon-query-events";
import { useDesktopEvents } from "./hooks/use-desktop-events";
import { QuitConfirmation } from "./quit-confirmation";
import { DaemonStartupNotice } from "./daemon-startup-notice";
import { GrantVoidNotices } from "./grant-void-notices";
import { routeIdFromPath } from "./routes";

export function AppShell() {
  const { pathname } = useLocation();
  const route = routeIdFromPath(pathname);
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const refreshPrivateAiCredential = useCallback(() => {
    void queryClient.invalidateQueries({
      queryKey: privateAiKeys.credential(core.scope),
    });
  }, [core.scope, queryClient]);
  const daemonEventError = useDaemonQueryEvents(core.scope);
  useAccountQueryLifecycle(core.scope);
  const publicProfile = usePublicProfile();
  const tenantId = core.data?.daemon.tenant_id ?? null;
  const onboarding = useOnboardingCompletion(tenantId);
  const desktop = useDesktopEvents(refreshPrivateAiCredential);
  const errors = [
    desktop.error,
    daemonEventError
      ? "Live daemon updates are unavailable. Refresh data manually."
      : null,
  ].filter((error): error is string => error !== null);
  const requiresOnboarding =
    (core.data?.daemon.logged_in === false ||
      (core.data?.daemon.logged_in === true && !onboarding.isComplete)) &&
    route !== null;

  return (
    <SidebarProvider>
      <AppNavbar
        queueCount={core.data?.daemon.queue_depth ?? 0}
        profile={publicProfile.data}
        profileState={publicProfile.state}
      />
      <SidebarInset>
        <main className="min-w-0 flex-1">
          {errors.map((error) => (
            <p
              key={error}
              className="mx-6 mt-4 rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
              role="alert"
            >
              {error}
            </p>
          ))}
          <DaemonStartupNotice startup={core.data?.startup} />
          <GrantVoidNotices grantVoids={core.data?.daemon.grant_voids} />
          <AppRoutes
            requiresOnboarding={requiresOnboarding}
            core={core}
            publicProfile={publicProfile}
            initialInvite={desktop.initialInvite}
            onOnboardingComplete={onboarding.complete}
          />
        </main>
      </SidebarInset>
      <QuitConfirmation
        open={desktop.quitRequested}
        onOpenChange={desktop.setQuitRequested}
      />
    </SidebarProvider>
  );
}
