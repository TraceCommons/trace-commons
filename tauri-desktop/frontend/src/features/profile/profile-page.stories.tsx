import type { Meta, StoryObj } from "@storybook/react-vite";
import { withQueryClient } from "../../lib/query/storybook-provider";
import { ProfilePage } from "./profile-page";

const meta = {
  title: "Features/Profile/ProfilePage",
  component: ProfilePage,
  decorators: [withQueryClient],
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof ProfilePage>;

export default meta;
type Story = StoryObj<typeof meta>;

const localCoreStatus = {
  state_dir: "storybook-preview",
  startup: "needs_roots" as const,
  daemon: {
    schema_version: "v1",
    logged_in: false,
    tenant_id: null,
    consent_scopes: [],
    paused: false,
    queue_depth: 0,
    health: { last_error_label: null, since: null },
  },
};

const publishedCoreStatus = {
  ...localCoreStatus,
  daemon: {
    ...localCoreStatus.daemon,
    logged_in: true,
    tenant_id: "tenant-story",
  },
};

export const LocalOnly: Story = {
  args: {
    coreStatusState: "ready",
    coreStatus: localCoreStatus,
    onRefresh: async () => {},
    publicProfile: null,
    publicProfileState: "ready",
    onPublicProfileRefresh: async () => {},
  },
};

export const Loading: Story = {
  args: {
    ...LocalOnly.args,
    coreStatus: null,
    coreStatusState: "loading",
  },
};

export const Published: Story = {
  args: {
    ...LocalOnly.args,
    coreStatus: publishedCoreStatus,
    publicProfile: {
      on_roster: true,
      handle: "manian",
      bio: "Ships billing systems by day.",
      public_since: "2026-05-12T00:00:00Z",
      public_url: "https://tracecommons.org/c/manian",
    },
  },
};

export const RustCoreError: Story = {
  args: {
    ...LocalOnly.args,
    coreStatus: null,
    coreStatusState: "error",
  },
};
