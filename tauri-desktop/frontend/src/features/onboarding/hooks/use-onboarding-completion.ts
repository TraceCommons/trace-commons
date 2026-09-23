import { useState } from "react";

function completionKey(tenantId: string) {
  return `trace-commons-onboarding-complete:${tenantId}`;
}

function readCompletion(tenantId: string) {
  try {
    return window.localStorage.getItem(completionKey(tenantId)) === "true";
  } catch {
    return false;
  }
}

export function useOnboardingCompletion(tenantId: string | null) {
  const [sessionCompletion, setSessionCompletion] = useState<{
    tenantId: string | null;
  } | null>(null);
  const complete = () => {
    if (tenantId) {
      try {
        window.localStorage.setItem(completionKey(tenantId), "true");
      } catch {
        // Keep the completed flow usable for this session if storage is blocked.
      }
    }
    setSessionCompletion({ tenantId });
  };

  return {
    isComplete:
      (sessionCompletion !== null && sessionCompletion.tenantId === tenantId) ||
      (tenantId !== null && readCompletion(tenantId)),
    complete,
  };
}
