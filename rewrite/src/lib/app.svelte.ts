import { call, native, subscribe, subscribeDictationHistory } from "./api";
import { meetingDataChanged } from "./meeting-update";
import { restartBlockedReason } from "./updates";
import {
  errorMessage,
  previewSnapshot,
  type Settings,
  type Snapshot,
} from "./protocol";

/** How long a status notice stays before it clears itself. */
export const STATUS_NOTICE_MS = 5_000;

export class AppState {
  snapshot = $state<Snapshot>(structuredClone(previewSnapshot));
  ready = $state(!native);
  pending = $state<string[]>([]);
  error = $state("");
  notice = $state("");
  meetingRevision = $state(0);
  dictationRevision = $state(0);
  dirty = $state(false);
  confirmation = $state<{
    title: string;
    detail: string;
    label: string;
    danger: boolean;
    alternativeLabel?: string;
    cancelLabel?: string;
  } | null>(null);
  private resolveConfirmation:
    ((confirmed: boolean | "alternative") => void) | undefined;
  readonly native = native;
  private lastMeetingUpdate = 0;
  private meetingUpdateTimer: ReturnType<typeof setTimeout> | undefined;
  private unlisten: (() => void) | undefined;
  private unlistenHistory: (() => void) | undefined;
  private dictationRequest = 0;

