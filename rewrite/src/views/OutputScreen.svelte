<script lang="ts">
  import { onMount } from "svelte";
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
  const retention = $derived(
    settings.doNotSaveDictations
      ? "nothing"
      : settings.keepDictationAudio
        ? "audio"
        : "text",
  );
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
  class="stock paper-card quiet-panel"
  aria-label="Paste and history settings"
>
  <div class="stage-setting wide-label">
    <span
      >Paste where I was typing <small data-text-surface="flat"
        >off: text waits on the clipboard</small
      ></span
    >
    <div class="stage-control end">
      <Toggle
        label="Paste where I was typing"
        checked={settings.autoPaste}
        disabled={!app.native ||
          !app.snapshot.capabilities.autoPaste ||
          app.busy("settings")}
        onchange={(autoPaste) => void app.saveSettings({ autoPaste })}
      />
    </div>
  </div>
  {#if !app.snapshot.capabilities.autoPaste}<p class="field-help">
      Automatic paste is unavailable in this environment.
    </p>{/if}
  {#if app.native && app.snapshot.capabilities.platform === "macos"}<MacPasteAccess
      variant="settings"
    />{/if}
  <div class="stage-setting wide-label">
    <span>Ring at the text cursor</span>
    <div class="stage-control end">
      <Toggle
        label="Ring at the text cursor"
        checked={settings.cursorIndicator}
        disabled={!app.native || app.busy("settings")}
        onchange={(cursorIndicator) =>
          void app.saveSettings({ cursorIndicator })}
      />
    </div>
  </div>
  <div class="stage-setting">
    <span>Keep</span>
    <div class="stage-control">
      <div class="segments" role="group" aria-label="Keep dictations">
        {#each [{ value: "nothing", label: "Nothing" }, { value: "text", label: "Text" }, { value: "audio", label: "Text and audio" }] as option}<button
            disabled={locked}
            aria-pressed={retention === option.value}
            onclick={() =>
              void app.saveSettings({
                doNotSaveDictations: option.value === "nothing",
                keepDictationAudio: option.value === "audio",
              })}>{option.label}</button
          >{/each}
      </div>
      <small data-text-surface="flat"
        >{count === null ? "Count unavailable" : `${count} kept`} · audio 30 days</small
      >
    </div>
  </div>
  {#if settings.doNotSaveDictations}<p class="field-help">
      New dictations are not saved. Existing text stays until you delete it.
      Copy last remains available until the next dictation or app exit.
    </p>{/if}
  {#if active}<p class="field-help">
      Finish the current dictation to change retention.
    </p>{/if}
  <div class="stage-setting wide-label">
    <span>Delete history or audio</span>
    <div class="stage-control end">
      <button
        class="button small"
        disabled={locked}
        onclick={() => {
          deleting = true;
          void refresh();
        }}>Delete…</button
      >
    </div>
  </div>
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
    <div class="stage-setting wide-label">
      <span
        >All dictations <small>{count ?? "—"} · words you added stay</small
        ></span
      ><button
        class="button danger small"
        disabled={locked || !count || app.busy("history-count")}
        onclick={() => void removeAll()}>Delete dictations</button
      >
    </div>
    <div class="stage-setting wide-label">
      <span
        >All kept audio <small
          >{audio
            ? `${audio.count} recordings · ${formatBytes(audio.bytes)}`
            : "Usage unavailable"}</small
        ></span
      ><button
        class="button danger small"
        disabled={locked || !audio?.count}
        onclick={() => void removeAudio()}>Delete audio</button
      >
    </div>
    {#if app.error}<p role="alert">{app.error}</p>{/if}
  </Sheet>{/if}
