import type { ReactNode } from "react";
import {
  Card,
  Pane,
  StatusDot,
  type StatusTone,
  StepProgress,
  Tag,
  Window,
} from "../../../design-system";
import type { FtuxStep } from "../ftux-model";

// The first-run window: the design system's scene with one padded pane at
// the FTUX width (450px), the step progress, and the screen inside it.
export function FtuxFrame({
  eyebrow,
  steps,
  current,
  children,
  overlays,
}: {
  eyebrow: string;
  steps: FtuxStep[];
  current: number;
  children: ReactNode;
  // Popups render beside the pane, so the pane can be made inert under them.
  overlays?: ReactNode;
}) {
  return (
    <Window className="ftux-scene">
      <span className="ftux-preview-tag" title="Backend calls are mocked">
        <Tag tone="ask">PREVIEW · MOCK DATA</Tag>
      </span>
      <Pane padded className="ftux-pane" aria-label={eyebrow} tabIndex={-1}>
        <div className="ftux-pane__bar">
          <span className="tc-eyebrow">{eyebrow}</span>
        </div>
        <StepProgress
          labels={steps.map((step) => step.label)}
          current={current}
        />
        <div className="ftux-pane__body">{children}</div>
      </Pane>
      {overlays}
    </Window>
  );
}

// A screen title: a light lead-in, then the bold part (the design's style).
export function ScreenTitle({ light, bold }: { light: string; bold: string }) {
  return (
    <h1>
      <span className="ftux-light">{light}</span>
      {bold}
    </h1>
  );
}

// Scrolls the screen's cards between the title and the footer.
export function ScreenBody({ children }: { children: ReactNode }) {
  return <div className="ftux-scroll tc-stack">{children}</div>;
}

export function ScreenFooter({
  note,
  noteId,
  children,
}: {
  note?: ReactNode;
  noteId?: string;
  children: ReactNode;
}) {
  return (
    <div className="ftux-footer" data-split={note ? true : undefined}>
      {note ? (
        <span id={noteId} className="tc-caption tc-text-tertiary">
          {note}
        </span>
      ) : null}
      {children}
    </div>
  );
}

// A notice on the design system's quiet card: status is a glyph and a
// label, never a coloured edge or fill.
export function StatusNote({
  tone,
  icon,
  role,
  children,
}: {
  tone: Extract<StatusTone, "ask" | "outside" | "on">;
  icon?: ReactNode;
  role?: "alert" | "status";
  children: ReactNode;
}) {
  return (
    <Card quiet className="ftux-notice" role={role} data-tone={tone}>
      <span className="ftux-notice__glyph">
        {icon ?? <StatusDot tone={tone} />}
      </span>
      <span>{children}</span>
    </Card>
  );
}

// A single status or error line.
export function StatusLine({
  tone,
  children,
}: {
  tone: "ok" | "error";
  children: ReactNode;
}) {
  return tone === "ok" ? (
    <span className="tc-status tc-text-on" role="status">
      {children}
    </span>
  ) : (
    <StatusNote tone="outside" role="alert">
      {children}
    </StatusNote>
  );
}
