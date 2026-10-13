<script lang="ts">
  import { recognitionProgressLabel } from "./lib/recognition-progress";
  import { onMount, tick } from "svelte";
  import Dictation from "./views/Dictation.svelte";
  import Microphone from "./views/Microphone.svelte";
  import Vocabulary from "./views/Vocabulary.svelte";
  import Meetings from "./views/Meetings.svelte";
  import Appearance from "./views/Appearance.svelte";
  import Shortcuts from "./views/Shortcuts.svelte";
  import About from "./views/About.svelte";
  import Setup from "./views/Setup.svelte";
  import Onboarding from "./views/Onboarding.svelte";
  import Terms from "./views/Terms.svelte";
  import { needsTermsAcceptance } from "./lib/terms";
  import Icon from "./components/Icon.svelte";
  import Confirmation from "./components/Confirmation.svelte";
  import DownloadConsent from "./components/DownloadConsent.svelte";
  import { downloadConsent } from "./lib/download-consent.svelte";
  import BrandMark from "./brand/BrandMark.svelte";
  import { paperScroll } from "./lib/paper-scroll";
  import ErrorBanner from "./components/ErrorBanner.svelte";
  import UpdateButton from "./components/UpdateButton.svelte";
  import type { ErrorAction } from "./lib/errors";
  import ScreenTab from "./components/ScreenTab.svelte";
  import { listen } from "@tauri-apps/api/event";
  import WindowControls from "./components/WindowControls.svelte";
  import CursorIndicator from "./components/CursorIndicator.svelte";
  import { app, STATUS_NOTICE_MS } from "./lib/app.svelte";
  import { call } from "./lib/api";
  import { errorMessage } from "./lib/protocol";
  import {
    controlWindow,
    draggableHeader,
    watchWindowMaximized,
    type WindowAction,
  } from "./lib/window";
  import type { ModelRole } from "./lib/model-library";
  let setupFocus = $state<ModelRole | null>(null);
  let dictationReviewSession = $state<number | null>(null);
  // Status notices clear themselves; errors stay until acted on.
  $effect(() => {
    const shown = app.notice;
    if (!shown) return;
    const timer = setTimeout(() => {
      if (app.notice === shown) app.notice = "";
    }, STATUS_NOTICE_MS);
    return () => clearTimeout(timer);
  });
  type Page =
    | "dictation"
    | "microphone"
    | "vocabulary"
    | "meetings"
    | "appearance"
    | "shortcuts"
    | "about";
  type SetupPage = "setup";
  const navigation = [
    { id: "dictation", label: "Dictation" },
    { id: "meetings", label: "Meetings" },
    { id: "microphone", label: "Microphone" },
    { id: "shortcuts", label: "Shortcuts" },
    { id: "vocabulary", label: "Vocabulary" },
    { id: "appearance", label: "Appearance" },
    { id: "setup", label: "Models" },
    { id: "about", label: "About" },
  ] satisfies { id: Page | SetupPage; label: string }[];
  const query = new URLSearchParams(window.location.search);
  const overlay = query.get("view") === "overlay";
  const cursor = query.get("view") === "cursor";
  let maximized = $state(false);
  let closing = false;
  let onboardingMode = $state<"first-run" | "tour" | null>(null);
  let setupChecked = $state(false);
  const termsRequired = $derived(
    app.native && app.ready && needsTermsAcceptance(app.snapshot.settings),
  );
  let page = $state<Page | SetupPage>(
    navigation.find((item) => item.id === query.get("page"))?.id ?? "dictation",
  );
  let dismissedError = $state("");
  const error = $derived(
    app.error ||
      (app.snapshot.error === dismissedError ? "" : app.snapshot.error || ""),
  );
  const statusText = $derived(
    !app.native
      ? "Browser preview"
      : !app.ready &&
          app.snapshot.state === "idle" &&
          !app.snapshot.meetingRecording
        ? ""
        : app.snapshot.meetingRecording
          ? "Meeting recording"
          : (recognitionProgressLabel(app.snapshot) ??
            {
              idle: "Idle",
              loading: "Preparing…",
              listening: "Listening",
              finishing: "Finishing…",
              cleaning: "Cleaning up text…",
              error: "Needs attention",
            }[app.snapshot.state]),
  );
  const listening = $derived(
    ["loading", "listening"].includes(app.snapshot.state) ||
      app.snapshot.meetingRecording,
  );

  onMount(() => {
    let disposed = false;
    let cleanup: (() => void) | undefined;
    let cleanupWindow: (() => void) | undefined;
    let cleanupNavigation: (() => void) | undefined;
    if (app.native && !overlay && !cursor) {
      void listen<string>("navigate-page", ({ payload }) => {
        const next = navigation.find((item) => item.id === payload);
        if (next) void navigate(next.id);
      }).then((unlisten) => {
        if (disposed) unlisten();
        else cleanupNavigation = unlisten;
      });
    }
    void app.initialize().then(async (unlisten) => {
      if (disposed) unlisten();
      else cleanup = unlisten;
      if (app.native && !overlay && !cursor && app.ready) {
        try {
          if (await call("begin_onboarding")) onboardingMode = "first-run";
        } catch (failure: unknown) {
          app.error = errorMessage(failure);
          if (app.snapshot.settings.setupProfile === null)
            onboardingMode = "first-run";
        }
      }
      setupChecked = true;
    });
    if (!overlay && !cursor) {
      void watchWindowMaximized(
        (value) => {
          maximized = value;
        },
        (error) => {
          app.error = errorMessage(error);
        },
      )
        .then((unlisten) => {
          if (disposed) unlisten();
          else cleanupWindow = unlisten;
        })
        .catch((error: unknown) => {
          app.error = errorMessage(error);
        });
    }
    if (!app.native) {
      if (query.get("theme") === "slate") app.snapshot.settings.theme = "slate";
      if (query.get("material") === "off")
        app.snapshot.settings.material = false;
    }
    return () => {
      disposed = true;
      cleanup?.();
      cleanupWindow?.();
      cleanupNavigation?.();
    };
  });
  async function navigate(next: Page | SetupPage): Promise<void> {
    if (onboardingMode || termsRequired) return;
    if (next === page) return;
    if (
      app.dirty &&
      !(await app.confirm(
        "Leave these changes?",
        "This page has changes that have not been saved.",
      ))
    )
      return;
    setupFocus = null;
    dictationReviewSession = null;
    page = next;
    app.error = "";
    app.notice = "";
    dismissedError = "";
    document.querySelector<HTMLElement>(".page-content")?.scrollTo({ top: 0 });
  }
  async function manageModels(role: ModelRole): Promise<void> {
    await navigate("setup");
    if (page !== "setup") return;
    setupFocus = null;
    await tick();
    setupFocus = role;
  }
  async function reviewMicrophoneTest(resultSessionId: number): Promise<void> {
    await navigate("dictation");
    if (page === "dictation") dictationReviewSession = resultSessionId;
  }
  async function errorAction(action: ErrorAction): Promise<void> {
    if (action === "copy-text") {
      await app.copy(!app.snapshot.interimText && !app.snapshot.outputText);
      return;
    }
    if (action === "retry") {
      await app.initialize();
      return;
    }
    if (action === "restart") {
      app.notice =
        app.snapshot.capabilities.platform === "macos"
          ? "Quit SonoJot from its menu bar, then open it again."
          : "Quit SonoJot from its tray menu, then open it again.";
      return;
    }
    if (
      action === "setup" ||
      action === "converter" ||
      action === "install-media"
    ) {
      await navigate("setup");
      return;
    }
    await navigate(action);
  }
  function keyboard(event: KeyboardEvent): void {
    if (overlay) return;
    if (event.defaultPrevented || app.confirmation) return;
    if (event.key === "Escape" && app.snapshot.state === "cleaning") {
      event.preventDefault();
      void app.dictation("stop-cleanup");
      return;
    }
    if (
      event.key === "Escape" &&
      ["loading", "listening", "finishing"].includes(app.snapshot.state)
    ) {
      event.preventDefault();
      void app.dictation("cancel");
    }
  }
  function beforeUnload(event: BeforeUnloadEvent): void {
    if (app.dirty && !closing) event.preventDefault();
  }
  async function windowAction(action: WindowAction): Promise<void> {
    if (
      action === "close" &&
      app.dirty &&
      !(await app.confirm(
        "Close without saving?",
        "This page has changes that have not been saved.",
        "Close without saving",
      ))
    )
      return;
    await app.perform("window", async () => {
      closing = action === "close";
      try {
        await controlWindow(action);
      } catch (error: unknown) {
        closing = false;
        throw error;
      }
    });
  }
