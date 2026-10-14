<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import BrandMark from "../brand/BrandMark.svelte";
  import Card from "../components/Card.svelte";
  import Meter from "../components/Meter.svelte";
  import ModelDownload from "../components/ModelDownload.svelte";
  import ProfileRecommendation from "../components/ProfileRecommendation.svelte";
  import RemoteModelBadge from "../components/RemoteModelBadge.svelte";
  import SidebarTour from "../components/SidebarTour.svelte";
  import TermsAcceptance from "../components/TermsAcceptance.svelte";
  import Shortcuts from "./Shortcuts.svelte";
  import MacPasteAccess from "../components/MacPasteAccess.svelte";
  import SystemAudioSetup from "../components/SystemAudioSetup.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import { errorMessage } from "../lib/protocol";
  import type { AiSettings } from "../lib/ai";
  import { library } from "../lib/model-library.svelte";
  import { profiles } from "../lib/setup-profiles.svelte";
  import {
    checking,
    formatBytes,
    installed,
    installing,
  } from "../lib/model-library";
  import {
    profileLabel,
    setupProfiles,
    type SetupProfile,
  } from "../lib/setup-profiles";
  import {
    canContinue,
    dwellProgress,
    microphoneConfirmed,
    onboardingSteps,
    onboardingDownloadIds,
    onboardingModelDetails,
    MODEL_DETAILS_TIMEOUT,
    practiceResult,
    stepProgress,
    shouldApplyTourProfile,
  } from "../lib/onboarding";
  import {
    PUNCTUATION_MODEL,
    activePunctuation,
    defaultCleanupName,
  } from "../lib/punctuation";
  import { scrollEdges } from "../lib/scroll-edges";
  import { needsTermsAcceptance } from "../lib/terms";
  import { acceptTerms } from "../lib/terms.svelte";
  import { ensureDownloadConsent } from "../lib/download-consent.svelte";
  import type { SetupStatus } from "../lib/setup";

  let { mode, onfinish }: { mode: "first-run" | "tour"; onfinish: () => void } =
    $props();
  let step = $state(0);
  const stepId = $derived(onboardingSteps[step].id);
  let termsAgreed = $state(
    untrack(() => !needsTermsAcceptance(app.snapshot.settings)),
  );
  let elapsed = $state(0);
  let entered = Date.now();
  let selected = $state<SetupProfile>(
    untrack(() =>
      mode === "tour"
        ? (app.snapshot.settings.setupProfile ?? "balanced")
        : "balanced",
    ),
  );
  let confirmed = $state<SetupProfile | null>(null);
  let busy = $state(false);
  let error = $state("");
  let level = $state(0);
  let micActive = $state(false);
  let micConfirmation = $state<"detected" | "confirmed" | null>(null);
  const micConfirmed = $derived(micConfirmation !== null);
  let micError = $state("");
  let micBusy = $state(false);
  let samples = 0;
  let micGeneration = 0;
  let micTimer: ReturnType<typeof setTimeout> | undefined;
  let aiSettings = $state<AiSettings | null>(null);
  let aiDetailsFailed = $state(false);
  let detailsExpired = $state(false);
  let practiceText = $state("");
  let practiceBaseline = $state<number | null>(null);
  let practiceStarted = false;
  let setupFinished = false;
  let mediaStatus = $state<SetupStatus | null>(null);
  // Consent review, then the download itself. Neither holds Continue: FFmpeg
  // is optional and keeps installing while the rest of setup goes on.
  let mediaPhase = $state<"consent" | "installing" | null>(null);
  const mediaBusy = $derived(mediaPhase !== null);
  let mediaError = $state("");
  const mediaAvailable = $derived(
    mediaStatus?.mediaImport.state === "installed" ||
      mediaStatus?.mediaImport.state === "ready",
  );
  const mediaInstallable = $derived(
    app.native &&
      !mediaBusy &&
      !mediaAvailable &&
      mediaStatus !== null &&
      mediaStatus.mediaImport.state !== "unsupported" &&
      mediaStatus.mediaImport.state !== "loading" &&
      mediaStatus.mediaImport.state !== "checking",
  );
  const mediaState = $derived(
    !app.native
      ? "FFmpeg installs in the desktop app."
      : mediaPhase === "consent"
        ? "Review the license to continue the download."
        : mediaPhase === "installing" ||
            mediaStatus?.mediaImport.state === "loading"
          ? "Downloading and installing FFmpeg…"
          : mediaAvailable
            ? "FFmpeg is installed."
            : !mediaStatus || mediaStatus.mediaImport.state === "checking"
              ? "Checking for FFmpeg…"
              : mediaStatus.mediaImport.state === "unsupported"
                ? "Automatic install is unavailable on this computer."
                : mediaStatus.mediaImport.state === "failed"
                  ? "The FFmpeg on this computer could not be used."
                  : "Not installed.",
  );
  // Speaker labels queue behind the profile downloads (one install at a time).
  const SPEAKER_MODEL = "nemotron-3-diarization";
  let speakerBusy = $state(false);
  let speakerRequested = $state(false);
  let speakerError = $state("");
  const speakerModel = $derived(
    library.data?.models.find((model) => model.id === SPEAKER_MODEL),
  );
  const speakerOffered = $derived(
    !!speakerModel &&
      (installed(speakerModel) ||
        installing(speakerModel) ||
        checking(speakerModel) ||
        speakerModel.installable),
  );
  const speakerInstallable = $derived(
    app.native &&
      !!speakerModel &&
      speakerModel.installable &&
      !installed(speakerModel) &&
      !installing(speakerModel) &&
      !checking(speakerModel) &&
      !speakerBusy &&
      !app.snapshot.meetingRecording,
  );
  const speakerState = $derived(
    !speakerModel
      ? ""
      : !app.native
        ? "Speaker labels install in the desktop app."
        : installed(speakerModel)
          ? "Installed. New meeting transcripts label each voice."
          : speakerBusy
            ? "Preparing the download…"
            : speakerModel.state === "queued"
              ? "Queued. It downloads after your models."
              : speakerModel.state === "downloading"
                ? `Downloading${speakerModel.totalBytes > 0 ? ` · ${Math.min(100, Math.floor((speakerModel.downloadedBytes / speakerModel.totalBytes) * 100))}%` : "…"}`
                : installing(speakerModel)
                  ? "Installing…"
                  : checking(speakerModel)
                    ? "Checking…"
                    : speakerModel.state === "failed"
                      ? "The download did not finish."
                      : "Not installed.",
  );
  const selection = $derived(
    selected === "custom"
      ? { ...app.snapshot.settings, reasons: [] }
      : profiles.hardware?.recommendations[selected],
  );
  const models = $derived(library.data?.models ?? []);
  // Same wording as Setup: setup always queues the punctuation model.
  const cleanupLine = $derived.by(() => {
    if (!library.data) return missingDetails;
    const punctuation = models.find((model) => model.id === PUNCTUATION_MODEL);
    const status =
      punctuation && checking(punctuation)
        ? "Checking…"
        : activePunctuation(models)
          ? "Installed"
          : punctuation?.installable
            ? "Download included"
            : "";
    return `${defaultCleanupName(models)}${status ? ` · ${status}` : ""}`;
  });
  const modelDetails = $derived(
    onboardingModelDetails(
      selection,
      library.data?.models ?? null,
      selected === "custom",
      aiSettings,
    ),
  );
  const detailsLoading = $derived(
    !detailsExpired &&
      ((!library.data && library.loading && !library.error) ||
        (!selection && profiles.checking && !profiles.hardwareError) ||
        (selected === "custom" && !aiSettings && !aiDetailsFailed) ||
        (modelDetails.downloadBytes === null &&
          models.some((model) => model.state === "checking"))),
  );
  const missingDetails = $derived(
    detailsLoading ? "Reading model details…" : "Model details unavailable",
  );
  let systemAudioResolved = $state(false);
  const canAdvance = $derived(
    canContinue(step, elapsed, micConfirmed, termsAgreed) &&
      (stepId !== "meetings" || systemAudioResolved) &&
      !busy &&
      mediaPhase !== "consent" &&
      !speakerBusy &&
      !micBusy &&
      !app.busy("settings") &&
      !app.dirty,
  );
  const dwell = $derived(dwellProgress(step, elapsed));
  const progress = $derived(stepProgress(step));
  const downloadIds = $derived([
    app.snapshot.settings.asrModel,
    PUNCTUATION_MODEL,
    ...(aiSettings?.backend === "openrouter"
      ? []
      : [app.snapshot.settings.notesModel]),
    ...(speakerRequested ? [SPEAKER_MODEL] : []),
  ]);
  const speechReady = $derived(
    models.some(
      (model) =>
        model.id === app.snapshot.settings.asrModel && installed(model),
    ),
  );
  const mac = $derived(app.snapshot.capabilities.platform === "macos");
  const punctuationReady = $derived(
    models.some(
      (model) =>
        model.id === PUNCTUATION_MODEL &&
        (installed(model) || !model.installable),
    ),
  );
  const practiceReady = $derived(speechReady && punctuationReady);
  const publishedPractice = $derived(
    practiceResult(app.snapshot, practiceBaseline),
  );

  $effect(() => {
    if (stepId === "ready" && publishedPractice !== null)
      practiceText = publishedPractice;
  });
  $effect(() => {
    if (stepId !== "models") return;
    detailsExpired = false;
    const timer = setTimeout(() => {
      detailsExpired = true;
    }, MODEL_DETAILS_TIMEOUT);
    return () => clearTimeout(timer);
  });

  $effect(() => {
    if ((stepId !== "extras" && stepId !== "ready") || !app.native) return;
    if (untrack(() => mediaBusy)) return;
    let disposed = false;
    void call("get_setup")
      .then((value) => {
        if (!disposed) mediaStatus = value;
      })
      .catch((failure: unknown) => {
        if (!disposed) mediaError = errorMessage(failure);
      });
    return () => {
      disposed = true;
    };
  });

  onMount(() => {
    const release = library.acquire();
    void profiles.check();
    let mounted = true;
    void call("get_ai_settings")
      .then((value) => {
        if (mounted) aiSettings = value;
      })
      .catch(() => {
        if (mounted) aiDetailsFailed = true;
      });
    const timer = setInterval(() => {
      if (document.visibilityState === "visible")
        elapsed = Date.now() - entered;
      else entered = Date.now() - elapsed;
    }, 100);
    return () => {
      clearInterval(timer);
      mounted = false;
      release();
      void stopMicrophone();
      profiles.draft = null;
    };
  });

  async function stopMicrophone(): Promise<void> {
    micGeneration++;
    clearTimeout(micTimer);
    micActive = false;
    level = 0;
    try {
      await call("onboarding_microphone", { action: "stop" });
    } catch (failure: unknown) {
      micError = errorMessage(failure);
    }
  }
  async function startMicrophone(): Promise<void> {
    micBusy = true;
    await stopMicrophone();
    const generation = micGeneration;
    micError = "";
    samples = 0;
    try {
      await call("onboarding_microphone", { action: "start" });
      if (generation !== micGeneration || stepId !== "microphone") {
        await stopMicrophone();
        return;
      }
      micActive = true;
      const poll = async () => {
        try {
          const result = await call("onboarding_microphone", {
            action: "level",
          });
          if (generation !== micGeneration) return;
          level = result.level;
          micActive = result.active;
          samples = microphoneConfirmed(level, samples);
          if (samples >= 3) micConfirmation = "detected";
          if (result.active) micTimer = setTimeout(() => void poll(), 200);
        } catch (failure: unknown) {
          if (generation === micGeneration) {
            micActive = false;
            micError = errorMessage(failure);
          }
        }
      };
      void poll();
    } catch (failure: unknown) {
      micError = errorMessage(failure);
    } finally {
      micBusy = false;
    }
  }
  async function changeMicrophone(value: string): Promise<void> {
    await stopMicrophone();
    micConfirmation = null;
    await app.saveSettings({ microphone: value || null });
    await startMicrophone();
  }
  async function move(next: number): Promise<void> {
    if (
      app.dirty &&
      !(await app.confirm(
        "Leave these changes?",
        "Your shortcut has changes that have not been saved.",
      ))
    )
      return;
    if (stepId === "microphone") await stopMicrophone();
    step = next;
    if (onboardingSteps[next].id === "ready" && !practiceStarted) {
      practiceStarted = true;
      practiceBaseline = app.snapshot.resultSessionId;
    }
    entered = Date.now();
    elapsed = 0;
    error = "";
    await tick();
    document
      .querySelector<HTMLElement>(".onboarding-content")
      ?.scrollTo({ top: 0 });
    document.querySelector<HTMLElement>("#onboarding-title")?.focus();
    if (stepId === "microphone") void startMicrophone();
  }
  async function advance(): Promise<void> {
    if (!canAdvance) return;
    busy = true;
    error = "";
    try {
      if (stepId === "terms") await acceptTerms();
      if (stepId === "models") {
        const saved = app.snapshot.settings.setupProfile ?? "balanced";
        if (
          mode === "first-run"
            ? confirmed !== selected
            : shouldApplyTourProfile(saved, selected)
        ) {
          // Do not let missing hardware details bypass component consent.
          if (!selection) {
            await profiles.check();
            if (!selection)
              throw new Error(
                "Model choices could not be read. Try again before downloading.",
              );
          }
          if (!(await ensureDownloadConsent(onboardingDownloadIds(selection))))
            return;
          app.apply(
            await call("confirm_onboarding_profile", { profile: selected }),
          );
          confirmed = selected;
          // Catalog details must never hold Continue after settings were saved.
          void library.refresh();
          void call("get_ai_settings")
            .then((value) => {
              aiSettings = value;
            })
            .catch(() => {
              aiDetailsFailed = true;
            });
        }
      }
      if (stepId === "ready") {
        onfinish();
      } else {
        // Capture is gated by setupProfile. Save the completed choices before
        // Ready so the real global shortcut can be tried inside the tour.
        if (stepId === "where" && mode === "first-run" && !setupFinished) {
          app.apply(await call("finish_onboarding"));
          setupFinished = true;
        }
        await move(step + 1);
      }
    } catch (failure: unknown) {
      error = errorMessage(failure);
    } finally {
      busy = false;
    }
  }
  async function installSpeakers(): Promise<void> {
    if (!speakerInstallable || !speakerModel || busy) return;
    const id = speakerModel.id;
    speakerBusy = true;
    speakerError = "";
    try {
      if (!(await ensureDownloadConsent([id]))) return;
      library.apply(await call("queue_onboarding_download", { id }));
      speakerRequested = true;
    } catch (failure: unknown) {
      speakerError = errorMessage(failure);
    } finally {
      speakerBusy = false;
    }
  }
  async function installMedia(): Promise<void> {
    if (!mediaInstallable || busy) return;
    mediaPhase = "consent";
    mediaError = "";
    try {
      if (!(await ensureDownloadConsent(["ffmpeg"]))) return;
      mediaPhase = "installing";
      mediaStatus = await call("install_media_support");
    } catch (failure: unknown) {
      mediaError = errorMessage(failure);
    } finally {
      mediaPhase = null;
    }
  }
  async function permission(
    kind: "microphone" | "accessibility",
  ): Promise<void> {
    try {
      await call("open_permission_settings", { kind });
    } catch (failure: unknown) {
      error = errorMessage(failure);
    }
  }
