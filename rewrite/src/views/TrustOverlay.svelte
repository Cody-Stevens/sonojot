<script lang="ts">
  // Draws doubtful-word marks over another app's text field, the popover
  // (card or strip), the fallback chip and panel. Everything it draws is
  // reported as the native window's hit region; the rest of the screen stays
  // click-through. It never takes focus unless the keyboard flow asks.
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import paper from "../brand/material/paper-stock-seamless.png";
  import symbol from "../brand/marks/sonojot-symbol-compact-pine.svg";
  import symbolLight from "../brand/marks/sonojot-symbol-compact-white.svg";
  import {
    decodePcm,
    describe,
    heading,
    optionsFor,
    placeChip,
    placePanel,
    placePopover,
    segments,
    voiceHint,
    type RectF,
    type TrustMark,
    type TrustOption,
    type TrustView,
  } from "../lib/trust";

  let view = $state<TrustView | null>(null);
  let openId = $state<number | null>(null);
  let via = $state<"hover" | "click" | "key" | null>(null);
  let hoverId = $state<number | null>(null);
  let panelOpen = $state(false);
  let selected = $state(0);
  let live = $state("");
  let playing = $state<number | null>(null);
  let progress = $state(0);
  let popEl = $state<HTMLElement | null>(null);
  let chipEl = $state<HTMLElement | null>(null);
  let panelEl = $state<HTMLElement | null>(null);
  let popPos = $state({ x: -9999, y: -9999, above: false, notch: 20 });
  let chipPos = $state({ x: -9999, y: -9999 });
  let panelPos = $state({ x: -9999, y: -9999 });
  let lastFocus = 0;
  let lastAnnounce = 0;
  let hoverTimer: ReturnType<typeof setTimeout> | undefined;
  let leaveTimer: ReturnType<typeof setTimeout> | undefined;

  const open = $derived(view?.marks.find((m) => m.id === openId) ?? null);
  const options = $derived(open ? optionsFor(open) : []);
  const openIndex = $derived(
    view && open ? view.marks.findIndex((m) => m.id === open.id) : -1,
  );
  const listening = $derived(view?.voice != null && view.voice === openId);

  function send(event: Record<string, unknown>) {
    void invoke("trust_ui", { event }).catch(() => {});
  }

  function say(text: string) {
    live = "";
    setTimeout(() => (live = text), 30);
  }

  // ----- the native view -----
  onMount(() => {
    let unlisten: (() => void) | undefined;
    void listen<TrustView>("trust-view", ({ payload }) => apply(payload)).then(
      (u) => {
        unlisten = u;
        send({ type: "ready" });
      },
    );
    return () => unlisten?.();
  });

  function apply(next: TrustView) {
    if (view && next.session !== view.session) {
      close(false, true);
      panelOpen = false;
      clips.clear();
    }
    view = next;
    if (openId !== null && !next.marks.some((m) => m.id === openId)) {
      close(false, true);
    }
    if (next.focus && next.focus.seq !== lastFocus) {
      lastFocus = next.focus.seq;
      const id = Number(next.focus.text);
      if (next.mode === "chip") openPanel(id, true);
      else void openPopover(id, "key");
    }
    if (next.announce && next.announce.seq !== lastAnnounce) {
      lastAnnounce = next.announce.seq;
      say(next.announce.text);
    }
    void tick().then(layout);
  }

  // Popover and panel grow when their audio loads: place them again.
  $effect(() => {
    const els = [popEl, panelEl, chipEl].filter((e): e is HTMLElement => !!e);
    if (!els.length) return;
    const observer = new ResizeObserver(() => layout());
    for (const el of els) observer.observe(el);
    return () => observer.disconnect();
  });

  // ----- geometry and the native hit region -----
  function layout() {
    if (!view) return;
    const area = view.area;
    if (open && popEl && view.mode === "inline") {
      const a = anchorOf(open);
      if (a) {
        const p = placePopover(
          a,
          { w: popEl.offsetWidth, h: popEl.offsetHeight },
          area,
          view.popoverStyle,
        );
        popPos = p;
      }
    }
    if (view.mode === "chip" && view.chip && chipEl) {
      chipPos = placeChip(
        view.chip,
        { w: chipEl.offsetWidth, h: chipEl.offsetHeight },
        area,
      );
      if (panelEl) {
        panelPos = placePanel(
          { ...chipPos, w: chipEl.offsetWidth, h: chipEl.offsetHeight },
          { w: panelEl.offsetWidth, h: panelEl.offsetHeight },
          area,
        );
      }
    }
    requestAnimationFrame(reportRegions);
  }

  function anchorOf(mark: TrustMark): RectF | null {
    return mark.rects[0] ?? null;
  }

  function reportRegions() {
    const rects: RectF[] = [];
    for (const el of document.querySelectorAll<HTMLElement>("[data-region]")) {
      const r = el.getBoundingClientRect();
      if (r.width > 0 && r.height > 0)
        rects.push({
          x: r.left - 4,
          y: r.top - 4,
          w: r.width + 8,
          h: r.height + 8,
        });
    }
    // Bridge the gap between a word and its popover so the pointer can cross.
    if (open && popEl && view?.mode === "inline") {
      const a = anchorOf(open);
      const p = popEl.getBoundingClientRect();
      if (a) {
        const top = Math.min(a.y + a.h, p.bottom);
        const bottom = Math.max(a.y, p.top);
        rects.push({
          x: Math.min(a.x, p.left + popPos.notch - 4),
          y: Math.min(top, bottom),
          w: Math.max(a.w, 30),
          h: Math.abs(bottom - top) + 2,
        });
      }
    }
    send({ type: "regions", rects });
  }

  // ----- popover -----
  async function openPopover(id: number, how: "hover" | "click" | "key") {
    if (!view) return;
    clearTimeout(leaveTimer);
    if (openId !== id) stop();
    openId = id;
    via = how;
    selected = 0;
    send({ type: "opened", mark: id, via: how });
    await tick();
    layout();
    const mark = view.marks.find((m) => m.id === id);
    if (!mark) return;
    if (how === "key") {
      popEl?.querySelector<HTMLElement>(".opt")?.focus();
      say(describe(mark, view.marks.indexOf(mark), view.marks.length));
    }
    if (how === "hover" && view.playOn === "hover" && mark.hasAudio)
      void play(mark.id);
  }

  function close(refocus: boolean, quiet = false) {
    clearTimeout(hoverTimer);
    clearTimeout(leaveTimer);
    stop();
    if (openId !== null && !quiet) send({ type: "closed", refocus });
    openId = null;
    via = null;
    popPos = { x: -9999, y: -9999, above: false, notch: 20 };
    void tick().then(() => requestAnimationFrame(reportRegions));
  }

  function enterMark(id: number) {
    hoverId = id;
    clearTimeout(leaveTimer);
    if (openId === id || via === "key" || listening) return;
    clearTimeout(hoverTimer);
    hoverTimer = setTimeout(() => void openPopover(id, "hover"), 320);
  }

  function leave() {
    hoverId = null;
    clearTimeout(hoverTimer);
    if (via !== "hover" || listening) return;
    clearTimeout(leaveTimer);
    leaveTimer = setTimeout(() => {
      if (via === "hover" && !listening) close(false);
    }, 380);
  }

  function choose(mark: TrustMark, option: TrustOption) {
    stop();
    if (option.keep) send({ type: "keep", mark: mark.id });
    else send({ type: "choose", mark: mark.id, text: option.text });
    if (view?.mode === "chip") return;
    openId = null;
    via = via === "key" ? "key" : null;
    if (via !== "key") {
      popPos = { x: -9999, y: -9999, above: false, notch: 20 };
      void tick().then(() => requestAnimationFrame(reportRegions));
    }
  }

  function next(back = false) {
    send({ type: "next", from: openId, back });
  }

  function keys(event: KeyboardEvent, mark: TrustMark) {
    const items = Array.from(
      (event.currentTarget as HTMLElement).querySelectorAll<HTMLElement>(
        ".opt",
      ),
    );
    const at = items.indexOf(document.activeElement as HTMLElement);
    const focusAt = (i: number) => {
      const j = (i + items.length) % items.length;
      selected = j;
      items[j]?.focus();
      say(
        `${items[j]?.getAttribute("aria-label") ?? ""}, ${j + 1} of ${items.length}`,
      );
    };
    const opts = optionsFor(mark);
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        focusAt(at + 1);
        return;
      case "ArrowUp":
        event.preventDefault();
        focusAt(at - 1);
        return;
      case "ArrowRight":
        event.preventDefault();
        next(false);
        return;
      case "ArrowLeft":
        event.preventDefault();
        next(true);
        return;
      case "Escape":
        event.preventDefault();
        if (view?.mode === "chip") closePanel(true);
        else close(true);
        say("Closed. The mark stays.");
        return;
      case "Enter": {
        event.preventDefault();
        const option = opts[at >= 0 ? at : 0];
        if (option) choose(mark, option);
        return;
      }
    }
    const k = event.key.toLowerCase();
    if (/^[1-3]$/.test(k)) {
      const option = opts.filter((o) => !o.keep)[Number(k) - 1];
      if (option) {
        event.preventDefault();
        choose(mark, option);
      }
    } else if (k === "k") {
      event.preventDefault();
      choose(mark, opts[opts.length - 1]);
    } else if (k === "p" && mark.hasAudio) {
      event.preventDefault();
      void play(mark.id);
    }
  }

  // ----- fallback chip and panel -----
  async function openPanel(id: number | null, fromKeyboard = false) {
    if (!view) return;
    panelOpen = true;
    const mark = view.marks.find((m) => m.id === id) ?? view.marks[0];
    if (!mark) return;
    openId = mark.id;
    via = fromKeyboard ? "key" : "click";
    send({ type: "opened", mark: mark.id, via });
    await tick();
    layout();
    if (fromKeyboard) {
      panelEl?.querySelector<HTMLElement>(".opt")?.focus();
      say(
        `Your last dictation. ${describe(mark, view.marks.indexOf(mark), view.marks.length)}`,
      );
    }
  }

  function closePanel(refocus: boolean) {
    panelOpen = false;
    close(refocus);
    panelPos = { x: -9999, y: -9999 };
  }

  $effect(() => {
    // Keep the panel on an open mark as marks resolve.
    if (
      panelOpen &&
      view &&
      openId !== null &&
      !view.marks.some((m) => m.id === openId)
    ) {
      const first = view.marks[0];
      if (first) void openPanel(first.id, via === "key");
      else closePanel(false);
    }
  });

  // ----- audio -----
  type Clip = {
    samples: Float32Array;
    peaks: number[];
    markStart: number;
    markEnd: number;
    seconds: number;
  };
  const clips = new Map<number, Clip>();
  let context: AudioContext | null = null;
  let source: AudioBufferSourceNode | null = null;
  let started = 0;
  let frame = 0;

  async function clip(id: number): Promise<Clip | null> {
    const cached = clips.get(id);
    if (cached) return cached;
    try {
      const data = await invoke<{
        pcm: string;
        sampleRate: number;
        markStart: number;
        markEnd: number;
        seconds: number;
        peaks: number[];
      } | null>("trust_snippet", { mark: id });
      if (!data) return null;
      const made = {
        samples: decodePcm(data.pcm),
        peaks: data.peaks,
        markStart: data.markStart,
        markEnd: data.markEnd,
        seconds: data.seconds,
      };
      clips.set(id, made);
      return made;
    } catch {
      return null;
    }
  }

  async function play(id: number) {
    if (playing === id) {
      stop();
      return;
    }
    stop();
    const c = await clip(id);
    if (!c || openId !== id) return;
    context ??= new AudioContext({ sampleRate: 16000 });
    const buffer = context.createBuffer(1, c.samples.length, 16000);
    buffer.copyToChannel(c.samples as Float32Array<ArrayBuffer>, 0);
    source = context.createBufferSource();
    source.buffer = buffer;
    source.connect(context.destination);
    source.onended = () => {
      if (playing === id) stop();
    };
    started = context.currentTime;
    source.start();
    playing = id;
    const step = () => {
      if (playing !== id || !context) return;
      progress = Math.min(1, (context.currentTime - started) / c.seconds);
      frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
  }

  function stop() {
    cancelAnimationFrame(frame);
    try {
      source?.stop();
    } catch {
      // already stopped
    }
    source = null;
    playing = null;
    progress = 0;
  }

  // Waveform: peaks, the doubtful span highlighted, a playhead.
  let waveEl = $state<HTMLCanvasElement | null>(null);
  let seconds = $state<Record<number, number>>({});
  async function draw(
    canvas: HTMLCanvasElement,
    id: number,
    at: number,
    active: boolean,
  ) {
    const c = await clip(id);
    if (c) seconds[id] = c.seconds;
    const g = canvas.getContext("2d");
    if (!g) return;
    const style = getComputedStyle(canvas);
    const W = canvas.width;
    const H = canvas.height;
    g.clearRect(0, 0, W, H);
    if (!c) return;
    const x0 = (c.markStart / c.seconds) * W;
    const x1 = (c.markEnd / c.seconds) * W;
    g.fillStyle = style.getPropertyValue("--mk-fill");
    g.fillRect(x0, 0, x1 - x0, H);
    g.fillStyle = style.getPropertyValue("--mk-edge");
    g.fillRect(x0, H - 4, x1 - x0, 4);
    const bw = W / c.peaks.length;
    const ink = style.getPropertyValue("--sj-ink");
    const soft = style.getPropertyValue("--sj-soft");
    c.peaks.forEach((v, i) => {
      const h = Math.max(3, v * (H - 14));
      const x = i * bw;
      g.fillStyle = active && x <= at * W ? ink : soft;
      g.globalAlpha = x >= x0 && x <= x1 ? 1 : 0.55;
      g.fillRect(x + 1, (H - h) / 2 - 2, Math.max(1.5, bw - 2), h);
    });
    g.globalAlpha = 1;
    if (active && at > 0) {
      g.fillStyle = style.getPropertyValue("--sj-dye");
      g.fillRect(at * W - 1.5, 0, 3, H);
    }
  }
  $effect(() => {
    if (waveEl && openId !== null)
      void draw(waveEl, openId, progress, playing === openId);
  });

  const reduceMotion =
    typeof matchMedia === "function" &&
    matchMedia("(prefers-reduced-motion: reduce)").matches;
</script>

<div
  class="trust"
  class:sj-dark={view?.hostDark}
  class:host-dark={view?.hostDark}
  class:reduce={reduceMotion}
  style={`--paper: url(${paper})`}
>
  {#if view?.mode === "inline"}
    {#each view.marks as mark (mark.id)}
      {#if mark.kind === "word"}
        {#each mark.rects as r, i (i)}
          <!-- Marks are for the pointer; the keyboard reaches them through the next-word shortcut. -->
          <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
          <div
            data-region
            class="mk {view.markStyle}"
            class:hot={openId === mark.id || hoverId === mark.id}
            class:cur={openId === mark.id && via === "key"}
            style={`left:${r.x - 1}px;top:${r.y + 1}px;width:${r.w + 2}px;height:${r.h + (view.markStyle === "dots" ? 3 : 1)}px`}
            onmouseenter={() => enterMark(mark.id)}
            onmouseleave={leave}
            onclick={() => void openPopover(mark.id, "click")}
          ></div>
        {/each}
      {:else}
        {#each mark.rects.slice(0, 1) as r (mark.id)}
          <!-- Marks are for the pointer; the keyboard reaches them through the next-word shortcut. -->
          <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
          <div
            data-region
            class="gapmk"
            class:hot={openId === mark.id || hoverId === mark.id}
            class:cur={openId === mark.id && via === "key"}
            style={`left:${r.x}px;top:${r.y + 1}px;width:${r.w}px;height:${r.h + 4}px`}
            onmouseenter={() => enterMark(mark.id)}
            onmouseleave={leave}
            onclick={() => void openPopover(mark.id, "click")}
          >
            <svg
              width="14"
              height={r.h + 4}
              viewBox={`0 0 14 ${r.h + 4}`}
              aria-hidden="true"
            >
              <path
                class="gw"
                d={`M7 ${r.h * 0.18} V ${r.h - 6}`}
                stroke-dasharray="2 2"
              />
              <path
                class="gw tip"
                d={`M2.5 ${r.h + 1} L7 ${r.h - 5} L11.5 ${r.h + 1}`}
              />
            </svg>
          </div>
        {/each}
      {/if}
    {/each}
  {/if}

  {#if view?.mode === "inline" && open}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      bind:this={popEl}
      data-region
      class="pop sj {view.popoverStyle}"
      class:above={popPos.above}
      class:listening
      role="dialog"
      tabindex="-1"
      aria-label={open.kind === "gap"
        ? "Possible missing word"
        : `Check the word ${open.heard}`}
      style={`left:${popPos.x}px;top:${popPos.y}px`}
      onmouseenter={() => clearTimeout(leaveTimer)}
      onmouseleave={leave}
      onkeydown={(e) => keys(e, open)}
    >
      {#if view.popoverStyle === "card"}
        <span class="notch" style={`left:${popPos.notch}px`}></span>
        <div class="head sj-flat">
          <span
            >{heading(open).lead}
            <b class="sj-raised">{heading(open).word}</b></span
          >
          <span class="corner">
            {#if view.marks.length > 1}<button
                class="icon"
                aria-label="Next doubtful word"
                title="Next doubtful word"
                onclick={() => next(false)}>›</button
              >{/if}
            <button
              class="icon"
              aria-label="Close"
              title="Close"
              onclick={() => close(via === "key")}>×</button
            >
          </span>
        </div>
        <div class="listen" aria-hidden={!listening}>
          <i></i><i></i><i></i><span
            >Listening. Say the word, or say “keep”.</span
          >
        </div>
        {#if open.hasAudio}
          <div class="wave">
            <button
              class="play"
              aria-label={playing === open.id
                ? "Stop"
                : "Play the audio around this word"}
              onclick={() => void play(open.id)}
            >
              {#if playing === open.id}<svg
                  viewBox="0 0 14 14"
                  aria-hidden="true"
                  ><rect
                    x="2.5"
                    y="2.5"
                    width="9"
                    height="9"
                    rx="1.5"
                    fill="currentColor"
                  /></svg
                >
              {:else}<svg viewBox="0 0 14 14" aria-hidden="true"
                  ><path d="M3 1.8v10.4L12 7z" fill="currentColor" /></svg
                >{/if}
            </button>
            <canvas
              width="440"
              height="80"
              aria-hidden="true"
              bind:this={waveEl}
            ></canvas>
            <span class="t sj-flat"
              >{seconds[open.id] ? `${seconds[open.id].toFixed(1)}s` : ""}</span
            >
          </div>
        {/if}
        <div class="opts" role="listbox" aria-label="Choices">
          {#each options as option, i (i)}
            {#if option.keep}<div class="sep"></div>{/if}
            <button
              class="opt"
              class:keep={option.keep}
              role="option"
              aria-selected={selected === i}
              aria-label={option.keep
                ? option.label
                : `${option.label}, ${option.source}`}
              onclick={() => choose(open, option)}
              ><span class="n">{option.keep ? "K" : i + 1}</span><span
                >{option.label}</span
              ><span class="src">{option.keep ? "" : option.source}</span
              ></button
            >
          {/each}
        </div>
        {#if view.message}<p class="msg" role="alert">
            {view.message.text}
          </p>{/if}
        <div class="foot sj-flat">
          <span>{voiceHint(view.dictationKey, view.hold)}</span>
        </div>
      {:else}
        <span class="ring">
          <button
            class="play"
            aria-label={playing === open.id
              ? "Stop"
              : "Play the audio around this word"}
            disabled={!open.hasAudio}
            onclick={() => void play(open.id)}
          >
            {#if playing === open.id}<svg viewBox="0 0 14 14" aria-hidden="true"
                ><rect
                  x="2.5"
                  y="2.5"
                  width="9"
                  height="9"
                  rx="1.5"
                  fill="currentColor"
                /></svg
              >
            {:else}<svg viewBox="0 0 14 14" aria-hidden="true"
                ><path d="M3 1.8v10.4L12 7z" fill="currentColor" /></svg
              >{/if}
          </button>
          <svg class="prog" viewBox="0 0 34 34" aria-hidden="true"
            ><circle
              cx="17"
              cy="17"
              r="16"
              fill="none"
              stroke-width="2"
              stroke-dasharray={`${progress * 101} 101`}
            /></svg
          >
        </span>
        {#if listening}<span class="listen strip-listen" aria-hidden="true"
            ><i></i><i></i><i></i></span
          >{/if}
        {#each options as option, i (i)}
          {#if option.keep}<span class="vsep"></span>{/if}
          <button
            class="opt"
            class:keep={option.keep}
            role="option"
            aria-selected={selected === i}
            aria-label={option.keep
              ? option.label
              : `${option.label}, ${option.source}`}
            onclick={() => choose(open, option)}
            >{option.keep && open.kind === "word"
              ? "Keep"
              : option.label}</button
          >
        {/each}
        {#if view.marks.length > 1}<span class="vsep"></span><button
            class="icon"
            aria-label="Next doubtful word"
            title="Next doubtful word"
            onclick={() => next(false)}>›</button
          >{/if}
        {#if view.message}<span class="msg" role="alert"
            >{view.message.text}</span
          >{/if}
      {/if}
    </div>
  {/if}

  {#if view?.mode === "chip" && view.chip && view.marks.length}
    <button
      bind:this={chipEl}
      data-region
      class="chip sj"
      aria-label="Words to check in your last dictation"
      style={`left:${chipPos.x}px;top:${chipPos.y}px`}
      onclick={() => (panelOpen ? closePanel(false) : void openPanel(null))}
    >
      <img class="sym" src={view.hostDark ? symbolLight : symbol} alt="" />{view
        .marks.length} to check
    </button>
    {#if panelOpen && open}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div
        bind:this={panelEl}
        data-region
        class="fbp sj"
        class:listening
        role="dialog"
        tabindex="-1"
        aria-label="Words to check"
        style={`left:${panelPos.x}px;top:${panelPos.y}px`}
        onkeydown={(e) => keys(e, open)}
      >
        <div class="head sj-flat">
          <span>Your last dictation · {view.marks.length} to check</span>
          <span class="corner"
            ><button
              class="icon"
              aria-label="Close"
              title="Close"
              onclick={() => closePanel(via === "key")}>×</button
            ></span
          >
        </div>
        <div class="fbtext">
          {#each segments(view.text, view.marks) as part, i (i)}
            {#if part.mark}
              {@const m = part.mark}
              <button
                class={m.kind === "gap" ? "g-on" : "w-on"}
                class:w-sel={m.id === openId}
                aria-label={m.kind === "gap"
                  ? `Possible missing word after ${m.heard}`
                  : m.heard}
                onclick={() => void openPanel(m.id)}
                >{m.kind === "gap" ? " " : part.text}</button
              >
            {:else}{part.text}{/if}
          {/each}
        </div>
        <div class="head sj-flat sub">
          <span
            >{heading(open).lead}
            <b class="sj-raised">{heading(open).word}</b></span
          >
        </div>
        <div class="listen" aria-hidden={!listening}>
          <i></i><i></i><i></i><span
            >Listening. Say the word, or say “keep”.</span
          >
        </div>
        {#if open.hasAudio}
          <div class="wave">
            <button
              class="play"
              aria-label={playing === open.id
                ? "Stop"
                : "Play the audio around this word"}
              onclick={() => void play(open.id)}
            >
              {#if playing === open.id}<svg
                  viewBox="0 0 14 14"
                  aria-hidden="true"
                  ><rect
                    x="2.5"
                    y="2.5"
                    width="9"
                    height="9"
                    rx="1.5"
                    fill="currentColor"
                  /></svg
                >
              {:else}<svg viewBox="0 0 14 14" aria-hidden="true"
                  ><path d="M3 1.8v10.4L12 7z" fill="currentColor" /></svg
                >{/if}
            </button>
            <canvas
              width="440"
              height="80"
              aria-hidden="true"
              bind:this={waveEl}
            ></canvas>
          </div>
        {/if}
        <div class="opts" role="listbox" aria-label="Choices">
          {#each options as option, i (i)}
            {#if option.keep}<div class="sep"></div>{/if}
            <button
              class="opt"
              class:keep={option.keep}
              role="option"
              aria-selected={selected === i}
              aria-label={option.keep
                ? option.label
                : `${option.label}, ${option.source}`}
              onclick={() => choose(open, option)}
              ><span class="n">{option.keep ? "K" : i + 1}</span><span
                >{option.label}</span
              ><span class="src">{option.keep ? "" : option.source}</span
              ></button
            >
          {/each}
        </div>
        {#if view.message}<p class="msg" role="alert">
            {view.message.text}
          </p>{/if}
        <p class="how sj-flat">
          {view.retype
            ? "This app hides where its text is. A fix is typed in for you while the cursor is still at the end of the dictation."
            : view.canReplace
              ? "This app hides where its text is. A fix is typed into the app for you."
              : "This app can't be changed from here. A choice is copied for you to paste."}
        </p>
        <div class="foot sj-flat">
          <span>{voiceHint(view.dictationKey, view.hold)}</span>
        </div>
      </div>
    {/if}
  {/if}
  <div class="sr-only" aria-live="polite">{live}</div>
</div>

<style>
  :global(html),
  :global(body) {
    background: transparent !important;
    overflow: hidden;
    margin: 0;
  }
  .trust {
    --sj-ink: #28352c;
    --sj-soft: #5b6052;
    --sj-sheet: #f9f7ef;
    --sj-card: #fffdf7;
    --sj-dye: #34432e;
    --sj-dye-ink: #f6efdc;
    --sj-edge: #b3aa90;
    --sj-contact: rgba(48, 43, 23, 0.29);
    --sj-line: #9387693a;
    /* Mirrors --focus in styles.css; no-white.test.ts keeps them equal. */
    --sj-focus: #386d99;
    --sj-well: #bcbba755;
    --sj-light: rgba(255, 255, 241, 0.77);
    --mk-fill: rgba(222, 160, 40, 0.36);
    --mk-fill-hover: rgba(222, 160, 40, 0.5);
    --mk-edge: #a2690f;
    position: fixed;
    inset: 0;
    pointer-events: none;
    font:
      14px/1.45 Stationery,
      "Source Serif 4",
      Georgia,
      serif;
    color: var(--sj-ink);
  }
  .trust.sj-dark {
    --sj-ink: #f3eedf;
    --sj-soft: #c6cbbe;
    --sj-sheet: #2c322c;
    --sj-card: #3b443a;
    --sj-dye: #c7d1b7;
    --sj-dye-ink: #253024;
    --sj-edge: #141a14;
    --sj-contact: rgba(7, 12, 8, 0.65);
    --sj-line: #e8eedc26;
    --sj-focus: #b4d0ea;
    --sj-well: #18201855;
    --sj-light: rgba(243, 238, 223, 0.12);
  }
  .trust.host-dark {
    --mk-fill: rgba(240, 190, 92, 0.3);
    --mk-fill-hover: rgba(240, 190, 92, 0.45);
    --mk-edge: #e9b65a;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  button {
    font: inherit;
    color: inherit;
  }
  [data-region] {
    pointer-events: auto;
  }
  .sj {
    color: var(--sj-ink);
    background-color: var(--sj-sheet);
    background-image: var(--paper);
    background-size: 540px;
    background-repeat: repeat;
    background-blend-mode: multiply;
    box-shadow:
      inset 1px 1px 1px var(--sj-light),
      0 2px 0 var(--sj-edge),
      0 4px 2px -1px var(--sj-contact),
      2px 7px 10px -5px var(--sj-contact);
  }
  .sj-flat {
    color: var(--sj-soft);
  }
  .sj-raised {
    text-shadow: 0 1px 0 var(--sj-light);
    font-weight: 600;
  }
  .mk {
    position: absolute;
    border-radius: 2px;
    cursor: pointer;
    animation: markin 0.2s ease-out;
  }
  .mk.band {
    background: linear-gradient(to top, var(--mk-fill) 0 58%, transparent 58%);
    border-bottom: 2px solid var(--mk-edge);
  }
  .mk.band.hot {
    background: linear-gradient(to top, var(--mk-fill-hover) 0 100%);
  }
  .mk.dots {
    background-image: radial-gradient(
      circle,
      var(--mk-edge) 1.3px,
      transparent 1.6px
    );
    background-size: 5px 4px;
    background-repeat: repeat-x;
    background-position: 0 100%;
  }
  .mk.dots.hot {
    background-color: var(--mk-fill);
  }
  .mk.cur {
    outline: 2px solid var(--sj-focus);
    outline-offset: 2px;
  }
  .gapmk {
    position: absolute;
    cursor: pointer;
    animation: markin 0.2s ease-out;
  }
  .gapmk svg {
    display: block;
    overflow: visible;
  }
  .gw {
    stroke: var(--mk-edge);
    stroke-width: 1.6;
    fill: none;
  }
  .gw.tip {
    stroke-width: 2;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .gapmk.hot .gw {
    stroke-width: 2.4;
  }
  .gapmk.cur {
    outline: 2px solid var(--sj-focus);
    outline-offset: 1px;
    border-radius: 3px;
  }
  @keyframes markin {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }
  .pop {
    position: absolute;
    border-radius: 12px;
    font-size: 15px;
    width: 304px;
    padding: 10px 10px 8px;
  }
  .pop .notch {
    position: absolute;
    width: 14px;
    height: 14px;
    transform: rotate(45deg);
    background-color: var(--sj-sheet);
    top: -7px;
    border-radius: 2px;
    box-shadow: -1px -1px 0 var(--sj-line);
  }
  .pop.above .notch {
    top: auto;
    bottom: -7px;
    box-shadow: 1px 1px 0 var(--sj-edge);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin: 0 2px 8px;
    font-size: 13px;
  }
  .head b {
    font-size: 15px;
    color: var(--sj-ink);
  }
  .head.sub {
    margin-top: 2px;
  }
  .corner {
    display: flex;
    gap: 2px;
  }
  .icon {
    width: 24px;
    height: 24px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--sj-soft);
    cursor: pointer;
    font-size: 17px;
    line-height: 1;
  }
  .icon:hover,
  .icon:focus-visible {
    background: var(--sj-well);
    color: var(--sj-ink);
  }
  .icon:focus-visible {
    outline: 2px solid var(--sj-focus);
    outline-offset: 1px;
  }
  .play {
    width: 34px;
    height: 34px;
    flex: none;
    border-radius: 50%;
    border: 0;
    cursor: pointer;
    background: var(--sj-dye);
    color: var(--sj-dye-ink);
    box-shadow:
      inset 1px 1px 1px #fff3,
      0 1px 0 var(--sj-edge),
      1px 3px 4px -2px var(--sj-contact);
    display: grid;
    place-items: center;
  }
  .play:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .play:active {
    box-shadow: inset 0 2px 2px var(--sj-contact);
  }
  .play svg {
    width: 14px;
    height: 14px;
  }
  .wave {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
  }
  .wave canvas {
    flex: 1;
    min-width: 0;
    height: 40px;
    border-radius: 6px;
    background: var(--sj-well);
    box-shadow: inset 0 2px 2px
      color-mix(in srgb, var(--sj-contact) 60%, transparent);
  }
  .wave .t {
    font-size: 12px;
    width: 30px;
    text-align: right;
  }
  .opts {
    display: grid;
    gap: 3px;
  }
  .opt {
    width: 100%;
    display: grid;
    grid-template-columns: 22px 1fr auto;
    align-items: baseline;
    gap: 6px;
    text-align: left;
    border: 0;
    border-radius: 7px;
    padding: 6px 8px;
    background: transparent;
    cursor: pointer;
    font-size: 16px;
    color: var(--sj-ink);
  }
  .opt .n,
  .opt .src {
    font-size: 12px;
    color: var(--sj-soft);
  }
  /* The highlighted choice is raised paper: the card's own stock, embossed,
     never a flat light fill. Keyboard focus adds the focus ring around it. */
  .opt:hover,
  .opt:focus-visible {
    background-color: var(--sj-card);
    background-image: var(--paper);
    background-size: 540px;
    background-blend-mode: multiply;
    box-shadow:
      inset 1px 1px 1px var(--sj-light),
      0 1px 0 var(--sj-edge),
      1px 3px 4px -2px var(--sj-contact);
    outline: none;
  }
  .opt:focus-visible {
    outline: 2px solid var(--sj-focus);
    outline-offset: 2px;
  }
  .opt.keep {
    font-size: 14px;
    color: var(--sj-soft);
  }
  .sep {
    height: 1px;
    background: var(--sj-line);
    margin: 4px 6px;
  }
  .foot {
    font-size: 12px;
    margin: 6px 4px 0;
  }
  .msg {
    font-size: 13px;
    margin: 6px 4px 0;
    color: var(--sj-ink);
  }
  .listen {
    display: none;
    align-items: center;
    gap: 8px;
    padding: 8px;
    border-radius: 8px;
    background: var(--sj-well);
    margin-bottom: 8px;
    font-size: 13px;
  }
  .listening .listen {
    display: flex;
  }
  .listen i {
    display: inline-block;
    width: 4px;
    height: 14px;
    background: var(--sj-dye);
    border-radius: 2px;
    animation: lv 0.6s ease-in-out infinite alternate;
  }
  .listen i:nth-child(2) {
    animation-delay: 0.15s;
  }
  .listen i:nth-child(3) {
    animation-delay: 0.3s;
  }
  @keyframes lv {
    from {
      transform: scaleY(0.3);
    }
    to {
      transform: scaleY(1.1);
    }
  }
  .pop.strip {
    width: auto;
    padding: 4px;
    border-radius: 99px;
    display: flex;
    align-items: center;
    gap: 2px;
    white-space: nowrap;
  }
  .pop.strip .play {
    width: 30px;
    height: 30px;
  }
  .pop.strip .opt {
    display: inline-block;
    width: auto;
    padding: 4px 10px;
    border-radius: 99px;
    font-size: 15px;
  }
  .pop.strip .opt.keep {
    font-size: 13px;
  }
  .pop.strip .msg {
    margin: 0 8px;
  }
  .ring {
    position: relative;
  }
  .ring svg.prog {
    position: absolute;
    inset: -2px;
    width: 34px;
    height: 34px;
    pointer-events: none;
    transform: rotate(-90deg);
  }
  .ring svg.prog circle {
    stroke: var(--sj-dye);
  }
  .strip-listen {
    display: flex;
    margin: 0 4px;
    padding: 4px 6px;
  }
  .vsep {
    width: 1px;
    height: 18px;
    background: var(--sj-line);
  }
  .chip {
    position: absolute;
    border-radius: 99px;
    padding: 4px 11px 4px 6px;
    display: flex;
    align-items: center;
    gap: 6px;
    border: 0;
    cursor: pointer;
    font-size: 13.5px;
    white-space: nowrap;
  }
  .chip .sym {
    width: 18px;
    height: 18px;
  }
  .chip:focus-visible {
    outline: 2px solid var(--sj-focus);
    outline-offset: 2px;
  }
  .fbp {
    position: absolute;
    width: 360px;
    border-radius: 12px;
    padding: 12px;
  }
  .fbtext {
    font-size: 15px;
    line-height: 1.6;
    padding: 8px 10px;
    border-radius: 8px;
    background: var(--sj-well);
    box-shadow: inset 0 2px 2px
      color-mix(in srgb, var(--sj-contact) 50%, transparent);
    margin: 6px 0 10px;
    max-height: 140px;
    overflow: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--sj-soft) transparent;
    white-space: pre-wrap;
  }
  .fbtext button {
    border: 0;
    padding: 0;
    background: none;
    cursor: pointer;
    font: inherit;
    color: inherit;
  }
  .w-on {
    background: linear-gradient(to top, var(--mk-fill) 0 52%, transparent 52%);
    border-bottom: 2px solid var(--mk-edge) !important;
  }
  .w-sel {
    outline: 2px solid var(--sj-focus);
    outline-offset: 1px;
    border-radius: 2px;
  }
  .g-on {
    display: inline-block;
    width: 12px;
    height: 1em;
    vertical-align: -2px;
    background: linear-gradient(var(--mk-edge), var(--mk-edge)) center / 2px
      100% no-repeat;
  }
  .how {
    font-size: 12px;
    margin: 6px 0 0;
  }
  @media (prefers-reduced-motion: reduce) {
    .trust *,
    .trust *::before,
    .trust *::after {
      animation: none !important;
      transition: none !important;
    }
  }
  .reduce *,
  .reduce *::before,
  .reduce *::after {
    animation: none !important;
    transition: none !important;
  }
</style>
