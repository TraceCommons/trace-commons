/** The Settings modal's sections, in order: id and navigation label. */
export const settingsSections = [
  { id: "connection", label: "Connection" },
  { id: "startup", label: "Startup & notifications" },
  { id: "watching", label: "Watching" },
  { id: "uses", label: "How traces may be used" },
  { id: "profile", label: "Public profile" },
  { id: "folders", label: "Watched folders" },
  { id: "tools", label: "Tools" },
  { id: "pai", label: "Private AI" },
  { id: "witness", label: "Redaction witness" },
  { id: "projects", label: "Projects" },
  { id: "log", label: "Changes on this machine" },
] as const;
