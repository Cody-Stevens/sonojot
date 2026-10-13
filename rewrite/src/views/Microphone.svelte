<script lang="ts">
  import Card from "../components/Card.svelte";
  import Meter from "../components/Meter.svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import DictationControl from "../components/DictationControl.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import {
    completedDictationSession,
    type DictationAction,
  } from "../lib/dictation-control";
  let { onreview }: { onreview?: (resultSessionId: number) => void } = $props();
  let test = $state<{ previousResult: number | null } | null>(null);
  const testResult = $derived(
    test === null
      ? null
      : completedDictationSession(app.snapshot, test.previousResult),
  );
  function testAction(action: DictationAction): void {
    if (action === "start")
      test = { previousResult: app.snapshot.resultSessionId };
    else if (action === "cancel") test = null;
  }
  const active = $derived(
    ["listening", "loading", "finishing", "cleaning"].includes(
      app.snapshot.state,
    ),
  );
</script>

<header class="page-heading">
  <h1 class="text-grain" data-text-surface="emboss">Microphone</h1>
</header>
<Card title="Your microphone">
  <label class="field-label" for="microphone">Input device</label>
  <select
    id="microphone"
    value={app.snapshot.settings.microphone ?? ""}
    disabled={!app.native ||
      active ||
      app.snapshot.meetingRecording ||
      app.busy("settings")}
    onchange={(event) =>
      app.saveSettings({ microphone: event.currentTarget.value || null })}
  >
    <option value="">System default microphone</option>
    {#each app.snapshot.devices as device}<option value={device.id}
        >{device.name}{device.isDefault ? " (default)" : ""}</option
      >{/each}
  </select>
  <p class="field-help" data-text-surface="flat">
    Choose the device you speak into.
  </p>
  <div class="spaced">
    <DictationControl microphoneTest onaction={testAction} />
  </div>
  <p class="field-help" data-text-surface="flat">
    Check the level while speaking. Your test appears in Dictation, under
    Recent. Automatic paste follows your dictation setting.
  </p>
  <div class="paper-well microphone-well">
    <div class="section-toolbar">
      <span>Input level</span><span class="field-help" data-text-surface="flat"
        >{app.snapshot.state === "loading"
          ? "Recording · loading model"
          : app.snapshot.state === "listening"
            ? "Listening"
            : app.snapshot.meetingRecording
              ? "Meeting recording"
              : "Microphone inactive"}</span
      >
    </div>
    <Meter
      level={app.snapshot.audioLevel}
      active={["loading", "listening"].includes(app.snapshot.state) ||
        app.snapshot.meetingRecording}
    />
    <div class="meter-scale" data-text-surface="flat">
      <span>Quiet</span><span>Loud</span>
    </div>
  </div>
  {#if testResult !== null && onreview}
    <button
      class="text-action spaced"
      onclick={() => {
        if (testResult !== null) onreview?.(testResult);
      }}>Review test result</button
    >
  {/if}
  <details class="license spaced">
    <summary data-text-surface="emboss">More microphone options</summary>
    <SettingRow
      label="Stop after silence"
      help="Choose how long to wait after you stop speaking."
      ><select
        aria-label="Stop dictation after silence"
        value={String(app.snapshot.settings.autoStopSeconds)}
        disabled={!app.native || app.busy("settings")}
        onchange={(event) =>
          app.saveSettings({
            autoStopSeconds: Number(event.currentTarget.value),
          })}
        ><option value="0">Only when I finish</option><option value="3"
          >3 seconds</option
        ><option value="5">5 seconds</option><option value="10"
          >10 seconds</option
        ><option value="20">20 seconds</option
        >{#if ![0, 3, 5, 10, 20].includes(app.snapshot.settings.autoStopSeconds)}<option
            value={String(app.snapshot.settings.autoStopSeconds)}
            >{app.snapshot.settings.autoStopSeconds} seconds</option
          >{/if}</select
      ></SettingRow
    >
    <div class="actions spaced">
      <button
        class="button quiet small"
        disabled={!app.native || app.busy("devices") || active}
        onclick={() =>
          app.perform("devices", async () =>
            app.apply(await call("get_snapshot")),
          )}>Refresh device list</button
      >
    </div>
  </details>
</Card>
