import { z } from "zod";

export const originalSearchFormSchema = z.object({
  needle: z.string().trim().min(1, "Enter a search term."),
});

export type OriginalSearchFormValues = z.infer<typeof originalSearchFormSchema>;
