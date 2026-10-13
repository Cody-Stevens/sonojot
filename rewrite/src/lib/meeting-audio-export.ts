// Original meeting audio export — pure frontend helpers (Round 6).
// No Tauri imports here so `tsx --test` can cover the Save-copy rules
// without a native host. The Svelte component owns the dialog + invoke.

export const AUDIO_EXPORT_COMMAND = "export_meeting_audio";
export const AUDIO_EXPORT_FALLBACK_COMMAND = "export_meeting";
export const AUDIO_EXPORT_FORMAT = "audio";
export const AUDIO_EXPORT_EXTENSION = "wav";
export const AUDIO_EXPORT_LABEL = "Original recording";
export const AUDIO_EXPORT_FILTER_LABEL = "Original recording (WAV)";
export const AUDIO_EXPORT_DIALOG_TITLE = "Save original recording";

export function audioExportFilename(title: string): string {
  const base =
    title
      .replace(/[<>:\"/\\|?*\u0000-\u001f]/g, "-")
      .replace(/[. ]+$/g, "")
      .slice(0, 100) || "Meeting";
  return `${base}.${AUDIO_EXPORT_EXTENSION}`;
}

export function audioExportSaveOptions(title: string): {
  dialogTitle: string;
  defaultPath: string;
  filters: { name: string; extensions: string[] }[];
} {
  return {
    dialogTitle: AUDIO_EXPORT_DIALOG_TITLE,
    defaultPath: audioExportFilename(title),
    filters: [
      { name: AUDIO_EXPORT_FILTER_LABEL, extensions: [AUDIO_EXPORT_EXTENSION] },
    ],
  };
}

export interface AudioExportState {
  status: string;
  hasAudio: boolean;
  busy: boolean;
  native: boolean;
}

/** Save-copy audio stays enabled while transcription/notes process; only an
 * unfinished recording, missing audio, or an in-flight save disables it.
 * Unsaved text drafts never block audio-only export (by design, no `dirty`). */
export function canExportAudio(state: AudioExportState): {
  enabled: boolean;
  reason: string;
} {
  if (!state.native)
    return {
      enabled: false,
      reason: "Open the desktop app to save the original recording.",
    };
  if (state.status === "recording")
    return {
      enabled: false,
      reason: "Stop the recording before exporting its audio.",
    };
  if (!state.hasAudio)
    return {
      enabled: false,
      reason: "No saved audio for this meeting yet.",
    };
  if (state.busy) return { enabled: false, reason: "Saving…" };
  return { enabled: true, reason: "" };
}

/** Shape for the dedicated command. Falls back to `export_meeting` with
 * `format: "audio"` only if the new command is unavailable (same bytes). */
export function audioExportInvokeArgs(
  id: string,
  path: string,
): { id: string; path: string } {
  return { id, path };
}

export function audioExportFallbackArgs(
  id: string,
  path: string,
): { id: string; path: string; format: string } {
  return { id, path, format: AUDIO_EXPORT_FORMAT };
}

/** Cancel (save dialog returns null) is a no-op: the caller must not invoke. */
export function shouldInvokeAudioExport(
  dialogResult: string | null,
): dialogResult is string {
  return typeof dialogResult === "string" && dialogResult.length > 0;
}

export function audioExportErrorMessage(error: unknown): string {
  return error instanceof Error
    ? error.message
    : typeof error === "string"
      ? error
      : "The original recording could not be saved. Please try again.";
}
