/** Semantic status colours. Status is a glyph + label, never a fill. */
export const statusColor = {
  on: "var(--tc-status-on)",
  ask: "var(--tc-status-ask)",
  off: "var(--tc-status-off)",
  outside: "var(--tc-status-outside)",
  shared: "var(--tc-data-shared)",
  kept: "var(--tc-data-kept)",
  inference: "var(--tc-data-inference)",
} as const;

export type StatusTone = keyof typeof statusColor;
