// Chart view for enterprise_seat.qnt. Quint gives a sum type value as { Tag: payload }.
import { stepLabels } from "./labels.mts";
const tag = (value: object) => Object.keys(value)[0];

export const title = "Enterprise Near AI Gateway: seats in an org";

type Mac = { screen: object; session: boolean; trace: object; holder: object; mode: object };
type Org = { status: object; sharing: boolean };
type Srv = { seat: object; job: object };

const macLabel = (m: Mac) => {
  switch (tag(m.screen)) {
    case "Personal": return "Personal";
    case "Joining": return "Joining your org";
    case "Managed": return m.session ? "Managed" : "Managed, signed out";
    case "Window": return "Removed, can withdraw";
    default: return "Removed";
  }
};

const serverLabel = (s: Srv) =>
  ({ NotEnrolled: "Not enrolled", Enrolled: "Enrolled", Revoked: "Revoked" })[tag(s.seat)] ?? tag(s.seat);

const orgLabel = (o: Org) => {
  const s = tag(o.status);
  if (s === "Active") return o.sharing ? "Active" : "Active, sharing off";
  return s;
};

const traceLabel = (m: Mac) => {
  const toOrg = tag(m.holder) === "OrgAccount";
  switch (tag(m.trace)) {
    case "NoTrace": return "No trace yet";
    case "Waiting": return "Held on your Mac";
    case "Claiming": return toOrg ? "Asking to send, for the org" : "Asking to send, for you";
    case "Sending": return toOrg ? "Sending, for the org" : "Sending, for you";
    case "Credited": return toOrg ? "Credited to the org" : "Credited to you";
    default: return "Withdrawn";
  }
};

export const state = (v: { mac: Mac; org: Org; srv: Srv }) => ({
  "Your Mac": macLabel(v.mac),
  "Your latest trace": traceLabel(v.mac),
  Server: serverLabel(v.srv),
  Org: orgLabel(v.org),
});

export const step = (action: string) => stepLabels[action] ?? action;

export const group = (v: { mac: Mac }) => {
  const s = tag(v.mac.screen);
  return s === "Personal" || s === "Joining" ? "personal" : s === "Managed" ? "org" : "leaving";
};
export const colors = { personal: "blue", org: "teal", leaving: "tan" };
