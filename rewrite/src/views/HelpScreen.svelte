<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../components/Icon.svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import Disclosure from "../components/quiet/Disclosure.svelte";
  import LegalLinks from "../components/LegalLinks.svelte";
  import ThirdPartyNotices from "../components/ThirdPartyNotices.svelte";
  import TermsText from "../components/TermsText.svelte";
  import { scrollEdges } from "../lib/scroll-edges";
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

<section class="stock paper-card ui-card row-card quiet-settings">
  <button class="settings-link" onclick={onreport}
    ><strong data-text-surface="emboss">Report a problem</strong><span
      class="door-detail"
      data-text-surface="flat">See what is sent first</span
    ><span class="door-chevron" aria-hidden="true"
      ><Icon name="chevron-right" size={15} /></span
    ></button
  >
  <SettingRow label="Command line" layout="choice">
    {#snippet info()}
      <p>
        sonojot-cli runs everything the app window can do, for scripts and other
        apps.
      </p>
    {/snippet}
    <div class="cli-row">
      <code class="cli-path" title={cliPath}
        >{cliPath || "Reading installed path…"}</code
      >
      <button
        class="button small"
        disabled={!cliPath || app.busy("copy-cli")}
        onclick={() =>
          app.perform(
            "copy-cli",
            () => call("copy_cli_path"),
            "CLI path copied.",
          )}>Copy path</button
      >
    </div>
  </SettingRow>
  <SettingRow label="First-run setup" layout="action">
    {#snippet info()}
      <p>
        Runs the first-run setup again, including the meeting consent reminder.
      </p>
    {/snippet}
    <button class="button small" disabled={locked} onclick={runAgain}
      >Run again</button
    >
  </SettingRow>
  <Disclosure title="Troubleshooting and legal">
    <div class="help-fold-body">
      <div class="actions">
        {#each ["data", "models", "logs"] as kind}<button
            class="button small"
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
          class="button small"
          onclick={() =>
            app.perform("diagnostics", async () => {
              diagnostics = await call("get_diagnostics");
            })}>Show diagnostics</button
        >
      </div>
      {#if diagnostics}
        <div class="scroll-edges-frame">
          <textarea
            rows="8"
            readonly
            aria-label="Desktop diagnostics"
            value={diagnostics}
            use:scrollEdges={{ frame: true }}
          ></textarea>
        </div>
      {/if}
      <div class="nested-fold">
        <Disclosure title="Terms of Use"><TermsText /></Disclosure>
      </div>
      <div class="nested-fold">
        <Disclosure title="Apache License 2.0">
          <pre class="license-pre">{apacheLicense}</pre>
        </Disclosure>
      </div>
      <LegalLinks /><ThirdPartyNotices />
    </div>
  </Disclosure>
</section>

<style>
  .cli-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px 14px;
    width: 100%;
  }
  .cli-path {
    flex: 1 1 200px;
    min-width: 0;
    font-size: 13px;
    line-height: 1.4;
    color: var(--soft-ink);
    overflow-wrap: anywhere;
  }
  .help-fold-body {
    display: grid;
    gap: 12px;
    padding-bottom: 12px;
  }
  .nested-fold {
    border-top: 1px solid var(--line);
  }
  .license-pre {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font:
      13px/1.6 "Cascadia Mono",
      "Courier New",
      monospace;
    margin: 0;
  }
  textarea {
    width: 100%;
  }
</style>
