import { Button } from "@/components/ui/button";
import type { SkillCopy, SkillReview } from "../skill-types";

export function SkillReviewPreview({
  copy,
  review,
  busy,
  onEdit,
  onApprove,
}: {
  copy: SkillCopy;
  review: SkillReview;
  busy: boolean;
  onEdit: () => void;
  onApprove: () => void;
}) {
  return (
    <div className="grid gap-4">
      <div className="flex flex-wrap gap-x-5 gap-y-[9px] text-[11px] text-muted-foreground">
        <span>
          <b>{copy.exact_package}</b> {review.draft.name}
        </span>
        <span>
          <b>{copy.digest}</b> <code>{review.skill_sha256}</code>
        </span>
      </div>
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        {copy.evaluation_disclosure}
      </p>
      <pre className="max-h-[340px] overflow-auto rounded-[9px] border border-border bg-muted p-4 font-mono text-[11px] leading-[1.55] text-foreground whitespace-pre-wrap">
        {review.skill_md}
      </pre>
      <div className="mt-6 flex gap-2.5">
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={onEdit}
          disabled={busy}
        >
          {copy.edit_skill}
        </Button>
        <Button
          className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
          type="button"
          onClick={onApprove}
          disabled={busy}
        >
          {busy ? copy.testing : copy.approve_and_test}
        </Button>
      </div>
    </div>
  );
}
