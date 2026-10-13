<script lang="ts">
  import Card from "../components/Card.svelte";
  import Toggle from "../components/Toggle.svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  const menuBar = $derived(app.snapshot.capabilities.platform === "macos");
</script>

<header class="page-heading">
  <h1 class="text-grain" data-text-surface="emboss">Appearance</h1>
</header>
<Card title="Paper">
  <div class="theme-options" role="group" aria-label="Color theme">
    {#each [{ value: "fog", name: "Fog paper", help: "Warm white with sage ink" }, { value: "slate", name: "Slate paper", help: "Charcoal green with pale ink" }] as theme}
      <button
        class="theme-choice"
        class:selected={app.snapshot.settings.theme === theme.value}
        aria-pressed={app.snapshot.settings.theme === theme.value}
        disabled={app.busy("settings")}
        onclick={() =>
          app.saveSettings({ theme: theme.value === "fog" ? "fog" : "slate" })}
      >
        <span class="theme-swatch" data-swatch={theme.value}
          ><span class="swatch-sheet"
            ><span class="swatch-ink">Aa</span><span class="swatch-lines"
            ></span></span
          ></span
        ><strong>{theme.name}</strong><small data-text-surface="flat"
          >{theme.help}</small
        >
      </button>
    {/each}
  </div>
  <SettingRow
    label="Paper and print"
    help="Fine paper tooth, shallow edges and a printed heading."
    ><Toggle
      label="Paper and print"
      checked={app.snapshot.settings.material}
      disabled={app.busy("settings")}
      onchange={(material) => app.saveSettings({ material })}
    /></SettingRow
  >
  <details class="license spaced">
    <summary data-text-surface="emboss">Screen tab and cursor</summary>
    <SettingRow
      label={menuBar ? "Menu bar" : "Screen tab"}
      help={menuBar
        ? "Hiding closes the panel and leaves the recording running. A new recording opens it."
        : "Hiding leaves the current recording running. New recordings show the tab again."}
    >
      <button
        class="button small"
        disabled={!app.native || app.busy("tab-visibility")}
        onclick={() =>
          app.perform("tab-visibility", async () =>
            app.apply(
              await call("set_screen_tab", {
                visible: !app.snapshot.tabVisible,
                expanded: false,
                pinned: true,
              }),
            ),
          )}
        >{menuBar
          ? app.snapshot.tabVisible
            ? "Hide panel"
            : "Show panel"
          : app.snapshot.tabVisible
            ? "Hide tab"
            : "Show tab"}</button
      >
    </SettingRow>
    {#if !menuBar}<SettingRow
        label="Hide the tab when SonoJot starts"
        help="Hide the idle tab at launch. Recording still reveals it."
      >
        <Toggle
          label="Hide the tab when SonoJot starts"
          checked={app.snapshot.settings.startTabHidden}
          disabled={!app.native || app.busy("settings")}
          onchange={(startTabHidden) => app.saveSettings({ startTabHidden })}
        />
      </SettingRow>{/if}
    <SettingRow
      label="Cursor ring while dictating"
      help="Show a ring by the cursor while dictating."
    >
      <Toggle
        label="Cursor ring while dictating"
        checked={app.snapshot.settings.cursorIndicator}
        disabled={!app.native || app.busy("settings")}
        onchange={(cursorIndicator) => app.saveSettings({ cursorIndicator })}
      />
    </SettingRow>
  </details>
</Card>
<Card title="Preview"
  ><div class="material-sample paper-well">
    <h3 class="text-grain" data-text-surface="emboss">Paper and print</h3>
    <p>Headings, body text and controls.</p>
    <kbd>{app.snapshot.settings.hotkey.split("+").join(" + ")}</kbd>
  </div>
  <p class="field-help" data-text-surface="flat">
    Turning off paper and print removes texture, lighting and relief. Your
    system’s reduced-motion and contrast preferences are respected.
  </p></Card
>
