<script lang="ts">
  import { onMount } from "svelte";
  import Sheet from "./Sheet.svelte";
  import { app } from "../../lib/app.svelte";
  import { ai } from "../../lib/ai.svelte";
  import { library } from "../../lib/model-library.svelte";
  import { profiles } from "../../lib/setup-profiles.svelte";
  import { profileChanges } from "../../lib/setup-profiles";
  import { modelName } from "../../lib/model-names";
  import { call } from "../../lib/api";
  import type { AiModel } from "../../lib/ai";
  let { onclose }: { onclose: () => void } = $props();
  let catalog = $state<AiModel[]>([]);
  const changes = $derived(
    profiles.hardware
      ? profileChanges(
          app.snapshot.settings,
          profiles.hardware.recommendations.balanced,
          ai.data,
          app.snapshot.localProofreaderAvailable,
        )
      : [],
  );
  onMount(() => {
    let active = true;
    void Promise.allSettled([
      call("list_ai_models"),
      call("list_ai_catalog"),
    ]).then((results) => {
      if (active)
        catalog = results.flatMap((result) =>
          result.status === "fulfilled" ? result.value : [],
        );
    });
    return () => {
      active = false;
    };
  });
</script>

<Sheet title="Back to recommended setup" {onclose}>
  <p>These choices will change:</p>
  <ul>
    {#each changes as change}<li>
        <strong data-text-surface="emboss"
          >{{
            asr: "Dictation speech",
            cleanup: "Cleanup",
            notes: "Notes",
            meetingSpeech: "Meeting speech",
          }[change.role]}</strong
        >:
        {modelName(change.current, library.data?.models ?? [], catalog)} → {modelName(
          change.recommended,
          library.data?.models ?? [],
          catalog,
        )}
      </li>{/each}
  </ul>
  <p>Your OpenRouter connection and keys stay saved.</p>
  <div class="sheet-actions">
    <button class="button" onclick={onclose}>Cancel</button>
    <button
      class="button"
      disabled={profiles.applyLocked}
      onclick={() => {
        onclose();
        void profiles.use("balanced", true);
      }}>Use recommended setup</button
    >
  </div>
</Sheet>
