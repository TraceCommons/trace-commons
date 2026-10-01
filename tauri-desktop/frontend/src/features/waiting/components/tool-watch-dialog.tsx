import { useState } from "react";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import { ButtonPrimary, GlassButton } from "../../../design-system";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import { useDirectoryPicker } from "../../../lib/tauri/use-platform-actions";
import { useSourceRoots } from "../../settings/public";
import type { ToolSource } from "../traces-model";

/**
 * The tool switch's confirmation. Turning a tool's watching on or off is the
 * same source declaration Settings' folder panel saves, so it carries the
 * same disclosure: the core's `source_settings` explanation (and the tool's
 * own, when the core has one) is read first, and nothing can be saved until
 * that copy has loaded. A failed save reports the core's line and the error
 * itself; only a closed folder picker reads as "nothing chosen".
 */
export function ToolWatchDialog({
  source,
  next,
  onClose,
}: {
  source: ToolSource;
  /** What the switch asked for: true is watch, false is off. */
  next: boolean;
  onClose: () => void;
}) {
  const disclosure = useContributorDisclosureCopy();
  const roots = useSourceRoots();
  const picker = useDirectoryPicker();
  const [error, setError] = useState<string | null>(null);
  const copy = disclosure.data?.source_settings;
  const tool = Object.values(copy?.tools ?? {}).find(
    (item) => item.key === source.name,
  );
  const busy = roots.busy || picker.isPending;
  const ready = Boolean(copy);

  const confirm = async () => {
    if (!copy) return;
    setError(null);
    let path = "";
    if (next) {
      // The core does not return saved paths, so watching needs a folder.
      try {
        path = await picker.pick("source_root");
      } catch {
        setError("Folder not chosen, so nothing changed.");
        return;
      }
    }
    try {
      await roots.save(source.name, next ? "watch" : "off", path);
      onClose();
    } catch (cause) {
      const detail = cause instanceof Error ? cause.message : String(cause);
      setError(`${copy.save_failed} ${detail}`.trim());
    }
  };

  return (
    <ResponsiveOverlay
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
      title={copy?.heading ?? source.label}
      footer={
        <div className="flex justify-end gap-2">
          <GlassButton type="button" onClick={onClose} disabled={busy}>
            Cancel
          </GlassButton>
          <ButtonPrimary
            size="sm"
            type="button"
            disabled={busy || !ready}
            onClick={() => void confirm()}
          >
            {next
              ? (tool?.choose_folder ?? copy?.choose_folder ?? "Choose folder")
              : (tool?.decline ?? copy?.no_candidate ?? "Turn off")}
          </ButtonPrimary>
        </div>
      }
    >
      <strong className="text-[13px]">{source.label}</strong>
      {copy ? (
        <>
          <p className="m-0 tc-body">{copy.explanation}</p>
          {tool?.explanation ? (
            <p className="m-0 tc-body">{tool.explanation}</p>
          ) : null}
        </>
      ) : disclosure.isError ? (
        <p className="tc-alert m-0" role="alert">
          Source settings copy unavailable. Saving is disabled until it loads.
        </p>
      ) : (
        <p className="m-0 tc-body tc-text-tertiary" role="status">
          Loading source settings disclosure…
        </p>
      )}
      {error ? (
        <p className="tc-alert m-0" role="alert">
          {error}
        </p>
      ) : null}
    </ResponsiveOverlay>
  );
}
