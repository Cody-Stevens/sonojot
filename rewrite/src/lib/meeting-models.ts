import type { AiSettings } from "./ai";
import { selectable, type ModelEntry } from "./model-library";
import type { Settings } from "./protocol";

type MeetingSettings = Pick<Settings, "asrModel" | "meetingAsrModel">;
type SpeechRoute = Pick<AiSettings, "connected" | "speechModel">;

export function meetingSpeechModels(models: ModelEntry[]): ModelEntry[] {
  return models.filter(
    (model) => selectable(model, "asr") || selectable(model, "meetingSpeech"),
  );
}

/** Keep an explicit local choice even when unavailable; never silently replace it. */
export function defaultMeetingModel(
  settings: MeetingSettings,
  online: SpeechRoute | null,
): string {
  return (
    settings.meetingAsrModel ||
    (online?.connected && online.speechModel ? "openrouter" : settings.asrModel)
  );
}

/**
 * Speaker echo removal: a saved choice wins; without one it is on only for
 * recognizers that report word times (SonoJot Parakeet on the CPU). The
 * backend applies the same rule from the recognition it actually got.
 */
export function speakerEchoRemoval(
  choice: boolean | null,
  meetingModel: string,
): { on: boolean; automatic: boolean } {
  if (choice !== null) return { on: choice, automatic: false };
  return {
    on: meetingModel.startsWith("parakeet") && !meetingModel.endsWith("-cuda"),
    automatic: true,
  };
}

export type MeetingModelChoice =
  { kind: "local"; id: string } | { kind: "online"; id: string };

export function meetingModelSettings(choice: MeetingModelChoice): {
  meetingAsrModel: string;
  speechModel: string;
} {
  return choice.kind === "local"
    ? { meetingAsrModel: choice.id, speechModel: "" }
    : { meetingAsrModel: "", speechModel: choice.id };
}

/** The two existing settings owners save separately. Restore the first if the second fails. */
export async function saveMeetingModelChoice(
  choice: MeetingModelChoice,
  previousSpeechModel: string,
  operations: {
    saveOnline: (speechModel: string) => Promise<void>;
    saveLocal: (meetingAsrModel: string) => Promise<void>;
  },
): Promise<void> {
  const next = meetingModelSettings(choice);
  const changedOnline = next.speechModel !== previousSpeechModel;
  if (changedOnline) await operations.saveOnline(next.speechModel);
  try {
    await operations.saveLocal(next.meetingAsrModel);
  } catch (failure: unknown) {
    if (changedOnline) {
      try {
        await operations.saveOnline(previousSpeechModel);
      } catch {
        throw new Error(
          "The meeting model could not be saved or its previous online route restored. Check the Meeting transcription choice.",
        );
      }
    }
    throw failure;
  }
}
