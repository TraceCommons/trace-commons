import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import type { ReactNode } from "react";
import { openSystemSettings } from "../../../lib/tauri/platform-api";
import type { CapabilityState } from "../../../lib/tauri/platform-api";
import { usePlatformCapabilities } from "../hooks/use-platform-capabilities";

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
    <div className="flex items-center justify-between gap-4 border-b border-border py-3 last:border-b-0">
      <div className="min-w-0">
        <strong className="block">{label}</strong>
        <span className="block text-xs text-muted-foreground">{detail}</span>
      </div>
      <div className="flex shrink-0 items-center gap-3">
        <span className="rounded-full bg-muted px-2.5 py-1 font-mono text-[10px] font-bold uppercase tracking-[.08em] text-muted-foreground">
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
    <section className="rounded-2xl border border-border bg-card/80 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            DESKTOP
          </span>
          <h2>System integrations</h2>
        </div>
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={() => void platform.refetch()}
          disabled={platform.isFetching || platform.busy}
        >
          Refresh
        </Button>
      </div>
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        Native permissions and startup state stay in Rust. Credentials and
        notification bodies never enter this UI state.
      </p>
      {platform.error && (
        <p className="mt-4 rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {platform.error}
        </p>
      )}
      {data && (
        <div className="mt-5 border-t border-border">
          <CapabilityRow
            label="Start at login"
            detail={`${data.os} · ${data.package.name} ${data.package.version}`}
            state={loginState}
            action={
              canToggleLogin ? (
                <Switch
                  aria-label="Start Trace Commons at login"
                  checked={loginState === "enabled"}
                  disabled={platform.busy}
                  onCheckedChange={(checked) =>
                    void platform.setStartAtLogin(checked)
                  }
                />
              ) : loginState === "requires_approval" ? (
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => void openSystemSettings("login_items")}
                  disabled={loginState !== "requires_approval"}
                >
                  System settings
                </Button>
              ) : null
            }
          />
          <CapabilityRow
            label="Notifications"
            detail="Digest notifications use Review and Not now only."
            state={notificationState}
            action={
              notificationState === "requires_approval" ? (
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => void platform.requestNotifications()}
                  disabled={platform.busy}
                >
                  Allow
                </Button>
              ) : notificationState === "denied" ? (
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => void openSystemSettings("notifications")}
                >
                  System settings
                </Button>
              ) : null
            }
          />
          <CapabilityRow
            label="Updates"
            detail={`${data.updates.owner} owns replacement; this build does not fetch or replace itself.`}
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
