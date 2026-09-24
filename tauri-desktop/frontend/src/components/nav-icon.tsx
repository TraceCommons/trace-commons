import {
  ChartLineUpIcon,
  ClockCounterClockwiseIcon,
  CpuIcon,
  GearIcon,
  ListChecksIcon,
  NoteIcon,
  SparkleIcon,
  UserCircleIcon,
} from "@phosphor-icons/react";

type NavIconProps = {
  name:
    | "insights"
    | "waiting"
    | "history"
    | "compute"
    | "private-ai"
    | "mission-drafts"
    | "profile"
    | "settings";
};

const icons = {
  insights: ChartLineUpIcon,
  waiting: ListChecksIcon,
  history: ClockCounterClockwiseIcon,
  compute: CpuIcon,
  "private-ai": SparkleIcon,
  "mission-drafts": NoteIcon,
  profile: UserCircleIcon,
  settings: GearIcon,
} as const;

export function NavIcon({ name }: NavIconProps) {
  const Icon = icons[name];
  return <Icon aria-hidden="true" className="size-4 shrink-0" weight="regular" />;
}
