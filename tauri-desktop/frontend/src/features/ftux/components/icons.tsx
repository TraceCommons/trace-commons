// Icons the design system does not carry. Paths are the WYSIWYG design's.
import type { ReactNode } from "react";

type IconProps = { size?: number };

function Svg({
  size,
  strokeWidth = 1.8,
  children,
}: {
  size: number;
  strokeWidth?: number;
  children: ReactNode;
}) {
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
      {children}
    </svg>
  );
}

export function CheckIcon({ size = 10 }: IconProps) {
  return (
    <Svg size={size} strokeWidth={3.5}>
      <path d="M5 12l5 5 9-10" />
    </Svg>
  );
}

export function DownloadIcon() {
  return (
    <Svg size={12} strokeWidth={2}>
      <path d="M12 4v11M7 10l5 5 5-5M4 20h16" />
    </Svg>
  );
}

export function ExternalIcon() {
  return (
    <Svg size={11} strokeWidth={2.4}>
      <path d="M7 17L17 7M9 7h8v8" />
    </Svg>
  );
}

export function BackIcon() {
  return (
    <Svg size={12} strokeWidth={2.2}>
      <path d="M15 5l-7 7 7 7" />
    </Svg>
  );
}

export function CloseIcon({ size = 12 }: IconProps) {
  return (
    <Svg size={size} strokeWidth={2.2}>
      <path d="M6 6l12 12M18 6L6 18" />
    </Svg>
  );
}

export function PasskeyIcon() {
  return (
    <Svg size={24}>
      <circle cx="9" cy="8" r="3.5" />
      <path d="M3 20a6 6 0 0 1 12 0" />
      <circle cx="18" cy="9" r="2.2" />
      <path d="M18 11.2V17l1.5 1.5M18 14.5h1.6" />
    </Svg>
  );
}

export function LockIcon() {
  return (
    <Svg size={24}>
      <rect x="5" y="10" width="14" height="10" rx="2.5" />
      <path d="M8 10V7.5a4 4 0 0 1 8 0V10" />
    </Svg>
  );
}

export function WarningIcon() {
  return (
    <Svg size={16}>
      <path d="M12 9v4M12 17h.01M10.3 3.9L2.5 18a2 2 0 0 0 1.7 3h15.6a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z" />
    </Svg>
  );
}

export function FingerprintIcon({
  size = 44,
  strokeWidth = 1.3,
}: IconProps & { strokeWidth?: number }) {
  return (
    <Svg size={size} strokeWidth={strokeWidth}>
      <path d="M7 8.5a6 6 0 0 1 10 0M5.5 12a8 8 0 0 1 13 0M9 12a3.5 3.5 0 0 1 6 1.5c0 2.5-.8 4.5-2 6.5M12 12v2.5c0 2.3-.7 4.2-1.8 5.8M15 16.8c-.3 1.4-.9 2.6-1.7 3.7" />
    </Svg>
  );
}

export function Spinner() {
  return <span className="ftux-spinner" aria-hidden="true" />;
}
