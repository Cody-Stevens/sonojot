<script lang="ts">
  import { onMount } from "svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import LegalLinks from "../components/LegalLinks.svelte";
  import ThirdPartyNotices from "../components/ThirdPartyNotices.svelte";
  import TermsText from "../components/TermsText.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import apacheLicense from "../../../LICENSE?raw";
  let { onreport, ontour }: { onreport: () => void; ontour: () => void } =
    $props();
  let cliPath = $state("");
  let diagnostics = $state("");
  const locked = $derived(
    !app.native ||
      app.busy("settings") ||
      !["idle", "error"].includes(app.snapshot.state) ||
      app.snapshot.meetingRecording,
  );
  onMount(() => {
    void app.perform("cli-path", async () => {
      cliPath = await call("get_cli_path");
    });
  });
  async function runAgain() {
    if (locked) return;
    await app.saveSettings({ meetingConsentAcknowledged: false });
    if (!app.error && !app.snapshot.settings.meetingConsentAcknowledged)
      ontour();
  }
</script>

<section class="stock paper-card quiet-settings">
  <button class="settings-link" onclick={onreport}
    ><strong data-text-surface="emboss">Report a problem</strong><span
      >See what is sent first</span
    ><span>›</span></button
  >
  <SettingRow label="Command line" help="sonojot-cli · commands for scripts"
    ><code class="cli-path">{cliPath || "Reading installed path…"}</code><button
      class="button"
      disabled={!cliPath || app.busy("copy-cli")}
      onclick={() =>
        app.perform(
          "copy-cli",
          () => call("copy_cli_path"),
          "CLI path copied.",
        )}>Copy path</button
    ></SettingRow
  >
  <SettingRow label="First-run setup" help="And the meeting consent reminder"
    ><button class="button" disabled={locked} onclick={runAgain}
      >Run again</button
    ></SettingRow
  >
</section>
<details class="stock paper-card quiet-settings">
  <summary data-text-surface="emboss">Troubleshooting and legal</summary>
  <div class="actions spaced">
    {#each ["data", "models", "logs"] as kind}<button
        class="button"
        onclick={() =>
          app.perform("folder", () =>
            call("open_path", {
              kind:
                kind === "data"
                  ? "data"
                  : kind === "models"
                    ? "models"
                    : "logs",
            }),
          )}>Open {kind} folder</button
      >{/each}<button
      class="button"
      onclick={() =>
        app.perform("diagnostics", async () => {
          diagnostics = await call("get_diagnostics");
        })}>Show diagnostics</button
    >
  </div>
  {#if diagnostics}<textarea
      class="spaced"
      rows="8"
      readonly
      aria-label="Desktop diagnostics"
      value={diagnostics}></textarea>{/if}
  <details class="spaced"><summary>Terms of Use</summary><TermsText /></details>
  <details class="spaced">
    <summary>Apache License 2.0</summary>
    <pre>{apacheLicense}</pre>
  </details>
  <LegalLinks /><ThirdPartyNotices />
</details>

<style>
  .cli-path {
    font-size: 12px;
    max-width: 420px;
    overflow-wrap: anywhere;
  }
  summary {
    cursor: pointer;
  }
  textarea {
    width: 100%;
  }
</style>
