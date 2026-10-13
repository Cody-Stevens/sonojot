import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  parseResponse,
  snapshotSchema,
  type CommandArguments,
  type CommandName,
  type CommandResult,
  type Snapshot,
} from "./protocol";

export const native = isTauri();
export async function call<C extends CommandName>(
  name: C,
  ...args: CommandArguments[C] extends undefined ? [] : [CommandArguments[C]]
): Promise<CommandResult<C>> {
  if (!native)
    throw new Error(
      "This action is available in the desktop app. Browser preview does not record audio or save app data.",
    );
  const result: unknown = await invoke(name, args[0]);
  return parseResponse(name, result);
}
export async function subscribe(
  onSnapshot: (value: Snapshot) => void,
  onError: (error: unknown) => void,
): Promise<() => void> {
  return listen<unknown>("app-state", ({ payload }) => {
    const result = snapshotSchema.safeParse(payload);
    if (result.success) onSnapshot(result.data);
    else
      onError(
        new Error(
          "A desktop update could not be read. Reopen SonoJot to reconnect.",
        ),
      );
  });
}
/** The backend emits this after a saved publication, deletion or correction. */
export async function subscribeDictationHistory(
  onChange: () => void,
): Promise<() => void> {
  return listen("dictation-history-changed", () => onChange());
}
/** The backend emits this when a background media support install starts and ends. */
export async function subscribeMediaSupport(
  onChange: () => void,
): Promise<() => void> {
  return listen("media-support-changed", () => onChange());
}
export async function chooseMedia(): Promise<string | null> {
  if (!native)
    throw new Error("Open the desktop app to import an audio or video file.");
  const path = await open({
    title: "Import a recording",
    multiple: false,
    directory: false,
    filters: [
      {
        name: "Audio and video",
        extensions: [
          "wav",
          "mp3",
          "m4a",
          "aac",
          "flac",
          "ogg",
          "opus",
          "mp4",
          "mov",
          "mkv",
          "webm",
          "wma",
        ],
      },
    ],
  });
  return typeof path === "string" ? path : null;
}
export async function chooseModelsFolder(): Promise<string | null> {
  if (!native) throw new Error("Open the desktop app to locate your models.");
  const path = await open({
    title: "Locate an existing models folder",
    directory: true,
    multiple: false,
  });
  return typeof path === "string" ? path : null;
}
export async function chooseConverter(): Promise<string | null> {
  if (!native)
    throw new Error("Open the desktop app to locate the media converter.");
  const path = await open({
    title: "Locate FFmpeg",
    directory: false,
    multiple: false,
  });
  return typeof path === "string" ? path : null;
}
export async function chooseExport(
  defaultPath: string,
  extension: string,
): Promise<string | null> {
  if (!native) throw new Error("Open the desktop app to export a meeting.");
  return save({
    title: "Export meeting",
    defaultPath,
    filters: [{ name: "Meeting document", extensions: [extension] }],
  });
}
export async function chooseAudioExport(
  defaultPath: string,
): Promise<string | null> {
  if (!native)
    throw new Error("Open the desktop app to save the original recording.");
  return save({
    title: "Save original recording",
    defaultPath,
    filters: [{ name: "Original recording (WAV)", extensions: ["wav"] }],
  });
}
