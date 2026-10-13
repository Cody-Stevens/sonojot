<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../lib/app.svelte";
  let dialog: HTMLDialogElement;
  let opened = $state(false);
  let closing = $state(false);
  let outcome: boolean | "alternative" = false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  function complete(): void {
    if (!closing) return;
    clearTimeout(timer);
    dialog.close();
    app.finishConfirmation(outcome);
  }

  function dismiss(confirmed: boolean | "alternative"): void {
    if (closing) return;
    closing = true;
    outcome = confirmed;
    opened = false;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      complete();
    } else {
      // Complete even if the window stops painting before transitionend.
      timer = setTimeout(complete, 160);
    }
  }

  onMount(() => {
    dialog.showModal();
    // Establish the closed frame before the next paint starts its entry.
    dialog.getBoundingClientRect();
    const frame = requestAnimationFrame(() => {
      opened = true;
    });
    return () => {
      cancelAnimationFrame(frame);
      clearTimeout(timer);
    };
  });
</script>

<dialog
  bind:this={dialog}
  class="stock confirmation"
  class:is-open={opened}
  aria-labelledby="confirmation-title"
  aria-describedby="confirmation-detail"
  oncancel={(event) => {
    event.preventDefault();
    dismiss(false);
  }}
  ontransitionend={(event) => {
    if (event.target === dialog && event.propertyName === "opacity") complete();
  }}
>
  {#if app.confirmation}<h2 id="confirmation-title" data-text-surface="emboss">
      {app.confirmation.title}
    </h2>
    <p id="confirmation-detail">{app.confirmation.detail}</p>
    <div class="actions">
      <button class="button" disabled={closing} onclick={() => dismiss(false)}
        >{app.confirmation.cancelLabel ??
          (app.confirmation.danger ? "Cancel" : "Keep working")}</button
      >
      {#if app.confirmation.alternativeLabel}
        <button
          class="button"
          disabled={closing}
          onclick={() => dismiss("alternative")}
          >{app.confirmation.alternativeLabel}</button
        >
      {/if}
      <button
        class="button"
        class:danger={app.confirmation.danger}
        class:primary={!app.confirmation.danger}
        disabled={closing}
        onclick={() => dismiss(true)}>{app.confirmation.label}</button
      >
    </div>{/if}
</dialog>
