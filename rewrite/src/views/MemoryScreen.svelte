<script lang="ts">
  import { onMount } from "svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import SegmentedControl from "../components/quiet/SegmentedControl.svelte";
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
  const memoryOptions: { value: MemoryMode; label: string }[] = [
    { value: "overlap", label: "Keep all ready" },
    { value: "sequential", label: "Load one at a time" },
  ];
  const retentionOptions = $derived<
    { value: ModelRetention; label: string; disabled?: boolean }[]
  >([
    {
      value: "immediately",
      label: "Immediately",
      disabled: settings.memoryMode === "overlap",
    },
    { value: "10m", label: "After 10 min" },
    { value: "30m", label: "After 30 min" },
    { value: "exit", label: "On quit" },
  ]);
  const accelOptions = [
    { value: "auto" as const, label: "GPU if possible" },
    { value: "cpu" as const, label: "CPU only" },
  ];
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
<section class="stock paper-card ui-card row-card quiet-settings">
  <SettingRow label="Models" layout="choice">
    <SegmentedControl
      label="Models in memory"
      options={memoryOptions}
      value={settings.memoryMode}
      disabled={locked}
      onchange={(memoryMode) =>
        void app.saveSettings({
          memoryMode,
          ...(memoryMode === "overlap" &&
          settings.modelRetention === "immediately"
            ? { modelRetention: "10m" as const }
            : {}),
        })}
    />
  </SettingRow>
  <SettingRow label="Unload when idle" layout="choice">
    <SegmentedControl
      label="Unload when idle"
      options={retentionOptions}
      value={settings.modelRetention}
      disabled={locked}
      onchange={(modelRetention) => void app.saveSettings({ modelRetention })}
    />
  </SettingRow>
  <SettingRow label="Text models" layout="choice">
    <SegmentedControl
      label="Text model processing"
      options={accelOptions}
      value={settings.textAcceleration === "cpu" ? "cpu" : "auto"}
      disabled={locked}
      onchange={async (textAcceleration) => {
        await app.saveSettings({ textAcceleration });
        await acceleration.refresh();
      }}
    />
  </SettingRow>
</section>
{#if locked}<p class="field-help">
    {library.error ||
      "Memory changes wait for capture, downloads and model processing to finish."}
  </p>{/if}
