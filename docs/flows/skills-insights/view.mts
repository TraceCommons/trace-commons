export const title = "Skills — inventory, usage and detail";
export const state = (v: any) => !v.featureOn ? "Skills disabled" : v.scan !== "idle" ? "Scanning inventory" : v.inventory === "none" ? "Inventory pending" : v.inventory === "empty" ? "No skills found" : v.inventory === "partial" ? "Inventory incomplete" : "Inventory available";
export const group = (v: any) => !v.featureOn ? "disabled" : v.scan !== "idle" ? "busy" : "ready";
export const colors = { disabled: "tan", busy: "yellow", ready: "teal" };
export const step = (action: string) => action.replace(/([a-z])([A-Z])/g, "$1 $2").toLowerCase();
