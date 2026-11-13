<script lang="ts">
  // Doubtful words: marks on the few words SonoJot doubts, where you pasted.
  // Kept in its own file so other Settings work does not collide with it.
  import { onMount, tick } from "svelte";
  import Toggle from "../components/Toggle.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import { safeShortcut, shortcutHelp } from "../lib/shortcut";
  import type { TrustSettings } from "../lib/protocol";

  const settings = $derived(app.snapshot.settings);
  const trust = $derived(settings.trust);
  const locked = $derived(!app.native || app.busy("settings"));
  let recent = $state<string[]>([]);
  let capturing = $state(false);
  let keyButton = $state<HTMLButtonElement | null>(null);

  onMount(() => {
    if (!app.native) return;
    void call("trust_recent_apps")
      .then((apps) => (recent = apps))
      .catch(() => {});
  });

  function save(patch: Partial<TrustSettings>) {
    void app.saveSettings({ trust: { ...trust, ...patch } });
  }

  const apps = $derived(
    [...new Set([...trust.offApps, ...recent])].sort((a, b) =>
      a.localeCompare(b),
    ),
  );
  const appName = (id: string) => id.replace(/\.exe$/i, "");

  function setApp(id: string, on: boolean) {
    const off = new Set(trust.offApps);
    if (on) off.delete(id);
    else off.add(id);
    save({ offApps: [...off] });
  }

  async function listen() {
    capturing = true;
    await tick();
    keyButton?.focus();
  }
  function capture(event: KeyboardEvent) {
    if (!capturing) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.key === "Escape") {
      capturing = false;
      return;
    }
    if (event.key === "Backspace" || event.key === "Delete") {
      capturing = false;
      save({ nextShortcut: "" });
      return;
    }
    if (["Control", "Alt", "Shift", "Meta"].includes(event.key) || event.repeat)
      return;
    const modifiers = [
      event.ctrlKey ? "Ctrl" : "",
      event.altKey ? "Alt" : "",
      event.shiftKey ? "Shift" : "",
      event.metaKey ? "Super" : "",
    ].filter(Boolean);
    const key =
      event.code === "Space"
        ? "Space"
        : /^Key[A-Z]$/.test(event.code)
          ? event.code.slice(3)
          : /^Digit\d$/.test(event.code)
            ? event.code.slice(5)
            : event.key;
    const shortcut = [...modifiers, key].join("+");
    if (!safeShortcut(shortcut)) return;
    capturing = false;
    save({ nextShortcut: shortcut });
  }

  const howMany = {
    fewer: "Only the likeliest mistakes.",
    balanced: "Usually none to three marks in a 60-word dictation.",
    more: "Also closer calls. A few more marks.",
  } as const;
</script>

<section
  class="stock paper-card quiet-panel trust-settings"
  aria-label="Doubtful words settings"
>
  <div class="stage-setting wide-label">
    <span
      >Mark doubtful words
      <button
        class="info"
        type="button"
        aria-label="About doubtful words"
        title="After a paste, SonoJot marks the words it is unsure of in the app you pasted into. It reads back only the text it just pasted, and where it is on screen. Nothing leaves this computer."
        >i</button
      ></span
    >
    <div class="stage-control end">
      <Toggle
        label="Mark doubtful words"
        checked={trust.enabled}
        disabled={locked}
        onchange={(enabled) => save({ enabled })}
      />
    </div>
  </div>
  {#if trust.enabled}
    <div class="stage-setting">
      <span>How many</span>
      <div class="stage-control">
        <div class="segments" role="group" aria-label="How many marks">
          {#each [{ value: "fewer", label: "Fewer" }, { value: "balanced", label: "Balanced" }, { value: "more", label: "More" }] as const as option}<button
              disabled={locked}
              aria-pressed={trust.sensitivity === option.value}
              onclick={() => save({ sensitivity: option.value })}
              >{option.label}</button
            >{/each}
        </div>
        <small data-text-surface="flat">{howMany[trust.sensitivity]}</small>
      </div>
    </div>
    <div class="stage-setting">
      <span>Mark</span>
      <div class="stage-control">
        <div class="segments" role="group" aria-label="Mark style">
          {#each [{ value: "band", label: "Marker band" }, { value: "dots", label: "Dotted underline" }] as const as option}<button
              disabled={locked}
              aria-pressed={trust.markStyle === option.value}
              onclick={() => save({ markStyle: option.value })}
              >{option.label}</button
            >{/each}
        </div>
      </div>
    </div>
    <div class="stage-setting">
      <span>Choices</span>
      <div class="stage-control">
        <div class="segments" role="group" aria-label="Choices style">
          {#each [{ value: "card", label: "Card" }, { value: "strip", label: "One line" }] as const as option}<button
              disabled={locked}
              aria-pressed={trust.popoverStyle === option.value}
              onclick={() => save({ popoverStyle: option.value })}
              >{option.label}</button
            >{/each}
        </div>
      </div>
    </div>
    <div class="stage-setting">
      <span>Play the audio</span>
      <div class="stage-control">
        <div class="segments" role="group" aria-label="Play the audio">
          {#each [{ value: "hover", label: "On hover" }, { value: "click", label: "On click" }] as const as option}<button
              disabled={locked}
              aria-pressed={trust.playOn === option.value}
              onclick={() => save({ playOn: option.value })}
              >{option.label}</button
            >{/each}
        </div>
      </div>
    </div>
    <div class="stage-setting">
      <span
        >Next doubtful word
        <button
          class="info"
          type="button"
          aria-label="About the next-word shortcut"
          title="Opens the first doubtful word of your last dictation, or brings its marks back. Arrow keys move between words and choices, Enter replaces, Escape closes. Also in the tray menu: Check doubtful words."
          >i</button
        ></span
      >
      <div class="stage-control">
        <button
          bind:this={keyButton}
          class="key-well"
          class:capturing
          disabled={locked}
          aria-label="Change the next doubtful word shortcut"
          onkeydown={capture}
          onblur={() => (capturing = false)}
          onclick={() => void listen()}
          >{#if capturing}Press new keys…{:else if trust.nextShortcut}{#each trust.nextShortcut.split("+") as key}<kbd
                >{key}</kbd
              >{/each}{:else}None{/if}</button
        >{#if capturing}<span class="field-help"
            >{shortcutHelp} Backspace clears it.</span
          >{/if}
      </div>
    </div>
    <div class="stage-setting apps">
      <span>On in these apps</span>
      {#if apps.length}<div class="app-list">
          {#each apps as id (id)}<div class="app-row">
              <span>{appName(id)}</span><Toggle
                label={`Doubtful words in ${appName(id)}`}
                checked={!trust.offApps.includes(id)}
                disabled={locked}
                onchange={(on) => setApp(id, on)}
              />
            </div>{/each}
        </div>{:else}<small data-text-surface="flat"
          >Apps you dictate into appear here.</small
        >{/if}
    </div>
  {/if}
</section>

<style>
  .info {
    width: 18px;
    height: 18px;
    margin-left: 6px;
    border-radius: 50%;
    border: 1px solid var(--line);
    background: transparent;
    color: var(--soft-ink);
    font:
      italic 600 11px/1 Stationery,
      Georgia,
      serif;
    cursor: help;
    vertical-align: 2px;
  }
  .apps {
    align-items: start;
  }
  .app-list {
    display: grid;
    gap: 6px;
    min-width: 220px;
  }
  .app-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 18px;
  }
</style>
