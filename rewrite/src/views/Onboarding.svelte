<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import BrandMark from "../brand/BrandMark.svelte";
  import Card from "../components/Card.svelte";
  import Meter from "../components/Meter.svelte";
  import ModelDownload from "../components/ModelDownload.svelte";
  import ProfileRecommendation from "../components/ProfileRecommendation.svelte";
  import RemoteModelBadge from "../components/RemoteModelBadge.svelte";
  import TermsAcceptance from "../components/TermsAcceptance.svelte";
  import ThirdPartyNotices from "../components/ThirdPartyNotices.svelte";
  import Shortcuts from "./Shortcuts.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import { errorMessage } from "../lib/protocol";
  import type { AiSettings } from "../lib/ai";
  import { library } from "../lib/model-library.svelte";
  import { profiles } from "../lib/setup-profiles.svelte";
  import { formatBytes, installed, installing } from "../lib/model-library";
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
  import { PUNCTUATION_MODEL } from "../lib/punctuation";
  import { paperScroll } from "../lib/paper-scroll";
  import { needsTermsAcceptance } from "../lib/terms";
  import { acceptTerms } from "../lib/terms.svelte";
  import { ensureDownloadConsent } from "../lib/download-consent.svelte";
  import { downloadConsentGroups } from "../lib/download-consent";
  import type { SetupStatus } from "../lib/setup";
  import { setupLabel } from "../lib/setup";

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
  let pastePermission = $state<string | null>(null);
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
  let mediaBusy = $state(false);
  let mediaSkipped = $state(false);
  let mediaError = $state("");
  const mediaAvailable = $derived(
    mediaStatus?.mediaImport.state === "installed" ||
      mediaStatus?.mediaImport.state === "ready",
  );
  const ffmpegTerms = $derived(
    downloadConsentGroups.find(
      (group) =>
        group.models.includes("ffmpeg") &&
        group.platform === app.snapshot.capabilities.platform,
    ),
  );
  const selection = $derived(
    selected === "custom"
      ? { ...app.snapshot.settings, reasons: [] }
      : profiles.hardware?.recommendations[selected],
  );
  const models = $derived(library.data?.models ?? []);
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
  const canAdvance = $derived(
    canContinue(step, elapsed, micConfirmed, termsAgreed) &&
      !busy &&
      !mediaBusy &&
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
    if (stepId !== "ready" || !app.native) return;
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
    if (stepId === "dictation" && mac)
      pastePermission = await call("onboarding_paste_permission");
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
  async function installMedia(): Promise<void> {
    if (!app.native || mediaBusy || mediaAvailable || busy) return;
    mediaBusy = true;
    mediaError = "";
    try {
      if (!(await ensureDownloadConsent(["ffmpeg"]))) return;
      mediaStatus = await call("install_media_support");
      mediaSkipped = false;
    } catch (failure: unknown) {
      mediaError = errorMessage(failure);
    } finally {
      mediaBusy = false;
    }
  }
  async function openMediaLicense(
    event: MouseEvent,
    group: string,
    index: number,
  ): Promise<void> {
    if (!app.native) return;
    event.preventDefault();
    try {
      await call("open_download_license", { group, index });
    } catch (failure: unknown) {
      mediaError = errorMessage(failure);
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
  aria-label={mode === "tour" ? "SonoJot tour" : "First-time setup"}
>
  <main id="page-content" class="onboarding-content" use:paperScroll>
    <div class="onboarding-page">
      <p class="step" data-text-surface="flat">
        Step {step + 1} of {onboardingSteps.length}{mode === "tour"
          ? " · Tour"
          : ""}
      </p>
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
      <h1 id="onboarding-title" tabindex="-1" data-text-surface="emboss">
        {onboardingSteps[step].title}
      </h1>
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
        <Card title="Say a few words">
          <label class="field-label" for="setup-microphone">Input device</label>
          <select
            id="setup-microphone"
            value={app.snapshot.settings.microphone ?? ""}
            disabled={micBusy || app.busy("settings")}
            onchange={(event) => changeMicrophone(event.currentTarget.value)}
          >
            <option value="">System default microphone</option>
            {#each app.snapshot.devices as device}<option value={device.id}
                >{device.name}{device.isDefault ? " (default)" : ""}</option
              >{/each}
          </select>
          <div class="paper-well microphone-well spaced">
            <Meter {level} active={micActive} />
            <p class="field-help" role="status" data-text-surface="flat">
              {micConfirmation === "detected"
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
          <p class="field-help" data-text-surface="flat">
            This check only measures input. It does not save or transcribe
            audio.
          </p>
          {#if mac}<p data-text-surface="flat">
              Allow microphone access when macOS asks. If access is denied,
              enable SonoJot in System Settings, Privacy &amp; Security,
              Microphone.
            </p>{/if}
          {#if micError}<p role="alert" data-text-surface="flat">{micError}</p>
            <p data-text-surface="flat">
              {mac
                ? "Check microphone access in System Settings, then try again."
                : "Check your device and allow desktop apps to use the microphone in Windows privacy settings."}
            </p>{/if}
          <div class="actions spaced">
            {#if !micActive}<button
                class="button"
                disabled={micBusy}
                onclick={startMicrophone}>Try microphone again</button
              >{/if}
            {#if micError || mac}<button
                class="button quiet"
                onclick={() => permission("microphone")}
                >Open {mac ? "System Settings" : "microphone settings"}</button
              >{/if}
            {#if elapsed >= 10000 && !micConfirmed}<button
                class="button"
                onclick={() => {
                  micConfirmation = "confirmed";
                }}>It's working</button
              >{/if}
          </div>
        </Card>
      {:else if stepId === "dictation"}
        <Shortcuts embedded />
        <div class="tab-explanation">
          <BrandMark />
          <p data-text-surface="flat">
            The screen tab opens recording controls at the top of your display.
            You can also open SonoJot from the {mac
              ? "menu bar"
              : "system tray"}.
          </p>
        </div>
        {#if mac}<p data-text-surface="flat">
            SonoJot currently copies text to the pasteboard on Mac. Press
            Command+V to paste. Automatic paste is not implemented yet; it will
            need Accessibility access.
          </p>
          {#if pastePermission}<p class="field-help" data-text-surface="flat">
              {pastePermission}
            </p>{/if}<button
            class="button quiet"
            onclick={() => permission("accessibility")}
            >Open Accessibility settings</button
          >{/if}
      {:else if stepId === "models"}
        <p data-text-surface="flat">
          Choose the models for speech and notes. You can choose individual
          models with Custom on the Models page later.
        </p>
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
        </div>
        {#if mode === "tour" && app.snapshot.settings.setupProfile === "custom"}<button
            class="button quiet spaced"
            aria-pressed={selected === "custom"}
            disabled={busy}
            onclick={() => {
              selected = "custom";
            }}>Keep my Custom models</button
          >{/if}
        <Card class="tour-card model-summary">
          <div data-text-surface="flat">
            <p>
              Speech: {modelDetails.speech ?? missingDetails}
            </p>
            <p>
              Notes: {modelDetails.notes ?? missingDetails}
              {#if selected === "custom" && (aiSettings?.backend === "openrouter" || selection?.notesModel === "openrouter")}<RemoteModelBadge
                />{/if}
            </p>
            <p>Punctuation: SonoJot Punctuation v2</p>
            <p>
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
          </div>
        </Card>
        <details class="profile-memory" open>
          <summary
            >Why we recommend {profileLabel(
              selected === "custom" ? "balanced" : selected,
            )}</summary
          >{#if profiles.hardware}<ProfileRecommendation
              profile={selected === "custom" ? "balanced" : selected}
            />{:else}<p data-text-surface="flat">{missingDetails}</p>{/if}
        </details>
      {:else if stepId === "meetings"}
        <p class="lead" data-text-surface="flat">
          Your recordings and transcripts live in Meetings.
        </p>
        <Card class="tour-card">
          <p data-text-surface="flat">
            Meetings record <strong
              >your microphone and this computer's audio</strong
            > by default, so you get your voice and the other people on a Zoom, Teams
            or Meet call.
          </p>
          <p data-text-surface="flat">
            You can change this in Meetings. If you switch to microphone only,
            the other people on a call won't be recorded; SonoJot will warn you
            before a meeting starts.
          </p>
          {#if !app.snapshot.capabilities.systemAudio}<p
              class="field-help"
              data-text-surface="flat"
            >
              System audio is unavailable on this platform. You can record your
              microphone or import a recording.
            </p>{/if}
          <p data-text-surface="flat">
            After recording, use <strong>Clean up the transcript</strong> to fix
            its written form. Use <strong>Process notes</strong> to choose a format
            and model. The original transcript stays available.
          </p>
        </Card>
      {:else if stepId === "where"}
        <Card class="tour-card"
          ><div class="sidebar-map" aria-label="Sidebar map">
            <div class="map-group">
              <strong data-text-surface="emboss">Dictation</strong><span
                data-text-surface="flat"
                >Dictation settings and recent text</span
              ><strong data-text-surface="emboss">Meetings</strong><span
                data-text-surface="flat">Recordings, transcripts and notes</span
              >
            </div>
            <div class="map-group settings-map">
              <strong data-text-surface="emboss">Microphone</strong><span
                data-text-surface="flat">Input device and level check</span
              ><strong data-text-surface="emboss">Shortcuts</strong><span
                data-text-surface="flat">Keys and how they work</span
              ><strong data-text-surface="emboss">Vocabulary</strong><span
                data-text-surface="flat">Names and words you use</span
              ><strong data-text-surface="emboss">Appearance</strong><span
                data-text-surface="flat">Paper, theme and screen tab</span
              ><strong data-text-surface="emboss">Models</strong><span
                data-text-surface="flat"
                >Profiles, downloads and online models</span
              ><strong data-text-surface="emboss">About</strong><span
                data-text-surface="flat">Version, help and this tour</span
              >
            </div>
            <div class="map-group update-map">
              <strong data-text-surface="emboss">Update available</strong><span
                data-text-surface="flat"
                >At the bottom. Click to download, then restart to update.</span
              >
            </div>
          </div></Card
        >
      {:else}
        <Card title="Try a dictation" class="tour-card">
          <label class="field-label" for="tour-dictation"
            >Press {app.snapshot.settings.hotkey} and say something. Your words appear
            here.</label
          >
          <p class="field-help" data-text-surface="flat">
            {app.snapshot.settings.hotkeyMode === "hold"
              ? "Hold the shortcut while speaking, then release it."
              : "Press the shortcut again to finish."}
          </p>
          <textarea
            id="tour-dictation"
            rows="3"
            bind:value={practiceText}
            placeholder="Your dictation"
            spellcheck="false"></textarea>
          {#if ["loading", "listening", "finishing", "cleaning"].includes(app.snapshot.state)}<p
              role="status"
              data-text-surface="flat"
            >
              {app.snapshot.state === "listening"
                ? "Listening…"
                : "Preparing your words…"}
            </p>{/if}
          {#if !practiceReady}<p class="field-help" data-text-surface="flat">
              {!speechReady && !punctuationReady
                ? "Speech and punctuation are not ready yet."
                : !speechReady
                  ? "Speech is not ready yet."
                  : "Punctuation is not ready yet."} Wait for the downloads below,
              or skip this step and try dictation later.
            </p>
            <button
              class="button quiet"
              disabled={busy || !canAdvance}
              onclick={advance}>Skip this step</button
            >
          {/if}
        </Card>
        <div class="downloads">
          {#each [...new Set(downloadIds)].filter((id) => id !== "openrouter") as id}
            {@const model = models.find((entry) => entry.id === id)}
            <ModelDownload
              {model}
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
        </div>
        <p class="field-help" data-text-surface="flat">
          Find download progress and retry controls on the Models page.
        </p>
        <Card title="Audio and video import" class="tour-card">
          {#if mediaAvailable}
            <p data-text-surface="flat">
              FFmpeg is available for importing recordings.
            </p>
          {:else}
            <p data-text-surface="flat">
              Audio and video import uses FFmpeg (LGPL). Download it now?
            </p>
            <p class="field-help" data-text-surface="flat">
              Review the GNU Lesser General Public License and source
              information before downloading. You can skip this and install it
              later, or choose your own FFmpeg, in Models → Performance and
              storage.
            </p>
            <div class="actions spaced">
              <button
                class="button"
                disabled={!app.native ||
                  busy ||
                  mediaBusy ||
                  !mediaStatus ||
                  mediaStatus.mediaImport.state === "unsupported" ||
                  mediaStatus.mediaImport.state === "loading"}
                onclick={installMedia}
              >
                {mediaBusy
                  ? "Preparing FFmpeg…"
                  : "Review license and download"}
              </button>
              <button
                class="button quiet"
                disabled={mediaBusy || mediaSkipped}
                onclick={() => {
                  mediaSkipped = true;
                }}>Skip FFmpeg</button
              >
            </div>
            <p class="field-help" role="status" data-text-surface="flat">
              {mediaSkipped
                ? "Skipped. You can install FFmpeg later."
                : setupLabel("mediaImport", mediaStatus?.mediaImport)}
            </p>
            {#if mediaStatus?.mediaImport.state === "unsupported"}<p
                class="field-help"
                data-text-surface="flat"
              >
                {mediaStatus.mediaImport.detail}
              </p>{/if}
          {/if}
          {#if ffmpegTerms}
            {@const group = ffmpegTerms}
            <ul class="media-links" data-text-surface="flat">
              {#each group.links as link, index}
                <li>
                  <a
                    href={link.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    onclick={(event) =>
                      openMediaLicense(event, group.id, index)}>{link.label}</a
                  >
                </li>
              {/each}
            </ul>
          {/if}
          <ThirdPartyNotices />
          {#if mediaError}<p class="flow-error" role="alert">
              {mediaError}
            </p>{/if}
        </Card>
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
  .onboarding-content {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 27px 36px 0;
    margin-bottom: 18px;
    scrollbar-gutter: stable;
    overscroll-behavior: contain;
  }
  .onboarding-page {
    max-width: 810px;
    margin: 0 auto;
    padding-bottom: 36px;
  }
  .step {
    color: var(--soft-ink);
    margin: 0 0 9px;
  }
  h1 {
    margin: 0 0 27px;
  }
  h1:focus {
    outline: none;
  }
  p {
    line-height: 1.5;
  }
  .lead {
    font-size: 1.35rem;
    max-width: 640px;
  }
  .welcome-mark {
    width: max-content;
    margin: 36px 0;
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
  .choices button {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
  }
  .choices small {
    font-size: 0.8rem;
  }
  :global(.onboarding .model-summary) {
    margin: 27px 0;
  }
  :global(.onboarding .model-summary p) {
    margin: 9px 0;
  }
  .profile-memory {
    color: var(--soft-ink);
  }
  .profile-memory summary {
    cursor: pointer;
  }
  .tab-explanation {
    display: flex;
    align-items: center;
    gap: 18px;
    margin-top: 27px;
  }
  :global(.onboarding .card-heading) {
    margin-bottom: 18px;
  }
  :global(.onboarding .card-heading h2) {
    font-size: 1.35rem;
    line-height: 1.25;
    margin: 0;
  }
  :global(.onboarding .tour-card > p:first-child),
  :global(.onboarding .model-summary p:first-child) {
    margin-top: 0;
  }
  textarea {
    width: 100%;
    min-height: 108px;
    margin: 9px 0;
    resize: vertical;
  }
  progress {
    display: block;
    width: 100%;
    height: 6px;
    accent-color: var(--ink);
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
  .map-group {
    display: grid;
    grid-template-columns: 190px 1fr;
    gap: 9px 27px;
  }
  .settings-map {
    margin-top: 36px;
  }
  .update-map {
    margin-top: 18px;
    border-top: 1px solid var(--line);
    padding-top: 18px;
  }
  .downloads {
    display: grid;
    gap: 9px;
  }
  .downloads + .field-help {
    margin-top: 18px;
  }
  .flow-error {
    color: var(--danger);
  }
  .media-links {
    padding-left: 22px;
    line-height: 1.6;
  }
  .media-links a {
    color: inherit;
    text-underline-offset: 4px;
  }
  @media (max-width: 800px) {
    .onboarding-content {
      padding: 18px 27px 0;
    }
    .onboarding-footer {
      padding: 18px 27px;
    }
    .map-group {
      grid-template-columns: 145px 1fr;
      gap: 9px 18px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .continue::before {
      transition: none;
    }
  }
</style>
