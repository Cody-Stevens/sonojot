<script lang="ts">
  import { onMount, type Snippet } from "svelte";

  // A small "i" that holds the longer explanation for a setting. It opens on
  // click or tap, or Enter or Space when focused (focus alone does not open it,
  // nor does hover), and closes on Escape, an outside click or when focus
  // leaves. Paper popover stock, no new look.
  let {
    label,
    children,
  }: {
    /** Names the button for screen readers: "About Live preview". */
    label: string;
    children: Snippet;
  } = $props();
  const uid = $props.id();
  let button: HTMLButtonElement;
  let note: HTMLDivElement;
  let open = $state(false);

  function place(): void {
    if (!open || !note.matches(":popover-open")) return;
    const anchor = button.getBoundingClientRect();
    const rect = note.getBoundingClientRect();
    note.style.left = `${Math.max(9, Math.min(anchor.left - 12, window.innerWidth - rect.width - 9))}px`;
    note.style.top = `${
      anchor.bottom + rect.height + 18 < window.innerHeight
        ? anchor.bottom + 8
        : Math.max(9, anchor.top - rect.height - 8)
    }px`;
  }
  function show(): void {
    if (open) return;
    open = true;
    note.showPopover();
    place();
  }
  function hide(): void {
    if (!open) return;
    open = false;
    if (note.matches(":popover-open")) note.hidePopover();
  }
  onMount(() => {
    function outside(event: PointerEvent): void {
      const target = event.target as Node | null;
      if (open && !button.contains(target) && !note.contains(target)) hide();
    }
    function key(event: KeyboardEvent): void {
      if (event.key !== "Escape" || !open) return;
      // Close only this note; an enclosing sheet stays open.
      event.preventDefault();
      event.stopPropagation();
      hide();
      button.focus({ preventScroll: true });
    }
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("keydown", key, true);
    window.addEventListener("resize", place);
    document.addEventListener("scroll", place, true);
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("keydown", key, true);
      window.removeEventListener("resize", place);
      document.removeEventListener("scroll", place, true);
      hide();
    };
  });
</script>

<button
  bind:this={button}
  type="button"
  class="info-button"
  aria-label={label}
  aria-expanded={open}
  aria-controls={uid}
  onblur={(event) => {
    if (!note.contains(event.relatedTarget as Node | null)) hide();
  }}
  onclick={(event) => {
    // Inside a choice card, the note never chooses the card.
    event.stopPropagation();
    event.preventDefault();
    if (open) hide();
    else show();
  }}><span aria-hidden="true">i</span></button
><div
  bind:this={note}
  id={uid}
  class="info-note stock paper-popover"
  role="note"
  aria-label={label}
  popover="manual"
  data-text-surface="flat"
>
  {@render children()}
</div>

<style>
  .info-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    width: 22px;
    height: 22px;
    margin: 0 2px;
    padding: 0;
    vertical-align: middle;
    border: 1px solid var(--line);
    border-radius: 50%;
    background: transparent;
    box-shadow: none;
    color: var(--soft-ink);
    font-family: inherit;
    font-size: 13px;
    font-style: italic;
    font-weight: 600;
    line-height: 1;
    cursor: pointer;
  }
  .info-button:hover,
  .info-button[aria-expanded="true"] {
    color: var(--ink);
    border-color: var(--soft-ink);
  }
  .info-button:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }
  /* Grow the touch target without growing the printed mark. */
  .info-button::after {
    content: "";
    position: absolute;
    inset: -10px;
  }
  .info-button {
    position: relative;
  }
  .info-note {
    position: fixed;
    inset: auto;
    margin: 0;
    width: max-content;
    max-width: min(340px, calc(100vw - 36px));
    padding: 12px 14px;
    font-size: 0.85rem;
    font-style: normal;
    font-weight: 400;
    line-height: 1.45;
    text-align: left;
    white-space: normal;
    opacity: 0;
    transition: opacity var(--motion-popover) ease-out;
  }
  .info-note:popover-open {
    opacity: 1;
  }
  .info-note :global(p) {
    margin: 0 0 6px;
  }
  .info-note :global(p:last-child) {
    margin-bottom: 0;
  }
  .info-note :global(dl) {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 3px 12px;
    margin: 6px 0 0;
  }
  .info-note :global(dt) {
    color: var(--soft-ink);
  }
  .info-note :global(dd) {
    margin: 0;
  }
  .info-note :global(small) {
    display: block;
    margin-top: 6px;
    color: var(--soft-ink);
    font-size: 0.78rem;
    overflow-wrap: anywhere;
  }
  @media (prefers-reduced-motion: reduce) {
    .info-note {
      transition: none;
    }
  }
</style>
