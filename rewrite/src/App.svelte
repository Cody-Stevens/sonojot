<script lang="ts">
  import { recognitionProgressLabel } from "./lib/recognition-progress";
  import { onMount, tick } from "svelte";
  import DictationTab from "./views/DictationTab.svelte";
  import CleanupScreen from "./views/CleanupScreen.svelte";
  import OpenRouterSheet from "./components/quiet/OpenRouterSheet.svelte";
  import { openRouter } from "./lib/openrouter.svelte";
  import InputScreen from "./views/InputScreen.svelte";
  import OutputScreen from "./views/OutputScreen.svelte";
  import TrustScreen from "./views/TrustScreen.svelte";
  import Vocabulary from "./views/Vocabulary.svelte";
  import MeetingsTab from "./views/MeetingsTab.svelte";
  import MeetingNotesScreen from "./views/MeetingNotesScreen.svelte";
  import ReportView from "./views/Report.svelte";
  import {
    shouldShowCrash,
    crashHeading,
    type PendingCrash,
  } from "./lib/reports";
  import SpeechScreen from "./views/SpeechScreen.svelte";
  import Settings from "./views/Settings.svelte";
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
  import Rail from "./components/Rail.svelte";
  import Sheet from "./components/quiet/Sheet.svelte";
  import StageHeader from "./components/quiet/StageHeader.svelte";
  import UpdateSheet from "./components/quiet/UpdateSheet.svelte";
  import WhatsNewSheet from "./components/quiet/WhatsNewSheet.svelte";
  import { whatsNew } from "./lib/whats-new.svelte";
  import {
    createRouter,
    resolveRoute,
    backRoute,
    routeTitle,
    sameRoute,
    type RouteTarget,
  } from "./lib/route.svelte";
  import type { ErrorAction } from "./lib/errors";
  import ScreenTab from "./components/ScreenTab.svelte";
  import PermissionHelper from "./components/PermissionHelper.svelte";
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
  import type { ModelsTarget } from "./lib/model-library";
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
  const query = new URLSearchParams(window.location.search);
  const syncTest = query.get("flavor") === "synctest";
  const overlay = query.get("view") === "overlay";
  const cursor = query.get("view") === "cursor";
  // macOS permission helper floating beside System Settings.
  const helper = query.get("view") === "permission";
  let maximized = $state(false);
  let closing = false;
  let onboardingMode = $state<"first-run" | "tour" | null>(null);
  let setupChecked = $state(false);
  let reportCrash = $state<PendingCrash | null>(null);
  let shownCrashId = $state<string | null>(null);
  const termsRequired = $derived(
    app.native && app.ready && needsTermsAcceptance(app.snapshot.settings),
  );
  const router = createRouter(
    syncTest
      ? { tab: "settings", stage: "sync" }
      : (resolveRoute(query.get("page")) ?? { tab: "dictation" }),
    beforeNavigate,
  );
  const route = $derived(
    router.current.tab === "settings" &&
      router.current.stage === "sync" &&
      !app.snapshot.capabilities.sync
      ? { tab: "settings" as const }
      : router.current,
  );
  const parentTitle = $derived(
    route.from === "settings" || route.tab === "settings"
      ? "Settings"
      : route.tab === "meetings"
        ? "Meetings"
        : "Dictation",
  );
  $effect(() => {
    if (
      shouldShowCrash(
        app.pendingCrash,
        setupChecked,
        onboardingMode !== null,
        termsRequired,
      ) &&
      app.pendingCrash &&
      shownCrashId !== app.pendingCrash.id
    ) {
      shownCrashId = app.pendingCrash.id;
      const crash = app.pendingCrash;
      void navigate("report").then((arrived) => {
        if (arrived) reportCrash = crash;
      });
    }
  });
  // After an update, What's new takes the place the crash notice used to,
  // once, after setup and terms. A real crash report still comes first.
  let whatsNewShown = false;
  $effect(() => {
    if (
      !whatsNewShown &&
      whatsNew.state.pending &&
      setupChecked &&
      onboardingMode === null &&
      !termsRequired &&
      !app.pendingCrash &&
      !overlay &&
      !cursor &&
      !helper
    ) {
      whatsNewShown = true;
      whatsNew.showInstalled();
    }
  });
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
    if (app.native && !overlay && !cursor && !helper) {
      void listen<string>("navigate-page", ({ payload }) => {
        if (resolveRoute(payload)) void navigate(payload);
      }).then((unlisten) => {
        if (disposed) unlisten();
        else cleanupNavigation = unlisten;
      });
    }
    void app.initialize().then(async (unlisten) => {
      if (disposed) unlisten();
      else cleanup = unlisten;
      if (app.native && !overlay && !cursor && !helper && app.ready) {
        try {
          if (await call("begin_onboarding")) onboardingMode = "first-run";
        } catch (failure: unknown) {
          app.error = errorMessage(failure);
          if (app.snapshot.settings.setupProfile === null)
            onboardingMode = "first-run";
        }
        try {
          await app.checkPendingCrash();
        } catch (failure: unknown) {
          app.error = errorMessage(failure);
        }
        try {
          await whatsNew.check();
        } catch {
          // What's new is a courtesy; the app works without it.
        }
      }
      setupChecked = true;
    });
    if (!overlay && !cursor && !helper) {
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
  async function beforeNavigate(): Promise<boolean> {
    if (onboardingMode || termsRequired) return false;
    if (
      app.dirty &&
      !(await app.confirm(
        "Leave these changes?",
        "This page has changes that have not been saved.",
      ))
    )
      return false;
    if (route.sheet === "report" && reportCrash) {
      try {
        await app.dismissPendingCrash();
      } catch (failure: unknown) {
        app.error = errorMessage(failure);
        return false;
      }
    }
    return true;
  }
  async function navigate(next: RouteTarget | string): Promise<boolean> {
    const resolved = resolveRoute(next);
    if (resolved && sameRoute(resolved, route)) return true;
    const closingSheet = !!route.sheet && !resolved?.sheet;
    if (!(await router.go(next))) return false;
    reportCrash = null;
    dictationReviewSession = null;
    app.error = "";
    app.notice = "";
    dismissedError = "";
    await tick();
    if (!closingSheet) {
      const content = document.querySelector<HTMLElement>(".page-content");
      content?.scrollTo({ top: 0 });
      if (!route.sheet) content?.focus({ preventScroll: true });
    }
    return true;
  }
  async function manageModels(target: ModelsTarget): Promise<void> {
    if (target === "punctuation")
      await navigate({ tab: "settings", stage: "disk" });
    else if (target === "notes")
      await navigate({ tab: "meetings", stage: "notes", from: route.from });
    else if (target === "meeting" || target === "speakers")
      await navigate({ tab: "meetings", stage: "speech", from: route.from });
    else
      await navigate({
        tab: "dictation",
        stage:
          target === "cleanup"
            ? "cleanup"
            : target === "dictationAudio"
              ? "output"
              : "speech",
        from: route.from,
      });
  }
  async function reviewMicrophoneTest(resultSessionId: number): Promise<void> {
    if (await navigate("dictation")) dictationReviewSession = resultSessionId;
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
    if (action === "converter" || action === "install-media") {
      await navigate({ tab: "settings", stage: "disk" });
      return;
    }
    await navigate(action);
  }
  function keyboard(event: KeyboardEvent): void {
    if (overlay || helper) return;
    if (
      event.defaultPrevented ||
      app.confirmation ||
      document.querySelector("dialog[open]")
    )
      return;
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
        if (closing && route.sheet === "report" && reportCrash)
          await app.dismissPendingCrash();
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
    >{helper ? "Permission helper" : overlay ? "Recording" : routeTitle(route)} ·
    SonoJot</title
  ></svelte:head
>

{#if cursor}
  <CursorIndicator />
{:else}<div
    class="studio type-material"
    class:overlay={overlay || helper}
    data-theme={app.snapshot.settings.theme}
    data-material={app.snapshot.settings.material ? "on" : "off"}
  >
    {#if overlay}
      <ScreenTab />
    {:else if helper}
      <PermissionHelper />
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
            {#if syncTest}<span data-text-surface="flat">Sync Test</span>{/if}
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
              void navigate("dictation");
            }}
          />
        {:else if app.native && !setupChecked}
          <p class="body-copy" role="status">Starting SonoJot…</p>
        {:else if termsRequired}
          <Terms onclose={() => windowAction("close")} />
        {:else}<div class="sheet-body">
            <Rail {route} onnavigate={navigate} />
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
                  {#if route.stage || route.from}
                    <StageHeader
                      title={route.sheet === "report"
                        ? "Help"
                        : routeTitle(route)}
                      parent={parentTitle}
                      heading={route.tab === "settings" ||
                        (route.tab === "meetings" &&
                          ["notes", "speech"].includes(route.stage ?? "")) ||
                        (route.tab === "dictation" &&
                          [
                            "input",
                            "output",
                            "trust",
                            "cleanup",
                            "speech",
                          ].includes(route.stage ?? ""))}
                      onback={() => void navigate(backRoute(route))}
                    />
                  {/if}
                  {#if app.native && !app.ready}<p
                      role="status"
                      data-text-surface="flat"
                    >
                      Starting SonoJot…
                    </p>
                  {:else if route.tab === "settings"}<Settings
                      {route}
                      onnavigate={navigate}
                      ontour={() => {
                        onboardingMode = "tour";
                      }}
                    />
                  {:else if route.tab === "dictation" && route.stage === "input"}<InputScreen
                      onreview={reviewMicrophoneTest}
                    />
                  {:else if route.tab === "dictation" && route.stage === "output"}<OutputScreen
                    />
                  {:else if route.tab === "dictation" && route.stage === "trust"}<TrustScreen
                    />
                  {:else if route.tab === "dictation" && route.stage === "cleanup"}<CleanupScreen
                      onmanage={manageModels}
                    />
                  {:else if (route.tab === "dictation" || route.tab === "meetings") && route.stage === "speech"}{#key route.tab}<SpeechScreen
                        meeting={route.tab === "meetings"}
                      />{/key}
                  {:else if route.tab === "meetings" && route.stage === "notes"}<MeetingNotesScreen
                      onmanage={manageModels}
                    />
                  {:else if route.tab === "dictation"}<DictationTab
                      onnavigate={navigate}
                      reviewSessionId={dictationReviewSession}
                    />{:else if route.tab === "words"}<Vocabulary
                      onmanage={manageModels}
                    />{:else if route.tab === "meetings"}<MeetingsTab
                      onnavigate={navigate}
                      onmanage={manageModels}
                      onerroraction={errorAction}
                    />{/if}
                </div>
              </main>
            </div>
          </div>{/if}
      </div>
    {/if}
    {#if route.sheet === "report" && (!app.native || setupChecked) && !termsRequired && !overlay && !helper && !onboardingMode}
      <Sheet
        title={reportCrash
          ? crashHeading(reportCrash.reason)
          : "Report a problem"}
        onclose={() => void navigate(backRoute(route))}
      >
        {#key reportCrash?.id ?? "bug"}<ReportView
            embedded
            crash={reportCrash}
            onclose={async () => {
              await navigate(backRoute(route));
            }}
          />{/key}
      </Sheet>
    {/if}
    {#if openRouter.request}{#key openRouter.request.role}<OpenRouterSheet
        />{/key}{/if}
    {#if app.confirmation}<Confirmation />{/if}
    {#if downloadConsent.groups.length}<DownloadConsent />{/if}
    {#if route.sheet === "update" && !overlay && !helper}
      <UpdateSheet onclose={() => void navigate(backRoute(route))} />
    {/if}
    {#if whatsNew.mode && !overlay && !helper}<WhatsNewSheet />{/if}
  </div>
{/if}
