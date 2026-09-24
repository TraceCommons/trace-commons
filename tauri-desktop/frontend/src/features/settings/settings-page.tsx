import { Button } from "@/components/ui/button";
import { PageHeader } from "../../components/page-header";
import { useCoreStatus } from "../../lib/tauri/use-core-status";
import { AuditPanel } from "./components/audit-panel";
import { BehaviorSettingsPanel } from "./components/behavior-settings-panel";
import { ConnectionPanel } from "./components/connection-panel";
import { ConsentSettingsPanel } from "./components/consent-settings-panel";
import { PrivacyControlsPanel } from "./components/privacy-controls-panel";
import { PlatformPanel } from "./components/platform-panel";
import { ProjectsPanel } from "./components/projects-panel";
import { RoutingPanel } from "./components/routing-panel";
import { SettingRow } from "./components/setting-row";
import { SourceRootsPanel } from "./components/source-roots-panel";
import { WitnessPanel } from "./components/witness-panel";
import { useAudit } from "./hooks/use-audit";
import { useBehaviorSettings } from "./hooks/use-behavior-settings";
import { useConsentSettings } from "./hooks/use-consent-settings";
import { useDaemonControl } from "./hooks/use-daemon-control";
import { usePrivacyControls } from "./hooks/use-privacy-controls";
import { useProjects } from "./hooks/use-projects";
import { useRouting } from "./hooks/use-routing";
import { useSettings } from "./hooks/use-settings";
import { useSourceRoots } from "./hooks/use-source-roots";
import { useWitness } from "./hooks/use-witness";

function settingValue(
  settings: Record<string, unknown> | null,
  key: string,
  fallback = "—",
) {
  const value = settings?.[key];
  return value === undefined || value === null ? fallback : String(value);
}

