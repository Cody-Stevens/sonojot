<script lang="ts">
  import Rail from "./Rail.svelte";
</script>

<div class="rail-tour">
  <div class="tour-picture" inert aria-hidden="true">
    <Rail route={{ tab: "dictation" }} preview />
  </div>
  <dl>
    <div>
      <dt data-text-surface="emboss">Dictation</dt>
      <dd data-text-surface="flat">
        Recent text, shortcut, microphone and speech settings.
      </dd>
    </div>
    <div>
      <dt data-text-surface="emboss">Meetings</dt>
      <dd data-text-surface="flat">Recordings, transcripts and notes.</dd>
    </div>
    <div>
      <dt data-text-surface="emboss">Words</dt>
      <dd data-text-surface="flat">Names and words you use.</dd>
    </div>
    <div>
      <dt data-text-surface="emboss">Settings</dt>
      <dd data-text-surface="flat">
        Paper, startup, models, updates and help.
      </dd>
    </div>
  </dl>
</div>

<style>
  .rail-tour {
    display: flex;
    align-items: flex-start;
    gap: 27px;
    overflow: auto;
  }
  .tour-picture {
    width: 92px;
    flex: none;
    pointer-events: none;
  }
  dl {
    margin: 12px 0;
    display: grid;
    gap: 10px;
  }
  dl div {
    min-height: 70px;
    display: flex;
    flex-direction: column;
    justify-content: center;
  }
  dt {
    font-weight: 600;
  }
  dd {
    margin: 4px 0 0;
    color: var(--soft-ink);
    font-size: 0.9rem;
  }
</style>
