import type { HistoryFilter } from "./components/history-filter";
import type { HistoryRecord } from "./types";

export type HistoryProjectGroup = {
  id: string;
  label: string;
  records: HistoryRecord[];
};

export function countHistory(
  records: HistoryRecord[],
): Partial<Record<HistoryFilter, number>> {
  return records.reduce(
    (result, record) => {
      result.all = (result.all ?? 0) + 1;
      if (record.status in result) {
        const status = record.status as HistoryFilter;
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

export function quarantineExplanations(records: HistoryRecord[]): string[] {
  return Array.from(
    new Set(
      records
        .filter((record) => record.status === "quarantined")
        .flatMap((record) => record.explanations),
    ),
  );
}
