<script lang="ts">
  import ModelPareto from "./ModelPareto.svelte";
  import PunctuationDetails from "./PunctuationDetails.svelte";
  import ModelLicense from "./ModelLicense.svelte";
  import {
    checking,
    formatBytes,
    installed,
    installing,
    memoryRange,
    modelRoles,
    modelUnavailableReason,
    shortDeviceLabel,
    unavailableHere,
    type ModelEntry,
    type RemovalPlan,
  } from "../lib/model-library";
  import { modelDisplayName } from "../lib/punctuation";
  let {
    models,
    selectedIds,
    remoteNotes = false,
    locked,
    lockedReason = "",
    installDisabled,
    removal,
    oninstall,
    onremove,
  }: {
    models: ModelEntry[];
    selectedIds: string[];
    /** Notes go to OpenRouter, so the saved local Notes model is not in use. */
    remoteNotes?: boolean;
    locked: boolean;
    /** Shown under installed rows while every Remove is paused. */
    lockedReason?: string;
    installDisabled: boolean;
    removal: (model: ModelEntry) => RemovalPlan;
    oninstall: (model: ModelEntry) => Promise<void>;
    onremove: (model: ModelEntry) => Promise<void>;
  } = $props();
  const uid = $props.id();
  let tab = $state("installed");
  let query = $state("");
  let filter = $state("all");
  let page = $state(0);
  const pageSize = 5;
  // Packages this computer cannot run follow the ones it can install.
  const filtered = $derived(
    models
      .filter(
        (model) =>
          (tab === "installed"
            ? installed(model) || checking(model)
            : !model.deprecated && !installed(model) && !checking(model)) &&
          (filter === "all" || model.roles.includes(filter)) &&
          `${modelDisplayName(model, models)} ${shortDeviceLabel(model)} ${model.languages.join(" ")}`
            .toLowerCase()
            .includes(query.toLowerCase()),
      )
      .sort(
        (a, b) => Number(unavailableHere(a)) - Number(unavailableHere(b)),
      ),
  );
  const pageCount = $derived(
    Math.max(1, Math.ceil(filtered.length / pageSize)),
  );
  const currentPage = $derived(Math.min(page, pageCount - 1));
  const rows = $derived(
    filtered.slice(currentPage * pageSize, (currentPage + 1) * pageSize),
  );
  function switchTab(value: string): void {
    tab = value;
    page = 0;
  }
  /** Removal is blocked until another model for this role is installed. */
  function showModelsFor(role: string): void {
    query = "";
    filter = role;
    switchTab("browse");
    document.getElementById(`${uid}-browse`)?.focus();
  }
  const roleModels: Record<string, string> = {
    asr: "Show speech models",
    notes: "Show text models",
  };
  function detail(model: ModelEntry): string {
    const memory = /metal/i.test(model.backend)
      ? `Estimated unified memory ${memoryRange(model.ramMinBytes, model.ramMaxBytes)}`
      : `Estimated RAM ${memoryRange(model.ramMinBytes, model.ramMaxBytes)}; GPU VRAM ${memoryRange(model.vramMinBytes, model.vramMaxBytes)}`;
    return `${model.family}. ${model.languages.join(", ")}. ${memory}.`;
  }
  /** Device · size · in-use roles · Older: the one meta line of a row. */
  function metaLine(model: ModelEntry): string {
    const size = model.diskBytes
      ? formatBytes(model.diskBytes)
      : model.id === "local-rules"
        ? "No download"
        : "Size unknown";
    return [
      shortDeviceLabel(model),
      size,
      model.roles.includes("meetingSpeech") && !model.roles.includes("asr")
        ? "Meetings only"
        : "",
      roles(model),
      model.older ? "Older" : "",
    ]
      .filter(Boolean)
      .join(" · ");
  }
  function roles(model: ModelEntry): string {
    const selected: string[] = modelRoles
      .filter(
        (role, index) =>
          (role.id !== "notes" || !remoteNotes) &&
          (selectedIds[index] === model.id || model.usedBy.includes(role.id)),
      )
      .map((role) => role.title);
    if (
      model.roles.includes("meetingSpeech") &&
      (selectedIds.includes(model.id) || model.usedBy.includes("meetingSpeech"))
    )
      selected.push("Meeting transcription");
    return selected.join(", ");
  }
