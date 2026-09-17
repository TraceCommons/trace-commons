import { z } from "zod";

export const inviteFormSchema = z.object({
  invite: z.string().trim().min(1, "Invite link is required."),
});

export const privacyFormSchema = z.object({
  privacyChoice: z.enum(["local", "scan"], {
    error: "Choose a privacy boundary.",
  }),
});

export type InviteFormValues = z.infer<typeof inviteFormSchema>;
export type PrivacyFormValues = z.infer<typeof privacyFormSchema>;
export type ConsentFormValues = { scopes: string[] };

export function consentFormSchema(alwaysOn: string[]) {
  return z.object({
    scopes: z.array(z.string()).superRefine((scopes, context) => {
      for (const name of alwaysOn) {
        if (!scopes.includes(name)) {
          context.addIssue({
            code: "custom",
            message: "Always-included scopes are required.",
          });
          return;
        }
      }
    }),
  });
}
