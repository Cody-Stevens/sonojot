<script lang="ts">
  import { onMount } from "svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import SegmentedControl from "../components/quiet/SegmentedControl.svelte";
  import Toggle from "../components/Toggle.svelte";
  import Sheet from "../components/quiet/Sheet.svelte";
  import MacPasteAccess from "../components/MacPasteAccess.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import { formatBytes } from "../lib/model-library";
  let count = $state<number | null>(null);
  let audio = $state<{ count: number; bytes: number } | null>(null);
  let deleting = $state(false);
  const settings = $derived(app.snapshot.settings);
  const active = $derived(
    ["loading", "listening", "finishing", "cleaning"].includes(
      app.snapshot.state,
    ),
  );
  const locked = $derived(
    !app.native ||
      active ||
      app.busy("settings") ||
      app.busy("history-delete-all") ||
      app.busy("dictation-audio-delete"),
  );
  const retention = $derived<"nothing" | "text" | "audio">(
    settings.doNotSaveDictations
      ? "nothing"
      : settings.keepDictationAudio
        ? "audio"
        : "text",
  );
  const retentionOptions = [
    { value: "nothing" as const, label: "Nothing" },
    { value: "text" as const, label: "Text" },
    { value: "audio" as const, label: "Text and audio" },
  ];
  async function refresh() {
    await Promise.all([
      app.perform("history-count", async () => {
        count = await call("count_dictations");
      }),
      app.perform("dictation-audio-usage", async () => {
        audio = await call("get_dictation_audio_usage");
      }),
    ]);
  }
  onMount(() => {
    if (app.native) void refresh();
  });
  async function removeAll() {
    const total = await app.perform("history-count", () =>
      call("count_dictations"),
    );
    if (total === undefined) return;
    count = total;
    if (!total) return;
    if (
      !(await app.confirm(
        `Delete all ${total} dictations?`,
        `This removes all saved originals, finished text and kept audio. Derived learning without another source is removed. Words you added and meetings stay.`,
        "Delete all",
        true,
      ))
    )
      return;
    const result = await app.perform("history-delete-all", () =>
      call("delete_all_dictations", { confirmed: true }),
    );
    if (result) {
      app.apply(result.snapshot);
      app.notice = `${result.deletedCount} dictations deleted.`;
      await refresh();
    }
  }
  async function removeAudio() {
    if (
      !(await app.confirm(
        "Delete all kept dictation audio?",
        "Saved text, words and meeting recordings stay. This cannot be undone.",
        "Delete audio",
        true,
      ))
    )
      return;
    const result = await app.perform("dictation-audio-delete", () =>
      call("delete_dictation_audio"),
    );
    if (result) {
      app.notice = `${result.deleted} recordings deleted.`;
      await refresh();
    }
  }
</script>

<section
  class="stock paper-card ui-card row-card"
  aria-label="Paste and history settings"
>
  <SettingRow
    label="Paste where I was typing"
    help="Off: text waits on the clipboard"
    layout="action"
  >
    <Toggle
      label="Paste where I was typing"
      checked={settings.autoPaste}
      disabled={!app.native ||
        !app.snapshot.capabilities.autoPaste ||
        app.busy("settings")}
      onchange={(autoPaste) => void app.saveSettings({ autoPaste })}
    />
  </SettingRow>
  {#if !app.snapshot.capabilities.autoPaste}<p class="field-help row-note">
      Automatic paste is unavailable in this environment.
    </p>{/if}
  {#if app.native && app.snapshot.capabilities.platform === "macos"}<MacPasteAccess
      variant="settings"
    />{/if}
  <SettingRow label="Ring at the text cursor" layout="action">
    <Toggle
      label="Ring at the text cursor"
      checked={settings.cursorIndicator}
      disabled={!app.native || app.busy("settings")}
      onchange={(cursorIndicator) =>
        void app.saveSettings({ cursorIndicator })}
    />
  </SettingRow>
  <SettingRow
    label="Keep"
    help={`${count === null ? "Count unavailable" : `${count} kept`} · audio 30 days`}
    layout="choice"
  >
    <SegmentedControl
      label="Keep dictations"
      options={retentionOptions}
      value={retention}
      disabled={locked}
      onchange={(option) =>
        void app.saveSettings({
          doNotSaveDictations: option === "nothing",
          keepDictationAudio: option === "audio",
        })}
    />
  </SettingRow>
  {#if settings.doNotSaveDictations}<p class="field-help row-note">
      New dictations are not saved. Existing text stays until you delete it.
      Copy last remains available until the next dictation or app exit.
    </p>{/if}
  {#if active}<p class="field-help row-note">
      Finish the current dictation to change retention.
    </p>{/if}
  <SettingRow label="Delete history or audio" layout="action">
    <button
      class="button small"
      disabled={locked}
      onclick={() => {
        deleting = true;
        void refresh();
      }}>Delete…</button
    >
  </SettingRow>
</section>
{#if deleting}<Sheet
    title="Delete"
    onclose={() => {
      if (
        !app.busy("history-delete-all") &&
        !app.busy("dictation-audio-delete")
      )
        deleting = false;
    }}
  >
    <SettingRow
      label="All dictations"
      help={`${count ?? "—"} · words you added stay`}
      layout="action"
    >
      <button
        class="button danger small"
        disabled={locked || !count || app.busy("history-count")}
        onclick={() => void removeAll()}>Delete dictations</button
      >
    </SettingRow>
    <SettingRow
      label="All kept audio"
      help={audio
        ? `${audio.count} recordings · ${formatBytes(audio.bytes)}`
        : "Usage unavailable"}
      layout="action"
    >
      <button
        class="button danger small"
        disabled={locked || !audio?.count}
        onclick={() => void removeAudio()}>Delete audio</button
      >
    </SettingRow>
    {#if app.error}<p role="alert">{app.error}</p>{/if}
  </Sheet>{/if}

<style>
  .row-note {
    margin: -6px 0 14px;
  }
</style>
