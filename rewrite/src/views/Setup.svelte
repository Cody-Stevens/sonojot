<script lang="ts">
  import { ensureDownloadConsent } from "../lib/download-consent.svelte";
  import { onDestroy, untrack } from "svelte";
  import AiSettings from "../components/AiSettings.svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import ErrorBanner from "../components/ErrorBanner.svelte";
  import ModelSetup from "../components/ModelSetup.svelte";
  import { app } from "../lib/app.svelte";
  import {
    call,
    chooseModelsFolder,
    chooseConverter,
    subscribeMediaSupport,
  } from "../lib/api";
  import { errorMessage } from "../lib/protocol";
  import type { ErrorAction } from "../lib/errors";
  import { selectSetup, setupLabel, type SetupStatus } from "../lib/setup";
  import type { ModelRole } from "../lib/model-library";
  import { installing } from "../lib/model-library";
  import { library } from "../lib/model-library.svelte";
  import { profiles } from "../lib/setup-profiles.svelte";

  let {
    onnavigate,
    focusRole = null,
  }: {
    onnavigate?: (page: "shortcuts" | "dictation") => void;
    focusRole?: ModelRole | null;
  } = $props();
  let status = $state<SetupStatus | null>(null);
  let error = $state("");
  let pending = $state<
    "refresh" | "models" | "open-models" | "converter" | "install" | null
  >(null);
  let request = 0;
  const actionPending = $derived(pending !== null && pending !== "refresh");
  let unlistenMedia: (() => void) | undefined;
  let destroyed = false;
  onDestroy(() => {
    request += 1;
    destroyed = true;
    unlistenMedia?.();
  });
  if (app.native)
    void subscribeMediaSupport(() => void refresh()).then((stop) => {
      if (destroyed) stop();
      else unlistenMedia = stop;
    });
  const control = $derived(
    selectSetup({ status, snapshot: app.snapshot, native: app.native }),
  );
  const modelsLocked = $derived(
    app.busy("settings") ||
      profiles.busy ||
      !!library.data?.models.some(
        (model) =>
          installing(model) ||
          ["loading", "busy", "unloading"].includes(model.state),
      ),
  );
  const mediaNeedsSetup = $derived(
    status !== null &&
      (status.mediaImport.state === "missing" ||
        status.mediaImport.state === "failed" ||
        status.mediaImport.state === "unsupported"),
  );
  // The backend reports unsupported where it cannot install its own converter.
  const canInstallMedia = $derived(
    mediaNeedsSetup && status?.mediaImport.state !== "unsupported",
  );
  // A background install finishes on its own; check back until it does.
  $effect(() => {
    if (
      status?.mediaImport.state !== "loading" &&
      ![status?.speech, status?.cleanup, status?.mediaImport].some(
        (capability) => capability?.state === "checking",
      )
    )
      return;
    const timer = setInterval(() => void refresh(), 3_000);
    return () => clearInterval(timer);
  });
  // A primitive key prevents fresh snapshot objects and meter updates from
  // invalidating an in-flight check. Never feed get_snapshot back into this effect.
  const readinessKey = $derived(
    [
      app.ready,
      app.snapshot.state,
      app.snapshot.settings.microphone,
      app.snapshot.settings.asrModel,
      app.snapshot.capabilities.globalShortcut,
      app.snapshot.interimText.length > 0,
    ].join("|"),
  );
  $effect(() => {
    if (readinessKey.startsWith("true|") && app.native)
      untrack(() => {
        void refresh();
      });
  });

  async function refresh(refreshDevices = false): Promise<void> {
    if (!app.native || pending) return;
    const current = ++request;
    pending = "refresh";
    try {
      if (refreshDevices) app.apply(await call("get_snapshot"));
      const next = await call("get_setup");
      if (current === request) {
        status = next;
        error = "";
      }
    } catch (failure: unknown) {
      if (current === request) {
        status = null;
        error = errorMessage(failure);
      }
    } finally {
      if (current === request) pending = null;
    }
  }

  async function locate(kind: "models" | "converter"): Promise<void> {
    if (
      !control.canConfigure ||
      (kind === "models" && modelsLocked) ||
      actionPending
    )
      return;
    const current = ++request;
    pending = kind;
    error = "";
    try {
      const path =
        kind === "models"
          ? await chooseModelsFolder()
          : await chooseConverter();
      if (path === null) return;
      const next =
        kind === "models"
          ? await call("locate_models", { path })
          : await call("locate_converter", { path });
      if (current === request) {
        status = next;
        app.apply(await call("get_snapshot"));
        app.notice =
          kind === "models"
            ? "Models verified and located."
            : "Media converter located.";
      }
    } catch (failure: unknown) {
      if (current === request) error = errorMessage(failure);
    } finally {
      if (current === request) pending = null;
    }
  }

  async function openModels(): Promise<void> {
    if (!app.native || actionPending) return;
    const current = ++request;
    pending = "open-models";
    error = "";
    try {
      await call("open_path", { kind: "models" });
    } catch (failure: unknown) {
      if (current === request) error = errorMessage(failure);
    } finally {
      if (current === request) pending = null;
    }
  }

  async function installMedia(): Promise<void> {
    if (!(await ensureDownloadConsent(["ffmpeg"]))) return;
    if (!control.canConfigure || actionPending) return;
    const current = ++request;
    pending = "install";
    error = "";
    try {
      const next = await call("install_media_support");
      if (current === request) {
        status = next;
        app.notice = "Media support installed.";
      }
    } catch (failure: unknown) {
      if (current === request) error = errorMessage(failure);
    } finally {
      if (current === request) pending = null;
    }
  }

  function repair(action: ErrorAction): void {
    if (action === "install-media") void installMedia();
    else if (action === "setup") void locate("models");
    else if (action === "converter") void locate("converter");
    else if (action === "shortcuts") onnavigate?.("shortcuts");
    else void refresh(action === "microphone");
  }
