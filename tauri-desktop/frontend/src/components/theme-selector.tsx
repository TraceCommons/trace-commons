import { MonitorIcon, MoonIcon, SunIcon } from "@phosphor-icons/react";
import { type Theme, useTheme } from "./theme-provider";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "./ui/dropdown-menu";
import { SidebarMenuButton, SidebarMenuItem } from "./ui/sidebar";

const themeOptions: Array<{ value: Theme; label: string }> = [
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
  { value: "system", label: "System" },
];

function ThemeIcon({ theme }: { theme: Theme }) {
  if (theme === "light") {
    return <SunIcon aria-hidden="true" className="size-4 shrink-0" />;
  }

  if (theme === "dark") {
    return <MoonIcon aria-hidden="true" className="size-4 shrink-0" />;
  }

  return <MonitorIcon aria-hidden="true" className="size-4 shrink-0" />;
}

export function ThemeSelector() {
  const { theme, setTheme } = useTheme();
  const selectedLabel =
    themeOptions.find((option) => option.value === theme)?.label ?? "System";

  return (
    <SidebarMenuItem>
      <DropdownMenu>
        <DropdownMenuTrigger render={<SidebarMenuButton aria-label="Theme" />}>
          <ThemeIcon theme={theme} />
          <span>Theme</span>
          <span className="ml-auto text-xs text-sidebar-foreground/60 group-data-[collapsible=icon]:hidden">
            {selectedLabel}
          </span>
        </DropdownMenuTrigger>
        <DropdownMenuContent side="right" align="end" className="min-w-40">
          <DropdownMenuGroup>
            <DropdownMenuLabel>Appearance</DropdownMenuLabel>
            <DropdownMenuRadioGroup
              value={theme}
              onValueChange={(value) => {
                if (
                  value === "light" ||
                  value === "dark" ||
                  value === "system"
                ) {
                  setTheme(value);
                }
              }}
            >
              {themeOptions.map((option) => (
                <DropdownMenuRadioItem key={option.value} value={option.value}>
                  <ThemeIcon theme={option.value} />
                  {option.label}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenu>
    </SidebarMenuItem>
  );
}
