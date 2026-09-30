import { type ReactNode, useMemo, useState } from "react";
import {
  isToolLogoId,
  MapArc,
  MapNode,
  NodeCard,
  SegmentedTabs,
  ToolLogoSvg,
} from "../../design-system";

/** Harness ids and source names that have a logo. */
const LOGO_ALIASES: Record<string, string> = {
  "claude-code": "claude",
  gemini: "anti",
  "gemini-cli": "anti",
};

function logoFor(id: string) {
  const key = LOGO_ALIASES[id] ?? id;
  return isToolLogoId(key) ? key : null;
}
import {
  type HarnessRow,
  useHarnesses,
  usePrivateAi,
} from "../../features/private-ai/public";
import {
  type FolderNode,
  MODE_LABEL,
  plural,
  type ToolNode,
  useTracesWorkspace,
} from "../../features/waiting/public";

export type MapView = "traces" | "ai";

const MAC: [number, number] = [290, 400];
const LIBRARY: [number, number] = [470, 150];
const CREDENTIAL: [number, number] = [290, 380];

type Card = { title: string; body: string; at: [number, number] };

const RULE_STYLE: Record<string, { fill: string; stroke: string }> = {
  auto_upload: { fill: "#12321f", stroke: "var(--tc-status-on)" },
  notify_only: { fill: "#2b2412", stroke: "var(--tc-status-ask)" },
  ignore: { fill: "#3a3a3e", stroke: "var(--tc-status-outside)" },
  unknown: { fill: "#2a2a2c", stroke: "var(--tc-status-off)" },
};

function toolPositions(tools: ToolNode[]): Map<string, [number, number]> {
  const positions = new Map<string, [number, number]>();
  const top = 170;
  const bottom = 630;
  const step = tools.length > 1 ? (bottom - top) / (tools.length - 1) : 0;
  tools.forEach((tool, index) => {
    positions.set(tool.id, [150, tools.length > 1 ? top + index * step : MAC[1]]);
  });
  return positions;
}

/**
 * The flow map. Traces: tools and their folders (ringed by contribution
 * rule), with an arc to the commons library for tools that have sent
 * anything or have a folder set to contribute automatically. Private AI:
 * each configured tool and whether it answers through the NEAR AI
 * credential. Only what the core reports is drawn.
 */
export function FlowMap({
  view,
  onViewChange,
  focusToolId,
}: {
  view: MapView;
  onViewChange: (view: MapView) => void;
  focusToolId: string | null;
}) {
  const workspace = useTracesWorkspace();
  const privateAiOn = workspace.settings.data?.private_inference === true;
  const [zoom, setZoom] = useState(1);
  const [hover, setHover] = useState<Card | null>(null);
  const [pinned, setPinned] = useState<Card | null>(null);
  const card = hover ?? pinned;

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: clicking the empty map unpins a node card; nodes are the focusable controls.
    <div
      className="tc-map absolute inset-0"
      onClick={() => setPinned(null)}
      role="presentation"
    >
      {view === "traces" ? (
        <TracesMap
          zoom={zoom}
          focusToolId={focusToolId}
          onHover={setHover}
          onPin={setPinned}
        />
      ) : (
        <PrivateAiMap on={privateAiOn} onHover={setHover} onPin={setPinned} />
      )}
      <SegmentedTabs
        floating
        label="Map view"
        className="absolute top-3.5 right-3.5"
        value={view}
        onChange={onViewChange}
        items={[
          { value: "traces", label: "Traces" },
          {
            value: "ai",
            label: "Private AI",
            dot: privateAiOn
              ? "var(--tc-status-on)"
              : "var(--tc-status-outside)",
          },
        ]}
      />
      {card ? (
        <div
          className="pointer-events-none absolute"
          style={{
            left: Math.min(card.at[0], 278),
            top: Math.min(card.at[1], 600),
          }}
        >
          <NodeCard
            title={card.title}
            body={card.body}
            hint={pinned && !hover ? undefined : "hover to peek · click to pin"}
          />
        </div>
      ) : null}
      {view === "traces" ? (
        <fieldset
          className="tc-segmented tc-segmented--floating absolute right-3.5 bottom-3.5 m-0 border-0"
          aria-label="Map zoom"
        >
          <button
            type="button"
            className="tc-segmented__item"
            aria-label="Zoom map out"
            onClick={(event) => {
              event.stopPropagation();
              setZoom(Math.max(0.6, zoom - 0.25));
            }}
          >
            −
          </button>
          <button
            type="button"
            className="tc-segmented__item"
            aria-label="Zoom map in"
            onClick={(event) => {
              event.stopPropagation();
              setZoom(Math.min(2, zoom + 0.25));
            }}
          >
            +
          </button>
        </fieldset>
      ) : null}
    </div>
  );
}

