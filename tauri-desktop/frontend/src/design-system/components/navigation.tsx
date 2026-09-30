import { Fragment, type ReactNode } from "react";
import { cx } from "./cx";

export type SegmentedItem<T extends string> = {
  value: T;
  label: ReactNode;
  /** Decisions owed, shown as a count pill. Never queue depth or credit. */
  badge?: number;
  /** A status dot after the label (Inference: Private AI on/off). */
  dot?: string;
};

/**
 * Segmented tabs in a well (Home · Inference · Traces). `floating` is the
 * variant that sits on the map (Traces · Private AI).
 */
export function SegmentedTabs<T extends string>({
  items,
  value,
  onChange,
  floating = false,
  label,
  className,
}: {
  items: SegmentedItem<T>[];
  value: T;
  onChange: (value: T) => void;
  floating?: boolean;
  label: string;
  className?: string;
}) {
  return (
    <div
      role="tablist"
      aria-label={label}
      className={cx(
        "tc-segmented",
        floating && "tc-segmented--floating",
        className,
      )}
    >
      {items.map((item) => (
        <button
          key={item.value}
          type="button"
          role="tab"
          aria-selected={item.value === value}
          className="tc-segmented__item"
          onClick={() => onChange(item.value)}
        >
          {item.label}
          {item.dot ? (
            <span
              className="tc-dot tc-dot--sm"
              style={{ "--tc-dot": item.dot } as React.CSSProperties}
              aria-hidden="true"
            />
          ) : null}
          {item.badge !== undefined ? (
            <span className="tc-badge tc-badge--count">{item.badge}</span>
          ) : null}
        </button>
      ))}
    </div>
  );
}

/** Round back button, then crumbs: secondary › tertiary › current. */
export function Breadcrumb({
  trail,
  onBack,
}: {
  trail: Array<{ label: string; onSelect?: () => void }>;
  onBack?: () => void;
}) {
  return (
    <nav className="tc-breadcrumb" aria-label="Breadcrumb">
      {onBack ? (
        <button
          type="button"
          className="tc-btn tc-btn--round tc-btn--small"
          aria-label={`Back to ${trail[0]?.label ?? "previous"}`}
          onClick={onBack}
        >
          <svg
            width="12"
            height="12"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.2"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path d="M15 5l-7 7 7 7" />
          </svg>
        </button>
      ) : null}
      {trail.map((crumb, index) => {
        const last = index === trail.length - 1;
        return (
          <Fragment key={crumb.label}>
            {index > 0 ? (
              <span className="tc-breadcrumb__sep" aria-hidden="true">
                ›
              </span>
            ) : null}
            {last ? (
              <span className="tc-breadcrumb__current" aria-current="page">
                {crumb.label}
              </span>
            ) : (
              <button
                type="button"
                className="tc-breadcrumb__crumb"
                onClick={crumb.onSelect}
              >
                {crumb.label}
              </button>
            )}
          </Fragment>
        );
      })}
    </nav>
  );
}

/** Sequential glass nodes joined by lines (FTUX). Not a tab control. */
export function StepProgress({
  labels,
  current,
}: {
  labels: string[];
  current: number;
}) {
  return (
    <ol className="tc-steps m-0 list-none p-0" aria-label="Progress">
      {labels.map((label, index) => {
        const state =
          index < current ? "done" : index === current ? "current" : "pending";
        return (
          <Fragment key={label}>
            {index > 0 ? (
              <li
                aria-hidden="true"
                className="tc-steps__line"
                data-done={index <= current}
              />
            ) : null}
            <li
              className="tc-steps__node"
              data-state={state}
              aria-current={state === "current" ? "step" : undefined}
            >
              <span className="tc-steps__dot" />
              <span className="tc-steps__label">{label}</span>
            </li>
          </Fragment>
        );
      })}
    </ol>
  );
}
