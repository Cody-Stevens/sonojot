<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  import Icon from "../Icon.svelte";
  import { scrollEdges } from "../../lib/scroll-edges";
  let {
    title,
    children,
    onclose,
    anchor = null,
  }: {
    title: string;
    children: Snippet;
    onclose: () => void;
    anchor?: HTMLElement | null;
  } = $props();
  const id = $props.id();
  let dialog: HTMLDialogElement;
  let opened = $state(false);
  let left = $state(0),
    top = $state(0);
  onMount(() => {
    const returnFocus = document.activeElement;
    dialog.showModal();
    if (anchor) {
      const rect = anchor.getBoundingClientRect();
      left = Math.max(
        12,
        Math.min(
          window.innerWidth - dialog.offsetWidth - 12,
          rect.right - dialog.offsetWidth,
        ),
      );
      top = Math.max(
        12,
        Math.min(
          rect.bottom + 6,
          window.innerHeight - dialog.offsetHeight - 12,
        ),
      );
    }
    const frame = requestAnimationFrame(() => {
      opened = true;
    });
    return () => {
      cancelAnimationFrame(frame);
      dialog.close();
      if (returnFocus instanceof HTMLElement && returnFocus.isConnected)
        returnFocus.focus({ preventScroll: true });
    };
  });
  function keyboard(event: KeyboardEvent) {
    // Native modal inertness plus an explicit Tab loop keep focus in this sheet.
    if (event.key !== "Tab") return;
    const items = [
      ...dialog.querySelectorAll<HTMLElement>(
        'button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex="0"]',
      ),
    ].filter((item) => item.getClientRects().length > 0);
    const first = items[0],
      last = items.at(-1);
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last?.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first?.focus();
    }
  }
</script>

<dialog
  bind:this={dialog}
  class="quiet-sheet-dialog"
  class:opened
  class:anchored={anchor !== null}
  style:left={anchor ? `${left}px` : undefined}
  style:top={anchor ? `${top}px` : undefined}
  style:--sheet-top={anchor ? `${top}px` : "0px"}
  aria-labelledby={id}
  onkeydown={keyboard}
  oncancel={(event) => {
    event.preventDefault();
    event.stopPropagation();
    onclose();
  }}
  onclick={(event) => {
    if (event.target === dialog) onclose();
  }}
>
  <div class="stock quiet-sheet">
    <header>
      <h2 {id} data-text-surface="emboss">{title}</h2>
      <button
        type="button"
        class="button small"
        aria-label="Close sheet"
        onclick={onclose}><Icon name="close" size={18} /></button
      >
    </header>
    <div class="sheet-scroll" use:scrollEdges>{@render children()}</div>
  </div>
</dialog>

<style>
  .quiet-sheet-dialog {
    border: 0;
    padding: 12px;
    margin: auto;
    background: transparent;
    color: var(--ink);
    width: min(650px, calc(100vw - 36px));
    max-width: none;
    max-height: calc(100dvh - 36px);
    overflow: visible;
    opacity: 0;
    transform: translateY(8px);
    transition:
      opacity 200ms var(--ease-settle),
      transform 200ms var(--ease-settle);
  }
  .quiet-sheet-dialog.opened {
    opacity: 1;
    transform: translateY(0);
  }
  .quiet-sheet-dialog.anchored {
    position: fixed;
    margin: 0;
    width: min(440px, calc(100vw - 24px));
  }
  .anchored .quiet-sheet {
    padding: 18px;
    max-height: calc(100dvh - var(--sheet-top) - 36px);
  }
  .quiet-sheet-dialog.anchored::backdrop {
    background: transparent;
  }
  .quiet-sheet-dialog::backdrop {
    background: color-mix(in srgb, var(--ink) 18%, transparent);
  }
  .quiet-sheet {
    border-radius: 18px 17px 19px 17px;
    padding: 24px;
    box-shadow: var(--raise);
    display: flex;
    flex-direction: column;
    max-height: calc(100dvh - 60px);
  }
  header {
    display: flex;
    align-items: center;
    gap: 18px;
    margin-bottom: 18px;
  }
  h2 {
    flex: 1;
    margin: 0;
    font-size: 1.35rem;
  }
  .sheet-scroll {
    min-height: 0;
    overflow: auto;
    padding: 4px;
  }
  @media (prefers-reduced-motion: reduce) {
    .quiet-sheet-dialog {
      transition: none;
      transform: none;
    }
  }
</style>
