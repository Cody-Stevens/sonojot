<script lang="ts">
  import { onMount } from "svelte";
  import SettingRow from "./SettingRow.svelte";
  import {
    MAC_PASTE_COPY,
    offerReset,
    openPermissionAssistant,
    permissionState,
    resetPermission,
    watchPermission,
  } from "../lib/permissions";
  import { errorMessage, type PermissionStatus } from "../lib/protocol";

  /** macOS Accessibility access for automatic paste, with the drag helper.
   * After one try without access, it also offers to reset an entry left by
   * an older build, which looks switched on but grants nothing. */
  let { variant = "onboarding" }: { variant?: "onboarding" | "settings" } =
    $props();
  let status = $state<PermissionStatus | null>(null);
  let busy = $state(false);
  let asked = $state(false);
  let error = $state("");
  const access = $derived(permissionState(status));
  const showReset = $derived(
    offerReset(status, asked || access === "assisting"),
  );

  async function run(action: () => Promise<PermissionStatus>): Promise<void> {
    busy = true;
    error = "";
    try {
      status = await action();
    } catch (failure: unknown) {
      error = errorMessage(failure);
    } finally {
      busy = false;
      asked = true;
    }
  }
  const grant = (): Promise<void> =>
    run(() => openPermissionAssistant("accessibility"));
  const reset = (): Promise<void> =>
    run(() => resetPermission("accessibility"));
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
  {#if showReset}<div class="reset-offer">
      <p class="field-help" data-text-surface="flat">
        {MAC_PASTE_COPY.resetHelp}
      </p>
      <button class="button small" disabled={busy} onclick={reset}
        >{MAC_PASTE_COPY.reset}</button
      >
    </div>{/if}
  {#if error}<p class="field-help danger-text" role="alert">{error}</p>{/if}
{:else}
  <div class="mac-paste" aria-live="polite">
    {#if access === "granted"}<p data-text-surface="flat">
        {MAC_PASTE_COPY.on}
      </p>
    {:else if access !== "unknown"}<p data-text-surface="flat">
        {MAC_PASTE_COPY.needed}
      </p>
      <div class="actions">
        <button class="button" disabled={busy} onclick={grant}
          >{MAC_PASTE_COPY.grant}</button
        >{#if showReset}<button class="button" disabled={busy} onclick={reset}
            >{MAC_PASTE_COPY.reset}</button
          >{/if}
      </div>
      {#if access === "assisting"}<p
          class="field-help"
          data-text-surface="flat"
        >
          {MAC_PASTE_COPY.assistant}
        </p>{/if}
      {#if showReset}<p class="field-help" data-text-surface="flat">
          {MAC_PASTE_COPY.resetHelp}
        </p>{/if}
    {/if}
    {#if error}<p class="field-help danger-text" role="alert">{error}</p>{/if}
  </div>
{/if}

<style>
  .assist {
    margin: 6px 0 12px;
  }
  .reset-offer {
    margin: 6px 0 12px;
  }
  .reset-offer .field-help {
    margin: 0 0 6px;
  }
  .mac-paste > .actions,
  .mac-paste > .field-help {
    margin-top: 9px;
  }
</style>
