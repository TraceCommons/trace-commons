import { z } from "zod";
import type { HistoryDetail, PublicRunEditorInput } from "./types";

export const publicRunFormSchema = z.object({
  title: z
    .string()
    .trim()
    .min(1, "Page title is required.")
    .max(100, "Page title must be 100 characters or fewer."),
  outcome_summary: z
    .string()
    .trim()
    .min(1, "Public outcome summary is required.")
    .max(600, "Public outcome summary must be 600 characters or fewer."),
  correction_excerpt: z
    .string()
    .max(1000, "Correction must be 1000 characters or fewer.")
    .nullable(),
  workflow: z
    .string()
    .trim()
    .min(1, "Reusable instructions are required.")
    .max(4000, "Reusable instructions must be 4000 characters or fewer."),
  reuse_permission: z
    .enum(["cc_by_4_0", "cc0_1_0"])
    .nullable()
    .superRefine((value, context) => {
      if (value === null)
        context.addIssue({
          code: "custom",
          message: "Choose a reuse permission.",
        });
    }),
  evidence: z
    .array(
      z.object({
        event_id: z.string().min(1, "Evidence event is required."),
        excerpt: z
          .string()
          .min(1, "Evidence excerpt is required.")
          .max(700, "Evidence excerpt must be 700 characters or fewer."),
      }),
    )
    .min(1, "Select at least one supporting excerpt.")
    .max(4, "Select no more than four supporting excerpts.")
    .superRefine((items, context) => {
      if (new Set(items.map((item) => item.event_id)).size !== items.length) {
        context.addIssue({
          code: "custom",
          message: "Select each supporting excerpt only once.",
        });
      }
    }),
  source: z
    .string()
    .trim()
    .refine((value) => {
      if (value === "") return true;
      const slug = "[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?";
      return new RegExp(
        `^(?:${slug}|https://tracecommons\\.ai/runs/${slug})$`,
      ).test(value);
    }, "Use a public run slug or a tracecommons.ai/runs link."),
});

export type PublicRunFormValues = z.infer<typeof publicRunFormSchema>;

export const skillDraftFormSchema = z.object({
  name: z
    .string()
    .trim()
    .min(1, "Skill name is required.")
    .max(64, "Skill name must be 64 characters or fewer.")
    .regex(/^[a-z0-9]+(?:-[a-z0-9]+)*$/, "Use a lowercase hyphenated name."),
  description: z
    .string()
    .trim()
    .min(1, "Applicability is required.")
    .max(1024, "Applicability must be 1024 characters or fewer."),
  procedure: z
    .string()
    .trim()
    .min(1, "Procedure is required.")
    .max(12000, "Procedure must be 12000 characters or fewer."),
});

export type SkillDraftFormValues = z.infer<typeof skillDraftFormSchema>;

export function publicRunInputFromDetail(
  detail: HistoryDetail,
): PublicRunEditorInput {
  const publication = detail.publication;
  return {
    title: publication?.title ?? "",
    outcome_summary: publication?.outcome_summary ?? "",
    correction_excerpt: publication?.correction_excerpt ?? null,
    workflow: publication?.workflow ?? "",
    reuse_permission: publication?.reuse_permission ?? null,
    evidence:
      publication?.evidence.flatMap((item) => {
        const match = detail.evidence.find(
          (candidate) => candidate.excerpt === item.excerpt,
        );
        return match
          ? [{ event_id: match.event_id, excerpt: match.excerpt }]
          : [];
      }) ?? [],
    source: publication?.source?.slug ?? detail.retained_source_slug ?? "",
  };
}
