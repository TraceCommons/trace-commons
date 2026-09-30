import {
  Menu,
  MenuItem,
  RoundButton,
  ToolbarIconButton,
} from "../../design-system";
import { useTracesWorkspace } from "../../features/waiting/public";

const isMac =
  typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);

/**
 * The left pane's title row: view options, the graph / map / inspector
 * toggles, and the round Settings button. On macOS the window's own traffic
 * lights sit to its left (overlay title bar), so the row leaves them room
 * and drags the window.
 */
export function MonitorToolbar({
  viewMenu,
  onViewMenu,
  graphOpen,
  onGraph,
  mapOpen,
  onMap,
  inspectorOpen,
  onInspector,
  onSettings,
}: {
  viewMenu: boolean;
  onViewMenu: (open: boolean) => void;
  graphOpen: boolean;
  onGraph: (open: boolean) => void;
  mapOpen: boolean;
  onMap: (open: boolean) => void;
  inspectorOpen: boolean;
  onInspector: (open: boolean) => void;
  onSettings: () => void;
}) {
  const workspace = useTracesWorkspace();
  return (
    <>
      <div
        data-tauri-drag-region
        className="flex items-center gap-2 pt-3.5 pr-3 pb-2"
        style={{ paddingLeft: isMac ? 78 : 16 }}
      >
        <span
          className="ml-auto flex gap-0.5 rounded-full p-0.5"
          style={{
            background: "var(--tc-control-fill)",
            boxShadow: "var(--tc-control-edge)",
            color: "#e6e6ec",
          }}
        >
          <ToolbarIconButton
            label="View options"
            aria-haspopup="menu"
            aria-expanded={viewMenu}
            onClick={(event) => {
              event.stopPropagation();
              onViewMenu(!viewMenu);
            }}
          >
            <svg width="14" height="12" viewBox="0 0 14 12" fill="none" stroke="currentColor" strokeWidth="1.4" aria-hidden="true">
              <path d="M1 2h12M1 6h12M1 10h12" />
            </svg>
          </ToolbarIconButton>
          <ToolbarIconButton
            label={graphOpen ? "Hide the graph" : "Show the graph"}
            pressed={graphOpen}
            onClick={() => onGraph(!graphOpen)}
          >
            <svg width="14" height="12" viewBox="0 0 14 12" fill="none" stroke="currentColor" strokeWidth="1.3" aria-hidden="true">
              <rect x="0.5" y="0.5" width="13" height="11" rx="2" />
              <path d="M0.5 7.5h13M4 7.5v4M8 7.5v4" />
            </svg>
          </ToolbarIconButton>
          <ToolbarIconButton
            label={mapOpen ? "Hide the flow map" : "Show the flow map"}
            pressed={mapOpen}
            onClick={() => onMap(!mapOpen)}
          >
            <svg width="14" height="12" viewBox="0 0 14 12" fill="none" stroke="currentColor" strokeWidth="1.3" aria-hidden="true">
              <path d="M0.5 2.5l4-2 5 2 4-2v9l-4 2-5-2-4 2z M4.5 0.5v9M9.5 2.5v9" />
            </svg>
          </ToolbarIconButton>
          <ToolbarIconButton
            label={inspectorOpen ? "Hide the inspector" : "Show the inspector"}
            pressed={inspectorOpen}
            onClick={() => onInspector(!inspectorOpen)}
          >
            <svg width="14" height="12" viewBox="0 0 14 12" fill="none" stroke="currentColor" strokeWidth="1.3" aria-hidden="true">
              <rect x="0.5" y="0.5" width="13" height="11" rx="2" />
              <path d="M9 0.5v11" />
            </svg>
          </ToolbarIconButton>
        </span>
        <RoundButton label="Settings" className="ml-1.5" onClick={onSettings}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" aria-hidden="true">
            <circle cx="12" cy="12" r="3" />
            <path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1L7 17M17 7l2.1-2.1" />
          </svg>
        </RoundButton>
      </div>
      {viewMenu ? (
        <Menu className="absolute top-10 right-3 z-10 w-[250px]">
          <MenuItem
            checked={workspace.showIgnored}
            onSelect={() => {
              workspace.setShowIgnored(!workspace.showIgnored);
              onViewMenu(false);
            }}
          >
            Show ignored folders
          </MenuItem>
        </Menu>
      ) : null}
    </>
  );
}
