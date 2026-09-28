import { QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { HashRouter, Route, Routes } from "react-router-dom";
import { ThemeProvider } from "../components/theme-provider";
import { TooltipProvider } from "../components/ui/tooltip";
import { FtuxPreviewRoute, ftuxPreviewPath } from "../features/ftux";
import { AppShell } from "./app-shell";
import "./app.css";
import { createQueryClient } from "../lib/query/query-client";

const root = document.getElementById("root");
if (!root) throw new Error("Application root is missing");

const queryClient = createQueryClient();

// The first-run preview runs on mock data and draws imitation macOS sheets,
// so it exists only in development builds or when a build opts in.
const ftuxPreviewEnabled =
  import.meta.env.DEV || import.meta.env.VITE_FTUX_PREVIEW === "1";

createRoot(root).render(
  <StrictMode>
    <ThemeProvider>
      <TooltipProvider>
        <QueryClientProvider client={queryClient}>
          <HashRouter>
            <Routes>
              {ftuxPreviewEnabled ? (
                <Route path={ftuxPreviewPath} element={<FtuxPreviewRoute />} />
              ) : null}
              <Route path="*" element={<AppShell />} />
            </Routes>
          </HashRouter>
        </QueryClientProvider>
      </TooltipProvider>
    </ThemeProvider>
  </StrictMode>,
);
