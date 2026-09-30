import { useQuery } from "@tanstack/react-query";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { getCertificateCopy } from "../api/certificate-copy-api";
import { waitingKeys } from "../api/query-keys";

export function useCertificateCopy(evidenceAdmitted: boolean) {
  const core = useCoreStatus();
  return useQuery({
    queryKey: waitingKeys.certificateCopy(core.scope, evidenceAdmitted),
    queryFn: () => getCertificateCopy(evidenceAdmitted),
    enabled: core.isSuccess,
  });
}
