import { z } from "zod";

export const inviteFormSchema = z.object({
  invite: z.string().trim().min(1, "Invite link is required."),
});

export type InviteFormValues = z.infer<typeof inviteFormSchema>;