export function SettingsPage() {
  const settings = useSettings();
  const core = useCoreStatus();
  const daemon = useDaemonControl();
  const roots = useSourceRoots();
  const projects = useProjects();
  const audit = useAudit();
  const privacy = usePrivacyControls();
  const behavior = useBehaviorSettings();
  const witness = useWitness();
  const consent = useConsentSettings();
  const routing = useRouting();
  const snapshot = settings.data;
  return (
    <div className="mx-auto max-w-[1080px] px-4 pb-12 pt-8 sm:px-8 sm:pb-16 sm:pt-10 lg:px-16 lg:pt-14">
      <PageHeader
        eyebrow="ACCOUNT / CONTROL"
        title="Settings"
        description="What this machine watches, and what your traces are allowed to do."
        phase="PHASE 4"
      />
      <ConnectionPanel status={core.data} settings={settings.data} />
      <PlatformPanel />
      {settings.state === "loading" && (
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          Reading daemon settings…
        </p>
      )}
      {settings.state === "error" && (
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          Settings unavailable. Refresh after Rust core starts.
        </p>
      )}
      {core.data && (
        <section className="rounded-2xl border border-border bg-card/80 p-[26px]">
          <div className="flex items-start justify-between gap-[18px]">
            <div>
              <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
                DAEMON
              </span>
              <h2>Contribution watcher</h2>
            </div>
            <span
              className={`whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start ${core.data.daemon.paused ? "bg-muted text-muted-foreground" : ""}`}
            >
              {core.data.daemon.paused ? "Paused" : "Watching"}
            </span>
          </div>
          <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
            Pausing stops contribution processing. It does not delete queued
            sessions or change consent.
          </p>
          {daemon.error && (
            <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
              {daemon.error}
            </p>
          )}
          <div className="mt-6 flex gap-2.5">
            <Button
              className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
              type="button"
              onClick={() => void daemon.command("pause_daemon")}
              disabled={daemon.busy || core.data.daemon.paused}
            >
              Pause watcher
            </Button>
            <Button
              className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
              type="button"
              onClick={() => void daemon.command("resume_daemon")}
              disabled={daemon.busy || !core.data.daemon.paused}
            >
              Resume watcher
            </Button>
          </div>
        </section>
      )}
      {settings.state === "ready" && (
        <div className="grid gap-4">
          <section className="rounded-2xl border border-border bg-card/80 p-[26px]">
            <div className="flex items-start justify-between gap-[18px]">
              <div>
                <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
                  WATCHER
                </span>
                <h2>Session discovery</h2>
              </div>
              <Button
                className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
                type="button"
                onClick={() => void settings.refresh()}
              >
                Refresh
              </Button>
            </div>
            <SettingRow
              label="Poll interval"
              value={`${settingValue(snapshot, "poll_interval_secs")} sec`}
              detail="How often local sources are checked"
            />
            <SettingRow
              label="Queue lifetime"
              value={`${settingValue(snapshot, "queue_ttl_days")} days`}
              detail="How long unresolved entries remain"
            />
            <SettingRow
              label="Notifications"
              value={settingValue(snapshot, "local_notifications", "off")}
              detail="Daemon-level notifications"
            />
          </section>
          <SourceRootsPanel
            snapshot={snapshot ?? {}}
            busy={roots.busy}
            error={roots.error}
            onSave={(source, mode, path) => roots.save(source, mode, path)}
          />
          <ConsentSettingsPanel
            options={consent.options}
            granted={core.data?.daemon.consent_scopes ?? []}
            state={consent.state}
            error={consent.error}
            onRefresh={consent.refresh}
            onToggle={consent.toggle}
          />
          <ProjectsPanel
            projects={projects.projects}
            state={projects.state}
            error={projects.error}
            onRefresh={projects.refresh}
            onSetMode={projects.setMode}
          />
          <AuditPanel
            entries={audit.entries}
            state={audit.state}
            error={audit.error}
            onRefresh={audit.refresh}
          />
          <PrivacyControlsPanel
            settings={snapshot ?? {}}
            storage={privacy.storage}
            state={privacy.state}
            error={privacy.error}
            onRefresh={privacy.refreshStorage}
            onInference={privacy.setInferenceEvidence}
            onToken={privacy.setTokenContribution}
            onCapture={privacy.setTokenCapture}
            onCleanup={privacy.cleanTokenStorage}
          />
          <BehaviorSettingsPanel
            settings={snapshot ?? {}}
            busy={behavior.busy}
            error={behavior.error}
            onRefresh={settings.refresh}
            onSave={(setting, value) => behavior.save(setting, value)}
          />
          <WitnessPanel
            data={witness.data}
            state={witness.state}
            error={witness.error}
            onRefresh={witness.refresh}
            onConfigure={witness.configure}
            onClear={witness.clear}
          />
          <RoutingPanel
            snapshot={snapshot ?? {}}
            status={core.data}
            discovery={routing.discovery}
            evidence={routing.evidence}
            state={routing.state}
            error={routing.error}
            onRefresh={routing.refresh}
            onCheck={routing.check}
            onConfigure={routing.configure}
          />
          <section className="rounded-2xl border border-border bg-card/80 p-[26px]">
            <div>
              <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
                LIMITS
              </span>
              <h2>Daily guardrails</h2>
            </div>
            <SettingRow
              label="Uploads per day"
              value={settingValue(snapshot, "max_uploads_per_day")}
              detail="Hard daily count cap"
            />
            <SettingRow
              label="Bytes per day"
              value={settingValue(snapshot, "max_bytes_per_day")}
              detail="Hard daily volume cap"
            />
            <SettingRow
              label="Queue capacity"
              value={settingValue(snapshot, "max_queue_entries")}
              detail="Maximum local entries"
            />
          </section>
          <section className="rounded-2xl border border-border bg-card/80 p-[26px]">
            <div>
              <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
                CREDENTIALS
              </span>
              <h2>Connection posture</h2>
            </div>
            <SettingRow
              label="Privacy filter"
              value={settingValue(snapshot, "near_ai_configured", "false")}
              detail="Presence only; secret never crosses IPC"
            />
            <SettingRow
              label="Inference access"
              value={settingValue(
                snapshot,
                "near_ai_inference_configured",
                "false",
              )}
              detail="Presence only; secret never crosses IPC"
            />
            <SettingRow
              label="Session retained"
              value={settingValue(
                snapshot,
                "near_ai_session_retained",
                "false",
              )}
              detail="Presence only; secret never crosses IPC"
            />
          </section>
        </div>
      )}
    </div>
  );
}
