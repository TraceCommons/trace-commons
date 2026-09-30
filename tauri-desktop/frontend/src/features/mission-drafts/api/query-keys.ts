export const missionDraftKeys = {
  list: ["local", "mission-drafts", "list"] as const,
  detail: (id: string) => ["local", "mission-drafts", "detail", id] as const,
};
