<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { scrollEdges } from "../lib/scroll-edges";
  import { call } from "../lib/api";
  import { smartRecoverySession } from "../lib/smart-cleanup";
  import { proofreaderAccountBlocked } from "../lib/proofreader";
  import ProofreaderPrivacyLink from "./ProofreaderPrivacyLink.svelte";
  let { reviewSessionId = null }: { reviewSessionId?: number | null } =
    $props();
  const reviewingTest = $derived(
    reviewSessionId !== null &&
      reviewSessionId === app.snapshot.resultSessionId,
  );
  const active = $derived(
    ["loading", "listening", "finishing", "cleaning"].includes(
      app.snapshot.state,
    ),
  );
  const incomplete = $derived(
    app.snapshot.recognitionCompleteness === "incomplete",
  );
  const empty = $derived(
    !incomplete &&
      app.snapshot.resultSessionId !== null &&
      !app.snapshot.originalText &&
      !app.snapshot.outputText,
  );
  let open = $state(false);
  const recoverySession = $derived(smartRecoverySession(app.snapshot));
  const proofread = $derived(
    app.snapshot.publicationStage === "proofread" && recoverySession !== null,
  );
  const accountBlocked = $derived(
    proofreaderAccountBlocked(app.snapshot.guardReason) ||
      proofreaderAccountBlocked(app.snapshot.cleanupStatus),
  );
  $effect(() => {
    if (app.snapshot.resultSessionId !== null || active)
      open = reviewingTest && !active;
  });
</script>

{#if (incomplete || empty || reviewingTest || proofread || accountBlocked) && !active}
  <div
    class="notice review-notice"
    role="region"
    aria-label="Current dictation"
  >
    <p role="status">
      {#if incomplete || empty}Recognition is incomplete. {app.snapshot
          .originalText || app.snapshot.outputText
          ? "Review the available text or dictate it again."
          : "No text could be confirmed. Dictate it again."}
      {:else if proofread}Proofread. You can review or copy the text before
        proofreading.
      {:else if accountBlocked}Proofreading was skipped.
      {:else}Test result. Review the text from your microphone test.{/if}
    </p>
    <button
      class="button small"
      aria-expanded={open}
      onclick={() => (open = !open)}
      >{open ? "Close review" : "Review available text"}</button
    >
    {#if accountBlocked}<ProofreaderPrivacyLink />{/if}
    {#if open}
      <div class="review-text">
        {#if recoverySession !== null && app.snapshot.preModelText}
          <h3 data-text-surface="emboss">Before AI changes</h3>
          <div class="paper-well transcript" use:scrollEdges>
            {app.snapshot.preModelText}
          </div>
          <button
            class="button small"
            disabled={!app.native || app.busy("copy-pre-model")}
            onclick={() => {
              const resultSessionId = recoverySession;
              if (resultSessionId === null) return;
              void app.perform(
                "copy-pre-model",
                () => call("copy_smart_original", { resultSessionId }),
                "Text before changes copied.",
              );
            }}>Copy before AI changes</button
          >
        {/if}
        {#each app.snapshot.recognitionWarnings as warning}<p
            class="field-help"
            data-text-surface="flat"
          >
            {warning}
          </p>{/each}
        {#each [{ original: true, title: "As recognized", text: app.snapshot.originalText }, { original: false, title: incomplete ? "Available text" : proofread ? "Proofread" : "Finished text", text: app.snapshot.outputText }] as version}
          <h3 data-text-surface="emboss">{version.title}</h3>
          <div class="paper-well transcript" use:scrollEdges>
            {version.text || "No text was recovered."}
          </div>
          <button
            class="button small"
            disabled={!app.native ||
              !version.text ||
              app.busy("copy") ||
              app.snapshot.resultSessionId === null}
            onclick={() => app.copy(version.original)}>Copy after review</button
          >
        {/each}
        <p class="field-help" data-text-surface="flat">
          This current-result review clears when another dictation replaces it
          or SonoJot closes. Saved history follows your retention setting.
        </p>
      </div>
    {/if}
  </div>
{/if}

<style>
  .review-notice {
    flex-wrap: wrap;
  }
  .review-notice > p {
    flex: 1;
    min-width: 200px;
    margin: 0;
  }
  .review-text {
    width: 100%;
  }
  .transcript {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    margin-bottom: 9px;
  }
</style>
