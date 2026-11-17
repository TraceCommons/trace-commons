export const title = "Provider accounts, managed launches and configuration";
export const state = (v: any): string => v.s.screen;
export const step = (action: string, _from: any, _to: any): string => {
  const words = action.replace(/([a-z0-9])([A-Z])/g, "$1 $2");
  return words.charAt(0).toUpperCase() + words.slice(1).toLowerCase();
};
