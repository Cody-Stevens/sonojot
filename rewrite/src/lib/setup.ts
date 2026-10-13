import { z } from "zod";
import type { Snapshot } from "./protocol";

export const setupStateSchema = z.enum([
  "checking",
  "missing",
  "installed",
  "loading",
  "ready",
  "failed",
  "unsupported",
]);
export const setupCapabilitySchema = z.object({
  state: setupStateSchema,
  detail: z.string().nullable(),
});
export const setupStatusSchema = z.object({
  microphone: setupCapabilitySchema,
  speech: setupCapabilitySchema,
  shortcut: setupCapabilitySchema,
  cleanup: setupCapabilitySchema,
  mediaImport: setupCapabilitySchema,
  modelsPath: z.string(),
});
export type SetupStatus = z.infer<typeof setupStatusSchema>;
export type SetupCapability = z.infer<typeof setupCapabilitySchema>;
export type SetupKind = Exclude<keyof SetupStatus, "modelsPath">;

const available = (capability: SetupCapability): boolean =>
  capability.state === "installed" || capability.state === "ready";
const needsRepair = (capability: SetupCapability): boolean =>
  capability.state === "missing" ||
  capability.state === "failed" ||
  capability.state === "unsupported";

/** A listening microphone does not prove that speech-model initialization finished. */
export function selectSetup(input: {
  status: SetupStatus | null;
  snapshot: Pick<Snapshot, "state" | "meetingRecording">;
  native: boolean;
}) {
  const { status, snapshot, native } = input;
  const captureActive =
    snapshot.state === "loading" || snapshot.state === "listening";
  const finishing =
    snapshot.state === "finishing" || snapshot.state === "cleaning";
  const canConfigure =
    native && !captureActive && !finishing && !snapshot.meetingRecording;
  return {
    captureActive,
    finishing,
    canConfigure,
    canStart:
      canConfigure &&
      status !== null &&
      available(status.microphone) &&
      available(status.speech),
    keepCaptureControls: captureActive || finishing,
    requiredNeedsAttention:
      status !== null &&
      (needsRepair(status.microphone) ||
        needsRepair(status.speech) ||
        status.shortcut.state !== "ready"),
  };
}

export function setupLabel(
  kind: SetupKind,
  capability: SetupCapability | undefined,
): string {
  if (!capability) return "Checking…";
  if (capability.state === "installed") {
    if (kind === "microphone") return "Available";
    return "Installed";
  }
  const labels = {
    checking: "Checking…",
    missing: kind === "mediaImport" ? "Not installed" : "Needs setup",
    loading: kind === "mediaImport" ? "Installing media support…" : "Preparing…",
    ready: kind === "shortcut" ? "Registered" : "Ready",
    failed: "Needs attention",
    unsupported: "Unavailable here",
  };
  return labels[capability.state];
}

export function setupHelp(
  kind: SetupKind,
  capability: SetupCapability | undefined,
): string {
  if (!capability) return "Checking this computer.";
  const state = capability.state;
  if (state === "checking") return "Checking installed files in the background.";
  if (kind === "mediaImport") {
    if (state === "ready" || state === "installed")
      return "The converter is ready to use.";
    if (state === "loading") return "Installing media support in the background.";
    if (state === "unsupported")
      return "Automatic install is unavailable on this system. Locate an existing FFmpeg to convert recordings.";
    if (state === "failed")
      return "The configured converter could not be used. Install media support or locate an existing FFmpeg.";
    return "Install media support or locate an existing FFmpeg.";
  }
  if (state === "unsupported")
    return "This capability is unavailable on this computer.";
  if (kind === "microphone") {
    if (state === "ready") return "Your microphone is capturing audio.";
    if (state === "installed")
      return "The selected microphone is available. Test it with a short dictation.";
    if (state === "loading") return "Opening your microphone.";
    return "Connect a microphone, then choose it below.";
  }
  if (kind === "speech") {
    if (state === "ready")
      return "The speech model has produced text in this recording.";
    if (state === "installed")
      return "Transcription loads when you start dictation.";
    if (state === "loading") return "Preparing transcription.";
    return "Locate the supported models folder to use local transcription.";
  }
  if (kind === "shortcut") {
    if (state === "ready") return "The shortcut is registered for dictation.";
    return "Choose a shortcut that is available. You can still record with the button.";
  }
  if (kind === "cleanup") {
    if (state === "ready")
      return "Local AI helps with punctuation and sentence boundaries.";
    if (state === "installed")
      return "Local AI helps with punctuation and sentence boundaries. Loads during dictation.";
    if (state === "loading")
      return "Preparing enhanced cleanup. Local formatting remains available.";
    return "Formatting and vocabulary work without enhanced cleanup. A graphics card is optional.";
  }
  return "This capability is unavailable on this computer.";
}
