<script lang="ts">
  import SettingRow from "../components/SettingRow.svelte";
  import SegmentedControl from "../components/quiet/SegmentedControl.svelte";
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
  const channelOptions = [
    { value: "stable" as const, label: "Stable" },
    { value: "nightly" as const, label: "Nightly" },
  ];
</script>

<section class="stock paper-card ui-card row-card quiet-settings">
  <SettingRow label="Version" layout="choice">
    <div class="update-col">
      <div class="update-summary">
        <span class="update-version">{app.snapshot.version}</span>
        <small data-text-surface="flat" role="status"
          >{update.installing
            ? updateRestartLabel(update.status)
            : updateStatusLabel(update.status)}</small
        >{#if update.installProblem}<small
            data-text-surface="flat"
            role="status">{update.installProblem}</small
          >{/if}{#if app.updateRestartBlockedReason}<small
            id="restart-reason"
            data-text-surface="flat"
            role="status">{app.updateRestartBlockedReason}</small
          >{/if}
      </div>
      <div class="actions">
        {#if offered}<button
            type="button"
            class="button small"
            onclick={() => void whatsNew.showOffer()}>See changes</button
          >{:else}<button
            type="button"
            class="button small"
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
      </div>
    </div>
  </SettingRow>
  <SettingRow label="What's new" layout="action">
    <button
      type="button"
      class="button small"
      disabled={!whatsNew.installed}
      onclick={() => whatsNew.showInstalled()}>Show</button
    >
  </SettingRow>
  <SettingRow label="Channel" layout="choice">
    {#snippet info()}
      <p>
        Nightly builds are early builds for testing. Changing the channel checks
        for a build for this computer. Returning to Stable can offer an older
        version.
      </p>
      <p>
        Your words and settings stay; features only in the newer build go away.
      </p>
    {/snippet}
    <SegmentedControl
      label="Update channel"
      options={channelOptions}
      value={app.snapshot.settings.updateChannel === "nightly"
        ? "nightly"
        : "stable"}
      disabled={channelLocked}
      onchange={(updateChannel) => app.saveSettings({ updateChannel })}
    />
  </SettingRow>
</section>

<style>
  .update-col {
    display: grid;
    gap: 10px;
    width: 100%;
    min-width: 0;
  }
  .update-summary {
    display: grid;
    gap: 4px;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .update-version {
    font-variant-numeric: tabular-nums;
    font-weight: 550;
  }
  .update-summary small {
    color: var(--soft-ink);
    font-size: 14px;
    line-height: 1.4;
  }
</style>
