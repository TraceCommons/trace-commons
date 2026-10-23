import { LOGO_PATHS, TOOL_TINT, type ToolLogoId } from "./logo-paths";

export function ToolLogo({
  id,
  size = 15,
  tint,
}: {
  id: ToolLogoId;
  size?: number;
  tint?: string;
}) {
  const logo = LOGO_PATHS[id];
  return (
    <svg
      width={size}
      height={size}
      viewBox={logo.vb}
      fill={tint ?? TOOL_TINT[id]}
      style={{ display: "block" }}
      aria-hidden="true"
    >
      {logo.d.map((d) => (
        <path key={d.slice(0, 24)} d={d} />
      ))}
    </svg>
  );
}

/** Logo positioned inside an SVG (the flow map). */
export function ToolLogoSvg({
  id,
  x,
  y,
  size = 16,
}: {
  id: ToolLogoId;
  x: number;
  y: number;
  size?: number;
}) {
  const logo = LOGO_PATHS[id];
  return (
    <svg
      x={x}
      y={y}
      width={size}
      height={size}
      viewBox={logo.vb}
      fill={TOOL_TINT[id]}
      style={{ pointerEvents: "none" }}
      aria-hidden="true"
    >
      {logo.d.map((d) => (
        <path key={d.slice(0, 24)} d={d} />
      ))}
    </svg>
  );
}
