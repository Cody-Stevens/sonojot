<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { scrollEdges } from "../lib/scroll-edges";
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
  $effect(() => {
    if (app.snapshot.resultSessionId !== null || active)
      open = reviewingTest && !active;
  });
</script>

{#if (incomplete || empty || reviewingTest) && !active}
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
      {:else}Test result. Review the text from your microphone test.{/if}
    </p>
    <button
      class="button small"
      aria-expanded={open}
      onclick={() => (open = !open)}
      >{open ? "Close review" : "Review available text"}</button
    >
    {#if open}
      <div class="review-text">
        {#each app.snapshot.recognitionWarnings as warning}<p
            class="field-help"
            data-text-surface="flat"
          >
            {warning}
          </p>{/each}
        {#each [{ original: true, title: "As recognized", text: app.snapshot.originalText }, { original: false, title: incomplete ? "Available text" : "Finished text", text: app.snapshot.outputText }] as version}
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
