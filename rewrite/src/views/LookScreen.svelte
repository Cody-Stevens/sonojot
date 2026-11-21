<script lang="ts">
  import Toggle from "../components/Toggle.svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import SegmentedControl from "../components/quiet/SegmentedControl.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  const settings = $derived(app.snapshot.settings);
  const menuBar = $derived(app.snapshot.capabilities.platform === "macos");
  const locked = $derived(!app.native || app.busy("settings"));
  const themeOptions = [
    { value: "fog" as const, label: "Fog" },
    { value: "slate" as const, label: "Slate" },
  ];
</script>

<section class="stock paper-card ui-card row-card quiet-settings">
  <SettingRow label="Theme" layout="choice">
    <SegmentedControl
      label="Theme"
      options={themeOptions}
      value={settings.theme === "slate" ? "slate" : "fog"}
      disabled={locked}
      onchange={(theme) => app.saveSettings({ theme })}
    />
  </SettingRow>
  <SettingRow label="Paper and print" layout="action">
    <Toggle
      label="Paper and print"
      checked={settings.material}
      disabled={locked}
      onchange={(material) => app.saveSettings({ material })}
    />
  </SettingRow>
  {#if !menuBar}<SettingRow
      label="Floating control at start"
      help="The tab at the top of the screen"
      layout="action"
      ><Toggle
        label="Floating control at start"
        checked={!settings.startTabHidden}
        disabled={locked}
        onchange={(visible) => app.saveSettings({ startTabHidden: !visible })}
      /></SettingRow
    >{/if}
  <SettingRow label="Start at login" layout="action">
    <Toggle
      label="Start at login"
      checked={settings.launchAtLogin}
      disabled={locked}
      onchange={(launchAtLogin) => app.saveSettings({ launchAtLogin })}
    />
  </SettingRow>
  <SettingRow
    label={menuBar ? "Menu-bar panel" : "Floating control now"}
    help="Hiding it leaves a recording running."
    layout="action"
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
        )}>{app.snapshot.tabVisible ? "Hide" : "Show"}</button
    >
  </SettingRow>
</section>
