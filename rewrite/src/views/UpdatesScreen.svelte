<script lang="ts">
  import SettingRow from "../components/SettingRow.svelte";
  import UpdateAction from "../components/UpdateAction.svelte";
  import { app } from "../lib/app.svelte";
  import {
    updateOffered,
    updateStatusLabel,
    updateRestartLabel,
  } from "../lib/updates";
  import { whatsNew } from "../lib/whats-new.svelte";
  const update = $derived(app.snapshot.update);
  const offered = $derived(updateOffered(update.status));
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
    ><div class="update-summary">
      <span class="update-version">{app.snapshot.version}</span>
      <small data-text-surface="flat" role="status"
        >{update.installing
          ? updateRestartLabel(update.status)
          : updateStatusLabel(update.status)}</small
      >{#if update.installProblem}<small data-text-surface="flat" role="status"
          >{update.installProblem}</small
        >{/if}{#if app.updateRestartBlockedReason}<small
          id="restart-reason"
          data-text-surface="flat"
          role="status">{app.updateRestartBlockedReason}</small
        >{/if}
    </div>
    <div class="actions">
      {#if offered}<button
          type="button"
          class="button"
          onclick={() => void whatsNew.showOffer()}>See changes</button
        >{:else}<button
          type="button"
          class="button"
          disabled={!app.native ||
            app.busy("update-check") ||
            update.installing ||
            update.status.state === "checking"}
          onclick={() => app.checkForUpdates()}>Check now</button
        >{/if}<UpdateAction
        describedby={app.updateRestartBlockedReason
          ? "restart-reason"
          : undefined}
      />
    </div></SettingRow
  >
  <SettingRow label="What's new"
    ><button
      type="button"
      class="button"
      disabled={!whatsNew.installed}
      onclick={() => whatsNew.showInstalled()}>Show</button
    ></SettingRow
  >
  <SettingRow label="Channel"
    >{#snippet info()}<p>
        Nightly builds are early builds for testing. Changing the channel
        checks for a build for this computer. Returning to Stable can offer an
        older version.
      </p>
      <p>
        Your words and settings stay; features only in the newer build go away.
      </p>{/snippet}<div class="segments" aria-label="Update channel">
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

<style>
  /* Version and status above the buttons, all in the control column.
     Long nightly versions and progress wrap instead of widening the row. */
  .update-summary {
    flex: 1 0 100%;
    display: grid;
    gap: 4px;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .update-version {
    font-variant-numeric: tabular-nums;
  }
  .update-summary small {
    color: var(--soft-ink);
    font-size: 15px;
    line-height: 1.35;
  }
</style>
