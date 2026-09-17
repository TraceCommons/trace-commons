import { z } from "zod";

export const missionImportFormSchema = z.object({
  file: z.unknown(),
});

export type MissionImportFormValues = z.infer<typeof missionImportFormSchema>;
