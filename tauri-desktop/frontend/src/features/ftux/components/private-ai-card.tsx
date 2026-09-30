import { Card, Toggle } from "../../../design-system";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";

// Every word on this card comes from the contributor core's shared copy, the
// same source the Private AI offer on the Waiting screen renders. The switch
// stays off and disabled until that copy has loaded, so nobody turns it on
// without reading what it exposes.
export function PrivateAiCard({
  checked,
  onToggle,
}: {
  checked: boolean;
  onToggle: () => void;
}) {
  const disclosure = useContributorDisclosureCopy();
  const copy = disclosure.data?.private_inference;
  return (
    <Card>
      <div className="ftux-row ftux-row--between ftux-row--top">
        <span className="tc-stack ftux-gap-2">
          <span className="tc-body-strong">
            {copy ? copy.destination : " "}
          </span>
          {copy ? (
            <>
              <span className="tc-label tc-text-secondary">
                {copy.offer_what}
              </span>
              <span className="tc-label tc-text-secondary">
                {copy.offer_exposure}
              </span>
              <span className="tc-label tc-text-secondary">
                {copy.offer_no_repoint}
              </span>
            </>
          ) : (
            <span className="tc-label tc-text-secondary">
              {disclosure.isError
                ? "Disclosure unavailable. Enabling is disabled."
                : "Loading disclosure…"}
            </span>
          )}
        </span>
        <Toggle
          settings
          checked={checked && copy !== undefined}
          label={copy?.offer_title ?? "Loading disclosure"}
          disabled={!copy}
          onChange={() => onToggle()}
        />
      </div>
    </Card>
  );
}
