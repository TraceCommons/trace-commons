import { z } from "zod";

export const coreStatusSchema = z.object({
  state_dir: z.string(),
  startup: z.enum(["running", "needs_roots", "daemon_unavailable"]),
  daemon: z.object({
    schema_version: z.string(),
    logged_in: z.boolean(),
    tenant_id: z.string().nullable(),
    consent_scopes: z.array(z.string()),
    paused: z.boolean(),
    queue_depth: z.number(),
    daily_budget: z
      .object({
        bytes_today: z.number(),
        max_bytes_per_day: z.number(),
        bytes_remaining: z.number(),
        uploads_today: z.number(),
        max_uploads_per_day: z.number(),
        uploads_remaining: z.number(),
        blocked: z.boolean(),
        blocked_entries: z.number(),
        blocked_bytes: z.number(),
      })
      .optional(),
    routing: z
      .object({
        state: z.string(),
        derived: z.boolean(),
        last_refresh_at: z.string().nullable(),
        unreadable_rows: z.number(),
      })
      .optional(),
    health: z.object({
      last_error_label: z.string().nullable(),
      since: z.string().nullable(),
    }),
  }),
});

export type CoreStatus = z.infer<typeof coreStatusSchema>;

export type CoreStatusState = "loading" | "ready" | "error";
