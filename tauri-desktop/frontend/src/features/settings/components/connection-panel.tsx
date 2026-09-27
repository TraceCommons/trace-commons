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
    <section className="rounded-2xl border border-border bg-card/80 p-[26px] mb-4">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            CONNECTION
          </span>
          <h2>{connected ? "Connected" : "Not connected"}</h2>
        </div>
        <span
          className={`whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start ${connected ? "" : "bg-muted text-muted-foreground"}`}
        >
          {connected ? "Ready" : "Local only"}
        </span>
      </div>
      {!connected && (
        <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
          Sessions may stay queued locally, but nothing can be sent until this
          device is enrolled.
        </p>
      )}
      {connected && (
        <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
          Consent and source declarations come from Rust. This panel reports
          their current state without exposing paths or credentials.
        </p>
      )}
      <div className="mt-5 grid gap-px border-t border-border">
        {sources.map(([label, key]) => (
          <div
            className="flex items-start gap-2.5 border-b border-border py-3"
            key={key}
          >
            <span
              className={`mt-px grid h-4 w-4 shrink-0 place-items-center rounded-full border border-input text-[10px] text-muted-foreground ${settings?.[key] === "watch" ? "border-primary bg-primary text-primary-foreground" : ""}`}
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
        <div className="flex items-start gap-2.5 border-b border-border py-3">
          <span
            className={`mt-px grid h-4 w-4 shrink-0 place-items-center rounded-full border border-input text-[10px] text-muted-foreground ${settings?.near_ai_configured === true ? "border-primary bg-primary text-primary-foreground" : ""}`}
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
