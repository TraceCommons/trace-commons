export const routePaths = {
  insights: "/insights",
  waiting: "/waiting",
  history: "/history",
  compute: "/compute",
  "private-ai": "/private-ai",
  "mission-drafts": "/mission-drafts",
  profile: "/profile",
  settings: "/settings",
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
  if (pathname === "/") return "insights";
  return routeIds.find((routeId) => routePaths[routeId] === pathname) ?? null;
}

export type NavItem = {
  id: RouteId;
  label: string;
  group: "workspace" | "account";
  icon:
    | "insights"
    | "waiting"
    | "history"
    | "compute"
    | "private-ai"
    | "mission-drafts"
    | "profile"
    | "settings";
  count?: number;
};

export const navItems: NavItem[] = [
  {
    id: "waiting",
    label: "Waiting",
    group: "workspace",
    icon: "waiting",
    count: 0,
  },
  { id: "history", label: "History", group: "workspace", icon: "history" },
  { id: "compute", label: "Compute", group: "workspace", icon: "compute" },
  {
    id: "private-ai",
    label: "Private AI",
    group: "workspace",
    icon: "private-ai",
  },
  { id: "insights", label: "Insights", group: "workspace", icon: "insights" },
  {
    id: "mission-drafts",
    label: "Mission drafts",
    group: "workspace",
    icon: "mission-drafts",
  },
  { id: "profile", label: "Profile", group: "account", icon: "profile" },
  { id: "settings", label: "Settings", group: "account", icon: "settings" },
];

/**
 * The navigation rows to show. Missions are hidden while the daemon's
 * `missions_enabled` setting is off, so the Mission drafts row goes with it.
 */
export function visibleNavItems(missionsEnabled: boolean): NavItem[] {
  return navItems.filter((item) => missionsEnabled || item.id !== "mission-drafts");
}

/** Whether a path from a desktop event may be navigated to. */
export function isNavigablePath(
  value: unknown,
  missionsEnabled: boolean,
): value is (typeof routePaths)[RouteId] {
  return (
    typeof value === "string" &&
    (missionsEnabled || value !== routePaths["mission-drafts"]) &&
    Object.values(routePaths).some((path) => path === value)
  );
}
