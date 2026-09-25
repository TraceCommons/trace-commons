import type { WatchAnswer } from "../ftux-model";
import type { DetectedTool } from "../types";
import {
  DownloadIcon,
  FolderIcon,
  PillSelect,
  type SelectOption,
} from "./glass";

const WATCH_OPTIONS: SelectOption<Exclude<WatchAnswer, "unanswered">>[] = [
  { value: "watch", label: "Watch this folder", tone: "green" },
  { value: "ignore", label: "I don’t use it", tone: "grey" },
];

export function ToolRow({
  tool,
  answer,
  folder,
  compactMeta,
  onAnswer,
  onChooseFolder,
  onInstall,
}: {
  tool: DetectedTool;
  answer: WatchAnswer;
  folder: string;
  // The Tools screen shows only the session count; Folders adds recency.
  compactMeta?: boolean;
  onAnswer: (answer: WatchAnswer) => void;
  onChooseFolder: () => void;
  onInstall: (url: string) => void;
}) {
  const found = tool.presence === "found";
  const meta =
    found && compactMeta && !tool.custom
      ? `${tool.sessionCount} sessions`
      : tool.detail;
  return (
    <div className="ftux-card ftux-card-tight">
      <div className="ftux-tool-head">
        <span className="ftux-badge" aria-hidden="true">
          {tool.badge}
        </span>
        <span className="ftux-tool-name">
          <span className="ftux-card-title">{tool.name}</span>
          <span className="ftux-tool-path" title={folder}>
            {folder}
          </span>
        </span>
        <span className="ftux-tool-meta">{meta}</span>
      </div>
      <div className="ftux-tool-actions">
        {found ? (
          <>
            <PillSelect
              label={`${tool.name}: watch this folder?`}
              value={answer === "unanswered" ? null : answer}
              options={WATCH_OPTIONS}
              onChange={onAnswer}
            />
            <button
              type="button"
              className="ftux-btn ftux-btn-small"
              title="Choose a different folder…"
              aria-label={`Choose a different folder for ${tool.name}`}
              onClick={onChooseFolder}
            >
              <FolderIcon />…
            </button>
          </>
        ) : (
          <>
            <span className="ftux-tool-hint">
              Install it, then this row asks again.
            </span>
            {tool.installUrl ? (
              <button
                type="button"
                className="ftux-btn ftux-btn-install"
                title={`Download ${tool.name}`}
                onClick={() => tool.installUrl && onInstall(tool.installUrl)}
              >
                <DownloadIcon />
                Get {tool.name}
              </button>
            ) : null}
          </>
        )}
      </div>
    </div>
  );
}
