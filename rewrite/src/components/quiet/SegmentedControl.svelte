<script lang="ts" module>
  export interface SegmentOption<V extends string> {
    value: V;
    label: string;
    disabled?: boolean;
    id?: string;
    controls?: string;
  }
</script>

<script lang="ts" generics="T extends string">
  let {
    label,
    options,
    value,
    mode = "group",
    size = "default",
    disabled = false,
    onchange,
  }: {
    label: string;
    options: readonly SegmentOption<T>[];
    value: T;
    mode?: "group" | "tabs";
    size?: "default" | "compact";
    disabled?: boolean;
    onchange: (value: T) => void;
  } = $props();

  let root = $state<HTMLDivElement | null>(null);
  let thumbX = $state(0);
  let thumbY = $state(0);
  let thumbW = $state(0);
  let thumbH = $state(0);
  let thumbReady = $state(false);
  let thumbAnimate = $state(false);
  let lastValue = "";

  function measureThumb(animate: boolean) {
    if (!root) return;
    const active = root.querySelector<HTMLButtonElement>(
      `button[data-segment-value="${CSS.escape(value)}"]`,
    );
    if (!active) return;
    const nextW = active.offsetWidth;
    const nextH = active.offsetHeight;
    if (!nextW || !nextH) return;
    thumbAnimate = animate && thumbReady;
    thumbX = active.offsetLeft;
    thumbY = active.offsetTop;
    thumbW = nextW;
    thumbH = nextH;
    thumbReady = true;
  }

  $effect(() => {
    const currentValue = value;
    const optionCount = options.length;
    const el = root;
    if (!el || optionCount === 0) return;
    const changed = lastValue !== "" && lastValue !== currentValue;
    lastValue = currentValue;
    measureThumb(changed);
    const observer = new ResizeObserver(() => measureThumb(false));
    observer.observe(el);
    for (const btn of el.querySelectorAll<HTMLButtonElement>(".seg-btn")) {
      observer.observe(btn);
    }
    let disposed = false;
    void document.fonts?.ready.then(() => {
      if (!disposed) measureThumb(false);
    });
    return () => {
      disposed = true;
      observer.disconnect();
    };
  });

  function onTabKeydown(event: KeyboardEvent) {
    if (mode !== "tabs") return;
    const enabled = options.filter((option) => !disabled && !option.disabled);
    if (!enabled.length) return;
    const current = enabled.findIndex((option) => option.value === value);
    let nextIndex = -1;
    if (event.key === "ArrowRight")
      nextIndex = (current + 1 + enabled.length) % enabled.length;
    else if (event.key === "ArrowLeft")
      nextIndex = (current - 1 + enabled.length) % enabled.length;
    else if (event.key === "Home") nextIndex = 0;
    else if (event.key === "End") nextIndex = enabled.length - 1;
    if (nextIndex < 0) return;
    event.preventDefault();
    const target = enabled[nextIndex];
    onchange(target.value);
    const button = root?.querySelector<HTMLButtonElement>(
      `button[data-segment-value="${CSS.escape(target.value)}"]`,
    );
    button?.focus();
  }
</script>

<div
  bind:this={root}
  class="seg-control"
  class:compact={size === "compact"}
  class:thumb-ready={thumbReady}
  class:thumb-animate={thumbAnimate}
  role={mode === "tabs" ? "tablist" : "group"}
  aria-label={label}
