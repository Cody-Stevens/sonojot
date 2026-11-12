<script lang="ts">
  import { onMount } from "svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import { app } from "../lib/app.svelte";
  import { library } from "../lib/model-library.svelte";
  import { profiles } from "../lib/setup-profiles.svelte";
  import { ai } from "../lib/ai.svelte";
  import { acceleration } from "../lib/acceleration.svelte";
  import { installing, formatBytes } from "../lib/model-library";
  import type { MemoryMode, ModelRetention } from "../lib/protocol";
  const settings = $derived(app.snapshot.settings);
  const hardware = $derived(library.data?.hardware);
  const locked = $derived(
    !app.native ||
      !app.ready ||
      !library.data ||
      profiles.busy ||
      ai.busy ||
      app.busy("settings") ||
      app.snapshot.meetingRecording ||
      !["idle", "error"].includes(app.snapshot.state) ||
      library.data.models.some(
        (model) =>
          installing(model) ||
          ["loading", "busy", "unloading"].includes(model.state),
      ),
  );
  onMount(() => {
    const release = library.acquire();
    return release;
  });
</script>

<p class="settings-hardware" data-text-surface="flat">
  {formatBytes(hardware?.totalRamBytes ?? null)}
  {hardware?.unifiedMemory
    ? "unified memory"
    : `system RAM · ${formatBytes(hardware?.dedicatedVramBytes ?? null)} graphics memory`}
</p>
<section class="stock paper-card quiet-settings">
  <SettingRow label="Models"
    ><div class="segments" aria-label="Models in memory">
      {#each [{ value: "overlap" as MemoryMode, label: "Keep all ready" }, { value: "sequential" as MemoryMode, label: "Load one at a time" }] as item}<button
          aria-pressed={settings.memoryMode === item.value}
          disabled={locked}
          onclick={() =>
            app.saveSettings({
              memoryMode: item.value,
              ...(item.value === "overlap" &&
              settings.modelRetention === "immediately"
                ? { modelRetention: "10m" as const }
                : {}),
            })}>{item.label}</button
        >{/each}
    </div></SettingRow
  >
  <SettingRow label="Unload when idle"
    ><div class="segments" aria-label="Unload when idle">
      {#each [{ value: "immediately" as ModelRetention, label: "Immediately" }, { value: "10m" as ModelRetention, label: "After 10 min" }, { value: "30m" as ModelRetention, label: "After 30 min" }, { value: "exit" as ModelRetention, label: "On quit" }] as item}<button
          aria-pressed={settings.modelRetention === item.value}
          disabled={locked ||
            (item.value === "immediately" && settings.memoryMode === "overlap")}
          onclick={() => app.saveSettings({ modelRetention: item.value })}
          >{item.label}</button
        >{/each}
    </div></SettingRow
  >
  <SettingRow label="Text models"
    ><div class="segments" aria-label="Text model processing">
      <button
        aria-pressed={settings.textAcceleration === "auto"}
        disabled={locked}
        onclick={async () => {
          await app.saveSettings({ textAcceleration: "auto" });
          await acceleration.refresh();
        }}>GPU if possible</button
      ><button
        aria-pressed={settings.textAcceleration === "cpu"}
        disabled={locked}
        onclick={async () => {
          await app.saveSettings({ textAcceleration: "cpu" });
          await acceleration.refresh();
        }}>CPU only</button
      >
    </div></SettingRow
  >
</section>
{#if locked}<p class="field-help">
    {library.error ||
      "Memory changes wait for capture, downloads and model processing to finish."}
  </p>{/if}