function MapSvg({ children, camera }: { children: ReactNode; camera?: string }) {
  return (
    <svg
      viewBox="0 0 580 760"
      preserveAspectRatio="xMidYMid meet"
      className="absolute inset-0 h-full w-full"
      role="img"
      aria-label="Flow map"
    >
      <g
        style={{
          transform: camera,
          transition: "transform .7s cubic-bezier(.4,0,.2,1)",
        }}
      >
        {children}
      </g>
    </svg>
  );
}

function cameraFor(
  focus: [number, number] | null,
  zoom: number,
): string | undefined {
  if (focus) {
    const scale = 1.6;
    return `translate(${(290 - scale * (focus[0] - 50)).toFixed(1)}px,${(380 - scale * focus[1]).toFixed(1)}px) scale(${scale})`;
  }
  if (zoom === 1) return undefined;
  return `translate(${(290 - zoom * 290).toFixed(1)}px,${(380 - zoom * 380).toFixed(1)}px) scale(${zoom})`;
}

function folderPosition(
  tool: [number, number],
  index: number,
  count: number,
): [number, number] {
  return [58, tool[1] + (index - (count - 1) / 2) * 46];
}

function TracesMap({
  zoom,
  focusToolId,
  onHover,
  onPin,
}: {
  zoom: number;
  focusToolId: string | null;
  onHover: (card: Card | null) => void;
  onPin: (card: Card | null) => void;
}) {
  const workspace = useTracesWorkspace();
  const { tree } = workspace;
  const positions = useMemo(() => toolPositions(tree), [tree]);
  const selectedToolId = workspace.selected?.tool.id ?? null;
  const pulsing = workspace.undo.scope !== null || workspace.bulk.busyId !== null;
  const waiting = tree.reduce((n, tool) => n + tool.waiting, 0);
  const contributed = tree.reduce((n, tool) => n + tool.contributed, 0);
  const macCard: Card = {
    title: "This Mac",
    body: `Sessions are recorded and scrubbed here. ${plural(waiting, "session")} waiting for you; ${contributed} contributed. Nothing is sent unless you say so.`,
    at: [MAC[0] + 20, MAC[1] - 110],
  };
  const libraryCard: Card = {
    title: "Library · commons",
    body: `${plural(contributed, "trace")} contributed from this machine. Folders set to contribute automatically send scrubbed sessions here; every other folder waits for you.`,
    at: [LIBRARY[0] - 250, LIBRARY[1] + 20],
  };
  const hoverable = (card: Card) => ({
    onHover: (on: boolean) => onHover(on ? card : null),
    onSelect: () => onPin(card),
  });

  return (
    <MapSvg
      camera={cameraFor(
        focusToolId ? (positions.get(focusToolId) ?? null) : null,
        zoom,
      )}
    >
      {tree.map((tool) => {
        const at = positions.get(tool.id);
        if (!at) return null;
        const sends =
          tool.contributed > 0 ||
          tool.folders.some((folder) => folder.mode === "auto_upload");
        if (!sends) return null;
        const automatic = tool.folders.some((f) => f.mode === "auto_upload");
        const dim = selectedToolId && selectedToolId !== tool.id ? 0.12 : 1;
        return (
          <MapArc
            key={`lib-${tool.id}`}
            from={[at[0] + 12, at[1] - 6]}
            to={LIBRARY}
            tone={automatic ? "on" : undefined}
            pulse={pulsing && automatic}
            dim={tool.mode === "off" ? 0.3 * dim : dim}
          />
        );
      })}
      <MapNode
        x={MAC[0]}
        y={MAC[1]}
        r={16}
        fill="#8e8e96"
        ring
        label="This Mac"
        {...hoverable(macCard)}
      />
      {tree.map((tool) => {
        const at = positions.get(tool.id);
        if (!at) return null;
        return (
          <ToolGroup
            key={tool.id}
            tool={tool}
            at={at}
            dim={selectedToolId && selectedToolId !== tool.id ? 0.3 : 1}
            onHover={onHover}
            onPin={onPin}
          />
        );
      })}
      <MapNode
        x={LIBRARY[0]}
        y={LIBRARY[1]}
        fill={contributed > 0 ? "var(--tc-blue)" : "var(--tc-map-node-off)"}
        ring={contributed > 0}
        label="Library · commons"
        labelBelow={false}
        {...hoverable(libraryCard)}
      />
      <MapLegend />
    </MapSvg>
  );
}

