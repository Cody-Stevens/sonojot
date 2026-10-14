<script lang="ts">
  import { onMount } from "svelte";
  import ModelAcceleration from "./ModelAcceleration.svelte";
  import RemoteModelBadge from "./RemoteModelBadge.svelte";
  import { app } from "../lib/app.svelte";
  import { ai } from "../lib/ai.svelte";
  import { acceleration } from "../lib/acceleration.svelte";
  import { library } from "../lib/model-library.svelte";
  import { profileLabel } from "../lib/setup-profiles";
  import { activePunctuation } from "../lib/punctuation";
  import {
    modelRoles,
    shortDeviceLabel,
    type ModelRole,
  } from "../lib/model-library";

  let {
    role,
    onmanage,
  }: { role: ModelRole; onmanage?: (role: ModelRole) => void } = $props();
  const definition = $derived(
    modelRoles.find((item) => item.id === role) ?? modelRoles[0],
  );
  const modelId = $derived(app.snapshot.settings[definition.setting]);
  const model = $derived(
    library.data?.models.find((item) => item.id === modelId),
  );
  const snapshotModel = $derived(
    app.snapshot.models.find((item) => item.id === modelId),
  );
  const rulesOnly = $derived(role === "cleanup" && modelId === "local-rules");
  const remote = $derived(
    role === "notes" && ai.data?.backend === "openrouter",
  );
  const textModel = $derived(role !== "asr" && !rulesOnly && !remote);
  const runtime = $derived(
    acceleration.data?.textRuntime?.modelId === modelId
      ? acceleration.data.textRuntime
      : null,
  );
  const showAcceleration = $derived(
    textModel &&
      !!runtime &&
      runtime.backend !== "cpu" &&
      (runtime.loading || (runtime.gpuLayers ?? 0) > 0),
  );
  const punctuation = $derived(activePunctuation(library.data?.models ?? []));
  const summary = $derived(
    rulesOnly
      ? punctuation
        ? `Cleanup · ${punctuation.name} with your vocabulary · CPU`
        : "Cleanup · Formatting and vocabulary · no AI model"
      : role === "notes" && !ai.data
        ? ai.busy
          ? "Notes · Checking model route…"
          : "Notes · Model route unavailable"
        : remote
          ? `Notes · ${ai.data?.model || "No model selected"} · OpenRouter${ai.data?.connected ? "" : " · disconnected"}`
          : role === "notes" && modelId === "openrouter"
            ? "Notes · No local model selected"
            : `${definition.title} · ${model?.name ?? snapshotModel?.name ?? (modelId || "No model selected")} · ${model ? shortDeviceLabel(model) : "Device unavailable"}`,
  );
  const profile = $derived(profileLabel(app.snapshot.settings.setupProfile));

  onMount(() => {
    if (role === "notes") void ai.refresh();
    return library.acquire();
  });
  $effect(() => {
    if (textModel) return acceleration.acquire();
  });
</script>

<div class="model-summary">
  <p class="field-help" data-text-surface="flat">
    {#if role === "asr" && profile}{profile} ·
    {/if}{summary}{#if remote}<RemoteModelBadge />{/if}
  </p>
  {#if onmanage}<button
      class="button quiet small"
      onclick={() => onmanage?.(role)}
      aria-label={`Change ${definition.title} model`}>Change</button
    >{/if}
  {#if showAcceleration}
    <ModelAcceleration {modelId} kind="cleanup" compact />
  {/if}
</div>

<style>
  .model-summary {
    min-width: 0;
    margin-top: 18px;
    padding-bottom: 9px;
  }
  .field-help {
    margin: 0;
    overflow-wrap: anywhere;
    font-size: 1rem;
    line-height: 1.35;
  }
</style>
