import { z } from "zod";

export const computeFormSchema = z.object({
  allowance: z
    .string()
    .trim()
    .regex(/^\d+$/, "Use a whole number of GiB.")
    .refine((value) => Number(value) >= 1, "Allowance must be at least 1 GiB."),
});

export type ComputeFormValues = z.infer<typeof computeFormSchema>;
