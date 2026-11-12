<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import Sheet from "./Sheet.svelte";
  import Well from "./Well.svelte";
  import RemoteModelBadge from "../RemoteModelBadge.svelte";
  import { openRouter } from "../../lib/openrouter.svelte";
  import { app } from "../../lib/app.svelte";
  import { ai } from "../../lib/ai.svelte";
  import { call } from "../../lib/api";
  import { library } from "../../lib/model-library.svelte";
  import {
    modelRoles,
    priceLine,
    textCost,
    routeNotes,
    quickModelGroupRemoval,
    type AiModel,
    type AiModelCheck,
  } from "../../lib/ai";
  import { cleanupOnlineIds, cleanupPatch } from "../../lib/cleanup-choice";
  import { saveMeetingModelChoice } from "../../lib/meeting-models";
  import {
    CLOUD_NOTICE_KEY,
    CLOUD_NOTICE_VERSION,
    needsCloudNotice,
    needsTrainingAcceptance,
    trainingAcceptance,
    trainingNotice,
  } from "../../lib/proofreader";
  import { errorMessage } from "../../lib/protocol";

  const request = openRouter.request!;
  const role = request.role;
  const title = {
    cleanup: "For cleanup",
    notes: "For meeting notes",
    meeting: "For meeting speech",
    manage: "Your online models",
  }[role];
  const wantedRole = role === "meeting" ? "speech" : "text";
  let key = $state("");
  let query = $state("");
  let catalog = $state<AiModel[]>([]);
  let recommended = $state<AiModel[]>([]);
  let loading = $state(true);
  let failed = $state(false);
  let error = $state("");
  let pending = $state(false);
  let expanded = $state(false);
  let shown = $state(12);
  let selected = $state(
    request.model ??
      (role === "cleanup"
        ? app.snapshot.settings.proofreaderModel
        : role === "meeting"
          ? ai.data?.speechModel
          : ai.data?.model) ??
      "",
  );
  let notice = $state("");
  let checks = $state<
    Record<string, { check: AiModelCheck | null; failed: boolean }>
  >({});
  let disposed = false;
  const blocked = $derived(
    !app.native ||
      app.busy("settings") ||
      app.busy("setup-profile") ||
      app.snapshot.meetingRecording ||
      ["loading", "listening", "finishing", "cleaning"].includes(
        app.snapshot.state,
      ),
  );
  const locked = $derived(blocked || pending || ai.busy);
  const models = $derived(
    [
      ...new Map(
        [...catalog, ...recommended].map((model) => [model.id, model]),
      ).values(),
    ].filter(
      (model) => role === "manage" || modelRoles(model).includes(wantedRole),
    ),
  );
  const savedIds = $derived(
    (ai.data?.quickModels ?? [])
      .filter((entry) => role === "manage" || entry.role === wantedRole)
      .map((entry) => entry.id),
  );
  const yourIds = $derived([
    ...new Set([
      ...(selected ? [selected] : []),
      ...savedIds,
      ...(role === "cleanup"
        ? cleanupOnlineIds([], "")
        : recommended.map((model) => model.id)),
    ]),
  ]);
  const yours = $derived(
    yourIds
      .map((id) => models.find((model) => model.id === id))
      .filter((model): model is AiModel => !!model),
  );
  const visible = $derived(expanded ? yours : yours.slice(0, 3));
  const matches = $derived(
    models
      .filter((model) => !visible.some((own) => own.id === model.id))
      .filter((model) =>
        `${model.name} ${model.id}`
          .toLowerCase()
          .includes(query.trim().toLowerCase()),
      )
      .slice(0, shown),
  );
  const choice = $derived(models.find((model) => model.id === selected));

  async function loadModels() {
    loading = true;
    failed = false;
    const results = await Promise.allSettled([
      call("list_ai_catalog"),
      call("list_ai_models"),
    ]);
    if (disposed) return;
    if (results[0].status === "fulfilled") catalog = results[0].value;
    if (results[1].status === "fulfilled") recommended = results[1].value;
    failed = results.some((result) => result.status === "rejected");
    loading = false;
  }
  onMount(() => {
    void ai.refresh();
    void loadModels();
    return library.acquire();
  });
  onDestroy(() => {
    disposed = true;
    key = "";
  });

  async function discardKey(): Promise<boolean> {
    if (
      key &&
      !(await app.confirm("Discard this key?", "The key has not been saved."))
    )
      return false;
    key = "";
    return true;
  }
  async function close() {
    if (pending || !(await discardKey())) return;
    if (app.busy("ai-connect"))
      await app.perform("ai-cancel", () => call("cancel_ai_connect"));
    key = "";
    openRouter.close();
  }
  async function connect(withKey = false) {
    if (locked || !ai.data?.supported) return;
    error = "";
    await ai.connect(withKey ? key.trim() : undefined);
    if (disposed) return;
    if (!ai.data?.connected) {
      error = app.error || "Could not connect. Try signing in again.";
      return;
    }
    key = "";
    if (choice && role !== "manage") await choose(choice);
  }
  async function disconnect() {
    if (locked) return;
    await ai.disconnect();
    if (!disposed) error = app.error;
  }
  async function choose(model: AiModel) {
    if (locked) return;
    selected = model.id;
    error = "";
    notice = "";
    if (!ai.data?.connected) return;
    pending = true;
    try {
      if (role !== "manage" && needsCloudNotice(app.snapshot.settings)) {
        const accepted = await app.chooseConfirmation({
          title: "Use OpenRouter?",
          detail: ai.data.help,
          label: "Accept online notice",
          cancelLabel: "Cancel",
        });
        if (accepted !== true || disposed || blocked) return;
        await app.saveSettings({
          componentAcknowledgments: {
            ...app.snapshot.settings.componentAcknowledgments,
            [CLOUD_NOTICE_KEY]: CLOUD_NOTICE_VERSION,
          },
        });
        if (needsCloudNotice(app.snapshot.settings))
          throw new Error(app.error || "The online notice could not be saved.");
      }
      if (
        role === "cleanup" &&
        needsTrainingAcceptance(model, app.snapshot.settings)
      ) {
        const accepted = await app.chooseConfirmation({
          title: "Allow this model to train on your text?",
          detail: trainingNotice(model),
          label: "Allow for this model",
          cancelLabel: "Cancel",
        });
        if (accepted !== true || disposed || blocked) return;
      }
      if (disposed || blocked || !ai.data?.connected) return;
      const roles: ("text" | "speech")[] =
        role === "manage" ? modelRoles(model) : [wantedRole];
      const additions = roles
        .filter(
          (next) =>
            !ai.data?.quickModels.some(
              (entry) => entry.id === model.id && entry.role === next,
            ),
        )
        .map((next) => ({ id: model.id, role: next }));
      if (
        additions.length &&
        !(await ai.save({
          quickModels: [...ai.data.quickModels, ...additions],
        }))
      )
        throw new Error(app.error || "The model could not be added.");
      if (disposed || blocked || !ai.data?.connected) return;
      if (role === "cleanup") {
        await app.saveSettings({
          ...cleanupPatch({ kind: "online", model: model.id }),
          ...(model.trainingRequired
            ? {
                proofreaderTrainingModels: trainingAcceptance(
                  model,
                  app.snapshot.settings,
                ),
              }
            : {}),
        });
        if (
          app.error ||
          !app.snapshot.settings.proofreader ||
          app.snapshot.settings.proofreaderModel !== model.id
        )
          throw new Error(app.error || "Cleanup could not be saved.");
      } else if (role === "meeting") {
        await saveMeetingModelChoice(
          { kind: "online", id: model.id },
          ai.data.speechModel,
          {
            saveOnline: async (speechModel) => {
              if (!(await ai.save({ speechModel })))
                throw new Error(
                  app.error || "The online model could not be saved.",
                );
            },
            saveLocal: async (meetingAsrModel) => {
              await app.saveSettings({ meetingAsrModel });
              if (
                app.error ||
                app.snapshot.settings.meetingAsrModel !== meetingAsrModel
              )
                throw new Error(
                  app.error || "Meeting speech could not be saved.",
                );
            },
          },
        );
      } else if (role === "notes") {
        const previous = { backend: ai.data.backend, model: ai.data.model };
        if (!(await ai.save({ backend: "openrouter", model: model.id })))
          throw new Error(app.error || "The online model could not be saved.");
        await app.saveSettings({
          notesModel: "openrouter",
          ...(app.snapshot.settings.setupProfile !== null
            ? { setupProfile: "custom" as const }
            : {}),
        });
        if (app.error || app.snapshot.settings.notesModel !== "openrouter") {
          const failure = app.error || "Notes could not be saved.";
          const restored = await ai.save(previous);
          throw new Error(
            restored
              ? failure
              : `${failure} Check the Notes model before continuing.`,
          );
        }
      }
      if (role === "manage") notice = `${model.name} is in your models.`;
      else {
        app.notice = `${model.name} selected.`;
        openRouter.close();
      }
    } catch (failure) {
      if (!disposed) error = errorMessage(failure);
    } finally {
      pending = false;
    }
  }
  async function checkModel(id: string) {
    if (checks[id]) return;
    try {
      const check = await call("check_ai_model", { model: id });
      if (!disposed) checks[id] = { check, failed: false };
    } catch {
      if (!disposed) checks[id] = { check: null, failed: true };
    }
  }
  async function remove(id: string) {
    if (locked || !ai.data) return;
    if (await ai.save(quickModelGroupRemoval(ai.data, id)))
      notice =
        "Removed from your saved models. Recommended models remain available.";
    else error = app.error;
  }
