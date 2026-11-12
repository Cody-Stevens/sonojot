<script lang="ts">
  import SettingRow from "../components/SettingRow.svelte";
  import UpdateAction from "../components/UpdateAction.svelte";
  import { app } from "../lib/app.svelte";
  import { updateStatusLabel } from "../lib/updates";
  const update = $derived(app.snapshot.update);
  const channelLocked = $derived(
    !app.native ||
      app.busy("settings") ||
      update.installing ||
      ["checking", "downloading", "ready"].includes(update.status.state) ||
      !["idle", "error"].includes(app.snapshot.state) ||
      app.snapshot.meetingRecording,
  );
</script>

<section class="stock paper-card quiet-settings">
  <SettingRow label="Version"
    ><span>{app.snapshot.version}</span><span class="field-help"
      >{updateStatusLabel(update.status)}</span
    ><button
      class="button"
      disabled={!app.native ||
        app.busy("update-check") ||
        update.installing ||
        ["checking", "downloading"].includes(update.status.state)}
      onclick={() => app.checkForUpdates()}>Check now</button
    ><UpdateAction
      describedby={app.updateRestartBlockedReason
        ? "restart-reason"
        : undefined}
    /></SettingRow
  >
  {#if update.installProblem}<p class="field-help" role="status">
      {update.installProblem}
    </p>{/if}
  {#if app.updateRestartBlockedReason}<p
      id="restart-reason"
      class="field-help"
      role="status"
    >
      {app.updateRestartBlockedReason}
    </p>{/if}
  <SettingRow label="Channel"
    ><div class="segments" aria-label="Update channel">
      <button
        aria-pressed={app.snapshot.settings.updateChannel === "stable"}
        disabled={channelLocked}
        onclick={() => app.saveSettings({ updateChannel: "stable" })}
        >Stable</button
      ><button
        aria-pressed={app.snapshot.settings.updateChannel === "nightly"}
        disabled={channelLocked}
        onclick={() => app.saveSettings({ updateChannel: "nightly" })}
        >Nightly</button
      >
    </div></SettingRow
  >
</section>
<p class="field-help">
  Nightly builds are early builds for testing. Changing the channel offers its
  newest build, even when that is older than this one. Your words and settings
  stay; features only in the newer build go away.
</p>
