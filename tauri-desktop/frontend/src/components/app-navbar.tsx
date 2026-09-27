import { NavLink, useLocation } from "react-router-dom";
import { navItems, routePaths } from "../app/routes";
import { BrandMarkIcon } from "./brand-mark-icon";
import { NavIcon } from "./nav-icon";
import { ThemeSelector } from "./theme-selector";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
} from "./ui/sidebar";

type NavbarProfile = { on_roster: boolean; handle: string | null };
type AppNavbarProps = {
  queueCount: number;
  profile: NavbarProfile | null;
  profileState: "loading" | "ready" | "error";
};

export function AppNavbar({
  queueCount,
  profile,
  profileState,
}: AppNavbarProps) {
  const published = profileState === "ready" && profile?.on_roster === true;
  return (
    <Sidebar collapsible="offcanvas">
      <SidebarHeader className="p-4">
        <div className="flex items-center gap-2.5 px-2 py-2">
        <BrandMarkIcon size={24} />
        <div>
          <span className="text-sm font-semibold tracking-tight">
            Trace Commons
          </span>
          <span className="block text-xs text-muted-foreground">
            Contributor workspace
          </span>
        </div>
        </div>
      </SidebarHeader>

      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupLabel>Workspace</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {navItems
                .filter((item) => item.group === "workspace")
                .map((item) => (
                  <NavButton
                    key={item.id}
                    item={item}
                    count={item.id === "waiting" ? queueCount : item.count}
                  />
                ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
        <SidebarGroup>
          <SidebarGroupLabel>Account</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {navItems
                .filter((item) => item.group === "account")
                .map((item) => (
                  <NavButton key={item.id} item={item} count={item.count} />
                ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>

      <SidebarFooter className="p-4">
        <SidebarMenu className="mb-3">
          <ThemeSelector />
        </SidebarMenu>
        <div className="flex items-center gap-2.5 border-t border-sidebar-border px-2 pt-4">
        <div
          className="grid size-8 place-items-center rounded-full bg-sidebar-primary text-xs font-bold text-sidebar-primary-foreground"
          aria-hidden="true"
        >
          TC
        </div>
        <div>
          <strong className="block text-xs font-medium">
            {published ? `@${profile?.handle ?? "unnamed"}` : "Local profile"}
          </strong>
          <span className="block text-xs text-muted-foreground">
            {published ? "Public roster" : "Not published"}
          </span>
        </div>
        </div>
      </SidebarFooter>
    </Sidebar>
  );
}

type NavButtonProps = {
  item: (typeof navItems)[number];
  count?: number;
};

function NavButton({ item, count }: NavButtonProps) {
  const { pathname } = useLocation();
  const isActive = pathname === routePaths[item.id];
  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        render={<NavLink to={routePaths[item.id]} end />}
        isActive={isActive}
        tooltip={item.label}
      >
        <NavIcon name={item.icon} />
        <span>{item.label}</span>
      </SidebarMenuButton>
      {count !== undefined && <SidebarMenuBadge>{count}</SidebarMenuBadge>}
    </SidebarMenuItem>
  );
}
