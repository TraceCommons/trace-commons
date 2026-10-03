import {
  ButtonSecondary,
  Card,
  FolderButton,
  Picker,
  type PickerOption,
  ToolTile,
} from "../../../design-system";
import type { WatchAnswer } from "../ftux-model";
import type { DetectedTool } from "../types";
import { DownloadIcon } from "./icons";

const WATCH_OPTIONS: PickerOption<Exclude<WatchAnswer, "unanswered">>[] = [
  { value: "watch", label: "Watch this folder", dot: "var(--tc-status-on)" },
  { value: "ignore", label: "I don’t use it", dot: "var(--tc-status-off)" },
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
    <Card className="tc-stack tc-stack--tight">
      <div className="ftux-row">
        <ToolTile
          tool={tool.logo ?? null}
          kind={tool.logo ? "tool" : "folder"}
          large
        />
        <span className="ftux-grow ftux-min-0 tc-stack ftux-gap-0">
          <span className="tc-body-strong">{tool.name}</span>
          <span
            className="tc-mono tc-text-tertiary ftux-ellipsis"
            title={folder}
          >
            {folder}
          </span>
        </span>
        <span className="tc-caption tc-text-tertiary ftux-nowrap">{meta}</span>
      </div>
      <div className="ftux-row ftux-row--end">
        {found ? (
          <>
            <Picker
              label={`${tool.name}: watch this folder?`}
              value={answer === "unanswered" ? null : answer}
              options={WATCH_OPTIONS}
              onChange={onAnswer}
            />
            <FolderButton
              label={`Choose a different folder for ${tool.name}`}
              onClick={onChooseFolder}
            />
          </>
        ) : (
          <>
            <span className="ftux-grow tc-caption tc-text-tertiary">
              Install it, then this row asks again.
            </span>
            {tool.installUrl ? (
              <ButtonSecondary
                size="sm"
                title={`Download ${tool.name}`}
                onClick={() => tool.installUrl && onInstall(tool.installUrl)}
              >
                <DownloadIcon />
                Get {tool.name}
              </ButtonSecondary>
            ) : null}
          </>
        )}
      </div>
    </Card>
  );
}
