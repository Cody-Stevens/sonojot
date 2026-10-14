<script lang="ts">
  import ErrorBanner from "./ErrorBanner.svelte";
  import { scrollEdges } from "../lib/scroll-edges";
  import fontLicense from "../brand/fonts/SourceSerif-LICENSE.md?raw";

  let expanded = $state(false);
  let notices = $state("");
  let loading = $state(false);
  let error = $state("");

  async function loadNotices(): Promise<void> {
    if (notices || loading) return;
    loading = true;
    error = "";
    try {
      // Keep the full legal texts out of the startup bundle. This chunk is local
      // in both the installer and the app, so reading it needs no connection.
      notices = (await import("../../THIRD_PARTY_NOTICES.md?raw")).default;
    } catch {
      error = "The notices could not be opened. Please try again.";
    } finally {
      loading = false;
    }
  }
</script>

<details
  class="license spaced"
  bind:open={expanded}
  ontoggle={(event) => {
    if (event.currentTarget.open) void loadNotices();
  }}
>
  <summary data-text-surface="emboss">Credits and licenses</summary>
  {#if expanded}
    <p class="body-copy spaced" data-text-surface="flat">
      Licenses and credits for the app and its optional downloads.
    </p>
    <details class="license" data-text-surface="flat">
      <summary>Read the Source Serif font license</summary>
      <pre>{fontLicense}</pre>
    </details>
    {#if loading}
      <p role="status" data-text-surface="flat">Loading notices…</p>
    {:else if error}
      <ErrorBanner {error} />
      <button class="button small" onclick={loadNotices}>Try again</button>
    {:else}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex (The scrollable document must be reachable for keyboard scrolling.) -->
      <div
        class="paper-well notices"
        use:scrollEdges
        role="region"
        aria-label="Third-party notices"
        tabindex="0"
        data-text-surface="flat"
      >
        <pre>{notices}</pre>
      </div>
    {/if}
  {/if}
</details>

<style>
  .notices {
    max-height: min(60vh, 600px);
    overflow: auto;
    overscroll-behavior: contain;
    padding: 18px;
    margin-top: 18px;
    user-select: text;
  }
  .notices:focus-visible {
    outline: 3px solid var(--focus);
    outline-offset: 4px;
  }
  .notices pre {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font: inherit;
    font-size: 0.9rem;
    line-height: 1.6;
  }
</style>
