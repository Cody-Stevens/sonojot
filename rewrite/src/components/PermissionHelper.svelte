<script lang="ts">
  import { onMount } from "svelte";
  import appIcon from "../brand/marks/app/128x128.png";
  import { call } from "../lib/api";
  import {
    closePermissionAssistant,
    STALE_HINT_DELAY_MS,
    watchPermission,
  } from "../lib/permissions";
  import {
    errorMessage,
    permissionKinds,
    type PermissionKind,
    type PermissionStatus,
  } from "../lib/protocol";

  /** Floating macOS helper beside a System Settings privacy pane. Its copy comes
   * from the permission's Rust spec; the icon is a native file drag of the
   * running SonoJot app, for when it is not listed yet. */
  const requested = new URLSearchParams(window.location.search).get("kind");
  const kind: PermissionKind =
    permissionKinds.find((value) => value === requested) ?? "accessibility";
  let status = $state<PermissionStatus | null>(null);
  let error = $state("");
  let showStaleHint = $state(false);

  onMount(() => {
    const stop = watchPermission(
      kind,
      (value) => {
        status = value;
      },
      (failure) => {
        error = errorMessage(failure);
      },
    );
    // macOS hides whether an old entry is switched on, so offer the fix once
    // the user has had time to flip the switch without effect.
    const timer = setTimeout(() => {
      showStaleHint = true;
    }, STALE_HINT_DELAY_MS);
    return () => {
      stop();
      clearTimeout(timer);
    };
  });

  function drag(event: DragEvent): void {
    // WebKit's own image drag would carry a PNG; replace it with the app bundle.
    event.preventDefault();
    error = "";
    void call("permission_drag").catch((failure: unknown) => {
      error = errorMessage(failure);
    });
  }
  async function close(): Promise<void> {
    try {
      await closePermissionAssistant(kind);
    } catch (failure: unknown) {
      error = errorMessage(failure);
    }
  }
</script>

<div class="stock helper-sheet" role="dialog" aria-labelledby="helper-title">
  <img
    class="helper-icon"
    src={appIcon}
    alt="SonoJot app icon. Drag it into the list in System Settings."
    title="Drag into the list"
    draggable="true"
    ondragstart={drag}
  />
  <div class="helper-copy">
    <strong id="helper-title" data-text-surface="emboss"
      >{status?.title ?? ""}</strong
    >
    {#if error}<p class="danger-text" role="alert">{error}</p>
    {:else if status}<p data-text-surface="flat">{status.body}</p>
      {#if showStaleHint}<p data-text-surface="flat" aria-live="polite">
          {status.staleHint}
        </p>{/if}{/if}
  </div>
  <button class="button quiet small" onclick={close}>Close</button>
</div>

<style>
  .helper-sheet {
    display: flex;
    align-items: center;
    gap: 16px;
    height: 100dvh;
    padding: 14px 18px 14px 14px;
    border-radius: 17px;
    overflow: hidden;
    box-shadow:
      inset calc(1px * var(--material-light-x, 1))
        calc(1px * var(--material-light-y, 1)) var(--surface-highlight-blur)
        var(--surface-highlight-spread) var(--surface-highlight-color),
      inset calc(-1px * var(--material-light-x, 1))
        calc(-1px * var(--material-light-y, 1)) var(--edge);
  }
  .helper-icon {
    flex: none;
    width: 72px;
    height: 72px;
    cursor: grab;
    -webkit-user-drag: element;
    transition: transform var(--motion-control) var(--ease-settle);
  }
  .helper-icon:hover {
    transform: translateY(-1px);
  }
  .helper-icon:active {
    cursor: grabbing;
  }
  .helper-copy {
    flex: 1;
    min-width: 0;
  }
  .helper-copy strong {
    display: block;
    font-size: 17px;
    line-height: 1.3;
  }
  .helper-copy p {
    margin: 4px 0 0;
    font-size: 14px;
    line-height: 1.4;
    color: var(--soft-ink);
  }
  .helper-copy p.danger-text {
    color: var(--danger);
  }
  @media (prefers-reduced-motion: reduce) {
    .helper-icon {
      transition: none;
    }
    .helper-icon:hover {
      transform: none;
    }
  }
</style>
