<script lang="ts">
  import { tick } from "svelte";
  import Sidebar from "./Sidebar.svelte";

  // One label per sidebar destination, drawn beside a live, inert copy of the
  // real sidebar so the picture follows the theme, the paper and any renames.
  const notes = [
    {
      id: "dictation",
      name: "Dictation",
      text: "Dictation settings and recent text",
    },
    {
      id: "meetings",
      name: "Meetings",
      text: "Recordings, transcripts and notes",
    },
    {
      id: "microphone",
      name: "Microphone",
      text: "Input device and level check",
    },
    { id: "shortcuts", name: "Shortcuts", text: "Keys and how they work" },
    { id: "vocabulary", name: "Vocabulary", text: "Names and words you use" },
    {
      id: "appearance",
      name: "Appearance",
      text: "Paper, theme and screen tab",
    },
    {
      id: "setup",
      name: "Models",
      text: "Profiles, downloads and online models",
    },
    { id: "about", name: "About", text: "Version, help and this tour" },
    {
      id: "update",
      name: "Update available",
      text: "When an update is ready: download, then restart.",
    },
  ];
  let frame = $state<HTMLDivElement>();
  let inner = $state<HTMLDivElement>();
  let scale = $state(1);
  let natural = $state({ width: 0, height: 0 });
  let tops = $state<Record<string, number>>({});
  let size = $state(18);

  async function layout(): Promise<void> {
    if (!frame || !inner) return;
    const width = inner.offsetWidth;
    const height = inner.offsetHeight;
    if (!height || !frame.clientHeight) return;
    scale = Math.min(1, frame.clientHeight / height);
    natural = { width, height };
    await tick();
    const origin = frame.getBoundingClientRect().top;
    const next: Record<string, number> = {};
    for (const note of notes) {
      const target = inner.querySelector(
        note.id === "update" ? ".update-button" : `[data-nav="${note.id}"]`,
      );
      if (!target) continue;
      const rect = target.getBoundingClientRect();
      next[note.id] = rect.top + rect.height / 2 - origin;
    }
    const centers = Object.values(next).sort((a, b) => a - b);
    let pitch = Infinity;
    for (let index = 1; index < centers.length; index++)
      pitch = Math.min(pitch, centers[index] - centers[index - 1]);
    // Labels share one size, as large as the closest pair of rows allows.
    size = Math.max(13, Math.min(18, Math.floor(pitch * 0.6)));
    tops = next;
  }

  $effect(() => {
    if (!frame || !inner) return;
    const observer = new ResizeObserver(() => void layout());
    observer.observe(frame);
    observer.observe(inner);
    void document.fonts?.ready.then(() => void layout());
    return () => observer.disconnect();
  });
</script>

<div class="sidebar-tour" bind:this={frame} style:--note-size={`${size}px`}>
  <div
    class="sidebar-shot"
    aria-hidden="true"
    inert
    style:width={natural.width ? `${natural.width * scale}px` : undefined}
    style:height={natural.height ? `${natural.height * scale}px` : undefined}
  >
    <div
      class="sidebar-scale"
      bind:this={inner}
      style:transform={`scale(${scale})`}
    >
      <Sidebar page="dictation" preview />
    </div>
  </div>
  <ul class="sidebar-notes" aria-label="Sidebar sections">
    {#each notes as note (note.id)}
      <li
        class:placed={tops[note.id] !== undefined}
        style:top={tops[note.id] === undefined
          ? undefined
          : `${tops[note.id]}px`}
        data-text-surface="flat"
      >
        <span class="sr-only">{note.name}:</span>
        {note.text}
      </li>
    {/each}
  </ul>
</div>

<style>
  .sidebar-tour {
    flex: 1 1 0;
    min-height: 0;
    display: flex;
    gap: 0;
  }
  .sidebar-shot {
    position: relative;
    flex: none;
    overflow: hidden;
    border: 1px solid var(--line);
    border-radius: 16px;
    box-shadow: inset 0 1px 2px var(--contact);
    pointer-events: none;
    user-select: none;
  }
  .sidebar-scale {
    position: absolute;
    top: 0;
    left: 0;
    width: 220px;
    transform-origin: 0 0;
  }
  .sidebar-scale :global(.paper-nav) {
    overflow: visible;
  }
  .sidebar-notes {
    position: relative;
    flex: 1;
    min-width: 0;
    margin: 0;
    padding: 0 0 0 9px;
    list-style: none;
    font-size: var(--note-size);
    line-height: 1.25;
  }
  .sidebar-notes li {
    padding-left: 45px;
  }
  .sidebar-notes li.placed {
    position: absolute;
    left: 0;
    right: 0;
    transform: translateY(-50%);
  }
  /* A leader runs from the picture's edge to each label. */
  .sidebar-notes li.placed::before {
    content: "";
    position: absolute;
    left: 0;
    top: 50%;
    width: 36px;
    border-top: 1px solid var(--soft-ink);
    opacity: 0.55;
  }
  .sidebar-notes li.placed::after {
    content: "";
    position: absolute;
    left: -3px;
    top: calc(50% - 3px);
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--soft-ink);
    opacity: 0.7;
  }
</style>
