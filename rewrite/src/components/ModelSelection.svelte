<script lang="ts">
  import { tick } from "svelte";
  import RemoteModelBadge from "./RemoteModelBadge.svelte";
  import { priceLine, type AiModel } from "../lib/ai";
  import { app } from "../lib/app.svelte";
  import { scrollEdges } from "../lib/scroll-edges";
  import { SMART_CLEANUP_MODEL } from "../lib/smart-cleanup";
  import {
    PUNCTUATION_MODEL,
    defaultCleanupStatus,
    modelDisplayName,
  } from "../lib/punctuation";
  import {
    checking,
    discoverable,
    formatBytes,
    installed,
    installing,
    memoryRange,
    modelUnavailableReason,
    shortDeviceLabel,
    unavailableHere,
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
  const matches = (m: ModelEntry) =>
    m.roles.includes(role) &&
    (discoverable(m) || m.id === selectedId) &&
    `${optionName(m)} ${shortDeviceLabel(m)} ${m.languages.join(" ")}`
      .toLowerCase()
      .includes(query.toLowerCase());
  // Default Cleanup is listed first, named for the punctuation model it runs.
  // Packages this computer cannot run are not ordinary choices; the saved one
  // stays visible so its state can be seen.
  const candidates = $derived(
    models
      .filter((m) => matches(m) && (!unavailableHere(m) || m.id === selectedId))
      .sort(
        (a, b) =>
          Number(b.id === "local-rules") - Number(a.id === "local-rules"),
      ),
  );
  const unavailable = $derived(
    models.filter(
      (m) => matches(m) && unavailableHere(m) && m.id !== selectedId,
    ),
  );
  const unavailableTitle = $derived(
    app.snapshot.capabilities.platform === "macos"
      ? "Not available on this Mac"
      : "Not available on this PC",
  );
  const punctuationPackage = $derived(
    models.find((m) => m.id === PUNCTUATION_MODEL),
  );
  function optionName(model: ModelEntry): string {
    return model.id === SMART_CLEANUP_MODEL
      ? "Smart cleanup (experimental)"
      : modelDisplayName(model, models);
  }
  function optionState(model: ModelEntry): string {
    const selectedHere = !selectedRemoteId && model.id === selectedId;
    if (model.id === "local-rules") {
      const status =
        punctuationPackage && installing(punctuationPackage)
          ? "Installing"
          : defaultCleanupStatus(models);
      return [
        shortDeviceLabel(model),
        "Recommended",
        status,
        selectedHere ? "Selected" : "",
      ]
        .filter(Boolean)
        .join(" · ");
    }
    return `${model.id === "openrouter" ? "Remote" : shortDeviceLabel(model)} · ${
      checking(model)
        ? "Checking…"
        : modelUnavailableReason(model)
          ? "Unavailable"
          : selectedHere
            ? "Selected"
            : installed(model)
              ? "Installed"
              : installing(model)
                ? "Installing"
                : model.installable
                  ? "Not installed"
                  : "Unavailable"
    }`;
  }
  // Default Cleanup works without the package; offer it from the picker.
  const missingPunctuation = $derived(
    candidateId === "local-rules" &&
      punctuationPackage &&
      punctuationPackage.installable &&
      !installed(punctuationPackage) &&
      !installing(punctuationPackage) &&
      !checking(punctuationPackage)
      ? punctuationPackage
      : undefined,
  );
  const candidate = $derived(candidates.find((m) => m.id === candidateId));
  const showOnline = $derived(allowOnline && onlineConnected);
  /** One line under the name: languages and memory, never the device again. */
  function detailLine(model: ModelEntry): string {
    const graphics =
      model.vramMinBytes || model.vramMaxBytes
        ? ` · ${memoryRange(model.vramMinBytes, model.vramMaxBytes)} graphics memory`
        : "";
    return `${model.languages.join(", ") || "Language details unavailable"} · Memory: ${memoryRange(
      model.ramMinBytes,
      model.ramMaxBytes,
    )} RAM${graphics}`;
  }
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
    const width = Math.min(Math.max(rect.width, 540), window.innerWidth - 36);
    const height = Math.min(600, window.innerHeight - 36);
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
    revealSelected();
  }
  /** Open with the current choice in view, centered if it was out of sight. */
  function revealSelected(): void {
    const well = panel.querySelector<HTMLElement>(".option-well");
    const chosen = panel.querySelector<HTMLElement>(
      '[role="option"][aria-selected="true"]',
    );
    if (!well || !chosen) return;
    const box = well.getBoundingClientRect();
    const row = chosen.getBoundingClientRect();
    if (row.top < box.top || row.bottom > box.bottom)
      well.scrollTop += row.top - box.top - (box.height - row.height) / 2;
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
          : selected
            ? optionName(selected)
            : (selectedId ??
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
  <!-- The well bounds the list; its edges show when more is above or below. -->
  <div class="paper-well option-well" use:scrollEdges>
    <div class="options" role="listbox" aria-label={`${title} choices`}>
      {#if showOnline}<p class="group-label" data-text-surface="flat">
          On this computer
        </p>{/if}
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
          }}
          ><span class="option-copy"><span>{localDefaultLabel}</span></span
          >{@render check()}</button
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
          <span class="option-copy">
            <span
              >{optionName(
                model,
              )}{#if model.id === "openrouter"}<RemoteModelBadge
                  embedded
                />{/if}</span
            >
            <span class="meta" data-text-surface="flat"
              >{optionState(model)}</span
            >
          </span>{@render check()}
        </button>
      {/each}
      {#if !candidates.length && !localDefaultLabel}<p
          class="empty"
          data-text-surface="flat"
        >
          No matching models.
        </p>{/if}
      {#if showOnline}
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
            <span class="option-copy">
              <span>{model.name}<RemoteModelBadge embedded /></span>
              <span class="meta" data-text-surface="flat"
                >{priceLine(model)}</span
              >
            </span>{@render check()}
          </button>
        {/each}
        {#if !remoteCandidates.length}<p class="empty" data-text-surface="flat">
            No matching online models in your list.
          </p>{/if}
      {/if}
    </div>
    {#if unavailable.length}
      <details class="unavailable">
        <summary
          >{unavailableTitle}
          <span class="meta" data-text-surface="flat"
            >· {unavailable.length}</span
          ></summary
        >
        <ul aria-label={unavailableTitle}>
          {#each unavailable as model (model.id)}
            <li>
              <span>{optionName(model)}</span>
              <span class="meta" data-text-surface="flat"
                >{modelUnavailableReason(model)}</span
              >
            </li>
          {/each}
        </ul>
      </details>
    {/if}
  </div>
  {#if candidate}
    <section
      class="details"
      aria-label={`Details for ${optionName(candidate)}`}
    >
      <p class="details-name">{optionName(candidate)}</p>
      <div class="meta" data-text-surface="flat">
        {#if modelUnavailableReason(candidate)}<p>
            {modelUnavailableReason(candidate)}
          </p>{/if}
        <p>{detailLine(candidate)}</p>
      </div>
      {#if !installed(candidate)}
        <div class="candidate" aria-live="polite">
          <span class="meta" data-text-surface="flat"
            >{formatBytes(candidate.diskBytes)} download. {modelUnavailableReason(
              candidate,
            )
              ? ""
              : "Select it after it downloads."}</span
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
      {/if}
      {#if missingPunctuation}
        <div class="candidate" aria-live="polite">
          <span class="meta" data-text-surface="flat"
            >{missingPunctuation.name} · {formatBytes(
              missingPunctuation.totalBytes || missingPunctuation.diskBytes,
            )}</span
          >
          <button
            class="button"
            disabled={installDisabled}
            onclick={async () => {
              if (missingPunctuation) await oninstall(missingPunctuation);
              close(true);
            }}>Install</button
          >
        </div>
      {/if}
    </section>
  {/if}
  {#if onaddonline && role !== "cleanup"}
    <footer class="online">
      <p class="meta help" data-text-surface="flat">
        {allowOnline
          ? `Online models send ${role === "asr" || role === "meetingSpeech" ? "meeting audio" : "text"} to OpenRouter.`
          : "Online speech is for meetings only."}
      </p>
      <button
        class="button quiet"
        onclick={() => {
          close();
          onaddonline?.();
        }}
      >
        {onlineConnected ? "Add OpenRouter models…" : "Connect OpenRouter…"}
      </button>
    </footer>
  {/if}
</div>

{#snippet check()}<svg
    class="check"
    viewBox="0 0 16 16"
    width="18"
    height="18"
    aria-hidden="true"
    ><path
      d="m3.5 8.5 3 3 6-7"
      fill="none"
      stroke="currentColor"
      stroke-width="1.8"
      stroke-linecap="round"
      stroke-linejoin="round"
    /></svg
  >{/snippet}

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
    gap: 18px;
  }
  .picker::backdrop {
    background: transparent;
  }
  input,
  .button {
    font-size: 1rem;
  }
  .picker > input {
    flex: none;
  }
  /* The list is the one part that shrinks; it keeps about three rows. */
  .option-well {
    flex: 1 1 auto;
    min-height: 162px;
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: 9px;
    border-radius: 12px;
  }
  /* No dead gaps between options: a clear border spaces the slips, so every
     point of the list belongs to an option. */
  .options {
    display: flex;
    flex-direction: column;
  }
  .option {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 18px;
    width: 100%;
    min-height: 54px;
    padding: 9px 12px;
    margin: 0;
    border: 1px solid transparent;
    border-radius: 9px;
    background: transparent;
    background-clip: padding-box;
    outline-offset: -1px;
    text-align: left;
    font-size: 1rem;
    line-height: 1.4;
  }
  .option:hover {
    background: color-mix(in srgb, var(--card) 22%, transparent);
  }
  /* The chosen model is a separate slip lifted from the well, with a check. */
  .option[aria-selected="true"] {
    background: var(--card);
    box-shadow:
      0 1px 2px var(--contact),
      inset 0 1px 0 var(--surface-highlight-color);
  }
  :global(.studio[data-material="off"]) .option[aria-selected="true"] {
    border-color: var(--line);
  }
  .option-copy {
    min-width: 0;
  }
  .option-copy span {
    display: block;
    overflow-wrap: anywhere;
  }
  .check {
    flex: none;
    color: var(--dye);
    visibility: hidden;
  }
  .option[aria-selected="true"] .check {
    visibility: visible;
  }
  /* Two sizes: 18px names, 15px for every secondary line. */
  .meta,
  .empty {
    color: var(--soft-ink);
    font-size: 15px;
  }
  .option .meta {
    line-height: 1.35;
  }
  .group-label {
    margin: 9px 12px 3px;
    font-size: 15px;
    color: var(--soft-ink);
  }
  .group-label:first-child {
    margin-top: 3px;
  }
  .empty {
    margin: 9px 12px;
  }
  .unavailable {
    margin: 9px 12px 3px;
    padding-top: 9px;
    border-top: 1px solid var(--line);
  }
  .unavailable summary {
    cursor: pointer;
    color: var(--soft-ink);
    font-size: 15px;
  }
  .unavailable ul {
    list-style: none;
    margin: 9px 0 0;
    padding: 0;
  }
  .unavailable li {
    padding: 6px 0;
    overflow-wrap: anywhere;
  }
  .unavailable li span {
    display: block;
  }
  .details {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 0 12px;
  }
  .details-name {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .details .meta p {
    margin: 0;
  }
  .details .meta {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .help {
    margin: 0;
  }
  .candidate {
    display: flex;
    align-items: center;
    gap: 18px;
    justify-content: space-between;
    margin-top: 5px;
  }
  .candidate span {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .candidate .button {
    flex: none;
  }
  /* Online choices sit apart from the local list: the note and its action on
     one line below a rule. */
  .online {
    flex: none;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 9px 18px;
    padding: 12px 12px 0;
    border-top: 1px solid var(--line);
  }
  .online .help {
    flex: 1 1 220px;
  }
  .online .button {
    flex: none;
  }
</style>
