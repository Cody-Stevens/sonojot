import { app } from "./app.svelte";
import { ai } from "./ai.svelte";
import { call } from "./api";
import { library } from "./model-library.svelte";
import { ensureDownloadConsent } from "./download-consent.svelte";
import {
  checking,
  installed,
  installing,
  modelUnavailableReason,
  type ModelEntry,
} from "./model-library";
import { saveMeetingModelChoice } from "./meeting-models";
import { errorMessage } from "./protocol";
type SpeechRole = "dictation" | "meeting" | "helper";

class SpeechChoice {
  pending = $state("");
  pendingRole = $state<SpeechRole | null>(null);
  failure = $state<{ id: string; message: string; role: SpeechRole } | null>(
    null,
  );
  private cancelled = false;
  get captureBusy() {
    return (
      app.snapshot.meetingRecording ||
      !["idle", "error"].includes(app.snapshot.state)
    );
  }
  get locked() {
    return (
      !app.native ||
      !app.ready ||
      this.captureBusy ||
      app.busy("settings") ||
      app.busy("setup-profile") ||
      app.busy("meeting-model") ||
      ai.busy ||
      library.data?.models.some((model) =>
        ["loading", "busy", "unloading"].includes(model.state),
      ) ||
      !!this.pending
    );
  }
  async use(model: ModelEntry, role: SpeechRole) {
    if (
      this.locked ||
      checking(model) ||
      modelUnavailableReason(model) ||
      library.data?.models.some(installing)
    )
      return;
    this.pending = model.id;
    this.pendingRole = role;
    this.cancelled = false;
    this.failure = null;
    const release = library.acquire();
    try {
      let current = model;
      if (!installed(current)) {
        if (
          !(await ensureDownloadConsent([model.id])) ||
          this.cancelled ||
          this.captureBusy
        )
          return;
        library.downloadId = model.id;
        library.apply(await call("start_model_install", { id: model.id }));
        current = library.data!.models.find((entry) => entry.id === model.id)!;
        while (
          current &&
          (checking(current) || installing(current)) &&
          !this.cancelled
        ) {
          await new Promise((resolve) => setTimeout(resolve, 750));
          library.apply(await call("get_model_library"));
          current = library.data!.models.find(
            (entry) => entry.id === model.id,
          )!;
        }
      }
      if (this.cancelled) return;
      if (!current || !installed(current))
        throw new Error(
          current?.error || "Download stopped. Retry to use this model.",
        );
      if (this.captureBusy || app.busy("setup-profile")) {
        app.notice = "Model installed. Choose it after recording has stopped.";
        return;
      }
      if (role === "dictation")
        await app.saveSettings({ asrModel: model.id, setupProfile: "custom" });
      if (role === "meeting") await this.saveMeeting(model.id);
      await library.refresh();
    } catch (failure: unknown) {
      this.failure = { id: model.id, message: errorMessage(failure), role };
    } finally {
      this.pending = "";
      this.pendingRole = null;
      release();
    }
  }
  async saveMeeting(id: string) {
    if (!ai.data) await ai.refresh();
    if (!ai.data)
      throw new Error("The online speech choice could not be read. Try again.");
    await saveMeetingModelChoice({ kind: "local", id }, ai.data.speechModel, {
      saveOnline: async (speechModel) => {
        if (!(await ai.save({ speechModel })))
          throw new Error(
            app.error || "Could not save the online speech choice.",
          );
      },
      saveLocal: async (meetingAsrModel) => {
        await app.saveSettings({ meetingAsrModel });
        if (app.snapshot.settings.meetingAsrModel !== meetingAsrModel)
          throw new Error(app.error || "Could not save meeting speech.");
      },
    });
  }
  async sameAsDictation() {
    if (this.locked) return;
    await app.perform("meeting-model", () => this.saveMeeting(""));
    await library.refresh();
  }
  async cancel(id: string) {
    this.cancelled = true;
    await app.perform("model-cancel", async () =>
      library.apply(await call("cancel_model_install", { id })),
    );
  }
}
export const speechChoice = new SpeechChoice();
