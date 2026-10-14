import { call } from "./api";
import { ensureDownloadConsent } from "./download-consent.svelte";
import { app } from "./app.svelte";
import { ai } from "./ai.svelte";
import { library } from "./model-library.svelte";
import {
  checking,
  installed,
  installing,
  type ModelLibrary,
} from "./model-library";
import { errorMessage } from "./protocol";
import {
  applyProfileSelection,
  profileLabel,
  selectedProfile,
  showCustomModels,
  type HardwareProfile,
  type SetupProfile,
} from "./setup-profiles";

class SetupProfilesState {
  hardware = $state<HardwareProfile | null>(null);
  checking = $state(false);
  hardwareError = $state("");
  draft = $state<SetupProfile | null>(null);
  retrySpeechId = $state("");
  needsCompletion = $state(false);
  /**
   * Why the last Use/Switch for a profile stopped, in the installer's own
   * words (for example "Not enough disk space…"). The profile card shows it.
   */
  failure = $state<{ profile: SetupProfile; message: string } | null>(null);
  private deferredNotes = new Set<string>();

  get selected(): SetupProfile {
    return selectedProfile(app.snapshot.settings.setupProfile, this.draft);
  }
  get custom(): boolean {
    return showCustomModels(app.snapshot.settings.setupProfile, this.draft);
  }
  get busy(): boolean {
    return app.busy("setup-profile");
  }
  get locked(): boolean {
    return !app.native || !app.ready;
  }
  get applyLocked(): boolean {
    return (
      this.locked ||
      this.busy ||
      ai.busy ||
      !library.data ||
      app.busy("settings") ||
      !["idle", "error"].includes(app.snapshot.state) ||
      app.snapshot.meetingRecording ||
      library.data.models.some(
        (model) =>
          installing(model) ||
          ["loading", "busy", "unloading"].includes(model.state),
      )
    );
  }

  async check(): Promise<void> {
    if (!app.native || this.checking || this.busy) return;
    this.checking = true;
    this.hardwareError = "";
    try {
      this.hardware = await call("get_hardware_profile");
    } catch (failure: unknown) {
      this.hardwareError = errorMessage(failure);
    } finally {
      this.checking = false;
      void library.refresh();
    }
  }

  async choose(profile: SetupProfile): Promise<void> {
    if (this.locked) return;
    this.retrySpeechId = "";
    this.failure = null;
    // Every tab previews. Only a model edit or Finish setup commits Custom.
    this.draft = profile;
  }

  private async readLibrary(): Promise<ModelLibrary> {
    const data = await call("get_model_library");
    library.apply(data);
    return data;
  }

  private async install(id: string): Promise<ModelLibrary> {
    if (!(await ensureDownloadConsent([id])))
      throw new Error("Download cancelled.");
    library.downloadId = id;
    try {
      const data = await call("start_model_install", { id });
      library.apply(data);
      const model = data.models.find((entry) => entry.id === id);
      if (model?.error && !installing(model) && !installed(model))
        throw new Error(model.error);
      return data;
    } catch (failure: unknown) {
      // Even a rejected start needs the existing strip's Retry action.
      if (library.data)
        library.apply({
          ...library.data,
          models: library.data.models.map((model) =>
            model.id === id
              ? { ...model, state: "failed", error: errorMessage(failure) }
              : model,
          ),
        });
      throw failure;
    }
  }

  private async complete(): Promise<void> {
    await call("complete_setup");
    this.needsCompletion = false;
  }

  private async prepareNotesAfterCheck(id: string): Promise<void> {
    if (this.deferredNotes.has(id)) return;
    this.deferredNotes.add(id);
    const release = library.acquire();
    try {
      let catalog = await this.readLibrary();
      let notes = catalog.models.find((model) => model.id === id);
      while (notes && checking(notes)) {
        if (
          app.snapshot.settings.notesModel !== id ||
          app.snapshot.settings.setupProfile === "custom"
        )
          return;
        await new Promise((resolve) => setTimeout(resolve, 1000));
        catalog = await this.readLibrary();
        notes = catalog.models.find((model) => model.id === id);
      }
      // A later profile or Custom selection may no longer want this download.
      if (
        app.snapshot.settings.notesModel !== id ||
        app.snapshot.settings.setupProfile === "custom"
      )
        return;
      if (!notes)
        throw new Error(
          "The selected notes model is not in the model catalog. Check again in Setup.",
        );
      if (!installed(notes) && !installing(notes)) {
        if (!notes.installable)
          throw new Error(
            "The selected notes model cannot be installed on this computer.",
          );
        await this.install(id);
      }
    } catch (failure: unknown) {
      app.error = `SETUP_NOTES_DOWNLOAD: ${errorMessage(failure)}`;
    } finally {
      this.deferredNotes.delete(id);
      release();
    }
  }