function ToolGroup({
  tool,
  at,
  dim,
  onHover,
  onPin,
}: {
  tool: ToolNode;
  at: [number, number];
  dim: number;
  onHover: (card: Card | null) => void;
  onPin: (card: Card | null) => void;
}) {
  const workspace = useTracesWorkspace();
  const shown = tool.folders.slice(0, 4);
  const off = tool.mode === "off";
  const toolCard: Card = {
    title: `${tool.label} · ${plural(tool.folders.length, "folder")}`,
    body:
      tool.mode === "watch"
        ? `Watched: new sessions are recorded and scrubbed on this Mac. ${tool.waiting ? `${tool.waiting} waiting for you.` : "Nothing waiting."}`
        : tool.mode === "off"
          ? "Not watched: nothing new is read from this tool."
          : "No sessions folder set for this tool yet.",
    at: [at[0] + 24, at[1] - 30],
  };
  return (
    <g opacity={off ? 0.4 * dim : dim} style={{ transition: "opacity .3s" }}>
      <path
        d={`M${at[0]} ${at[1]} C ${(MAC[0] + at[0]) / 2} ${at[1]}, ${(MAC[0] + at[0]) / 2} ${MAC[1]}, ${MAC[0] - 16} ${MAC[1]}`}
        fill="none"
        stroke="rgba(255,255,255,0.18)"
        strokeWidth={1.2}
      />
      {shown.map((folder, index) => (
        <FolderDot
          key={folder.id}
          folder={folder}
          tool={at}
          at={folderPosition(at, index, shown.length)}
          onHover={onHover}
          onPin={(card) => {
            onPin(card);
            workspace.reveal([tool.id]);
            workspace.select({ kind: "folder", id: folder.id });
          }}
        />
      ))}
      {tool.folders.length > shown.length ? (
        <text className="tc-map__sublabel" x={58} y={at[1] + 118} textAnchor="middle">
          +{tool.folders.length - shown.length} more
        </text>
      ) : null}
      <MapNode
        x={at[0]}
        y={at[1]}
        r={13}
        fill="rgba(255,255,255,0.12)"
        ring={workspace.selected?.tool.id === tool.id}
        label={tool.label}
        onHover={(on) => onHover(on ? toolCard : null)}
        onSelect={() => {
          onPin(toolCard);
          workspace.select({ kind: "tool", id: tool.id });
        }}
      >
        {tool.logo ? (
          <ToolLogoSvg id={tool.logo} x={at[0] - 8} y={at[1] - 8} />
        ) : (
          <text
            x={at[0]}
            y={at[1] + 3.5}
            textAnchor="middle"
            fontSize={9}
            fontWeight={700}
            fill="#f2f2f4"
          >
            {tool.label.slice(0, 2)}
          </text>
        )}
        {off ? (
          <path
            d={`M${at[0] - 9} ${at[1] - 9}L${at[0] + 9} ${at[1] + 9}`}
            stroke="#f2f2f4"
            strokeWidth={2}
          />
        ) : null}
      </MapNode>
    </g>
  );
}

function FolderDot({
  folder,
  tool,
  at,
  onHover,
  onPin,
}: {
  folder: FolderNode;
  tool: [number, number];
  at: [number, number];
  onHover: (card: Card | null) => void;
  onPin: (card: Card) => void;
}) {
  const style = RULE_STYLE[folder.mode ?? "unknown"];
  const label =
    folder.label.length > 18 ? `${folder.label.slice(0, 17)}…` : folder.label;
  const card: Card = {
    title: folder.label,
    body: `${folder.path ? `${folder.path}. ` : ""}Rule: ${folder.mode ? MODE_LABEL[folder.mode] : "not set"}. ${plural(folder.entries.length, "session")} waiting, ${folder.contributed} contributed.`,
    at: [at[0] + 20, at[1] - 30],
  };
  return (
    // biome-ignore lint/a11y/useSemanticElements: an SVG group has no button element; it takes role="button" and a tab stop.
    <g
      className="tc-map__node"
      role="button"
      tabIndex={0}
      aria-label={folder.label}
      onMouseEnter={() => onHover(card)}
      onMouseLeave={() => onHover(null)}
      onClick={(event) => {
        event.stopPropagation();
        onPin(card);
      }}
      onKeyDown={(event) => {
        if (event.key === "Enter") onPin(card);
      }}
    >
      <path
        d={`M${tool[0] - 13} ${tool[1]} C ${tool[0] - 42} ${tool[1]}, ${tool[0] - 42} ${at[1]}, ${at[0] + 8} ${at[1]}`}
        fill="none"
        stroke="rgba(255,255,255,0.22)"
        strokeWidth={1.2}
      />
      <circle
        cx={at[0]}
        cy={at[1]}
        r={7}
        fill={style.fill}
        stroke={style.stroke}
        strokeWidth={2}
      />
      <text
        x={at[0]}
        y={at[1] + 18}
        textAnchor="middle"
        fontSize={9}
        fontWeight={600}
        fill="#d6d6dc"
      >
        {label}
      </text>
    </g>
  );
}

