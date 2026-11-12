import { app } from "./app.svelte";
import { ai } from "./ai.svelte";
import { library } from "./model-library.svelte";
import { profiles } from "./setup-profiles.svelte";
import { call } from "./api";
import {
  installed,
  installing,
  removalPlan,
  removalDetail,
  type ModelEntry,
} from "./model-library";
import { PUNCTUATION_MODEL } from "./punctuation";
import { modelName } from "./model-names";

class ModelStorage {
  pending = $state("");
  get locked(): boolean {
    return (
      !app.native ||
      !app.ready ||
      !library.data ||
      !!this.pending ||
      profiles.busy ||
      ai.busy ||
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
  removal(model: ModelEntry) {
    const saved = app.snapshot.settings.setupProfile;
    const models = library.data?.models ?? [];
    return removalPlan(model, {
      selection: app.snapshot.settings,
      models,
      remoteNotes: ai.data?.backend === "openrouter",
      recommended:
        profiles.hardware?.recommendations[
          saved && saved !== "custom" ? saved : "balanced"
        ],
      nameOf: (id) => modelName(id, models),
    });
  }
  async remove(model: ModelEntry) {
    const plan = this.removal(model);
    if (this.locked || !model.removable || !installed(model) || plan.blocked)
      return;
    if (
      !(await app.confirm(
        `Remove ${model.name}?`,
        removalDetail(plan),
        "Remove",
        true,
      ))
    )
      return;
    const current = library.data?.models.find((item) => item.id === model.id);
    if (
      this.locked ||
      !current?.removable ||
      !installed(current) ||
      JSON.stringify(plan) !== JSON.stringify(this.removal(current))
    )
      return;
    this.pending = model.id;
    try {
      await app.perform("model-remove", async () => {
        if (
          model.id === PUNCTUATION_MODEL &&
          !app.snapshot.settings.punctuationV3RecommendationAcknowledged
        ) {
          await app.saveSettings({
            punctuationV3RecommendationAcknowledged: true,
          });
          if (!app.snapshot.settings.punctuationV3RecommendationAcknowledged)
            return;
        }
        if (plan.switches.length) {
          await app.saveSettings({
            ...Object.fromEntries(
              plan.switches.map((item) => [item.setting, item.to]),
            ),
            ...(app.snapshot.settings.setupProfile !== null
              ? { setupProfile: "custom" as const }
              : {}),
          });
          if (
            app.error ||
            plan.switches.some(
              (item) => app.snapshot.settings[item.setting] !== item.to,
            )
          )
            return;
        }
        library.apply(
          await call("remove_model", { id: model.id, confirmed: true }),
        );
        app.apply(await call("get_snapshot"));
      });
    } finally {
      this.pending = "";
    }
  }
}
export const modelStorage = new ModelStorage();
