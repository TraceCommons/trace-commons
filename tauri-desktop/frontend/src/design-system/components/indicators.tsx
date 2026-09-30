import type { CSSProperties, ReactNode } from "react";
import { cx } from "./cx";
import type { ToolLogoId } from "./logo-paths";
import { ToolLogo } from "./logos";

import { statusColor, type StatusTone } from "./status";

export function StatusDot({
  color,
  tone,
  size = "default",
  halo = false,
  ring = false,
  empty = false,
  className,
}: {
  color?: string;
  tone?: StatusTone;
  size?: "sm" | "md" | "default";
  halo?: boolean;
  ring?: boolean;
  empty?: boolean;
  className?: string;
}) {
  const fill = color ?? (tone ? statusColor[tone] : undefined);
  return (
    <span
      aria-hidden="true"
      className={cx(
        "tc-dot",
        size === "sm" && "tc-dot--sm",
        size === "md" && "tc-dot--md",
        halo && "tc-dot--halo",
        ring && "tc-dot--ring",
        empty && "tc-dot--empty",
        className,
      )}
      style={fill ? ({ "--tc-dot": fill } as CSSProperties) : undefined}
    />
  );
}

/** Dot + label pair, the only way status is shown. */
export function StatusLabel({
  tone,
  color,
  children,
}: {
  tone?: StatusTone;
  color?: string;
  children: ReactNode;
}) {
  return (
    <span className="tc-status">
      <StatusDot tone={tone} color={color} />
      {children}
    </span>
  );
}

/** Mono chip with a 0.5px ring at 60% of its colour. `glass` for settings. */
export function Chip({
  tone,
  color,
  glass = false,
  dot = true,
  children,
}: {
  tone?: StatusTone;
  color?: string;
  glass?: boolean;
  dot?: boolean;
  children: ReactNode;
}) {
  const ink = color ?? (tone ? statusColor[tone] : undefined);
  return (
    <span
      className={cx("tc-chip", glass && "tc-chip--glass")}
      style={ink && !glass ? ({ "--tc-chip": ink } as CSSProperties) : undefined}
    >
      {dot ? (
        <StatusDot
          size="sm"
          color={glass ? (ink ?? statusColor.on) : ink}
        />
      ) : null}
      {children}
    </span>
  );
}

/** Tinted verdict pill (Contributed, Under review, Taken back, Drafts). */
export function Tag({
  tone,
  children,
}: {
  tone?: "on" | "ask" | "outside" | "accent";
  children: ReactNode;
}) {
  return (
    <span className="tc-tag" data-tone={tone}>
      {children}
    </span>
  );
}

/** Decisions owed. On the menu bar and nav only; never queue depth or credit. */
export function Badge({
  count,
  subtle = false,
  label,
}: {
  count: number;
  subtle?: boolean;
  label?: string;
}) {
  return (
    <span className={cx("tc-badge", subtle && "tc-badge--count")}>
      <span aria-hidden={label ? true : undefined}>{count}</span>
      {label ? <span className="sr-only">{label}</span> : null}
    </span>
  );
}

/** 22px tile: tinted tool logo, or a folder / session glyph. */
export function ToolTile({
  tool,
  kind = "tool",
  fallback,
  large = false,
}: {
  tool?: ToolLogoId | null;
  kind?: "tool" | "folder" | "session";
  fallback?: string;
  large?: boolean;
}) {
  return (
    <span
      aria-hidden="true"
      className={cx(
        "tc-tool-tile",
        kind === "folder" && "tc-tool-tile--folder",
        kind === "session" && "tc-tool-tile--session",
        large && "tc-tool-tile--lg",
      )}
    >
      {kind === "tool" && tool ? (
        <ToolLogo id={tool} size={large ? 20 : 15} />
      ) : kind === "folder" ? (
        "dir"
      ) : kind === "session" ? (
        "▤"
      ) : (
        (fallback ?? "")
      )}
    </span>
  );
}

export type BarBucket = {
  key: string;
  label: string;
  /** Upward (shared) value. */
  up: number;
  /** Downward (kept) value. */
  down: number;
  /** Accessible description of the bucket. */
  description: string;
};

/**
 * Shared-over-kept bar graph: shared rises from the mid axis in purple, kept
 * falls in blue. Values are scaled to the largest bucket.
 */
