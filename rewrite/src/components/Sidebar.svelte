<script lang="ts">
  import Icon from "./Icon.svelte";
  import UpdateButton from "./UpdateButton.svelte";
  import { app } from "../lib/app.svelte";
  import { navigation, type Page, type SetupPage } from "../lib/navigation";
  import { scrollEdges } from "../lib/scroll-edges";

  let {
    page,
    onnavigate = () => {},
    preview = false,
  }: {
    page: Page | SetupPage;
    onnavigate?: (page: Page | SetupPage) => void;
    /** A static picture of the sidebar for the setup tour. */
    preview?: boolean;
  } = $props();
  const id = $props.id();
</script>

<aside class="paper-nav" use:scrollEdges>
  <nav aria-label="Main navigation">
    {#snippet navItem(item: (typeof navigation)[number])}<button
        type="button"
        class="nav-item"
        class:selected={page === item.id}
        aria-current={page === item.id ? "page" : undefined}
        data-nav={item.id}
        onclick={() => onnavigate(item.id)}
        >{#if item.id === "setup"}<svg
            width="24"
            height="24"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.55"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
            ><path d="m12 3 9 5-9 5-9-5 9-5Zm-9 9 9 5 9-5M3 16l9 5 9-5" /></svg
          >
        {:else}<Icon
            name={item.id === "report" ? "meetings" : item.id}
          />{/if}<span>{item.label}</span>{#if item.id === "report"}<small
            class="alpha-marker"
            data-text-surface="flat">Alpha</small
          >{/if}
        {#if item.id === "meetings" && app.snapshot.meetingRecording && !preview}<span
            class="nav-recording"
            aria-label="Recording"
          ></span>{/if}</button
      >{/snippet}
    <div class="nav-primary">
      {#each navigation.slice(0, 2) as item}{@render navItem(item)}{/each}
      {#if app.snapshot.capabilities.platform === "linux"}
        {@render navItem({ id: "report", label: "Feedback" })}
      {/if}
    </div>
    <div class="nav-settings" role="group" aria-labelledby={`${id}-settings`}>
      <span
        id={`${id}-settings`}
        class="nav-group-label"
        data-text-surface="flat">Settings</span
      >
      {#each navigation.slice(2) as item}
        {#if item.id !== "report"}
          {@render navItem(item)}
        {/if}
      {/each}
    </div>
  </nav>
  <UpdateButton {preview} />
</aside>

<style>
  .alpha-marker {
    margin-inline-start: auto;
    font-size: 0.7rem;
    color: var(--soft-ink);
  }
  .selected .alpha-marker {
    color: inherit;
  }
</style>
