<script lang="ts">
  import { app } from "../lib/app.svelte";
  import {
    dictationReady,
    selectDictationControl,
    type DictationAction,
  } from "../lib/dictation-control";
  import Icon from "./Icon.svelte";
  let {
    compact = false,
    microphoneTest = false,
    onaction,
  }: {
    compact?: boolean;
    microphoneTest?: boolean;
    onaction?: (action: DictationAction) => void;
  } = $props();
  const control = $derived(
    selectDictationControl({
      snapshot: app.snapshot,
      native: app.native,
      ready:
        app.ready &&
        ((!microphoneTest && app.snapshot.settings.setupProfile === null) ||
          dictationReady(app.snapshot)),
      pending: app.pending,
    }),
  );
  async function act(action: DictationAction): Promise<void> {
    onaction?.(action);
    await app.dictation(action, microphoneTest);
  }
</script>

<div class="actions" class:compact>
  <button
    class="button primary"
    disabled={control.disabled}
    onclick={() => act(control.action)}
  >
    <Icon
      name={control.active || control.finishing || control.cleaning
        ? "stop"
        : "microphone"}
      size={compact ? 19 : 21}
    />
    {microphoneTest && control.action === "start" && !control.finishing
      ? "Test with a dictation"
      : control.label}
  </button>
  {#if control.cleaning}<span class="muted"
      >{app.snapshot.settings.hotkey} · Esc</span
    >{/if}
  {#if control.showCancel}<button
      class="button quiet"
      title="Cancel dictation without publishing this recording"
      disabled={control.cancelDisabled}
      onclick={() => act("cancel")}
      >{compact ? "Cancel" : "Cancel dictation"}</button
    >{/if}
</div>
