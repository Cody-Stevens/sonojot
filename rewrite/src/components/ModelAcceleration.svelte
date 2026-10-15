<script lang="ts">
  import { untrack } from "svelte";
  import { app } from "../lib/app.svelte";
  import { acceleration } from "../lib/acceleration.svelte";
  import { isGpuAsrModel, isMetalAsrModel, modelAcceleration } from "../lib/acceleration";
  let {
    modelId,
    kind,
    controls = false,
    compact = false,
    disabled = false,
  }: {
    modelId: string;
    kind: "asr" | "cleanup";
    controls?: boolean;
    compact?: boolean;
    disabled?: boolean;
  } = $props();
  const id = $props.id();
  const localModel = $derived(kind === "cleanup" && modelId !== "local-rules");
  const gpuAsr = $derived(kind === "asr" && isGpuAsrModel(modelId));
  const needsStatus = $derived(
    localModel ||
      gpuAsr ||
      (kind === "asr" && isMetalAsrModel(modelId)),
  );
  const platform = $derived(app.snapshot.capabilities.platform);
  const isMac = $derived((platform ?? "").toLowerCase() === "macos");
  const gpuOption = $derived(
    isMac ? "Metal when available" : platform === "linux" ? "Vulkan when available" : "NVIDIA CUDA when available",
  );
  const info = $derived(
    modelAcceleration(
      modelId,
      kind,
      acceleration.data,
      app.snapshot.settings.textAcceleration === "cpu",
      platform,
    ),
  );
  $effect(() => {
    if (needsStatus) return acceleration.acquire();
  });
  const phase = $derived(
    [
      modelId,
      app.snapshot.sessionId,
      app.snapshot.state,
      app.snapshot.recognitionProgress?.stage,
      app.snapshot.resultSessionId,
    ].join("|"),
  );
  $effect(() => {
    if (phase && needsStatus && app.ready && app.native) {
      untrack(() => {
        void acceleration.refresh();
      });
    }
  });
</script>

<div class="model-acceleration" class:compact data-model-id={modelId}>
  <span
    class="device-badge"
    class:gpu={info.gpu}
    title={info.detail}
    data-text-surface="flat"
  >
    <svg
      viewBox="0 0 24 24"
      width="15"
      height="15"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      aria-hidden="true"
    >
      <rect x="6" y="6" width="12" height="12" rx="2" />
      <path d="M9 2v4m6-4v4M9 18v4m6-4v4M2 9h4m-4 6h4M18 9h4m-4 6h4" />
    </svg>
    {#if needsStatus && !app.native}Acceleration checked in desktop app
    {:else if needsStatus && acceleration.unavailable}Acceleration status
      unavailable
    {:else}{info.label}{/if}
  </span>
  {#if controls && localModel}
    <div class="device-choice">
      <label for={id}>Text model processing</label>
      <select
        {id}
        value={app.snapshot.settings.textAcceleration}
        disabled={!app.native || disabled || app.busy("settings")}
        onchange={async (event) => {
          const textAcceleration = event.currentTarget.value;
          if (textAcceleration !== "auto" && textAcceleration !== "cpu") return;
          await app.saveSettings({ textAcceleration });
          await acceleration.refresh();
        }}
      >
        <option value="auto">{gpuOption}</option>
        <option value="cpu">CPU only</option>
      </select>
    </div>
  {/if}
</div>

<style>
  .model-acceleration {
    margin: 9px 0 18px;
    min-width: 0;
  }
  .compact {
    margin: 4px 0 0;
  }
  .device-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 0.78rem;
    line-height: 1.4;
    color: var(--soft-ink);
  }
  .device-badge.gpu {
    color: var(--ink);
  }
  .device-badge svg {
    flex: none;
  }
  .device-choice {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 9px 18px;
    margin-top: 9px;
  }
  .device-choice label {
    font-size: 0.83rem;
  }
  .device-choice select {
    width: auto;
    min-width: 180px;
    max-width: 100%;
  }
</style>
