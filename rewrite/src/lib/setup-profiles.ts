import { z } from "zod";
import {
  checking,
  installed,
  installing,
  memoryRange,
  modelRoles,
  type ModelEntry,
  type ModelLibrary,
} from "./model-library";
import type { Settings, Snapshot } from "./protocol";
import { PUNCTUATION_MODEL, profileIncludesPunctuation } from "./punctuation";

export const setupProfileSchema = z.enum([
  "accurate", "balanced", "light", "custom",
]);
export type SetupProfile = z.infer<typeof setupProfileSchema>;
export type AutomaticProfile = Exclude<SetupProfile, "custom">;
const bytes = z.number().finite().nonnegative().nullable();
const cores = z.number().int().positive().nullable();
export const profileSelectionSchema = z.object({
  asrModel: z.string(),
  cleanupModel: z.string().min(1),
  notesModel: z.string().min(1),
  reasons: z.array(z.string()),
});
export type ProfileSelection = z.infer<typeof profileSelectionSchema>;
export const hardwareProfileSchema = z.object({
  hardware: z.object({
    platform: z.enum(["windows", "macos", "unknown"]),
    appleSilicon: z.boolean(),
    totalRamBytes: bytes,
    physicalCores: cores,
    logicalCores: cores,
    gpuName: z.string().nullable(),
    gpuDriverVersion: z.string().nullable(),
    gpuComputeCapability: z.string().nullable(),
    totalVramBytes: bytes,
    cudaOk: z.boolean(),
  }),
  recommendations: z.object({
    accurate: profileSelectionSchema,
    balanced: profileSelectionSchema,
    light: profileSelectionSchema,
  }),
});
export type HardwareProfile = z.infer<typeof hardwareProfileSchema>;

export const setupProfiles = [
  { id: "accurate", title: "Accurate", subtitle: "Best transcription, more memory" },
  { id: "balanced", title: "Balanced", subtitle: "Accurate and responsive" },
  { id: "light", title: "Light", subtitle: "Fastest start, least memory" },
  { id: "custom", title: "Custom", subtitle: "Choose every model yourself" },
] as const satisfies readonly { id: SetupProfile; title: string; subtitle: string }[];

export function profileLabel(profile: SetupProfile | null): string {
  return setupProfiles.find((item) => item.id === profile)?.title ?? "";
}

export function plainProfileReason(profile: AutomaticProfile, speech: ModelEntry | undefined): string {
  const name = profileLabel(profile);
  if (!speech) return `${name} balances speech accuracy with the memory available on this computer.`;
  if (profile === "light") return "Light uses a smaller speech model to leave more memory for other apps.";
  if (/qwen/i.test(speech.id) && /metal|cuda/i.test(speech.backend))
    return `${name} runs Qwen3-ASR on the graphics chip and leaves room for other apps.`;
  if (/qwen/i.test(speech.id)) return `${name} uses Qwen3-ASR for accuracy on this computer's processor.`;
  return `${name} uses a smaller speech model that fits this computer's memory.`;
}

export function selectedProfile(
  saved: SetupProfile | null,
  draft: SetupProfile | null,
): SetupProfile {
  return draft ?? saved ?? "balanced";
}

export function showCustomModels(saved: SetupProfile | null, draft: SetupProfile | null): boolean {
  return selectedProfile(saved, draft) === "custom";
}

export function hardwareLine(hardware: HardwareProfile["hardware"]): string {
  const capacity = (value: number) => `${Math.round(value / 2 ** 30)} GB`;
  const parts = [hardware.appleSilicon ? "Apple Silicon"
    : hardware.gpuName ?? (hardware.platform === "windows"
      ? "Graphics details unavailable" : hardware.platform === "macos" ? "Mac" : "This computer")];
  if (!hardware.appleSilicon && hardware.gpuName) {
    parts.push(hardware.totalVramBytes === null ? "Graphics memory unknown"
      : `${capacity(hardware.totalVramBytes)} graphics memory`);
    if (!hardware.cudaOk) parts.push("NVIDIA compatibility unconfirmed");
  }
  parts.push(hardware.totalRamBytes === null
    ? "Memory unavailable"
    : `${capacity(hardware.totalRamBytes)} ${hardware.appleSilicon ? "unified memory" : "RAM"}`);
  if (hardware.physicalCores !== null) parts.push(`${hardware.physicalCores} cores`);
  else if (hardware.logicalCores !== null) parts.push(`${hardware.logicalCores} logical cores`);
  else parts.push("Core count unknown");
  return parts.join(" · ");
}

export function selectionModels(selection: ProfileSelection, models: ModelEntry[]): (ModelEntry | undefined)[] {
  return modelRoles.map((role) => models.find((model) => model.id === selection[role.setting]));
}

