<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import Card from "../components/Card.svelte";
  import Toggle from "../components/Toggle.svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import { app } from "../lib/app.svelte";
  let { embedded = false }: { embedded?: boolean } = $props();
  let hotkey = $state(app.snapshot.settings.hotkey);
  let previousHotkey = app.snapshot.settings.hotkey;
  let capturing = $state(false);
  $effect(() => {
    const current = app.snapshot.settings.hotkey;
    untrack(() => {
      if (hotkey === previousHotkey) hotkey = current;
      previousHotkey = current;
    });
  });
  $effect(() => {
    app.dirty = hotkey !== app.snapshot.settings.hotkey;
  });
  onDestroy(() => {
    app.dirty = false;
  });
  function capture(event: KeyboardEvent): void {
    if (!capturing) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.key === "Escape") {
      capturing = false;
      return;
    }
    if (["Control", "Alt", "Shift", "Meta"].includes(event.key)) return;
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
    hotkey = [...modifiers, key].join("+");
    capturing = false;
  }
</script>

{#if !embedded}<header class="page-heading">
    <h1 class="text-grain" data-text-surface="emboss">Shortcuts</h1>
  </header>{/if}
<!-- In setup the step heading names the shortcut. -->
<Card title={embedded ? undefined : "Your dictation shortcut"}>
  <p class="body-copy" data-text-surface="flat">
    Use this shortcut from the app you are writing in.
  </p>
  <div class="shortcut-editor">
    <label class="field-label" for="hotkey">Keyboard shortcut</label>
    <div class="input-action">
      <input
        id="hotkey"
        bind:value={hotkey}
        onkeydown={capture}
        onblur={() => {
          capturing = false;
        }}
        readonly={capturing}
        aria-describedby="shortcut-help"
        disabled={!app.native || app.busy("settings")}
        placeholder="Ctrl+Space"
      /><button
        class="button"
        disabled={!app.native || app.busy("settings")}
        onclick={() => {
          capturing = true;
          document.getElementById("hotkey")?.focus();
        }}>{capturing ? "Press keys…" : "Record keys"}</button
      >
      <button
        class="button primary"
        disabled={!app.native ||
          app.busy("settings") ||
          !hotkey.trim() ||
          hotkey === app.snapshot.settings.hotkey}
        onclick={() => app.saveSettings({ hotkey: hotkey.trim() })}
        >{app.busy("settings") ? "Saving…" : "Save"}</button
      >
    </div>
    <p id="shortcut-help" class="field-help" data-text-surface="flat">
      {capturing
        ? "Press the combination you want. Escape cancels."
        : "For example, Ctrl+Space or Alt+Shift+D. You can also type a combination."}
    </p>
  </div>
  {#if app.snapshot.capabilities.platform === "linux"}<p
      class="field-help"
      data-text-surface="flat"
    >
      {app.snapshot.capabilities.globalShortcut
        ? "Hyprland runs this shortcut. SonoJot keeps it in ~/.config/hypr/sonojot.lua."
        : "This desktop has no global shortcuts for apps. Bind a key to `SonoJot --shortcut press` in your compositor, or use the dictation button."}
    </p>{:else if !app.snapshot.capabilities.globalShortcut}<p
      class="field-help"
      data-text-surface="flat"
    >
      Global shortcuts are unavailable in this environment. Use the dictation
      button.
    </p>{/if}
  <SettingRow
    label="How the shortcut works"
    help={app.snapshot.settings.hotkeyMode === "hold"
      ? "Speak while holding the keys. Release to finish."
      : "Press once to begin, again to finish."}
    ><select
      aria-label="How the shortcut works"
      value={app.snapshot.settings.hotkeyMode}
      disabled={!app.native || app.busy("settings")}
      onchange={(event) =>
        app.saveSettings({
          hotkeyMode: event.currentTarget.value === "hold" ? "hold" : "toggle",
        })}
      ><option value="toggle">Press to start and stop</option><option
        value="hold">Hold to speak</option
      ></select
    ></SettingRow
  >
</Card>
{#if !embedded}<Card
    ><SettingRow
      label="Start SonoJot when I sign in"
      help={app.snapshot.capabilities.platform === "macos"
        ? "Keep SonoJot available from the menu bar."
        : "Keep SonoJot available from the system tray."}
      ><Toggle
        label="Start SonoJot when I sign in"
        checked={app.snapshot.settings.launchAtLogin}
        disabled={!app.native || app.busy("settings")}
        onchange={(launchAtLogin) => app.saveSettings({ launchAtLogin })}
      /></SettingRow
    ></Card
  >{/if}
