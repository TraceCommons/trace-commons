export const title = "Standing consent and queued approval origins";
export const state = (v: any): string => v.s.screen;
export const step = (action: string, _from: any, _to: any): string => {
  const sentence = action.replace(/([a-z0-9])([A-Z])/g, "$1 $2");
  return sentence.charAt(0).toUpperCase() + sentence.slice(1).toLowerCase();
};
