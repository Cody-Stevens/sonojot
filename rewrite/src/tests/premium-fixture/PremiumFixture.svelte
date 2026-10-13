<script lang="ts">
  import Card from "../../components/Card.svelte";
  import SettingRow from "../../components/SettingRow.svelte";
  import Toggle from "../../components/Toggle.svelte";
  import WindowControls from "../../components/WindowControls.svelte";
  import Confirmation from "../../components/Confirmation.svelte";
  import BrandMark from "../../brand/BrandMark.svelte";
  import { app } from "../../lib/app.svelte";

  const query = new URLSearchParams(window.location.search);
  const theme = query.get("theme") === "slate" ? "slate" : "fog";
  const material = query.get("material") === "off" ? "off" : "on";
  let enabled = $state(true);
  let maximized = $state(false);
  let mode = $state("Clean");
  let result = $state("");
  const history = [
    "A short dictation with the words you meant.",
    "A longer dictation that deliberately exceeds one line so that keyboard focus reveals the whole sentence, including this final phrase.",
  ];
</script>

<div class="studio" data-theme={theme} data-material={material}>
  <div class="stock sheet">
    <header class="sheet-head">
      <div class="brand"><BrandMark /><span>SonoJot</span></div>
      <div class="header-end">
        <div class="folio">
          <span class="connection-status" data-text-surface="flat"
            ><span class="status-dot recording"></span>Listening</span
          >
        </div>
        <WindowControls
          {maximized}
          disabled={false}
          onaction={(action) => {
            if (action === "toggleMaximize") maximized = !maximized;
          }}
        />
      </div>
    </header>
    <div class="sheet-body">
      <aside class="paper-nav">
        <nav aria-label="Fixture navigation">
          {#each ["Dictation", "Microphone", "Vocabulary", "Meetings", "Appearance", "Shortcuts", "About"] as name}
            <button class="nav-item" class:selected={name === "Dictation"}
              >{name}</button
            >
          {/each}
        </nav>
      </aside>
      <div class="stock mounted page-frame">
        <main id="page-content" class="page-content" tabindex="-1">
          <header class="page-heading">
            <h1 data-text-surface="emboss">A place for your words.</h1>
            <p data-text-surface="flat">Shared controls, paper and spacing.</p>
          </header>
          <Card title="How your words arrive">
            <div class="segments" role="group" aria-label="Text handling">
              {#each [{ name: "Verbatim", help: "Every spoken word" }, { name: "Clean", help: "A light edit" }, { name: "Code", help: "For your editor" }] as item}
                <button
                  aria-pressed={mode === item.name}
                  onclick={() => {
                    mode = item.name;
                  }}
                >
                  {item.name}<small data-text-surface="flat">{item.help}</small>
                </button>
              {/each}
            </div>
            <p class="mode-help" data-text-surface="flat">
              Keep the meaning and the details while giving punctuation a little
              room.
            </p>
            <div class="paper-well">The words you meant, ready to keep.</div>
            <SettingRow
              label="Paste when finished"
              help="Send finished text to the app you are writing in."
            >
              <Toggle
                label="Paste when finished"
                checked={enabled}
                onchange={(value) => {
                  enabled = value;
                }}
              />
            </SettingRow>
            <SettingRow
              label="Sound cues"
              help="A short sound at the start and finish."
            >
              <Toggle
                label="Sound cues"
                checked={!enabled}
                onchange={(value) => {
                  enabled = !value;
                }}
              />
            </SettingRow>
            <SettingRow
              label="Stop after silence"
              help="Choose how long to wait after you stop speaking."
            >
              <select aria-label="Stop after silence"
                ><option>Only when I finish</option><option>5 seconds</option
                ></select
              >
            </SettingRow>
          </Card>
          <Card title="Recent dictations">
            <ul class="history-list" aria-label="Recent dictations">
              {#each history as text}<li>
                  <button class="history-item" title={text}
                    ><span>{text}</span><small data-text-surface="flat"
                      >Sep 11, 8:30 AM</small
                    ></button
                  >
                </li>{/each}
            </ul>
          </Card>
          <Card title="A deliberate action">
            <p class="body-copy" data-text-surface="flat">
              The confirmation keeps its focus inside the dialog and returns it
              to the action that opened it.
            </p>
            <div class="actions">
              <button
                class="button primary"
                onclick={async () => {
                  result = (await app.confirm(
                    "Remove this dictation?",
                    "This removes the saved dictation from recent history.",
                    "Remove dictation",
                    true,
                  ))
                    ? "Removed"
                    : "Kept";
                }}>Open confirmation</button
              >
              <button class="button">Keep a copy</button>
              <button class="button" disabled>Unavailable</button>
            </div>
            {#if result}<p
                role="status"
                class="field-help"
                data-text-surface="flat"
              >
                {result}
              </p>{/if}
          </Card>
        </main>
        <footer class="paper-footer" data-text-surface="flat">
          <span>Your words, on your device.</span><span>SonoJot fixture</span>
        </footer>
      </div>
    </div>
  </div>
  {#if app.confirmation}<Confirmation />{/if}
</div>
