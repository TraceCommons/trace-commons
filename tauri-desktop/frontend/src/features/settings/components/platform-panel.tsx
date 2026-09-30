import type { ReactNode } from "react";
import { openSystemSettings } from "../../../lib/tauri/platform-api";
import type { CapabilityState } from "../../../lib/tauri/platform-api";
import { usePlatformCapabilities } from "../hooks/use-platform-capabilities";
import { GlassButton, TertiaryLink, Toggle } from "@/design-system";

function stateLabel(state: CapabilityState) {
  return state.replaceAll("_", " ");
}

function CapabilityRow({
  label,
  detail,
  state,
  action,
}: {
  label: string;
  detail: string;
  state: CapabilityState;
  action?: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-4 border-b border-tc-hairline py-3 last:border-b-0">
      <div className="min-w-0">
        <strong className="block">{label}</strong>
        <span className="block text-xs text-tc-secondary">{detail}</span>
      </div>
      <div className="flex shrink-0 items-center gap-3">
        <span className="tc-card font-mono text-[10px] font-bold uppercase tracking-[.08em] text-tc-secondary">
          {stateLabel(state)}
        </span>
        {action}
      </div>
    </div>
  );
}

export function PlatformPanel() {
  const platform = usePlatformCapabilities();
  const data = platform.data;
  const loginState = data?.startup.state ?? "unknown";
  const notificationState = data?.notifications.state ?? "unknown";
  const canToggleLogin =
    loginState === "not_registered" || loginState === "enabled";
  return (
    <section className="tc-card">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            DESKTOP
          </span>
          <h2>System integrations</h2>
        </div>
        <TertiaryLink
          type="button"
          onClick={() => void platform.refetch()}
          disabled={platform.isFetching || platform.busy}
        >
          Refresh
        </TertiaryLink>
      </div>
      <p className="m-0 tc-caption tc-text-tertiary">
        Native permissions and startup state stay in Rust. Credentials and
        notification bodies never enter this UI state.
      </p>
      {platform.error && (
        <p className="tc-card tc-card--quiet mt-4 border-tc-outside/30 text-[12px] text-tc-outside">
          {platform.error}
        </p>
      )}
      {data && (
        <div className="mt-5 border-t border-tc-hairline">
          <CapabilityRow
            label="Start at login"
            detail={`${data.os} · ${data.package.name} ${data.package.version}`}
            state={loginState}
            action={
              canToggleLogin ? (
                <Toggle
                  settings
                  label="Start Trace Commons at login"
                  checked={loginState === "enabled"}
                  disabled={platform.busy}
                  onChange={(checked) =>
                    void platform.setStartAtLogin(checked)
                  }
                />
              ) : loginState === "requires_approval" ? (
                <GlassButton
                  type="button"
                  onClick={() => void openSystemSettings("login_items")}
                  disabled={loginState !== "requires_approval"}
                >
                  System settings
                </GlassButton>
              ) : null
            }
          />
          <CapabilityRow
            label="Notifications"
            detail="Digest notifications use Review and Not now only."
            state={notificationState}
            action={
              notificationState === "requires_approval" ? (
                <GlassButton
                  type="button"
                  onClick={() => void platform.requestNotifications()}
                  disabled={platform.busy}
                >
                  Allow
                </GlassButton>
              ) : notificationState === "denied" ? (
                <GlassButton
                  type="button"
                  onClick={() => void openSystemSettings("notifications")}
                >
                  System settings
                </GlassButton>
              ) : null
            }
          />
          <CapabilityRow
            label="Updates"
            detail={`${data.updates.owner} owns replacement: ${data.updates.action}. This build does not fetch or replace itself.`}
            state={data.updates.state}
          />
          <CapabilityRow
            label="Deep links"
            detail={`${data.deep_links.scheme}:// enrollment, credential, and public-run routes`}
            state={data.deep_links.state}
          />
          <CapabilityRow
            label="Tray"
            detail="Daemon-derived counts and review navigation."
            state={data.tray.state}
          />
        </div>
      )}
    </section>
  );
}
