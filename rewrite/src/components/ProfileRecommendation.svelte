<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { library } from "../lib/model-library.svelte";
  import { profiles } from "../lib/setup-profiles.svelte";
  import InfoTip from "./quiet/InfoTip.svelte";
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
  <div class="reason">
    {plainProfileReason(profile, speech)}<InfoTip label="About this computer"
      ><p aria-label="Computer hardware">
        {profiles.hardware
          ? hardwareLine(profiles.hardware.hardware)
          : "Hardware details unavailable"}
      </p>
      <p>
        {memory.label.replace("Speech about", "Speech uses about")}. Notes loads
        only when you use it.
      </p>
      <button
        class="button quiet small"
        disabled={!app.native || profiles.checking || profiles.busy}
        onclick={() => profiles.check()}
        >{profiles.checking ? "Checking…" : "Check again"}</button
      ></InfoTip
    >
  </div>
</div>

<style>
  p,
  .reason {
    margin: 9px 0;
    overflow-wrap: anywhere;
  }
  .recommendation {
    color: var(--soft-ink);
  }
</style>
