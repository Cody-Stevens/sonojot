<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { updateAction, updateActionDisabled } from "../lib/updates";

  // The same next step as the sidebar update button, as an ordinary button
  // for About's update row and the main-window notice.
  let { describedby }: { describedby?: string } = $props();
  const update = $derived(app.snapshot.update);
  const action = $derived(updateAction(update.status, update.installing));
  const disabled = $derived(
    updateActionDisabled(
      update.status,
      update.installing,
      app.updateRestartBlockedReason,
      app.native,
      (key) => app.busy(key),
    ),
  );

  async function activate(): Promise<void> {
    if (disabled || !action) return;
    if (action.kind === "download") await app.downloadUpdate();
    else await app.restartToUpdate();
  }
</script>

{#if action}<button
    type="button"
    class="button small"
    class:primary={!disabled}
    class:quiet={disabled}
    aria-describedby={describedby}
    {disabled}
    onclick={activate}>{action.label}</button
  >{/if}
