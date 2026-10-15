<script lang="ts">
  import { onMount } from "svelte";
  import Card from "../components/Card.svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import { app, STATUS_NOTICE_MS } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import { errorMessage } from "../lib/protocol";
  import {
    crashHeading,
    reportCanSend,
    type PendingCrash,
    type Report,
  } from "../lib/reports";

  let {
    crash,
    onclose,
  }: { crash: PendingCrash | null; onclose: () => Promise<void> } = $props();
  let draft = $state<Report | null>(null);
  let notes = $state("");
  let consent = $state(false);
  let preparing = $state(true);
  let preparedValid = $state(false);
  let sending = $state(false);
  let error = $state("");
  let failedSubmission = $state(false);
  let receipt = $state("");
  let copied = $state(false);
  let revision = 0;
  let disposed = false;
  let receiptTimer: ReturnType<typeof setTimeout> | undefined;
  let attempted: string | null = null;
  const report = $derived(draft ? { ...draft, notes } : null);
  const json = $derived(report ? JSON.stringify(report, null, 2) : "");
  const noteCount = $derived(Array.from(notes).length);
  const checkboxLabel = $derived(
    crash
      ? "Send this anonymous crash report to help fix bugs"
      : "Include anonymous diagnostics",
  );

  onMount(() => {
    void prepare(false);
    return () => {
      disposed = true;
      revision++;
      clearTimeout(receiptTimer);
      app.dirty = false;
    };
  });
  $effect(() => {
    app.dirty = !receipt && (notes.length > 0 || sending);
  });

  async function prepare(includeDiagnostics: boolean): Promise<void> {
    const current = ++revision;
    preparing = true;
    preparedValid = false;
    error = "";
    // Unchecking removes diagnostics immediately, including during an older request.
    if (draft) draft = { ...draft, diagnostics: null };
    try {
      const platform = app.snapshot.capabilities.platform;
      const prepared: Report = app.native
        ? await call("prepare_report", {
            kind: crash ? "crash" : "bug",
            includeDiagnostics,
          })
        : {
            schema: 1 as const,
            kind: "bug" as const,
            requestId: crypto.randomUUID(),
            app: {
              version: app.snapshot.version,
              platform:
                platform === "linux" || platform === "macos"
                  ? platform
                  : "windows",
              arch: "preview",
              os: "Browser preview",
            },
            notes: "",
            crash: null,
            diagnostics: null,
          };
      if (disposed || current !== revision) return;
      draft = {
        ...prepared,
        requestId: draft?.requestId ?? prepared.requestId,
      };
      preparedValid = true;
      rotateChangedAttempt();
    } catch (failure: unknown) {
      if (!disposed && current === revision) error = errorMessage(failure);
    } finally {
      if (!disposed && current === revision) preparing = false;
    }
  }

  // A lost response can safely retry the same bytes. An edited report is a new request.
  function rotateChangedAttempt(): void {
    if (
      draft &&
      attempted &&
      JSON.stringify({ ...draft, notes }) !== attempted
    ) {
      draft = { ...draft, requestId: crypto.randomUUID() };
      attempted = null;
    }
    copied = false;
  }
  async function send(): Promise<void> {
    if (
      !reportCanSend(report, consent) ||
      !report ||
      !preparedValid ||
      preparing ||
      sending ||
      receipt ||
      !app.native
    )
      return;
    rotateChangedAttempt();
    if (!draft) return;
    const payload = { ...draft, notes };
    attempted = JSON.stringify(payload);
    sending = true;
    error = "";
    try {
      const result = await call("send_report", { report: payload });
      if (disposed) return;
      receipt = result.receiptId;
      failedSubmission = false;
      app.dirty = false;
      if (crash) await app.dismissPendingCrash();
      receiptTimer = setTimeout(() => {
        void close();
      }, STATUS_NOTICE_MS);
    } catch (failure: unknown) {
      if (!disposed) {
        error = errorMessage(failure);
        failedSubmission = !receipt;
      }
    } finally {
      if (!disposed) sending = false;
    }
  }
  async function close(): Promise<void> {
    try {
      if (crash) await app.dismissPendingCrash();
      app.dirty = false;
      await onclose();
    } catch (failure: unknown) {
      error = errorMessage(failure);
    }
  }
  async function copy(): Promise<void> {
    if (!report) return;
    try {
      await call("copy_report", { report });
      copied = true;
    } catch (failure: unknown) {
      error = errorMessage(failure);
    }
  }
  async function issues(event: MouseEvent): Promise<void> {
    if (!app.native) return;
    event.preventDefault();
    try {
      await call("open_report_issues");
    } catch (failure: unknown) {
      error = errorMessage(failure);
    }
  }
