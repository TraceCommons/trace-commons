import {
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  useId,
} from "react";
import { cx } from "./cx";
import { StatusDot } from "./indicators";

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement>;

/** The one purple action on a surface. */
export function ButtonPrimary({
  size = "md",
  className,
  type = "button",
  ...props
}: ButtonProps & { size?: "md" | "sm" }) {
  return (
    <button
      type={type}
      className={cx(
        "tc-btn tc-btn--primary",
        size === "sm" && "tc-btn--sm",
        className,
      )}
      {...props}
    />
  );
}

/** Dark-purple secondary action (Download). */
export function ButtonSecondary({
  size = "md",
  className,
  type = "button",
  ...props
}: ButtonProps & { size?: "md" | "sm" }) {
  return (
    <button
      type={type}
      className={cx(
        "tc-btn tc-btn--secondary",
        size === "sm" && "tc-btn--sm",
        className,
      )}
      {...props}
    />
  );
}

/** Glass pill button (Check Now, Review permission). */
export function GlassButton({ className, type = "button", ...props }: ButtonProps) {
  return (
    <button
      type={type}
      className={cx("tc-btn tc-btn--glass", className)}
      {...props}
    />
  );
}

/** Round glass button (settings, back). `label` is required: it is icon-only. */
export function RoundButton({
  label,
  small = false,
  className,
  type = "button",
  ...props
}: ButtonProps & { label: string; small?: boolean }) {
  return (
    <button
      type={type}
      aria-label={label}
      title={label}
      className={cx(
        "tc-btn tc-btn--round",
        small && "tc-btn--small",
        className,
      )}
      {...props}
    />
  );
}

/** 30×26 pill with an icon (graph period controls, show-in-map). */
export function PillIconButton({
  label,
  className,
  type = "button",
  ...props
}: ButtonProps & { label: string }) {
  return (
    <button
      type={type}
      aria-label={label}
      title={label}
      className={cx("tc-btn tc-btn--pill-icon", className)}
      {...props}
    />
  );
}

/** Toolbar icon inside a grouped glass capsule (view, graph, map, inspector). */
export function ToolbarIconButton({
  label,
  pressed,
  className,
  type = "button",
  ...props
}: ButtonProps & { label: string; pressed?: boolean }) {
  return (
    <button
      type={type}
      aria-label={label}
      title={label}
      aria-pressed={pressed}
      className={cx("tc-btn tc-btn--icon", className)}
      {...props}
    />
  );
}

/** "Submit · 3" on tree rows; "Sent" in status-on once done. */
export function SubmitPill({
  done = false,
  className,
  type = "button",
  ...props
}: ButtonProps & { done?: boolean }) {
  return (
    <button
      type={type}
      data-done={done}
      className={cx("tc-btn tc-btn--submit", className)}
      {...props}
    />
  );
}

/** Folder chooser: icon + "…", the title carries the full label. */
export function FolderButton({
  label,
  className,
  type = "button",
  ...props
}: ButtonProps & { label: string }) {
  return (
    <button
      type={type}
      title={label}
      aria-label={label}
      className={cx("tc-btn tc-btn--folder", className)}
      {...props}
    >
      <svg
        width="13"
        height="13"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinejoin="round"
        aria-hidden="true"
      >
        <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
      </svg>
      …
    </button>
  );
}

/** "⋮" row menu trigger. */
export function Kebab({
  label = "More",
  open,
  className,
  type = "button",
  ...props
}: ButtonProps & { label?: string; open?: boolean }) {
  return (
    <button
      type={type}
      aria-label={label}
      title={label}
      aria-haspopup="menu"
      aria-expanded={open}
      className={cx("tc-btn tc-btn--kebab", className)}
      {...props}
    >
      <svg
        width="4"
        height="14"
        viewBox="0 0 4 14"
        fill="currentColor"
        aria-hidden="true"
      >
        <circle cx="2" cy="2" r="1.6" />
        <circle cx="2" cy="7" r="1.6" />
        <circle cx="2" cy="12" r="1.6" />
      </svg>
    </button>
  );
}

/** Accent text action, no container. */
export function TertiaryLink({ className, type = "button", ...props }: ButtonProps) {
  return (
    <button type={type} className={cx("tc-link", className)} {...props} />
  );
}

/** "›" that rotates 90° when open, then a label. No container. */
export function Expander({
  open,
  onToggle,
  children,
  controls,
}: {
  open: boolean;
  onToggle?: () => void;
  children: ReactNode;
  controls?: string;
}) {
  return (
    <button
      type="button"
      className="tc-expander"
      aria-expanded={open}
      aria-controls={controls}
      onClick={onToggle}
    >
      <span className="tc-expander__chevron" aria-hidden="true">
        ›
      </span>
      {children}
    </button>
  );
}

export type PickerOption<T extends string> = {
  value: T;
  label: string;
  /** Status colour for the dot; omit for no dot. */
  dot?: string;
  disabled?: boolean;
};

/**
 * Native select dressed as a glass pill: dot + label + separate chevron.
 * With no value it reads "Choose…" and shows no dot.
 */
