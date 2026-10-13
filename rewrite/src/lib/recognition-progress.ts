import type { Snapshot } from "./protocol";

export function recognitionProgressLabel(snapshot: Snapshot): string | null {
  const progress = snapshot.recognitionProgress;
  if (
    !progress ||
    !["loading", "listening", "finishing"].includes(snapshot.state)
  )
    return null;
  const seconds = Math.floor(progress.elapsedMs / 1000);
  if (progress.stage === "loading")
    return snapshot.state === "finishing"
      ? `Loading speech model · ${seconds}s`
      : `Recording · loading model ${seconds}s`;
  if (progress.stage === "recovering")
    return `Recovering dictation · ${seconds}s`;
  return null;
}