</script>

<div class="library">
  <!-- One toolbar row: view, search and role filter. -->
  <div class="toolbar">
    <div class="tabs" role="tablist" aria-label="Model library views">
      {#each ["installed", "browse"] as value, index}
        <button
          id={`${uid}-${value}`}
          role="tab"
          aria-selected={tab === value}
          aria-controls={`${uid}-rows`}
          tabindex={tab === value ? 0 : -1}
          onclick={() => switchTab(value)}
          onkeydown={(event) => {
            if (
              ["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)
            ) {
              event.preventDefault();
              const next =
                event.key === "Home"
                  ? "installed"
                  : event.key === "End"
                    ? "browse"
                    : index === 0
                      ? "browse"
                      : "installed";
              switchTab(next);
              document.getElementById(`${uid}-${next}`)?.focus();
            }
          }}>{value === "installed" ? "Installed" : "Get more"}</button
        >
      {/each}
    </div>
    <input
      type="search"
      placeholder="Search models"
      aria-label="Search model library"
      bind:value={query}
      oninput={() => {
        page = 0;
      }}
    />
    <select
      aria-label="Filter model library"
      bind:value={filter}
      onchange={() => {
        page = 0;
      }}
      ><option value="all">All roles</option>{#each modelRoles as role}<option
          value={role.id}>{role.title}</option
        >{/each}<option value="meetingSpeech">Meetings</option><option
        value="punctuation">Punctuation</option
      ></select
    >
  </div>
  {#if tab === "browse"}<ModelPareto />{/if}
  <div
    id={`${uid}-rows`}
    role="tabpanel"
    aria-labelledby={`${uid}-${tab}`}
    tabindex="0"
  >
    {#if locked && lockedReason && tab === "installed"}<p
        class="meta paused"
        data-text-surface="flat"
        role="status"
      >
        {lockedReason}
      </p>{/if}
    {#each rows as model (model.id)}
      {@const plan =
        installed(model) && model.removable ? removal(model) : null}
      {@const reason = plan?.blocked ?? ""}
      <!-- Row anatomy: name, one meta line, the action; everything else is
           behind Details. Only a reason that blocks the action stays out. -->
      <div class="entry" data-model-id={model.id}>
        <div class="copy">
          <span class="name">{modelDisplayName(model, models)}</span>
          <span class="meta" data-text-surface="flat">{metaLine(model)}</span>
          {#if modelUnavailableReason(model)}<span
              class="meta"
              data-text-surface="flat">{modelUnavailableReason(model)}</span
            >{/if}
          {#if reason}<span
              class="meta removal-reason"
              data-text-surface="flat"
              role="status">{reason}</span
            >{#if plan?.needs && roleModels[plan.needs]}<button
                class="button quiet removal-fix"
                onclick={() => plan?.needs && showModelsFor(plan.needs)}
                >{roleModels[plan.needs]}</button
              >{/if}{/if}
          <details class="meta row-details" data-text-surface="flat">
            <summary>Details</summary>
            <div class="details-body">
              {#if model.kind === "punctuation"}
                {#if !installed(model) && model.totalBytes > 0}<p>
                    {formatBytes(model.totalBytes)} to download
                  </p>{/if}
                <PunctuationDetails id={model.id} inline />
              {:else}
                {#if model.roles.includes("meetingSpeech")}<p>
                    {model.guidance}
                  </p>{/if}
                <p>{detail(model)}</p>
              {/if}
              <ModelLicense id={model.id} />
            </div>
          </details>
        </div>
        <div class="action">
          {#if checking(model)}<span class="meta" data-text-surface="flat"
              >Checking…</span
            >
          {:else if installing(model)}<span
              class="meta"
              data-text-surface="flat">Installing</span
            >
          {:else if installed(model)}
            {#if model.removable}<button
                class="button quiet"
                disabled={locked || !!plan?.blocked}
                title={reason || `Remove ${model.name}`}
                aria-label={`Remove ${model.name}`}
                onclick={() => onremove(model)}>Remove</button
              >{:else}<span class="meta" data-text-surface="flat"
                >Included</span
              >{/if}
          {:else if model.installable}<button
              class="button"
              disabled={installDisabled}
              aria-label={`${model.error ? "Retry" : "Install"} ${model.name}`}
              onclick={() => oninstall(model)}
              >{model.error ? "Retry" : "Install"}</button
            >
          {:else}<span class="meta" data-text-surface="flat">Unavailable</span
            >{/if}
        </div>
      </div>
    {/each}
    {#if !rows.length}<p class="meta empty" data-text-surface="flat">
        {query || filter !== "all"
          ? "No matching models."
          : tab === "installed"
            ? "No installed models."
            : "All available models are installed."}
      </p>{/if}
  </div>
  {#if pageCount > 1}<div class="pagination">
      <button
        class="button quiet"
        aria-label="Previous models"
        disabled={currentPage === 0}
        onclick={() => {
          page = currentPage - 1;
        }}>Previous</button
      ><span class="meta" data-text-surface="flat" aria-live="polite"
        >{currentPage + 1} / {pageCount}</span
      ><button
        class="button quiet"
        aria-label="Next models"
        disabled={currentPage === pageCount - 1}
        onclick={() => {
          page = currentPage + 1;
        }}>Next</button
      >
    </div>{/if}
</div>

<style>
  .library {
    font-size: 1rem;
  }
  .toolbar {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) minmax(135px, 200px);
    align-items: center;
    gap: 9px 18px;
    margin-bottom: 9px;
  }
  /* View tabs: a small recessed track, the chosen view a lifted slip. */
  .tabs {
    display: flex;
    gap: 3px;
    padding: 3px;
    border-radius: 12px;
    background-color: var(--well);
    box-shadow: inset 0 1.5px 2px var(--contact);
  }
  .tabs button {
    border: 0;
    border-radius: 9px;
    padding: 6px 14px;
    background: transparent;
    color: var(--ink);
    font-size: 1rem;
    line-height: 1.3;
    white-space: nowrap;
  }
  .tabs [aria-selected="true"] {
    background-color: var(--card);
    box-shadow:
      0 1px 2px var(--contact),
      inset 0 1px 0 var(--surface-highlight-color);
  }
  .tabs button:focus-visible {
    outline: 2px solid var(--ink);
    outline-offset: 1px;
  }
  :global(.studio[data-material="off"]) .tabs [aria-selected="true"] {
    outline: 1px solid var(--line);
  }
  input,
  select,
  .button {
    font-size: 1rem;
  }
  .entry {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: start;
    gap: 18px;
    padding: 14px 0;
    border-bottom: 1px solid var(--line);
  }
  .entry:last-child {
    border: 0;
  }
  .action {
    display: flex;
    align-items: center;
    min-height: 44px;
  }
  .action .button {
    flex: none;
  }
  .copy {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .meta {
    color: var(--soft-ink);
    font-size: 15px;
    line-height: 1.45;
  }
  .row-details {
    margin-top: 2px;
    align-self: stretch;
  }
  summary {
    cursor: pointer;
    width: fit-content;
  }
  summary:focus-visible {
    outline: 2px solid var(--ink);
    outline-offset: 3px;
  }
  .details-body {
    padding: 4px 0 0 18px;
  }
  .details-body :global(p) {
    margin: 4px 0 0;
    font-size: 15px;
  }
  .empty {
    margin: 18px 0;
  }
  .paused {
    margin: 9px 0 0;
  }
  .removal-reason {
    margin-top: 4px;
  }
  .removal-fix {
    margin: 9px 0 4px;
  }
  .pagination {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 18px;
    margin-top: 18px;
  }
  @media (max-width: 760px) {
    .toolbar {
      grid-template-columns: minmax(0, 1fr) minmax(135px, 1fr);
    }
    .tabs {
      grid-column: 1 / -1;
      justify-self: start;
    }
  }
  @media (max-width: 550px) {
    .toolbar {
      grid-template-columns: minmax(0, 1fr);
    }
    .entry {
      grid-template-columns: minmax(0, 1fr);
      gap: 9px;
    }
  }
</style>
