import type { CoreStatus, CoreStatusState } from "../../lib/tauri/types";

export type ProfilePageProps = {
  coreStatus: CoreStatus | null;
  coreStatusState: CoreStatusState;
  onRefresh: () => Promise<void>;
  publicProfile: PublicProfile | null;
  publicProfileState: "loading" | "ready" | "error";
  onPublicProfileRefresh: () => Promise<void>;
};

export type PublicProfile = {
  on_roster: boolean;
  handle: string | null;
  bio: string | null;
  public_since: string | null;
  public_url: string | null;
};
