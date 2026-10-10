import {
  type HTMLAttributes,
  type ReactNode,
  useEffect,
  useId,
  useRef,
} from "react";
import { createPortal } from "react-dom";
import { cx } from "./cx";

type DivProps = HTMLAttributes<HTMLDivElement>;

/**
 * The scene the glass sits over, and the window's padding around its panes.
 * `floating` drops the backdrop and padding so the panes are the outermost
 * containment and float as one unit (the app's transparent window); each
 * pane then paints the scene behind its own glass. `rim` draws the gradient
 * window rim, for mocks that show a chrome window.
 */
export function Window({
  rim = false,
  floating = false,
  className,
  ...props
}: DivProps & { rim?: boolean; floating?: boolean }) {
  return (
    <div
      className={cx(
        "tc-root tc-window",
        rim && "tc-window--rim",
        floating && "tc-window--floating",
        className,
      )}
      {...props}
    />
  );
}

/**
 * One blurred glass sheet. Panes are the only blurred layer. `data-glass`
 * marks it for the app's native glass, which sits under each marked
 * element when the window supports it.
 */
export function Pane({
  padded = false,
  className,
  ...props
}: DivProps & { padded?: boolean }) {
  return (
    <div
      data-glass=""
      className={cx("tc-pane", padded && "tc-pane--padded", className)}
      {...props}
    />
  );
}

/** Popover tier: menu-bar panel, floating menus. */
export function Popover({ className, ...props }: DivProps) {
  return <div className={cx("tc-popover", className)} {...props} />;
}

const FOCUSABLE =
  'a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])';

/**
 * A modal over its container (settings, passkey). Closes on the scrim, on
 * Escape, and from its own close button; Tab stays inside it while it is
 * open, and focus returns to where it was when it closes. `narrow` fixes the
 * width at 450px, the width the FTUX flows use. `viewport` lifts it out of
 * its container to cover the whole window (confirmations raised from deep
 * inside a pane). `footer` holds the actions, below the scrolling body.
 * A modal is never a native glass region: it keeps its own near-opaque
 * backing, so what it covers never reads through it.
 */
export function Modal({
  open,
  onClose,
  title,
  subtitle,
  headerAccessory,
  footer,
  narrow = false,
  viewport = false,
  children,
  className,
  bodyClassName,
}: {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  subtitle?: ReactNode;
  headerAccessory?: ReactNode;
  footer?: ReactNode;
  narrow?: boolean;
  viewport?: boolean;
  children: ReactNode;
  className?: string;
  bodyClassName?: string;
}) {
  const titleId = useId();
  const subtitleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  // Held in a ref so a caller's inline `onClose` does not re-run the focus
  // effect (and pull focus back to the dialog) on every render.
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement as HTMLElement | null;
    dialogRef.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      // Only the topmost dialog answers: a confirmation raised over the
      // settings modal closes alone.
      const open = document.querySelectorAll('[role="dialog"][aria-modal="true"]');
      if (open[open.length - 1] !== dialogRef.current) return;
      if (event.key === "Escape") {
        closeRef.current();
        return;
      }
      const dialog = dialogRef.current;
      if (event.key !== "Tab" || !dialog) return;
      const items = Array.from(
        dialog.querySelectorAll<HTMLElement>(FOCUSABLE),
      );
      if (items.length === 0) {
        event.preventDefault();
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement;
      if (event.shiftKey && (active === first || active === dialog)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      } else if (!dialog.contains(active)) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      previous?.focus?.();
    };
  }, [open]);

  if (!open) return null;
  const modal = (
    // biome-ignore lint/a11y/noStaticElementInteractions: the scrim closes on click; Escape and the close button are the keyboard paths.
    <div
      className={cx("tc-scrim", viewport && "tc-scrim--viewport tc-root")}
      onClick={onClose}
      role="presentation"
    >
      {/* biome-ignore lint/a11y/useKeyWithClickEvents: stops a click inside the dialog reaching the scrim; not an action. */}
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={subtitle ? subtitleId : undefined}
        tabIndex={-1}
        className={cx(
          "tc-modal flex flex-col outline-none",
          narrow && "tc-modal--narrow",
          className,
        )}
        onClick={(event) => event.stopPropagation()}
      >
        <div className="tc-modal__header">
          <div>
            <div id={titleId} className="tc-title font-bold">
              {title}
            </div>
            {subtitle ? (
              <div id={subtitleId} className="tc-caption tc-text-tertiary">
                {subtitle}
              </div>
            ) : null}
          </div>
          <div className="flex items-center gap-2">
            {headerAccessory}
            <button
              type="button"
              className="tc-btn tc-btn--round tc-btn--small"
              aria-label="Close"
              onClick={onClose}
            >
              <span aria-hidden="true" className="text-xs">
                ✕
              </span>
            </button>
          </div>
        </div>
        <div className={cx("min-h-0 flex-1", bodyClassName)}>{children}</div>
        {footer ? <div className="tc-modal__footer">{footer}</div> : null}
      </div>
    </div>
  );
  return viewport ? createPortal(modal, document.body) : modal;
}

/**
 * A sheet: the "exactly what would be sent" surface inside a pane or modal.
 */
export function Sheet({ className, ...props }: DivProps) {
  return <div className={cx("tc-sheet", className)} {...props} />;
}

/** Floating menu (view options, row menu). */
export function Menu({ className, ...props }: DivProps) {
  return <div role="menu" className={cx("tc-menu", className)} {...props} />;
}

export function MenuItem({
  checked,
  disabled,
  onSelect,
  children,
}: {
  checked?: boolean;
  disabled?: boolean;
  onSelect?: () => void;
  children: ReactNode;
}) {
  if (checked === undefined)
    return (
      <button
        type="button"
        role="menuitem"
        disabled={disabled}
        className="tc-menu__item"
        onClick={onSelect}
      >
        {children}
      </button>
    );
  return (
    <button
      type="button"
      role="menuitemcheckbox"
      aria-checked={checked}
      disabled={disabled}
      className="tc-menu__item"
      onClick={onSelect}
    >
      <span className="tc-menu__check" aria-hidden="true">
        {checked ? "✓" : ""}
      </span>
      {children}
    </button>
  );
}

export function MenuSeparator() {
  return <hr className="tc-menu__separator m-0 border-0" />;
}
