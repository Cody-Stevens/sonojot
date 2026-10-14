<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { updateNoticeMessage } from "../lib/updates";
  import UpdateAction from "./UpdateAction.svelte";

  // One slip above the page while an update waits. Later hides it for this
  // version until SonoJot next starts; the sidebar button stays.
  const update = $derived(app.snapshot.update);
  const message = $derived(updateNoticeMessage(update));
  const reason = $derived(
    update.status.state === "ready" && !update.installing
      ? app.updateRestartBlockedReason
      : null,
  );
</script>

{#if message}<section class="notice update-notice" aria-label="Update">
    <div class="update-text">
      <span role="status">{message}</span>
      {#if reason}<p
          id="update-notice-reason"
          class="field-help"
          data-text-surface="flat"
        >
          {reason}
        </p>{/if}
    </div>
    <div class="update-buttons">
      <UpdateAction describedby={reason ? "update-notice-reason" : undefined} />
      {#if !update.installing}<button
          type="button"
          class="button small quiet"
          disabled={!app.native || app.busy("update-notice")}
          onclick={() => app.dismissUpdateNotice()}>Later</button
        >{/if}
    </div>
  </section>{/if}

<style>
  .update-notice {
    align-items: center;
    flex-wrap: wrap;
    overflow-wrap: anywhere;
  }
  .update-text {
    flex: 1 1 14rem;
    min-width: 0;
  }
  .update-text .field-help {
    margin: var(--space-1) 0 0;
  }
  .update-buttons {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
</style>
