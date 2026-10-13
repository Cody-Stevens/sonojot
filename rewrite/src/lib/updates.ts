import type { UpdateStatus } from "./protocol";
import { formatBytes } from "./model-library";

export function updateStatusLabel(status: UpdateStatus): string {
  switch (status.state) {
    case "idle":
      return "Updates are checked automatically.";
    case "checking":
      return "Checking for updates…";
    case "available":
      return `SonoJot ${status.version} is available.`;
    case "downloading":
      return status.percent === null
        ? "Downloading update…"
        : `Downloading update · ${Math.floor(status.percent)}%`;
    case "ready":
      return `SonoJot ${status.version} is ready.`;
    case "error":
      return status.message;
  }
}

export function updateButtonLabel(
  status: UpdateStatus,
  installing: boolean,
): string {
  if (installing) return "Restarting…";
  switch (status.state) {
    case "idle":
      return "";
    case "checking":
      return "Checking…";
    case "available":
      return "Update available";
    case "downloading":
      return status.percent === null
        ? "Downloading…"
        : `Downloading… ${Math.floor(status.percent)}%`;
    case "ready":
      return "Restart to update";
    case "error":
      return "Update failed · Retry";
  }
}

export function updateTooltip(
  status: UpdateStatus,
  installing: boolean,
  blockedReason: string | null,
): string {
  if (installing) return "Restarting to finish updating SonoJot.";
  switch (status.state) {
    case "idle":
      return "";
    case "checking":
      return "Checking for a new version of SonoJot.";
    case "available": {
      // Release metadata is plain text, never HTML. Keep the tooltip brief.
      const notes = status.notes?.replace(/\s+/g, " ").trim() ?? "";
      const summary =
        notes.length > 240 ? `${notes.slice(0, 237).trimEnd()}…` : notes;
      return [
        `SonoJot ${status.version}${status.size === null ? "" : ` · ${formatBytes(status.size)}`}.`,
        summary,
        "Click to download. Nothing changes until you restart.",
      ]
        .filter(Boolean)
        .join(" ");
    }
    case "downloading":
      return "Keep working. You'll be asked to restart when it's ready.";
    case "ready":
      return [
        "Click to restart and finish updating. Save any edits first.",
        blockedReason,
      ]
        .filter(Boolean)
        .join(" ");
    case "error":
      return `${status.message} Click to check for updates again.`;
  }
}

export function restartBlockedReason(
  backendReason: string | null,
  dirty: boolean,
  pending: readonly string[],
): string | null {
  if (backendReason) return backendReason;
  if (dirty) return "Save or discard your edits before restarting.";
  if (pending.length) return "Wait for the current action to finish.";
  return null;
}
