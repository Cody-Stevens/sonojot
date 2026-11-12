<script lang="ts">
  import { app } from "../../lib/app.svelte";
  import { updateOffered, updateStatusLabel } from "../../lib/updates";
  import { whatsNew } from "../../lib/whats-new.svelte";
  import UpdateAction from "../UpdateAction.svelte";
  import Sheet from "./Sheet.svelte";
  let { onclose }: { onclose: () => void } = $props();
  const id = $props.id();
  const update = $derived(app.snapshot.update);
  const blocked = $derived(
    update.status.state === "ready" ? app.updateRestartBlockedReason : null,
  );
</script>

<Sheet title="Updates" {onclose}>
  <p class="update-line" role="status">
    {update.installing
      ? "Restarting to update…"
      : updateStatusLabel(update.status)}
  </p>
  {#if update.installProblem}<p class="update-line" data-text-surface="flat">
      {update.installProblem}
    </p>{/if}
  {#if blocked}<p {id} class="field-help update-line" data-text-surface="flat">
      {blocked}
    </p>{/if}
  <div class="actions">
    <UpdateAction describedby={blocked ? id : undefined} />
    {#if updateOffered(update.status)}
      <button
        type="button"
        class="button"
        onclick={() => void whatsNew.showOffer()}>See changes</button
      >
    {/if}
    {#if update.status.state === "idle" || update.status.state === "error"}
      <button
        type="button"
        class="button"
        disabled={!app.native || app.busy("update-check")}
        onclick={() => app.checkForUpdates()}>Check now</button
      >
    {/if}
    <button
      type="button"
      class="button quiet"
      disabled={!app.native || update.installing || app.busy("update-notice")}
      onclick={async () => {
        await app.dismissUpdateNotice();
        onclose();
      }}>Later</button
    >
  </div>
</Sheet>

<style>
  .update-line {
    margin: 0 0 18px;
    overflow-wrap: anywhere;
  }
</style>
