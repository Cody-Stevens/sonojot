import type { Snapshot } from "./protocol";

export type DictationAction = "start" | "stop" | "cancel" | "stop-cleanup";

/** A settled test result can be reviewed even when no text was recovered. */
export function completedDictationSession(
  snapshot: Pick<
    Snapshot,
    | "state"
    | "sessionId"
    | "resultSessionId"
    | "meetingRecording"
    | "originalText"
    | "outputText"
  >,
  previousResult: number | null,
): number | null {
  return snapshot.sessionId === null &&
    !snapshot.meetingRecording &&
    (snapshot.state === "idle" || snapshot.state === "error") &&
    snapshot.resultSessionId !== previousResult
    ? snapshot.resultSessionId
    : null;
}

export function dictationReady(
  snapshot: Pick<Snapshot, "models" | "devices" | "settings">,
): boolean {
  const { settings, devices, models } = snapshot;
  const microphone =
    settings.microphone === null
      ? devices.some((device) => device.isDefault)
      : devices.some((device) => device.id === settings.microphone);
  const speech = models.some(
    (model) =>
      model.kind === "asr" &&
      model.ready &&
      (model.id === settings.asrModel ||
        (settings.asrModel === "moonshine-medium" &&
          model.id === "moonshine-small")),
  );
  return microphone && speech;
}
export interface DictationControlInput {
  snapshot: Pick<Snapshot, "state" | "meetingRecording">;
  native: boolean;
  ready: boolean;
  pending: readonly string[];
}

/** Capture state decides the action. Pending IPC only prevents duplicate commands. */
export function selectDictationControl(input: DictationControlInput) {
  const { snapshot, native, ready, pending } = input;
  const active = snapshot.state === "loading" || snapshot.state === "listening";
  const finishing = snapshot.state === "finishing";
  const cleaning = snapshot.state === "cleaning";
  const stopping = pending.includes("dictation-stop");
  const cancelling = pending.includes("dictation-cancel");
  return {
    active,
    finishing,
    cleaning,
    action: cleaning
      ? ("stop-cleanup" as const)
      : active
        ? ("stop" as const)
        : ("start" as const),
    label: cleaning
      ? "Stop cleanup"
      : finishing
        ? "Finishing…"
        : active
          ? "Finish dictation"
          : "Start dictation",
    disabled:
      !native ||
      finishing ||
      (cleaning
        ? pending.includes("dictation-stop-cleanup")
        : active
          ? stopping || cancelling
          : !ready ||
            snapshot.meetingRecording ||
            pending.some((key) => key.startsWith("dictation-"))),
    showCancel: active || finishing,
    cancelDisabled: !native || cancelling,
  };
}

export function canRequestDictation(
  action: DictationAction,
  input: DictationControlInput,
): boolean {
  const control = selectDictationControl(input);
  return action === "cancel"
    ? control.showCancel && !control.cancelDisabled
    : action === control.action && !control.disabled;
}