  busy(key: string): boolean {
    return this.pending.includes(key);
  }
  get updateRestartBlockedReason(): string | null {
    return restartBlockedReason(
      this.snapshot.update.restartBlockedReason,
      this.dirty,
      this.pending,
    );
  }
  async checkForUpdates(): Promise<void> {
    await this.perform("update-check", async () => {
      this.apply(await call("check_for_updates"));
    });
  }
  async restartToUpdate(): Promise<void> {
    if (this.updateRestartBlockedReason || this.snapshot.update.installing) return;
    await this.perform("update-install", async () => {
      this.apply(await call("restart_to_update"));
    });
  }
  async dismissUpdateNotice(): Promise<void> {
    await this.perform("update-notice", async () => {
      this.apply(await call("dismiss_update_notice"));
    });
  }
  async downloadUpdate(): Promise<void> {
    if (this.snapshot.update.status.state !== "available") return;
    await this.perform("update-download", async () => {
      this.apply(await call("download_update"));
    });
  }
  confirm(
    title: string,
    detail: string,
    label = "Discard changes",
    danger = false,
  ): Promise<boolean> {
    return this.chooseConfirmation({ title, detail, label, danger }).then(
      (choice) => choice === true,
    );
  }
  chooseConfirmation(options: {
    title: string;
    detail: string;
    label: string;
    danger?: boolean;
    alternativeLabel?: string;
    cancelLabel?: string;
  }): Promise<boolean | "alternative"> {
    return new Promise((resolve) => {
      this.resolveConfirmation?.(false);
      this.resolveConfirmation = resolve;
      this.confirmation = { ...options, danger: options.danger ?? false };
    });
  }
  finishConfirmation(confirmed: boolean | "alternative"): void {
    this.confirmation = null;
    this.resolveConfirmation?.(confirmed);
    this.resolveConfirmation = undefined;
  }
  async perform<T>(
    key: string,
    action: () => Promise<T>,
    success?: string,
  ): Promise<T | undefined> {
    if (this.busy(key)) return undefined;
    this.pending = [...this.pending, key];
    this.error = "";
    this.notice = "";
    try {
      const value = await action();
      if (success) this.notice = success;
      return value;
    } catch (error: unknown) {
      this.error = errorMessage(error);
      return undefined;
    } finally {
      this.pending = this.pending.filter((item) => item !== key);
    }
  }
  apply = (value: Snapshot): void => {
    if (value.revision < this.snapshot.revision) return;
    // Refresh saved history only when a result is committed, never on raw interims.
    const completed =
      value.resultSessionId !== null &&
      !["loading", "listening", "finishing", "cleaning"].includes(
        value.state,
      ) &&
      (value.resultSessionId !== this.snapshot.resultSessionId ||
        value.state !== this.snapshot.state ||
        value.outputText !== this.snapshot.outputText);
    if (completed) this.dictationRevision += 1;
    const changed =
      value.activeMeetingId !== this.snapshot.activeMeetingId ||
      value.meetingRecording !== this.snapshot.meetingRecording;
    const meetingChanged = meetingDataChanged(this.snapshot, value);
    this.snapshot = value;
    // Meter and dictation pushes do not invalidate saved meeting data.
    if (!meetingChanged) return;
    const now = Date.now();
    if (changed || now - this.lastMeetingUpdate >= 1500) {
      clearTimeout(this.meetingUpdateTimer);
      this.meetingUpdateTimer = undefined;
      this.meetingRevision += 1;
      this.lastMeetingUpdate = now;
    } else if (this.meetingUpdateTimer === undefined) {
      // Deliver the last update in a burst even when no further event arrives.
      this.meetingUpdateTimer = setTimeout(
        () => {
          this.meetingUpdateTimer = undefined;
          this.meetingRevision += 1;
          this.lastMeetingUpdate = Date.now();
        },
        1500 - (now - this.lastMeetingUpdate),
      );
    }
  };
  async initialize(): Promise<() => void> {
    if (!native) return () => {};
    this.unlisten?.();
    this.unlisten = undefined;
    this.unlistenHistory?.();
    this.unlistenHistory = undefined;
    await this.perform("initialize", async () => {
      this.unlisten = await subscribe(this.apply, (error) => {
        this.error = errorMessage(error);
      });
      this.unlistenHistory = await subscribeDictationHistory(() => {
        this.dictationRevision += 1;
      });
      this.apply(await call("get_snapshot"));
      this.ready = true;
    });
    return () => {
      this.unlisten?.();
      this.unlisten = undefined;
      this.unlistenHistory?.();
      this.unlistenHistory = undefined;
      clearTimeout(this.meetingUpdateTimer);
      this.meetingUpdateTimer = undefined;
    };
  }
  async saveSettings(patch: Partial<Settings>): Promise<void> {
    if (!native) {
      this.snapshot.settings = { ...this.snapshot.settings, ...patch };
      this.notice =
        "Preview changed. App settings are saved only in the desktop app.";
      return;
    }
    await this.perform("settings", async () =>
      this.apply(
        await call("save_settings", {
          settings: { ...this.snapshot.settings, ...patch },
        }),
      ),
    );
  }
  async dictation(
    action: "start" | "stop" | "cancel" | "stop-cleanup",
    microphoneTest = false,
  ): Promise<void> {
    const key = `dictation-${action}`;
    if (
      this.busy(key) ||
      (action === "start" &&
        this.pending.some((item) => item.startsWith("dictation-")))
    )
      return;
    const request = ++this.dictationRequest;
    this.pending = [...this.pending, key];
    this.error = "";
    this.notice = "";
    try {
      const next =
        action === "start"
          ? microphoneTest ? await call("test_microphone") : await call("start_dictation")
          : action === "stop"
            ? await call("stop_dictation")
            : action === "stop-cleanup"
              ? await call("stop_cleanup")
              : await call("cancel_dictation");
      if (request === this.dictationRequest) this.apply(next);
    } catch (error: unknown) {
      if (request === this.dictationRequest) this.error = errorMessage(error);
    } finally {
      this.pending = this.pending.filter((item) => item !== key);
    }
  }
  async copy(original: boolean): Promise<void> {
    const incomplete = this.snapshot.recognitionCompleteness === "incomplete";
    await this.perform(
      "copy",
      () =>
        call("copy_text", {
          original,
          resultSessionId: this.snapshot.resultSessionId,
        }),
      incomplete
        ? "Reviewed text copied."
        : original
          ? "Original text copied."
          : "Text copied.",
    );
  }
}
export const app = new AppState();
