<script lang="ts">
  import { formatBytes } from "../../lib/model-library";
  let { bytes, total }: { bytes: number | null; total: number | null } =
    $props();
</script>

<span
  class="memory"
  title="Estimated system RAM; bar shows the share of total RAM"
  data-text-surface="flat"
>
  <span class="track" aria-hidden="true"
    ><span
      style:width={`${bytes !== null && total ? Math.min(100, (bytes / total) * 100) : 0}%`}
    ></span></span
  >
  <span
    >{bytes === null
      ? "Not measured"
      : bytes === 0
        ? "0 GB"
        : formatBytes(bytes)}</span
  >
</span>

<style>
  .memory {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 155px;
    flex: 0 0 155px;
    font-size: 13px;
    color: var(--soft-ink);
  }
  .track {
    width: 75px;
    height: 7px;
    flex: 0 0 75px;
    border-radius: 2px;
    background: var(--well);
    box-shadow: var(--press);
    overflow: hidden;
  }
  .track > span {
    display: block;
    height: 100%;
    background: var(--dye);
    opacity: 0.65;
  }
  :global(.studio[data-material="off"]) .track {
    box-shadow: none;
  }
</style>
