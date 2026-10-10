import type { ReactNode } from "react";
import { Kebab, SubmitPill, WatchSwitch } from "./controls";
import { ToolTile } from "./indicators";
import type { ToolLogoId } from "./logo-paths";

export type ListRowProps = {
  /** 0 tool · 1 folder · 2 session. Indent is +18px per level. */
  depth: 0 | 1 | 2;
  logo?: ToolLogoId | null;
  title: string;
  sub?: ReactNode;
  /** Sub-line colour: amber when flagged, green when contributed. */
  flag?: "ask" | "on" | null;
  selected?: boolean;
  off?: boolean;
  expanded?: boolean;
  onToggleExpand?: () => void;
  onSelect?: () => void;
  submitLabel?: string;
  submitTip?: string;
  submitDone?: boolean;
  onSubmit?: (() => void) | null;
  /** Watch switch: null hides it (a session has no switch of its own). */
  watched?: boolean | null;
  watchLabel?: string;
  onToggleWatch?: (watched: boolean) => void;
  watchDisabled?: boolean;
  menuOpen?: boolean;
  onOpenMenu?: (() => void) | null;
};

/**
 * Tree row · tool › folder › session. Grid 16 24 1fr auto 38 22, gap 8,
 * indent +18 per level. Title 13/600 (500 at session), sub 11 tertiary.
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: one row, three depths; each optional control is a presence check.
export function ListRow({
  depth,
  logo,
  title,
  sub,
  flag,
  selected = false,
  off = false,
  expanded = false,
  onToggleExpand,
  onSelect,
  submitLabel,
  submitTip,
  submitDone = false,
  onSubmit,
  watched = null,
  watchLabel = "Watch this folder",
  onToggleWatch,
  watchDisabled = false,
  menuOpen = false,
  onOpenMenu,
}: ListRowProps) {
  const expandable = depth < 2 && Boolean(onToggleExpand);
  return (
    <div
      role="treeitem"
      aria-level={depth + 1}
      aria-selected={selected}
      aria-expanded={expandable ? expanded : undefined}
      tabIndex={0}
      className="tc-list-row"
      data-depth={depth}
      data-selected={selected}
      data-off={off}
      style={{ "--tc-row-indent": `${8 + depth * 18}px` } as React.CSSProperties}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (event.target !== event.currentTarget) return;
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onSelect?.();
        }
        if (expandable && event.key === "ArrowRight" && !expanded)
          onToggleExpand?.();
        if (expandable && event.key === "ArrowLeft" && expanded)
          onToggleExpand?.();
      }}
    >
      {expandable ? (
        <button
          type="button"
          className="tc-chevron border-0 bg-transparent p-0"
          data-open={expanded}
          aria-label={expanded ? `Collapse ${title}` : `Expand ${title}`}
          style={{ color: selected ? "#fff" : undefined }}
          onClick={(event) => {
            event.stopPropagation();
            onToggleExpand?.();
          }}
        >
          ›
        </button>
      ) : (
        <span />
      )}
      <ToolTile
        tool={depth === 0 ? logo : null}
        kind={depth === 0 ? "tool" : depth === 1 ? "folder" : "session"}
        fallback={depth === 0 ? title.slice(0, 2) : undefined}
      />
      <span className="tc-list-row__text">
        <span className="tc-list-row__title">{title}</span>
        {sub ? (
          <span className="tc-list-row__sub" data-flag={flag ?? undefined}>
            {sub}
          </span>
        ) : null}
      </span>
      {submitLabel ? (
        <SubmitPill
          title={submitTip}
          done={submitDone}
          disabled={!onSubmit}
          onClick={(event) => {
            event.stopPropagation();
            onSubmit?.();
          }}
        >
          {submitLabel}
        </SubmitPill>
      ) : (
        <span />
      )}
      {watched === null ? (
        <span />
      ) : (
        <WatchSwitch
          checked={watched}
          label={watchLabel}
          disabled={watchDisabled || !onToggleWatch}
          onChange={(next) => onToggleWatch?.(next)}
        />
      )}
      <span className="inline-flex justify-center">
        {onOpenMenu ? (
          <Kebab
            open={menuOpen}
            onClick={(event) => {
              event.stopPropagation();
              onOpenMenu();
            }}
          />
        ) : null}
      </span>
    </div>
  );
}
