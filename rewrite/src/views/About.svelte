<script lang="ts">
  import Card from "../components/Card.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import { updateStatusLabel } from "../lib/updates";
  import UpdateAction from "../components/UpdateAction.svelte";
  import ThirdPartyNotices from "../components/ThirdPartyNotices.svelte";
  import LegalLinks from "../components/LegalLinks.svelte";
  import TermsText from "../components/TermsText.svelte";
  import apacheLicense from "../../../LICENSE?raw";
  import { TERMS_VERSION } from "../lib/terms";
  let diagnostics = $state("");
  let { ontour }: { ontour?: () => void } = $props();
  // Explains a disabled Restart to update beside Check now.
  const updateReason = $derived(
    app.snapshot.update.status.state === "ready" &&
      !app.snapshot.update.installing
      ? app.updateRestartBlockedReason
      : null,
  );
</script>

<header class="page-heading">
  <h1 class="text-grain" data-text-surface="emboss">About</h1>
</header>
<Card title="SonoJot">
  {#if ontour}<button
      class="button"
      disabled={!app.native ||
        !["idle", "error"].includes(app.snapshot.state) ||
        app.snapshot.meetingRecording}
      onclick={ontour}>Take the tour again</button
    >{/if}
  <dl class="details-list" data-text-surface="flat">
    <div>
      <dt>Version</dt>
      <dd>{app.snapshot.version}</dd>
    </div>
    <div>
      <dt>Platform</dt>
      <dd>{app.snapshot.capabilities.platform}</dd>
    </div>
  </dl>
  <div class="update-check">
    <div class="update-actions">
      <button
        class="text-action"
        disabled={!app.native ||
          app.busy("update-check") ||
          app.snapshot.update.installing ||
          app.snapshot.update.status.state === "checking" ||
          app.snapshot.update.status.state === "downloading"}
        onclick={() => app.checkForUpdates()}>Check now</button
      >
      <UpdateAction
        describedby={updateReason ? "about-update-reason" : undefined}
      />
    </div>
    <p class="field-help" role="status" data-text-surface="flat">
      {app.snapshot.update.installing
        ? "Restarting to update…"
        : updateStatusLabel(app.snapshot.update.status)}
    </p>
    {#if updateReason}<p
        id="about-update-reason"
        class="field-help"
        data-text-surface="flat"
      >
        {updateReason}
      </p>{/if}
  </div>
  <h3 class="spaced" data-text-surface="emboss">Where recognition runs</h3>
  <p class="body-copy" data-text-surface="flat">
    Dictation runs on this computer. Meeting audio can be transcribed online
    only when OpenRouter is connected and an online speech model is selected.
    Originals stay available alongside cleaned text.
  </p>
  <details class="license spaced">
    <summary data-text-surface="emboss">Troubleshooting</summary>
    <div class="actions spaced">
      {#each [{ kind: "data", name: "Open data folder" }, { kind: "models", name: "Open models folder" }, { kind: "logs", name: "Open logs folder" }] as item}<button
          class="button small"
          disabled={!app.native || app.busy("folder")}
          onclick={() =>
            app.perform("folder", () =>
              call("open_path", {
                kind:
                  item.kind === "data"
                    ? "data"
                    : item.kind === "models"
                      ? "models"
                      : "logs",
              }),
            )}>{item.name}</button
        >{/each}
    </div>
    <p class="body-copy spaced" data-text-surface="flat">
      Inspect recording, model and delivery status.
    </p>
    <button
      class="button"
      disabled={!app.native || app.busy("diagnostics")}
      onclick={() =>
        app.perform("diagnostics", async () => {
          diagnostics = await call("get_diagnostics");
        })}
      >{app.busy("diagnostics")
        ? "Reading…"
        : diagnostics
          ? "Refresh diagnostics"
          : "Show diagnostics"}</button
    >{#if diagnostics}<textarea
        class="paper-well diagnostics"
        readonly
        aria-label="Desktop diagnostics"
        rows="9"
        value={diagnostics}></textarea>{/if}
  </details>
  <details class="license spaced">
    <summary data-text-surface="emboss">SonoJot Terms of Use</summary>
    <p class="body-copy spaced" data-text-surface="flat">
      Version {TERMS_VERSION}. Bundled for offline reading.
    </p>
    <TermsText />
  </details>
  <details class="license spaced">
    <summary data-text-surface="emboss">Apache License 2.0</summary>
    <p class="body-copy spaced" data-text-surface="flat">
      SonoJot's source code is licensed under Apache-2.0.
    </p>
    <pre>{apacheLicense}</pre>
  </details>
  <LegalLinks />
  <ThirdPartyNotices />
</Card>

<style>
  .update-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-2);
  }
  .update-check .field-help {
    margin: var(--space-1) 0 0;
  }
</style>
