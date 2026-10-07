import type { ReactNode } from "react";
import { ButtonPrimary, GlassButton, SectionRule, TertiaryLink } from "../../design-system";
import { settingsSections } from "./sections";
import { RouteDisclosurePanel } from "../../components/route-disclosure";
import { useCoreStatus } from "../../lib/tauri/use-core-status";
import { useShellStatusLines } from "../../lib/tauri/use-contributor-copy";
import { AuditPanel } from "./components/audit-panel";
import { AutomaticGrantPanel } from "./components/automatic-grant-panel";
import { BehaviorSettingsPanel } from "./components/behavior-settings-panel";
import { ConnectionPanel } from "./components/connection-panel";
import { ConsentSettingsPanel } from "./components/consent-settings-panel";
import { LegacyMigrationPanel } from "./components/legacy-migration-panel";
import { PlatformPanel } from "./components/platform-panel";
import { PrivacyControlsPanel } from "./components/privacy-controls-panel";
import { ProjectsPanel } from "./components/projects-panel";
import { RoutingPanel } from "./components/routing-panel";
import { SettingRow } from "./components/setting-row";
import { SourceRootsPanel } from "./components/source-roots-panel";
import { WitnessPanel } from "./components/witness-panel";
import { useAudit } from "./hooks/use-audit";
import { useAutomaticGrant } from "./hooks/use-automatic-grant";
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


const sectionTitle = Object.fromEntries(
  settingsSections.map((section) => [section.id, section.label]),
) as Record<(typeof settingsSections)[number]["id"], string>;

export function SettingsPage({
  onTurnOnAutomaticContributing,
  profile,
  privateAi,
  compute,
}: {
  /** Opens the Flow 1 grant screens again (K10). */
  onTurnOnAutomaticContributing?: () => void;
  /** The public profile, rendered in its own section. */
  profile?: ReactNode;
  /** The Private AI section's content (it has its own screen). */
  privateAi?: ReactNode;
  /**
   * The Compute section's content. Drawn whatever the settings snapshot's
   * state, so compute consent can always be paused or withdrawn.
   */
  compute?: ReactNode;
} = {}) {
  const lines = useShellStatusLines();
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
  const automaticGrant = useAutomaticGrant();
  const snapshot = settings.data;
  return (
    <div className="tc-page">
      <SectionRule id="connection">{sectionTitle.connection}</SectionRule>
      <ConnectionPanel status={core.data} settings={settings.data} />
      <LegacyMigrationPanel
        status={core.data?.daemon.legacy_invite_migration}
      />
      <SectionRule id="startup">{sectionTitle.startup}</SectionRule>
      <PlatformPanel />
      <SectionRule id="watching">{sectionTitle.watching}</SectionRule>
      {settings.state === "loading" && (
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          Reading daemon settings…
        </p>
      )}
      {settings.state === "error" && (
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          {lines.readUnavailable}
        </p>
      )}
      {core.data && (
        <section className="tc-card">
          <div className="flex items-start justify-between gap-3">
            <div>
              <span className="mb-1.5 block tc-eyebrow">
                DAEMON
              </span>
              <h2>Contribution watcher</h2>
            </div>
            <span
              className={`tc-chip tc-chip--glass self-start ${core.data.daemon.paused ? "bg-tc-tint text-tc-secondary" : ""}`}
            >
              {core.data.daemon.paused ? "Paused" : "Watching"}
            </span>
          </div>
          <p className="m-0 tc-caption tc-text-tertiary">
            Pausing stops contribution processing. It does not delete queued
            sessions or change consent.
          </p>
          {daemon.error && (
            <p className="tc-alert">
              {daemon.error}
            </p>
          )}
          <div className="mt-3 flex flex-wrap gap-2">
            <GlassButton
              type="button"
              onClick={() => void daemon.command("pause_daemon")}
              disabled={daemon.busy || core.data.daemon.paused}
            >
              Pause watcher
            </GlassButton>
            <ButtonPrimary size="sm"
              type="button"
              onClick={() => void daemon.command("resume_daemon")}
              disabled={daemon.busy || !core.data.daemon.paused}
            >
              Resume watcher
            </ButtonPrimary>
          </div>
        </section>
      )}
      {settings.state === "ready" && (
        <>
          <section className="tc-card">
            <div className="flex items-start justify-between gap-3">
              <div>
                <span className="mb-1.5 block tc-eyebrow">
                  WATCHER
                </span>
                <h2>Session discovery</h2>
              </div>
              <TertiaryLink
                type="button"
                onClick={() => void settings.refresh()}
              >
                Refresh
              </TertiaryLink>
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
          <BehaviorSettingsPanel
            settings={snapshot ?? {}}
            busy={behavior.busy}
            error={behavior.error}
            onRefresh={settings.refresh}
            onSave={(setting, value) => behavior.save(setting, value)}
          />
          <SectionRule id="uses">{sectionTitle.uses}</SectionRule>
          <ConsentSettingsPanel
            options={consent.options}
            granted={core.data?.daemon.consent_scopes ?? []}
            state={consent.state}
            error={consent.error}
            onRefresh={consent.refresh}
            onToggle={consent.toggle}
          />
          <AutomaticGrantPanel
            grant={automaticGrant.grant}
            state={automaticGrant.state}
            error={automaticGrant.error}
            withdrawn={automaticGrant.withdrawn}
            onRefresh={automaticGrant.refresh}
            onWithdraw={automaticGrant.withdraw}
            onTurnOn={onTurnOnAutomaticContributing}
          />
          <SectionRule id="profile">{sectionTitle.profile}</SectionRule>
          {profile}
          <SectionRule id="folders">{sectionTitle.folders}</SectionRule>
          <SourceRootsPanel
            snapshot={snapshot ?? {}}
            busy={roots.busy}
            error={roots.error}
            onSave={(source, mode, path) => roots.save(source, mode, path)}
          />
          <SectionRule id="tools">{sectionTitle.tools}</SectionRule>
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
          <SectionRule id="pai">{sectionTitle.pai}</SectionRule>
          {privateAi}
          <SectionRule id="witness">{sectionTitle.witness}</SectionRule>
          <WitnessPanel
            data={witness.data}
            state={witness.state}
            error={witness.error}
            onRefresh={witness.refresh}
            onConfigure={witness.configure}
            onClear={witness.clear}
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
          <RouteDisclosurePanel />
          <SectionRule id="projects">{sectionTitle.projects}</SectionRule>
          <ProjectsPanel
            projects={projects.projects}
            state={projects.state}
            error={projects.error}
            onRefresh={projects.refresh}
            onSetMode={projects.setMode}
          />
          <SectionRule id="log">{sectionTitle.log}</SectionRule>
          <AuditPanel
            entries={audit.entries}
            state={audit.state}
            error={audit.error}
            onRefresh={audit.refresh}
          />
        </>
      )}
      {compute && (
        <>
          <SectionRule id="compute">{sectionTitle.compute}</SectionRule>
          {compute}
        </>
      )}
    </div>
  );
}
