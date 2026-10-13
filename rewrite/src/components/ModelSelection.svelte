<script lang="ts">
  import { tick } from "svelte";
  import RemoteModelBadge from "./RemoteModelBadge.svelte";
  import { priceLine, type AiModel } from "../lib/ai";
  import { SMART_CLEANUP_MODEL } from "../lib/smart-cleanup";
  import {
    checking,
    discoverable,
    formatBytes,
    installed,
    installing,
    memoryRange,
    modelUnavailableReason,
    shortDeviceLabel,
    type ModelEntry,
    type ModelSelectionRole,
  } from "../lib/model-library";

  let {
    role,
    title,
    selectedId,
    models,
    disabled,
    installDisabled,
    onselect,
    oninstall,
    remoteModels = [],
    selectedRemoteId = "",
    onlineConnected = false,
    allowOnline = role === "notes",
    onremote,
    onaddonline,
    pickerId = `model-${role}`,
    localDefaultLabel,
  }: {
    role: ModelSelectionRole;
    title: string;
    selectedId: string;
    models: ModelEntry[];
    disabled: boolean;
    installDisabled: boolean;
    onselect: (id: string) => Promise<void>;
    oninstall: (model: ModelEntry) => Promise<void>;
    remoteModels?: AiModel[];
    selectedRemoteId?: string;
    onlineConnected?: boolean;
    allowOnline?: boolean;
    onremote?: (id: string) => Promise<void>;
    onaddonline?: () => void;
    pickerId?: string;
    localDefaultLabel?: string;
  } = $props();
  const uid = $props.id();
  let trigger: HTMLButtonElement;
  let panel: HTMLDivElement;
  let search: HTMLInputElement;
  let open = $state(false);
  let query = $state("");
  let candidateId = $state("");
  let position = $state({ left: 0, top: 0, width: 0, height: 400 });
  const selected = $derived(models.find((m) => m.id === selectedId));
  const remoteSelected = $derived(
    remoteModels.find((m) => m.id === selectedRemoteId),
  );
  const remoteCandidates = $derived(
    remoteModels.filter((model) =>
      `${model.name} ${model.id}`.toLowerCase().includes(query.toLowerCase()),
    ),
  );
  const candidates = $derived(
    models.filter(
      (m) =>
        m.roles.includes(role) &&
        (discoverable(m) || m.id === selectedId) &&
        `${m.name} ${shortDeviceLabel(m)} ${m.languages.join(" ")}`
          .toLowerCase()
          .includes(query.toLowerCase()),
    ),
  );
  const candidate = $derived(candidates.find((m) => m.id === candidateId));
  const tabStop = $derived(candidate?.id ?? candidates[0]?.id);

  async function show(): Promise<void> {
    if (disabled) return;
    if (open) {
      panel.hidePopover();
      return;
    }
    query = "";
    candidateId = selectedId;
    const rect = trigger.getBoundingClientRect();
    const width = Math.min(Math.max(rect.width, 450), window.innerWidth - 36);
    const height = Math.min(450, window.innerHeight - 36);
    position = {
      left: Math.max(
        18,
        Math.min(rect.right - width, window.innerWidth - width - 18),
      ),
      top: Math.max(
        18,
        Math.min(rect.bottom + 9, window.innerHeight - height - 18),
      ),
      width,
      height,
    };
    panel.showPopover();
    await tick();
    search.focus();
  }
  function close(restore = false): void {
    panel?.hidePopover();
    if (restore) trigger.focus();
  }
  async function choose(model: ModelEntry): Promise<void> {
    candidateId = model.id;
    if (!installed(model) || disabled) return;
    await onselect(model.id);
    close(true);
  }
  function move(event: KeyboardEvent): void {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    if (event.target === search && ["Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const options = Array.from(
      panel.querySelectorAll<HTMLButtonElement>('[role="option"]'),
    );
    const current = options.findIndex(
      (option) => option === document.activeElement,
    );
    const next =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? options.length - 1
          : event.key === "ArrowDown"
            ? (current + 1) % options.length
            : current < 0
              ? options.length - 1
              : (current - 1 + options.length) % options.length;
    options[next]?.focus();
  }
</script>

<svelte:window onresize={() => close()} />

<button
  bind:this={trigger}
  id={pickerId}
  class="button selection"
  aria-label={`${title} model`}
  aria-haspopup="dialog"
  aria-expanded={open}
  aria-controls={`${uid}-picker`}
  {disabled}
  onclick={show}
  onkeydown={(event) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      void show();
    }
  }}
