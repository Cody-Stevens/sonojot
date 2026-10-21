import { catalogRows, type AiModel } from "./ai";
import type { Settings } from "./protocol";

export const PROOFREADER_LABEL =
  "Proofread with an online model (experimental)";
export const PROOFREADER_HELP =
  "Fix misheard words and small slips. Sends dictation text and vocabulary to OpenRouter.";
export const PROOFREADER_DEFAULT_MODEL = "openai/gpt-6-luna";
export const OPENROUTER_PRIVACY_URL = "https://openrouter.ai/settings/privacy";
export const CLOUD_NOTICE_KEY = "openrouter-cloud";
export const CLOUD_NOTICE_VERSION = "1";

export function needsCloudNotice(
  settings: Pick<Settings, "componentAcknowledgments">,
): boolean {
  return (
    settings.componentAcknowledgments[CLOUD_NOTICE_KEY] !== CLOUD_NOTICE_VERSION
  );
}

export function proofreaderModels(
  models: readonly AiModel[],
  query: string,
): AiModel[] {
  return catalogRows(models, query.trim().toLocaleLowerCase())
    .filter(({ roles }) => roles.includes("text"))
    .map(({ item }) => item);
}

export function needsTrainingAcceptance(
  model: AiModel,
  settings: Pick<Settings, "proofreaderTrainingModels">,
): boolean {
  return (
    model.trainingRequired &&
    !settings.proofreaderTrainingModels.includes(model.id)
  );
}

export function trainingNotice(model: Pick<AiModel, "id" | "name">): string {
  return `${model.name} (${model.id}) may let its provider use your dictation text and vocabulary to train models. Accepting allows data collection for this exact model when proofreading. Your OpenRouter account privacy settings still apply.`;
}

export function trainingAcceptance(
  model: AiModel,
  settings: Pick<Settings, "proofreaderTrainingModels">,
): string[] {
  return [...new Set([...settings.proofreaderTrainingModels, model.id])];
}

export function proofreaderGuardLabel(reason: string): string {
  const labels: Record<string, string> = {
    ok: "Proofread",
    accepted: "Proofread",
    unchanged: "Proofreading kept the text unchanged",
    deadline_exceeded: "Proofreading timed out; earlier text kept",
    timeout: "Proofreading timed out; earlier text kept",
    cancelled: "Proofreading cancelled",
    stopped: "Proofreading stopped; earlier text kept",
    cleanup_stopped: "Proofreading stopped; earlier text kept",
    training_consent_required: "Review this model's training notice",
    cloud_consent_required: "Review the OpenRouter notice",
    connection_required: "Connect OpenRouter to proofread dictations",
    invalid_model: "Choose a proofreading model",
    settings_unavailable:
      "Proofreading settings unavailable; earlier text kept",
    input_limit: "Text exceeds the proofreading limit",
    output_limit: "Proofreading result exceeds the limit",
    word_limit: "Text exceeds the proofreading limit",
    empty_input: "No text to proofread",
    account_training_blocked:
      "Your OpenRouter account blocks this training model",
    "paid-model-training-violation-by-account":
      "Your OpenRouter account blocks this training model",
    empty_response: "Empty proofreading result blocked",
    invalid_response: "Invalid proofreading result blocked",
    incomplete_response: "Incomplete proofreading result blocked",
    response_markers: "Model commentary or markup blocked",
    em_dash_added: "Added em dash blocked",
    digits_changed: "Number change blocked",
    address_changed: "Link or email address change blocked",
    edit_run_too_long: "Large wording change blocked",
    deletion_run_too_long: "Long word removal blocked",
    too_many_deletions: "Large word removal blocked",
    line_breaks_changed: "Line break change blocked",
    provider_error: "Proofreading unavailable; earlier text kept",
    worker_unavailable: "Proofreading unavailable; earlier text kept",
    worker_failed: "Proofreading unavailable; earlier text kept",
    connection_unavailable: "Proofreading unavailable; earlier text kept",
  };
  return (
    labels[reason.replace(/^proofreader:/, "")] ??
    "Proofreading skipped; earlier text kept"
  );
}

export function proofreaderMetadata(status: string | null): {
  model: string;
  outcome: string;
  reason: string;
} | null {
  if (!status) return null;
  const fields = new Map(
    status
      .split(";")
      .filter((part) => part.includes("="))
      .map((part) => {
        const separator = part.indexOf("=");
        return [part.slice(0, separator), part.slice(separator + 1)] as const;
      }),
  );
  const model = fields.get("proofreader_model");
  const outcome = fields.get("proofreader_outcome");
  const reason = fields.get("proofreader_reason");
  return model && outcome && reason ? { model, outcome, reason } : null;
}

export function proofreaderOutcomeLabel(outcome: string): string {
  const labels: Record<string, string> = {
    accepted: "Proofread",
    unchanged: "Text unchanged",
    rejected: "Proposal rejected; earlier text kept",
    timeout: "Timed out; earlier text kept",
    error: "Unavailable; earlier text kept",
    skipped: "Skipped; earlier text kept",
    stopped: "Stopped; earlier text kept",
    cancelled: "Cancelled",
  };
  return labels[outcome] ?? outcome;
}

export function proofreaderAccountBlocked(reason: string | null): boolean {
  return (
    !!reason &&
    /account_training_blocked|paid-model-training-violation-by-account/.test(
      reason,
    )
  );
}
