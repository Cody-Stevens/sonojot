<script lang="ts">
  import { tick } from "svelte";
  import Picker from "../components/quiet/Picker.svelte";
  import Meter from "../components/Meter.svelte";
  import Toggle from "../components/Toggle.svelte";
  import DictationControl from "../components/DictationControl.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import { presentError } from "../lib/errors";
  import { safeShortcut, shortcutHelp } from "../lib/shortcut";
  import {
    completedDictationSession,
    type DictationAction,
  } from "../lib/dictation-control";
  let { onreview }: { onreview?: (id: number) => void } = $props();
  let capturing = $state(false);
  let keyButton: HTMLButtonElement;
  let test = $state<{ previousResult: number | null } | null>(null);
  const settings = $derived(app.snapshot.settings);
  const active = $derived(
    ["listening", "loading", "finishing", "cleaning"].includes(
      app.snapshot.state,
    ),
  );
  const locked = $derived(
    !app.native ||
      active ||
      app.snapshot.meetingRecording ||
      app.busy("settings"),
  );
  const testResult = $derived(
    test ? completedDictationSession(app.snapshot, test.previousResult) : null,
  );
  const conflict = $derived(
    !!(app.error || app.snapshot.error) &&
      presentError(app.error || app.snapshot.error).family === "shortcut",
  );
  const defaultDevice = $derived(
    app.snapshot.devices.find((device) => device.isDefault),
  );
  const microphones = $derived([
    {
      value: "",
      label: `System default${defaultDevice ? ` · ${defaultDevice.name}` : " · unavailable"}`,
    },
    ...app.snapshot.devices.map((device) => ({
      value: device.id,
      label: device.name,
    })),
    ...(settings.microphone &&
    !app.snapshot.devices.some((device) => device.id === settings.microphone)
      ? [
          {
            value: settings.microphone,
            label: `${settings.microphone} · unavailable`,
          },
        ]
      : []),
  ]);
  const stops = $derived([
    { value: "toggle", label: "I press the keys again" },
    { value: "hold", label: "I let go (hold to talk)" },
    ...[
      ...new Set([
        3,
        5,
        10,
        20,
        ...(settings.autoStopSeconds ? [settings.autoStopSeconds] : []),
      ]),
    ]
      .sort((a, b) => a - b)
      .map((seconds) => ({
        value: String(seconds),
        label: `I pause for ${seconds} s`,
      })),
  ]);
  async function listen() {
    capturing = true;
    await tick();
    keyButton.focus();
  }
  function capture(event: KeyboardEvent) {
    if (!capturing) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.key === "Escape") {
      capturing = false;
      return;
    }
    if (["Control", "Alt", "Shift", "Meta"].includes(event.key) || event.repeat)
      return;
    const modifiers = [
      event.ctrlKey ? "Ctrl" : "",
      event.altKey ? "Alt" : "",
      event.shiftKey ? "Shift" : "",
      event.metaKey ? "Super" : "",
    ].filter(Boolean);
    const key =
      event.code === "Space"
        ? "Space"
        : /^Key[A-Z]$/.test(event.code)
          ? event.code.slice(3)
          : /^Digit\d$/.test(event.code)
            ? event.code.slice(5)
            : event.key;
    const hotkey = [...modifiers, key].join("+");
    if (!safeShortcut(hotkey)) return;
    capturing = false;
    void app.saveSettings({ hotkey });
  }
  function stopChoice(value: string) {
    void app.saveSettings(
      value === "toggle" || value === "hold"
        ? { hotkeyMode: value, autoStopSeconds: 0 }
        : { hotkeyMode: "toggle", autoStopSeconds: Number(value) },
    );
  }
  function testAction(action: DictationAction) {
    if (action === "start")
      test = { previousResult: app.snapshot.resultSessionId };
    else if (action === "cancel") test = null;
  }
</script>

{#if !app.snapshot.capabilities.globalShortcut}
  <div class="notice">
    <span
      >This desktop has no global shortcuts. Bind <code
        >sonojot --shortcut press</code
      > to a key.</span
    ><button
      class="button small"
      disabled={!app.native || app.busy("copy-shortcut")}
      onclick={() =>
        void app.perform(
          "copy-shortcut",
          () => call("copy_shortcut_command"),
          "Shortcut command copied.",
        )}>Copy</button
    >
  </div>
{/if}
{#if conflict}<p class="notice" role="status">
    <span class="quiet-tag amber">TAKEN</span>The shortcut could not be
    registered. Choose a different combination.
  </p>{/if}
<section
  class="stock paper-card quiet-panel"
  aria-label="Shortcut and mic settings"
>
  <div class="stage-setting">
    <span>Keys</span>
    <div class="stage-control">
      <button
        bind:this={keyButton}
        class="stock key-well"
        class:capturing
        disabled={locked}
        aria-label="Change shortcut"
        onkeydown={capture}
        onblur={() => (capturing = false)}
        onclick={() => void listen()}
        >{#if capturing}Press new keys…{:else}{#each settings.hotkey.split("+") as key}<kbd
              >{key}</kbd
            >{/each}{/if}</button
      >{#if capturing}<span class="field-help">{shortcutHelp}</span>{/if}
    </div>
  </div>
  <div class="stage-setting">
    <span>Stops when</span>
    <div class="stage-control">
      <Picker
        label="Stops when"
        value={settings.autoStopSeconds
          ? String(settings.autoStopSeconds)
          : settings.hotkeyMode}
        options={stops}
        disabled={locked}
        onchange={stopChoice}
      />
    </div>
  </div>
  <div class="stage-setting">
    <span>Microphone</span>
    <div class="stage-control">
      <Picker
        label="Microphone"
        value={settings.microphone ?? ""}
        options={microphones}
        disabled={locked}
        onchange={(microphone) =>
          void app.saveSettings({ microphone: microphone || null })}
      />
      <div class="input-meter">
        <Meter
          level={app.snapshot.audioLevel}
          active={["listening", "loading"].includes(app.snapshot.state) ||
            app.snapshot.meetingRecording}
        />
      </div>
    </div>
  </div>
  <div class="stage-setting wide-label">
    <span
      >Sound cues <small data-text-surface="flat">at start and stop</small
      ></span
    >
    <div class="stage-control end">
      <Toggle
        label="Sound cues"
        checked={settings.audioCues}
        disabled={!app.native || app.busy("settings")}
        onchange={(audioCues) => void app.saveSettings({ audioCues })}
      />
    </div>
  </div>
</section>
<details class="license mic-test">
  <summary>Test microphone</summary>
  <div class="spaced">
    <DictationControl microphoneTest onaction={testAction} />
  </div>
  <p class="field-help">
    Speak to check the level. The test uses your dictation settings, including
    automatic paste and history.
  </p>
  {#if testResult !== null && onreview}<button
      class="button small"
      onclick={() => {
        if (testResult !== null) onreview?.(testResult);
      }}>Review test result</button
    >{/if}
  <button
    class="button small"
    disabled={locked || app.busy("devices")}
    onclick={() =>
      void app.perform("devices", async () =>
        app.apply(await call("get_snapshot")),
      )}>Refresh device list</button
  >
</details>

<style>
  .key-well {
    min-width: 230px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    border: 1px solid var(--line);
    border-radius: 9px;
    box-shadow: var(--press);
    background-color: var(--well);
    color: var(--ink);
    padding: 8px 13px;
    min-height: 44px;
  }
  .key-well.capturing {
    outline: 2px solid var(--dye);
    outline-offset: 3px;
  }
  .input-meter {
    width: 90px;
  }
  .mic-test {
    margin-top: 22px;
  }
</style>
