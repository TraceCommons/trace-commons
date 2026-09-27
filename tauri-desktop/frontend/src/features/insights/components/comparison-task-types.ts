import type { ComparisonContext, ComparisonTaskDetail } from "../comparisons";

export type ComparisonTasksApi = {
  tasks: ComparisonTaskDetail[];
  detail: ComparisonTaskDetail | null;
  state: "loading" | "ready" | "busy" | "error";
  error: string | null;
  open: (id: string) => void;
  close: () => void;
  create: (ids: string[]) => Promise<boolean>;
  replaceEpisodes: (ids: string[]) => Promise<boolean>;
  setContext: (context: ComparisonContext) => Promise<boolean>;
  setOutcome: (value: string) => Promise<boolean>;
  clearOutcome: () => Promise<boolean>;
  reconfirm: () => Promise<boolean>;
};

export type SelectionField = {
  value: string[];
  onChange: (value: string[]) => void;
};
