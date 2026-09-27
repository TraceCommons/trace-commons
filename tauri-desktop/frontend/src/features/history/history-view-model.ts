import type { HistoryFilter } from "./components/history-filter";
import type { HistoryRecord } from "./types";

export type HistoryProjectGroup = {
  id: string;
  label: string;
  records: HistoryRecord[];
};

const historyStatuses: Exclude<HistoryFilter, "all">[] = [
  "accepted",
  "submitted",
  "quarantined",
  "withdrawn",
];

export function countHistory(
  records: HistoryRecord[],
): Partial<Record<HistoryFilter, number>> {
  return records.reduce(
    (result, record) => {
      result.all = (result.all ?? 0) + 1;
      const status = historyStatuses.find(
        (candidate) => candidate === record.status,
      );
      if (status) {
        result[status] = (result[status] ?? 0) + 1;
      }
      return result;
    },
    {} as Partial<Record<HistoryFilter, number>>,
  );
}

export function filterHistory(
  records: HistoryRecord[],
  filter: HistoryFilter,
): HistoryRecord[] {
  return filter === "all"
    ? records
    : records.filter((record) => record.status === filter);
}

export function groupHistory(records: HistoryRecord[]): HistoryProjectGroup[] {
  return Array.from(
    records
      .reduce((groups, record) => {
        const group = groups.get(record.project_id) ?? {
          id: record.project_id,
          label: record.project_label,
          records: [],
        };
        group.records.push(record);
        groups.set(record.project_id, group);
        return groups;
      }, new Map<string, HistoryProjectGroup>())
      .values(),
  );
}

export function quarantineExplanations(
  records: HistoryRecord[],
  heldRowFallback?: string | null,
): string[] {
  const explanations = Array.from(
    new Set(
      records
        .filter((record) => record.status === "quarantined")
        .flatMap((record) => contributorFacingExplanations(record.explanations)),
    ),
  );
  return explanations.length > 0 || !heldRowFallback
    ? explanations
    : [heldRowFallback];
}

export function contributorFacingExplanations(explanations: string[]) {
  return explanations.filter((explanation) => !explanation.includes("sha256:"));
}

/**
 * The credit figure under a history row, or null when there is none to state.
 *
 * Same shape as the GTK (`ui/history.rs` `credit_line`) and Windows
 * (`HistoryCopy.CreditLine`) shells: a settled figure stands alone, a figure
 * still moving says so, and a row with no credit says nothing rather than a
 * confident zero. A zero final beside a pending figure is not a settlement,
 * so the pending figure is what gets stated.
 */
export function historyCreditLine(
  final: number | null | undefined,
  pending: number | null | undefined,
): string | null {
  if (typeof final === "number" && Number.isFinite(final) && final > 0) {
    return `credit ${final.toFixed(1)}`;
  }
  if (typeof pending === "number" && Number.isFinite(pending) && pending > 0) {
    return `credit ${pending.toFixed(1)}, still being scored`;
  }
  return null;
}