>
  <span
    class="seg-thumb"
    aria-hidden="true"
    style:transform={`translate3d(${thumbX}px, ${thumbY}px, 0)`}
    style:width={`${thumbW}px`}
    style:height={`${thumbH}px`}
  ></span>
  {#each options as option (option.value)}
    {@const selected = value === option.value}
    <button
      type="button"
      id={option.id}
      class="seg-btn"
      data-segment-value={option.value}
      role={mode === "tabs" ? "tab" : undefined}
      aria-pressed={mode === "group" ? selected : undefined}
      aria-selected={mode === "tabs" ? selected : undefined}
      aria-controls={mode === "tabs" ? option.controls : undefined}
      tabindex={mode === "tabs" ? (selected ? 0 : -1) : undefined}
      disabled={disabled || option.disabled}
      onkeydown={mode === "tabs" ? onTabKeydown : undefined}
      onclick={() => onchange(option.value)}
    >
      {option.label}
    </button>
  {/each}
</div>

<style>
  .seg-control {
    position: relative;
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    align-self: flex-start;
    justify-self: start;
    width: fit-content;
    gap: 3px;
    padding: 3px;
    max-width: 100%;
    border-radius: var(--r-control);
    background-color: var(--well);
    box-shadow: var(--press);
  }
  .seg-thumb {
    position: absolute;
    top: 0;
    left: 0;
    z-index: 0;
    border-radius: var(--r-thumb);
    pointer-events: none;
    opacity: 0;
    background-color: var(--card);
    background-image: var(--paper-image);
    background-size: var(--paper-tile-size);
    background-position: top left;
    background-blend-mode: multiply;
    border: 1px solid var(--surface-border-color);
    box-shadow: var(--raise-sm);
  }
  .seg-control.thumb-ready .seg-thumb {
    opacity: 1;
  }
  .seg-control.thumb-animate .seg-thumb {
    transition:
      transform 220ms cubic-bezier(0.22, 1, 0.36, 1),
      width 220ms cubic-bezier(0.22, 1, 0.36, 1),
      height 220ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  .seg-btn {
    position: relative;
    z-index: 1;
    flex: 0 0 auto;
    min-width: 0;
    min-height: 30px;
    padding: 6px 12px;
    border: 1px solid transparent;
    border-radius: var(--r-thumb);
    background: transparent;
    color: var(--soft-ink);
    font-size: 15px;
    font-weight: 450;
    line-height: 1.2;
    white-space: nowrap;
    transition:
      background-color var(--motion-control) linear,
      border-color var(--motion-control) linear,
      color var(--motion-control) linear,
      transform var(--motion-control) var(--ease-settle);
  }
  .compact .seg-btn {
    min-height: 26px;
    padding: 4px 10px;
    font-size: 14px;
  }
  .seg-btn:hover:not(:disabled):not([aria-pressed="true"]):not(
      [aria-selected="true"]
    ) {
    background-color: color-mix(in srgb, var(--card) 55%, transparent);
    color: var(--ink);
  }
  .seg-btn:active:not(:disabled) {
    transform: translateY(0.5px);
  }
  .seg-btn[aria-pressed="true"],
  .seg-btn[aria-selected="true"] {
    color: var(--ink);
    font-weight: 600;
  }
  .seg-control:not(.thumb-ready) .seg-btn[aria-pressed="true"],
  .seg-control:not(.thumb-ready) .seg-btn[aria-selected="true"] {
    background-color: var(--card);
    background-image: var(--paper-image);
    background-size: var(--paper-tile-size);
    background-position: top left;
    background-blend-mode: multiply;
    border-color: var(--surface-border-color);
    box-shadow: var(--raise-sm);
  }
  .seg-btn:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }
  :global(.studio[data-material="off"]) .seg-control,
  :global(.studio[data-material="off"]) .seg-thumb,
  :global(.studio[data-material="off"]) .seg-btn,
  :global(.studio[data-material="off"])
    .seg-btn:hover:not(:disabled):not([aria-pressed="true"]):not(
      [aria-selected="true"]
    ),
  :global(.studio[data-material="off"]) .seg-btn[aria-pressed="true"],
  :global(.studio[data-material="off"]) .seg-btn[aria-selected="true"] {
    background-image: none;
    box-shadow: none;
  }
  :global(.studio[data-material="off"]) .seg-thumb,
  :global(.studio[data-material="off"])
    .seg-control:not(.thumb-ready)
    .seg-btn[aria-pressed="true"],
  :global(.studio[data-material="off"])
    .seg-control:not(.thumb-ready)
    .seg-btn[aria-selected="true"] {
    border-color: var(--line);
  }
  :global(.studio[data-material="off"]) .seg-control {
    border: 1px solid var(--line);
  }
  @media (prefers-contrast: more) {
    .seg-control,
    .seg-thumb,
    .seg-btn,
    .seg-btn:hover:not(:disabled):not([aria-pressed="true"]):not(
        [aria-selected="true"]
      ),
    .seg-btn[aria-pressed="true"],
    .seg-btn[aria-selected="true"] {
      background-image: none;
      box-shadow: none;
      border: 1px solid currentColor;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .seg-btn,
    .seg-thumb {
      transition: none;
    }
    .seg-btn:active:not(:disabled) {
      transform: none;
    }
  }
</style>
