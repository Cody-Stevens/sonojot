<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  function open(event: MouseEvent, url: string): void {
    event.preventDefault();
    if (event.button > 1) return;
    void app.perform("legal-link", () => call("open_legal_link", { url }));
  }
</script>

<p class="body-copy spaced legal-links" data-text-surface="flat">
  {#each [{ name: "Privacy Notice", url: "https://sonojot.com/privacy" }, { name: "Trademark Policy", url: "https://sonojot.com/trademarks" }] as link}
    <a
      href={link.url}
      onclick={(event) => open(event, link.url)}
      onauxclick={(event) => open(event, link.url)}>{link.name}</a
    >
  {/each}
</p>

<style>
  .legal-links {
    display: flex;
    flex-wrap: wrap;
    gap: 9px 18px;
  }
</style>
