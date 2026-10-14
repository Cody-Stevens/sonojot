<script lang="ts">
  import { recognitionProgressLabel } from "../lib/recognition-progress";
  import { onDestroy, tick } from "svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import { errorMessage } from "../lib/protocol";
  import { smartRecoverySession } from "../lib/smart-cleanup";
  import { presentError } from "../lib/errors";
  import { dictationReady } from "../lib/dictation-control";
  import {
    recordingSystemNotice,
    tabMeetingWarning,
  } from "../lib/meeting-source";
  import { fixSystemAudio, refreshSystemAudio } from "../lib/system-audio.svelte";
  import {
    COLLAPSE_DELAY_MS,
    ScreenTabRequests,
    startTabMeeting,
    tabCapture,
    tabCollapseAllowed,
    tabPreview,
    tabRepairPage,
  } from "../lib/screen-tab";
  import { suffixPreviewOffset } from "../lib/preview";
  import BrandMark from "../brand/BrandMark.svelte";
  import Icon from "./Icon.svelte";
  import Meter from "./Meter.svelte";

  let root: HTMLDivElement;
  let recordButton = $state<HTMLButtonElement>();
  let previewBox = $state<HTMLDivElement>();
  let previewText = $state<HTMLSpanElement>();
  let pointerInside = false;
  let keyboardInside = false;
  let pointerAction = false;
  let collapseTimer: ReturnType<typeof setTimeout> | undefined;
  let hiddenLocally = $state(false);
  let previewOverflows = $state(false);
  let feedback = $state("");
  let meetingConfirmation = $state(false);
  let meetingConsentPending = $state(false);
  let recoveredSession = $state<number | null>(null);
  let dragStart: { x: number; y: number } | null = null;
  let dragging = false;
  let suppressLipClick = false;
  const menubar =
    new URLSearchParams(window.location.search).get("presentation") ===
    "menubar";
  function dragLip(event: PointerEvent): void {
    if (
      !dragStart ||
      dragging ||
      Math.hypot(event.screenX - dragStart.x, event.screenY - dragStart.y) < 6
    )
      return;
    dragStart = null;
    dragging = true;
    suppressLipClick = true;
    cancelCollapse();
    void app
      .perform("screen-tab-drag", () => call("drag_screen_tab"))
      .finally(() => {
        dragging = false;
        pointerInside = false;
        scheduleCollapse();
      });
  }
  const error = $derived(app.error || app.snapshot.error || "");
  const errorPresentation = $derived(presentError(error));
  const repairPage = $derived(tabRepairPage(errorPresentation.action.id));
  const repairLabel = $derived(
    errorPresentation.action.id === "copy-text"
      ? "Copy text"
      : repairPage === "meetings"
        ? "Open meetings"
        : repairPage === "appearance"
          ? "Open appearance"
          : repairPage === "microphone"
            ? "Check microphone"
            : repairPage === "shortcuts"
              ? "Open shortcuts"
              : "Open Models",
  );
  const capture = $derived(tabCapture(app.snapshot));
  const sourceStatus = $derived(
    capture === "meeting" ? app.snapshot.meetingSourceStatus.join(" · ") : "",
  );
  const recording = $derived(capture !== "idle");
  const recoverySession = $derived(smartRecoverySession(app.snapshot));
  const temporary = $derived(app.snapshot.tabTemporary);
  const visible = $derived(app.snapshot.tabVisible && !hiddenLocally);
  const expanded = $derived(menubar ? visible : app.snapshot.tabExpanded);
  const meetingWarning = $derived(
    meetingConfirmation && !recording && visible
      ? tabMeetingWarning(
          app.snapshot.settings.meetingSource,
          app.snapshot.capabilities.systemAudioStatus,
        )
      : null,
  );
  // While recording, a missing other side outranks the live transcript.
  const systemNotice = $derived(
    capture === "meeting"
      ? recordingSystemNotice(app.snapshot.meetingSystemAudio)
      : null,
  );
  const idleRecordBlocked = $derived(
    !app.ready ||
      (app.snapshot.settings.setupProfile !== null &&
        !dictationReady(app.snapshot)) ||
      app.busy("screen-tab-start") ||
      app.busy("screen-tab-meeting") ||
      app.pending.some((key) => key.startsWith("dictation-")),
  );
  const status = $derived(
    capture === "meeting"
      ? "Meeting recording"
      : (recognitionProgressLabel(app.snapshot) ??
          (app.snapshot.state === "loading"
            ? "Preparing microphone"
            : capture === "dictation"
              ? "Listening"
              : capture === "cleaning"
                ? "Cleaning up text"
                : capture === "finishing"
                  ? "Finishing dictation"
                  : feedback || app.snapshot.deliveryNotice || "Idle")),
  );
  const recordLabel = $derived(
    capture === "meeting"
      ? "Stop meeting"
      : capture === "dictation"
        ? "Stop dictation"
        : capture === "cleaning"
          ? "Cleaning up text"
          : capture === "finishing"
            ? "Finishing dictation"
            : "Record",
  );
  const shortcut = $derived(
    capture === "meeting"
      ? ""
      : capture === "cleaning"
        ? app.snapshot.settings.hotkey
        : recording && app.snapshot.settings.hotkeyMode === "hold"
          ? `Release ${app.snapshot.settings.hotkey}`
          : app.snapshot.settings.hotkey,
  );
  const requests = new ScreenTabRequests(
    async (intent) => {
      if (!app.native) {
        app.snapshot.tabVisible = intent.visible;
        app.snapshot.tabExpanded = intent.expanded;
        if (intent.focus || intent.pinned || !intent.visible)
          app.snapshot.tabTemporary = false;
        return;
      }
      app.apply(await call("set_screen_tab", intent));
    },
    (error) => {
      hiddenLocally = false;
      app.error = errorMessage(error);
    },
  );
  $effect(() => {
    if (!app.snapshot.tabVisible || app.snapshot.tabTemporary)
      hiddenLocally = false;
  });
  $effect(() => {
    if (!visible || recording) meetingConfirmation = false;
  });
  $effect(() => {
    if (menubar) return;
    // A shortcut can finish capture while the pointer remains in another app.
    if (expanded && visible) scheduleCollapse();
    else cancelCollapse();
  });
  $effect(() => {
    const box = previewBox;
    const span = previewText;
    if (!box || !span) {
      previewOverflows = false;
      return;
    }
    // The left fade belongs to real overflow only: the newest words sit at the
    // right edge and older text must actually have moved left under the clip.
    const observer = new ResizeObserver(measurePreview);
    observer.observe(box);
    observer.observe(span);
    measurePreview();
    return () => observer.disconnect();
  });
  function measurePreview(): void {
    previewOverflows =
      !!previewBox &&
      !!previewText &&
      suffixPreviewOffset(
        previewText.getBoundingClientRect().width,
        previewBox.clientWidth,
      ) > 0.5;
  }
  function cancelCollapse(): void {
    clearTimeout(collapseTimer);
    collapseTimer = undefined;
  }
  function expand(focus = false): void {
    if (menubar || !visible) return;
    cancelCollapse();
    requests.request({ visible: true, expanded: true, focus });
  }
  function collapseState() {
    return {
      pointerInside,
      keyboardInside,
      visible,
      recording,
      temporary,
      hasError: !!error || !!meetingWarning || meetingConsentPending,
    };
  }
  function scheduleCollapse(): void {
    if (menubar) return;
    cancelCollapse();
    if (dragging || dragStart) return;
    if (!tabCollapseAllowed(collapseState())) return;
    collapseTimer = setTimeout(() => {
      if (tabCollapseAllowed(collapseState()))
        requests.request({ visible: true, expanded: false, focus: false });
    }, COLLAPSE_DELAY_MS);
  }
  function enter(): void {
    if (menubar) return;
    pointerInside = true;
    expand(keyboardInside);
  }
  function leave(): void {
    pointerInside = false;
    scheduleCollapse();
  }
  function focusIn(): void {
    if (pointerAction) return;
    keyboardInside = true;
    expand(true);
  }
  function focusOut(event: FocusEvent): void {
    if (
      event.relatedTarget instanceof Node &&
      root.contains(event.relatedTarget)
    )
      return;
    keyboardInside = false;
    scheduleCollapse();
  }
  function keyboard(event: KeyboardEvent): void {
    pointerAction = false;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      if (meetingConfirmation) {
        dismissMeetingWarning();
        return;
      }
      if (capture === "cleaning") {
        void app.dictation("stop-cleanup");
        return;
      }
      if (menubar) {
        keyboardInside = false;
        cancelCollapse();
        requests.request({ visible: false, expanded: false, focus: false });
        return;
      }
      keyboardInside = false;
      cancelCollapse();
      requests.request({ visible: true, expanded: false, focus: false });
    } else if (
      event.key === "ArrowDown" &&
      event.target === root.querySelector(".tab-lip")
    ) {
      event.preventDefault();
      keyboardInside = true;
      expand(true);
      void tick().then(() =>
        (
          root.querySelector<HTMLButtonElement>(".record-both") ?? recordButton
        )?.focus(),
      );
    }
  }
  async function record(): Promise<void> {
    cancelCollapse();
    expand(keyboardInside);
    feedback = "";
    if (capture === "dictation") await app.dictation("stop");
    else if (capture === "meeting") await meeting();
    else if (capture === "idle" && !idleRecordBlocked)
      await app.perform("screen-tab-start", async () =>
        app.apply(await call("screen_tab_record")),
      );
    scheduleCollapse();
  }
  async function copyLast(): Promise<void> {
    await app.perform("screen-tab-copy", async () => {
      await call("copy_last_dictation");
      feedback = "Last dictation copied.";
    });
  }
  async function copyBeforeAiChanges(): Promise<void> {
    const resultSessionId = recoverySession;
    if (resultSessionId === null) return;
    await app.perform("screen-tab-original", async () => {
      await call("copy_smart_original", { resultSessionId });
      feedback = "Text before AI changes copied.";
      recoveredSession = resultSessionId;
    });
  }
  async function meeting(): Promise<void> {
    if (app.busy("screen-tab-meeting") || app.busy("settings")) return;
    await app.perform("screen-tab-meeting", async () => {
      if (capture === "meeting") {
        await call("stop_meeting");
        return;
      }
      // A long-hidden WebView may have missed a settings event from the app.
      app.apply(await call("get_snapshot"));
      await refreshSystemAudio();
      if (!visible || capture !== "idle") return;
      if (!app.snapshot.settings.meetingConsentAcknowledged) {
        // The native dialog has room for consent and the source warning together.
        // Do not also open the compact source warning underneath it.
        meetingConsentPending = true;
        cancelCollapse();
        try {
          await call("screen_tab_meeting");
        } catch (error: unknown) {
          if (errorMessage(error) !== "MEETING_START_CANCELLED") throw error;
        } finally {
          meetingConsentPending = false;
        }
        app.apply(await call("get_snapshot"));
        scheduleCollapse();
      } else if (
        tabMeetingWarning(
          app.snapshot.settings.meetingSource,
          app.snapshot.capabilities.systemAudioStatus,
        )
      ) {
        meetingConfirmation = true;
        cancelCollapse();
        // Choosing an action keeps its warning open, without taking editor focus.
        requests.request({
          visible: true,
          expanded: true,
          focus: false,
          pinned: true,
        });
      } else {
        await call("screen_tab_meeting");
      }
    });
  }
  async function confirmMeeting(choice: "start" | "primary"): Promise<void> {
    const warning = meetingWarning;
    if (
      !warning ||
      capture !== "idle" ||
      app.busy("screen-tab-meeting") ||
      app.busy("settings")
    )
      return;
    if (choice === "primary" && warning.primary?.action === "fix") {
      // Keep the warning open: once allowed, its button becomes Record both sides.
      await app.perform("screen-tab-meeting", async () => {
        await fixSystemAudio();
      });
      return;
    }
    const meetingSource =
      choice === "primary" ? "both" : warning.start.source;
    await app.perform("screen-tab-meeting", () =>
      startTabMeeting(
        meetingSource !== app.snapshot.settings.meetingSource,
        async () => {
          app.apply(
            await call("save_settings", {
              settings: { ...app.snapshot.settings, meetingSource },
            }),
          );
        },
        async () => {
          // Escape, Hide or a shortcut can arrive while saving the source.
          if (meetingConfirmation && visible && capture === "idle")
            await call("screen_tab_meeting");
        },
      ),
    );
    meetingConfirmation = false;
    scheduleCollapse();
  }
  function dismissMeetingWarning(): void {
    meetingConfirmation = false;
    scheduleCollapse();
  }
  function hide(): void {
    cancelCollapse();
    pointerInside = false;
    keyboardInside = false;
    meetingConfirmation = false;
    hiddenLocally = true;
    requests.request({ visible: false, expanded: false, focus: false });
  }
  function keep(): void {
    cancelCollapse();
    hiddenLocally = false;
    // `pinned` pins an edge reveal passively; editor focus stays put.
    requests.request({
      visible: true,
      expanded: true,
      focus: false,
      pinned: true,
    });
  }
  async function repairError(): Promise<void> {
    if (errorPresentation.action.id === "copy-text") await app.copy(false);
    else
      await app.perform("screen-tab-repair", () =>
        call("show_page", { page: repairPage }),
      );
  }
  onDestroy(() => {
    cancelCollapse();
    requests.dispose();
  });
