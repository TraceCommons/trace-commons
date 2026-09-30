export function accountScope(tenantId: string | null | undefined) {
  return tenantId ?? "anonymous";
}
