import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef } from "react";

export function useAccountQueryLifecycle(scope: string) {
  const queryClient = useQueryClient();
  const previousScope = useRef(scope);

  useEffect(() => {
    if (previousScope.current === scope) return;
    queryClient.removeQueries({
      predicate: (query) =>
        query.queryKey[0] === "account" && query.queryKey[1] !== scope,
    });
    previousScope.current = scope;
  }, [queryClient, scope]);
}