</script>

<section
  class="onboarding"
  data-step={stepId}
  aria-label={mode === "tour" ? "SonoJot tour" : "First-time setup"}
>
  <main id="page-content" class="onboarding-content" use:scrollEdges>
    <div class="onboarding-page">
      <div class="step-head">
        <h1 id="onboarding-title" tabindex="-1" data-text-surface="emboss">
          {onboardingSteps[step].title}
        </h1>
        <p class="step" data-text-surface="flat">
          Step {step + 1} of {onboardingSteps.length}{mode === "tour"
            ? " · Tour"
            : ""}
        </p>
      </div>
      <div
        class="step-progress"
        role="progressbar"
        aria-label="Setup step progress"
        aria-valuemin="0"
        aria-valuemax={onboardingSteps.length}
        aria-valuenow={step + 1}
        aria-valuetext={`Step ${step + 1} of ${onboardingSteps.length}`}
      >
        <span style:width={`${progress * 100}%`}></span>
      </div>
      {#if stepId === "welcome"}
        <div class="welcome-mark"><BrandMark /></div>
        <p class="lead" data-text-surface="flat">
          Dictate into the app you are writing in. Record meetings and turn them
          into transcripts and notes.
        </p>
        <p data-text-surface="flat">
          Speech recognition runs on this computer. This setup checks your
          microphone, sets your shortcut and prepares your models.
        </p>
        <p class="field-help" data-text-surface="flat">
          {mode === "tour"
            ? "Your current choices stay in place unless you change them."
            : "If you close setup, you can finish it next time. Downloads begin only after you confirm your choices and any required terms."}
        </p>
      {:else if stepId === "terms"}
        <TermsAcceptance bind:agreed={termsAgreed} disabled={busy} />
      {:else if stepId === "microphone"}
        <p data-text-surface="flat">
          Say a few words and watch the level. This check only measures input.
          It does not save or transcribe audio.
        </p>
        <Card class="tour-card microphone-card">
          <div class="microphone-grid">
            <div>
              <label class="field-label" for="setup-microphone"
                >Input device</label
              >
              <select
                id="setup-microphone"
                value={app.snapshot.settings.microphone ?? ""}
                disabled={micBusy || app.busy("settings")}
                onchange={(event) =>
                  changeMicrophone(event.currentTarget.value)}
              >
                <option value="">System default microphone</option>
                {#each app.snapshot.devices as device}<option value={device.id}
                    >{device.name}{device.isDefault ? " (default)" : ""}</option
                  >{/each}
              </select>
            </div>
            <div class="paper-well microphone-well">
              <Meter {level} active={micActive} />
              <p
                role="status"
                class:mic-error={!!micError}
                data-text-surface="flat"
              >
                {micError
                  ? micError
                  : micConfirmation === "detected"
                    ? "Input detected. You can continue."
                    : micConfirmation === "confirmed"
                      ? "Microphone confirmed. You can continue."
                      : micBusy
                        ? "Opening the microphone…"
                        : micActive
                          ? "Speak and watch the input level."
                          : "The microphone is not active."}
              </p>
            </div>
          </div>
          {#if mac}<p data-text-surface="flat">
              Allow microphone access when macOS asks. If access is denied,
              enable SonoJot in System Settings, Privacy &amp; Security,
              Microphone.
            </p>
          {:else if micError}<p data-text-surface="flat">
              Check your device and allow desktop apps to use the microphone in
              Windows privacy settings.
            </p>{/if}
          {#if !micActive || micError || mac || (elapsed >= 10000 && !micConfirmed)}<div
              class="actions"
            >
              {#if !micActive}<button
                  class="button"
                  disabled={micBusy}
                  onclick={startMicrophone}>Try microphone again</button
                >{/if}
              {#if micError || mac}<button
                  class="button quiet"
                  onclick={() => permission("microphone")}
                  >Open {mac
                    ? "System Settings"
                    : "microphone settings"}</button
                >{/if}
              {#if elapsed >= 10000 && !micConfirmed}<button
                  class="button"
                  onclick={() => {
                    micConfirmation = "confirmed";
                  }}>It's working</button
                >{/if}
            </div>{/if}
        </Card>
      {:else if stepId === "shortcut"}
        <Shortcuts embedded />
      {:else if stepId === "dictation"}
        {#if !mac && app.snapshot.capabilities.autoPaste}<p
            data-text-surface="flat"
          >
            When you finish speaking, SonoJot pastes the text where your cursor
            is.
          </p>{/if}
        <div class="tab-explanation">
          <BrandMark />
          <p data-text-surface="flat">
            The screen tab opens recording controls at the top of your display.
            You can also open SonoJot from the {mac
              ? "menu bar"
              : "system tray"}.
          </p>
        </div>
        {#if mac}<MacPasteAccess />{/if}
        <!-- Mac-only permission sections (for example System audio recording)
             go here, after the Accessibility block, as their own components. -->
      {:else if stepId === "models"}
        <div class="segments choices" role="group" aria-label="Setup profile">
          {#each setupProfiles.filter((profile) => profile.id !== "custom") as profile}<button
              aria-pressed={selected === profile.id}
              disabled={busy}
              onclick={() => {
                selected = profile.id;
              }}
              ><span>{profile.title}</span>{#if profile.id === "balanced"}<small
                  data-text-surface="flat">Recommended</small
                >{/if}</button
            >{/each}
          {#if mode === "tour" && app.snapshot.settings.setupProfile === "custom"}<button
              aria-pressed={selected === "custom"}
              disabled={busy}
              onclick={() => {
                selected = "custom";
              }}
              ><span>Custom</span><small data-text-surface="flat"
                >Keep my models</small
              ></button
            >{/if}
        </div>
        <div class="model-columns">
          <Card class="tour-card model-summary">
            <dl data-text-surface="flat">
              <dt>Speech</dt>
              <dd>{modelDetails.speech ?? missingDetails}</dd>
              <dt>Notes</dt>
              <dd>
                {modelDetails.notes ?? missingDetails}
                {#if selected === "custom" && (aiSettings?.backend === "openrouter" || selection?.notesModel === "openrouter")}<RemoteModelBadge
                  />{/if}
              </dd>
              <dt>Cleanup</dt>
              <dd>{cleanupLine}</dd>
            </dl>
            <p data-text-surface="flat">
              {modelDetails.downloadBytes === null
                ? detailsLoading
                  ? "Checking download size…"
                  : "Model details unavailable"
                : modelDetails.downloadBytes === 0
                  ? "No download needed"
                  : `${formatBytes(modelDetails.downloadBytes)} to download`}
            </p>
            {#if detailsLoading}<progress
                class="details-progress"
                aria-label="Loading model details"
              ></progress>{/if}
          </Card>
          <section class="profile-memory" aria-labelledby="profile-reason">
            <h2 id="profile-reason" data-text-surface="flat">
              Why we recommend {profileLabel(
                selected === "custom" ? "balanced" : selected,
              )}
            </h2>
            {#if profiles.hardware}<ProfileRecommendation
                profile={selected === "custom" ? "balanced" : selected}
              />{:else}<p data-text-surface="flat">{missingDetails}</p>{/if}
          </section>
        </div>
        <p data-text-surface="flat">
          Later, in Models, you can change these or add online OpenRouter
          models.
        </p>
      {:else if stepId === "meetings"}
        <div class="meetings-copy" data-text-surface="flat">
          <p>Your recordings, transcripts and notes live in Meetings.</p>
          {#if app.snapshot.capabilities.systemAudioStatus !== "unsupported"}<p>
              Meetings record <em class="key"
                >your microphone and this computer's audio</em
              > by default, so you get your voice and the other people on a Zoom,
              Teams or Meet call. If you switch to microphone only, the other people
              on a call won't be recorded; SonoJot will warn you before a meeting
              starts.
            </p>
          {:else}<p>
              {mac ? "On a Mac" : "On this computer"}, meetings record
              <em class="key">your microphone only</em>. SonoJot doesn't record
              the {mac ? "Mac's" : "computer's"} own audio, so other people on a Zoom,
              Teams or Meet call are recorded only as your microphone picks them up.
              You can also import a recording of a call.
            </p>{/if}
          <p>
            After recording, use <em class="key">Clean up the transcript</em>
            to fix its written form. Use
            <em class="key">Process notes</em> to choose a format and model. The original
            transcript stays available.
          </p>
        </div>
        <SystemAudioSetup bind:resolved={systemAudioResolved} />
      {:else if stepId === "extras"}
        <p data-text-surface="flat">
          Optional downloads for Meetings. Skip them if you like; you can add
          them later.
        </p>
        <Card class="tour-card extras-card">
          <section class="extra" aria-labelledby="extra-media">
            <div class="extra-copy" data-text-surface="flat">
              <h2 id="extra-media">Import recordings</h2>
              <p>
                FFmpeg converts audio and video files so you can import them. {mac
                  ? "SonoJot's own LGPL build, from releases.sonojot.com."
                  : "An LGPL build, from GitHub."}
              </p>
              <p class="extra-state" role="status">{mediaState}</p>
            </div>
            {#if !mediaAvailable}<button
                class="button"
                disabled={!mediaInstallable || busy}
                onclick={installMedia}>Review license and download</button
              >{/if}
            {#if mediaPhase === "installing"}<progress
                aria-label="Downloading FFmpeg"
              ></progress>{/if}
            {#if mediaStatus?.mediaImport.state === "unsupported" && mediaStatus.mediaImport.detail}<p
                class="extra-note"
                data-text-surface="flat"
              >
                {mediaStatus.mediaImport.detail}
              </p>{/if}
            {#if mediaError}<p class="extra-note flow-error" role="alert">
                {mediaError}
              </p>{/if}
          </section>
          {#if speakerOffered && speakerModel}
            <section class="extra" aria-labelledby="extra-speakers">
              <div class="extra-copy" data-text-surface="flat">
                <h2 id="extra-speakers">Speaker labels</h2>
                <p>
                  Shows who said what in meeting transcripts. {speakerModel.name},
                  {formatBytes(
                    speakerModel.totalBytes || speakerModel.diskBytes,
                  )}, runs on this computer.
                </p>
                <p class="extra-state" role="status">{speakerState}</p>
              </div>
              {#if !installed(speakerModel) && !installing(speakerModel)}<button
                  class="button"
                  disabled={!speakerInstallable || busy}
                  onclick={installSpeakers}>Download</button
                >{/if}
              {#if installing(speakerModel) && speakerModel.state !== "queued"}<progress
                  aria-label="Downloading speaker labels"
                  max={speakerModel.totalBytes || 1}
                  value={speakerModel.totalBytes > 0
                    ? Math.min(
                        speakerModel.downloadedBytes,
                        speakerModel.totalBytes,
                      )
                    : undefined}
                ></progress>{/if}
              {#if speakerError || speakerModel.error}<p
                  class="extra-note flow-error"
                  role="alert"
                >
                  {speakerError || speakerModel.error}
                </p>{/if}
            </section>
          {/if}
        </Card>
      {:else if stepId === "where"}
        <SidebarTour />
      {:else}
        <div class="ready-columns">
          <Card class="tour-card practice">
            <label class="field-label" for="tour-dictation"
              >Press {app.snapshot.settings.hotkey} and say something. Your words
              appear here.</label
            >
            <p class="field-help" data-text-surface="flat">
              {app.snapshot.settings.hotkeyMode === "hold"
                ? "Hold the shortcut while speaking, then release it."
                : "Press the shortcut again to finish."}
            </p>
            <div class="scroll-edges-frame">
              <textarea
                id="tour-dictation"
                bind:value={practiceText}
                placeholder="Your dictation"
                spellcheck="false"
                use:scrollEdges={{ frame: true }}></textarea>
            </div>
            {#if ["loading", "listening", "finishing", "cleaning"].includes(app.snapshot.state)}<p
                class="field-help"
                role="status"
                data-text-surface="flat"
              >
                {app.snapshot.state === "listening"
                  ? "Listening…"
                  : "Preparing your words…"}
              </p>
            {:else if !practiceReady}<p
                class="field-help"
                data-text-surface="flat"
              >
                {!speechReady && !punctuationReady
                  ? "Speech and punctuation are not ready yet."
                  : !speechReady
                    ? "Speech is not ready yet."
                    : "Punctuation is not ready yet."} Wait for the downloads, or
                try dictation later.
              </p>{/if}
          </Card>
          <section class="ready-downloads" aria-label="Downloads">
            {#each [...new Set(downloadIds)].filter((id) => id !== "openrouter") as id}
              {@const model = models.find((entry) => entry.id === id)}
              <ModelDownload
                {model}
                compact
                idleLabel="Not installed"
                pending={busy}
                retryDisabled={models.some(installing)}
                oncancel={async () => {
                  await app.perform("download", async () => {
                    library.apply(await call("cancel_model_install", { id }));
                  });
                }}
                onretry={async () => {
                  await app.perform("download", async () => {
                    if (!(await ensureDownloadConsent([id]))) return;
                    library.apply(await call("start_model_install", { id }));
                  });
                }}
              />
            {/each}
            {#if mediaBusy || mediaAvailable || mediaError}<p
                class="media-ready"
                data-text-surface="flat"
              >
                FFmpeg · {mediaError
                  ? "Needs attention"
                  : mediaAvailable
                    ? "Installed"
                    : "Installing…"}
              </p>{/if}
            <p class="field-help" data-text-surface="flat">
              You can follow and retry downloads in Models.
            </p>
          </section>
        </div>
      {/if}
      {#if error || app.error || library.error}<p
          class="flow-error"
          role="alert"
        >
          {error || app.error || library.error}
        </p>{/if}
    </div>
  </main>
  <footer class="onboarding-footer">
    <button
      class="button quiet"
      disabled={step === 0 || busy || micBusy || mediaBusy}
      onclick={() => move(step - 1)}>Back</button
    >
    <span class="dwell-note" data-text-surface="flat"
      >{stepId !== "ready" && dwell < 1
        ? `Continue in ${Math.ceil((onboardingSteps[step].dwell - elapsed) / 1000)} seconds`
        : stepId === "terms" && !termsAgreed
          ? "Read and agree to continue"
          : stepId === "microphone" && !micConfirmed
            ? "Check your microphone to continue"
            : stepId === "meetings" && !systemAudioResolved
            ? "Allow system audio, or choose Not now"
            : app.dirty
              ? "Save your shortcut to continue"
              : ""}</span
    >
    <button
      class="button primary continue"
      style:--dwell={`${stepId === "ready" ? 100 : dwell * 100}%`}
      disabled={!canAdvance}
      onclick={advance}
      ><span
        >{busy
          ? "Saving…"
          : stepId === "ready"
            ? "Go to SonoJot"
            : stepId === "terms"
              ? "Agree and continue"
              : stepId === "models"
                ? selected === "custom"
                  ? "Keep my models"
                  : `Use ${profileLabel(selected)}`
                : "Continue"}</span
      ></button
    >
  </footer>
</section>

<style>
  .onboarding {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  /* Every step is laid out to fit the window, so this area does not scroll.
     onboarding-fit.spec.js checks each step at the minimum and default window
     sizes. Overflow stays reachable only as a fallback for enlarged text. */
  .onboarding-content {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden auto;
    padding: 27px 36px 0;
    margin-bottom: 18px;
    overscroll-behavior: contain;
  }
  .onboarding-page {
    flex: 1 1 auto;
    min-height: 0;
    width: 100%;
    max-width: 810px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
  }
  /* Steps keep their natural height; only these parts give and take space. */
  .onboarding-page > * {
    flex: none;
  }
  .onboarding-page > :global(.terms-acceptance),
  .onboarding-page > :global(.sidebar-tour) {
    flex: 1 1 0;
    min-height: 0;
  }
  .onboarding-page > :global(.sidebar-tour) {
    max-height: 600px;
  }
  .onboarding-page p {
    margin: 0 0 18px;
    line-height: 1.5;
  }
  .step-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 18px;
    margin-bottom: 12px;
  }
  h1 {
    margin: 0;
    font-size: 2rem;
    line-height: 1.2;
  }
  h1:focus {
    outline: none;
  }
  .onboarding-page .step {
    flex: none;
    margin: 0;
    color: var(--soft-ink);
    font-size: 0.9rem;
    white-space: nowrap;
  }
  .step-progress {
    height: 6px;
    margin-bottom: 27px;
    border: 1px solid var(--line);
    border-radius: 5px;
    background: var(--well);
  }
  .step-progress span {
    display: block;
    height: 100%;
    background: var(--ink);
    border-radius: 5px;
  }
  .onboarding-page .lead {
    font-size: 1.35rem;
    max-width: 640px;
  }
  .welcome-mark {
    width: max-content;
    margin: 18px 0 36px;
    transform: scale(1.6);
    transform-origin: left center;
  }
  .onboarding-footer {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 18px 36px;
    border-top: 1px solid var(--line);
  }
  .dwell-note {
    flex: 1;
    color: var(--soft-ink);
    font-size: 0.9rem;
    text-align: right;
  }
  .continue {
    position: relative;
    overflow: hidden;
    min-width: 180px;
  }
  .continue::before {
    content: "";
    position: absolute;
    bottom: 0;
    left: 0;
    width: var(--dwell);
    height: 3px;
    background: currentColor;
    opacity: 0.4;
  }
  .continue span {
    position: relative;
  }
  :global(.onboarding .tour-card) {
    margin: 0 0 18px;
    padding: 18px 24px;
  }
  :global(.onboarding .tour-card > :last-child) {
    margin-bottom: 0;
  }

  /* Microphone */
  .microphone-grid {
    display: grid;
    grid-template-columns: minmax(0, 0.85fr) minmax(0, 1.15fr);
    align-items: end;
    gap: 18px 27px;
  }
  .onboarding-page .microphone-well {
    margin: 0;
    padding: 12px 18px;
  }
  .onboarding-page .microphone-well p {
    margin: 6px 0 0;
    font-size: 15px;
    color: var(--soft-ink);
  }
  .onboarding-page .microphone-well .mic-error {
    color: var(--danger);
  }
  :global(.onboarding .microphone-card > p) {
    margin: 12px 0 0;
  }
  :global(.onboarding .microphone-card > .actions) {
    margin-top: 12px;
  }

  /* Dictation */
  .tab-explanation {
    display: flex;
    align-items: center;
    gap: 18px;
    margin-bottom: 18px;
  }
  .tab-explanation p {
    margin: 0;
  }

  /* Models */
  .choices {
    margin-bottom: 18px;
  }
  .choices button {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 12px 8px;
    font-size: 19px;
  }
  .choices small {
    font-size: 0.8rem;
    margin-top: 4px;
  }
  .model-columns {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    align-items: start;
    gap: 18px 27px;
    margin-bottom: 18px;
  }
  :global(.onboarding .model-summary) {
    margin: 0;
  }

  dl {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 4px 18px;
    margin: 0;
    line-height: 1.4;
  }
  dt {
    color: var(--soft-ink);
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  :global(.onboarding .model-summary) p {
    margin: 9px 0 0;
  }
  .details-progress {
    margin-top: 9px;
  }
  .profile-memory {
    color: var(--soft-ink);
  }
  .profile-memory h2 {
    margin: 0;
    font-size: 1rem;
    line-height: 1.4;
    color: var(--ink);
  }
  .profile-memory :global(.recommendation p) {
    margin: 4px 0;
    line-height: 1.4;
  }

  /* Meetings: one size of body text. Key phrases get a soft, thick dye
     underline that sits below the ink and never changes on hover, so it does
     not read as a link (links use the thin ink underline). It is an <em>:
     the paper type renderer skips paragraphs holding strong, b or a, which
     made those paragraphs print differently from their neighbours. */
  .meetings-copy p:last-child {
    margin-bottom: 0;
  }
  .key {
    font-style: normal;
    text-decoration-line: underline;
    text-decoration-color: color-mix(in srgb, var(--dye) 38%, transparent);
    text-decoration-thickness: 0.3em;
    text-underline-offset: 0.02em;
    text-decoration-skip-ink: none;
  }

  /* Meeting extras: one row per optional download, action on the right. */
  .extra {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 9px 27px;
  }
  .extra + .extra {
    margin-top: 14px;
    padding-top: 14px;
    border-top: 1px solid var(--line);
  }
  .extra-copy {
    flex: 1 1 320px;
    min-width: 0;
  }
  .extra-copy h2 {
    margin: 0;
    font-size: 1rem;
    line-height: 1.4;
  }
  .onboarding-page .extra-copy p {
    margin: 0;
    line-height: 1.4;
  }
  .onboarding-page .extra-copy .extra-state {
    margin-top: 4px;
    color: var(--soft-ink);
  }
  .extra progress,
  .onboarding-page .extra-note {
    flex: 1 0 100%;
    margin: 0;
  }

  /* Ready */
  .ready-columns {
    flex: 0 1 auto;
    min-height: 0;
    display: flex;
    align-items: stretch;
    gap: 27px;
  }
  .ready-columns > :global(*) {
    flex: 1 1 0;
    min-width: 0;
    min-height: 0;
  }
  :global(.onboarding .practice) {
    display: flex;
    flex-direction: column;
    margin: 0;
  }
  :global(.onboarding .practice) > * {
    flex: none;
  }

  :global(.onboarding .practice) .field-help {
    margin: 6px 0 0;
  }
  textarea {
    flex: 1 1 auto;
    width: 100%;
    height: 7.5em;
    min-height: 3.5em;
    margin: 9px 0 0;
    resize: none;
  }
  .ready-columns > .ready-downloads {
    flex-grow: 1.25;
  }
  .ready-downloads {
    display: flex;
    flex-direction: column;
  }
  .ready-downloads > :global(.download:first-child) {
    padding-top: 0;
  }
  .onboarding-page .ready-downloads p {
    margin: 9px 0 0;
  }
  .media-ready {
    color: var(--soft-ink);
  }
  progress {
    display: block;
    width: 100%;
    height: 6px;
    accent-color: var(--ink);
  }
  .flow-error {
    color: var(--danger);
  }

  @media (max-width: 800px) {
    .onboarding-content {
      padding: 18px 27px 0;
    }
    .onboarding-footer {
      padding: 18px 27px;
    }
  }
  /* Short windows, down to the 540px minimum: tighter rhythm and a slimmer
     header, the same content. The recommendation keeps its plain reason. */
  @media (max-height: 700px) {
    :global(.sheet:has(> .onboarding) > .sheet-head) {
      min-height: 0;
      padding-top: 10px;
      padding-bottom: 10px;
    }
    .onboarding-content {
      padding-top: 12px;
      margin-bottom: 0;
    }
    .onboarding-footer {
      padding-top: 10px;
      padding-bottom: 10px;
    }
    h1 {
      font-size: 1.5rem;
    }
    .step-head {
      margin-bottom: 6px;
    }
    .step-progress {
      margin-bottom: 14px;
    }
    .profile-memory :global(.recommendation > .hardware-row),
    .profile-memory :global(.recommendation > p:not(:last-child)) {
      display: none;
    }
    /* Summary above a one-line reason, so model names do not wrap. */
    .model-columns {
      grid-template-columns: minmax(0, 1fr);
      gap: 9px;
    }
    .profile-memory :global(.recommendation > p:last-child) {
      margin: 0;
    }
    /* The reason sentence names the profile; its heading stays for readers. */
    .profile-memory h2 {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip: rect(0, 0, 0, 0);
      white-space: nowrap;
    }
    .extra-copy h2,
    .onboarding-page .extra-copy h2 + p {
      display: inline;
    }
    .extra-copy h2::after {
      content: ". ";
    }
    .onboarding-page p {
      margin-bottom: 12px;
    }
    .onboarding-page .lead {
      font-size: 1.15rem;
    }
    .welcome-mark {
      margin: 0 0 18px;
      transform: scale(1.2);
    }
    :global(.onboarding .tour-card) {
      margin-bottom: 12px;
      padding: 14px 20px;
    }
    .choices,
    .model-columns {
      margin-bottom: 12px;
    }
    .choices button {
      flex-direction: row;
      justify-content: center;
      align-items: baseline;
      gap: 9px;
      padding: 9px 8px;
      font-size: 17px;
    }
    .choices small {
      margin-top: 0;
    }
    .tab-explanation {
      margin-bottom: 12px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .continue::before {
      transition: none;
    }
  }
</style>
