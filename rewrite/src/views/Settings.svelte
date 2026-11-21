<script lang="ts" module>
  // Advanced stays open when you come Back from Memory or On disk.
  const remembered = { advanced: false };
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../components/Icon.svelte";
  import PageHeader from "../components/quiet/PageHeader.svelte";
  import Disclosure from "../components/quiet/Disclosure.svelte";
  import RecommendedSetup from "../components/quiet/RecommendedSetup.svelte";
  import LookScreen from "./LookScreen.svelte";
  import MemoryScreen from "./MemoryScreen.svelte";
  import DiskScreen from "./DiskScreen.svelte";
  import UpdatesScreen from "./UpdatesScreen.svelte";
  import HelpScreen from "./HelpScreen.svelte";
  import SyncScreen from "./SyncScreen.svelte";
  import { app } from "../lib/app.svelte";
  import { ai } from "../lib/ai.svelte";
  import { library } from "../lib/model-library.svelte";
  import { profiles } from "../lib/setup-profiles.svelte";
  import { profileChanges } from "../lib/setup-profiles";
  import { cleanupChoice } from "../lib/cleanup-choice";
  import { modelName } from "../lib/model-names";
  import { formatBytes, installed } from "../lib/model-library";
  import { call } from "../lib/api";
  import type { AiModel } from "../lib/ai";
  import { type Route, type RouteTarget } from "../lib/route.svelte";
  let {
    route,
    onnavigate,
    onback,
    parentTitle = "Settings",
    ontour,
  }: {
    route: Route;
    onnavigate: (target: RouteTarget) => void;
    onback?: () => void;
    parentTitle?: string;
    ontour: () => void;
  } = $props();
  let reset = $state(false);
  let catalog = $state<AiModel[]>([]);
  const settings = $derived(app.snapshot.settings);
  const models = $derived(library.data?.models ?? []);
  const changes = $derived(
    profiles.hardware
      ? profileChanges(
          settings,
          profiles.hardware.recommendations.balanced,
          ai.data,
          app.snapshot.localProofreaderAvailable,
        )
      : [],
  );
  const cleanup = $derived(
    cleanupChoice(settings, app.snapshot.localProofreaderAvailable),
  );
  const cleanupName = $derived(
    cleanup.kind === "off"
      ? ""
      : cleanup.kind === "sonojot"
        ? "SonoJot proofreader"
        : modelName(cleanup.model, models, catalog),
  );
  const notesName = $derived(
    settings.meetingNotesPreset === null
      ? "No notes"
      : modelName(
          ai.data?.backend === "openrouter"
            ? ai.data.model
            : settings.notesModel,
          models,
          catalog,
        ),
  );
  const meetingName = $derived(
    settings.meetingAsrModel
      ? modelName(settings.meetingAsrModel, models, catalog)
      : ai.data?.speechModel && ai.data.connected
        ? modelName(ai.data.speechModel, models, catalog)
        : "Same as dictation",
  );
  const source = $derived(
    { both: "You and others", microphone: "You", system: "Others" }[
      settings.meetingSource
    ],
  );
  type Door = { label: string; detail: string; target: Route; role?: string };
  const groups = $derived<Door[][]>(
    route.stage === "dictation"
      ? [
          [
            {
              label: "Shortcut and mic",
              detail: `${settings.hotkey} · ${settings.microphone || "Default microphone"}`,
              target: {
                tab: "dictation",
                stage: "input",
                from: "settings-group",
              },
            },
            {
              label: "Speech",
              detail: modelName(settings.asrModel, models, catalog),
              role: "asr",
              target: {
                tab: "dictation",
                stage: "speech",
                from: "settings-group",
              },
            },
            {
              label: "Cleanup",
              detail: [
                { original: "As heard", format: "Format", clean: "Clean" }[
                  settings.cleanupMode
                ],
                cleanupName,
              ]
                .filter(Boolean)
                .join(" · "),
              role: "cleanup",
              target: {
                tab: "dictation",
                stage: "cleanup",
                from: "settings-group",
              },
            },
            {
              label: "Paste and history",
              detail: `${settings.autoPaste ? "Paste where I was typing" : "Clipboard only"} · ${settings.doNotSaveDictations ? "keep nothing" : settings.keepDictationAudio ? "keep text and audio" : "keep text"}`,
              target: {
                tab: "dictation",
                stage: "output",
                from: "settings-group",
              },
            },
            {
              label: "Doubtful words",
              detail: settings.trust.enabled
                ? `${{ fewer: "Fewer", balanced: "Balanced", more: "More" }[settings.trust.sensitivity]} · ${settings.trust.markStyle === "band" ? "marker band" : "dotted underline"}`
                : "Off",
              target: {
                tab: "dictation",
                stage: "trust",
                from: "settings-group",
              },
            },
          ],
        ]
      : route.stage === "meetings"
        ? [
            [
              {
                label: "Speech and speakers",
                detail: `${source} · ${meetingName} · ${settings.meetingSpeakerLabels ? "speaker names" : "no speaker names"}`,
                role: "meetingSpeech",
                target: {
                  tab: "meetings",
                  stage: "speech",
                  from: "settings-group",
                },
              },
              {
                label: "Notes",
                detail: notesName,
                role: "notes",
                target: {
                  tab: "meetings",
                  stage: "notes",
                  from: "settings-group",
                },
              },
            ],
          ]
        : [
            [
              {
                label: "Dictation",
                detail: [
                  settings.hotkey,
                  modelName(settings.asrModel, models, catalog),
                  cleanupName,
                ]
                  .filter(Boolean)
                  .join(" · "),
                target: { tab: "settings", stage: "dictation" },
              },
              {
                label: "Meetings",
                detail: `${source} · ${meetingName} · ${notesName}`,
                target: { tab: "settings", stage: "meetings" },
              },
            ],
            [
              {
                label: "Look and startup",
                detail: `${settings.theme === "slate" ? "Slate" : "Fog"} · ${settings.material ? "paper and print" : "plain"} · ${settings.launchAtLogin ? "start at login" : "manual start"}`,
                target: { tab: "settings", stage: "look" },
              },
              ...(app.snapshot.capabilities.sync
                ? [
                    {
                      label: "Account and sync",
                      detail: "Encrypted vocabulary · devices · recovery",
                      target: {
                        tab: "settings" as const,
                        stage: "sync" as const,
                      },
                    },
                  ]
                : []),
              {
                label: "Updates",
                detail: `${app.snapshot.version} · ${settings.updateChannel}`,
                target: { tab: "settings", stage: "updates" },
              },
              {
                label: "Help",
                detail: "Report a problem · command line · setup",
                target: { tab: "settings", stage: "help" },
              },
            ],
          ],
  );
  // Memory and disk details sit one click down, in one Advanced fold.
  const advanced = $derived<Door[]>(
    route.stage
      ? []
      : [
          {
            label: "Memory",
            detail: `${settings.memoryMode === "overlap" ? "Keep all ready" : "Load one at a time"} · ${settings.modelRetention === "exit" ? "unload on quit" : settings.modelRetention === "immediately" ? "unload immediately" : `unload after ${settings.modelRetention.replace("m", " min")}`}`,
            target: { tab: "settings", stage: "memory" },
          },
          {
            label: "On disk",
            detail: `${models.filter(installed).length} models · ${formatBytes(models.filter(installed).reduce((sum, m) => sum + m.diskBytes, 0))}`,
            target: { tab: "settings", stage: "disk" },
          },
        ],
  );
  onMount(() => {
    const release = library.acquire();
    void profiles.check();
    let active = true;
    void ai.refresh().then(async () => {
      if (!active || !ai.data?.connected) return;
      const results = await Promise.allSettled([
        call("list_ai_models"),
        call("list_ai_catalog"),
      ]);
      if (active)
        catalog = results.flatMap((result) =>
          result.status === "fulfilled" ? result.value : [],
        );
    });
    return () => {
      active = false;
      release();
    };
  });
