<script lang="ts">
  import { call, native } from "../lib/api";
  import { app } from "../lib/app.svelte";
  import {
    PUNCTUATION_HELP,
    PUNCTUATION_MODEL,
    punctuationModelPage,
  } from "../lib/punctuation";

  let { id = PUNCTUATION_MODEL }: { id?: string } = $props();
  const help = $derived(
    id === "sonojot-punct-v1"
      ? "Fixes sentence breaks and commas in dictation; runs on CPU"
      : PUNCTUATION_HELP,
  );

  function openModelPage(event: MouseEvent): void {
    if (!native) return;
    event.preventDefault();
    void app.perform("punctuation-model-page", () =>
      call("open_punctuation_model_page", { id }),
    );
  }
</script>

<p data-text-surface="flat">{help}</p>
<details data-text-surface="flat">
  <summary>Details</summary>
  <p>
    SonoJot Punctuation {id === "sonojot-punct-v1" ? "v1" : "v2"}, derived from
    Horizon-Labs/punctuation-restoration-small and jhu-clsp/mmBERT-small.
    Training labels use Rev.com Earnings-21{id === "sonojot-punct-v2"
      ? " and Earnings-22; v2 continues v1"
      : ""}.
  </p>
  <p>
    Modified versions of these model weights must stay under CC BY-SA 4.0. Keep
    attribution, license and change notices with redistributed models. This
    condition does not by itself apply to every transcription or the whole app.
  </p>
  <p>
    CC BY-SA 4.0 · <a
      href={punctuationModelPage(id)}
      onclick={openModelPage}
      target="_blank"
      rel="noopener noreferrer">Model page ↗</a
    >
  </p>
</details>

<style>
  p {
    margin: 9px 0;
  }
  p,
  details {
    color: var(--soft-ink);
  }
  summary {
    cursor: pointer;
  }
  a {
    color: inherit;
    text-decoration: underline;
  }
</style>
