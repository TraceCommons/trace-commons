import type { SkillCopy, SkillReview } from "../skill-types";
import { ButtonPrimary, GlassButton } from "@/design-system";

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
      <div className="flex flex-wrap gap-x-5 gap-y-[9px] text-[11px] text-tc-secondary">
        <span>
          <b>{copy.exact_package}</b> {review.draft.name}
        </span>
        <span>
          <b>{copy.digest}</b> <code>{review.skill_sha256}</code>
        </span>
      </div>
      <p className="m-0 tc-caption tc-text-tertiary">
        {copy.evaluation_disclosure}
      </p>
      <pre className="tc-code max-h-[340px] overflow-auto leading-[1.55] whitespace-pre-wrap">
        {review.skill_md}
      </pre>
      <div className="mt-3 flex flex-wrap gap-2">
        <GlassButton
          type="button"
          onClick={onEdit}
          disabled={busy}
        >
          {copy.edit_skill}
        </GlassButton>
        <ButtonPrimary size="sm"
          type="button"
          onClick={onApprove}
          disabled={busy}
        >
          {busy ? copy.testing : copy.approve_and_test}
        </ButtonPrimary>
      </div>
    </div>
  );
}
