<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import Sheet from "./Sheet.svelte";
  import Toggle from "../Toggle.svelte";
  import { app } from "../../lib/app.svelte";
  import { call } from "../../lib/api";
  import { hasPhrase, replaceWord } from "../../lib/dictation-history";
  import type { DictationRecord } from "../../lib/protocol";
  let {
    record,
    phrase = "",
    onchanged,
    onclose,
  }: {
    record: DictationRecord;
    phrase?: string;
    onchanged: (record: DictationRecord) => void;
    onclose: () => void;
  } = $props();
  let heard = $state(untrack(() => phrase));
  let written = $state("");
  let always = $state(true);
  let saved = $state(false);
  let failure = $state("");
  const corrected = $derived(replaceWord(record.published, heard, written));
  const inRaw = $derived(hasPhrase(record.raw, heard));
  const dirty = $derived(!!written.trim() || heard !== phrase);
  const valid = $derived(
    !!written.trim() &&
      (always ? inRaw : corrected !== null && corrected !== record.published),
  );
  const busy = $derived(
    app.busy("history-correct") || app.busy("history-learn"),
  );
  $effect(() => {
    app.dirty = dirty;
  });
  onDestroy(() => {
    app.dirty = false;
  });
  async function close() {
    if (busy) return;
    if (
      dirty &&
      !(await app.confirm(
        "Leave these changes?",
        "This word correction has changes that have not been saved.",
      ))
    )
      return;
    onclose();
  }
  async function save() {
    if (!valid && !saved) return;
    failure = "";
    if (!saved && corrected !== null && corrected !== record.published) {
      const text = corrected;
      const result = await app.perform("history-correct", () =>
        call("save_dictation_correction", {
          id: record.id,
          text,
          scope: null,
        }),
      );
      if (!result) {
        failure = app.error;
        return;
      }
      onchanged(result.record);
      saved = true;
    }
    if (always) {
      const result = await app.perform("history-learn", () =>
        call("learn_correction", {
          id: record.id,
          heard: heard.trim(),
          written: written.trim(),
        }),
      );
      if (!result) {
        failure = `${saved ? "The dictation was corrected, but the word could not be saved. " : ""}${app.error}`;
        return;
      }
    }
    app.notice = always
      ? saved
        ? "Dictation corrected. Word added to Words."
        : "Word added to Words."
      : "Dictation corrected.";
    onclose();
  }
</script>

<Sheet title="Fix a word" onclose={() => void close()}>
  <form
    onsubmit={(event) => {
      event.preventDefault();
      void save();
    }}
  >
    <div class="word-pair">
      <label
        >When it hears<input
          bind:value={heard}
          maxlength="250"
          disabled={busy || saved}
          placeholder="Word or phrase"
        /></label
      >
      <span aria-hidden="true">→</span>
      <label
        >Write it as<input
          bind:value={written}
          maxlength="250"
          disabled={busy || saved}
          placeholder="Your correction"
        /></label
      >
    </div>
    {#if heard.trim() && !saved && corrected === null && written.trim()}<p
        class="field-help"
      >
        {always && inRaw
          ? "This phrase is only in As heard. Saving adds it to Words; the final text stays as it is."
          : "Use a phrase in the final text."}
      </p>{/if}
    {#if always && heard.trim() && !inRaw}<p class="field-help">
        To reuse this word, choose a phrase in the As heard version.
      </p>{/if}
    <div class="word-footer">
      <Toggle
        label="Always, in every dictation"
        checked={always}
        disabled={busy || saved}
        onchange={(value) => (always = value)}
      />
      <span>Always, in every dictation</span>
      <button
        class="button primary"
        type="submit"
        disabled={!app.native || busy || (!saved && !valid)}
        >{busy ? "Saving…" : "Save"}</button
      >
    </div>
    {#if failure}<p role="alert">{failure}</p>{/if}
  </form>
</Sheet>

<style>
  .word-pair {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  label {
    flex: 1;
    min-width: 0;
  }
  input {
    width: 100%;
    margin-top: 6px;
  }
  .word-footer {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 24px;
  }
  .word-footer .button {
    margin-left: auto;
  }
</style>
