import { useState } from "react";
import { Button } from "@/components/ui/button";
import { openSystemSettings } from "../../../lib/tauri/platform-api";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import { usePlatformCapabilities } from "../../settings/public";
import type { OnboardingStepProps } from "./onboarding-step-types";

export function OnboardingDoneStep({
  onboarding,
  onComplete,
}: Pick<OnboardingStepProps, "onboarding" | "onComplete">) {
  const disclosure = useContributorDisclosureCopy();
  const copy = disclosure.data;
  const platform = usePlatformCapabilities();
  const [notificationDismissed, setNotificationDismissed] = useState(false);
  const notificationState = platform.data?.notifications.state ?? "unknown";
  const notificationText = !copy
    ? null
    : notificationState === "available"
      ? copy.onboarding_shell.notification_allowed
      : notificationState === "denied"
        ? copy.onboarding_shell.notification_denied
        : notificationState === "requires_approval"
          ? copy.onboarding_shell.notification_not_asked
          : copy.onboarding_shell.notification_unknown;
  return (
    <section className="tc-card mb-2.5">
      <span className="mb-1.5 block tc-eyebrow">
        DONE
      </span>
      <h2>{copy?.onboarding.heading ?? "Your first contribution"}</h2>
      {copy ? (
        <div className="grid gap-3">
          <p>{copy.onboarding_shell.done_body}</p>
          <p>{copy.onboarding.review}</p>
          <p>{copy.onboarding.follow_up}</p>
          <p>{copy.onboarding.agent_setup}</p>
          <section className="grid gap-2 border-t border-border pt-4">
            <h3>{copy.onboarding_shell.notification_heading}</h3>
            <p className="m-0 text-sm text-muted-foreground">
              {copy.onboarding_shell.notification_purpose}
            </p>
            {notificationState === "requires_approval" && !notificationDismissed && (
              <p className="m-0 text-sm">{copy.onboarding_shell.notification_offer}</p>
            )}
            {notificationText && (
              <p className="m-0 text-sm text-muted-foreground" role="status">
                {notificationText}
              </p>
            )}
            {notificationState === "requires_approval" && !notificationDismissed && (
              <div className="flex flex-wrap gap-2">
                <Button
                  type="button"
                  variant="outline"
                  disabled={platform.busy}
                  onClick={() => void platform.requestNotifications()}
                >
                  {platform.busy
                    ? "Requesting…"
                    : copy.onboarding_shell.notification_allow}
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  onClick={() => setNotificationDismissed(true)}
                >
                  {copy.onboarding_shell.not_now}
                </Button>
              </div>
            )}
            {notificationState === "denied" && (
              <Button
                type="button"
                variant="outline"
                onClick={() => void openSystemSettings("notifications")}
              >
                {copy.onboarding_shell.system_settings}
              </Button>
            )}
          </section>
        </div>
      ) : (
        <p role="alert">Shared onboarding copy unavailable. Retry loading it.</p>
      )}
      {onboarding.withdrawn && (
        <p className="m-0 mb-3 text-sm" role="status">
          Automatic contributing is off. No automatic grant is in force. Settings
          shows this under Automatic contributing.
        </p>
      )}
      {onboarding.granted && !onboarding.withdrawn && (
        <Button
          className="mb-3 mr-2.5"
          type="button"
          variant="outline"
          onClick={() => void onboarding.withdrawGrant()}
          disabled={onboarding.state === "busy"}
        >
          Turn off automatic contributing
        </Button>
      )}
      <Button
        className="tc-btn tc-btn--primary tc-btn--sm"
        type="button"
        onClick={onComplete}
        disabled={!copy}
      >
        Done
      </Button>
    </section>
  );
}
