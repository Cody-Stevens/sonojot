<script lang="ts">
  // Doubtful words: marks on the few words SonoJot doubts, where you pasted.
  // Kept in its own file so other Settings work does not collide with it.
  import { onMount, tick } from "svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import SegmentedControl from "../components/quiet/SegmentedControl.svelte";
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
  const sensitivityOptions = [
    { value: "fewer" as const, label: "Fewer" },
    { value: "balanced" as const, label: "Balanced" },
    { value: "more" as const, label: "More" },
  ];
  const markOptions = [
    { value: "band" as const, label: "Marker band" },
    { value: "dots" as const, label: "Dotted underline" },
  ];
  const popoverOptions = [
    { value: "card" as const, label: "Card" },
    { value: "strip" as const, label: "One line" },
  ];
  const playOptions = [
    { value: "hover" as const, label: "On hover" },
    { value: "click" as const, label: "On click" },
  ];
</script>

<section
  class="stock paper-card ui-card row-card trust-settings"
  aria-label="Doubtful words settings"
>
  <SettingRow label="Mark doubtful words" layout="action">
    {#snippet info()}
      <p>
        After a paste, SonoJot highlights the few words it is unsure of right in
        the app you pasted into. Hover or click a highlight to hear what you
        said, pick a fix, or keep it.
      </p>
      <p>
        Highlights stay on their words while you edit elsewhere and disappear
        once resolved. When an app does not expose word positions, nothing pops
        up. Nothing leaves this computer.
      </p>
    {/snippet}
    <Toggle
      label="Mark doubtful words"
      checked={trust.enabled}
      disabled={locked}
      onchange={(enabled) => save({ enabled })}
    />
  </SettingRow>
  {#if trust.enabled}
    <SettingRow label="How many" layout="choice">
      <div class="trust-choice">
        <SegmentedControl
          label="How many marks"
          options={sensitivityOptions}
          value={trust.sensitivity}
          disabled={locked}
          onchange={(sensitivity) => save({ sensitivity })}
        />
        <small class="field-help" data-text-surface="flat"
          >{howMany[trust.sensitivity]}</small
        >
      </div>
    </SettingRow>
    <SettingRow label="Mark" layout="choice">
      <SegmentedControl
        label="Mark style"
        options={markOptions}
        value={trust.markStyle}
        disabled={locked}
        onchange={(markStyle) => save({ markStyle })}
      />
    </SettingRow>
    <SettingRow label="Choices" layout="choice">
      <SegmentedControl
        label="Choices style"
        options={popoverOptions}
        value={trust.popoverStyle}
        disabled={locked}
        onchange={(popoverStyle) => save({ popoverStyle })}
      />
    </SettingRow>
    <SettingRow label="Play the audio" layout="choice">
      <SegmentedControl
        label="Play the audio"
        options={playOptions}
        value={trust.playOn}
        disabled={locked}
        onchange={(playOn) => save({ playOn })}
      />
    </SettingRow>
    <SettingRow label="Next doubtful word" layout="choice">
      {#snippet info()}
        <p>
          Jumps to the next doubtful word in your last dictation with the
          keyboard. Arrow keys move between words and choices, Enter replaces, K
          keeps, P plays the audio, Escape closes. Also in the tray menu: Check
          doubtful words.
        </p>
      {/snippet}
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
    </SettingRow>
    <SettingRow label="On in these apps" layout="choice">
      {#if apps.length}<div class="app-list">
          {#each apps as id (id)}<div class="app-row">
              <span>{appName(id)}</span><Toggle
                label={`Doubtful words in ${appName(id)}`}
                checked={!trust.offApps.includes(id)}
                disabled={locked}
                onchange={(on) => setApp(id, on)}
              />
            </div>{/each}
        </div>{:else}<small class="field-help" data-text-surface="flat"
          >Apps you dictate into appear here.</small
        >{/if}
    </SettingRow>
  {/if}
</section>

<style>
  .trust-choice {
    display: grid;
    gap: 8px;
  }
  .app-list {
    display: grid;
    gap: 10px;
    width: 100%;
    max-width: 320px;
  }
  .app-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 18px;
  }
</style>
