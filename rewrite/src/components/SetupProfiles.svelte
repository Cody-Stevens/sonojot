<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import { app } from "../lib/app.svelte";
  import { ai } from "../lib/ai.svelte";
  import { library } from "../lib/model-library.svelte";
  import {
    checking,
    formatBytes,
    installed,
    modelRoles,
  } from "../lib/model-library";
  import { profiles } from "../lib/setup-profiles.svelte";
  import {
    PUNCTUATION_MODEL,
    defaultCleanupName,
    defaultCleanupStatus,
    profileIncludesPunctuation,
  } from "../lib/punctuation";
  import ProfileRecommendation from "./ProfileRecommendation.svelte";
  import {
    missingDownloadBytes,
    modelInstallStatus,
    profileCardState,
    profileChangeText,
    profileLabel,
    sameProfileModels,
    setupProfiles,
  } from "../lib/setup-profiles";

  /** Custom's role pickers, shown where the automatic profiles list models. */
  let { custom }: { custom?: Snippet } = $props();
  const models = $derived(library.data?.models ?? []);
  const selected = $derived(profiles.selected);
  const recommendation = $derived(
    selected === "custom" ? null : profiles.hardware?.recommendations[selected],
  );
  // Preview the same recommendation that Use this will apply. Before hardware
  // returns, an existing profile can still show its saved model names.
  const selection = $derived(
    recommendation ??
      (profiles.draft === null &&
      app.snapshot.settings.setupProfile === selected
        ? { ...app.snapshot.settings, reasons: [] }
        : null),
  );
  const includePunctuation = $derived(
    profileIncludesPunctuation(app.snapshot.settings.setupProfile, selected),
  );
  // Default Cleanup is rules, vocabulary and the punctuation model.
  const cleanupName = $derived(defaultCleanupName(models));
  const cleanupStatus = $derived(
    defaultCleanupStatus(models, includePunctuation),
  );
  const missing = $derived(
    selection
      ? missingDownloadBytes(selection, models, includePunctuation)
      : null,
  );
  const unfinished = $derived(
    app.snapshot.settings.setupProfile === null || profiles.needsCompletion,
  );
  const speech = $derived(
    models.find(
      (model) =>
        model.id ===
        (selected === "custom"
          ? app.snapshot.settings.asrModel
          : recommendation?.asrModel),
    ),
  );
  const speechChecking = $derived(!!speech && checking(speech));
  const speechReady = $derived(!!speech && installed(speech));
  const checkingSelection = $derived(
    !!selection &&
      models.some(
        (model) =>
          (modelRoles.some((role) => selection[role.setting] === model.id) ||
            (includePunctuation && model.id === PUNCTUATION_MODEL)) &&
          checking(model),
      ),
  );
  const equivalent = $derived(
    selected === "accurate" &&
      !!selection &&
      !!profiles.hardware &&
      sameProfileModels(selection, profiles.hardware.recommendations.balanced),
  );
  const description = $derived(
    equivalent
      ? "Same models as Balanced on this computer"
      : setupProfiles.find((profile) => profile.id === selected)?.subtitle,
  );
  // "In use" only when the saved models are this profile's current picks.
  const card = $derived(
    profileCardState({
      saved: app.snapshot.settings.setupProfile,
      selected,
      unfinished,
      settings: app.snapshot.settings,
      recommendation,
      localNotes: ai.data ? ai.data.backend === "local" : null,
    }),
  );
  function modelName(id: string): string {
    if (id === "local-rules") return cleanupName;
    if (id === "openrouter") return "an OpenRouter model";
    return (
      models.find((model) => model.id === id)?.name ??
      app.snapshot.models.find((model) => model.id === id)?.name ??
      (id || "no model")
    );
  }
  const changeText = $derived(
    card.kind === "changed" && selected !== "custom"
      ? profileChangeText(selected, card.changes, modelName)
      : "",
  );
  const failure = $derived(
    profiles.failure?.profile === selected ? profiles.failure.message : "",
  );
  const applyDisabled = $derived(
    profiles.applyLocked ||
      speechChecking ||
      (selected === "custom"
        ? !speechReady
        : !recommendation?.asrModel || profiles.checking),
  );
  onMount(() => {
    void profiles.check();
  });
</script>

