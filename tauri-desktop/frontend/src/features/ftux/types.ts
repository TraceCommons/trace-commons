import type {
  FtuxPath,
  RepoRule,
  RepoSelection,
  ToolPresence,
  WatchAnswer,
} from "./ftux-model";

export type DetectedTool = {
  id: string;
  badge: string;
  name: string;
  folder: string;
  presence: ToolPresence;
  sessionCount: number;
  detail: string;
  installUrl?: string;
  // Added by the person on the Tools screen rather than found on disk.
  custom?: boolean;
};

export type PastSession = {
  id: string;
  date: string;
  title: string;
  durationMinutes: number | null;
};

export type RepoCandidate = {
  folder: string;
  note?: string;
  defaultRule: RepoRule;
  sessions: PastSession[];
};

export type SharingMode = "auto" | "ask";

export type PasskeyStore = "1password" | "passwords";

export type JoinState = {
  invite: { host: string; payRange: string } | null;
  passkey: { name: string; store: PasskeyStore } | null;
  nearAi: boolean;
};

export type FtuxSettings = {
  path: FtuxPath;
  join: JoinState;
  watch: Record<string, WatchAnswer>;
  customFolders: Record<string, string>;
  repos: RepoSelection[];
  optionalUses: boolean[];
  listHandle: boolean;
  sharing: SharingMode;
  privateAi: boolean;
};