</script>

{#if !route.stage}
  <PageHeader title="Settings" />
{/if}
{#if route.stage === "look"}<LookScreen />
{:else if route.stage === "sync" && app.snapshot.capabilities.sync}<SyncScreen
  />
{:else if route.stage === "memory"}<MemoryScreen />
{:else if route.stage === "disk"}<DiskScreen
    {onnavigate}
    onback={onback ?? (() => onnavigate({ tab: "settings" }))}
    {parentTitle}
  />
{:else if route.stage === "updates"}<UpdatesScreen />
{:else if route.stage === "help"}<HelpScreen
    {ontour}
    onreport={() =>
      onnavigate({ tab: "settings", stage: "help", sheet: "report" })}
  />
{:else}
  {#if route.stage}<div class="stock paper-card ui-card setup-bar">
      <div>
        <strong data-text-surface="emboss"
          >{changes.length
            ? "Custom setup"
            : profiles.hardware
              ? "Recommended setup"
              : "Checking recommended setup…"}</strong
        ><span data-text-surface="flat"
          >{changes.length
            ? `${changes.length} ${changes.length === 1 ? "stage differs" : "stages differ"} across dictation and meetings`
            : "For this computer"}</span
        >
      </div>
      {#if changes.length}<button
          class="button small"
          disabled={profiles.applyLocked}
          onclick={() => (reset = true)}>Back to recommended</button
        >{/if}
    </div>{/if}
  {#each groups as group}<section
      class="stock paper-card ui-card row-card quiet-settings"
    >
      {#each group as door}{@const change = changes.find(
          (item) => item.role === door.role,
        )}
        <button class="settings-link" onclick={() => onnavigate(door.target)}
          ><strong data-text-surface="emboss">{door.label}</strong><span
            class="door-detail"
            data-text-surface="flat"
            >{door.detail}{#if change}<span class="stage-diff"
                ><small class="stamp">Custom</small> recommended: {modelName(
                  change.recommended,
                  models,
                  catalog,
                )}</span
              >{:else if !route.stage && changes.some( (item) => (door.label === "Dictation" ? ["asr", "cleanup"].includes(item.role) : door.label === "Meetings" && ["notes", "meetingSpeech"].includes(item.role)) )}<small
                class="stamp">Custom</small
              >{/if}</span
          ><span class="door-chevron" aria-hidden="true"
            ><Icon name="chevron-right" size={15} /></span
          ></button
        >
      {/each}
    </section>{/each}
  {#if advanced.length}<section
      class="stock paper-card ui-card row-card quiet-settings"
    >
      <Disclosure
        class="advanced"
        title="Advanced"
        detail="Memory · On disk"
        open={remembered.advanced}
        ontoggle={(open) => (remembered.advanced = open)}
      >
        <div class="advanced-doors">
          {#each advanced as door}<button
              class="settings-link"
              onclick={() => onnavigate(door.target)}
              ><strong data-text-surface="emboss">{door.label}</strong><span
                class="door-detail"
                data-text-surface="flat">{door.detail}</span
              ><span class="door-chevron" aria-hidden="true"
                ><Icon name="chevron-right" size={15} /></span
              ></button
            >{/each}
        </div>
      </Disclosure>
    </section>{/if}
{/if}
{#if profiles.failure}<p class="field-help" role="alert">
    {profiles.failure.message}
  </p>
  <button
    class="button small"
    disabled={profiles.applyLocked}
    onclick={() => profiles.use("balanced", true)}
    >Retry recommended setup</button
  >{/if}
{#if profiles.needsCompletion}<p class="field-help">
    Recommended setup needs to finish preparing its models.
  </p>
  <button
    class="button small"
    disabled={profiles.applyLocked}
    onclick={() => profiles.use("balanced", true)}>Continue setup</button
  >{/if}
{#if reset}<RecommendedSetup onclose={() => (reset = false)} />{/if}

<style>
  .setup-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 12px 18px;
    padding: 16px 22px;
  }
  .setup-bar > div {
    display: grid;
    gap: 3px;
  }
  .setup-bar strong {
    font-size: 17px;
    font-weight: 600;
  }
  .setup-bar span {
    font-size: 14px;
    color: var(--soft-ink);
  }
  .door-detail {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 8px;
  }
  .stage-diff {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    width: 100%;
    font-size: 13px;
  }
  .advanced-doors {
    margin-top: -14px;
  }
  :global(.advanced > .disclosure-summary) {
    display: grid;
    grid-template-columns: 200px minmax(0, 1fr) 16px;
    gap: 16px;
  }
  @media (max-width: 760px) {
    :global(.advanced > .disclosure-summary) {
      grid-template-columns: minmax(0, 1fr) 16px;
      gap: 4px 12px;
    }
    :global(.advanced > .disclosure-summary .disclosure-chevron) {
      grid-column: 2;
      grid-row: 1 / span 2;
    }
  }
</style>
