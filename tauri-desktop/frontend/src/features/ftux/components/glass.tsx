import type { ReactNode } from "react";
import type { FtuxStep } from "../ftux-model";

// Icons ------------------------------------------------------------------
// Paths are the ones drawn in the WYSIWYG design.

type IconProps = { size?: number };

export function CheckIcon({ size = 9 }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="#fff"
      strokeWidth="3.5"
      aria-hidden="true"
    >
      <path d="M5 12l5 5 9-10" />
    </svg>
  );
}

export function ChevronIcon({ size = 10 }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="#a9a9b0"
      strokeWidth="1.8"
      aria-hidden="true"
      style={{ flex: "none", pointerEvents: "none" }}
    >
      <path d="M4 6.5l4 4 4-4" />
    </svg>
  );
}

export function FolderIcon() {
  return (
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
  );
}

export function DownloadIcon() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M12 4v11M7 10l5 5 5-5M4 20h16" />
    </svg>
  );
}

export function ExternalIcon() {
  return (
    <svg
      width="11"
      height="11"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.4"
      aria-hidden="true"
    >
      <path d="M7 17L17 7M9 7h8v8" />
    </svg>
  );
}

export function BackIcon() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      aria-hidden="true"
    >
      <path d="M15 5l-7 7 7 7" />
    </svg>
  );
}

export function CloseIcon({ size = 12 }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
      aria-hidden="true"
    >
      <path d="M6 6l12 12M18 6L6 18" />
    </svg>
  );
}

export function PasskeyIcon() {
  return (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <circle cx="9" cy="8" r="3.5" />
      <path d="M3 20a6 6 0 0 1 12 0" />
      <circle cx="18" cy="9" r="2.2" />
      <path d="M18 11.2V17l1.5 1.5M18 14.5h1.6" />
    </svg>
  );
}

export function LockIcon() {
  return (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <rect x="5" y="10" width="14" height="10" rx="2.5" />
      <path d="M8 10V7.5a4 4 0 0 1 8 0V10" />
    </svg>
  );
}

export function WarningIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      aria-hidden="true"
    >
      <path d="M12 9v4M12 17h.01M10.3 3.9L2.5 18a2 2 0 0 0 1.7 3h15.6a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z" />
    </svg>
  );
}

export function FingerprintIcon({
  size = 44,
  strokeWidth = 1.3,
}: IconProps & { strokeWidth?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M7 8.5a6 6 0 0 1 10 0M5.5 12a8 8 0 0 1 13 0M9 12a3.5 3.5 0 0 1 6 1.5c0 2.5-.8 4.5-2 6.5M12 12v2.5c0 2.3-.7 4.2-1.8 5.8M15 16.8c-.3 1.4-.9 2.6-1.7 3.7" />
    </svg>
  );
}

// Layout -----------------------------------------------------------------

export function GlassWindow({
  eyebrow,
  steps,
  current,
  children,
}: {
  eyebrow: string;
  steps: FtuxStep[];
  current: number;
  children: ReactNode;
}) {
  return (
    <section className="ftux-window" aria-label={eyebrow}>
      <div className="ftux-window-bar">
        <span className="ftux-eyebrow">{eyebrow}</span>
      </div>
      <Stepper steps={steps} current={current} />
      <div className="ftux-body">{children}</div>
    </section>
  );
}

function stepState(index: number, current: number) {
  if (index < current) return "done";
  return index === current ? "current" : "todo";
}

export function Stepper({
  steps,
  current,
}: {
  steps: FtuxStep[];
  current: number;
}) {
  return (
    <div className="ftux-stepper">
      <div className="ftux-stepper-track" aria-hidden="true">
        {steps.map((step, index) => (
          <span key={step.screen} style={{ display: "contents" }}>
            {index > 0 ? (
              <span
                className="ftux-stepper-line"
                data-done={index <= current}
              />
            ) : null}
            <span
              className="ftux-stepper-dot"
              data-state={stepState(index, current)}
            >
              {index < current ? <CheckIcon size={8} /> : null}
            </span>
          </span>
        ))}
      </div>
      <ol
        className="ftux-stepper-labels"
        style={{ listStyle: "none", margin: 0, padding: 0 }}
      >
        {steps.map((step, index) => (
          <li
            key={step.screen}
            data-state={stepState(index, current)}
            aria-current={index === current ? "step" : undefined}
          >
            {step.label}
          </li>
        ))}
      </ol>
    </div>
  );
}

export function ScreenTitle({ light, bold }: { light: string; bold: string }) {
  return (
    <h1 className="ftux-title">
      <span className="ftux-light">{light}</span>
      {bold}
    </h1>
  );
}

// Controls ---------------------------------------------------------------

export type SelectOption<T extends string> = {
  value: T;
  label: string;
  tone?: "green" | "yellow" | "grey";
};

export function PillSelect<T extends string>({
  label,
  value,
  options,
  placeholder,
  onChange,
}: {
  label: string;
  value: T | null;
  options: SelectOption<T>[];
  placeholder?: string;
  onChange: (value: T) => void;
}) {
  const selected = options.find((option) => option.value === value);
  return (
    <span className="ftux-select" data-placeholder={!selected}>
      {selected?.tone ? (
        <span className="ftux-dot" data-tone={selected.tone} />
      ) : null}
      <select
        aria-label={label}
        value={selected ? selected.value : ""}
        onChange={(event) => onChange(event.target.value as T)}
      >
        {selected ? null : (
          <option value="" disabled>
            {placeholder ?? "Choose…"}
          </option>
        )}
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
      <ChevronIcon />
    </span>
  );
}

export function GlassCheckbox({
  checked,
  label,
  disabled,
  onToggle,
}: {
  checked: boolean | "mixed";
  label: string;
  disabled?: boolean;
  onToggle?: () => void;
}) {
  return (
    // biome-ignore lint/a11y/useSemanticElements: the glass box and its mixed state cannot be drawn on a native checkbox.
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      aria-label={label}
      className="ftux-check"
      disabled={disabled}
      onClick={onToggle}
    >
      {checked === true ? <CheckIcon /> : null}
      {checked === "mixed" ? (
        <span
          style={{ width: 7, height: 2, borderRadius: 1, background: "#fff" }}
        />
      ) : null}
    </button>
  );
}

export function GlassSwitch({
  checked,
  label,
  onToggle,
}: {
  checked: boolean;
  label: string;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className="ftux-switch"
      onClick={onToggle}
    />
  );
}

export function Spinner() {
  return <span className="ftux-spinner" aria-hidden="true" />;
}