<div class="segments profile-choices" role="group" aria-label="Setup profile">
  {#each setupProfiles as profile}
    <button
      type="button"
      aria-pressed={selected === profile.id}
      disabled={profiles.locked}
      onclick={() => profiles.choose(profile.id)}
    >
      <span data-text-surface="emboss">{profile.title}</span>
      {#if profile.id === "balanced"}<small data-text-surface="flat"
          >Recommended</small
        >{/if}
    </button>
  {/each}
</div>

<div class="profile-details" role="group" aria-label="Selected profile details">
  <p class="profile-description" data-text-surface="flat">{description}</p>
  {#if selected !== "custom"}
    {#if selection}
      <!-- One row per role: label, model, install state. The same columns as
           Custom's pickers, so switching tabs never reflows the layout. -->
      <div class="role-list">
        {#each modelRoles as role}
          {@const id = selection[role.setting]}
          {@const model = models.find((item) => item.id === id)}
          <!-- Every row states its install state, so a recommended model that
               is missing never reads as the one in use. -->
          {@const status =
            id === "local-rules" ? cleanupStatus : modelInstallStatus(model)}
          <p
            id={`setup-${role.id}`}
            class="profile-summary"
            tabindex="-1"
            role="group"
            aria-label={role.title}
          >
            <span class="role" data-text-surface="flat"
              >{role.title}<span class="sr-only">:</span></span
            >{" "}<span class="name"
              >{id === "local-rules"
                ? cleanupName
                : (model?.name ??
                  app.snapshot.models.find((item) => item.id === id)?.name ??
                  (library.data
                    ? id || "No model selected"
                    : "Reading model details…"))}</span
            >{#if status}{" "}<span class="state" data-text-surface="flat"
                ><span class="sr-only">{"· "}</span>{status}</span
              >{/if}
          </p>
        {/each}
      </div>
    {:else}<p class="meta" data-text-surface="flat">
        Reading model choices…
      </p>{/if}
  {:else}{@render custom?.()}{/if}
</div>

{#if card.kind === "in-use"}
  <p class="profile-action in-use-line">
    <Icon name="check" size={18} /><span class="in-use" data-text-surface="flat"
      >In use</span
    >
  </p>
{:else if card.kind === "unknown"}
  {#if profiles.checking || !ai.data}<p
      class="profile-action meta"
      data-text-surface="flat"
    >
      Checking…
    </p>{/if}
{:else if card.kind === "changed"}
  <!-- The saved profile's recommendation changed after setup (an update can
       do that). Say so, and offer the switch; never switch by itself. -->
  <div class="profile-action profile-change">
    <p data-text-surface="flat">{changeText}</p>
    <div class="actions">
      <button
        class="button primary"
        disabled={applyDisabled}
        onclick={() => profiles.use()}
      >
        {profiles.busy
          ? "Preparing…"
          : `Switch to ${profileLabel(selected)}${
              missing ? ` · ${formatBytes(missing)} download` : ""
            }`}
      </button>
      {#if checkingSelection}<span class="meta" data-text-surface="flat"
          >Checking…</span
        >{/if}
    </div>
  </div>
{:else if selected !== "custom" || unfinished}
  <div class="actions profile-action">
    <button
      class="button primary"
      disabled={applyDisabled}
      onclick={() => profiles.use()}
    >
      {profiles.busy
        ? "Preparing…"
        : unfinished
          ? "Finish setup"
          : `Use ${profileLabel(selected)}`}
    </button>
    <span class="meta" data-text-surface="flat">
      {#if selected !== "custom" && selection}
        {checkingSelection
          ? "Checking…"
          : missing === null
            ? "Download size unavailable"
            : missing === 0
              ? "No download needed"
              : `${formatBytes(missing)} to download`}
      {:else if selected === "custom" && speechChecking}
        Checking…
      {:else if selected === "custom" && !speechReady}
        Choose and install a speech model above.
      {/if}
    </span>
  </div>
{/if}
{#if failure}<p
    class="profile-failure danger-text"
    role="alert"
    data-text-surface="flat"
  >
    {failure}
  </p>{/if}

<details class="profile-memory fold">
  <summary data-text-surface="flat"
    >Why we recommend {profileLabel(
      selected === "custom" ? "balanced" : selected,
    )}</summary
  >
  <ProfileRecommendation
    profile={selected === "custom" ? "balanced" : selected}
  />
</details>

<style>
  /* Same recessed paper track as Dictation's cleanup-mode segments. */
  .profile-choices button {
    display: flex;
    flex-direction: column;
    align-items: center;
    flex: 1 1 120px;
    font-size: 1rem;
    overflow-wrap: anywhere;
  }
  .profile-choices small {
    font-size: 15px;
    line-height: 1.25;
    margin-top: 4px;
  }
  .profile-details p {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .profile-details .profile-description {
    margin-bottom: 18px;
  }
  /* Label, model, state: the shared row anatomy of the Models card. */
  .role-list {
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  .profile-summary {
    display: grid;
    grid-template-columns: var(--role-label, 108px) minmax(0, 1fr) auto;
    align-items: baseline;
    gap: 0 18px;
    scroll-margin-top: 18px;
  }
  .profile-summary .role,
  .profile-summary .state,
  .meta,
  .profile-memory {
    color: var(--soft-ink);
  }
  .profile-summary .state,
  .meta {
    font-size: 15px;
  }
  .profile-summary .state {
    text-align: right;
    white-space: nowrap;
  }
  .profile-memory {
    margin-top: 18px;
  }
  .profile-memory summary {
    cursor: pointer;
    font-size: 15px;
    width: fit-content;
  }
  .profile-memory :global(.recommendation) {
    margin-top: 9px;
    padding-left: 18px;
    font-size: 15px;
  }
  .profile-summary:focus-visible,
  .profile-memory summary:focus-visible {
    outline: 2px solid var(--ink);
    outline-offset: 3px;
  }
  .profile-action {
    margin-top: 27px;
  }
  .profile-change p,
  .profile-failure {
    margin: 0 0 18px;
    overflow-wrap: anywhere;
  }
  .profile-failure {
    margin: 9px 0 0;
  }
  .in-use-line {
    display: flex;
    align-items: center;
    gap: 9px;
    color: var(--dye);
    margin-bottom: 0;
  }
  .in-use {
    color: var(--ink);
  }
  @media (max-width: 550px) {
    .profile-summary {
      grid-template-columns: minmax(0, 1fr) auto;
    }
    .profile-summary .role {
      grid-column: 1 / -1;
    }
  }
</style>
