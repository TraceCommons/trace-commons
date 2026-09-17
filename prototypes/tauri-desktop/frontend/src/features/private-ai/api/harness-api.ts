import { invokeTauri } from "../../../lib/tauri/core-api";

export type HarnessRow = {
  id: string;
  name: string;
  installed: boolean;
  connected: boolean;
  config_path: string | null;
  connect_command: string;
  state: string;
  can_connect: boolean;
  can_disconnect: boolean;
  state_line: string;
};

export type HarnessList = {
  harnesses: HarnessRow[];
  view: {
    title: string;
    what: string;
    spend_scope: string;
    none_found: string;
    credential_notice: string;
    spend_line: string;
    state_lines: Record<string, string>;
  };
};

export type HarnessPlan = {
  id: string;
  action: string;
  outcome: string;
  plan_id: string | null;
  path: string | null;
  changes: string[];
  occupied: Array<{ slot: string; current: string }>;
  view: {
    preview_title: string;
    confirm: string;
    cancel: string;
    slot_taken: string;
    outcome_line: string;
    can_commit: boolean;
  };
};

function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error("Invalid harness response");
  return value as Record<string, unknown>;
}

function string(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "string")
    throw new Error(`Invalid harness field: ${key}`);
  return value[key] as string;
}

function nullableString(value: Record<string, unknown>, key: string) {
  if (value[key] !== null && typeof value[key] !== "string")
    throw new Error(`Invalid harness field: ${key}`);
  return value[key] as string | null;
}

function boolean(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "boolean")
    throw new Error(`Invalid harness field: ${key}`);
  return value[key] as boolean;
}

export async function getHarnessList(): Promise<HarnessList> {
  const value = record(await invokeTauri<unknown>("private_ai_harnesses"));
  if (!Array.isArray(value.harnesses)) throw new Error("Invalid harness rows");
  const view = record(value.view);
  const stateLines = record(view.state_lines);
  if (Object.values(stateLines).some((line) => typeof line !== "string"))
    throw new Error("Invalid harness state lines");
  return {
    harnesses: value.harnesses.map((raw) => {
      const row = record(raw);
      const id = string(row, "id");
      return {
        id,
        name: string(row, "name"),
        installed: boolean(row, "installed"),
        connected: boolean(row, "connected"),
        config_path: nullableString(row, "config_path"),
        connect_command: string(row, "connect_command"),
        state: string(row, "state"),
        can_connect: boolean(row, "can_connect"),
        can_disconnect: boolean(row, "can_disconnect"),
        state_line: typeof stateLines[id] === "string" ? stateLines[id] : "",
      };
    }),
    view: {
      title: string(view, "title"),
      what: string(view, "what"),
      spend_scope: string(view, "spend_scope"),
      none_found: string(view, "none_found"),
      credential_notice: string(view, "credential_notice"),
      spend_line: string(view, "spend_line"),
      state_lines: Object.fromEntries(
        Object.entries(stateLines).map(([id, line]) => [id, line as string]),
      ),
    },
  };
}

export async function planHarness(
  id: string,
  action: "connect" | "disconnect",
): Promise<HarnessPlan> {
  const value = record(
    await invokeTauri<unknown>("plan_harness", { id, action }),
  );
  const view = record(value.view);
  if (
    !Array.isArray(value.changes) ||
    !value.changes.every((item) => typeof item === "string") ||
    !Array.isArray(value.occupied)
  )
    throw new Error("Invalid harness plan");
  return {
    id: string(value, "id"),
    action: string(value, "action"),
    outcome: string(value, "outcome"),
    plan_id: nullableString(value, "plan_id"),
    path: nullableString(value, "path"),
    changes: value.changes as string[],
    occupied: value.occupied.map((item) => {
      const row = record(item);
      return { slot: string(row, "slot"), current: string(row, "current") };
    }),
    view: {
      preview_title: string(view, "preview_title"),
      confirm: string(view, "confirm"),
      cancel: string(view, "cancel"),
      slot_taken: string(view, "slot_taken"),
      outcome_line: string(view, "outcome_line"),
      can_commit: boolean(view, "can_commit"),
    },
  };
}

export async function commitHarness(planId: string) {
  return invokeTauri<unknown>("commit_harness", { planId });
}