export function Picker<T extends string>({
  value,
  options,
  onChange,
  label,
  placeholder = "Choose…",
  disabled = false,
  className,
}: {
  value: T | null;
  options: PickerOption<T>[];
  onChange: (value: T) => void;
  label: string;
  placeholder?: string;
  disabled?: boolean;
  className?: string;
}) {
  const current = options.find((option) => option.value === value) ?? null;
  return (
    <span className={cx("tc-picker", className)} data-disabled={disabled}>
      {current?.dot ? <StatusDot color={current.dot} size="md" /> : null}
      <span>{current?.label ?? placeholder}</span>
      <svg
        className="tc-picker__chevron"
        width="10"
        height="10"
        viewBox="0 0 16 16"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        aria-hidden="true"
      >
        <path d="M4 6.5l4 4 4-4" />
      </svg>
      <select
        aria-label={label}
        value={value ?? ""}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value as T)}
      >
        {value === null ? (
          <option value="" disabled>
            {placeholder}
          </option>
        ) : null}
        {options.map((option) => (
          <option
            key={option.value}
            value={option.value}
            disabled={option.disabled}
          >
            {option.label}
          </option>
        ))}
      </select>
    </span>
  );
}

type SwitchProps = Omit<ButtonProps, "onChange"> & {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
};

function SwitchBase({
  checked,
  onChange,
  label,
  variant,
  className,
  ...props
}: SwitchProps & { variant: "toggle" | "settings" | "watch" }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      title={label}
      className={cx(
        "tc-toggle",
        variant === "settings" && "tc-toggle--settings",
        variant === "watch" && "tc-toggle--watch",
        className,
      )}
      onClick={(event) => {
        event.stopPropagation();
        onChange(!checked);
      }}
      {...props}
    />
  );
}

/** 40×24 toggle, blue when on. `settings` makes it purple, as in Settings. */
export function Toggle({
  settings = false,
  ...props
}: SwitchProps & { settings?: boolean }) {
  return <SwitchBase variant={settings ? "settings" : "toggle"} {...props} />;
}

/** 38×22 watch switch on tree rows, green when watching. */
export function WatchSwitch(props: SwitchProps) {
  return <SwitchBase variant="watch" {...props} />;
}

/**
 * 15px rounded checkbox: purple gradient + white check when on, dark well
 * when off. `indeterminate` is for a group checkbox whose children differ.
 */
export function Checkbox({
  checked,
  indeterminate = false,
  onChange,
  label,
  disabled,
  describedBy,
}: {
  checked: boolean;
  indeterminate?: boolean;
  onChange?: (checked: boolean) => void;
  label: string;
  disabled?: boolean;
  describedBy?: string;
}) {
  const state = indeterminate ? "mixed" : checked;
  return (
    // biome-ignore lint/a11y/useSemanticElements: the design's 15px glass checkbox with a mixed state; role and aria-checked carry the semantics.
    <button
      type="button"
      role="checkbox"
      aria-checked={state}
      aria-label={label}
      aria-describedby={describedBy}
      disabled={disabled || !onChange}
      className="tc-checkbox"
      onClick={() => onChange?.(!checked)}
    >
      {indeterminate ? (
        <svg width="9" height="9" viewBox="0 0 24 24" aria-hidden="true">
          <path d="M6 12h12" stroke="#fff" strokeWidth="3.5" />
        </svg>
      ) : checked ? (
        <svg
          width="9"
          height="9"
          viewBox="0 0 24 24"
          fill="none"
          stroke="#fff"
          strokeWidth="3.5"
          aria-hidden="true"
        >
          <path d="M5 12l5 5 9-10" />
        </svg>
      ) : null}
    </button>
  );
}

/** Checkbox with its sentence beside it. Read-only when `onChange` is absent. */
export function CheckRow({
  checked,
  onChange,
  children,
  disabled,
}: {
  checked: boolean;
  onChange?: (checked: boolean) => void;
  children: ReactNode;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <div className="tc-check-row">
      <Checkbox
        checked={checked}
        onChange={onChange}
        disabled={disabled}
        label={typeof children === "string" ? children : "Option"}
        describedBy={id}
      />
      <span id={id}>{children}</span>
    </div>
  );
}

/** Labelled field (eyebrow label over a dark input). */
export function TextField({
  label,
  hint,
  multiline = false,
  className,
  ...props
}: InputHTMLAttributes<HTMLInputElement> & {
  label: string;
  hint?: ReactNode;
  multiline?: boolean;
}) {
  const id = useId();
  return (
    <div className={cx("tc-field", className)}>
      <label htmlFor={id} className="tc-eyebrow">
        {label}
      </label>
      {multiline ? (
        <textarea
          id={id}
          className="tc-input"
          value={props.value as string | undefined}
          placeholder={props.placeholder}
          disabled={props.disabled}
          onChange={
            props.onChange as unknown as React.ChangeEventHandler<HTMLTextAreaElement>
          }
        />
      ) : (
        <input id={id} className="tc-input" {...props} />
      )}
      {hint ? <p className="m-0 tc-caption tc-text-tertiary">{hint}</p> : null}
    </div>
  );
}
