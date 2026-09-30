/**
 * Paths the app answers. The native side sends some of them (the tray's
 * Settings and Private AI items, a notification's `/waiting`), so they stay
 * stable; the Monitor maps each onto a tab, a Home sub-view or the Settings
 * modal (see `monitorViewFromPath`).
 */
export const routePaths = {
  home: "/home",
  waiting: "/waiting",
  history: "/history",
  "private-ai": "/private-ai",
  "mission-drafts": "/mission-drafts",
  profile: "/profile",
  settings: "/settings",
  // Kept so a deep link or tray path to them still lands somewhere: the
  // Monitor design has no place for these views, so they open Home.
  insights: "/insights",
  compute: "/compute",
} as const;

export type RouteId = keyof typeof routePaths;

/**
 * Flows reached from inside the app, never from the navigation or a deep
 * link, so they are kept out of `routePaths`. `automatic-contributing` is
 * the Flow 1 grant screens again (K10), opened from Settings and from the
 * grant's void notice.
 */
export const flowPaths = {
  "automatic-contributing": "/automatic-contributing",
} as const;

const routeIds = Object.keys(routePaths) as RouteId[];

export function routeIdFromPath(pathname: string): RouteId | null {
  if (pathname === "/") return "home";
  return routeIds.find((routeId) => routePaths[routeId] === pathname) ?? null;
}

/** The Monitor's tabs: Home · Inference · Traces. */
export type MonitorTab = "home" | "inference" | "traces";

/** What the left pane shows. Missions and History are Home sub-views. */
export type MonitorView = "home" | "missions" | "history" | "inference" | "traces";

export function monitorViewFromPath(pathname: string): MonitorView {
  switch (routeIdFromPath(pathname)) {
    case "waiting":
      return "traces";
    case "private-ai":
      return "inference";
    case "mission-drafts":
      return "missions";
    case "history":
      return "history";
    default:
      return "home";
  }
}

export function monitorTabFromView(view: MonitorView): MonitorTab {
  if (view === "traces") return "traces";
  if (view === "inference") return "inference";
  return "home";
}

export const viewPaths: Record<MonitorView, string> = {
  home: routePaths.home,
  missions: routePaths["mission-drafts"],
  history: routePaths.history,
  inference: routePaths["private-ai"],
  traces: routePaths.waiting,
};

/** Paths that open the Settings modal, and the section each lands on. */
export function settingsSectionFromPath(pathname: string): string | null {
  const route = routeIdFromPath(pathname);
  if (route === "settings") return "connection";
  if (route === "profile") return "profile";
  return null;
}
