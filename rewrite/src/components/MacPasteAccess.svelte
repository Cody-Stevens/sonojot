<script lang="ts">
  import { onMount } from "svelte";
  import SettingRow from "./SettingRow.svelte";
  import {
    MAC_PASTE_COPY,
    openPermissionAssistant,
    permissionState,
    watchPermission,
  } from "../lib/permissions";
  import { errorMessage, type PermissionStatus } from "../lib/protocol";

  /** macOS Accessibility access for automatic paste, with the drag helper. */
  let { variant = "onboarding" }: { variant?: "onboarding" | "settings" } =
    $props();
  let status = $state<PermissionStatus | null>(null);
  let busy = $state(false);
  let error = $state("");
  const access = $derived(permissionState(status));

  async function grant(): Promise<void> {
    busy = true;
    error = "";
    try {
      status = await openPermissionAssistant("accessibility");
    } catch (failure: unknown) {
      error = errorMessage(failure);
    } finally {
      busy = false;
    }
  }
  onMount(() =>
    watchPermission(
      "accessibility",
      (value) => {
        status = value;
      },
      (failure) => {
        error = errorMessage(failure);
      },
    ),
  );
</script>

{#if variant === "settings"}
  {#if access !== "unknown"}<SettingRow
      label={MAC_PASTE_COPY.rowLabel}
      help={access === "granted"
        ? MAC_PASTE_COPY.rowOn
        : MAC_PASTE_COPY.rowNeeded}
      >{#if access !== "granted"}<button
          class="button small"
          disabled={busy}
          onclick={grant}>{MAC_PASTE_COPY.grant}</button
        >{/if}</SettingRow
    >{/if}
  {#if access === "assisting"}<p
      class="field-help assist"
      data-text-surface="flat"
      role="status"
    >
      {MAC_PASTE_COPY.assistant}
    </p>{/if}
  {#if error}<p class="field-help danger-text" role="alert">{error}</p>{/if}
{:else}
  <div class="mac-paste" aria-live="polite">
    {#if access === "granted"}<p data-text-surface="flat">
        {MAC_PASTE_COPY.on}
      </p>
    {:else if access !== "unknown"}<p data-text-surface="flat">
        {MAC_PASTE_COPY.needed}
      </p>
      <button class="button" disabled={busy} onclick={grant}
        >{MAC_PASTE_COPY.grant}</button
      >
      {#if access === "assisting"}<p
          class="field-help"
          data-text-surface="flat"
        >
          {MAC_PASTE_COPY.assistant}
        </p>{/if}
    {/if}
    {#if error}<p class="field-help danger-text" role="alert">{error}</p>{/if}
  </div>
{/if}

<style>
  .assist {
    margin: 6px 0 12px;
  }
  .mac-paste > .button,
  .mac-paste > .field-help {
    margin-top: 9px;
  }
</style>
