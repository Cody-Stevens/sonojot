import { RECOMMENDED_ONLINE_NAMES } from "./cleanup-choice";
import type { AiModel } from "./ai";
import type { ModelEntry } from "./model-library";

const choices: Record<string, string> = {
  "local-rules": "No proofreader",
  openrouter: "OpenRouter",
  "Same as dictation": "Same as dictation",
  "SonoJot cleanup": "SonoJot proofreader",
};

export function modelName(
  id: string,
  local: readonly Pick<ModelEntry, "id" | "name">[],
  online: readonly Pick<AiModel, "id" | "name">[] = [],
): string {
  return (
    choices[id] ||
    local.find((model) => model.id === id)?.name ||
    online.find((model) => model.id === id)?.name ||
    RECOMMENDED_ONLINE_NAMES[id] ||
    (id.includes("/") ? "Selected OpenRouter model" : "Selected model")
  );
}
