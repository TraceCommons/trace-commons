import type { ReactNode } from "react";
import { Modal } from "@/design-system";

type ResponsiveOverlayProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: string;
  children: ReactNode;
  footer?: ReactNode;
};

/**
 * A confirmation or review raised from inside a pane: the design system's
 * modal, lifted to cover the whole window. The body scrolls; the footer
 * holds the actions.
 */
export function ResponsiveOverlay({
  open,
  onOpenChange,
  title,
  description,
  children,
  footer,
}: ResponsiveOverlayProps) {
  return (
    <Modal
      open={open}
      onClose={() => onOpenChange(false)}
      title={title}
      subtitle={
        description ? (
          <span className="whitespace-pre-line">{description}</span>
        ) : undefined
      }
      footer={footer}
      viewport
      bodyClassName="flex flex-col gap-3 overflow-y-auto px-[18px] py-3"
    >
      {children}
    </Modal>
  );
}
