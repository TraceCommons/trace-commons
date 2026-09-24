import { z } from "zod";

export const privateAiProviderSchema = z.object({
  provider: z.enum(["github", "google", "near"], {
    error: "Choose a credential provider.",
  }),
});

export type PrivateAiProviderValues = z.infer<typeof privateAiProviderSchema>;
