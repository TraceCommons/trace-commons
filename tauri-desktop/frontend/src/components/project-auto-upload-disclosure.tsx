import { useContributorDisclosureCopy } from "../lib/tauri/use-contributor-copy";

export function ProjectAutoUploadDisclosure() {
  // The ask-first mode's one name is the core's (`folder_mode_labels`).
  const shared = useContributorDisclosureCopy();
  const askMe = shared.data?.folder_mode_labels.notify_only ?? "";
  return (
    <p className="m-0 tc-label font-normal leading-[17px] tc-text-secondary">
      New eligible sessions from this project can be approved and sent after
      they finish and settle, without another review. Pattern-based scrubbing
      can miss private information. This setting affects this project only;
      return it to {askMe} in Settings to stop automatic contribution.
    </p>
  );
}
