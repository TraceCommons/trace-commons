export const title = "Windows, GTK and Tauri platform boundaries";
export const state = (v: any): string => {
  const s = v.s;
  if (!s.alive) return "platform_stopped";
  if (s.quit) return "platform_quit";
  if (!s.visible) return "platform_hidden";
  if (s.screen === "local") return "platform_local";
  if (s.changeOpen) return "platform_change";
  if (["opening", "browser", "verifying"].includes(s.auth)) return "platform_auth";
  if (["starting", "checking"].includes(s.screen)) return "platform_starting";
  if (s.screen === "roots") return "platform_roots";
  if (s.screen === "first_run") return "platform_first_run";
  if (s.screen === "offline") return "platform_offline";
  if (s.screen === "refused") return "platform_refused";
  if (s.screen === "not_found") return "platform_not_found";
  if (s.page === "settings" && s.update !== "idle") return "platform_update";
  if (s.page === "settings") return "platform_settings";
  return "platform_workspace";
};
export const step = (action: string, _from: any, _to: any): string => {
  const words = action.replace(/([a-z0-9])([A-Z])/g, "$1 $2");
  return words.charAt(0).toUpperCase() + words.slice(1).toLowerCase() + ".";
};
