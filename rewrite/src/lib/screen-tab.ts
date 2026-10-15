import type { CommandArguments, Settings, Snapshot } from "./protocol";
import type { ErrorAction } from "./errors";

/** Keep repair destinations aligned with the error's settings page. */
export function tabRepairPage(
  action: ErrorAction,
): CommandArguments["show_page"]["page"] {
  switch (action) {
    case "microphone":
    case "shortcuts":
    case "meetings":
    case "appearance":
      return action;
    default:
      return "setup";
  }
}

export const COLLAPSE_DELAY_MS = 380;

// One source of truth for the one-sided meeting warning (Meetings page and screen tab).
export { meetingSourceWarning } from "./meeting-source";

/** A failed save must never fall through to a one-sided recording. */
export async function startTabMeeting(
  recordBoth: boolean,
  saveBoth: () => Promise<void>,
  start: () => Promise<void>,
): Promise<void> {
  if (recordBoth) await saveBoth();
  await start();
}
export interface ScreenTabIntent {
  visible: boolean;
  expanded: boolean;
  focus: boolean;
  /** Keep Visible: pins an edge reveal without taking focus. */
  pinned?: boolean;
}

/** Geometry commands are ordered independently from capture commands. */
export class ScreenTabRequests {
  private wanted: ScreenTabIntent | undefined;
  private running = false;
  private disposed = false;
  constructor(
    private readonly send: (intent: ScreenTabIntent) => Promise<void>,
    private readonly onError: (error: unknown) => void,
  ) {}
  request(intent: ScreenTabIntent): void {
    if (this.disposed) return;
    this.wanted = intent;
    void this.drain();
  }
  dispose(): void {
    this.disposed = true;
    this.wanted = undefined;
  }
  private async drain(): Promise<void> {
    if (this.running) return;
    this.running = true;
    try {
      while (this.wanted && !this.disposed) {
        const intent = this.wanted;
        this.wanted = undefined;
        try {
          await this.send(intent);
        } catch (error: unknown) {
          this.onError(error);
        }
      }
    } finally {
      this.running = false;
    }
  }
}

export function tabCapture(
  snapshot: Pick<Snapshot, "state" | "meetingRecording">,
): "idle" | "dictation" | "finishing" | "cleaning" | "meeting" {
  if (snapshot.meetingRecording) return "meeting";
  switch (snapshot.state) {
    case "loading":
    case "listening":
      return "dictation";
    case "finishing":
      return "finishing";
    case "cleaning":
      return "cleaning";
    default:
      return "idle";
  }
}

/** Bound the preview at Unicode code-point boundaries, preserving the literal newest suffix. No truncation text is inserted. */
export function tabPreview(text: string, maximum = 280): string {
  const points = Array.from(text);
  return points.length > maximum ? points.slice(-maximum).join("") : text;
}

/**
 * Committed text followed by the live-preview draft, bounded together like
 * tabPreview. The draft only trails the committed text; it is never saved.
 */
export function tabPreviewParts(
  committed: string,
  draft: string,
  maximum = 280,
): { committed: string; draft: string } {
  if (!draft) return { committed: tabPreview(committed, maximum), draft: "" };
  const tail = Array.from((committed ? " " : "") + draft);
  if (tail.length >= maximum)
    return { committed: "", draft: tail.slice(-maximum).join("") };
  return {
    committed: tabPreview(committed, maximum - tail.length),
    draft: tail.join(""),
  };
}

/** Idle collapse waits for an empty tab; a temporary edge reveal stays open until the host hides it. */
export function tabCollapseAllowed(state: {
  pointerInside: boolean;
  keyboardInside: boolean;
  visible: boolean;
  recording: boolean;
  temporary: boolean;
  hasError?: boolean;
}): boolean {
  return (
    state.visible &&
    !state.hasError &&
    !state.recording &&
    !state.temporary &&
    !state.pointerInside &&
    !state.keyboardInside
  );
}
