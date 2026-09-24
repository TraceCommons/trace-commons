import { z } from "zod";

function hasOnlyAllowedBioCharacters(value: string) {
  return [...value].every((character) => {
    if (character === "\n") return true;
    const code = character.codePointAt(0) ?? 0;
    return !(code <= 31 || (code >= 127 && code <= 159));
  });
}

export const profileFormSchema = z.object({
  handle: z
    .string()
    .trim()
    .min(3, "Handle must be at least 3 characters.")
    .max(32, "Handle must be 32 characters or fewer.")
    .regex(
      /^[A-Za-z0-9](?:[A-Za-z0-9]|[-_](?![-_]))*[A-Za-z0-9]$/,
      "Use letters, numbers, hyphens, or underscores; no consecutive separators.",
    )
    .refine(
      (value) =>
        ![
          "admin",
          "administrator",
          "anonymous",
          "api",
          "billing",
          "community",
          "contact",
          "ironclaw",
          "leaderboard",
          "legal",
          "moderator",
          "operator",
          "owner",
          "privacy",
          "profile",
          "root",
          "security",
          "staff",
          "support",
          "system",
          "team",
          "trace",
          "trace-commons",
          "tracecommons",
        ].includes(value.toLowerCase()),
      "That handle is reserved.",
    ),
  bio: z
    .string()
    .refine(
      (value) => new TextEncoder().encode(value).length <= 280,
      "Bio must be 280 bytes or fewer.",
    )
    .refine(
      hasOnlyAllowedBioCharacters,
      "Bio contains an unsupported control character.",
    ),
});

export const profileConsentSchema = z.object({
  acknowledged: z
    .boolean()
    .refine(
      (value) => value,
      "Acknowledge public attribution before continuing.",
    ),
});

export type ProfileFormValues = z.infer<typeof profileFormSchema>;
export type ProfileConsentValues = z.infer<typeof profileConsentSchema>;
