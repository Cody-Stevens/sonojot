<script lang="ts">
  import { ensureDownloadConsent } from "../lib/download-consent.svelte";
  import { onMount, tick, untrack, type Snippet } from "svelte";
  import Card from "./Card.svelte";
  import ModelAcceleration from "./ModelAcceleration.svelte";
  import ModelSelection from "./ModelSelection.svelte";
  import ModelBudget from "./ModelBudget.svelte";
  import ModelDownload from "./ModelDownload.svelte";
  import ModelLibrary from "./ModelLibrary.svelte";
  import SettingRow from "./SettingRow.svelte";
  import SetupProfiles from "./SetupProfiles.svelte";
  import PunctuationDetails from "./PunctuationDetails.svelte";
  import {
    PUNCTUATION_MODEL,
    PUNCTUATION_UPGRADE_REASON,
    modelDisplayName,
    profileIncludesPunctuation,
    punctuationRecommendation,
  } from "../lib/punctuation";
  import RemoteModelBadge from "./RemoteModelBadge.svelte";
  import { ai } from "../lib/ai.svelte";
  import {
    priceLine,
    routeNotes,
    type AiModel,
    type AiModelCheck,
  } from "../lib/ai";
  import { errorMessage } from "../lib/protocol";
  import {
    saveMeetingModelChoice,
    showMeetingModelPicker,
    type MeetingModelChoice,
  } from "../lib/meeting-models";
  import {
    SMART_CLEANUP_MODEL,
    SMART_CLEANUP_HELP,
    smartCleanupAvailable,
  } from "../lib/smart-cleanup";
  import { profiles } from "../lib/setup-profiles.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import { acceleration } from "../lib/acceleration.svelte";
  import { textGpuName } from "../lib/acceleration";
  import { library } from "../lib/model-library.svelte";
  import {
    checking,
    formatBytes,
    installed,
    installing,
    modelRoles,
    modelUnavailableReason,
    removalDetail,
    removalPlan,
    selectable,
    type ModelEntry,
    type ModelRole,
    type RemovalPlan,
  } from "../lib/model-library";

  let {
    focusRole = null,
    storage,
  }: { focusRole?: ModelRole | null; storage?: Snippet } = $props();
  let pending = $state("");
  let catalog = $state<AiModel[]>([]);
  let routeChecks = $state<Record<string, AiModelCheck | null>>({});
  let routeErrors = $state<Record<string, string>>({});
  let routePending = $state<Record<string, boolean>>({});
  const routeIds: Record<string, string> = {};
  const routeRequests: Record<string, number> = {};
  const models = $derived(library.data?.models ?? []);
  const recommendedPunctuation = $derived(
    punctuationRecommendation(app.snapshot.settings, models),
  );
  const remoteNotes = $derived(
    ai.data?.backend === "openrouter" ? ai.data.model : "",
  );
  const onlineConnected = $derived(ai.data?.connected ?? false);
  const speechChoices = $derived(onlineModels("speech"));
  const meetingModel = $derived(
    models.find((model) => model.id === app.snapshot.settings.meetingAsrModel),
  );
  const remoteMeeting = $derived(
    app.snapshot.settings.meetingAsrModel ? "" : (ai.data?.speechModel ?? ""),
  );
  const showMeetingPicker = $derived(
    showMeetingModelPicker(
      models,
      onlineConnected,
      speechChoices.length,
      app.snapshot.settings.meetingAsrModel,
    ),
  );
  function onlineModels(role: "text" | "speech"): AiModel[] {
    return (ai.data?.quickModels ?? [])
      .filter((model) => model.role === role)
      .map(
        ({ id }) =>
          catalog.find((model) => model.id === id) ?? {
            id,
            name: id,
            contextLength: 0,
            promptPrice: null,
            completionPrice: null,
            description: null,
            inputModalities: role === "speech" ? ["audio"] : ["text"],
          },
      );
  }
  function roleModels(role: ModelRole): ModelEntry[] {
    return models.filter(
      (model) =>
        model.id !== SMART_CLEANUP_MODEL ||
        (role === "cleanup" &&
          smartCleanupAvailable({
            ...app.snapshot.settings,
            setupProfile:
              profiles.custom && app.snapshot.settings.setupProfile !== null
                ? "custom"
                : app.snapshot.settings.setupProfile,
          })),
    );
  }
  const active = $derived(
    ["loading", "listening", "finishing", "cleaning"].includes(
      app.snapshot.state,
    ) ||
      app.snapshot.meetingRecording ||
      models.some((model) =>
        ["loading", "busy", "unloading"].includes(model.state),
      ),
  );
  const installRunning = $derived(models.some(installing));
  const settingsLocked = $derived(
    !app.native ||
      active ||
      installRunning ||
      profiles.busy ||
      ai.busy ||
      app.busy("settings") ||
      !!pending,
  );
  const locked = $derived(settingsLocked || !library.data);
  const installDisabled = $derived(locked || installRunning || !library.data);
  const selectedIds = $derived([
    ...modelRoles.map((role) => app.snapshot.settings[role.setting]),
    app.snapshot.settings.meetingAsrModel,
  ]);
  const selected = $derived(
    modelRoles
      .filter((role) => role.id !== "notes" || !remoteNotes)
      .map((role) =>
        roleModels(role.id).find(
          (model) =>
            model.id === app.snapshot.settings[role.setting] &&
            model.roles.includes(role.id),
        ),
      ),
  );
  const sequential = $derived(
    app.snapshot.settings.memoryMode === "sequential",
  );
  const showTextProcessing = $derived(
    selected.some(
      (model) => model?.kind === "cleanup" && model.id !== "local-rules",
    ),
  );
  const gpuOption = $derived(
    `${textGpuName(app.snapshot.capabilities.platform, acceleration.data?.textGpu.backend)} when available`,
  );
  const download = $derived(
    models.find(installing) ??
      models.find((model) => model.id === library.downloadId),
  );

  onMount(() => {
    const release = library.acquire();
    void ai.refresh();
    return release;
  });
  $effect(() => {
    if (onlineConnected)
      void call("list_ai_catalog")
        .then((value) => {
          catalog = value;
        })
        .catch(() => {});
  });
  $effect(() => {
    const role = focusRole;
    if (role && app.ready) {
      untrack(() => {
        void profiles.choose("custom");
      });
      void tick().then(() => {
        const row = document.getElementById(`setup-${role}`);
        row?.focus({ preventScroll: true });
        row?.scrollIntoView({ block: "start" });
      });
    }
  });
  $effect(() => {
    const ids = { notes: remoteNotes, meeting: ai.data?.speechModel ?? "" };
    untrack(() => {
      for (const role of ["notes", "meeting"] as const) {
        if (routeIds[role] === undefined || routeIds[role] === ids[role])
          continue;
        routeRequests[role] = (routeRequests[role] ?? 0) + 1;
        routeIds[role] = ids[role];
        routeChecks[role] = null;
        routeErrors[role] = "";
        routePending[role] = false;
      }
    });
  });

  function addOnline(): void {
    const target = document.getElementById(
      ai.data?.connected ? "ai-model-search" : "ai-connect",
    );
    document
      .getElementById("openrouter-models")
      ?.scrollIntoView({ block: "start" });
    target?.focus({ preventScroll: true });
  }

  async function checkRoute(
    role: "notes" | "meeting",
    id: string,
  ): Promise<void> {
    const request = (routeRequests[role] ?? 0) + 1;
    routeRequests[role] = request;
    routeIds[role] = id;
    routeChecks[role] = null;
    routeErrors[role] = "";
    routePending[role] = !!id;
    if (!id) return;
    try {
      const result = await call("check_ai_model", { model: id });
      if (routeRequests[role] === request) routeChecks[role] = result;
    } catch (failure: unknown) {
      if (routeRequests[role] === request)
        routeErrors[role] = errorMessage(failure);
    } finally {
      if (routeRequests[role] === request) routePending[role] = false;
    }
  }

  async function selectOnline(
    role: "notes" | "meeting",
    id: string,
  ): Promise<void> {
    if (role === "meeting") {
      await selectMeeting({ kind: "online", id });
      return;
    }
    if (settingsLocked || !ai.data || (id && !ai.data.connected)) return;
    if (ai.data.backend === "openrouter" && ai.data.model === id) {
      void checkRoute(role, id);
      return;
    }
    pending = `select-${role}`;
    routeErrors[role] = "";
    const previous = {
      backend: ai.data.backend,
      model: ai.data.model,
      speechModel: ai.data.speechModel,
    };
    try {
      const saved = await ai.save({ backend: "openrouter", model: id });
      if (!saved) {
        routeErrors[role] = app.error || "The model choice could not be saved.";
        return;
      }
      if (
        app.snapshot.settings.setupProfile !== null &&
        app.snapshot.settings.setupProfile !== "custom"
      ) {
        await app.saveSettings({ setupProfile: "custom" });
        if (app.error) {
          const failure = app.error;
          const restored = await ai.save(previous);
          routeErrors[role] = restored
            ? failure
            : `${failure} The online route changed, but the profile could not be saved.`;
          return;
        }
      }
      void checkRoute(role, id);
    } finally {
      pending = "";
    }
  }

  async function selectMeeting(choice: MeetingModelChoice): Promise<void> {
    if (settingsLocked) return;
    if (
      choice.kind === "local" &&
      choice.id &&
      !models.some(
        (model) => model.id === choice.id && selectable(model, "meetingSpeech"),
      )
    )
      return;
    if (choice.kind === "online" && !onlineConnected) return;
    pending = "select-meeting";
    routeErrors.meeting = "";
    try {
      if (!ai.data) await ai.refresh();
      if (!ai.data)
        throw new Error(
          "OpenRouter settings could not be checked. Use Try again in Online models below.",
        );
      await saveMeetingModelChoice(choice, ai.data.speechModel, {
        saveOnline: async (speechModel) => {
          if (!(await ai.save({ speechModel })))
            throw new Error(
              app.error || "The meeting model choice could not be saved.",
            );
        },
        saveLocal: async (meetingAsrModel) => {
          if (app.snapshot.settings.meetingAsrModel === meetingAsrModel) return;
          await app.saveSettings({ meetingAsrModel });
          if (
            app.error ||
            app.snapshot.settings.meetingAsrModel !== meetingAsrModel
          )
            throw new Error(
              app.error || "The meeting model choice could not be saved.",
            );
        },
      });
      void checkRoute("meeting", choice.kind === "online" ? choice.id : "");
      await library.refresh();
    } catch (failure: unknown) {
      routeErrors.meeting = errorMessage(failure);
    } finally {
      pending = "";
    }
  }

  async function select(
    role: (typeof modelRoles)[number],
    id: string,
  ): Promise<void> {
    const model = roleModels(role.id).find((model) => model.id === id);
    if (locked || !model || !selectable(model, role.id)) return;
    if (role.id === "notes" && !ai.data) {
      routeErrors.notes =
        "OpenRouter settings could not be checked. Use Try again in Online models below.";
      return;
    }
    if (
      app.snapshot.settings[role.setting] === id &&
      (role.id !== "notes" || ai.data?.backend === "local")
    )
      return;
    pending = `select-${role.id}`;
    try {
      await app.saveSettings({
        [role.setting]: id,
        ...(app.snapshot.settings.setupProfile !== null
          ? { setupProfile: "custom" as const }
          : {}),
      });
      if (app.error || app.snapshot.settings[role.setting] !== id) return;
      if (role.id === "notes" && ai.data?.backend === "openrouter") {
        if (!(await ai.save({ backend: "local" }))) {
          routeErrors.notes =
            app.error || "The local Notes route could not be saved.";
          return;
        }
        void checkRoute("notes", "");
      }
      await library.refresh();
    } finally {
      pending = "";
    }
  }
  async function install(model: ModelEntry): Promise<void> {
    if (
      !profiles.custom &&
      profiles.retrySpeechId &&
      (model.id === profiles.retrySpeechId ||
        (model.id === PUNCTUATION_MODEL &&
          profileIncludesPunctuation(
            app.snapshot.settings.setupProfile,
            profiles.selected,
          )))
    ) {
      await profiles.use();
      return;
    }
    const current = models.find((item) => item.id === model.id);
    if (
      installDisabled ||
      !current?.installable ||
      checking(current) ||
      installed(current) ||
      installing(current)
    )
      return;
    pending = model.id;
    try {
      await app.perform("model-install", async () => {
        if (!(await ensureDownloadConsent([model.id]))) return;
        if (model.id === PUNCTUATION_MODEL && !(await acknowledgePunctuation()))
          return;
        library.downloadId = model.id;
        try {
          library.apply(await call("start_model_install", { id: model.id }));
        } catch (failure: unknown) {
          // A rejected start still belongs in the existing download strip.
          if (library.data)
            library.apply({
              ...library.data,
              models: library.data.models.map((entry) =>
                entry.id === model.id
                  ? {
                      ...entry,
                      state: "failed",
                      error:
                        failure instanceof Error
                          ? failure.message
                          : String(failure),
                    }
                  : entry,
              ),
            });
          throw failure;
        }
      });
    } finally {
      pending = "";
    }
  }
  async function acknowledgePunctuation(): Promise<boolean> {
    if (app.snapshot.settings.punctuationV2RecommendationAcknowledged)
      return true;
    await app.saveSettings({ punctuationV2RecommendationAcknowledged: true });
    return app.snapshot.settings.punctuationV2RecommendationAcknowledged;
  }
  async function cancel(model: ModelEntry): Promise<void> {
    if (!app.native || pending || !installing(model)) return;
    pending = model.id;
    library.downloadId = model.id;
    try {
      await app.perform("model-cancel", async () =>
        library.apply(await call("cancel_model_install", { id: model.id })),
      );
    } finally {
      pending = "";
    }
  }
  /** Replacements come from the saved profile's recommendation, else Balanced. */
  function removal(model: ModelEntry): RemovalPlan {
    const saved = app.snapshot.settings.setupProfile;
    return removalPlan(model, {
      selection: app.snapshot.settings,
      models,
      remoteNotes: !!remoteNotes,
      recommended:
        profiles.hardware?.recommendations[
          saved && saved !== "custom" ? saved : "balanced"
        ],
      nameOf: (id) => {
        const entry = models.find((item) => item.id === id);
        return entry ? modelDisplayName(entry, models) : id;
      },
    });
  }
  const samePlan = (a: RemovalPlan, b: RemovalPlan) =>
    a.blocked === b.blocked &&
    JSON.stringify(a.switches) === JSON.stringify(b.switches);
  async function remove(model: ModelEntry): Promise<void> {
    const plan = removal(model);
    if (
      locked ||
      installRunning ||
      !model.removable ||
      !installed(model) ||
      plan.blocked
    )
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
    // Capture or a selection in another window may have started while
    // confirming. Only the confirmed switches may be saved.
    const current = models.find((item) => item.id === model.id);
    if (
      locked ||
      installRunning ||
      !current?.removable ||
      !installed(current) ||
      !samePlan(plan, removal(current))
    )
      return;
    pending = model.id;
    try {
      await app.perform("model-remove", async () => {
        if (model.id === PUNCTUATION_MODEL && !(await acknowledgePunctuation()))
          return;
        if (plan.switches.length) {
          const patch = Object.fromEntries(
            plan.switches.map((item) => [item.setting, item.to]),
          );
          await app.saveSettings({
            ...patch,
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
      pending = "";
    }
  }
</script>

{#snippet routeStatus(role: "notes" | "meeting", id: string)}
  {#if id}
    <p class="secondary" data-text-surface="flat">
      <RemoteModelBadge />
      {catalog.find((model) => model.id === id)?.name ?? id} · {priceLine(
        catalog.find((model) => model.id === id) ?? {
          id,
          name: id,
          contextLength: 0,
          promptPrice: null,
          completionPrice: null,
          description: null,
          inputModalities: [],
        },
      )}
    </p>
    <p class="secondary" data-text-surface="flat">
      {role === "meeting" ? "Meeting audio" : "Text"} goes to OpenRouter and is billed
      there.
    </p>
  {/if}
  {#if routePending[role]}<p class="secondary" role="status">
      Checking model routes…
    </p>{/if}
  {#if routeErrors[role]}<p class="secondary" role="alert">
      {routeErrors[role]}
    </p>{/if}
  {#if routeChecks[role]?.availability !== "listed" && routeChecks[role]}
    <p class="secondary" role="status">
      OpenRouter could not confirm an active route for this model. It may not
      run.
    </p>
  {/if}
  {#if routeChecks[role]}
    <details class="route-details">
      <summary>Details</summary>
      {#each routeNotes(routeChecks[role], false) as note}<p
          class="secondary"
          data-text-surface="flat"
        >
          {note}
        </p>{/each}
    </details>
  {/if}
{/snippet}

<div class="model-setup">
  <Card title="Speech and text models">
    <SetupProfiles />
    {#if recommendedPunctuation}
      <section
        class="punctuation-recommendation"
        aria-label="Recommended download"
      >
        <p data-text-surface="flat">
          Recommended: {recommendedPunctuation.name} · CPU · {formatBytes(
            recommendedPunctuation.totalBytes ||
              recommendedPunctuation.diskBytes,
          )} to download
        </p>
        <p data-text-surface="flat">{PUNCTUATION_UPGRADE_REASON}</p>
        <PunctuationDetails />
        <div class="actions">
          <button
            class="button"
            disabled={installDisabled}
            onclick={() =>
              recommendedPunctuation && install(recommendedPunctuation)}
            >Install punctuation</button
          >
          <button
            class="button quiet"
            disabled={settingsLocked}
            onclick={() => acknowledgePunctuation()}>Dismiss</button
          >
        </div>
      </section>
    {/if}
    {#if profiles.custom}
      {#each modelRoles as role, index}
        {@const roleCatalog = roleModels(role.id)}
        {@const model = roleCatalog.find(
          (item) =>
            item.id === selectedIds[index] && item.roles.includes(role.id),
        )}
        <div
          id={`setup-${role.id}`}
          class="model-anchor"
          tabindex="-1"
          role="group"
          aria-label={role.title}
        >
          <label for={`model-${role.id}`}>{role.title}</label>
          <div class="choice">
            <ModelSelection
              role={role.id}
              title={role.title}
              selectedId={selectedIds[index]}
              models={roleCatalog}
              disabled={locked || !library.data}
              {installDisabled}
              onselect={(id) => select(role, id)}
              oninstall={install}
              remoteModels={role.id === "notes" ? onlineModels("text") : []}
              selectedRemoteId={role.id === "notes" ? remoteNotes : ""}
              onlineConnected={ai.data?.connected ?? false}
              onremote={(id) => selectOnline("notes", id)}
              onaddonline={addOnline}
            />
            {#if role.id === "cleanup" && model?.id === SMART_CLEANUP_MODEL}
              <span class="secondary" data-text-surface="flat"
                >{SMART_CLEANUP_HELP}</span
              >
            {/if}
            {#if role.id === "notes" && remoteNotes}
              {@render routeStatus("notes", remoteNotes)}
            {:else if model && (model.kind === "asr" || model.kind === "cleanup") && installed(model)}<ModelAcceleration
                modelId={model.id}
                kind={model.kind}
                compact
              />
            {:else if library.data}<span
                class="secondary"
                data-text-surface="flat"
                >{model && checking(model)
                  ? "Checking…"
                  : model
                    ? "Not installed"
                    : "Unavailable"}</span
              >{/if}
            {#if role.id === "notes" && !remoteNotes && routeErrors.notes}
              <p class="secondary" role="alert">{routeErrors.notes}</p>
            {/if}
          </div>
        </div>
      {/each}
      {#if library.data}
        <ModelBudget
          {selected}
          hardware={library.data?.hardware}
          {sequential}
          cpu={app.snapshot.settings.textAcceleration === "cpu"}
        />
      {:else}<p class="field-help" data-text-surface="flat">
          Reading memory and model estimates…
        </p>{/if}
    {/if}
    {#if showMeetingPicker}
      <div
        class="model-anchor"
        id="setup-meeting"
        role="group"
        aria-label="Meeting transcription"
      >
        <label for="model-meeting">Meeting transcription</label>
        <div class="choice">
          <ModelSelection
            role="meetingSpeech"
            title="Meeting transcription"
            pickerId="model-meeting"
            selectedId={app.snapshot.settings.meetingAsrModel}
            selectedRemoteId={remoteMeeting}
            {models}
            localDefaultLabel="Same as Speech (on this computer)"
            remoteModels={speechChoices}
            disabled={locked}
            {installDisabled}
            {onlineConnected}
            allowOnline={true}
            onselect={(id) => selectMeeting({ kind: "local", id })}
            onremote={(id) => selectOnline("meeting", id)}
            oninstall={install}
            onaddonline={addOnline}
          />
          {#if meetingModel && modelUnavailableReason(meetingModel)}
            <p class="secondary" data-text-surface="flat">
              Unavailable · {modelUnavailableReason(meetingModel)}
            </p>
          {/if}
          {@render routeStatus("meeting", remoteMeeting)}
        </div>
      </div>
    {/if}
    <ModelDownload
      model={download}
      pending={!app.native || !!pending}
      retryDisabled={installDisabled}
      oncancel={cancel}
      onretry={install}
    />
    {#if !library.data}<p class="secondary" data-text-surface="flat">
        {app.native
          ? library.error || "Reading model library…"
          : "Model availability is checked in the desktop app."}
      </p>{/if}
    <details class="management-fold">
      <summary data-text-surface="flat">Manage downloads</summary>
      {#if library.error}<p
          role="alert"
          class="secondary"
          data-text-surface="flat"
        >
          {library.error}
        </p>{/if}
      <ModelLibrary
        {models}
        {selectedIds}
        remoteNotes={!!remoteNotes}
        locked={locked || installRunning}
        lockedReason={active
          ? "Remove is available after dictation, meetings and model work finish."
          : installRunning
            ? "Remove is available after the current download finishes."
            : ""}
        {removal}
        {installDisabled}
        oninstall={install}
        onremove={remove}
      />
    </details>

    <details class="management-fold">
      <summary data-text-surface="flat">Performance and storage</summary>
      <SettingRow label="Memory use">
        <select
          aria-label="Memory use"
          id="memory-mode"
          value={app.snapshot.settings.memoryMode}
          disabled={settingsLocked}
          onchange={(event) => {
            const memoryMode = event.currentTarget.value;
            if (
              !settingsLocked &&
              (memoryMode === "overlap" || memoryMode === "sequential")
            )
              void app.saveSettings({ memoryMode });
          }}
        >
          <option value="overlap">Keep speech and cleanup ready</option><option
            value="sequential">Load one stage at a time</option
          >
        </select>
      </SettingRow>
      <SettingRow label="Release unused models">
        <select
          aria-label="Release unused models"
          id="model-retention"
          value={app.snapshot.settings.modelRetention}
          disabled={settingsLocked}
          onchange={(event) => {
            const modelRetention = event.currentTarget.value;
            if (
              !settingsLocked &&
              (modelRetention === "immediately" ||
                modelRetention === "10m" ||
                modelRetention === "30m" ||
                modelRetention === "exit")
            )
              void app.saveSettings({ modelRetention });
          }}
        >
          <option
            value="immediately"
            disabled={app.snapshot.settings.memoryMode === "overlap"}
            >Immediately</option
          ><option value="10m">After 10 minutes</option><option value="30m"
            >After 30 minutes</option
          ><option value="exit">When SonoJot closes</option>
        </select>
      </SettingRow>
      {#if app.snapshot.settings.memoryMode === "overlap"}<p
          class="secondary setting-note"
          data-text-surface="flat"
        >
          Immediate release is unavailable while speech and cleanup are kept
          ready. Choose sequential loading first if you want immediate release.
        </p>{/if}
      {#if showTextProcessing}
        <SettingRow label="Text model processing">
          <select
            aria-label="Text model processing"
            value={app.snapshot.settings.textAcceleration}
            disabled={settingsLocked}
            onchange={async (event) => {
              const textAcceleration = event.currentTarget.value;
              if (
                settingsLocked ||
                (textAcceleration !== "auto" && textAcceleration !== "cpu")
              )
                return;
              await app.saveSettings({ textAcceleration });
              await acceleration.refresh();
            }}
          >
            <option value="auto">{gpuOption}</option><option value="cpu"
              >CPU only</option
            >
          </select>
        </SettingRow>
      {/if}
      {#if active}<p class="secondary" data-text-surface="flat">
          Finish model processing to change settings.
        </p>{/if}
      {@render storage?.()}
    </details>
  </Card>
</div>

<style>
  .model-setup {
    font-size: 1rem;
  }
  .management-fold {
    margin-top: 27px;
    padding-top: 18px;
    border-top: 1px solid var(--line);
  }
  .management-fold > summary {
    margin-bottom: 18px;
  }
  summary {
    cursor: pointer;
  }
  .route-details {
    margin-top: 9px;
    color: var(--soft-ink);
  }
  .punctuation-recommendation {
    margin-top: 27px;
    padding-top: 18px;
    border-top: 1px solid var(--line);
    overflow-wrap: anywhere;
  }
  .punctuation-recommendation > p {
    margin: 0;
  }
  .punctuation-recommendation .actions {
    margin-top: 18px;
  }
  .model-anchor {
    display: grid;
    grid-template-columns: 108px minmax(0, 1fr);
    gap: 18px;
    align-items: baseline;
    padding: 9px 0;
    scroll-margin-top: 18px;
  }
  .model-anchor:focus-visible {
    outline: 2px solid var(--ink);
    outline-offset: 3px;
  }
  label {
    font-size: 1rem;
  }
  .choice {
    min-width: 0;
  }
  .secondary {
    color: var(--soft-ink);
    margin: 18px 0 0;
  }
  .setting-note {
    margin-top: -9px;
  }
  .model-setup :global(.paper-card) {
    margin-top: 27px;
    padding: 27px;
  }
  .model-setup :global(.paper-card > h2) {
    margin-bottom: 18px;
  }
  .model-setup :global(.row-copy),
  .model-setup :global(.row-control select),
  .model-setup :global(.device-badge) {
    font-size: 1rem;
  }
  .model-setup :global(.device-badge) {
    gap: 9px;
  }
  .model-setup :global(.row-control) {
    max-width: 65%;
  }
  .model-setup :global(.row-control select) {
    max-width: 360px;
  }
  @media (max-width: 650px) {
    .model-anchor {
      grid-template-columns: minmax(0, 1fr);
      gap: 9px;
      padding: 9px 0 18px;
    }
    .model-setup :global(.paper-card) {
      padding: 18px;
    }
    .model-setup :global(.paper-row) {
      flex-wrap: wrap;
    }
    .model-setup :global(.row-control) {
      width: 100%;
      max-width: none;
    }
    .model-setup :global(.row-control select) {
      max-width: none;
    }
  }
</style>
