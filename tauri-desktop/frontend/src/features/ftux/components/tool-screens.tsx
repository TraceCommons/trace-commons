import { useState } from "react";
import type { WatchAnswer } from "../ftux-model";
import type { DetectedTool } from "../types";
import { ScreenTitle, Spinner } from "./glass";
import { ToolRow } from "./tool-row";

type ToolListProps = {
  tools: DetectedTool[] | null;
  answers: Record<string, WatchAnswer>;
  customFolders: Record<string, string>;
  canContinue: boolean;
  onAnswer: (toolId: string, answer: WatchAnswer) => void;
  onChooseFolder: (toolId: string) => void;
  onInstall: (url: string) => void;
  onContinue: () => void;
};

function ToolList({
  tools,
  answers,
  customFolders,
  compactMeta,
  onAnswer,
  onChooseFolder,
  onInstall,
}: ToolListProps & { compactMeta?: boolean }) {
  if (tools === null) {
    return (
      <p className="ftux-status" role="status">
        <Spinner /> Looking for coding tools on this Mac…
      </p>
    );
  }
  return (
    <>
      {tools.map((tool) => (
        <ToolRow
          key={tool.id}
          tool={tool}
          compactMeta={compactMeta}
          answer={answers[tool.id] ?? "unanswered"}
          folder={customFolders[tool.id] ?? tool.folder}
          onAnswer={(answer) => onAnswer(tool.id, answer)}
          onChooseFolder={() => onChooseFolder(tool.id)}
          onInstall={onInstall}
        />
      ))}
    </>
  );
}

function ContinueButton({
  enabled,
  onContinue,
}: {
  enabled: boolean;
  onContinue: () => void;
}) {
  return (
    <button
      type="button"
      className="ftux-btn ftux-btn-primary"
      disabled={!enabled}
      title={enabled ? undefined : "Answer every tool above to continue"}
      onClick={onContinue}
    >
      Continue
    </button>
  );
}

// Connect and forget: W-2.
export function FoldersScreen(
  props: ToolListProps & { onCustomize: () => void },
) {
  return (
    <>
      <ScreenTitle light="Which folders may this " bold="app watch?" />
      <p className="ftux-lede">
        We've found the following tools on your device. Traces work by reading
        coding-session transcripts from locations you specify. Select an option
        from each of the tools below to continue.
      </p>
      <div className="ftux-scroll">
        <ToolList {...props} />
      </div>
      <div className="ftux-footer ftux-footer-split">
        <button type="button" className="ftux-link" onClick={props.onCustomize}>
          Customize instead
        </button>
        <ContinueButton
          enabled={props.canContinue}
          onContinue={props.onContinue}
        />
      </div>
    </>
  );
}

// Customize and tailor: W-4.
export function ToolsScreen(
  props: ToolListProps & { onAddTool: (droppedName?: string) => void },
) {
  const [dragging, setDragging] = useState(false);
  return (
    <>
      <ScreenTitle light="Connect your " bold="tools and folders." />
      <div className="ftux-scroll">
        <ToolList {...props} compactMeta />
        {props.tools ? (
          <button
            type="button"
            className="ftux-add-tool"
            data-dragging={dragging}
            onClick={() => props.onAddTool()}
            onDragOver={(event) => {
              event.preventDefault();
              setDragging(true);
            }}
            onDragLeave={() => setDragging(false)}
            onDrop={(event) => {
              event.preventDefault();
              setDragging(false);
              const name = event.dataTransfer.files[0]?.name;
              props.onAddTool(name || undefined);
            }}
          >
            <span className="ftux-badge" aria-hidden="true">
              +
            </span>
            <span style={{ display: "flex", flexDirection: "column" }}>
              <span className="ftux-card-title">
                Not seeing your tool above? Click to add or drag &amp; drop.
              </span>
              <span className="ftux-card-text ftux-muted">
                OpenCode, Theia IDE, Cursor, or a dev server over SSH
              </span>
            </span>
          </button>
        ) : null}
      </div>
      <div className="ftux-footer">
        <ContinueButton
          enabled={props.canContinue}
          onContinue={props.onContinue}
        />
      </div>
    </>
  );
}
