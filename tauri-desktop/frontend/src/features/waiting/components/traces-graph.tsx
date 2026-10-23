import { useMemo, useState } from "react";
import {
  type BarBucket,
  BarGraph,
  LegendCell,
  PillIconButton,
} from "../../../design-system";
import {
  buildGraphBuckets,
  formatDay,
  type GraphRange,
  isContributed,
  rangeLabel,
  toolIdForSource,
} from "../traces-model";
import { useTracesWorkspace } from "../traces-workspace";

/**
 * Shared over kept, per period. Shared counts sessions that left this
 * machine (history, by submission time); kept counts sessions still here
 * (the queue, by discovery time). Counts, not bytes: the history record does
 * not carry sizes.
 */
export function TracesGraph({
  focusActive,
  canFocus,
  focusTip,
  onToggleFocus,
}: {
  focusActive: boolean;
  canFocus: boolean;
  focusTip: string;
  onToggleFocus: () => void;
}) {
  const workspace = useTracesWorkspace();
  const [range, setRange] = useState<GraphRange>(1);
  const [offset, setOffset] = useState(0);
  const [hovered, setHovered] = useState<number | null>(null);
  const [slide, setSlide] = useState<"l" | "r" | null>(null);
  const toolFilter =
    workspace.selected?.tool.source ? workspace.selected.tool.id : null;

  const records = workspace.history.data?.history ?? [];
  const sharedTimes = useMemo(
    () =>
      records
        .filter(isContributed)
        .filter(
          (record) => !toolFilter || toolIdForSource(record.source) === toolFilter,
        )
        .map((record) => Date.parse(record.submitted_at))
        .filter(Number.isFinite),
    [records, toolFilter],
  );
  const keptTimes = useMemo(
    () =>
      workspace.entries
        .filter(
          (entry) => !toolFilter || toolIdForSource(entry.source) === toolFilter,
        )
        .map((entry) => Date.parse(entry.discovered_at))
        .filter(Number.isFinite),
    [workspace.entries, toolFilter],
  );
  const buckets = useMemo(
    () =>
      buildGraphBuckets({
        range,
        offset,
        now: new Date(),
        sharedTimes,
        keptTimes,
      }),
    [range, offset, sharedTimes, keptTimes],
  );
  const hoveredBucket = hovered === null ? null : buckets[hovered];
  const shared = hoveredBucket
    ? hoveredBucket.shared
    : buckets.reduce((n, b) => n + b.shared, 0);
  const kept = hoveredBucket
    ? hoveredBucket.kept
    : buckets.reduce((n, b) => n + b.kept, 0);
  const bars: BarBucket[] = buckets.map((bucket) => ({
    key: bucket.start.toISOString(),
    label: bucket.label,
    up: bucket.shared,
    down: bucket.kept,
    description: `${formatDay(bucket.start)}: ${bucket.shared} shared, ${bucket.kept} kept`,
  }));
  const label = hoveredBucket
    ? formatDay(hoveredBucket.start)
    : rangeLabel(range, offset);

  return (
    <div className="flex flex-col gap-2 px-3 py-2.5">
      <div className="tc-legend">
        <LegendCell color="var(--tc-data-shared)" label="shared" value={shared} />
        <LegendCell color="var(--tc-data-kept)" label="kept" value={kept} />
      </div>
      <BarGraph
        buckets={bars}
        hovered={hovered}
        onHover={setHovered}
        animationKey={`${range}:${offset}`}
        scaleFloor={5}
        slide={slide}
      />
      <div className="flex items-center gap-1.5 tc-label">
        <PillIconButton
          label="Previous period"
          onClick={() => {
            setOffset(offset - 1);
            setSlide("r");
          }}
        >
          ‹
        </PillIconButton>
        <PillIconButton
          label={focusTip}
          aria-pressed={focusActive}
          disabled={!canFocus}
          style={{
            color: focusActive ? "#4da3ff" : canFocus ? "#d6d6dc" : "#6b6b70",
          }}
          onClick={onToggleFocus}
        >
          <svg
            width="16"
            height="12"
            viewBox="0 0 16 12"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.4"
            aria-hidden="true"
          >
            <circle cx="4" cy="8" r="3" />
            <circle cx="12" cy="8" r="3" />
            <path d="M4 5V1h2v3M12 5V1h-2v3M7 8h2" />
          </svg>
        </PillIconButton>
        <PillIconButton
          label="Zoom out"
          disabled={range === 2}
          onClick={() => {
            setRange(Math.min(2, range + 1) as GraphRange);
            setOffset(0);
            setSlide(null);
          }}
        >
          −
        </PillIconButton>
        <span
          className="flex h-[26px] min-w-0 flex-1 items-center justify-center truncate rounded-full px-2 font-medium"
          style={{
            background: "rgba(255,255,255,0.07)",
            boxShadow:
              "inset 0 1px 0 rgba(255,255,255,0.18), inset 0 -1px 0 rgba(0,0,0,0.15)",
          }}
          aria-live="polite"
        >
          {label}
        </span>
        <PillIconButton
          label="Zoom in"
          disabled={range === 0}
          onClick={() => {
            setRange(Math.max(0, range - 1) as GraphRange);
            setOffset(0);
            setSlide(null);
          }}
        >
          +
        </PillIconButton>
        <PillIconButton
          label="Next period"
          disabled={offset === 0}
          style={{ opacity: offset === 0 ? 0.4 : 1 }}
          onClick={() => {
            setOffset(Math.min(0, offset + 1));
            setSlide("l");
          }}
        >
          ›
        </PillIconButton>
        <PillIconButton
          label="Jump to now"
          disabled={offset === 0}
          style={{ opacity: offset === 0 ? 0.4 : 1 }}
          onClick={() => {
            setOffset(0);
            setSlide("l");
          }}
        >
          ›|
        </PillIconButton>
      </div>
    </div>
  );
}
