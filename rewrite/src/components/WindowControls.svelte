<script lang="ts">
  import type { WindowAction } from "../lib/window";
  let {
    maximized,
    disabled,
    onaction,
  }: {
    maximized: boolean;
    disabled: boolean;
    onaction: (action: WindowAction) => void;
  } = $props();
</script>

<div class="window-controls" aria-label="Window controls" data-no-window-drag>
  <button
    type="button"
    class="window-button"
    aria-label="Minimize window"
    title="Minimize window"
    data-help="Minimize window"
    {disabled}
    onclick={() => onaction("minimize")}
    ><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 8h10" /></svg
    ></button
  >
  <button
    type="button"
    class="window-button"
    aria-label={maximized ? "Restore window" : "Maximize window"}
    title={maximized ? "Restore window" : "Maximize window"}
    data-help={maximized ? "Restore window" : "Maximize window"}
    {disabled}
    onclick={() => onaction("toggleMaximize")}
    ><svg viewBox="0 0 16 16" aria-hidden="true"
      >{#if maximized}<path d="M5 3h8v8M3 5h8v8H3Z" />{:else}<path
          d="M3 3h10v10H3Z"
        />{/if}</svg
    ></button
  >
  <button
    type="button"
    class="window-button window-close"
    aria-label="Close window"
    title="Close window"
    data-help="Close window"
    {disabled}
    onclick={() => onaction("close")}
    ><svg viewBox="0 0 16 16" aria-hidden="true"
      ><path d="m4 4 8 8m0-8-8 8" /></svg
    ></button
  >
</div>
