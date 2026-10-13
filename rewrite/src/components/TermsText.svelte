<script lang="ts">
  import terms from "../../legal/terms-of-use.md?raw";
  import { renderLegalMarkdown, legalLink } from "../lib/legal-markdown";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  // The "Version: N" line is a machine marker; the text shows its own version line.
  const html = renderLegalMarkdown(terms.replace(/^Version: \d+\r?\n+/m, ""));
  function openLink(event: MouseEvent): void {
    const anchor =
      event.target instanceof Element ? event.target.closest("a") : null;
    if (!anchor) return;
    event.preventDefault();
    const url = legalLink(anchor.href);
    if (url)
      void app.perform("legal-link", () => call("open_legal_link", { url }));
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions (The region scrolls by keyboard; generated anchors provide native keyboard activation and delegate clicks here.) -->
<div
  class="paper-well terms-text"
  role="region"
  aria-label="Full SonoJot Terms of Use"
  tabindex="0"
  data-text-surface="flat"
  onclick={openLink}
  onauxclick={(event) => {
    event.preventDefault();
    if (event.button === 1) openLink(event);
  }}
>
  {@html html}
</div>

<style>
  .terms-text {
    max-height: min(45vh, 420px);
    overflow: auto;
    overscroll-behavior: contain;
    padding: 18px;
    margin: 18px 0;
    user-select: text;
  }
  .terms-text:focus-visible {
    outline: 3px solid var(--focus);
    outline-offset: 4px;
  }
  .terms-text :global(p),
  .terms-text :global(li) {
    margin: 0;

    overflow-wrap: anywhere;
    font: inherit;
    line-height: 1.6;
  }
  .terms-text :global(p) {
    margin: 0 0 1em;
  }
  .terms-text :global(h2),
  .terms-text :global(h3) {
    font-size: 1.15rem;
    line-height: 1.4;
    margin: 1.4em 0 0.6em;
  }
  .terms-text :global(h2:first-child) {
    margin-top: 0;
  }
  .terms-text :global(ul),
  .terms-text :global(ol) {
    padding-left: 1.5em;
    margin: 0 0 1em;
  }
  .terms-text :global(li) {
    margin-bottom: 0.6em;
  }
  .terms-text :global(a) {
    color: var(--ink);
    text-decoration: underline;
  }
</style>
