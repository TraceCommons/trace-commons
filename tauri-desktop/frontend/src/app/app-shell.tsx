import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { useLocation } from "react-router-dom";
import { useOnboardingCompletion } from "../features/onboarding/public";
import { TracesWorkspaceProvider } from "../features/waiting/public";
import { privateAiKeys } from "../features/private-ai/public";
import { usePublicProfile } from "../features/profile/public";
import { useCoreStatus } from "../lib/tauri/use-core-status";
import { AppRoutes } from "./app-routes";
import { DaemonStartupNotice } from "./daemon-startup-notice";
import { GrantVoidNotices } from "./grant-void-notices";
import { useAccountQueryLifecycle } from "./hooks/use-account-query-lifecycle";
import { useDaemonQueryEvents } from "./hooks/use-daemon-query-events";
import { useDesktopEvents } from "./hooks/use-desktop-events";
import { LegacyMigrationNotice } from "./legacy-migration-notice";
import { QuitConfirmation } from "./quit-confirmation";
import { ArmingRewordingNotices, GateHeldNotice } from "./switch-on-notices";
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

  const notices = (
    <>
      {errors.map((error) => (
        <p key={error} className="tc-alert m-0" role="alert">
          {error}
        </p>
      ))}
      <DaemonStartupNotice startup={core.data?.startup} />
      <GrantVoidNotices grantVoids={core.data?.daemon.grant_voids} />
      <LegacyMigrationNotice
        status={core.data?.daemon.legacy_invite_migration}
      />
      <ArmingRewordingNotices
        rewordings={core.data?.daemon.arming_rewordings}
      />
      <GateHeldNotice held={core.data?.daemon.automatic_contribution_held} />
    </>
  );

  return (
    <TracesWorkspaceProvider key={core.scope}>
      <AppRoutes
        requiresOnboarding={requiresOnboarding}
        core={core}
        publicProfile={publicProfile}
        initialInvite={desktop.initialInvite}
        onOnboardingComplete={onboarding.complete}
        notices={notices}
      />
      <QuitConfirmation
        open={desktop.quitRequested}
        onOpenChange={desktop.setQuitRequested}
      />
    </TracesWorkspaceProvider>
  );
}
