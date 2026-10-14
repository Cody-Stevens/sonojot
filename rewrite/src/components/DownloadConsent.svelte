<script lang="ts">
  import { onMount } from "svelte";
  import { call, native } from "../lib/api";
  import { downloadConsent } from "../lib/download-consent.svelte";
  import { errorMessage } from "../lib/protocol";
  import { scrollEdges } from "../lib/scroll-edges";

  let dialog: HTMLDialogElement;
  let agreed = $state<string[]>([]);
  let saving = $state(false);
  let error = $state("");
  const allAgreed = $derived(
    downloadConsent.groups.every((group) => agreed.includes(group.id)),
  );
  onMount(() => {
    dialog.showModal();
    return () => {
      downloadConsent.cancel();
    };
  });
  async function accept(): Promise<void> {
    if (!allAgreed || saving) return;
    saving = true;
    error = "";
    try {
      await downloadConsent.accept();
    } catch (failure: unknown) {
      error = errorMessage(failure);
    } finally {
      saving = false;
    }
  }
  async function openLink(
    event: MouseEvent,
    group: string,
    index: number,
  ): Promise<void> {
    if (!native) return;
    event.preventDefault();
    try {
      await call("open_download_license", { group, index });
    } catch (failure: unknown) {
      error = errorMessage(failure);
    }
  }
</script>

<dialog
  bind:this={dialog}
  class="stock confirmation is-open license-dialog"
  aria-labelledby="download-terms-title"
  oncancel={(event) => {
    event.preventDefault();
    if (!saving) downloadConsent.cancel();
  }}
>
  <h2 id="download-terms-title" data-text-surface="emboss">
    Before downloading
  </h2>
  <div class="license-scroll" use:scrollEdges>
    {#each downloadConsent.groups as group (group.id)}
      <section>
        <h3 data-text-surface="emboss">{group.title}</h3>
        <p>{group.summary}</p>
        <ul data-text-surface="flat">
          {#each group.components as component}<li>
              {component.name} · {component.version}
            </li>{/each}
        </ul>
        <ul class="links">
          {#each group.links as link, index}
            <li>
              <a
                href={link.url}
                target="_blank"
                rel="noopener noreferrer"
                onclick={(event) => openLink(event, group.id, index)}
                >{link.label} ↗</a
              >
            </li>
          {/each}
        </ul>
        <label class="agreement"
          ><input
            type="checkbox"
            bind:group={agreed}
            value={group.id}
            disabled={saving}
          />
          <span>{group.checkbox}</span></label
        >
      </section>
    {/each}
    <p class="local-note" data-text-surface="flat">
      Your acknowledgment is saved only on this device. Package agreements and
      third-party notices are also available in About → Credits and licenses.
    </p>
    {#if error}<p role="alert">{error}</p>{/if}
  </div>
  <div class="actions">
    <button
      class="button"
      disabled={saving}
      onclick={() => downloadConsent.cancel()}>Cancel</button
    >
    <button
      class="button primary"
      disabled={!allAgreed || saving}
      onclick={accept}>{saving ? "Saving…" : "Agree and continue"}</button
    >
  </div>
</dialog>

<style>
  .license-dialog {
    width: min(720px, calc(100vw - 36px));
    max-height: calc(100dvh - 36px);
  }
  .license-scroll {
    overflow-y: auto;
    max-height: calc(100dvh - 245px);
    padding-right: 9px;
  }
  section + section {
    border-top: 1px solid var(--line);
    margin-top: 27px;
    padding-top: 18px;
  }
  h3 {
    margin: 0 0 9px;
  }
  p {
    margin: 9px 0;
  }
  ul {
    padding-left: 27px;
  }
  .links {
    list-style: none;
    padding-left: 0;
  }
  a {
    color: inherit;
    text-underline-offset: 4px;
  }
  .agreement {
    display: flex;
    align-items: start;
    gap: 12px;
    margin: 18px 0;
  }
  input {
    flex-shrink: 0;
    width: 20px;
    height: 20px;
    margin-top: 4px;
    accent-color: var(--ink);
  }
  .local-note {
    color: var(--soft-ink);
  }
</style>
