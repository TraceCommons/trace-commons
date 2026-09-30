type BrandMarkIconProps = { size?: number; monochrome?: boolean };

export function BrandMarkIcon({
  size = 28,
  monochrome = false,
}: BrandMarkIconProps) {
  return (
    <svg
      aria-hidden="true"
      className="block shrink-0"
      width={size}
      height={size}
      viewBox="0 0 64 64"
      fill="none"
    >
      {!monochrome && (
        <rect
          x="1"
          y="1"
          width="62"
          height="62"
          fill="var(--color-mark-field)"
          stroke="var(--color-mark-frame)"
          strokeWidth="2"
        />
      )}
      <path
        d="M11 28V11h17"
        stroke={monochrome ? "currentColor" : "var(--color-green)"}
        strokeWidth="7"
      />
      <path
        d="M53 36v17H36"
        stroke={monochrome ? "currentColor" : "var(--color-blue)"}
        strokeWidth="7"
      />
    </svg>
  );
}
