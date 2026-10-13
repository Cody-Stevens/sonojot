<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { library } from "../lib/model-library.svelte";
  import { profiles } from "../lib/setup-profiles.svelte";
  import {
    hardwareLine,
    plainProfileReason,
    profileSpeechMemory,
    type AutomaticProfile,
  } from "../lib/setup-profiles";
  let { profile }: { profile: AutomaticProfile } = $props();
  const selection = $derived(profiles.hardware?.recommendations[profile]);
  const speech = $derived(
    library.data?.models.find((model) => model.id === selection?.asrModel),
  );
  const memory = $derived(
    profileSpeechMemory(speech, profiles.hardware?.hardware.platform),
  );
</script>

<div class="recommendation" data-text-surface="flat">
  <div class="hardware-row">
    <p aria-label="Computer hardware">
      {profiles.hardware
        ? hardwareLine(profiles.hardware.hardware)
        : "Hardware details unavailable"}
    </p>
    <button
      class="button quiet small"
      disabled={!app.native || profiles.checking || profiles.busy}
      onclick={() => profiles.check()}
      >{profiles.checking ? "Checking…" : "Check again"}</button
    >
  </div>
  <p>
    {memory.label.replace("Speech about", "Speech uses about")}. Notes loads
    only when you use it.
  </p>
  <p>{plainProfileReason(profile, speech)}</p>
</div>

<style>
  .hardware-row {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 9px 18px;
  }
  p {
    margin: 9px 0;
    overflow-wrap: anywhere;
  }
  .hardware-row p {
    flex: 1 1 280px;
  }
  .recommendation {
    color: var(--soft-ink);
  }
</style>
