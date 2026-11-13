<script lang="ts">
  import { tick } from "svelte";
  import Icon from "../Icon.svelte";
  let {
    label,
    value,
    options,
    disabled = false,
    onchange,
  }: {
    label: string;
    value: string;
    options: { value: string; label: string }[];
    disabled?: boolean;
    onchange: (value: string) => void;
  } = $props();
  const id = $props.id();
  let open = $state(false);
  let root: HTMLDivElement;
  let trigger: HTMLButtonElement;
  async function show() {
    if (disabled) return;
    open = true;
    await tick();
    const buttons = [
      ...root.querySelectorAll<HTMLButtonElement>('[role="option"]'),
    ];
    (
      buttons.find(
        (button) => button.getAttribute("aria-selected") === "true",
      ) ?? buttons[0]
    )?.focus({ preventScroll: true });
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
    <span
      >{options.find((option) => option.value === value)?.label ?? value}</span
    ><span aria-hidden="true">⌄</span>
  </button>
  {#if open}<div
      class="stock paper-popover picker-options"
      {id}
      role="listbox"
      aria-label={label}
    >
      {#each options as option}<button
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
    gap: 16px;
    width: 100%;
    min-width: 230px;
    padding: 9px 13px;
    color: var(--ink);
    background-color: var(--well);
    border: 1px solid var(--line);
    border-radius: 9px;
    box-shadow: var(--press);
    text-align: left;
    font-size: 16px;
  }
  .picker-well span:first-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    border-radius: 12px;
    box-shadow: var(--raise);
  }
  .picker-options button {
    display: flex;
    align-items: center;
    gap: 18px;
    justify-content: space-between;
    width: 100%;
    padding: 12px;
    border: 0;
    border-radius: 7px;
    text-align: left;
    background: transparent;
    color: var(--ink);
  }
  /* Hover and keyboard highlight: "Raised selected paper" in styles.css. */
</style>
