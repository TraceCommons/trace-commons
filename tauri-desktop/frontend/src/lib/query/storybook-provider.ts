import { QueryClientProvider } from "@tanstack/react-query";
import { createElement, type ReactNode } from "react";
import { createQueryClient } from "./query-client";

type Story = () => ReactNode;

export const withQueryClient = (Story: Story) =>
  createElement(
    QueryClientProvider,
    { client: createQueryClient() },
    createElement(Story),
  );
