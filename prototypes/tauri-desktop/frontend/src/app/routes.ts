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

const routeIds = Object.keys(routePaths) as RouteId[];

export function routeIdFromPath(pathname: string): RouteId {
  return (
    routeIds.find((routeId) => routePaths[routeId] === pathname) ?? "insights"
  );
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
