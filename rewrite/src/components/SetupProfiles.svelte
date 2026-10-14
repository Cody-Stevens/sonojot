<script lang="ts">
  import { onMount } from "svelte";
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
    profileLabel,
    sameProfileModels,
    setupProfiles,
  } from "../lib/setup-profiles";

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
  const inUse = $derived(
    !unfinished &&
      selected !== "custom" &&
      app.snapshot.settings.setupProfile === selected &&
      ai.data?.backend === "local",
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
      {#each modelRoles as role}
        {@const id = selection[role.setting]}
        {@const model = models.find((item) => item.id === id)}
        <p
          id={`setup-${role.id}`}
          class="profile-summary"
          tabindex="-1"
          role="group"
          aria-label={role.title}
          data-text-surface="flat"
        >
          {role.title}: {id === "local-rules"
            ? cleanupName
            : (model?.name ??
              app.snapshot.models.find((item) => item.id === id)?.name ??
              (library.data
                ? id || "No model selected"
                : "Reading model details…"))}
          {#if id === "local-rules" && cleanupStatus}
            · {cleanupStatus}{:else if model && checking(model)}
            · Checking…{/if}
        </p>
      {/each}
    {:else}<p class="field-help" data-text-surface="flat">
        Reading model choices…
      </p>{/if}
  {/if}
</div>

{#if inUse}
  <p class="profile-action">
    <span class="in-use" data-text-surface="flat">In use</span>
  </p>
{:else if selected !== "custom" || unfinished}
  <div class="actions profile-action">
    <button
      class="button primary"
      disabled={profiles.applyLocked ||
        speechChecking ||
        (selected === "custom"
          ? !speechReady
          : !recommendation?.asrModel || profiles.checking)}
      onclick={() => profiles.use()}
    >
      {profiles.busy
        ? "Preparing…"
        : unfinished
          ? "Finish setup"
          : `Use ${profileLabel(selected)}`}
    </button>
    <span class="field-help" data-text-surface="flat">
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
        Choose and install a speech model below.
      {/if}
    </span>
  </div>
{/if}

<details class="profile-memory">
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
    font-size: 0.8rem;
    line-height: 1.25;
    margin-top: 4px;
  }
  .profile-details p {
    margin: 9px 0 0;
    overflow-wrap: anywhere;
  }
  .profile-details .profile-description {
    margin-bottom: 18px;
  }
  .profile-summary,
  .profile-memory {
    color: var(--soft-ink);
  }
  .profile-summary {
    scroll-margin-top: 18px;
  }
  .profile-memory {
    margin-top: 18px;
  }
  .profile-memory summary {
    cursor: pointer;
  }
  .profile-summary:focus-visible,
  .profile-memory summary:focus-visible {
    outline: 2px solid var(--ink);
    outline-offset: 3px;
  }
  .profile-action {
    margin-top: 18px;
  }
  .in-use {
    display: inline-block;
    padding: 3px 9px;
    border: 1px solid var(--line);
    border-radius: 4px;
    color: var(--soft-ink);
  }
</style>
