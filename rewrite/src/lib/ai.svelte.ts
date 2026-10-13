import { call } from "./api";
import { app } from "./app.svelte";
import { library } from "./model-library.svelte";
import { selectable } from "./model-library";
import { settingsSchema } from "./protocol";
import { SMART_CLEANUP_MODEL } from "./smart-cleanup";
import type { AiSettings, AiSettingsPatch } from "./ai";

class AiState {
  data = $state<AiSettings | null>(null);
  private pending = $state(0);
  private tail: Promise<unknown> = Promise.resolve();
  private refreshTask: Promise<void> | null = null;

  get busy(): boolean {
    return this.pending > 0;
  }

  private enqueue<T>(
    key: string,
    action: () => Promise<T>,
  ): Promise<T | undefined> {
    this.pending++;
    // Resolve patches only when their turn starts, against the latest saved data.
    const task = this.tail.then(() => app.perform(key, action));
    this.tail = task.finally(() => {
      this.pending--;
    });
    return task;
  }

  refresh(): Promise<void> {
    if (!app.native) return Promise.resolve();
    if (this.refreshTask) return this.refreshTask;
    this.refreshTask = this.enqueue("ai-settings", async () => {
      // Keep the last saved state if a later refresh fails.
      this.data = await call("get_ai_settings");
    }).then(() => {
      this.refreshTask = null;
    });
    return this.refreshTask;
  }

  async save(patch: AiSettingsPatch): Promise<boolean> {
    if (!app.native) return false;
    // Copy list entries too: callers must not mutate a queued save afterwards.
    const requested = {
      ...patch,
      ...(patch.quickModels
        ? { quickModels: patch.quickModels.map((item) => ({ ...item })) }
        : {}),
    };
    const saved = await this.enqueue("ai-settings", async () => {
      const current = this.data ?? (await call("get_ai_settings"));
      this.data = current;
      const next = {
        backend: requested.backend ?? current.backend,
        model: requested.model ?? current.model,
        speechModel: requested.speechModel ?? current.speechModel,
        quickModels: requested.quickModels ?? current.quickModels,
      };
      if (requested.backend === "local") await this.restoreLocalNotes();
      this.data = await call("save_ai_settings", {
        settings: next,
      });
      return true;
    });
    return saved === true;
  }

  async connect(): Promise<void> {
    if (!app.native) return;
    await this.enqueue("ai-connect", async () => {
      if (this.data?.connected) return;
      this.data = await call("connect_openrouter");
    });
  }

  async disconnect(): Promise<void> {
    if (!app.native) return;
    await this.enqueue("ai-settings", async () => {
      await this.restoreLocalNotes();
      this.data = await call("disconnect_openrouter");
    });
  }

  private async restoreLocalNotes(): Promise<void> {
    if (app.snapshot.settings.notesModel !== "openrouter") return;
    if (app.busy("settings"))
      throw new Error("Wait for the current settings change, then try again.");
    const defaultId = settingsSchema.shape.notesModel.parse(undefined);
    const available = library.data?.models.filter(
      (model) => model.id !== "openrouter" && selectable(model, "notes"),
    );
    const snapshotModels = app.snapshot.models.filter(
      (model) =>
        model.kind === "cleanup" &&
        model.ready &&
        model.id !== SMART_CLEANUP_MODEL &&
        model.id !== "local-rules" &&
        model.id !== "openrouter",
    );
    const notesModel =
      available?.find((model) => model.id === defaultId)?.id ??
      available?.[0]?.id ??
      snapshotModels.find((model) => model.id === defaultId)?.id ??
      snapshotModels[0]?.id ??
      defaultId;
    await app.saveSettings({ notesModel });
    if (app.snapshot.settings.notesModel === "openrouter")
      throw new Error(app.error || "The local Notes model could not be saved.");
  }
}

export const ai = new AiState();
