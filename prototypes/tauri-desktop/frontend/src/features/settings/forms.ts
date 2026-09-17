import { z } from "zod";
import type { SourceMode, SourceName } from "./api/source-roots-api";

const absolutePath = z
  .string()
  .trim()
  .refine(
    (value) =>
      value === "" || value.startsWith("/") || /^[A-Za-z]:[\\/]/.test(value),
    "Use an absolute directory path.",
  );

export function behaviorFormSchema(min: number, max: number) {
  return z.object({
    value: z
      .string()
      .trim()
      .regex(/^\d+$/, "Use a whole number.")
      .refine(
        (value) => Number(value) >= min && Number(value) <= max,
        `Use a value from ${min} to ${max}.`,
      ),
  });
}

export type BehaviorFormValues = { value: string };

export const routingFormSchema = z.object({
  enabled: z.boolean(),
  port: z
    .string()
    .trim()
    .regex(/^\d+$/, "Use a valid port.")
    .refine(
      (value) => Number(value) >= 1 && Number(value) <= 65535,
      "Port must be between 1 and 65535.",
    ),
  tokenDir: absolutePath,
});

export const witnessFormSchema = z.object({
  url: z.string().trim().url("Enter a valid witness URL."),
  signingAddress: z.string().trim().min(1, "Signing address is required."),
  measurements: z
    .string()
    .refine(
      (value) => value.split("\n").some((entry) => entry.trim().length > 0),
      "Add at least one measurement pin.",
    ),
});

export const sourceRootFormSchema = z
  .object({
    mode: z.enum(["watch", "off"]),
    path: absolutePath,
  })
  .superRefine((values, context) => {
    if (values.mode === "watch" && values.path.length === 0) {
      context.addIssue({
        code: "custom",
        path: ["path"],
        message: "Choose an absolute directory path.",
      });
    }
  });

export const projectModeFormSchema = z.object({
  mode: z.enum(["notify_only", "auto_upload", "ignore"]),
});

export function consentSettingsFormSchema(alwaysOn: string[]) {
  return z.object({
    scopes: z.array(z.string()).superRefine((scopes, context) => {
      if (alwaysOn.some((name) => !scopes.includes(name))) {
        context.addIssue({
          code: "custom",
          message: "Always-included scopes are required.",
        });
      }
    }),
  });
}

export type RoutingFormValues = z.infer<typeof routingFormSchema>;
export type WitnessFormValues = z.infer<typeof witnessFormSchema>;
export type SourceRootFormValues = z.infer<typeof sourceRootFormSchema>;
export type ProjectModeFormValues = z.infer<typeof projectModeFormSchema>;
export type ConsentSettingsFormValues = { scopes: string[] };
export type SourceRootValues = {
  modes: Record<SourceName, SourceMode>;
  paths: Record<SourceName, string>;
};
