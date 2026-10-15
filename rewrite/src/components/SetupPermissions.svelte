<script lang="ts">
  // Setup's macOS Permissions step: microphone, paste and system audio in one
  // place, each row the same anatomy (name, why, status, one action) and the
  // same flow: macOS's own prompt when it never asked, otherwise the System
  // Settings pane with the permission helper (permissions.rs). Nothing here
  // holds Continue; each permission can also be allowed later.
  import { onMount } from "svelte";
  import Icon from "./Icon.svelte";
  import { call } from "../lib/api";
  import { app } from "../lib/app.svelte";
  import {
    SETUP_PERMISSIONS,
    openPermissionAssistant,
    permissionLabel,
    permissionState,
    watchPermission,
  } from "../lib/permissions";
  import {
    errorMessage,
    type PermissionKind,
    type PermissionStatus,
  } from "../lib/protocol";
  import { fixSystemAudio, watchSystemAudio } from "../lib/system-audio.svelte";

  let statuses = $state<Partial<Record<PermissionKind, PermissionStatus>>>({});
  let busy = $state<PermissionKind | null>(null);
  let error = $state("");
  // "Allow all" asks for each missing permission in turn. The next one starts
  // once the current one is allowed, its helper closes, or SonoJot comes back
  // to the front after macOS's own prompt.
  let queue: PermissionKind[] = [];
  let current = $state<PermissionKind | null>(null);
  let sawHelper = false;
  let askedAt = 0;

  const rows = $derived(
    SETUP_PERMISSIONS.filter(
      (row) =>
        row.kind !== "system_audio" ||
        app.snapshot.capabilities.systemAudioStatus !== "unsupported",
    ),
  );
  const stateOf = (kind: PermissionKind) =>
    permissionState(statuses[kind] ?? null);
  const missing = $derived(
    rows
      .filter((row) => stateOf(row.kind) !== "granted")
      .map((row) => row.kind),
  );

  function update(kind: PermissionKind, status: PermissionStatus): void {
    statuses[kind] = status;
    if (kind !== current) return;
    if (status.granted) next();
    else if (status.assistantOpen) sawHelper = true;
    else if (sawHelper) next();
  }

  function next(): void {
    sawHelper = false;
    current = queue.shift() ?? null;
    if (current) void allow(current);
  }

  async function allow(kind: PermissionKind): Promise<void> {
    busy = kind;
    error = "";
    askedAt = Date.now();
    try {
      const status =
        kind === "system_audio"
          ? await fixSystemAudio()
          : await openPermissionAssistant(kind);
      if (status) update(kind, status);
    } catch (failure: unknown) {
      error = errorMessage(failure);
      if (kind === current) next();
    } finally {
      busy = null;
    }
  }

  function allowAll(): void {
    queue = [...missing];
    next();
  }

  // Granting system audio here means the user wants both sides recorded.
  let wasFixable = false;
  $effect(() => {
    const status = app.snapshot.capabilities.systemAudioStatus;
    if (status === "notDetermined" || status === "denied") wasFixable = true;
    else if (
      wasFixable &&
      status === "ready" &&
      app.snapshot.settings.meetingSource === "microphone"
    ) {
      wasFixable = false;
      void app.saveSettings({ meetingSource: "both" });
    }
  });

  onMount(() => {
    const stops = SETUP_PERMISSIONS.map(({ kind }) =>
      watchPermission(
        kind,
        (status) => update(kind, status),
        (failure) => {
          error = errorMessage(failure);
        },
      ),
    );
    stops.push(watchSystemAudio(1000));
    // macOS's own prompts answer without an event; read every status again
    // while something is missing, and move "Allow all" on when SonoJot is
    // back in front without a helper open.
    const timer = setInterval(() => {
      if (document.visibilityState !== "visible") return;
      for (const kind of missing) void readStatus(kind);
    }, 1500);
    const onFocus = (): void => {
      if (
        current &&
        busy === null &&
        !statuses[current]?.assistantOpen &&
        Date.now() - askedAt > 800
      )
        next();
    };
    window.addEventListener("focus", onFocus);
    return () => {
      for (const stop of stops) stop();
      clearInterval(timer);
      window.removeEventListener("focus", onFocus);
      queue = [];
    };
  });

  async function readStatus(kind: PermissionKind): Promise<void> {
    try {
      update(kind, await call("permission_status", { kind }));
    } catch {
      // The watcher reports errors; a missed poll is retried.
    }
  }
</script>

<div class="setup-permissions">
  <div class="permissions-head">
    <p data-text-surface="flat">
      {missing.length === 0
        ? "Everything is allowed."
        : "Allow these now, or skip any and allow it later."}
    </p>
    {#if missing.length > 1}<button
        class="button primary"
        disabled={busy !== null || current !== null || !app.native}
        onclick={allowAll}>Allow all</button
      >{/if}
  </div>
  <ul class="permission-rows" aria-label="Permissions">
    {#each rows as row (row.kind)}
      {@const rowState = stateOf(row.kind)}
      <li class="permission-row" class:granted={rowState === "granted"}>
        <div class="permission-copy">
          <h2 data-text-surface="emboss">{row.name}</h2>
          <p data-text-surface="flat">{row.why}</p>
        </div>
        <p
          class="permission-state"
          role="status"
          aria-label={`${row.name} permission`}
        >
          {#if rowState === "granted"}<Icon name="check" size={18} />{/if}<span
            data-text-surface="flat">{permissionLabel(rowState)}</span
          >
        </p>
        <div class="permission-action">
          {#if rowState !== "granted" && rowState !== "unknown"}<button
              class="button"
              disabled={busy !== null || !app.native}
              aria-label={`Allow ${row.name}`}
              onclick={() => allow(row.kind)}
              >{rowState === "assisting" ? "Open again" : "Allow"}</button
            >{/if}
        </div>
      </li>
    {/each}
  </ul>
  {#if error}<p class="flow-error" role="alert">{error}</p>{/if}
</div>

<style>
  /* The Downloads step's head: one line, its action on the right. */
  .permissions-head {
    display: flex;
    align-items: center;
    gap: 18px 27px;
    min-height: 42px;
    margin-bottom: 9px;
  }
  .permissions-head p {
    flex: 1;
    margin: 0;
    line-height: 1.5;
  }
  .permissions-head .button {
    flex: none;
  }
  .permission-rows {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  /* One row anatomy for every permission: name and why, status, action. */
  .permission-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(6.5em, auto);
    align-items: center;
    gap: 6px 18px;
    padding: 12px 0;
  }
  .permission-row + .permission-row {
    border-top: 1px solid var(--line);
  }
  .permission-copy h2 {
    margin: 0;
    font-size: 18px;
    font-weight: inherit;
    line-height: 1.35;
  }
  .permission-copy p,
  .permission-state {
    margin: 0;
    font-size: 15px;
    line-height: 1.4;
    color: var(--soft-ink);
  }
  .permission-state {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 9px;
  }
  /* Allowed reads like the Models page's In use: a dyed check, plain text. */
  .granted .permission-state {
    color: var(--dye);
  }
  .granted .permission-state span {
    color: var(--ink);
  }
  .permission-action {
    display: flex;
    justify-content: flex-end;
    white-space: nowrap;
  }
  .flow-error {
    margin: 12px 0 0;
    color: var(--danger);
  }
  @media (max-height: 700px) {
    .permission-row {
      padding: 8px 0;
    }
  }
</style>
