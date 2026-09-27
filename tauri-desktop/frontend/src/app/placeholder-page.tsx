import type { RouteId } from "./routes";

type PlaceholderPageProps = { route: RouteId };

const labels: Record<RouteId, string> = {
  insights: "Insights",
  waiting: "Waiting",
  history: "History",
  compute: "Compute",
  "private-ai": "Private AI",
  "mission-drafts": "Mission drafts",
  profile: "Profile",
  settings: "Settings",
};

export function PlaceholderPage({ route }: PlaceholderPageProps) {
  return (
    <section className="mx-auto max-w-[720px] px-16 py-[120px]">
      <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
        NEXT PHASE
      </span>
      <h1>{labels[route]}</h1>
      <p>
        This surface stays behind its own feature boundary until its Rust
        contract and states are accepted.
      </p>
    </section>
  );
}
