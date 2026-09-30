import { type ReactNode, useEffect, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import {
  Breadcrumb,
  Pane,
  SegmentedTabs,
  Window,
} from "../../design-system";
import { HistoryPage } from "../../features/history";
import { MissionDraftsPage } from "../../features/mission-drafts";
import { PrivateAiPage } from "../../features/private-ai";
import type { usePublicProfile } from "../../features/profile/public";
import { WaitingPage } from "../../features/waiting";
import {
  TracesGraph,
  TracesTree,
  useTracesWorkspace,
} from "../../features/waiting/public";
import type { useCoreStatus } from "../../lib/tauri/use-core-status";
import {
  type MonitorTab,
  type MonitorView,
  monitorTabFromView,
  monitorViewFromPath,
  settingsSectionFromPath,
  viewPaths,
} from "../routes";
import { FlowMap, type MapView } from "./flow-map";
import { HomeView } from "./home-view";
import { InferenceInspector } from "./inference-inspector";
import { MonitorToolbar } from "./monitor-toolbar";
import { SettingsModal } from "./settings-modal";

function initialOpen(minWidth: number) {
  return typeof window === "undefined" || window.innerWidth >= minWidth;
}

const TAB_VIEW: Record<MonitorTab, MonitorView> = {
  home: "home",
  inference: "inference",
  traces: "traces",
};

/**
 * The Monitor window: three glass panes over the scene. Left: toolbar,
 * Home · Inference · Traces, and the view. Middle: the flow map. Right: the
 * inspector. Settings opens as a modal over all three.
 */
export function MonitorShell({
  core,
  publicProfile,
  notices,
}: {
  core: ReturnType<typeof useCoreStatus>;
  publicProfile: ReturnType<typeof usePublicProfile>;
  /** Core notices every view must show (startup, grant voids, rewordings). */
  notices: ReactNode;
}) {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const view = monitorViewFromPath(pathname);
  const [mapOpen, setMapOpen] = useState(() => initialOpen(1100));
  const [inspectorOpen, setInspectorOpen] = useState(() => initialOpen(900));
  const [graphOpen, setGraphOpen] = useState(true);
  const [viewMenu, setViewMenu] = useState(false);
  const [mapView, setMapView] = useState<MapView>(
    view === "inference" ? "ai" : "traces",
  );
  const [focus, setFocus] = useState(false);
  const [settings, setSettings] = useState<{ section: string | null } | null>(
    null,
  );

  // The tray and deep links open Settings (and the profile) by path.
  useEffect(() => {
    const section = settingsSectionFromPath(pathname);
    if (section) {
      setSettings({ section });
      navigate(viewPaths.home, { replace: true });
    }
  }, [pathname, navigate]);

  useEffect(() => {
    if (view === "inference") setMapView("ai");
    if (view === "traces") setMapView("traces");
  }, [view]);

  const go = (next: MonitorView) => navigate(viewPaths[next]);

  return (
    <Window className="h-screen" onClick={() => setViewMenu(false)}>
      <Pane
        className="flex min-w-[320px] flex-col overflow-hidden"
        style={
          mapOpen
            ? {
                width: "clamp(320px, 34vw, var(--tc-width-pane-left))",
                flex: "none",
              }
            : { flex: 1 }
        }
      >
        <MonitorToolbar
          viewMenu={viewMenu}
          onViewMenu={setViewMenu}
          graphOpen={graphOpen}
          onGraph={setGraphOpen}
          mapOpen={mapOpen}
          onMap={setMapOpen}
          inspectorOpen={inspectorOpen}
          onInspector={setInspectorOpen}
          onSettings={() => setSettings({ section: null })}
        />
        <MonitorTabs view={view} queueDepth={core.data?.daemon.queue_depth} onGo={go} />
        <div
          className={`min-h-0 flex-1 overflow-auto ${view === "traces" ? "px-2" : "px-3 pb-3"}`}
        >
          <div className="tc-page mb-2.5">{notices}</div>
          <LeftPaneView view={view} core={core} onGo={go} />
        </div>
        {view === "traces" ? (
          <GraphFooter
            open={graphOpen}
            focus={focus}
            onFocus={() => {
              setFocus(!focus);
              if (!mapOpen) setMapOpen(true);
            }}
          />
        ) : null}
      </Pane>
      {mapOpen ? (
        <div className="relative min-w-0 flex-1">
          <FocusedMap view={mapView} onViewChange={setMapView} focus={focus} />
        </div>
      ) : null}
      {inspectorOpen ? (
        <Pane
          className="flex-none overflow-auto px-4 py-4.5"
          style={{ width: "var(--tc-width-inspector)" }}
          aria-label="Inspector"
        >
          {view === "inference" ? (
            <InferenceInspector />
          ) : (
            <WaitingPage key={core.scope} status={core.data} />
          )}
        </Pane>
      ) : null}
      <SettingsModal
        open={settings !== null}
        section={settings?.section ?? null}
        onClose={() => setSettings(null)}
        core={core}
        publicProfile={publicProfile}
      />
    </Window>
  );
}

/** Home · Inference · Traces, and the breadcrumb for Home's sub-views. */
function MonitorTabs({
  view,
  queueDepth,
  onGo,
}: {
  view: MonitorView;
  queueDepth: number | undefined;
  onGo: (view: MonitorView) => void;
}) {
  const workspace = useTracesWorkspace();
  const privateAiOn = workspace.settings.data?.private_inference === true;
  const subView = view === "missions" || view === "history";
  return (
    <>
      <SegmentedTabs<MonitorTab>
        label="Monitor"
        className="mx-3 mt-0.5 mb-2"
        value={monitorTabFromView(view)}
        onChange={(next) => onGo(TAB_VIEW[next])}
        items={[
          { value: "home", label: "Home" },
          {
            value: "inference",
            label: "Inference",
            dot: privateAiOn
              ? "var(--tc-status-on)"
              : "var(--tc-status-outside)",
          },
          {
            value: "traces",
            label: "Traces",
            // Decisions owed: the sessions waiting for an answer.
            badge: queueDepth ?? workspace.entries.length,
          },
        ]}
      />
      {subView ? (
        <div className="px-3 pb-2">
          <Breadcrumb
            onBack={() => onGo("home")}
            trail={[
              { label: "Home", onSelect: () => onGo("home") },
              { label: view === "missions" ? "Missions" : "History" },
            ]}
          />
        </div>
      ) : null}
    </>
  );
}

function LeftPaneView({
  view,
  core,
  onGo,
}: {
  view: MonitorView;
  core: ReturnType<typeof useCoreStatus>;
  onGo: (view: MonitorView) => void;
}) {
  switch (view) {
    case "missions":
      return <MissionDraftsPage />;
    case "history":
      return <HistoryPage key={core.scope} />;
    case "inference":
      return <PrivateAiPage key={core.scope} />;
    case "traces":
      return <TracesTree />;
    default:
      return (
        <HomeView
          status={core.data}
          onOpenTraces={() => onGo("traces")}
          onOpenMissions={() => onGo("missions")}
          onOpenHistory={() => onGo("history")}
        />
      );
  }
}

/** The binoculars focus the map on the selected tool. */
function useMapFocus(focus: boolean) {
  const workspace = useTracesWorkspace();
  const tool = workspace.selected?.tool ?? null;
  const canFocus = Boolean(tool?.source);
  return {
    tool,
    canFocus,
    focusToolId: focus && canFocus && tool ? tool.id : null,
  };
}

function GraphFooter({
  open,
  focus,
  onFocus,
}: {
  open: boolean;
  focus: boolean;
  onFocus: () => void;
}) {
  const { tool, canFocus, focusToolId } = useMapFocus(focus);
  const tip = !canFocus
    ? "Select a tool, project or session first"
    : focusToolId
      ? "Back to the whole map"
      : `Show ${tool?.label} in the map`;
  return (
    <div
      className="flex-none overflow-hidden"
      style={{
        height: open ? 206 : 0,
        borderTop: "0.5px solid rgba(255,255,255,0.12)",
        transition: "height .25s ease",
      }}
    >
      <TracesGraph
        focusActive={focusToolId !== null}
        canFocus={canFocus}
        focusTip={tip}
        onToggleFocus={onFocus}
      />
    </div>
  );
}

function FocusedMap({
  view,
  onViewChange,
  focus,
}: {
  view: MapView;
  onViewChange: (view: MapView) => void;
  focus: boolean;
}) {
  const { focusToolId } = useMapFocus(focus);
  return (
    <FlowMap view={view} onViewChange={onViewChange} focusToolId={focusToolId} />
  );
}