</script>

<header class="page-heading">
  <h1 class="text-grain" data-text-surface="emboss">Models</h1>
  <p data-text-surface="flat">
    Choose how SonoJot balances accuracy and speed on this computer.
  </p>
</header>

{#if error}<ErrorBanner
    {error}
    onaction={repair}
    ondismiss={() => {
      error = "";
    }}
  />{/if}

<ModelSetup {focusRole}>
  {#snippet storage()}
    <SettingRow label="Models folder">
      <div class="actions">
        <button
          class="button quiet storage-action"
          disabled={!control.canConfigure || modelsLocked || actionPending}
          onclick={() => locate("models")}
          >{pending === "models" ? "Checking folder…" : "Change…"}</button
        >
        <button
          class="button quiet storage-action"
          disabled={!app.native || actionPending}
          onclick={openModels}
          >{pending === "open-models" ? "Opening…" : "Open"}</button
        >
      </div>
    </SettingRow>
    <SettingRow
      label="Audio and video import"
      help="Needed to import recordings."
    >
      {#if pending === "install"}<span
          class="setup-state"
          data-text-surface="flat">Installing media support…</span
        >
      {:else if mediaNeedsSetup}<div class="actions">
          {#if canInstallMedia}<button
              class="button storage-action"
              disabled={!control.canConfigure || actionPending}
              onclick={installMedia}>Install media support</button
            >{/if}
          <button
            class="button quiet storage-action"
            disabled={!control.canConfigure || actionPending}
            onclick={() => locate("converter")}
            >{pending === "converter"
              ? "Checking…"
              : "Locate converter"}</button
          >
        </div>
      {:else}<span class="setup-state" data-text-surface="flat"
          >{app.native
            ? setupLabel("mediaImport", status?.mediaImport)
            : "Desktop app"}</span
        >{/if}
    </SettingRow>
  {/snippet}
</ModelSetup>

<div class="setup-options">
  <AiSettings />
</div>

<style>
  .setup-options :global(.row-copy),
  .setup-options :global(.row-copy small),
  .setup-options :global(.button),
  .setup-options :global(select),
  .setup-options :global(input),
  .setup-options :global(.field-label),
  .setup-options :global(.field-help) {
    font-size: 1rem;
  }
  .setup-options :global(.paper-card) {
    margin-top: 27px;
    padding: 27px;
  }
  @media (max-width: 650px) {
    .setup-options :global(.paper-card) {
      padding: 18px;
    }
  }
  /* Models keeps the 18px control scale (see the 0.20.0 Setup notes in the
     design bible), including the storage rows in Performance and storage. */
  .storage-action {
    font-size: 1rem;
  }
  .setup-state {
    white-space: nowrap;
    color: var(--muted);
  }
</style>
