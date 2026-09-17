export type CoreStatus = {
  prototype: boolean;
  state_dir: string;
  daemon: {
    schema_version: string;
    logged_in: boolean;
    tenant_id: string | null;
    consent_scopes: string[];
    paused: boolean;
    queue_depth: number;
    daily_budget?: {
      bytes_today: number;
      max_bytes_per_day: number;
      bytes_remaining: number;
      uploads_today: number;
      max_uploads_per_day: number;
      uploads_remaining: number;
      blocked: boolean;
      blocked_entries: number;
      blocked_bytes: number;
    };
    routing?: {
      state: string;
      derived: boolean;
      last_refresh_at: string | null;
      unreadable_rows: number;
    };
    health: {
      last_error_label: string | null;
      since: string | null;
    };
  };
};

export type CoreStatusState = "loading" | "ready" | "error";
