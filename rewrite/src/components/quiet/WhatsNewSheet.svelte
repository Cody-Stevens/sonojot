<script lang="ts">
  import { app } from "../../lib/app.svelte";
  import { changelogSummary, releaseDate } from "../../lib/changelog";
  import { whatsNew } from "../../lib/whats-new.svelte";
  import UpdateAction from "../UpdateAction.svelte";
  import Sheet from "./Sheet.svelte";

  const offer = $derived(whatsNew.mode === "offer");
  const notes = $derived(offer ? whatsNew.offer : whatsNew.installed);
  const summary = $derived(changelogSummary(notes));
  const offered = $derived(
    ["available", "ready"].includes(app.snapshot.update.status.state),
  );
</script>

<Sheet title="What's new" onclose={() => void whatsNew.close()}>
  {#if offer && whatsNew.loading}<p role="status" data-text-surface="flat">
      Getting the notes…
    </p>
  {:else if offer && whatsNew.error}<p role="status">{whatsNew.error}</p>
  {:else if !notes?.releases.length}<p data-text-surface="flat">
      {offer
        ? "This update has no notes yet."
        : "There are no notes for this version."}
    </p>
  {:else}
    {#if summary}<p class="changelog-summary" data-text-surface="flat">
        {summary}
      </p>{/if}
    <div class="changelog">
      {#each notes.releases as release (release.version)}
        <section aria-labelledby={`release-${release.version}`}>
          <header>
            <h3 id={`release-${release.version}`} data-text-surface="emboss">
              SonoJot {release.version}
            </h3>
            {#if releaseDate(release.date)}<small data-text-surface="flat"
                >{releaseDate(release.date)}</small
              >{/if}
          </header>
          <ul>
            {#each release.notes as note}<li>{note}</li>{/each}
          </ul>
        </section>
      {/each}
    </div>
  {/if}
  <div class="actions changelog-actions">
    {#if offer && offered}<UpdateAction />{/if}
    <button
      type="button"
      class="button"
      class:quiet={offer && offered}
      onclick={() => void whatsNew.close()}>Close</button
    >
  </div>
</Sheet>

<style>
  .changelog-summary {
    margin: 0 0 18px;
  }
  .changelog {
    display: grid;
    gap: 27px;
  }
  section header {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    column-gap: 18px;
    row-gap: 4px;
  }
  h3 {
    margin: 0;
    font-size: 1.05rem;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  small {
    color: var(--soft-ink);
    font-size: 15px;
  }
  ul {
    margin: 9px 0 0;
    padding-left: 1.2em;
    display: grid;
    gap: 9px;
  }
  li {
    line-height: 1.45;
    overflow-wrap: anywhere;
  }
  .changelog-actions {
    margin-top: 27px;
  }
</style>
