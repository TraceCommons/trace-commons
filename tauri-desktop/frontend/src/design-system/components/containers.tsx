import type { HTMLAttributes, ReactNode } from "react";
import { cx } from "./cx";
import { StatusDot, StatusLabel } from "./indicators";
import type { StatusTone } from "./status";

type DivProps = HTMLAttributes<HTMLDivElement>;

/** Tint layer on a pane. Never blurred. */
export function Card({
  quiet = false,
  flush = false,
  interactive = false,
  className,
  ...props
}: DivProps & { quiet?: boolean; flush?: boolean; interactive?: boolean }) {
  return (
    <div
      className={cx(
        "tc-card",
        quiet && "tc-card--quiet",
        flush && "tc-card--flush",
        interactive && "tc-card--interactive",
        className,
      )}
      {...props}
    />
  );
}

/** Card at the quiet tier (12px radius). */
export function QuietCard(props: DivProps & { flush?: boolean }) {
  return <Card quiet {...props} />;
}

/** Card with an eyebrow heading and an optional accessory on the right. */
export function EyebrowCard({
  eyebrow,
  accessory,
  children,
  className,
  onClick,
}: {
  eyebrow: ReactNode;
  accessory?: ReactNode;
  children?: ReactNode;
  className?: string;
  onClick?: () => void;
}) {
  const body = (
    <>
      <div className="tc-card__head">
        <span className="tc-eyebrow">{eyebrow}</span>
        {accessory}
      </div>
      {children}
    </>
  );
  return onClick ? (
    <button
      type="button"
      className={cx(
        "tc-card tc-card--interactive flex w-full flex-col gap-2 border-0 text-left text-inherit",
        className,
      )}
      onClick={onClick}
    >
      {body}
    </button>
  ) : (
    <Card className={cx("flex flex-col gap-2", className)}>{body}</Card>
  );
}

/** Inset track (legend, segmented tabs, unchecked boxes). */
export function Well({ className, ...props }: DivProps) {
  return <div className={cx("tc-well", className)} {...props} />;
}

/** The consent sentence, verbatim, directly above Submit. */
export function ConsentBlock({ children }: { children: ReactNode }) {
  return <div className="tc-consent">{children}</div>;
}

/** Grid row inside a flush card. Hairline on top, never around. */
export function TableRow({
  columns,
  className,
  ...props
}: DivProps & { columns: string }) {
  return (
    <div
      className={cx("tc-table-row", className)}
      style={{ gridTemplateColumns: columns, ...props.style }}
      {...props}
    />
  );
}

export function TableHead({
  columns,
  children,
}: {
  columns: string;
  children: ReactNode;
}) {
  return (
    <div
      className="tc-table-head tc-eyebrow"
      style={{ gridTemplateColumns: columns }}
    >
      {children}
    </div>
  );
}

/** One well per datum: dot + 2px halo, label, tabular count. */
export function LegendCell({
  color,
  label,
  value,
}: {
  color: string;
  label: ReactNode;
  value: ReactNode;
}) {
  return (
    <div className="tc-legend-cell">
      <span className="tc-legend-cell__label">
        <StatusDot color={color} halo />
        {label}
      </span>
      <span className="tc-legend-cell__value">{value}</span>
    </div>
  );
}

/** Label/value pairs (inspector: Path, Sessions, User). */
export function KeyValueList({
  items,
}: {
  items: Array<{ label: string; value: ReactNode; mono?: boolean }>;
}) {
  return (
    <dl className="tc-kv m-0">
      {items.map((item) => (
        <FragmentPair key={item.label} {...item} />
      ))}
    </dl>
  );
}

function FragmentPair({
  label,
  value,
  mono,
}: {
  label: string;
  value: ReactNode;
  mono?: boolean;
}) {
  return (
    <>
      <dt>{label}</dt>
      <dd className={cx("m-0 min-w-0", mono && "tc-mono break-all")}>
        {value}
      </dd>
    </>
  );
}

/**
 * A notice on a quiet card: the title carries the tone as a status dot, the
 * body stays in the text colours. Tone never tints the card or its edge.
 * `role` defaults to "alert"; pass "status" for news that is not a fault.
 */
export function Notice({
  tone = "ask",
  title,
  children,
  role = "alert",
  className,
}: {
  tone?: StatusTone;
  title?: ReactNode;
  children?: ReactNode;
  role?: "alert" | "status";
  className?: string;
}) {
  return (
    <div role={role} className={cx("tc-card tc-card--quiet tc-notice", className)}>
      {title ? (
        <div className="tc-notice__title">
          <StatusLabel tone={tone}>{title}</StatusLabel>
        </div>
      ) : null}
      {children ? <div className="tc-notice__body">{children}</div> : null}
    </div>
  );
}

/** Settings section heading: purple eyebrow and a rule to the right. */
export function SectionRule({
  children,
  id,
}: {
  children: ReactNode;
  id?: string;
}) {
  return (
    <div className="tc-section-rule" data-sec={id}>
      <h3 className="m-0">{children}</h3>
    </div>
  );
}
