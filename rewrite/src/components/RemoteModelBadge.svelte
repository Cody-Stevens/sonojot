<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "./Icon.svelte";

  // Inside a picker button, that button owns focus and the description.
  let { embedded = false }: { embedded?: boolean } = $props();
  const uid = $props.id();
  const warning =
    "Not a local model. Audio or text is sent to OpenRouter and may not be private.";
  let badge = $state<HTMLElement>();
  let tooltip: HTMLSpanElement;

  onMount(() => {
    if (!badge) return;
    const marker = badge;
    const target = embedded ? marker.closest("button") : marker;
    if (!target) return;
    const description = target.getAttribute("aria-describedby");
    target.setAttribute(
      "aria-describedby",
      [description, uid].filter(Boolean).join(" "),
    );
    let hideTimer: ReturnType<typeof setTimeout> | undefined;
    function hide(): void {
      clearTimeout(hideTimer);
      tooltip.hidePopover();
    }
    function show(): void {
      clearTimeout(hideTimer);
      tooltip.showPopover();
      const anchor = marker.getBoundingClientRect();
      const rect = tooltip.getBoundingClientRect();
      tooltip.style.left = `${Math.max(9, Math.min(anchor.right - rect.width, window.innerWidth - rect.width - 9))}px`;
      tooltip.style.top = `${anchor.top > rect.height + 18 ? anchor.top - rect.height - 9 : anchor.bottom + 9}px`;
    }
    function leave(): void {
      // Keep the tooltip reachable while the pointer crosses the small gap.
      hideTimer = setTimeout(hide, 100);
    }
    function key(event: KeyboardEvent): void {
      if (event.key === "Escape") hide();
    }
    function reposition(): void {
      if (tooltip.matches(":popover-open")) show();
    }
    target.addEventListener("mouseenter", show);
    target.addEventListener("mouseleave", leave);
    target.addEventListener("focus", show);
    target.addEventListener("blur", hide);
    target.addEventListener("keydown", key);
    tooltip.addEventListener("mouseenter", show);
    tooltip.addEventListener("mouseleave", leave);
    window.addEventListener("resize", reposition);
    document.addEventListener("scroll", reposition, true);
    return () => {
      clearTimeout(hideTimer);
      target.removeEventListener("mouseenter", show);
      target.removeEventListener("mouseleave", leave);
      target.removeEventListener("focus", show);
      target.removeEventListener("blur", hide);
      target.removeEventListener("keydown", key);
      tooltip.removeEventListener("mouseenter", show);
      tooltip.removeEventListener("mouseleave", leave);
      window.removeEventListener("resize", reposition);
      document.removeEventListener("scroll", reposition, true);
      if (description) target.setAttribute("aria-describedby", description);
      else target.removeAttribute("aria-describedby");
    };
  });
</script>

{#if embedded}
  <span bind:this={badge} class="remote-model-badge" aria-hidden="true"
    ><Icon name="eye" size={18} /></span
  >
{:else}
  <button
    bind:this={badge}
    type="button"
    class="remote-model-badge"
    aria-label="Remote model privacy"><Icon name="eye" size={18} /></button
  >
{/if}
<span
  bind:this={tooltip}
  id={uid}
  class="remote-model-tooltip stock paper-popover"
  role="tooltip"
  popover="manual"
  data-text-surface="flat">{warning}</span
>

<style>
  .remote-model-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    vertical-align: middle;
    padding: 3px;
    margin: 0 3px;
    border: 0;
    border-radius: 4px;
    background: transparent;
    box-shadow: none;
    color: var(--soft-ink);
    line-height: 1;
  }
  button.remote-model-badge {
    cursor: help;
  }
  .remote-model-tooltip {
    position: fixed;
    inset: auto;
    margin: 0;
    width: max-content;
    max-width: min(300px, calc(100vw - 36px));
    padding: 9px 12px;
    font-size: 0.85rem;
    line-height: 1.4;
    text-align: left;
    white-space: normal;
  }
</style>