</script>

<svelte:window onkeydown={keyboard} onbeforeunload={beforeUnload} />
<svelte:head
  ><title
    >{overlay
      ? "Recording"
      : navigation.find((item) => item.id === page)?.label} · SonoJot</title
  ></svelte:head
>

{#if cursor}
  <CursorIndicator />
{:else}<div
    class="studio type-material"
    class:overlay
    data-theme={app.snapshot.settings.theme}
    data-material={app.snapshot.settings.material ? "on" : "off"}
  >
    {#if overlay}
      <ScreenTab />
    {:else}
      {#if !onboardingMode}<a class="skip-link" href="#page-content"
          >Skip to page</a
        >{/if}
      <div class="stock sheet">
        <header
          class="sheet-head"
          use:draggableHeader={{
            onMaximize: () => {
              void windowAction("toggleMaximize");
            },
            onError: (error) => {
              app.error = errorMessage(error);
            },
          }}
        >
          <div class="brand">
            <BrandMark variant="lockup" />
          </div>
          <div class="header-end">
            {#if statusText}<div class="folio">
                <span class="connection-status" data-text-surface="flat"
                  ><span class="status-dot" class:recording={listening}
                  ></span>{statusText}</span
                >
              </div>{/if}
            <WindowControls
              {maximized}
              disabled={!app.native || app.busy("window")}
              onaction={windowAction}
            />
          </div>
        </header>
        {#if onboardingMode}
          <Onboarding
            mode={onboardingMode}
            onfinish={() => {
              onboardingMode = null;
              page = "dictation";
            }}
          />
        {:else if app.native && !setupChecked}
          <p class="body-copy" role="status">Starting SonoJot…</p>
        {:else if termsRequired}
          <Terms onclose={() => windowAction("close")} />
        {:else}<div class="sheet-body">
            <aside class="paper-nav">
              <nav aria-label="Main navigation">
                {#snippet navItem(item: (typeof navigation)[number])}<button
                    type="button"
                    class="nav-item"
                    class:selected={page === item.id}
                    aria-current={page === item.id ? "page" : undefined}
                    onclick={() => navigate(item.id)}
                    >{#if item.id === "setup"}<svg
                        width="24"
                        height="24"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.55"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        aria-hidden="true"
                        ><path
                          d="m12 3 9 5-9 5-9-5 9-5Zm-9 9 9 5 9-5M3 16l9 5 9-5"
                        /></svg
                      >
                    {:else}<Icon name={item.id} />{/if}<span>{item.label}</span
                    >{#if item.id === "meetings" && app.snapshot.meetingRecording}<span
                        class="nav-recording"
                        aria-label="Recording"
                      ></span>{/if}</button
                  >{/snippet}
                <div class="nav-primary">
                  {#each navigation.slice(0, 2) as item}{@render navItem(
                      item,
                    )}{/each}
                </div>
                <div
                  class="nav-settings"
                  role="group"
                  aria-labelledby="settings-label"
                >
                  <span
                    id="settings-label"
                    class="nav-group-label"
                    data-text-surface="flat">Settings</span
                  >
                  {#each navigation.slice(2) as item}{@render navItem(
                      item,
                    )}{/each}
                </div>
              </nav>
              <UpdateButton />
            </aside>
            <div class="stock mounted page-frame">
              <main
                id="page-content"
                class="page-content"
                tabindex="-1"
                use:paperScroll
              >
                <div class="page-flow">
                  <div class="page-notices">
                    {#if error}<ErrorBanner
                        {error}
                        canCopyText={app.snapshot.resultSessionId !== null &&
                          !!(
                            app.snapshot.interimText ||
                            app.snapshot.outputText ||
                            app.snapshot.originalText
                          )}
                        onaction={errorAction}
                        ondismiss={() => {
                          dismissedError = app.snapshot.error || "";
                          app.error = "";
                        }}
                      />{/if}
                    {#if app.notice}<div class="notice" role="status">
                        <span>{app.notice}</span><button
                          class="icon-button"
                          aria-label="Dismiss notification"
                          title="Dismiss notification"
                          data-help="Dismiss notification"
                          onclick={() => {
                            app.notice = "";
                          }}><Icon name="close" size={18} /></button
                        >
                      </div>{/if}
                    {#if !app.native}<div class="preview-notice">
                        <strong>Browser preview</strong><span
                          data-text-surface="flat"
                          >Explore the interface. Recording, files and saved
                          settings require the desktop app.</span
                        >
                      </div>{/if}
                  </div>
                  {#if app.native && !app.ready}<p
                      role="status"
                      data-text-surface="flat"
                    >
                      Starting SonoJot…
                    </p>
                  {:else if page === "dictation"}<Dictation
                      onmanage={manageModels}
                      initialView={dictationReviewSession === null
                        ? "preferences"
                        : "recent"}
                      reviewSessionId={dictationReviewSession}
                    />{:else if page === "microphone"}<Microphone
                      onreview={reviewMicrophoneTest}
                    />{:else if page === "vocabulary"}<Vocabulary
                      onmanage={manageModels}
                    />{:else if page === "meetings"}<Meetings
                      onmanage={manageModels}
                      onerroraction={errorAction}
                    />{:else if page === "appearance"}<Appearance
                    />{:else if page === "shortcuts"}<Shortcuts
                    />{:else if page === "setup"}<Setup
                      focusRole={setupFocus}
                      onnavigate={navigate}
                    />{:else}<About
                      ontour={() => {
                        onboardingMode = "tour";
                      }}
                    />{/if}
                </div>
              </main>
            </div>
          </div>{/if}
      </div>
    {/if}
    {#if app.confirmation}<Confirmation />{/if}
    {#if downloadConsent.groups.length}<DownloadConsent />{/if}
  </div>
{/if}
