<script lang="ts">
  import { tick } from "svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import Picker from "../components/quiet/Picker.svelte";
  import Disclosure from "../components/quiet/Disclosure.svelte";
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
  let {
    onreview,
    item,
  }: { onreview?: (id: number) => void; item?: string } = $props();
  let capturing = $state(false);
  let keyButton: HTMLButtonElement;
  let root: HTMLElement;
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
  const microphones = $derived.by(() => {
    const seen = new Set<string>([""]);
    const list = [
      {
        value: "",
        label: `System default${defaultDevice ? ` · ${defaultDevice.name}` : " · unavailable"}`,
      },
    ];
    for (const device of app.snapshot.devices) {
      if (seen.has(device.id)) continue;
      seen.add(device.id);
      list.push({ value: device.id, label: device.name });
    }
    if (settings.microphone && !seen.has(settings.microphone)) {
      list.push({
        value: settings.microphone,
        label: `${settings.microphone} · unavailable`,
      });
    }
    return list;
  });
  $effect(() => {
    const target = item;
    if (target !== "microphone" && target !== "shortcuts") return;
    void tick().then(() => {
      if (target === "shortcuts") {
        keyButton?.focus();
      } else {
        root
          ?.querySelector<HTMLButtonElement>('button[aria-label="Microphone"]')
          ?.focus();
      }
    });
  });
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
  bind:this={root}
  class="stock paper-card ui-card row-card"
  aria-label="Shortcut and mic settings"
>
  <SettingRow label="Keys" layout="choice">
    <button
      bind:this={keyButton}
      class="key-well"
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
  </SettingRow>
  <SettingRow label="Stops when" layout="choice">
    <Picker
      label="Stops when"
      value={settings.autoStopSeconds
        ? String(settings.autoStopSeconds)
        : settings.hotkeyMode}
      options={stops}
      disabled={locked}
      onchange={stopChoice}
    />
  </SettingRow>
  <SettingRow label="Microphone" layout="choice">
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
  </SettingRow>
  <SettingRow label="Sound cues" help="At start and stop" layout="action">
    <Toggle
      label="Sound cues"
      checked={settings.audioCues}
      disabled={!app.native || app.busy("settings")}
      onchange={(audioCues) => void app.saveSettings({ audioCues })}
    />
  </SettingRow>
  <Disclosure title="Test microphone">
    <div class="mic-test-body">
      <DictationControl microphoneTest onaction={testAction} />
      <p class="field-help">
        Speak to check the level. The test uses your dictation settings,
        including automatic paste and history.
      </p>
      <div class="actions">
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
      </div>
    </div>
  </Disclosure>
</section>

<style>
  .input-meter {
    width: 90px;
  }
  .mic-test-body {
    display: grid;
    gap: 12px;
    padding-bottom: 12px;
  }
  .mic-test-body p {
    margin: 0;
  }
</style>
