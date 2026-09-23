import { QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { HashRouter } from "react-router-dom";
import { ThemeProvider } from "../components/theme-provider";
import { TooltipProvider } from "../components/ui/tooltip";
import { AppShell } from "./app-shell";
import "./app.css";
import { createQueryClient } from "../lib/query/query-client";

const root = document.getElementById("root");
if (!root) throw new Error("Application root is missing");

const queryClient = createQueryClient();

createRoot(root).render(
  <StrictMode>
    <ThemeProvider>
      <TooltipProvider>
        <QueryClientProvider client={queryClient}>
          <HashRouter>
            <AppShell />
          </HashRouter>
        </QueryClientProvider>
      </TooltipProvider>
    </ThemeProvider>
  </StrictMode>,
);
