<script lang="ts">
  import { tick } from "svelte";
  import Icon from "../Icon.svelte";
  import { scrollEdges } from "../../lib/scroll-edges";

  let {
    label,
    value,
    options,
    disabled = false,
    onchange,
  }: {
    label: string;
    value: string;
    options: readonly { value: string; label: string }[];
    disabled?: boolean;
    onchange: (value: string) => void;
  } = $props();
  const id = $props.id();
  let open = $state(false);
  let openUp = $state(false);
  let maxHeight = $state(300);
  let root: HTMLDivElement;
  let trigger: HTMLButtonElement;
  async function show() {
    if (disabled) return;
    const rect = trigger.getBoundingClientRect();
    const scrollport = trigger.closest<HTMLElement>(".page-content");
    const bounds = scrollport
      ? scrollport.getBoundingClientRect()
      : { top: 0, bottom: window.innerHeight };
    const topLimit = Math.max(8, bounds.top + 8);
    const bottomLimit = Math.min(window.innerHeight - 8, bounds.bottom - 8);
    const spaceBelow = Math.max(0, bottomLimit - rect.bottom - 10);
    const spaceAbove = Math.max(0, rect.top - topLimit - 10);
    openUp = spaceBelow < 180 && spaceAbove > spaceBelow;
    const available = openUp ? spaceAbove : spaceBelow;
    maxHeight = Math.max(110, Math.min(300, Math.floor(available)));
    open = true;
    await tick();
    const buttons = [
      ...root.querySelectorAll<HTMLButtonElement>('[role="option"]'),
    ];
    const target =
      buttons.find(
        (button) => button.getAttribute("aria-selected") === "true",
      ) ?? buttons[0];
    if (target) {
      target.scrollIntoView({ block: "nearest" });
      target.focus({ preventScroll: true });
    }
  }
  function close(returnFocus = false) {
    open = false;
    if (returnFocus) trigger.focus({ preventScroll: true });
  }
  function keyboard(event: KeyboardEvent) {
    if (event.key === "Escape" && open) {
      event.preventDefault();
      event.stopPropagation();
      close(true);
      return;
    }
    if (event.key === "Tab") {
      if (open) close(true);
      return;
    }
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    if (!open) {
      void show();
      return;
    }
    const buttons = [
      ...root.querySelectorAll<HTMLButtonElement>('[role="option"]'),
    ];
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? buttons.length - 1
          : (index + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) %
            buttons.length;
    buttons[next]?.focus();
  }
  $effect(() => {
    if (disabled) open = false;
  });
</script>

<svelte:window
  onpointerdown={(event) => {
    if (open && event.target instanceof Node && !root.contains(event.target))
      close();
  }}
/>
<div class="picker" bind:this={root} role="group" aria-label={label}>
  <button
    bind:this={trigger}
    onkeydown={keyboard}
    type="button"
    class="picker-well"
    aria-label={label}
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-controls={id}
    {disabled}
    onclick={() => (open ? close() : void show())}
  >
    <span class="picker-label"
      >{options.find((option) => option.value === value)?.label ?? value}</span
    >
    <span class="picker-chevron" class:open aria-hidden="true">
      <Icon name="chevron-down" size={15} />
    </span>
  </button>
  {#if open}<div
      class="stock paper-popover picker-options"
      class:open-up={openUp}
      style:max-height={`${maxHeight}px`}
      {id}
      role="listbox"
      aria-label={label}
      use:scrollEdges
    >
      {#each options as option, index (`${index}:${option.value}`)}<button
          type="button"
          onkeydown={keyboard}
          role="option"
          aria-selected={value === option.value}
          onclick={() => {
            onchange(option.value);
            close(true);
          }}
          ><span>{option.label}</span>{#if value === option.value}<Icon
              name="check"
              size={16}
            />{/if}</button
        >{/each}
    </div>{/if}
</div>

<style>
  .picker {
    position: relative;
    max-width: 100%;
  }
  .picker-well {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    width: 100%;
    min-width: min(230px, 100%);
    min-height: 36px;
    padding: 6px 12px;
    color: var(--ink);
    background-color: var(--well);
    border: 1px solid var(--line);
    border-radius: var(--r-control);
    box-shadow: var(--press);
    text-align: left;
    font-size: 15px;
  }
  .picker-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .picker-chevron {
    display: grid;
    place-items: center;
    flex: none;
    color: var(--soft-ink);
    transition: transform var(--motion-control) var(--ease-settle);
  }
  .picker-chevron.open {
    transform: rotate(180deg);
  }
  .picker-options {
    position: absolute;
    z-index: 20;
    top: calc(100% + 7px);
    left: 0;
    min-width: 100%;
    width: max-content;
    max-width: min(420px, 70vw);
    max-height: 300px;
    overflow: auto;
    padding: 7px;
    border-radius: var(--r-control);
    box-shadow: var(--raise);
  }
  .picker-options.open-up {
    top: auto;
    bottom: calc(100% + 7px);
  }
  .picker-options button {
    display: flex;
    align-items: center;
    gap: 18px;
    justify-content: space-between;
    width: 100%;
    padding: 10px 12px;
    border: 0;
    border-radius: var(--r-thumb);
    text-align: left;
    background: transparent;
    color: var(--ink);
    font-size: 15px;
  }
  @media (prefers-reduced-motion: reduce) {
    .picker-chevron {
      transition: none;
    }
  }
  /* Hover and keyboard highlight: "Raised selected paper" in styles.css. */
</style>
