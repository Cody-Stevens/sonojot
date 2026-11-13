<script lang="ts">
  import { untrack } from "svelte";
  import { z } from "zod";
  import Toggle from "../Toggle.svelte";
  import { sync, syncAction } from "../../lib/sync.svelte";
  const schema = z.object({
    used: z.number(),
    quota: z.number(),
    audio: z.object({ audioDefault: z.boolean(), alwaysDownload: z.boolean() }),
  });
  let storage = $state<z.infer<typeof schema> | null>(null);
  let error = $state("");
  let busy = $state(false);
  const size = (bytes: number) =>
    bytes >= 1e9
      ? `${(bytes / 1e9).toFixed(1)} GB`
      : bytes >= 1e6
        ? `${(bytes / 1e6).toFixed(1)} MB`
        : `${Math.ceil(bytes / 1024)} KB`;
  async function reload() {
    try {
      storage = schema.parse(await syncAction("meeting_storage"));
    } catch (e) {
      error = String(e);
    }
  }
  $effect(() => {
    const last = sync.status?.lastSync;
    untrack(() => {
      void reload();
    });
  });
  async function change(audioDefault: boolean, alwaysDownload: boolean) {
    busy = true;
    error = "";
    try {
      await syncAction("meeting_preferences", { audioDefault, alwaysDownload });
      await reload();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="storage" data-text-surface="flat">
  {#if storage}
    <p>
      Meeting storage: {size(storage.used)}{storage.quota > 0
        ? ` of ${size(storage.quota)}`
        : ". Sync to check your allowance."}
    </p>
    <div class="audio-choice">
      <span>Upload meeting audio by default</span><Toggle
        label="Upload meeting audio by default"
        disabled={busy}
        checked={storage.audio.audioDefault}
        onchange={(value) =>
          change(value, storage?.audio.alwaysDownload ?? false)}
      />
    </div>
    <div class="audio-choice">
      <span>Always download synced audio</span><Toggle
        label="Always download synced audio"
        disabled={busy}
        checked={storage.audio.alwaysDownload}
        onchange={(value) =>
          change(storage?.audio.audioDefault ?? false, value)}
      />
    </div>
    <p class="field-help">
      Audio uses about 0.5 MB per minute for one source, or 1 MB for microphone
      and system audio.
    </p>
    <p class="field-help">
      When storage fills, the oldest meetings stop syncing after a device
      confirms a complete local copy. Turning audio uploads off keeps audio
      already synced.
    </p>
  {:else}<p>Loading meeting storage…</p>{/if}
  {#if error}<p role="alert">{error}</p>
    <button class="button small" onclick={reload}>Retry</button>{/if}
</div>

<style>
  .storage {
    margin: 18px 0;
  }
  .audio-choice {
    display: flex;
    gap: 9px;
    align-items: center;
    justify-content: space-between;
    margin: 9px 0;
  }
</style>
