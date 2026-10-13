import { z } from "zod";

const bytes = z.number().finite().nonnegative();
export const modelKindSchema = z.enum([
  "asr",
  "cleanup",
  "diarization",
  "punctuation",
]);
export const modelEntrySchema = z.object({
  id: z.string(),
  name: z.string(),
  kind: modelKindSchema,
  roles: z.array(z.string()),
  family: z.string(),
  languages: z.array(z.string()),
  older: z.boolean(),
  deprecated: z.boolean().default(false),
  diskBytes: bytes,
  ramMinBytes: bytes.nullable(),
  ramMaxBytes: bytes.nullable(),
  vramMinBytes: bytes.nullable(),
  vramMaxBytes: bytes.nullable(),
  memoryBasis: z.string(),
  guidance: z.string(),
  evidence: z.string(),
  backend: z.string(),
  compatibility: z.string(),
  state: z.string(),
  downloadedBytes: bytes,
  totalBytes: bytes,
  error: z.string().nullable(),
  installable: z.boolean(),
  removable: z.boolean(),
  usedBy: z.array(z.string()),
  revision: z.string(),
  quantization: z.string(),
  contextTokens: z.number().int().positive().nullable(),
});
export const modelLibrarySchema = z.object({
  hardware: z.object({
    unifiedMemory: z.boolean().optional(),
    totalRamBytes: bytes.nullable(),
    availableRamBytes: bytes.nullable(),
    dedicatedVramBytes: bytes.nullable(),
    availableVramBytes: bytes.nullable().optional(),
  }),
  models: z.array(modelEntrySchema),
  guidance: z.string(),
});
export type ModelEntry = z.infer<typeof modelEntrySchema>;
export type ModelLibrary = z.infer<typeof modelLibrarySchema>;
/** Retired packages remain reachable when installed, selected or in progress. */
export function discoverable(
  model: Pick<ModelEntry, "deprecated" | "state" | "usedBy">,
): boolean {
  return (
    !model.deprecated ||
    model.usedBy.length > 0 ||
    [
      "installed",
      "unloaded",
      "installed/unloaded",
      "installed_unloaded",
      "loading",
      "ready",
      "busy",
      "unloading",
      "checking",
      "queued",
      "downloading",
      "verifying",
      "installing",
      "cancelling",
      "canceling",
    ].includes(model.state)
  );
}
export type ModelRole = "asr" | "cleanup" | "notes";
export type ModelSelectionRole = ModelRole | "meetingSpeech";
export const GRANITE_SPEECH_MODEL = "granite-speech-4.1-2b-nar-q8";
// These are the dictation/profile roles. Meeting speech is selected separately.
export const modelRoles = [
  { id: "asr", setting: "asrModel", title: "Speech" },
  { id: "cleanup", setting: "cleanupModel", title: "Cleanup" },
  { id: "notes", setting: "notesModel", title: "Notes" },
] as const;

export function formatBytes(value: number | null): string {
  if (value === null) return "Unavailable";
  if (value === 0) return "0 B";
  return value >= 1e9
    ? `${(value / 1e9).toFixed(1)} GB`
    : `${Math.ceil(value / 1e6)} MB`;
}
export function memoryRange(min: number | null, max: number | null): string {
  if (min === null || max === null) return "Unrated";
  return min === max
    ? formatBytes(min)
    : `${formatBytes(min)}–${formatBytes(max)}`;
}
export function installed(model: ModelEntry): boolean {
  return [
    "installed",
    "unloaded",
    "installed/unloaded",
    "installed_unloaded",
    "loading",
    "ready",
    "busy",
    "unloading",
  ].includes(model.state);
}
export function installing(model: ModelEntry): boolean {
  return [
    "queued",
    "downloading",
    "verifying",
    "installing",
    "cancelling",
    "canceling",
  ].includes(model.state);
}
/** Background inventory checks are not installations or proof of readiness. */
export function checking(model: ModelEntry): boolean {
  return model.state === "checking";
}
export function deviceLabel(model: ModelEntry): string {
  if (model.id === "local-rules") return "CPU · no AI model";
  if (/metal/i.test(model.backend))
    return model.kind === "asr"
      ? "Apple GPU · Metal"
      : "CPU or Apple GPU · Metal";
  if (model.kind === "cleanup" && /prism|bonsai/i.test(model.backend))
    return "NVIDIA CUDA only · Prism runtime";
  if (model.kind === "cleanup") return "CPU or NVIDIA CUDA";
  if (/cuda|nvidia/i.test(model.backend)) return "NVIDIA CUDA package";
  return /cpu/i.test(model.backend)
    ? "CPU only"
    : model.backend || "Device unverified";
}
export function shortDeviceLabel(model: ModelEntry): string {
  if (model.id === GRANITE_SPEECH_MODEL) return "NVIDIA CUDA / Apple Metal";
  const label = deviceLabel(model);
  if (/metal/i.test(model.backend)) return "Apple Metal";
  if (model.kind === "cleanup" && /prism|bonsai/i.test(model.backend))
    return "NVIDIA CUDA only";
  if (model.kind === "cleanup" && model.id !== "local-rules")
    return "CPU or NVIDIA CUDA";
  if (/cuda|nvidia/i.test(label)) return "NVIDIA CUDA";
  return /cpu/i.test(label) ? "CPU" : "Device unknown";
}
/** Speech may overlap one text runtime; Cleanup and Notes never add together. */
export function planRange(
  models: (ModelEntry | undefined)[],
  sequential: boolean,
): string {
  // Auxiliary helpers are outside the selected Speech/Text plan.
  const planned = models.filter(
    (model) => model?.kind !== "diarization" && model?.kind !== "punctuation",
  );
  if (
    planned.some(
      (model) =>
        !model ||
        (model.id !== "local-rules" &&
          (model.ramMinBytes === null || model.ramMaxBytes === null)),
    )
  )
    return "Unrated";
  const distinct = [
    ...new Map(
      planned
        .filter((m): m is ModelEntry => !!m && m.id !== "local-rules")
        .map((m) => [m.id, m]),
    ).values(),
  ];
  const combine = (key: "ramMinBytes" | "ramMaxBytes") => {
    const speech = Math.max(
      0,
      ...distinct.filter((m) => m.kind === "asr").map((m) => m[key] ?? 0),
    );
    const text = Math.max(
      0,
      ...distinct.filter((m) => m.kind === "cleanup").map((m) => m[key] ?? 0),
    );
    return sequential ? Math.max(speech, text) : speech + text;
  };
  return memoryRange(combine("ramMinBytes"), combine("ramMaxBytes"));
}

export function selectable(
  model: ModelEntry,
  role: ModelSelectionRole,
): boolean {
  return model.roles.includes(role) && installed(model);
}

/** Availability and the reason come from the native catalog, never a GPU guess. */
export function modelUnavailableReason(model: ModelEntry): string | null {
  if (
    model.state === "unavailable" ||
    model.state === "unsupported" ||
    (!model.installable &&
      !installed(model) &&
      !checking(model) &&
      !installing(model))
  )
    return (
      model.error || model.compatibility || "Unavailable on this computer."
    );
  return null;
}

export function removalBlock(
  model: ModelEntry,
  selectedIds: readonly string[],
): string | null {
  if (checking(model)) return "Checking…";
  if (selectedIds.includes(model.id) || model.usedBy.length)
    return "Selected model";
  if (["loading", "busy", "unloading"].includes(model.state))
    return "Model in use";
  if (installing(model)) return "Installation in progress";
  return null;
}