</script>

<header class="page-heading">
  <h1 class="text-grain" data-text-surface="emboss">
    {crash ? crashHeading(crash.reason) : "Report a bug"}
  </h1>
</header>
<Card>
  {#if receipt}
    <p role="status">
      Report sent. Receipt: <span class="receipt">{receipt}</span>
    </p>
  {:else}
    {#if crash?.reason === "unexpectedExit"}
      <p class="body-copy" data-text-surface="flat">
        A crash, forced quit or power loss may have caused this.
      </p>
    {/if}
    <div class="report-notes">
      <label for="report-notes"
        >{crash ? "Notes (optional)" : "What went wrong?"}</label
      >
      <textarea
        id="report-notes"
        rows="5"
        bind:value={notes}
        disabled={sending}
        aria-describedby="report-notes-help"
        required={!crash}
        oninput={() => {
          queueMicrotask(rotateChangedAttempt);
        }}></textarea>
      <small id="report-notes-help" class="field-help" data-text-surface="flat">
        Leave out private information. {noteCount.toLocaleString()} / 4,000
      </small>
    </div>
    <SettingRow label={checkboxLabel}>
      <input
        type="checkbox"
        aria-label={checkboxLabel}
        bind:checked={consent}
        disabled={sending || !app.native}
        onchange={(event) => {
          void prepare(event.currentTarget.checked);
        }}
      />
    </SettingRow>
    <details class="report-preview spaced">
      <summary>Show what will be sent</summary>
      {#if preparing}<p role="status" data-text-surface="flat">
          Preparing report…
        </p>
      {:else if report}<pre>{json}</pre>{/if}
    </details>
  {/if}
  {#if error}<p class="report-error spaced" role="alert">{error}</p>{/if}
  <div class="actions spaced">
    {#if !receipt}<button
        class="button primary"
        disabled={!app.native ||
          !preparedValid ||
          preparing ||
          sending ||
          !reportCanSend(report, consent)}
        onclick={send}>{sending ? "Sending…" : "Send"}</button
      >{/if}
    <button class="button" disabled={sending} onclick={close}
      >{receipt ? "Done" : crash ? "Not now" : "Cancel"}</button
    >
    {#if failedSubmission}
      <button
        class="button"
        disabled={sending || preparing || !reportCanSend(report, true)}
        onclick={copy}>{copied ? "Copied" : "Copy report"}</button
      >
      <a
        href="https://github.com/Cody-Stevens/sonojot/issues"
        onclick={issues}
        target="_blank"
        rel="noreferrer">Report on GitHub ↗</a
      >
    {/if}
  </div>
</Card>

<style>
  .report-notes {
    display: grid;
    gap: 9px;
  }
  textarea {
    width: 100%;
    min-height: 135px;
    resize: vertical;
  }
  input[type="checkbox"] {
    width: 22px;
    height: 22px;
    accent-color: var(--dye);
  }
  pre {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 360px;
    overflow: auto;
    font-size: 0.72rem;
    padding: 18px;
  }
  .receipt {
    overflow-wrap: anywhere;
  }
  .report-error {
    color: var(--danger);
  }
</style>
