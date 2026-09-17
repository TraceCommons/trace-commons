import { Button } from "@/components/ui/button";
import type {
  InstalledSkill,
  SkillCopy,
  SkillInstallPlan,
} from "../skill-types";

export function SkillInstallPreview({
  copy,
  plan,
  installed,
  busy,
  onInstall,
  onRollback,
}: {
  copy: SkillCopy;
  plan: SkillInstallPlan;
  installed: InstalledSkill | null;
  busy: boolean;
  onInstall: () => void;
  onRollback: () => void;
}) {
  if (installed)
    return (
      <div className="grid gap-4 border-l-[3px] border-primary bg-primary/10 p-3.5">
        <strong>{copy.installed}</strong>
        <span>
          {installed.name} · {installed.tool}
        </span>
        <code>{installed.target_location}</code>
        <span>
          {copy.digest}: {installed.skill_sha256}
        </span>
        <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
          {copy.rollback_disclosure}
        </p>
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary text-destructive"
          type="button"
          onClick={onRollback}
          disabled={busy}
        >
          {busy ? copy.rolling_back : copy.rollback}
        </Button>
      </div>
    );
  return (
    <div className="grid gap-4">
      <div className="grid gap-[5px] border-l-[3px] border-chart-2 bg-background p-3.5">
        <strong>{copy.install_preview}</strong>
        <span>{copy.install_disclosure}</span>
      </div>
      <div className="grid gap-0 border-t border-border pt-4">
        <Path label={copy.target_path} value={plan.target_location} />
        <Path label={copy.skill_file} value={plan.skill_location} />
        <Path label={copy.ownership_marker} value={plan.marker_location} />
        <Path label={copy.marker_digest} value={plan.marker_file_sha256} />
      </div>
      <pre className="max-h-[340px] overflow-auto rounded-[9px] border border-border bg-muted p-4 font-mono text-[11px] leading-[1.55] text-foreground whitespace-pre-wrap">
        {plan.marker_json}
      </pre>
      <div className="mt-6 flex gap-2.5">
        <Button
          className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
          type="button"
          onClick={onInstall}
          disabled={busy || !plan.can_install || plan.occupied}
        >
          {busy ? copy.installing : copy.install_action}
        </Button>
      </div>
      {plan.occupied && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
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