</script>

{#snippet modelRow(model: AiModel)}
  <div class="online-model">
    <button
      class="quiet-option"
      class:chosen={selected === model.id}
      disabled={locked}
      aria-pressed={selected === model.id}
      onclick={() => void choose(model)}
    >
      <span class="choice-dot" aria-hidden="true"></span>
      <span class="option-copy"
        >{model.name}{#if model.trainingRequired}<small class="quiet-tag amber"
            >MAY TRAIN ON TEXT</small
          >{/if}</span
      >
      <small class="cost" data-text-surface="flat"
        >{role === "meeting" || !modelRoles(model).includes("text")
          ? "Audio cost not listed"
          : textCost(model)}</small
      >
    </button>
    {#if selected === model.id}<details
        class="model-details"
        ontoggle={(event) => {
          if (event.currentTarget.open) void checkModel(model.id);
        }}
      >
        <summary>Model details</summary>
        <p class="field-help">{model.id} · {priceLine(model)}</p>
        {#if role !== "meeting" && modelRoles(model).includes("text")}<p
            class="field-help"
          >
            Estimate: 1,333 input and output tokens plus 500 instruction tokens
            per 1,000 words. Reasoning, vocabulary and actual output can change
            the cost.
          </p>{:else}<p class="field-help">
            The catalog gives text-token prices, not an audio-hour rate. Audio
            is billed to your OpenRouter account.
          </p>{/if}
        {#each routeNotes(checks[model.id]?.check ?? null, checks[model.id]?.failed ?? false) as note}<p
            class="field-help"
          >
            {note}
          </p>{/each}
        {#if checks[model.id]?.failed}<button
            class="button small"
            onclick={() => {
              delete checks[model.id];
              void checkModel(model.id);
            }}>Try again</button
          >{/if}
        {#if role !== "cleanup" && model.trainingRequired}<p class="field-help">
            Notes and meeting speech request no data collection. This model may
            reject that request.
          </p>{/if}
        {#if savedIds.includes(model.id)}<button
            class="button small"
            disabled={locked}
            onclick={() => void remove(model.id)}
            >Remove from your models</button
          >{/if}
      </details>{/if}
  </div>
{/snippet}

<Sheet title="OpenRouter" onclose={() => void close()}>
  <p class="sheet-context" data-text-surface="flat">
    {title}
    <RemoteModelBadge />
  </p>
  {#if !ai.data}<p role="status">
      {ai.busy ? "Checking connection…" : "Could not check the connection."}
    </p>
    <button class="button" disabled={ai.busy} onclick={() => void ai.refresh()}
      >Try again</button
    >
  {:else if app.busy("ai-connect")}
    <div class="account">
      <span role="status"
        >{key
          ? "Checking your key…"
          : "Finish signing in, in your browser"}</span
      ><button
        class="button"
        onclick={() =>
          void app.perform("ai-cancel", () => call("cancel_ai_connect"))}
        >Cancel</button
      >
    </div>
  {:else if ai.data.connected}
    <div class="account">
      <span class="quiet-tag">CONNECTED</span><small
        >Yours · billed per request</small
      ><button
        class="button small"
        onclick={() =>
          void app.perform("privacy", () => call("open_openrouter_privacy"))}
        >Privacy ↗</button
      ><button
        class="button small"
        disabled={locked}
        onclick={() => void disconnect()}>Disconnect</button
      >
    </div>
  {:else}
    <div class="account">
      <button
        class="button primary"
        disabled={locked || !ai.data.supported}
        onclick={() => void connect()}>Sign in with OpenRouter</button
      ><span>or</span>
    </div>
    <form
      class="key-entry"
      onsubmit={(event) => {
        event.preventDefault();
        void connect(true);
      }}
    >
      <input
        type="password"
        aria-label="OpenRouter key"
        placeholder="Paste a key · sk-or-v1-…"
        autocomplete="off"
        spellcheck="false"
        maxlength="4096"
        bind:value={key}
        disabled={locked || !ai.data.supported}
      /><button
        class="button"
        disabled={locked || !key.trim() || !ai.data.supported}>Use key</button
      >
    </form>
    {#if !ai.data.supported}<p class="field-help">{ai.data.help}</p>{/if}
  {/if}
  {#if error}<p class="inline-error" role="alert">{error}</p>{/if}
  {#if notice}<p class="field-help" role="status">{notice}</p>{/if}
  {#if !app.busy("ai-connect")}
    <h3 data-text-surface="emboss">Your models</h3>
    {#each visible as model (model.id)}{@render modelRow(model)}{/each}
    {#if yours.length > 3}<button
        class="text-action"
        onclick={() => (expanded = !expanded)}
        >{expanded
          ? "Show fewer"
          : `${yours.length - 3} more ${yours.length === 4 ? "model" : "models"}`}</button
      >{/if}
    {#if loading}<p role="status" class="field-help">
        Loading models…
      </p>{:else if !yours.length}<p class="field-help">
        Choose a model from the catalog below.
      </p>{/if}
    {#if failed}<p class="field-help">
        Could not refresh every model list. Saved choices are unchanged.
      </p>
      <button
        class="button small"
        disabled={loading}
        onclick={() => void loadModels()}>Try again</button
      >{/if}
    <div class="online-search">
      <Well
        ><span aria-hidden="true">⌕</span><input
          aria-label="All OpenRouter models"
          placeholder="All OpenRouter models"
          bind:value={query}
          oninput={() => (shown = 12)}
        /></Well
      >
    </div>
    {#if query.trim()}
      <div aria-label="OpenRouter catalog results">
        {#each matches as model (model.id)}{@render modelRow(model)}{/each}
      </div>
      {#if !matches.length && !loading}<p class="field-help">
          No other matching {role === "meeting" ? "audio" : "text"} models.
        </p>{/if}
      {#if matches.length === shown}<button
          class="button small"
          onclick={() => (shown += 12)}>Show more</button
        >{/if}
    {/if}
  {/if}
  {#if role !== "manage"}<button
      class="text-action"
      disabled={locked || app.busy("ai-connect")}
      onclick={async () => {
        if (await discardKey()) openRouter.open("manage");
      }}>Manage saved models</button
    >{/if}
</Sheet>

<style>
  .sheet-context {
    color: var(--soft-ink);
    margin: -8px 0 18px;
    font-size: 15px;
  }
  .account,
  .key-entry {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    margin-bottom: 14px;
  }
  .account small {
    flex: 1;
    color: var(--soft-ink);
  }
  .key-entry input {
    flex: 1;
    min-width: 180px;
    font-size: 15px;
  }
  h3 {
    font-size: 16px;
    margin: 22px 0 8px;
  }
  .cost {
    font-size: 13px;
    text-align: right;
    white-space: nowrap;
    flex: none;
    color: var(--soft-ink);
  }
  .option-copy .quiet-tag {
    display: table;
    margin-top: 5px;
  }
  .model-details {
    margin: 0 12px 12px 32px;
    font-size: 14px;
  }
  .online-search {
    margin-top: 18px;
  }
  .inline-error {
    color: var(--danger);
    font-size: 15px;
  }
</style>
