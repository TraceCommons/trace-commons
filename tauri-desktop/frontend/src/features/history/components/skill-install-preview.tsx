import type {
  InstalledSkill,
  SkillCopy,
  SkillInstallPlan,
} from "../skill-types";
import { ButtonPrimary, GlassButton } from "@/design-system";

export function SkillInstallPreview({
  copy,
  plan,
  installed,
  busy,
  onInstall,
  onRollback,
}: {
  copy: SkillCopy;
  plan: SkillInstallPlan | null;
  installed: InstalledSkill | null;
  busy: boolean;
  onInstall: () => void;
  onRollback: () => void;
}) {
  if (installed)
    return (
      <div className="tc-card tc-card--quiet grid gap-4">
        <strong>{copy.installed}</strong>
        <span>
          {installed.name} · {installed.tool}
        </span>
        <code>{installed.target_location}</code>
        <span>
          {copy.digest}: {installed.skill_sha256}
        </span>
        <p className="m-0 tc-caption tc-text-tertiary">
          {copy.rollback_disclosure}
        </p>
        <GlassButton
          className="tc-text-outside"
          type="button"
          onClick={onRollback}
          disabled={busy}
        >
          {busy ? copy.rolling_back : copy.rollback}
        </GlassButton>
      </div>
    );
  if (!plan) return null;
  return (
    <div className="grid gap-4">
      <div className="grid gap-[5px] tc-card tc-card--quiet">
        <strong>{copy.install_preview}</strong>
        <span>{copy.install_disclosure}</span>
      </div>
      <div className="grid gap-0 border-t border-tc-hairline pt-4">
        <Path label={copy.target_path} value={plan.target_location} />
        <Path label={copy.skill_file} value={plan.skill_location} />
        <Path label={copy.ownership_marker} value={plan.marker_location} />
        <Path label={copy.marker_digest} value={plan.marker_file_sha256} />
      </div>
      <pre className="tc-code max-h-[340px] overflow-auto leading-[1.55] whitespace-pre-wrap">
        {plan.marker_json}
      </pre>
      <div className="mt-3 flex flex-wrap gap-2">
        <ButtonPrimary size="sm"
          type="button"
          onClick={onInstall}
          disabled={busy || !plan.can_install || plan.occupied}
        >
          {busy ? copy.installing : copy.install_action}
        </ButtonPrimary>
      </div>
      {plan.occupied && (
        <p className="tc-alert">
          Target path is occupied. Installation refused.
        </p>
      )}
    </div>
  );
}

function Path({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <span>{label}</span>
      <code>{value}</code>
    </div>
  );
}
