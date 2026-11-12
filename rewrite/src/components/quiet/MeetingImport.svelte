<script lang="ts">
  import { onMount } from "svelte";
  import Sheet from "./Sheet.svelte";
  import { app } from "../../lib/app.svelte";
  import { call, chooseMedia, subscribeMediaSupport } from "../../lib/api";
  import { ensureDownloadConsent } from "../../lib/download-consent.svelte";
  import type { MeetingSession } from "../../lib/meeting-session.svelte";
  import type { SetupStatus } from "../../lib/setup";
  let {
    session,
    path = $bindable(""),
    onclose,
  }: { session: MeetingSession; path?: string; onclose: () => void } = $props();
  let speaker = $state("");
  let status = $state<SetupStatus | null>(null);
  let disposed = false;
  const locked = $derived(
    app.busy("meeting-import") ||
      app.busy("media-install") ||
      session.dictating ||
      app.snapshot.meetingRecording,
  );
  async function refresh() {
    const next = await app.perform("media-check", () => call("get_setup"));
    if (!disposed && next) status = next;
  }
  onMount(() => {
    void refresh();
    let stop: (() => void) | undefined;
    void subscribeMediaSupport(() => void refresh()).then((next) => {
      if (disposed) next();
      else stop = next;
    });
    return () => {
      disposed = true;
      stop?.();
    };
  });
  async function choose() {
    const result = await app.perform("media-choose", chooseMedia);
    if (result) path = result;
  }
  async function install() {
    if (!(await ensureDownloadConsent(["ffmpeg"])) || locked) return;
    const next = await app.perform("media-install", () =>
      call("install_media_support"),
    );
    if (!disposed && next) status = next;
  }
</script>

<Sheet
  title="Import a recording"
  onclose={() => {
    if (!locked) onclose();
  }}
>
  <div class="import-well">
    <p>
      {path ? path.split(/[\\/]/).at(-1) : "Drop audio or video into Meetings"}
    </p>
    <button class="button" disabled={locked} onclick={choose}
      >{path ? "Choose another file" : "Choose file"}</button
    >
  </div>
  <label class="field-label spaced" for="import-speaker"
    >One speaker? <small>Their name (optional)</small></label
  ><input
    id="import-speaker"
    bind:value={speaker}
    maxlength="48"
    disabled={locked}
  />
  <div class="media-support">
    <span>Media support <small>For video and most audio</small></span
    >{#if status?.mediaImport.state === "installed" || status?.mediaImport.state === "ready"}<span
        class="quiet-tag">INSTALLED</span
      >{:else}<button
        class="button"
        disabled={locked ||
          !status ||
          status.mediaImport.state === "unsupported"}
        onclick={install}
        >{app.busy("media-install") ? "Installing…" : "Install"}</button
      >{/if}
  </div>
  {#if status?.mediaImport.detail}<p class="field-help">
      {status.mediaImport.detail}
    </p>{/if}
  <div class="actions">
    <button
      class="button primary"
      disabled={locked || !path}
      onclick={async () => {
        if (await session.importFile(path, speaker)) {
          path = "";
          speaker = "";
          onclose();
        }
      }}>{app.busy("meeting-import") ? "Importing…" : "Import"}</button
    >
  </div>
  {#if app.error}<p role="alert" class="field-help">{app.error}</p>{/if}
</Sheet>

<style>
  .import-well {
    padding: 24px;
    text-align: center;
    box-shadow: var(--deboss);
    background: var(--well);
    border-radius: 12px;
  }
  .import-well p {
    overflow-wrap: anywhere;
    margin-top: 0;
  }
  .media-support {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin: 24px 0;
  }
  .media-support small {
    display: block;
    color: var(--soft-ink);
    font-size: 13px;
  }
  input {
    width: 100%;
  }
  .actions {
    justify-content: flex-end;
  }
</style>