>
  <span
    >{selectedRemoteId
      ? (remoteSelected?.name ?? selectedRemoteId)
      : ((!selectedId ? localDefaultLabel : undefined) ??
        (selectedId === SMART_CLEANUP_MODEL
          ? "Smart cleanup (experimental)"
          : (selected?.name ??
            selectedId ??
            "No selection")))}{#if selectedRemoteId}<RemoteModelBadge
        embedded
      />{/if}</span
  >
  <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"
    ><path
      d="m4 6 4 4 4-4"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
    /></svg
  >
</button>

<div
  bind:this={panel}
  id={`${uid}-picker`}
  class="stock paper-popover picker"
  popover="auto"
  role="dialog"
  tabindex="-1"
  aria-label={`${title} models`}
  style:left={`${position.left}px`}
  style:top={`${position.top}px`}
  style:width={`${position.width}px`}
  style:max-height={`${position.height}px`}
  ontoggle={() => {
    open = panel.matches(":popover-open");
  }}
  onfocusout={(event) => {
    if (
      event.relatedTarget instanceof Node &&
      !panel.contains(event.relatedTarget) &&
      event.relatedTarget !== trigger
    )
      close();
  }}
  onkeydown={(event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      close(true);
    } else move(event);
  }}
>
  <input
    bind:this={search}
    type="search"
    aria-label={`Search ${title.toLowerCase()} models`}
    placeholder="Search models"
    bind:value={query}
  />
  <div class="options" role="listbox" aria-label={`${title} choices`}>
    <p class="group-label" data-text-surface="flat">On this computer</p>
    {#if localDefaultLabel}
      <button
        class="option"
        role="option"
        aria-selected={!selectedRemoteId && !selectedId}
        onclick={async () => {
          if (!disabled) {
            await onselect("");
            close(true);
          }
        }}>{localDefaultLabel}</button
      >
    {/if}
    {#each candidates as model (model.id)}
      <button
        class="option"
        role="option"
        aria-selected={!selectedRemoteId && selectedId === model.id}
        tabindex={tabStop === model.id ? 0 : -1}
        onclick={() => choose(model)}
        onfocus={() => {
          candidateId = model.id;
        }}
      >
        <span
          >{model.id === SMART_CLEANUP_MODEL
            ? "Smart cleanup (experimental)"
            : model.name}{#if model.id === "openrouter"}<RemoteModelBadge
              embedded
            />{/if}</span
        >
        <span class="meta" data-text-surface="flat"
          >{model.id === "openrouter" ? "Remote" : shortDeviceLabel(model)} · {checking(
            model,
          )
            ? "Checking…"
            : modelUnavailableReason(model)
              ? "Unavailable"
              : !selectedRemoteId && model.id === selectedId
                ? "Selected"
                : installed(model)
                  ? "Installed"
                  : installing(model)
                    ? "Installing"
                    : model.installable
                      ? "Not installed"
                      : "Unavailable"}</span
        >
      </button>
    {/each}
    {#if !candidates.length && !localDefaultLabel}<p
        class="empty"
        data-text-surface="flat"
      >
        No matching models.
      </p>{/if}
    {#if allowOnline && onlineConnected}
      <p class="group-label" data-text-surface="flat">OpenRouter</p>
      {#each remoteCandidates as model (model.id)}
        <button
          class="option"
          role="option"
          aria-selected={selectedRemoteId === model.id}
          onclick={async () => {
            if (disabled) return;
            await onremote?.(model.id);
            close(true);
          }}
        >
          <span>{model.name}<RemoteModelBadge embedded /></span>
          <span class="meta" data-text-surface="flat">{priceLine(model)}</span>
        </button>
      {/each}
      {#if !remoteCandidates.length}<p class="empty" data-text-surface="flat">
          No matching online models in your list.
        </p>{/if}
    {/if}
  </div>
  {#if candidate}
    <details class="model-details">
      <summary>Details for {candidate.name}</summary>
      <div class="meta" data-text-surface="flat">
        {#if modelUnavailableReason(candidate)}<p>
            {modelUnavailableReason(candidate)}
          </p>{/if}
        <p>
          {shortDeviceLabel(candidate)} · {candidate.languages.join(", ") ||
            "Language details unavailable"}
        </p>
        <p>
          Memory: {memoryRange(candidate.ramMinBytes, candidate.ramMaxBytes)} RAM
          · {memoryRange(candidate.vramMinBytes, candidate.vramMaxBytes)} graphics
          memory
        </p>
      </div>
    </details>
  {/if}
  {#if candidate && !installed(candidate)}
    <div class="candidate" aria-live="polite">
      <span data-text-surface="flat"
        >{candidate.name} · {formatBytes(candidate.diskBytes)}</span
      >
      <button
        class="button"
        disabled={installDisabled ||
          checking(candidate) ||
          !candidate.installable ||
          installing(candidate)}
        onclick={async () => {
          if (candidate) await oninstall(candidate);
          close(true);
        }}
        >{checking(candidate)
          ? "Checking…"
          : installing(candidate)
            ? "Installing…"
            : candidate.installable
              ? "Install"
              : "Unavailable"}</button
      >
    </div>
    <p class="meta install-help" data-text-surface="flat">
      {modelUnavailableReason(candidate) ?? "Select it after it downloads."}
    </p>
  {/if}
  {#if onaddonline && role !== "cleanup"}
    <p class="meta install-help" data-text-surface="flat">
      {allowOnline
        ? `Online choices send ${role === "asr" || role === "meetingSpeech" ? "meeting audio" : "text"} to OpenRouter and are billed there.`
        : "Dictation speech stays on this computer. Online speech is available for meeting transcription."}
    </p>
    <button
      class="button quiet"
      onclick={() => {
        close();
        onaddonline?.();
      }}
    >
      {onlineConnected
        ? "Add OpenRouter models…"
        : "Connect OpenRouter to add online models…"}
    </button>
  {/if}
</div>

<style>
  .selection {
    width: 100%;
    justify-content: space-between;
    text-align: left;
    gap: 18px;
    font-size: 1rem;
  }
  .selection span {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .picker {
    margin: 0;
    padding: 18px;
    overflow: hidden;
  }
  .picker:popover-open {
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  .picker::backdrop {
    background: transparent;
  }
  input,
  .button {
    font-size: 1rem;
  }
  .options {
    overflow-y: auto;
    min-height: 0;
    padding: 9px;
    margin: 0 -9px;
  }
  .option {
    display: block;
    width: 100%;
    padding: 9px;
    margin: 0;
    border: 0;
    border-radius: 9px;
    background: transparent;
    text-align: left;
    font-size: 1rem;
  }
  .option:hover,
  .option[aria-selected="true"] {
    background: var(--well);
  }
  .option span {
    display: block;
    overflow-wrap: anywhere;
  }
  .meta,
  .empty {
    color: var(--soft-ink);
  }
  .group-label {
    margin: 9px;
    color: var(--soft-ink);
  }
  .model-details {
    flex: none;
    max-height: 135px;
    overflow: auto;
  }
  .model-details summary {
    cursor: pointer;
  }
  .model-details p {
    margin: 9px 0;
  }
  .install-help {
    margin: 0;
    font-size: 0.9rem;
  }
  .empty {
    margin: 9px;
  }
  .candidate {
    display: flex;
    align-items: center;
    gap: 18px;
    justify-content: space-between;
    border-top: 1px solid var(--line);
    padding-top: 9px;
  }
  .candidate span {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .candidate .button {
    flex: none;
  }
</style>