/** Shared role packages count once. Unknown catalog entries keep the total unknown. */
export function missingDownloadBytes(selection: ProfileSelection, models: ModelEntry[], includePunctuation = false): number | null {
  let total = 0;
  const ids = new Set(modelRoles.map((role) => selection[role.setting]));
  if (includePunctuation) ids.add(PUNCTUATION_MODEL);
  for (const id of ids) {
    const model = models.find((entry) => entry.id === id);
    if (!model || checking(model)) return null;
    if (id === PUNCTUATION_MODEL && !model.installable) continue;
    if (!installed(model)) total += model.totalBytes || model.diskBytes;
  }
  return total;
}

export function sameProfileModels(left: ProfileSelection, right: ProfileSelection): boolean {
  return modelRoles.every((role) => left[role.setting] === right[role.setting]);
}

type RoleSelection = Pick<Settings, "asrModel" | "cleanupModel" | "notesModel">;
export type ModelRoleId = (typeof modelRoles)[number]["id"];
/** One role where the saved choice differs from the profile's current pick. */
export interface ProfileChange {
  role: ModelRoleId;
  recommended: string;
  current: string;
}

/**
 * What a profile tab may claim. "in-use" only when every role the profile sets
 * matches its current recommendation; "changed" when the saved profile's
 * recommendation moved since setup (an update can change it); "unknown" while
 * the recommendation for this computer has not been read.
 */
export type ProfileCardState =
  | { kind: "available" }
  | { kind: "unknown" }
  | { kind: "in-use" }
  | { kind: "changed"; changes: ProfileChange[] };

export function profileChanges(settings: RoleSelection, recommendation: RoleSelection): ProfileChange[] {
  return modelRoles
    .filter((role) => settings[role.setting] !== recommendation[role.setting])
    .map((role) => ({
      role: role.id,
      recommended: recommendation[role.setting],
      current: settings[role.setting],
    }));
}

export function profileCardState(options: {
  saved: SetupProfile | null;
  selected: SetupProfile;
  unfinished: boolean;
  settings: RoleSelection;
  recommendation: RoleSelection | null | undefined;
  /**
   * Automatic profiles name local Notes models, so OpenRouter Notes never
   * match. Null while the Notes route has not been read.
   */
  localNotes: boolean | null;
}): ProfileCardState {
  const { saved, selected, unfinished, settings, recommendation, localNotes } = options;
  if (unfinished || selected === "custom" || saved !== selected || localNotes === false) return { kind: "available" };
  if (!recommendation || localNotes === null) return { kind: "unknown" };
  const changes = profileChanges(settings, recommendation);
  return changes.length ? { kind: "changed", changes } : { kind: "in-use" };
}

const roleWords: Record<ModelRoleId, string> = { asr: "speech", cleanup: "cleanup", notes: "notes" };

function listWords(items: string[]): string {
  return items.length < 3
    ? items.join(" and ")
    : `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

/**
 * Plain card text for a saved profile whose recommendation changed, e.g.
 * "Balanced now uses Qwen3-ASR 1.7B (Q4 Metal) for speech. You're using
 * Whisper Small · Metal."
 */
export function profileChangeText(
  profile: AutomaticProfile,
  changes: ProfileChange[],
  name: (id: string) => string,
): string {
  if (!changes.length) return "";
  const uses = listWords(changes.map((change) => `${name(change.recommended)} for ${roleWords[change.role]}`));
  const current = listWords(changes.map((change) => name(change.current)));
  return `${profileLabel(profile)} now uses ${uses}. You're using ${current}.`;
}

/** Install state words for a profile row, matching default Cleanup's words. */
export function modelInstallStatus(model: ModelEntry | undefined): string {
  if (!model) return "";
  if (checking(model)) return "Checking…";
  if (installed(model)) return "Installed";
  if (installing(model)) return "Downloading";
  return "Not installed";
}

// Exact Windows packages only. Worker peaks in decimal GB, not planning bounds.
// Source: rewrite/evaluation/BENCHMARK-WINDOWS-2026-09-28.md, app-worker measurements.
// Do not extend these observations to Moonshine Small or other Whisper variants.
const measuredSpeech = new Map<string, { ramGB: number; graphicsGB?: number }>([
  ["moonshine-medium", { ramGB: 0.56 }],
  ["whisper-small", { ramGB: 1.28 }],
  ["whisper-tiny.en", { ramGB: 0.53 }],
  ["qwen3-asr-1.7b-q8-cuda", { ramGB: 1, graphicsGB: 3.4 }],
  ["qwen3-asr-1.7b-q4-cpu", { ramGB: 2.14 }],
]);

