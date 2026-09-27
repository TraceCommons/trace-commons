import { z } from "zod";

const uuid = z
  .string()
  .regex(
    /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
    "Enter a valid UUID.",
  )
  .refine(
    (value) => value !== "00000000-0000-0000-0000-000000000000",
    "Enter a non-empty UUID.",
  );
const digest = z
  .string()
  .regex(/^[a-f0-9]{64}$/, "Use 64 lowercase hex characters.");
const date = z
  .string()
  .regex(/^\d{4}-\d{2}-\d{2}$/, "Use YYYY-MM-DD.")
  .refine((value) => {
    const parsed = new Date(`${value}T00:00:00Z`);
    return (
      !Number.isNaN(parsed.getTime()) &&
      parsed.toISOString().slice(0, 10) === value
    );
  }, "Enter a valid calendar date.");
const contextLabel = z
  .string()
  .trim()
  .min(1, "Value is required.")
  .max(128, "Use at most 128 characters.")
  .regex(/^[A-Za-z0-9._:/+-]+$/, "Use letters, numbers, . _ : / + or -.");
const cohortLabel = contextLabel.max(96, "Use at most 96 characters.");

function cohortLabels(value: string) {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

export const analysisSourceSchema = z.object({
  source: z.enum(["codex", "claude_code", "trajectory"]),
  file: z.unknown(),
});

export const annotationFormSchema = z.object({
  category: z.enum([
    "unknown",
    "refactor",
    "tests",
    "docs",
    "debugging",
    "other",
  ]),
  outcome: z.enum(["unknown", "accepted", "partial", "rejected"]),
});

export const gitEvidenceFormSchema = z.object({
  repository: z
    .string()
    .trim()
    .min(1, "Choose a repository.")
    .refine(
      (value) => value.startsWith("/") || /^[A-Za-z]:[\\/]/.test(value),
      "Choose an absolute repository path.",
    ),
  commit: z
    .string()
    .trim()
    .regex(
      /^[a-f0-9]{40}(?:[a-f0-9]{24})?$/,
      "Use a 40 or 64 character lowercase commit.",
    ),
  reportFile: z.unknown(),
});

export const workflowFormSchema = z.object({
  snapshotSelection: z.array(z.string()),
  episodeSelection: z.array(z.string()),
  category: annotationFormSchema.shape.category,
  outcome: annotationFormSchema.shape.outcome,
});

export const comparisonTaskFormSchema = z.object({
  episodeSelection: z.array(z.string()),
  editSelection: z.array(z.string()),
  projectId: uuid,
  taskDate: date,
  language: z.union([z.literal(""), contextLabel]),
  harnessId: z.union([z.literal(""), contextLabel]),
  harnessVersion: z.union([z.literal(""), contextLabel]),
  toolPolicyId: z.union([z.literal(""), contextLabel]),
  toolPolicyVersion: z.union([z.literal(""), contextLabel]),
  promptDigest: z
    .string()
    .refine(
      (value) => value === "" || digest.safeParse(value).success,
      "Use 64 lowercase hex characters.",
    ),
  reasoning: z.enum([
    "unknown",
    "none",
    "minimal",
    "low",
    "medium",
    "high",
    "xhigh",
  ]),
  outcome: z.enum(["pending", "accepted", "partial", "rejected", "unknown"]),
});

export const comparisonSpecificationFormSchema = z
  .object({
    projectId: uuid,
    language: contextLabel,
    fingerprint: digest,
    cohorts: z
      .string()
      .trim()
      .refine((value) => {
        const labels = cohortLabels(value);
        return (
          labels.length === 2 &&
          labels.every((label) => cohortLabel.safeParse(label).success) &&
          labels[0] < labels[1]
        );
      }, "Enter two sorted cohort labels."),
    dateStart: date,
    dateEnd: date,
    cutoff: z
      .string()
      .min(1, "Evidence cutoff is required.")
      .refine(
        (value) => !Number.isNaN(new Date(value).getTime()),
        "Use a valid evidence cutoff.",
      )
      .refine(
        (value) => new Date(value).getTime() <= Date.now(),
        "Evidence cutoff cannot be in the future.",
      ),
  })
  .refine((values) => values.dateEnd >= values.dateStart, {
    path: ["dateEnd"],
    message: "End date must be on or after start date.",
  });

export type AnalysisSourceValues = z.infer<typeof analysisSourceSchema>;
export type AnnotationFormValues = z.infer<typeof annotationFormSchema>;
export type GitEvidenceFormValues = z.infer<typeof gitEvidenceFormSchema>;
export type WorkflowFormValues = z.infer<typeof workflowFormSchema>;
export type ComparisonTaskFormValues = z.infer<typeof comparisonTaskFormSchema>;
export type ComparisonSpecificationFormValues = z.infer<
  typeof comparisonSpecificationFormSchema
>;
