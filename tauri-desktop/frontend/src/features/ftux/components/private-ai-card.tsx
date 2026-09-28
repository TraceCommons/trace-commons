import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import { GlassSwitch } from "./glass";

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
    <div className="ftux-card">
      <div className="ftux-card-row" style={{ alignItems: "flex-start" }}>
        <span style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <span className="ftux-card-title">
            {copy ? copy.destination : " "}
          </span>
          {copy ? (
            <>
              <span className="ftux-card-text">{copy.offer_what}</span>
              <span className="ftux-card-text">{copy.offer_exposure}</span>
              <span className="ftux-card-text">{copy.offer_no_repoint}</span>
            </>
          ) : (
            <span className="ftux-card-text">
              {disclosure.isError
                ? "Disclosure unavailable. Enabling is disabled."
                : "Loading disclosure…"}
            </span>
          )}
        </span>
        <GlassSwitch
          checked={checked && copy !== undefined}
          label={copy?.offer_title ?? "Loading disclosure"}
          disabled={!copy}
          onToggle={onToggle}
        />
      </div>
    </div>
  );
}