export function profileSpeechMemory(
  speech: ModelEntry | undefined,
  platform: HardwareProfile["hardware"]["platform"] | undefined,
): { label: string; detail: string } {
  if (!speech) return { label: "Speech memory estimate unavailable", detail: "Model details have not been read yet." };
  const measured = platform === "windows" ? measuredSpeech.get(speech.id) : undefined;
  if (measured) return {
    label: `Speech about ${Number(measured.ramGB.toFixed(1))} GB RAM${measured.graphicsGB ? ` + ${measured.graphicsGB} GB graphics memory` : ""}`,
    detail: `September 28, 2026 Windows benchmark: ${measured.ramGB} GB peak worker RAM${measured.graphicsGB ? ` and about ${measured.graphicsGB} GB graphics memory` : ""}. Measured on one computer; memory varies with hardware and audio. Planning includes additional headroom. Source: BENCHMARK-WINDOWS-2026-09-28.md.`,
  };
  const memory = memoryRange(speech.ramMinBytes, speech.ramMaxBytes);
  const graphics = (speech.vramMaxBytes ?? 0) > 0 || /cuda/i.test(speech.backend)
    ? memoryRange(speech.vramMinBytes, speech.vramMaxBytes) : null;
  return {
    label: memory === "Unrated" ? "Speech memory estimate unavailable"
      : `Speech about ${memory} ${/metal/i.test(speech.backend) ? "unified memory" : "RAM"}${graphics ? ` + ${graphics === "Unrated" ? "unknown" : graphics} graphics memory` : ""}`,
    detail: `Estimated range, not a measured worker peak. ${speech.memoryBasis} ${speech.evidence}`,
  };
}

export interface ProfileOperations {
  readLibrary: () => Promise<ModelLibrary>;
  install: (id: string) => Promise<ModelLibrary>;
  wait: () => Promise<void>;
  apply: (profile: SetupProfile) => Promise<Snapshot>;
  complete: () => Promise<void>;
}

/** First-run automatic setup includes punctuation. Returns a Notes ID if its check is pending. */
export async function applyProfileSelection(
  profile: SetupProfile,
  selection: Pick<Settings, "asrModel">,
  operations: ProfileOperations,
  savedProfile: SetupProfile | null,
): Promise<string | undefined> {
  let catalog = await operations.readLibrary();
  let speech = catalog.models.find((model) => model.id === selection.asrModel);
  // A checking row may already exist on disk. Wait before deciding to install.
  while (speech && checking(speech)) {
    await operations.wait();
    catalog = await operations.readLibrary();
    speech = catalog.models.find((model) => model.id === selection.asrModel);
  }
  if (!speech) throw new Error("The selected speech model is not in the model catalog. Check again in Setup.");
  if (!installed(speech)) {
    if (profile === "custom") throw new Error("Install your chosen speech model before finishing setup.");
    if (!installing(speech)) {
      if (!speech.installable) throw new Error("The selected speech model cannot be installed on this computer.");
      catalog = await operations.install(speech.id);
      speech = catalog.models.find((model) => model.id === selection.asrModel);
    }
    while (speech && (installing(speech) || checking(speech))) {
      await operations.wait();
      catalog = await operations.readLibrary();
      speech = catalog.models.find((model) => model.id === selection.asrModel);
    }
    if (!speech || !installed(speech)) {
      throw new Error(speech?.error || "Speech model installation stopped. Retry the download to finish setup.");
    }
  }
  if (profileIncludesPunctuation(savedProfile, profile)) {
    // Install sequentially through the same owner. A cancel/failure keeps Setup
    // unfinished so Retry can resume it without replacing saved choices.
    catalog = await operations.readLibrary();
    let punctuation = catalog.models.find((model) => model.id === PUNCTUATION_MODEL);
    while (punctuation && checking(punctuation)) {
      await operations.wait();
      catalog = await operations.readLibrary();
      punctuation = catalog.models.find((model) => model.id === PUNCTUATION_MODEL);
    }
    if (!punctuation) throw new Error("The punctuation model is not in the model catalog. Check again in Setup.");
    // Unsupported targets retain the existing rule fallback.
    if (punctuation.installable && !installed(punctuation)) {
      if (!installing(punctuation)) {
        catalog = await operations.install(punctuation.id);
        punctuation = catalog.models.find((model) => model.id === PUNCTUATION_MODEL);
      }
      while (punctuation && (installing(punctuation) || checking(punctuation))) {
        await operations.wait();
        catalog = await operations.readLibrary();
        punctuation = catalog.models.find((model) => model.id === PUNCTUATION_MODEL);
      }
      if (!punctuation || !installed(punctuation)) {
        throw new Error(punctuation?.error || "Punctuation model installation stopped. Retry the download to finish setup.");
      }
    }
  }
  const snapshot = await operations.apply(profile);
  await operations.complete();
  if (profile === "custom") return;
  // Use the committed recommendation, in case the explicit apply refreshed hardware.
  try {
    catalog = await operations.readLibrary();
    const notes = catalog.models.find((model) => model.id === snapshot.settings.notesModel);
    if (!notes) throw new Error("The selected notes model is not in the model catalog. Check again in Setup.");
    // Setup is already saved. Let the caller follow this check without holding
    // the profile/settings operation open or offering a duplicate download.
    if (checking(notes)) return notes.id;
    if (!installed(notes) && !installing(notes)) {
      if (!notes.installable) throw new Error("The selected notes model cannot be installed on this computer.");
      await operations.install(notes.id);
    }
  } catch (failure: unknown) {
    throw new Error(`SETUP_NOTES_DOWNLOAD: ${failure instanceof Error ? failure.message : String(failure)}`);
  }
}
