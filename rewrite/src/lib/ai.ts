import { z } from "zod";

export const aiQuickModelSchema = z.object({
  id: z.string(),
  role: z.enum(["text", "speech"]),
});
export type AiQuickModel = z.infer<typeof aiQuickModelSchema>;
export const aiSettingsSchema = z
  .object({
    backend: z.enum(["local", "openrouter"]),
    model: z.string(),
    speechModel: z.string().default(""),
    quickModels: z.array(aiQuickModelSchema).optional(),
    connected: z.boolean(),
    supported: z.boolean(),
    help: z.string(),
  })
  .transform((settings) => ({
    ...settings,
    // Only missing fields migrate; an explicitly empty list stays empty.
    quickModels:
      settings.quickModels ??
      [
        { id: settings.model, role: "text" as const },
        { id: settings.speechModel, role: "speech" as const },
      ].filter((item) => item.id !== ""),
  }));
export type AiSettings = z.infer<typeof aiSettingsSchema>;
export type AiSettingsPatch = Partial<
  Pick<AiSettings, "backend" | "model" | "speechModel" | "quickModels">
>;

export function quickModelRemoval(
  settings: AiSettings,
  entry: AiQuickModel,
): AiSettingsPatch {
  return {
    quickModels: settings.quickModels.filter(
      (item) => item.id !== entry.id || item.role !== entry.role,
    ),
    ...(entry.role === "text" && settings.model === entry.id
      ? { backend: "local" as const }
      : {}),
    ...(entry.role === "speech" && settings.speechModel === entry.id
      ? { speechModel: "" }
      : {}),
  };
}
export const aiModelSchema = z.object({
  id: z.string(),
  name: z.string(),
  contextLength: z.number(),
  promptPrice: z.string().nullable(),
  completionPrice: z.string().nullable(),
  description: z.string().nullable().default(null),
  inputModalities: z.array(z.string()).default([]),
  /** Curated OpenRouter reasoning effort for a recommended model. */
  reasoningEffort: z.string().nullable().optional(),
});
export type AiModel = z.infer<typeof aiModelSchema>;

const effortNames: Record<string, string> = {
  max: "Maximum",
  xhigh: "Extra-high",
  high: "High",
  medium: "Medium",
  low: "Low",
  minimal: "Minimal",
};
/** Short row note for a curated effort, e.g. "High reasoning". */
export function effortLabel(effort: string | null | undefined): string {
  if (!effort || effort === "none") return "";
  return `${effortNames[effort] ?? effort} reasoning`;
}

export function roleLabel(role: AiQuickModel["role"]): string {
  return role === "speech" ? "Meeting transcription" : "Text and notes";
}

/** Plain list of roles, e.g. "Text and notes, meeting transcription". */
export function rolesLabel(roles: readonly AiQuickModel["role"][]): string {
  const text = roles.map((role, index) =>
    index === 0 ? roleLabel(role) : roleLabel(role).toLocaleLowerCase(),
  );
  return text.join(", ");
}

export interface CatalogRow {
  item: AiModel;
  roles: AiQuickModel["role"][];
}
/** One row per model; a speech-capable text model lists both roles. */
export function catalogRows(
  items: readonly AiModel[],
  query: string,
): CatalogRow[] {
  return items
    .map((item) => ({ item, roles: modelRoles(item) }))
    .filter(
      ({ item, roles }) =>
        roles.length > 0 &&
        (!query ||
          `${item.name} ${item.id} ${roles.join(" ")} ${roles.map(roleLabel).join(" ")} ${item.inputModalities.join(" ")}`
            .toLocaleLowerCase()
            .includes(query)),
    );
}

/** Group the saved per-role list into one entry per model, in list order. */
export function groupQuickModels(
  entries: readonly AiQuickModel[],
): { id: string; roles: AiQuickModel["role"][] }[] {
  const groups = new Map<string, AiQuickModel["role"][]>();
  for (const entry of entries)
    groups.set(entry.id, [...(groups.get(entry.id) ?? []), entry.role]);
  return [...groups].map(([id, roles]) => ({
    id,
    roles: (["text", "speech"] as const).filter((role) => roles.includes(role)),
  }));
}

/** Settings patch that removes a model from every role it holds. */
export function quickModelGroupRemoval(
  settings: AiSettings,
  id: string,
): AiSettingsPatch {
  const roles = settings.quickModels
    .filter((entry) => entry.id === id)
    .map((entry) => entry.role);
  return {
    quickModels: settings.quickModels.filter((entry) => entry.id !== id),
    ...(roles.includes("text") && settings.model === id
      ? { backend: "local" as const }
      : {}),
    ...(roles.includes("speech") && settings.speechModel === id
      ? { speechModel: "" }
      : {}),
  };
}
export const vocabularySuggestionSchema = z.object({
  heard: z.string(),
  written: z.string(),
  reason: z.string(),
});
export type VocabularySuggestion = z.infer<typeof vocabularySuggestionSchema>;
export const aiModelCheckSchema = z.object({
  availability: z.enum(["listed", "unlisted", "unknown"]),
  privacy: z.enum(["zdrListed", "unknown"]),
  variant: z.string().nullable(),
  routes: z.number().int().nonnegative(),
  activeRoutes: z.number().int().nonnegative(),
});
export type AiModelCheck = z.infer<typeof aiModelCheckSchema>;

/** Older text catalogs did not publish input modalities. */
export function modelRoles(item: AiModel): AiQuickModel["role"][] {
  const roles: AiQuickModel["role"][] = [];
  if (!item.inputModalities.length || item.inputModalities.includes("text"))
    roles.push("text");
  if (item.inputModalities.includes("audio")) roles.push("speech");
  return roles;
}

function perMillion(price: string | null): number | null {
  if (price === null || !price.trim()) return null;
  const value = Number(price) * 1e6;
  return Number.isFinite(value) && value >= 0 ? value : null;
}

function dollars(value: number): string {
  if (value > 0 && value < 0.0001) return "<$0.0001";
  const text = value.toFixed(value < 1 ? 4 : 2);
  return `$${text.replace(/0+$/, "").replace(/\.$/, "")}`;
}

export function priceLine(item: AiModel): string {
  const input = perMillion(item.promptPrice);
  const output = perMillion(item.completionPrice);
  if (input === null || output === null)
    return "OpenRouter does not list a price for this model.";
  return `${dollars(input)} input, ${dollars(output)} output per 1M tokens`;
}

export function routeNotes(
  check: AiModelCheck | null,
  failed: boolean,
): string[] {
  if (failed)
    return [
      "Route availability could not be checked. The model may still work.",
    ];
  if (!check) return [];
  const notes: string[] = [];
  if (check.variant === "batch") {
    notes.push(
      "Batch variants are meant for asynchronous jobs and may not accept interactive requests. SonoJot only sends interactive requests, so this model may not answer.",
    );
  } else if (check.variant) {
    notes.push(
      `This selects the “${check.variant}” variant of the model, not the standard route.`,
    );
  }
  if (check.availability === "unlisted") {
    notes.push(
      "OpenRouter does not list an active provider route for this model. It may not run.",
    );
  } else if (check.availability === "unknown") {
    notes.push("OpenRouter could not confirm a route for this model.");
  }
  if (check.privacy === "zdrListed") {
    notes.push(
      "All currently active listed routes appear on OpenRouter's zero-data-retention list. SonoJot requests no data collection on every call.",
    );
  } else if (check.availability === "listed") {
    notes.push(
      "Route data retention is not published for this model. SonoJot requests no data collection on every call; account-level provider filters cannot be checked in advance.",
    );
  }
  return notes;
}
