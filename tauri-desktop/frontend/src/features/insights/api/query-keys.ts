export const insightsKeys = {
  scope: () => ["local", "insights"] as const,
  data: () => [...insightsKeys.scope(), "data"] as const,
  detail: (id: string) => [...insightsKeys.scope(), "detail", id] as const,
  episodes: () => [...insightsKeys.scope(), "episodes"] as const,
  episode: (id: string) => [...insightsKeys.scope(), "episode", id] as const,
  comparisonTasks: () => [...insightsKeys.scope(), "comparison-tasks"] as const,
  comparisonTask: (id: string) =>
    [...insightsKeys.scope(), "comparison-task", id] as const,
  comparisonSpecifications: () =>
    [...insightsKeys.scope(), "comparison-specifications"] as const,
  comparisonSpecification: (id: string) =>
    [...insightsKeys.scope(), "comparison-specification", id] as const,
};