</script>

{#snippet hideIcon()}
  <svg
    width="16"
    height="16"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.6"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"><path d="M4 4h16M12 20V8m-5 5 5-5 5 5" /></svg
  >
{/snippet}
{#snippet pinIcon()}
  <svg
    width="16"
    height="16"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.6"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    ><path d="M9 4h6M12 4v5.5M7.5 14h9L12 9.5 7.5 14ZM12 14v6" /></svg
  >
{/snippet}
{#snippet errorAction()}
  <div class="tab-error" role="alert">
    <span title={errorPresentation.title}
      >{errorPresentation.family === "setup-incomplete"
        ? "Finish setup"
        : errorPresentation.title}</span
    >
    <button
      type="button"
      disabled={!app.native ||
        app.busy("screen-tab-repair") ||
        app.busy("copy") ||
        (errorPresentation.action.id === "copy-text" &&
          (app.snapshot.resultSessionId === null ||
            !app.snapshot.outputText.trim()))}
      onclick={repairError}>{repairLabel}</button
    >
  </div>
{/snippet}

<svelte:window
  onblur={() => {
    pointerAction = false;
    keyboardInside = false;
    scheduleCollapse();
  }}
  onpointerup={() => {
    pointerAction = false;
  }}
/>
<!-- Native reveal moves this whole canvas. Its region excludes stock above the display and all transparent corners. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions (The group delegates events; every action is a native button.) -->
<div
  bind:this={root}
  class="screen-tab"
  class:native={app.native}
  class:menubar
  class:expanded
  class:recording
  class:hidden={!visible}
  onpointerenter={enter}
  onpointerleave={leave}
  onpointerdown={() => {
    pointerAction = true;
  }}
  onfocusin={focusIn}
  onfocusout={focusOut}
  onkeydown={keyboard}
  role="group"
  aria-label={menubar ? "Dictation controls" : "Screen tab"}
>
  {#if !menubar}<button
      type="button"
      class="tab-lip stock"
      title="Drag to another monitor"
      onpointerdown={(event) => {
        if (event.button !== 0 || !app.native) return;
        suppressLipClick = false;
        dragStart = { x: event.screenX, y: event.screenY };
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onpointermove={dragLip}
      onpointerup={() => {
        dragStart = null;
      }}
      onpointercancel={() => {
        dragStart = null;
      }}
      aria-label="Open dictation controls"
      aria-expanded={expanded}
      aria-controls="screen-tab-controls"
      onclick={(event) => {
        if (!suppressLipClick || event.detail === 0) expand(event.detail === 0);
        suppressLipClick = false;
      }}
      ><BrandMark
        variant="symbol"
        ink={app.snapshot.settings.theme === "slate" ? "dark" : "light"}
      /></button
    >{/if}
  <section
    id="screen-tab-controls"
    class="tab-sheet stock"
    inert={!expanded}
    aria-label="Dictation controls"
  >
    {#if meetingWarning}
      <div class="tab-meeting-warning">
        <div class="tab-warning-copy">
          <span role="alert" data-text-surface="flat">{meetingWarning.text}</span>
          <button
            type="button"
            class="start-anyway"
            disabled={!app.native ||
              app.busy("screen-tab-meeting") ||
              app.busy("settings")}
            onclick={() => confirmMeeting("start")}
            >{meetingWarning.start.label}</button
          >
        </div>
        <div class="tab-actions">
          {#if meetingWarning.primary}<button
              type="button"
              class="record-both"
              disabled={!app.native ||
                app.busy("screen-tab-meeting") ||
                app.busy("settings")}
              onclick={() => confirmMeeting("primary")}
              >{meetingWarning.primary.label}</button
            >{/if}
          <button
            type="button"
            aria-label="Cancel meeting start"
            title="Cancel meeting start"
            disabled={app.busy("screen-tab-meeting")}
            onclick={dismissMeetingWarning}
            ><Icon name="close" size={14} /></button
          >
        </div>
      </div>
    {:else}
      <div class="tab-control-row">
        <div class="tab-lead">
          <button
            bind:this={recordButton}
            type="button"
            class="tab-record"
            aria-label={recordLabel}
            title={recordLabel}
            disabled={!app.native ||
              capture === "finishing" ||
              capture === "cleaning" ||
              (capture === "meeting" && app.busy("screen-tab-meeting")) ||
              (capture === "dictation" &&
                (app.busy("dictation-stop") || app.busy("dictation-cancel"))) ||
              (capture === "idle" && idleRecordBlocked)}
            onclick={record}
            ><Icon name={recording ? "stop" : "microphone"} size={19} /></button
          >
          <div class="tab-heading">
            {#if error && !recording}
              {@render errorAction()}
            {:else if recoverySession !== null}
              <div class="tab-recovery">
                <span data-text-surface="flat" role="status"
                  >{recoveredSession === recoverySession
                    ? "Copied"
                    : "AI changes"}</span
                >
                <button
                  type="button"
                  title="Copy text from before AI cleanup or layout changes"
                  disabled={!app.native || app.busy("screen-tab-original")}
                  onclick={copyBeforeAiChanges}>Copy before AI changes</button
                >
              </div>
            {:else}
              <strong role="status" title={status}>{status}</strong>
              {#if shortcut}<span
                  class="tab-shortcut"
                  data-text-surface="flat"
                  title={capture === "cleaning"
                    ? `${shortcut} to stop cleanup and use available text`
                    : recording
                      ? `${shortcut} to finish dictation`
                      : `Dictation shortcut: ${shortcut}`}>{shortcut}</span
                >{/if}
            {/if}
          </div>
        </div>
        {#if recording}
          {#if capture === "meeting" && app.snapshot.meetingLevels}
            <!-- Two-source meetings show each side, so a silent source is obvious. -->
            <div class="tab-stems">
              <span class="tab-stem" title="Your microphone"
                ><Icon name="microphone" size={13} /><Meter
                  level={app.snapshot.meetingLevels.microphone}
                  active
                  compact
                /></span
              >
              <span class="tab-stem" title="Other speakers (system audio)"
                ><Icon name="meetings" size={13} /><Meter
                  level={app.snapshot.meetingLevels.system}
                  active
                  compact
                /></span
              >
            </div>
          {:else}
            <Meter
              level={app.snapshot.audioLevel}
              active={capture === "dictation" || capture === "meeting"}
              compact
            />
          {/if}
        {/if}
        <div class="tab-trail">
          {#if capture === "cleaning"}<button
              type="button"
              class="tab-cancel"
              title={`Stop cleanup and use available text (${shortcut}; Esc while focused)`}
              disabled={!app.native || app.busy("dictation-stop-cleanup")}
              onclick={() => app.dictation("stop-cleanup")}>Stop cleanup</button
            >{/if}
          {#if capture === "dictation" || capture === "finishing"}<button
              type="button"
              class="tab-cancel"
              title="Cancel dictation without publishing this recording"
              data-help="Cancel dictation without publishing this recording"
              disabled={!app.native || app.busy("dictation-cancel")}
              onclick={() => app.dictation("cancel")}>Cancel</button
            >{/if}
          {#if recording}{#if temporary}<button
                type="button"
                class="tab-pin"
                aria-label="Keep visible"
                title="Keep visible"
                onclick={keep}>{@render pinIcon()}</button
              >{/if}<button
              type="button"
              class="tab-hide"
              aria-label="Hide"
              title={menubar
                ? "Hide panel; keep recording"
                : "Hide screen tab; keep recording"}
              onclick={hide}>{@render hideIcon()}</button
            >{:else}
            <div class="tab-actions">
              <button
                type="button"
                disabled={!app.native ||
                  !app.snapshot.canCopyLast ||
                  app.busy("screen-tab-copy")}
                onclick={copyLast}
                ><Icon name="copy" size={14} />Copy last</button
              >
              <button
                type="button"
                disabled={!app.native ||
                  !app.ready ||
                  app.busy("screen-tab-meeting") ||
                  app.busy("settings") ||
                  app.busy("screen-tab-start") ||
                  app.pending.some((key) => key.startsWith("dictation-"))}
                onclick={meeting}
                ><Icon name="microphone" size={14} />Start meeting</button
              >
              <button
                type="button"
                disabled={!app.native}
                onclick={() =>
                  app.perform("screen-tab-page", () =>
                    call("show_page", { page: "dictation" }),
                  )}><Icon name="app" size={14} />Open app</button
              >
              <!-- The recovery link needs the room, so Keep and Hide drop their
                   labels while it shows, as they do in the recording strip. -->
              {#if temporary}<button
                  type="button"
                  class:icon-only={recoverySession !== null}
                  aria-label={recoverySession !== null
                    ? "Keep visible"
                    : undefined}
                  title="Keep visible"
                  onclick={keep}
                  >{@render pinIcon()}{#if recoverySession === null}Keep{/if}</button
                >{/if}
              <button
                type="button"
                class:icon-only={recoverySession !== null}
                aria-label={recoverySession !== null ? "Hide" : undefined}
                title={recoverySession !== null ? "Hide" : undefined}
                onclick={hide}
                >{@render hideIcon()}{#if recoverySession === null}Hide{/if}</button
              >
            </div>
          {/if}
        </div>
      </div>
      {#if recording}
        <div class="tab-live-row">
          {#if error}{@render errorAction()}{:else if systemNotice}<div
              class="tab-system-alert"
              role="alert"
            >
              <span title={systemNotice.text}>{systemNotice.text}</span>
              {#if systemNotice.fix}<button
                  type="button"
                  disabled={!app.native || app.busy("screen-tab-fix")}
                  onclick={() =>
                    app.perform("screen-tab-fix", async () => {
                      await fixSystemAudio();
                    })}>{systemNotice.fix}</button
                >{/if}
            </div>{:else if sourceStatus}<div
              class="tab-preview"
              data-text-surface="flat"
              role="status"
              aria-label="Recording source status"
              title={sourceStatus}
            >
              <span>{sourceStatus}</span>
            </div>{:else}<div
              bind:this={previewBox}
              class="tab-preview"
              class:overflowing={previewOverflows}
              data-text-surface="flat"
              aria-label="Live transcript"
            >
              <span bind:this={previewText}
                >{tabPreview(app.snapshot.interimText)}</span
              >
            </div>{/if}
        </div>
      {/if}
    {/if}
  </section>
</div>

<style>
  .screen-tab {
    --sheet-height: 48px;
    position: fixed;
    top: 0;
    left: 0;
    width: 600px;
    height: calc(var(--sheet-height) + 26px);
    color: var(--ink);
    font-size: 14px;
    pointer-events: none;
  }
  .screen-tab.recording {
    --sheet-height: 78px;
  }
  .screen-tab.menubar {
    height: var(--sheet-height);
    overflow: hidden;
  }
  .screen-tab.menubar .tab-sheet {
    border-top: 0;
  }
  @media (prefers-reduced-motion: no-preference) {
    .screen-tab.menubar .tab-sheet {
      animation: menu-sheet-drop 200ms cubic-bezier(0.2, 0.7, 0.2, 1);
    }
  }
  @keyframes menu-sheet-drop {
    from {
      transform: translateY(-100%);
    }
    to {
      transform: none;
    }
  }
  .screen-tab.hidden {
    /* display:none is required: descendants like the brand mark img override
       visibility back to visible, and the lip keeps pointer-events:auto. */
    display: none;
  }
  .screen-tab:not(.native) {
    transform: translateY(calc(-1 * var(--sheet-height)));
    transition: transform 200ms cubic-bezier(0.2, 0.7, 0.2, 1);
  }
  .screen-tab:not(.native).expanded {
    transform: translateY(0);
  }
  .tab-lip {
    position: absolute;
    top: var(--sheet-height);
    left: 275px;
    width: 50px;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 10px 2px;
    border: 1px solid var(--dye);
    border-top: 0;
    border-radius: 0 0 8px 8px;
    background-color: var(--dye);
    color: var(--dye-ink);
    box-shadow:
      inset 0 1px 2px var(--contact),
      inset 0 -1px var(--surface-highlight-blur) var(--surface-highlight-spread)
        var(--surface-highlight-color);
    pointer-events: auto;
  }
  .tab-lip :global(.sono-mark) {
    width: 26px;
    height: 24px;
  }
  .tab-sheet {
    position: absolute;
    inset: 0 0 auto;
    width: 600px;
    height: var(--sheet-height);
    padding: 6px 12px;
    border: 1px solid var(--edge);
    border-top: 0;
    border-radius: 0 0 8px 8px;
    box-shadow: inset 0 -2px 0 var(--line);
    pointer-events: auto;
  }
  .tab-sheet[inert] {
    pointer-events: none;
  }
  .tab-control-row {
    display: grid;
    grid-template-columns: minmax(0, max-content) minmax(0, 1fr) auto;
    align-items: center;
    column-gap: 8px;
    height: 34px;
  }
  .tab-lead {
    grid-column: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .screen-tab:not(.recording) .tab-control-row {
    grid-template-columns: minmax(0, 1fr) auto;
  }
  .screen-tab:not(.recording) .tab-trail {
    grid-column: 2;
  }
  .tab-trail {
    grid-column: 3;
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .tab-heading {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 7px;
    line-height: 17px;
  }
  .tab-heading strong {
    flex: 0 1 auto;
    min-width: 0;
    font-size: 13px;
    font-weight: 620;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tab-shortcut {
    flex: 0 1 auto;
    min-width: 0;
    font-size: 11px;
    line-height: 15px;
    color: var(--soft-ink);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tab-recovery {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    line-height: 16px;
    min-width: 0;
    font-size: 12px;
    white-space: nowrap;
  }
  .tab-recovery button {
    border: 0;
    padding: 0;
    background: transparent;
    text-decoration: underline;
    font-size: inherit;
  }
  .tab-record {
    flex: none;
    width: 34px;
    height: 34px;
    padding: 0;
    border: 1px solid var(--edge);
    background: var(--dye);
    color: var(--dye-ink);
    border-radius: 5px;
    display: grid;
    place-items: center;
    box-shadow:
      inset 0 1px var(--surface-highlight-blur) var(--surface-highlight-spread)
        var(--surface-highlight-color),
      inset 0 -1px 0 var(--contact);
  }
  .tab-record:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .tab-record:active:not(:disabled) {
    box-shadow: inset 0 1px 3px var(--contact);
  }
  .tab-cancel {
    flex: none;
    height: 30px;
    padding: 0 8px;
    border: 1px solid var(--edge);
    border-radius: 5px;
    background: var(--well);
    font-size: 13px;
  }
  .tab-hide,
  .tab-pin {
    flex: none;
    width: 28px;
    height: 30px;
    padding: 0;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 4px;
    background: transparent;
  }
  .tab-live-row {
    display: flex;
    align-items: center;
    height: 18px;
    margin-top: 10px;
  }
  .tab-control-row :global(.meter.compact) {
    grid-column: 2;
    justify-self: center;
    width: 78px;
    height: 9px;
    margin: 0;
    gap: 1px;
  }
  .tab-stems {
    grid-column: 2;
    justify-self: center;
    display: grid;
    gap: 3px;
  }
  .tab-stem {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--soft-ink);
  }
  .tab-stems :global(.meter.compact) {
    width: 66px;
    height: 7px;
  }
  .tab-preview {
    flex: 1;
    min-width: 0;
    display: flex;
    justify-content: flex-end;
    overflow: hidden;
    white-space: nowrap;
    font-size: 13px;
    line-height: 18px;
    color: var(--soft-ink);
  }
  .tab-preview.overflowing {
    mask-image: linear-gradient(to right, transparent, #000 9px);
    -webkit-mask-image: linear-gradient(to right, transparent, #000 9px);
  }
  .tab-preview > span {
    flex: none;
    min-width: 100%;
  }
  .tab-actions {
    display: flex;
    align-items: center;
    flex: none;
    gap: 3px;
  }
  .tab-meeting-warning {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 34px;
  }
  .tab-warning-copy {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    line-height: 16px;
  }
  .start-anyway {
    padding: 0;
    border: 0;
    background: transparent;
    font: inherit;
    text-decoration: underline;
    white-space: nowrap;
  }
  .tab-actions button {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    height: 30px;
    padding: 2px 6px;
    border: 0;
    background: transparent;
    font-size: 12px;
    white-space: nowrap;
    border-radius: 4px;
  }
  .tab-actions button.icon-only {
    width: 28px;
    padding: 0;
  }
  .tab-actions button:hover:not(:disabled),
  .tab-hide:hover,
  .tab-pin:hover {
    background: var(--well);
  }
  .tab-meeting-warning .record-both {
    background: var(--dye);
    color: var(--dye-ink);
  }
  .tab-system-alert {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 9px;
    color: var(--danger);
    font-size: 12px;
    font-weight: 620;
    line-height: 18px;
  }
  .tab-system-alert span {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .tab-system-alert button {
    flex: none;
    border: 0;
    padding: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-decoration: underline;
  }
  .tab-error {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 7px;
    color: var(--danger);
    font-size: 11px;
    line-height: 16px;
  }
  .tab-error span {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .tab-error button {
    flex: none;
    border: 0;
    padding: 0;
    background: transparent;
    font-size: 11px;
    text-decoration: underline;
  }
  :global([data-material="off"]) .tab-lip,
  :global([data-material="off"]) .tab-sheet,
  :global([data-material="off"]) .tab-record {
    box-shadow: none;
  }
  button {
    transition:
      background-color 140ms,
      box-shadow 140ms,
      filter 140ms;
  }
  @media (prefers-reduced-motion: reduce) {
    .screen-tab:not(.native),
    button {
      transition: none;
    }
  }
  button:focus-visible {
    outline-offset: -3px;
  }
</style>
