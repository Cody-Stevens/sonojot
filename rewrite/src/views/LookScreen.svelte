<script lang="ts">
  import Toggle from "../components/Toggle.svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  const settings = $derived(app.snapshot.settings);
  const menuBar = $derived(app.snapshot.capabilities.platform === "macos");
  const locked = $derived(!app.native || app.busy("settings"));
</script>

<section class="stock paper-card quiet-settings">
  <SettingRow label="Theme"
    ><div class="segments" aria-label="Theme">
      {#each ["fog", "slate"] as theme}<button
          aria-pressed={settings.theme === theme}
          disabled={locked}
          onclick={() =>
            app.saveSettings({ theme: theme === "slate" ? "slate" : "fog" })}
          >{theme === "fog" ? "Fog" : "Slate"}</button
        >{/each}
    </div></SettingRow
  >
  <SettingRow label="Paper and print"
    ><Toggle
      label="Paper and print"
      checked={settings.material}
      disabled={locked}
      onchange={(material) => app.saveSettings({ material })}
    /></SettingRow
  >
  {#if !menuBar}<SettingRow
      label="Floating control at start"
      help="The tab at the top of the screen"
      ><Toggle
        label="Floating control at start"
        checked={!settings.startTabHidden}
        disabled={locked}
        onchange={(visible) => app.saveSettings({ startTabHidden: !visible })}
      /></SettingRow
    >{/if}
  <SettingRow label="Start at login"
    ><Toggle
      label="Start at login"
      checked={settings.launchAtLogin}
      disabled={locked}
      onchange={(launchAtLogin) => app.saveSettings({ launchAtLogin })}
    /></SettingRow
  >
  <SettingRow
    label={menuBar ? "Menu-bar panel" : "Floating control now"}
    help="Hiding it leaves a recording running."
    ><button
      class="button"
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
        )}>{app.snapshot.tabVisible ? "Hide" : "Show"}</button
    ></SettingRow
  >
</section>
