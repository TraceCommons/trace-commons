import { z } from "zod";
import { invokeTauri } from "../../../lib/tauri/core-api";
import type { CertificateCopy } from "../types";

const certificateCopySchema = z.object({
  list_title: z.string(),
  row_line: z.string(),
  list_empty: z.string(),
});

export async function getCertificateCopy(
  evidenceAdmitted: boolean,
): Promise<CertificateCopy> {
  const result = certificateCopySchema.safeParse(
    await invokeTauri("certificate_copy", { evidenceAdmitted }),
  );
  if (!result.success) throw new Error("Invalid certificate copy");
  return result.data;
}