export function BarGraph({
  buckets,
  onHover,
  hovered,
  animationKey,
  slide,
  scaleFloor = 1,
}: {
  buckets: BarBucket[];
  /** Smallest value the tallest bar stands for, so a count of 1 is short. */
  scaleFloor?: number;
  onHover?: (index: number | null) => void;
  hovered?: number | null;
  animationKey?: string;
  slide?: "l" | "r" | null;
}) {
  const max = Math.max(scaleFloor, ...buckets.flatMap((b) => [b.up, b.down]));
  const thin = buckets.length > 14;
  const height = (value: number) =>
    value <= 0 ? 0 : Math.max(3, Math.round((value / max) * 46));
  return (
    <ul
      key={animationKey}
      className="tc-bar-graph m-0 list-none p-0"
      data-thin={thin}
      style={{
        gridTemplateColumns: `repeat(${Math.max(1, buckets.length)}, 1fr)`,
        animation: slide
          ? `tc-slide-${slide} .45s cubic-bezier(.4,0,.2,1)`
          : undefined,
      }}
    >
      {/* biome-ignore lint/complexity/noExcessiveCognitiveComplexity: per-bucket styling for thin and hovered bars. */}
      {buckets.map((bucket, index) => (
        <li
          key={bucket.key}
          aria-label={bucket.description}
          className="tc-bar-graph__col"
          onMouseEnter={() => onHover?.(index)}
          onMouseLeave={() => onHover?.(null)}
        >
          <div
            className="tc-bar-graph__track"
            style={{
              borderRadius: thin ? 3 : undefined,
              background:
                hovered === index ? "rgba(255,255,255,0.18)" : undefined,
            }}
          >
            <div className="tc-bar-graph__axis" />
            <div
              className="tc-bar-graph__up"
              style={{
                height: height(bucket.up),
                left: thin ? "15%" : undefined,
                right: thin ? "15%" : undefined,
              }}
            />
            <div
              className="tc-bar-graph__down"
              style={{
                height: height(bucket.down),
                left: thin ? "15%" : undefined,
                right: thin ? "15%" : undefined,
              }}
            />
          </div>
          <span
            className="tc-bar-graph__label"
            style={{
              fontSize: thin ? 9 : undefined,
              color: hovered === index ? "var(--tc-text-primary)" : undefined,
            }}
          >
            {bucket.label}
          </span>
        </li>
      ))}
    </ul>
  );
}

/** Map node: a circle with a label, dimmed when inactive. SVG only. */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: label placement and optional handlers are independent choices.
export function MapNode({
  x,
  y,
  r = 11,
  fill,
  ring = false,
  label,
  sublabel,
  labelBelow = true,
  dim = 1,
  onSelect,
  onHover,
  children,
}: {
  x: number;
  y: number;
  r?: number;
  fill: string;
  ring?: boolean;
  label?: string;
  sublabel?: string;
  labelBelow?: boolean;
  dim?: number;
  onSelect?: () => void;
  onHover?: (hovering: boolean) => void;
  children?: ReactNode;
}) {
  const interactive = Boolean(onSelect);
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: an SVG group has no button element; it takes role="button" and a tab stop when it acts.
    <g
      className="tc-map__node"
      opacity={dim}
      role={interactive ? "button" : undefined}
      tabIndex={interactive ? 0 : undefined}
      aria-label={interactive ? label : undefined}
      onClick={(event) => {
        event.stopPropagation();
        onSelect?.();
      }}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") onSelect?.();
      }}
      onMouseEnter={() => onHover?.(true)}
      onMouseLeave={() => onHover?.(false)}
      onFocus={() => onHover?.(true)}
      onBlur={() => onHover?.(false)}
    >
      <circle
        cx={x}
        cy={y}
        r={r}
        fill={fill}
        stroke={ring ? "#fff" : "transparent"}
        strokeWidth={2.5}
      />
      {children}
      {label ? (
        <text
          className="tc-map__label"
          x={labelBelow ? x : x + r + 7}
          y={labelBelow ? y + r + 16 : y - 3}
          textAnchor={labelBelow ? "middle" : "start"}
        >
          {label}
        </text>
      ) : null}
      {sublabel ? (
        <text
          className="tc-map__sublabel"
          x={labelBelow ? x : x + r + 7}
          y={labelBelow ? y + r + 30 : y + 11}
          textAnchor={labelBelow ? "middle" : "start"}
        >
          {sublabel}
        </text>
      ) : null}
    </g>
  );
}

/** Quadratic arc between two points; `pulse` draws the steady upload flow. */
export function MapArc({
  from,
  to,
  tone,
  pulse = false,
  dim = 1,
  bend = 0.28,
}: {
  from: [number, number];
  to: [number, number];
  tone?: "on" | "outside";
  pulse?: boolean;
  dim?: number;
  bend?: number;
}) {
  const [x0, y0] = from;
  const [x1, y1] = to;
  const mx = (x0 + x1) / 2;
  const my = (y0 + y1) / 2;
  const dx = x1 - x0;
  const dy = y1 - y0;
  const d = `M${x0} ${y0} Q${(mx - dy * bend).toFixed(0)} ${(my + dx * bend).toFixed(0)} ${x1} ${y1}`;
  return (
    <path
      className="tc-map__arc"
      d={d}
      data-tone={tone}
      data-pulse={pulse}
      opacity={dim}
      style={
        pulse
          ? { color: tone === "outside" ? "#ff6b6b" : "#8a3dff" }
          : undefined
      }
    />
  );
}

/** Node card shown on map hover / pin (popover tier). */
export function NodeCard({
  title,
  body,
  hint,
  style,
}: {
  title: ReactNode;
  body: ReactNode;
  hint?: ReactNode;
  style?: CSSProperties;
}) {
  return (
    <div className="tc-node-card" style={style} role="status">
      <div className="tc-node-card__title">{title}</div>
      <div className="tc-node-card__body">{body}</div>
      {hint ? <div className="tc-node-card__hint">{hint}</div> : null}
    </div>
  );
}
