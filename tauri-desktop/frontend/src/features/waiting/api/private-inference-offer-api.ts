import {
  daemonCall,
  invokeTauriDiscardResult,
} from "../../../lib/tauri/core-api";

export type PrivateInferenceOfferState = {
  configured: boolean;
  answered: boolean;
  enabled: boolean;
};

export async function getPrivateInferenceOfferState(): Promise<PrivateInferenceOfferState> {
  const value = await daemonCall("get_settings");
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error("Invalid Private AI settings");
  const item = value as Record<string, unknown>;
  if (
    typeof item.near_ai_inference_configured !== "boolean" ||
    typeof item.private_inference_offer_seen !== "boolean" ||
    typeof item.private_inference !== "boolean"
  )
    throw new Error("Invalid Private AI settings");
  return {
    configured: item.near_ai_inference_configured,
    answered: item.private_inference_offer_seen,
    enabled: item.private_inference,
  };
}

export async function answerPrivateInferenceOffer(enabled: boolean) {
  return invokeTauriDiscardResult("set_private_inference", { enabled });
}
