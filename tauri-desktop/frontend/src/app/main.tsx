import { QueryClientProvider } from "@tanstack/react-query";
import { isTauri } from "@tauri-apps/api/core";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { HashRouter, Route, Routes } from "react-router-dom";
import { FtuxPreviewRoute, ftuxPreviewPath } from "../features/ftux";
import { AppShell } from "./app-shell";
import "./app.css";
import { createQueryClient } from "../lib/query/query-client";

// Inside the app the window itself is transparent: the panes float over the
// desktop with no frame around them. In a plain browser the page keeps the
// scene as its background.
if (isTauri()) document.documentElement.classList.add("tc-native");

const root = document.getElementById("root");
if (!root) throw new Error("Application root is missing");

const queryClient = createQueryClient();

// The first-run preview runs on mock data and draws imitation macOS sheets,
// so it exists only in development builds or when a build opts in.
const ftuxPreviewEnabled =
  import.meta.env.DEV || import.meta.env.VITE_FTUX_PREVIEW === "1";

createRoot(root).render(
  <StrictMode>
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
  </StrictMode>,
);
