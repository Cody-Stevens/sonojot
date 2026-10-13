<script lang="ts">
  import { onMount, tick } from "svelte";
  import { suffixPreviewOffset } from "../lib/preview";
  let { text }: { text: string } = $props();
  let viewport: HTMLDivElement;
  let content: HTMLSpanElement;
  let offset = $state(0);
  function measure(): void {
    if (viewport && content)
      offset = suffixPreviewOffset(
        content.getBoundingClientRect().width,
        viewport.clientWidth,
      );
  }
  $effect(() => {
    const currentText = text;
    void tick().then(() => {
      if (currentText === text) measure();
    });
  });
  onMount(() => {
    const observer = new ResizeObserver(measure);
    observer.observe(viewport);
    observer.observe(content);
    measure();
    return () => observer.disconnect();
  });
</script>

<div class="overlay-preview">
  <div
    bind:this={viewport}
    class="overlay-transcript"
    class:overflowing={offset > 0.5}
    aria-label="Live transcript"
    dir="ltr"
  >
    <span bind:this={content} style:transform={`translateX(${-offset}px)`}
      >{text}</span
    >
  </div>
  {#if offset > 0.5}<span class="preview-omission" aria-hidden="true">…</span
    >{/if}
</div>

<style>
  .overlay-preview {
    position: relative;
    padding-top: 9px;
  }
  .overlay-transcript {
    margin-top: 0;
  }
  .preview-omission {
    position: absolute;
    left: 0;
    top: 9px;
    color: var(--soft-ink);
    font-size: 13px;
    line-height: 1.35;
    pointer-events: none;
    user-select: none;
  }
</style>
