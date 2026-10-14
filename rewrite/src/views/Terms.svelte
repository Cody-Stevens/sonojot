<script lang="ts">
  import TermsAcceptance from "../components/TermsAcceptance.svelte";
  import { app } from "../lib/app.svelte";
  import { acceptTerms } from "../lib/terms.svelte";
  import { errorMessage } from "../lib/protocol";
  import { scrollEdges } from "../lib/scroll-edges";

  let { onclose }: { onclose: () => Promise<void> } = $props();
  let agreed = $state(false);
  let saving = $state(false);
  let error = $state("");

  async function accept(): Promise<void> {
    if (!agreed || saving || app.busy("settings")) return;
    saving = true;
    error = "";
    try {
      await acceptTerms();
    } catch (failure: unknown) {
      error = errorMessage(failure);
    } finally {
      saving = false;
    }
  }
</script>

<main id="page-content" class="terms-page" use:scrollEdges>
  <div class="terms-content">
    <h1 data-text-surface="emboss">SonoJot Terms of Use</h1>
    <p class="body-copy" data-text-surface="flat">
      {app.snapshot.settings.termsAcceptedVersion > 0
        ? "Please review these updated terms before continuing."
        : "Please review these terms before continuing."}
      You can close this window and decide later.
    </p>
    <TermsAcceptance bind:agreed disabled={saving} />
    {#if error || app.error}<p class="error" role="alert">
        {error || app.error}
      </p>{/if}
    <div class="actions spaced">
      <button
        class="button quiet"
        disabled={saving || app.busy("window")}
        onclick={onclose}>Close</button
      >
      <button
        class="button primary"
        disabled={!agreed || saving || app.busy("settings")}
        onclick={accept}
      >
        {saving ? "Saving…" : "Agree and continue"}
      </button>
    </div>
  </div>
</main>

<style>
  /* Like first-run setup, the page fits the window: only the Terms panel
     scrolls. Overflow stays available for very large text. */
  .terms-page {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 27px 36px;
    display: flex;
    flex-direction: column;
  }
  .terms-content {
    flex: 1;
    min-height: 0;
    width: 100%;
    max-width: 810px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
  }
  .terms-content > * {
    flex: none;
  }
  .terms-content > :global(.terms-acceptance) {
    flex: 1 1 0;
  }
  h1 {
    margin: 0 0 18px;
  }
  .error {
    color: var(--danger);
  }
  @media (max-width: 600px), (max-height: 700px) {
    .terms-page {
      padding: 18px;
    }
  }
</style>
