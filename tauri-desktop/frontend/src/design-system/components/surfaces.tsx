import {
  type HTMLAttributes,
  type ReactNode,
  useEffect,
  useId,
  useRef,
} from "react";
import { cx } from "./cx";

type DivProps = HTMLAttributes<HTMLDivElement>;

/**
 * The scene the glass sits over, and the window's padding around its panes.
 * `rim` draws the gradient window rim; the app itself uses panes as the
 * outermost containment, so the rim is for mocks only.
 */
export function Window({
  rim = false,
  className,
  ...props
}: DivProps & { rim?: boolean }) {
  return (
    <div
      className={cx("tc-root tc-window", rim && "tc-window--rim", className)}
      {...props}
    />
  );
}

/** One blurred glass sheet. Panes are the only blurred layer. */
export function Pane({
  padded = false,
  className,
  ...props
}: DivProps & { padded?: boolean }) {
  return (
    <div
      className={cx("tc-pane", padded && "tc-pane--padded", className)}
      {...props}
    />
  );
}

/** Popover tier: menu-bar panel, floating menus. */
export function Popover({ className, ...props }: DivProps) {
  return <div className={cx("tc-popover", className)} {...props} />;
}

/**
 * A modal over its container (settings, passkey). Closes on the scrim, on
 * Escape, and from its own close button. `narrow` fixes the width at 450px,
 * the width the FTUX flows use.
 */
export function Modal({
  open,
  onClose,
  title,
  subtitle,
  headerAccessory,
  narrow = false,
  children,
  className,
  bodyClassName,
}: {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  subtitle?: ReactNode;
  headerAccessory?: ReactNode;
  narrow?: boolean;
  children: ReactNode;
  className?: string;
  bodyClassName?: string;
}) {
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement as HTMLElement | null;
    dialogRef.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      previous?.focus?.();
    };
  }, [open, onClose]);

  if (!open) return null;
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: the scrim closes on click; Escape and the close button are the keyboard paths.
    <div className="tc-scrim" onClick={onClose} role="presentation">
      {/* biome-ignore lint/a11y/useKeyWithClickEvents: stops a click inside the dialog reaching the scrim; not an action. */}
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
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
              <div className="tc-caption tc-text-tertiary">{subtitle}</div>
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
      </div>
    </div>
  );
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