function MapLegend() {
  const items = [
    { x: 24, style: RULE_STYLE.auto_upload, label: "Automatically" },
    { x: 108, style: RULE_STYLE.notify_only, label: "Ask me" },
    { x: 164, style: RULE_STYLE.ignore, label: "Never" },
  ];
  return (
    <g fontSize={9} fill="var(--tc-text-tertiary)">
      {items.map((item) => (
        <g key={item.label}>
          <circle
            cx={item.x}
            cy={730}
            r={5}
            fill={item.style.fill}
            stroke={item.style.stroke}
            strokeWidth={1.5}
          />
          <text x={item.x + 10} y={733}>
            {item.label}
          </text>
        </g>
      ))}
      <circle
        cx={216}
        cy={730}
        r={5}
        fill="none"
        stroke="var(--tc-status-off)"
        strokeWidth={1.5}
        strokeDasharray="2 2"
      />
      <text x={226} y={733}>
        Tool not watched
      </text>
    </g>
  );
}

function PrivateAiMap({
  on,
  onHover,
  onPin,
}: {
  on: boolean;
  onHover: (card: Card | null) => void;
  onPin: (card: Card | null) => void;
}) {
  const harnesses = useHarnesses();
  const privateAi = usePrivateAi();
  const rows: HarnessRow[] = harnesses.data?.harnesses ?? [];
  const connected = rows.filter((row) => row.connected);
  const step = rows.length > 1 ? 420 / (rows.length - 1) : 0;
  const credentialLine =
    privateAi.credential?.view?.state_line ??
    privateAi.credential?.state ??
    "Credential state unavailable";
  const credentialCard: Card = {
    title: "NEAR AI credential",
    body: `${credentialLine}. ${plural(connected.length, "tool")} point at it. Private AI is ${on ? "on" : "off"}.`,
    at: [CREDENTIAL[0] - 130, CREDENTIAL[1] + 60],
  };
  return (
    <MapSvg>
      <g fill="none" strokeLinecap="round">
        {rows.map((row, index) => {
          const y = rows.length > 1 ? 170 + index * step : CREDENTIAL[1];
          return (
            <path
              key={row.id}
              d={`M118 ${y} C 200 ${y}, 210 ${CREDENTIAL[1]}, ${CREDENTIAL[0]} ${CREDENTIAL[1]}`}
              stroke={
                on && row.connected
                  ? "var(--tc-status-on)"
                  : "rgba(255,255,255,0.18)"
              }
              strokeWidth={2}
              strokeDasharray={row.connected ? undefined : "5 5"}
            />
          );
        })}
      </g>
      {rows.map((row, index) => {
        const y = rows.length > 1 ? 170 + index * step : CREDENTIAL[1];
        const card: Card = {
          title: row.name,
          body: row.state_line,
          at: [130, y - 70],
        };
        return (
          <MapNode
            key={row.id}
            x={106}
            y={y}
            r={13}
            fill="rgba(255,255,255,0.12)"
            label={row.name}
            dim={row.installed ? 1 : 0.5}
            onHover={(hovering) => onHover(hovering ? card : null)}
            onSelect={() => onPin(card)}
          >
            {logoFor(row.id) ? (
              <ToolLogoSvg id={logoFor(row.id) ?? "claude"} x={98} y={y - 8} />
            ) : null}
          </MapNode>
        );
      })}
      <MapNode
        x={CREDENTIAL[0]}
        y={CREDENTIAL[1]}
        r={26}
        fill={on ? "#2c7a5b" : "var(--tc-map-node-off)"}
        ring={on}
        label="NEAR AI credential"
        sublabel={on ? `On · ${plural(connected.length, "tool")}` : "Off · tools answer at their vendors"}
        onHover={(hovering) => onHover(hovering ? credentialCard : null)}
        onSelect={() => onPin(credentialCard)}
      >
        <path
          d={`M${CREDENTIAL[0]} ${CREDENTIAL[1] - 14}l10 4v7c0 7-5 12-10 14-5-2-10-7-10-14v-7z M${CREDENTIAL[0] - 5} ${CREDENTIAL[1]}l3 3 7-7`}
          fill="none"
          stroke="#f2f2f4"
          strokeWidth={2}
          strokeLinejoin="round"
          strokeLinecap="round"
          pointerEvents="none"
        />
      </MapNode>
      {rows.length === 0 ? (
        <text
          className="tc-map__sublabel"
          x={290}
          y={470}
          textAnchor="middle"
        >
          {harnesses.data?.view.none_found ?? "No configured tools found."}
        </text>
      ) : null}
    </MapSvg>
  );
}
