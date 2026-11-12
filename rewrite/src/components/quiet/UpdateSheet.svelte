<script lang="ts">
  import { app } from "../../lib/app.svelte";
  import { updateStatusLabel } from "../../lib/updates";
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
  <p role="status">
    {update.installing
      ? "Restarting to update…"
      : updateStatusLabel(update.status)}
  </p>
  {#if update.installProblem}<p data-text-surface="flat">
      {update.installProblem}
    </p>{/if}
  {#if blocked}<p {id} class="field-help" data-text-surface="flat">
      {blocked}
    </p>{/if}
  <div class="actions">
    <UpdateAction describedby={blocked ? id : undefined} />
    {#if update.status.state === "idle" || update.status.state === "error"}
      <button
        class="button"
        disabled={!app.native || app.busy("update-check")}
        onclick={() => app.checkForUpdates()}>Check now</button
      >
    {/if}
    <button
      class="button quiet"
      disabled={!app.native || update.installing || app.busy("update-notice")}
      onclick={async () => {
        await app.dismissUpdateNotice();
        onclose();
      }}>Later</button
    >
  </div>
</Sheet>
