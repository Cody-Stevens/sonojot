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
  <div class="tabs" role="tablist" aria-label="Model library views">
    {#each ["installed", "browse"] as value, index}
      <button
        id={`${uid}-${value}`}
        class="button"
        role="tab"
        aria-selected={tab === value}
        aria-controls={`${uid}-rows`}
        tabindex={tab === value ? 0 : -1}
        onclick={() => switchTab(value)}
        onkeydown={(event) => {
          if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
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
  {#if tab === "browse"}<ModelPareto />{/if}
  <div class="filters">
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
      <div class="entry" data-model-id={model.id}>
        <div class="copy">
          <span>{modelDisplayName(model, models)}</span><span
            class="meta"
            data-text-surface="flat"
            >{model.roles.includes("meetingSpeech")
              ? "meetings · "
              : ""}{shortDeviceLabel(model)} · {model.diskBytes
              ? formatBytes(model.diskBytes)
              : model.id === "local-rules"
                ? "No download"
                : "Size unknown"}{#if roles(model)}{` · ${roles(model)}`}{/if}{#if model.older}{" · Older"}{/if}</span
          >
          {#if model.roles.includes("meetingSpeech")}<span
              class="meta"
              data-text-surface="flat">{model.guidance}</span
            >{/if}
          <ModelLicense id={model.id} />
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
          {#if model.kind === "punctuation"}
            {#if !installed(model) && model.totalBytes > 0}<span
                class="meta"
                data-text-surface="flat"
                >{formatBytes(model.totalBytes)} to download</span
              >{/if}
            <PunctuationDetails id={model.id} />
          {:else}
            <details class="meta" data-text-surface="flat">
              <summary>Details</summary>
              <p>{detail(model)}</p>
            </details>
          {/if}
        </div>
        {#if checking(model)}<span class="meta" data-text-surface="flat"
            >Checking…</span
          >
        {:else if installing(model)}<span class="meta" data-text-surface="flat"
            >Installing</span
          >
        {:else if installed(model)}
          {#if model.removable}<button
              class="button quiet"
              disabled={locked || !!plan?.blocked}
              title={reason || `Remove ${model.name}`}
              aria-label={`Remove ${model.name}`}
              onclick={() => onremove(model)}>Remove</button
            >{:else}<span class="meta" data-text-surface="flat">Included</span
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
    {/each}
    {#if !rows.length}<p class="empty" data-text-surface="flat">
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
      ><span data-text-surface="flat" aria-live="polite"
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
  .tabs {
    display: flex;
    gap: 9px;
    margin-bottom: 18px;
  }
  .tabs [aria-selected="true"] {
    background-color: var(--dye);
    color: var(--dye-ink);
  }
  .filters {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(135px, 1fr);
    gap: 18px;
    margin-bottom: 9px;
  }
  input,
  select,
  .button {
    font-size: 1rem;
  }
  .entry {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 18px;
    padding: 18px 0;
    border-bottom: 1px solid var(--line);
  }
  .entry:last-child {
    border: 0;
  }
  .entry .button {
    flex: none;
  }
  .copy {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .copy span {
    display: block;
  }
  details {
    margin-top: 9px;
  }
  summary {
    cursor: pointer;
  }
  details p {
    margin: 9px 0 0;
  }
  .meta,
  .empty {
    color: var(--soft-ink);
  }
  .empty {
    margin: 18px 0;
  }
  .paused {
    margin: 9px 0 0;
  }
  .removal-reason {
    margin-top: 9px;
  }
  .removal-fix {
    margin-top: 9px;
  }
  .pagination {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 18px;
    margin-top: 18px;
  }
  @media (max-width: 550px) {
    .filters {
      grid-template-columns: minmax(0, 1fr);
      gap: 9px;
    }
    .entry {
      flex-wrap: wrap;
      gap: 9px;
    }
    .copy {
      flex: 1 1 100%;
    }
  }
</style>
