<script lang="ts">
  import symbolPine from "./marks/sonojot-symbol-compact-pine.svg";
  import symbolWhite from "./marks/sonojot-symbol-compact-white.svg";
  import lockupPine from "./marks/sonojot-horizontal-pine.svg";
  import lockupWhite from "./marks/sonojot-horizontal-white.svg";
  let { variant = "symbol", ink = "auto" }: { variant?: "symbol" | "lockup"; ink?: "auto" | "light" | "dark" } = $props();
</script>

<span
  class="sono-mark"
  class:lockup={variant === "lockup"}
  data-ink={ink}
  role="img"
  aria-label="SonoJot"
>
  <img
    class="pine"
    src={variant === "lockup" ? lockupPine : symbolPine}
    alt=""
    draggable="false"
  />
  <img
    class="white"
    src={variant === "lockup" ? lockupWhite : symbolWhite}
    alt=""
    draggable="false"
  />
</span>

<style>
  .sono-mark {
    display: inline-grid;
    width: 2.5rem;
    aspect-ratio: 1;
    flex: none;
  }
  .sono-mark.lockup {
    width: 13rem;
    aspect-ratio: 2400 / 650;
  }
  img {
    grid-area: 1 / 1;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .white {
    visibility: hidden;
  }
  :global([data-theme="slate"]) .sono-mark[data-ink="auto"] .pine,
  .sono-mark[data-ink="light"] .pine {
    visibility: hidden;
  }
  :global([data-theme="slate"]) .sono-mark[data-ink="auto"] .white,
  .sono-mark[data-ink="light"] .white {
    visibility: visible;
  }
</style>
