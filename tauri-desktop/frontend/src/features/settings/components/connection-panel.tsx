import type { CoreStatus } from "../../../lib/tauri/types";

type ConnectionPanelProps = {
  status: CoreStatus | null;
  settings: Record<string, unknown> | null;
};

const sources = [
  ["Claude Code", "claude_source_mode"],
  ["Codex", "codex_source_mode"],
  ["Gemini CLI", "gemini_source_mode"],
  ["Cline", "cline_source_mode"],
  ["OpenCode", "opencode_source_mode"],
] as const;

function modeLabel(value: unknown) {
  return value === "watch"
    ? "Folder set"
    : value === "off"
      ? "Off"
      : "Not declared";
}

export function ConnectionPanel({ status, settings }: ConnectionPanelProps) {
  const connected = status?.daemon.logged_in === true;
  return (
    <section className="tc-card mb-4">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            CONNECTION
          </span>
          <h2>{connected ? "Connected" : "Not connected"}</h2>
        </div>
        <span
          className={`tc-chip tc-chip--glass self-start ${connected ? "" : "bg-tc-tint text-tc-secondary"}`}
        >
          {connected ? "Ready" : "Local only"}
        </span>
      </div>
      {!connected && (
        <p className="m-0 tc-caption tc-text-tertiary">
          Sessions may stay queued locally, but nothing can be sent until this
          device is enrolled.
        </p>
      )}
      {connected && (
        <p className="m-0 tc-caption tc-text-tertiary">
          Consent and source declarations come from Rust. This panel reports
          their current state without exposing paths or credentials.
        </p>
      )}
      <div className="mt-3 grid gap-px">
        {sources.map(([label, key]) => (
          <div
            className="flex items-start gap-2.5 border-b border-tc-hairline py-3"
            key={key}
          >
            <span
              className={`mt-px grid h-4 w-4 shrink-0 place-items-center rounded-full border border-tc-hairline text-[10px] text-tc-secondary ${settings?.[key] === "watch" ? "border-tc-purple bg-tc-purple text-tc-on-accent" : ""}`}
              aria-hidden="true"
            >
              {settings?.[key] === "watch" ? "✓" : "–"}
            </span>
            <span>
              <strong>{label} sessions folder</strong>
              <small>{modeLabel(settings?.[key])}</small>
            </span>
          </div>
        ))}
        <div className="flex items-start gap-2.5 border-b border-tc-hairline py-3">
          <span
            className={`mt-px grid h-4 w-4 shrink-0 place-items-center rounded-full border border-tc-hairline text-[10px] text-tc-secondary ${settings?.near_ai_configured === true ? "border-tc-purple bg-tc-purple text-tc-on-accent" : ""}`}
            aria-hidden="true"
          >
            {settings?.near_ai_configured === true ? "✓" : "–"}
          </span>
          <span>
            <strong>Extra privacy scan</strong>
            <small>
              {settings?.near_ai_configured === true
                ? "Configured"
                : "Not configured"}
            </small>
          </span>
        </div>
      </div>
    </section>
  );
}