  private async apply(profile: SetupProfile) {
    // Let unrelated preference edits finish, then serialize only the atomic save.
    while (app.busy("settings")) {
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    app.pending = [...app.pending, "settings"];
    try {
      if (profile !== "custom" && !ai.data) {
        await ai.refresh();
        if (!ai.data)
          throw new Error(
            "The Notes route could not be checked. Try again in Online models.",
          );
      }
      const next = await call("apply_setup_profile", { profile });
      app.apply(next);
      if (profile !== "custom") {
        // Automatic profiles name local Notes models, so their route must agree.
        if (
          ai.data?.backend !== "local" &&
          !(await ai.save({ backend: "local" }))
        ) {
          throw new Error(
            "Models were saved, but Notes still uses OpenRouter. Choose a local Notes model in Custom.",
          );
        }
      }
      return next;
    } finally {
      app.pending = app.pending.filter((key) => key !== "settings");
    }
  }

  async use(profile = this.selected): Promise<void> {
    if (this.applyLocked || (profile !== "custom" && !this.hardware)) return;
    this.failure = null;
    // Own the whole operation, including confirmation, across route changes.
    const deferredNotes = await app.perform("setup-profile", () =>
      this.useProfile(profile).catch((failure: unknown) => {
        const message = errorMessage(failure);
        // A Notes failure after the profile saved keeps its own banner and
        // Open Models action; everything else belongs to the profile card.
        if (!message.startsWith("SETUP_NOTES_DOWNLOAD:"))
          this.failure = { profile, message };
        throw failure;
      }),
    );
    // Show the reason once, on the card, instead of a generic banner whose
    // Details hide words such as "Not enough disk space".
    // (Read through the declared type; the catch above assigns it.)
    const failure = this.failure as SetupProfilesState["failure"];
    if (failure && app.error === failure.message) app.error = "";
    if (deferredNotes) void this.prepareNotesAfterCheck(deferredNotes);
  }

  private async useProfile(profile: SetupProfile): Promise<string | undefined> {
    if (
      app.snapshot.settings.setupProfile === "custom" &&
      profile !== "custom" &&
      !(await app.confirm(
        `Use ${profileLabel(profile)}?`,
        "Replace your Speech, Cleanup and Notes model choices with this profile.",
        "Use this",
      ))
    )
      return;
    if (
      app.busy("settings") ||
      !["idle", "error"].includes(app.snapshot.state) ||
      app.snapshot.meetingRecording
    )
      return;
    const release = library.acquire();
    try {
      if (profile === "custom" && app.snapshot.settings.setupProfile !== null) {
        await this.apply(profile);
        if (this.draft === profile) this.draft = null;
        if (this.needsCompletion) await this.complete();
        return;
      }
      const selection =
        profile === "custom"
          ? app.snapshot.settings
          : this.hardware?.recommendations[profile];
      if (!selection) return;
      if (
        !(await ensureDownloadConsent([
          selection.asrModel,
          selection.cleanupModel,
          selection.notesModel,
        ]))
      )
        return;
      // Waiting is scoped to the requested model, never the whole catalog.
      if (
        library.data?.models.some(
          (model) => model.id === selection.asrModel && checking(model),
        )
      )
        return;
      this.retrySpeechId = selection.asrModel;
      const notesToCheck = await applyProfileSelection(
        profile,
        selection,
        {
          readLibrary: () => this.readLibrary(),
          install: (id) => this.install(id),
          wait: () => new Promise((resolve) => setTimeout(resolve, 1000)),
          apply: async (chosen) => {
            const next = await this.apply(chosen);
            if (this.draft === profile) this.draft = null;
            this.retrySpeechId = "";
            this.needsCompletion = true;
            return next;
          },
          complete: () => this.complete(),
        },
        app.snapshot.settings.setupProfile,
      );
      app.notice = "Setup saved.";
      return notesToCheck;
    } finally {
      release();
    }
  }
}

export const profiles = new SetupProfilesState();
